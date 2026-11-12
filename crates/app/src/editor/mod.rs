//! The bare-bones trim editor: in/out points, volume, a three-band EQ, lossless export.

mod audio_card;
mod audio_edits;
mod keyframes;
mod panel;
mod timeline;
mod waveform;

use std::path::{Path, PathBuf};

use cut::{CutPlan, StreamIndex, Ticks, Timebase, TrimRequest};
use eframe::egui;
use export::{ExportJob, ExportRequest, ExportResult};
use library::{CutSession, EditorPreferences, TrackSession, log};
use media::{AudioEdits, MediaInfo, StreamKind};
use playback::AudioSelection;

use crate::os::PendingDialog;

pub use panel::{EditorAction, height, show as show_panel};

use audio_edits::{describe_audio, track_edit};
use keyframes::{KeyframeFinder, KeyframeIndex};
use waveform::Waveform;

/// Editor state for the open file. Positions are microseconds from the start of the file,
/// like the player's; cut points are converted to the primary stream's ticks for export.
pub struct Editor {
    source: PathBuf,
    stream: StreamIndex,
    timebase: Timebase,
    origin: Ticks,
    duration: Ticks,
    pub start: Ticks,
    pub end: Ticks,
    audio_streams: Vec<StreamIndex>,
    /// What to call each track in `audio_streams`: a short name and a full description.
    audio_labels: Vec<(String, String)>,
    audio: AudioEdits,
    /// The track being heard and edited, or all of them at once.
    audio_view: AudioSelection,
    keyframes: KeyframeFinder,
    index: KeyframeIndex,
    waveform: Option<Waveform>,
    /// Waits for the keyframe scan to be done with the disk.
    waveform_pending: bool,
    /// The waveform of the tracks in view, redone when they or their edits change.
    waveform_view: Vec<f32>,
    waveform_view_stale: bool,
    repaint: egui::Context,
    pub snap_to_keyframes: bool,
    /// Playing the selection, which stops at the end point.
    pub previewing: bool,
    pub smart_cut: bool,
    /// Why smart cut can't be used for this file, if it can't.
    pub smart_cut_unavailable: Option<String>,
    export: Option<ExportJob>,
    pub result: Option<Result<ExportResult, String>>,
    save_dialog: Option<PendingDialog>,
}

impl Editor {
    /// `None` for files without a video or audio stream to cut on. `audio_view` is what the
    /// player is playing: every track, or one.
    pub fn new(
        info: &MediaInfo,
        audio_view: AudioSelection,
        session: Option<&CutSession>,
        preferences: &EditorPreferences,
        repaint: &egui::Context,
    ) -> Option<Editor> {
        let primary = info
            .video()
            .map(|(stream, _)| stream)
            .or_else(|| info.streams_of(StreamKind::Audio).next())?;
        let timebase = primary.timebase?;
        let duration = info.duration?;
        let mut editor = Editor {
            source: info.path.clone(),
            stream: primary.index,
            timebase,
            origin: info.start_time,
            duration,
            start: Ticks::ZERO,
            end: duration,
            audio_streams: info
                .streams_of(StreamKind::Audio)
                .map(|stream| stream.index)
                .collect(),
            audio_labels: info
                .streams_of(StreamKind::Audio)
                .map(|stream| {
                    let short = match &stream.language {
                        Some(language) => {
                            format!("#{} · {}", stream.index.0, language.to_uppercase())
                        }
                        None => format!("Track #{}", stream.index.0),
                    };
                    (short, crate::controls::track_label(stream))
                })
                .collect(),
            audio: AudioEdits::default(),
            audio_view,
            keyframes: KeyframeFinder::new(info.path.clone(), primary.index, repaint.clone()),
            index: KeyframeIndex::new(info.path.clone(), primary.index, repaint.clone()),
            waveform: None,
            waveform_pending: true,
            waveform_view: Vec::new(),
            waveform_view_stale: true,
            repaint: repaint.clone(),
            snap_to_keyframes: preferences.snap_to_keyframes,
            previewing: false,
            smart_cut: preferences.smart_cut,
            smart_cut_unavailable: export::smart_cut_support(info, primary.index).err(),
            export: None,
            result: None,
            save_dialog: None,
        };
        if let Some(session) = session.filter(|s| s.timebase == timebase) {
            editor.start = editor
                .to_position(session.start)
                .clamp(Ticks::ZERO, duration);
            editor.end = editor
                .to_position(session.end)
                .clamp(editor.start, duration);
            let streams: Vec<usize> = editor.audio_streams.iter().map(|s| s.0).collect();
            for track in session.track_edits(&streams) {
                let stream = StreamIndex(track.stream);
                if editor.audio_streams.contains(&stream) {
                    editor.audio.set(stream, track_edit(&track));
                }
            }
        }
        Some(editor)
    }

