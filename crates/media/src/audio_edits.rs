//! What the trim editor does to each audio track, shared by its preview and the export.

use std::collections::BTreeMap;

use cut::StreamIndex;

use crate::{Decibels, Equalizer};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TrackKeep {
    #[default]
    Keep,
    Remove,
}

/// The edit to one audio track.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TrackEdit {
    pub gain: Decibels,
    pub equalizer: Equalizer,
    pub keep: TrackKeep,
}

impl TrackEdit {
    pub fn is_removed(&self) -> bool {
        self.keep == TrackKeep::Remove
    }

    /// Kept, but with gain or EQ that only a re-encode can apply.
    pub fn reencodes(&self) -> bool {
        !self.is_removed() && (self.gain.0 != 0 || !self.equalizer.is_flat())
    }
}

/// Edits by track; a track without one is left as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioEdits(BTreeMap<StreamIndex, TrackEdit>);

impl AudioEdits {
    pub fn get(&self, stream: StreamIndex) -> TrackEdit {
        self.0.get(&stream).copied().unwrap_or_default()
    }

    pub fn set(&mut self, stream: StreamIndex, edit: TrackEdit) {
        if edit == TrackEdit::default() {
            self.0.remove(&stream);
        } else {
            self.0.insert(stream, edit);
        }
    }

    /// No track is changed.
    pub fn is_none(&self) -> bool {
        self.0.is_empty()
    }

    /// The changed tracks and their edits.
    pub fn iter(&self) -> impl Iterator<Item = (StreamIndex, TrackEdit)> + '_ {
        self.0.iter().map(|(stream, edit)| (*stream, *edit))
    }
}
