use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    NoOutputDevice,
    Device(String),
    UnsupportedSampleFormat(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::NoOutputDevice => f.write_str("no audio output device"),
            AudioError::Device(message) => write!(f, "audio device error: {message}"),
            AudioError::UnsupportedSampleFormat(format) => {
                write!(f, "unsupported device sample format {format}")
            }
        }
    }
}

impl std::error::Error for AudioError {}
