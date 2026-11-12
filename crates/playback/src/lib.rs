//! Playback engine: clock, decode scheduling, seeking, frame stepping, speed.
//!
//! A reader thread demuxes and routes packets to a video and an audio thread. The [`Player`]
//! on the UI thread polls their results each frame and never blocks. Every seek bumps a
//! [`Generation`]; anything produced for an older generation is dropped.

mod audio_worker;
mod clock;
mod error;
mod frame_queue;
mod messages;
mod mixer;
mod player;
mod reader;
mod subtitle_list;
mod video_worker;

pub use clock::{AnchorPair, AudioAnchor, AudioReading, Clock, Generation};
pub use error::PlaybackError;
pub use frame_queue::{FrameQueue, Shown, Timed};
pub use messages::{AudioSelection, Opened};
pub use player::{LoadState, Player, PlayerOptions, SeekMode, Update};