    pub fn duration(&self) -> Ticks {
        self.duration
    }

    fn to_stream(&self, position: Ticks) -> Ticks {
        (position + self.origin).rescale(Timebase::MICROS, self.timebase)
    }

    /// A stream timestamp as player microseconds.
    pub fn stream_to_position(&self, ticks: Ticks) -> Ticks {
        self.to_position(ticks)
    }

    fn to_position(&self, ticks: Ticks) -> Ticks {
        ticks.rescale(self.timebase, Timebase::MICROS) - self.origin
    }

    pub fn preferences(&self) -> EditorPreferences {
        EditorPreferences {
            snap_to_keyframes: self.snap_to_keyframes,
            smart_cut: self.smart_cut,
        }
    }

    /// Whether the export will re-encode the head of the cut.
    pub fn uses_smart_cut(&self) -> bool {
        self.smart_cut && self.smart_cut_unavailable.is_none()
    }

    /// Keyframe positions found so far, in player microseconds.
    pub fn keyframe_positions(&self) -> impl Iterator<Item = Ticks> + '_ {
        self.index.keyframes().iter().map(|&k| self.to_position(k))
    }

    /// Peak loudness per equal slice of the file, while it's being computed too.
    pub fn waveform_complete(&self) -> bool {
        !self.waveform_pending && self.waveform.as_ref().is_none_or(Waveform::is_complete)
    }

    /// Moves the start, snapping to the nearest keyframe when snapping is on.
    pub fn set_start(&mut self, position: Ticks) {
        let position = if self.snap_to_keyframes {
            let target = self.to_stream(position);
            let end = self.to_stream(self.end);
            keyframes::snap_start(self.index.keyframes(), target, end)
                .map_or(position, |k| self.to_position(k))
        } else {
            position
        };
        self.start = position.clamp(Ticks::ZERO, self.duration);
        if self.end <= self.start {
            self.end = self.duration;
        }
        self.result = None;
    }

    /// Moves the start to the previous or next keyframe. Returns the new start if it moved.
    pub fn step_start_to_keyframe(&mut self, forward: bool) -> Option<Ticks> {
        let current = self.to_stream(self.start);
        let keyframes = self.index.keyframes();
        let target = if forward {
            keyframes::next(keyframes, current)
        } else {
            keyframes::previous(keyframes, current)
        }?;
        let position = self.to_position(target).clamp(Ticks::ZERO, self.duration);
        if position >= self.end {
            return None;
        }
        self.start = position;
        self.result = None;
        Some(position)
    }

    pub fn set_end(&mut self, position: Ticks) {
        self.end = position.clamp(Ticks::ZERO, self.duration);
        if self.end <= self.start {
            self.start = Ticks::ZERO;
        }
        self.result = None;
    }

    fn request(&self) -> TrimRequest {
        TrimRequest {
            stream: self.stream,
            timebase: self.timebase,
            start: self.to_stream(self.start),
            end: self.to_stream(self.end),
        }
    }

    /// The plan once the start's keyframe is known; `None` while looking it up.
    pub fn plan(&mut self) -> Option<Result<CutPlan, cut::CutError>> {
        let request = self.request();
        let span = self.keyframes.lookup(request.start)?;
        Some(match (span.before, span.after) {
            (None, Some(first)) => request.plan_from_first_keyframe(first),
            _ if self.uses_smart_cut() => {
                request.plan_smart(span.before, span.after, &span.frame_times)
            }
            _ => request.plan(span.before),
        })
    }

    /// Where the cut will really start, in player microseconds.
    pub fn plan_start_position(&self, plan: &CutPlan) -> Ticks {
        self.to_position(plan.start)
    }

    pub fn source(&self) -> &std::path::Path {
        &self.source
    }

    pub fn session(&self, now: u64) -> CutSession {
        CutSession {
            path: self.source.clone(),
            timebase: self.timebase,
            start: self.to_stream(self.start),
            end: self.to_stream(self.end),
            gain: 0,
            equalizer: [0; 3],
            remove_audio: false,
            tracks: self
                .audio
                .iter()
                .map(|(stream, edit)| TrackSession {
                    stream: stream.0,
                    gain: edit.gain.0,
                    equalizer: [
                        edit.equalizer.bass.0,
                        edit.equalizer.mid.0,
                        edit.equalizer.treble.0,
                    ],
                    remove: edit.is_removed(),
                })
                .collect(),
            saved_at: now,
        }
    }

    pub fn is_exporting(&self) -> bool {
        self.export.is_some()
    }

    pub fn export_fraction(&self) -> f32 {
        self.export.as_ref().map_or(0.0, ExportJob::fraction)
    }

    /// Asks where to save, or picks a free name next to the source if there's no dialog.
    pub fn begin_export(&mut self) {
        // The dialog's thread picks a free name; checking the disk here could stall the UI.
        let suggested = export::suggested_output(&self.source);
        match crate::os::save_file_dialog(&suggested) {
            Some(dialog) => self.save_dialog = Some(dialog),
            None => self.export_to(export::unique_path(&suggested, Path::exists)),
        }
    }

    fn export_to(&mut self, output: PathBuf) {
        let plan = match self.plan() {
            Some(Ok(plan)) => plan,
            Some(Err(error)) => {
                self.result = Some(Err(error.to_string()));
                return;
            }
            None => {
                self.result = Some(Err("Still finding the keyframe; try again".into()));
                return;
            }
        };
        self.result = None;
        log::info(format!(
            "exporting {} to {}: ticks {}..{} of stream {}, smart cut {}, audio {}",
            self.source.display(),
            output.display(),
            plan.start.0,
            plan.end.0,
            plan.stream.0,
            plan.reencode.map_or(0, |head| head.frames),
            describe_audio(&self.audio)
        ));
        self.export = Some(ExportJob::start(ExportRequest {
            source: self.source.clone(),
            output,
            plan,
            audio: self.audio.clone(),
        }));
    }

    pub fn cancel_export(&mut self) {
        if let Some(job) = &self.export {
            job.cancel();
        }
    }

    /// Advances the save dialog and the running export. Returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = self.index.poll();
        // One scan at a time: on a hard disk two would make each other seek.
        if self.index.is_complete() && self.waveform_pending {
            self.waveform_pending = false;
            self.waveform = Some(Waveform::new(
                self.source.clone(),
                self.audio_streams.clone(),
                self.duration,
                self.repaint.clone(),
            ));
        }
        if let Some(waveform) = &mut self.waveform {
            self.waveform_view_stale |= waveform.poll();
        }
        if self.waveform_view_stale {
            self.refresh_waveform_view();
            changed = true;
        }
        if let Some(dialog) = &self.save_dialog
            && let Some(choice) = dialog.poll()
        {
            self.save_dialog = None;
            if let Some(path) = choice {
                self.export_to(path);
            }
            changed = true;
        }
        if let Some(job) = &mut self.export {
            changed = true;
            if let Some(result) = job.poll() {
                match &result {
                    Ok(done) => log::info(format!(
                        "exported {}: {:.2} s, {} frames re-encoded",
                        done.output.display(),
                        done.duration.0 as f64 / 1e6,
                        done.reencoded_frames
                    )),
                    Err(error) => log::error(format!("export failed: {error}")),
                }
                self.export = None;
                self.result = Some(result.map_err(|error| error.to_string()));
            }
        }
        changed
    }
}
