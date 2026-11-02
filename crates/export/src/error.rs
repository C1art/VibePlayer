use std::fmt;
use std::path::PathBuf;

use cut::CutError;
use media::MediaError;

#[derive(Debug)]
pub enum ExportError {
    Media(MediaError),
    Cut(CutError),
    Io(std::io::Error),
    /// The output would replace the file being cut.
    WouldOverwriteSource(PathBuf),
    OutputExists(PathBuf),
    /// The written file didn't probe as planned.
    Verification(String),
    /// Smart cut isn't possible for this stream; the reason is for the user.
    SmartCut(String),
    Cancelled,
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::Media(error) => error.fmt(f),
            ExportError::Cut(error) => error.fmt(f),
            ExportError::Io(error) => error.fmt(f),
            ExportError::WouldOverwriteSource(path) => {
                write!(
                    f,
                    "refusing to overwrite the source file {}",
                    path.display()
                )
            }
            ExportError::OutputExists(path) => write!(f, "{} already exists", path.display()),
            ExportError::Verification(reason) => write!(f, "the exported file is wrong: {reason}"),
            ExportError::SmartCut(reason) => write!(f, "smart cut unavailable: {reason}"),
            ExportError::Cancelled => f.write_str("export cancelled"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<MediaError> for ExportError {
    fn from(error: MediaError) -> Self {
        ExportError::Media(error)
    }
}

impl From<CutError> for ExportError {
    fn from(error: CutError) -> Self {
        ExportError::Cut(error)
    }
}

impl From<std::io::Error> for ExportError {
    fn from(error: std::io::Error) -> Self {
        ExportError::Io(error)
    }
}
