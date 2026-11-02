use std::fmt;

#[derive(Debug)]
pub enum UpdateError {
    Network(String),
    BadResponse(String),
    NoInstaller,
    Io(std::io::Error),
    Cancelled,
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::Network(message) => write!(f, "couldn't reach GitHub: {message}"),
            UpdateError::BadResponse(message) => write!(f, "unexpected release data: {message}"),
            UpdateError::NoInstaller => f.write_str("the release has no Windows installer"),
            UpdateError::Io(error) => error.fmt(f),
            UpdateError::Cancelled => f.write_str("download cancelled"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<ureq::Error> for UpdateError {
    fn from(error: ureq::Error) -> Self {
        UpdateError::Network(error.to_string())
    }
}

impl From<std::io::Error> for UpdateError {
    fn from(error: std::io::Error) -> Self {
        UpdateError::Io(error)
    }
}
