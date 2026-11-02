//! Audio output: the cpal device, a lock-free sample queue, and live volume.
//! Resampling, downmixing, speed and EQ happen earlier, in `media`'s filter graph.

mod error;
mod output;
mod volume;

pub use error::AudioError;
pub use output::{AudioControl, AudioOutput, OutputFormat};
pub use volume::Volume;
