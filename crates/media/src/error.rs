use std::fmt;
use std::path::PathBuf;

use cut::StreamIndex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// An FFmpeg call failed.
    Ffmpeg {
        operation: &'static str,
        code: i32,
        message: String,
    },
    /// FFmpeg takes paths as UTF-8 strings.
    PathNotUtf8(PathBuf),
    OutOfMemory,
    NoSuchStream(StreamIndex),
    NoDecoder(String),
    NoEncoder(String),
    Unsupported(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::Ffmpeg {
                operation, message, ..
            } => write!(f, "{operation} failed: {message}"),
            MediaError::PathNotUtf8(path) => {
                write!(f, "path is not valid Unicode: {}", path.display())
            }
            MediaError::OutOfMemory => f.write_str("out of memory"),
            MediaError::NoSuchStream(index) => write!(f, "no stream #{}", index.0),
            MediaError::NoDecoder(codec) => write!(f, "no decoder for {codec}"),
            MediaError::NoEncoder(codec) => write!(f, "no encoder for {codec}"),
            MediaError::Unsupported(what) => write!(f, "unsupported: {what}"),
        }
    }
}

impl std::error::Error for MediaError {}
