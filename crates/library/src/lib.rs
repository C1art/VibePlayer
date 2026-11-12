//! Settings, recents, resume positions, cut sessions and update preferences, stored in
//! `%APPDATA%\VibePlayer`.

mod error;
mod history;
pub mod log;
mod saver;
mod settings;
mod store;

pub use error::LibraryError;
pub use history::{CutSession, History, RecentFile, ResumeEntry, TrackSession};
pub use saver::Saver;
pub use settings::{ColorTheme, EditorPreferences, Settings, UpdatePreferences, WindowPlacement};
pub use store::{Store, data_dir};
