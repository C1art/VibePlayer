use std::fmt;

use audio::AudioError;
use media::MediaError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackError {
    Media(MediaError),
    Audio(AudioError),
}

impl fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaybackError::Media(error) => error.fmt(f),
            PlaybackError::Audio(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PlaybackError {}

impl From<MediaError> for PlaybackError {
    fn from(error: MediaError) -> Self {
        PlaybackError::Media(error)
    }
}

impl From<AudioError> for PlaybackError {
    fn from(error: AudioError) -> Self {
        PlaybackError::Audio(error)
    }
}
