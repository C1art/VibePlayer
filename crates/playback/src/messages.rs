use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use audio::AudioControl;
use crossbeam::atomic::AtomicCell;
use cut::{StreamIndex, Ticks};
use media::{AudioEffects, MediaInfo, Packet, StreamParams, Subtitle, VideoFrame};

use crate::{AnchorPair, Generation, PlaybackError};

/// Sent from the reader to a decoder thread, in file order.
pub(crate) enum PacketMessage {
    Packet(Generation, Packet),
    /// Drop buffered state; with `exact_target`, skip output before it.
    Flush {
        generation: Generation,
        exact_target: Option<Ticks>,
    },
    End(Generation),
    /// Switch to another stream of the same kind, or none.
    SetStream(Option<StreamParams>),
}

pub(crate) enum Command {
    Seek {
        generation: Generation,
        target: Ticks,
        exact: bool,
    },
    SelectAudio(Option<StreamIndex>),
    SelectSubtitle(Option<StreamIndex>),
}

/// What the engine reports back to the player.
pub(crate) enum Event {
    Opened(Box<Opened>),
    Failed(PlaybackError),
    VideoDecoder {
        hardware: bool,
    },
    AudioReady(AudioControl),
    AudioFailed(PlaybackError),
    /// Whether the selected audio stream could be decoded.
    AudioDecoder {
        available: bool,
    },
    /// The video stream can't be decoded; play the rest without it.
    VideoFailed(PlaybackError),
    Subtitle(Generation, Subtitle),
    VideoEnded(Generation),
    AudioEnded(Generation),
}

/// The file's description and the streams chosen for playback.
#[derive(Clone, Debug)]
pub struct Opened {
    pub info: MediaInfo,
    pub video: Option<StreamIndex>,
    pub audio: Option<StreamIndex>,
    pub subtitle: Option<StreamIndex>,
}

pub(crate) struct DecodedVideo {
    pub generation: Generation,
    pub frame: VideoFrame,
}

/// Lock-free state shared by the player and its threads.
pub(crate) struct Shared {
    generation: AtomicU64,
    pub audio_anchor: AtomicCell<Option<AnchorPair>>,
    pub audio_effects: AtomicCell<AudioEffects>,
    quit: AtomicBool,
}

impl Shared {
    pub fn new(effects: AudioEffects) -> Shared {
        Shared {
            generation: AtomicU64::new(0),
            audio_anchor: AtomicCell::new(None),
            audio_effects: AtomicCell::new(effects),
            quit: AtomicBool::new(false),
        }
    }

    pub fn generation(&self) -> Generation {
        Generation(self.generation.load(Ordering::Acquire))
    }

    pub fn set_generation(&self, generation: Generation) {
        self.generation.store(generation.0, Ordering::Release);
    }

    /// Work for `generation` is no longer wanted.
    pub fn is_stale(&self, generation: Generation) -> bool {
        self.generation() != generation || self.should_quit()
    }

    pub fn should_quit(&self) -> bool {
        self.quit.load(Ordering::Acquire)
    }

    pub fn quit(&self) {
        self.quit.store(true, Ordering::Release);
    }
}
