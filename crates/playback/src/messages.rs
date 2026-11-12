use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use audio::AudioControl;
use crossbeam::atomic::AtomicCell;
use cut::{StreamIndex, Ticks};
use media::{AudioEdits, MediaInfo, Packet, Speed, StreamKind, StreamParams, Subtitle, VideoFrame};

use crate::{AnchorPair, Generation, PlaybackError};

/// Sent from the reader to a decoder thread, in file order.
pub(crate) enum PacketMessage {
    Packet(Generation, Packet),
    /// Drop buffered state; with `exact_target`, skip output before it.
    Flush {
        generation: Generation,
        /// Where the seek went, in microseconds from the start of the file.
        target: Ticks,
        exact_target: Option<Ticks>,
    },
    End(Generation),
    /// Switch to other audio streams; empty for none.
    SetStreams(Vec<(StreamIndex, StreamParams)>),
}

pub(crate) enum Command {
    Seek {
        generation: Generation,
        target: Ticks,
        exact: bool,
    },
    SelectAudio(AudioSelection),
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

/// Which audio tracks play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioSelection {
    /// Every audio track, mixed together.
    All,
    Track(StreamIndex),
}

impl AudioSelection {
    /// The streams this selects in `info`.
    pub fn streams(self, info: &MediaInfo) -> Vec<StreamIndex> {
        match self {
            AudioSelection::All => info
                .streams_of(StreamKind::Audio)
                .map(|stream| stream.index)
                .collect(),
            AudioSelection::Track(stream) => vec![stream],
        }
    }
}

/// The file's description and the streams chosen for playback.
#[derive(Clone, Debug)]
pub struct Opened {
    pub info: MediaInfo,
    pub video: Option<StreamIndex>,
    /// `None` for a file without audio.
    pub audio: Option<AudioSelection>,
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
    pub speed: AtomicCell<Speed>,
    /// Per-track gain, EQ and removal for the editor preview, bumped in `audio_edits_version`.
    audio_edits: Mutex<AudioEdits>,
    audio_edits_version: AtomicU64,
    quit: AtomicBool,
}

impl Shared {
    pub fn new(speed: Speed) -> Shared {
        Shared {
            generation: AtomicU64::new(0),
            audio_anchor: AtomicCell::new(None),
            speed: AtomicCell::new(speed),
            audio_edits: Mutex::new(AudioEdits::default()),
            audio_edits_version: AtomicU64::new(0),
            quit: AtomicBool::new(false),
        }
    }

    pub fn set_audio_edits(&self, edits: AudioEdits) {
        if let Ok(mut current) = self.audio_edits.lock() {
            *current = edits;
        }
        self.audio_edits_version.fetch_add(1, Ordering::AcqRel);
    }

    /// The edits, if they changed since `version`; updates `version`.
    pub fn audio_edits_since(&self, version: &mut u64) -> Option<AudioEdits> {
        let latest = self.audio_edits_version.load(Ordering::Acquire);
        if latest == *version {
            return None;
        }
        *version = latest;
        self.audio_edits.lock().ok().map(|edits| edits.clone())
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
