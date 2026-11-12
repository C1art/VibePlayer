//! The editor's audio state: per-track edits, the track in view, and its waveform.

use cut::StreamIndex;
use library::TrackSession;
use media::{AudioEdits, Decibels, Equalizer, TrackEdit, TrackKeep};
use playback::AudioSelection;

use super::Editor;

impl Editor {
    pub fn audio_changes(&self) -> &AudioEdits {
        &self.audio
    }

    pub fn audio_streams(&self) -> &[StreamIndex] {
        &self.audio_streams
    }

    /// Each audio track with its short name and full description.
    pub fn audio_tracks(&self) -> impl Iterator<Item = (StreamIndex, &str, &str)> {
        self.audio_streams
            .iter()
            .zip(&self.audio_labels)
            .map(|(stream, (short, full))| (*stream, short.as_str(), full.as_str()))
    }

    pub fn audio_view(&self) -> AudioSelection {
        self.audio_view
    }

    /// Hears and edits one track, or all of them together.
    pub fn set_audio_view(&mut self, view: AudioSelection) {
        self.audio_view = view;
        self.waveform_view_stale = true;
    }

    fn viewed_tracks(&self) -> Vec<StreamIndex> {
        match self.audio_view {
            AudioSelection::All => self.audio_streams.clone(),
            AudioSelection::Track(stream) => vec![stream],
        }
    }

    /// The edit the audio controls show: the viewed track's, or the first track's for all.
    pub fn shown_edit(&self) -> TrackEdit {
        self.viewed_tracks()
            .first()
            .map(|stream| self.audio.get(*stream))
            .unwrap_or_default()
    }

    /// The viewed tracks have different edits, so the controls show only the first's.
    pub fn viewed_edits_differ(&self) -> bool {
        let mut edits = self.viewed_tracks().into_iter().map(|s| self.audio.get(s));
        let first = edits.next();
        edits.any(|edit| Some(edit) != first)
    }

    /// Changes the viewed track, or every track when all are in view.
    pub fn edit_audio(&mut self, change: impl Fn(&mut TrackEdit)) {
        for stream in self.viewed_tracks() {
            let mut edit = self.audio.get(stream);
            change(&mut edit);
            self.audio.set(stream, edit);
        }
        self.result = None;
        self.waveform_view_stale = true;
    }

    /// Lets the player be heard as the export will be: the viewed tracks, with their edits.
    pub fn preview_audio(&self, player: &mut playback::Player) {
        player.set_audio_edits(self.audio.clone());
        if player.opened().and_then(|o| o.audio) != Some(self.audio_view) {
            player.select_audio(self.audio_view);
        }
    }

    /// The loudness of the viewed tracks; for all of them, of those kept.
    pub fn waveform(&self) -> &[f32] {
        &self.waveform_view
    }

    pub(super) fn refresh_waveform_view(&mut self) {
        let Some(waveform) = &self.waveform else {
            return;
        };
        let viewed = self.viewed_tracks();
        let kept: Vec<StreamIndex> = viewed
            .iter()
            .copied()
            .filter(|stream| !self.audio.get(*stream).is_removed())
            .collect();
        let shown = if kept.is_empty() { &viewed } else { &kept };
        self.waveform_view = waveform.peaks(shown);
        self.waveform_view_stale = false;
    }

    pub fn keyframes_complete(&self) -> bool {
        self.index.is_complete()
    }
}

pub(super) fn track_edit(track: &TrackSession) -> TrackEdit {
    let [bass, mid, treble] = track.equalizer;
    TrackEdit {
        gain: Decibels(track.gain),
        equalizer: Equalizer {
            bass: Decibels(bass),
            mid: Decibels(mid),
            treble: Decibels(treble),
        },
        keep: if track.remove {
            TrackKeep::Remove
        } else {
            TrackKeep::Keep
        },
    }
}

/// For the log: what happens to each edited audio track.
pub(super) fn describe_audio(edits: &AudioEdits) -> String {
    if edits.is_none() {
        return "copied".into();
    }
    edits
        .iter()
        .map(|(stream, edit)| {
            let what = if edit.is_removed() {
                "removed"
            } else if edit.reencodes() {
                "re-encoded"
            } else {
                "copied"
            };
            format!("#{} {what}", stream.0)
        })
        .collect::<Vec<_>>()
        .join(", ")
}
