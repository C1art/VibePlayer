//! Settings, recents, resume positions, cut sessions and update preferences, stored in
//! `%APPDATA%\VibePlayer`.

mod error;
mod history;
mod saver;
mod settings;
mod store;

pub use error::LibraryError;
pub use history::{CutSession, History, RecentFile, ResumeEntry};
pub use saver::Saver;
pub use settings::{Settings, UpdatePreferences};
pub use store::{Store, data_dir};
