use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum LibraryError {
    NoDataDirectory,
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    Json {
        path: PathBuf,
        error: serde_json::Error,
    },
}

impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryError::NoDataDirectory => f.write_str("no settings directory available"),
            LibraryError::Io { path, error } => write!(f, "{}: {error}", path.display()),
            LibraryError::Json { path, error } => write!(f, "{}: {error}", path.display()),
        }
    }
}

impl std::error::Error for LibraryError {}
