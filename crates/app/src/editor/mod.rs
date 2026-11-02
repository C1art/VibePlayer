//! The bare-bones trim editor: in/out points, volume, a three-band EQ, lossless export.

mod keyframes;
mod panel;
mod timeline;

use std::path::{Path, PathBuf};

use cut::{CutPlan, StreamIndex, Ticks, Timebase, TrimRequest};
use export::{AudioChanges, ExportJob, ExportRequest, ExportResult};
use library::CutSession;
use media::{Decibels, Equalizer, MediaInfo, StreamKind};

use crate::os::PendingDialog;

pub use panel::{EditorAction, HEIGHT, show as show_panel};

use keyframes::KeyframeFinder;

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
    pub gain: Decibels,
    pub equalizer: Equalizer,
    keyframes: KeyframeFinder,
    export: Option<ExportJob>,
    pub result: Option<Result<ExportResult, String>>,
    save_dialog: Option<PendingDialog>,
}

impl Editor {
    /// `None` for files without a video or audio stream to cut on.
    pub fn new(info: &MediaInfo, session: Option<&CutSession>) -> Option<Editor> {
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
            gain: Decibels(0),
            equalizer: Equalizer::default(),
            keyframes: KeyframeFinder::new(info.path.clone(), primary.index),
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
            editor.gain = Decibels(session.gain);
            let [bass, mid, treble] = session.equalizer;
            editor.equalizer = Equalizer {
                bass: Decibels(bass),
                mid: Decibels(mid),
                treble: Decibels(treble),
            };
        }
        Some(editor)
    }

    pub fn duration(&self) -> Ticks {
        self.duration
    }

    fn to_stream(&self, position: Ticks) -> Ticks {
        (position + self.origin).rescale(Timebase::MICROS, self.timebase)
    }

    fn to_position(&self, ticks: Ticks) -> Ticks {
        ticks.rescale(self.timebase, Timebase::MICROS) - self.origin
    }

    pub fn set_start(&mut self, position: Ticks) {
        self.start = position.clamp(Ticks::ZERO, self.duration);
        if self.end <= self.start {
            self.end = self.duration;
        }
        self.result = None;
    }

    pub fn set_end(&mut self, position: Ticks) {
        self.end = position.clamp(Ticks::ZERO, self.duration);
        if self.end <= self.start {
            self.start = Ticks::ZERO;
        }
        self.result = None;
    }

    pub fn audio_changes(&self) -> AudioChanges {
        AudioChanges {
            gain: self.gain,
            equalizer: self.equalizer,
        }
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
        let keyframe = self.keyframes.lookup(request.start)?;
        Some(request.plan(keyframe))
    }

    /// Where the cut will really start, in player microseconds.
    pub fn plan_start_position(&self, plan: &CutPlan) -> Ticks {
        self.to_position(plan.start)
    }

    pub fn session(&self, now: u64) -> CutSession {
        CutSession {
            path: self.source.clone(),
            timebase: self.timebase,
            start: self.to_stream(self.start),
            end: self.to_stream(self.end),
            gain: self.gain.0,
            equalizer: [
                self.equalizer.bass.0,
                self.equalizer.mid.0,
                self.equalizer.treble.0,
            ],
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
        let suggested = export::unique_path(&export::suggested_output(&self.source), Path::exists);
        match crate::os::save_file_dialog(&suggested) {
            Some(dialog) => self.save_dialog = Some(dialog),
            None => self.export_to(suggested),
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
        self.export = Some(ExportJob::start(ExportRequest {
            source: self.source.clone(),
            output,
            plan,
            audio: self.audio_changes(),
        }));
    }

    pub fn cancel_export(&mut self) {
        if let Some(job) = &self.export {
            job.cancel();
        }
    }

    /// Advances the save dialog and the running export. Returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
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
                self.export = None;
                self.result = Some(result.map_err(|error| error.to_string()));
            }
        }
        changed
    }
}
