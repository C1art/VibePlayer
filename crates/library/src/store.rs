use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::history::HISTORY_VERSION;
use crate::settings::SETTINGS_VERSION;
use crate::{History, LibraryError, Settings};

const SETTINGS_FILE: &str = "settings.json";
const HISTORY_FILE: &str = "history.json";

/// `%APPDATA%\VibePlayer` on Windows; the XDG config directory elsewhere.
pub fn data_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }?;
    Some(base.join("VibePlayer"))
}

/// Reads and writes the persisted files in one directory.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Store {
        Store { dir }
    }

    pub fn open_default() -> Result<Store, LibraryError> {
        data_dir()
            .map(Store::new)
            .ok_or(LibraryError::NoDataDirectory)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Missing or unreadable files fall back to defaults so a bad file never stops the player.
    pub fn load_settings(&self) -> Settings {
        let mut settings: Settings = self.load(SETTINGS_FILE).unwrap_or_default();
        migrate_settings(&mut settings);
        settings
    }

    pub fn load_history(&self) -> History {
        let mut history: History = self.load(HISTORY_FILE).unwrap_or_default();
        migrate_history(&mut history);
        history
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), LibraryError> {
        self.save(SETTINGS_FILE, settings)
    }

    pub fn save_history(&self, history: &History) -> Result<(), LibraryError> {
        self.save(HISTORY_FILE, history)
    }

    fn load<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        let text = fs::read_to_string(self.dir.join(name)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Writes to a temporary file first so a crash never leaves a half-written file.
    fn save<T: Serialize>(&self, name: &str, value: &T) -> Result<(), LibraryError> {
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |error| LibraryError::Io { path, error }
        };
        fs::create_dir_all(&self.dir).map_err(io(&self.dir))?;
        let path = self.dir.join(name);
        let temp = self.dir.join(format!("{name}.tmp"));
        let json = serde_json::to_string_pretty(value).map_err(|error| LibraryError::Json {
            path: path.clone(),
            error,
        })?;
        fs::write(&temp, json).map_err(io(&temp))?;
        fs::rename(&temp, &path).map_err(io(&path))
    }
}

/// Upgrades settings written by older versions. Version 1 is the first format.
fn migrate_settings(settings: &mut Settings) {
    if settings.version < SETTINGS_VERSION {
        settings.version = SETTINGS_VERSION;
    }
    settings.volume = settings.volume.min(100);
    settings.seek_step_seconds = settings.seek_step_seconds.clamp(1, 60);
}

fn migrate_history(history: &mut History) {
    if history.version < HISTORY_VERSION {
        history.version = HISTORY_VERSION;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> Store {
        let dir =
            std::env::temp_dir().join(format!("vibeplayer-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Store::new(dir)
    }

    #[test]
    fn settings_round_trip_through_disk() {
        let store = temp_store("round-trip");
        let settings = Settings {
            volume: 42,
            ..Settings::default()
        };
        store.save_settings(&settings).unwrap();
        assert_eq!(store.load_settings(), settings);
    }

    #[test]
    fn missing_fields_take_defaults_and_unknown_fields_are_ignored() {
        let store = temp_store("partial");
        fs::create_dir_all(store.dir()).unwrap();
        fs::write(
            store.dir().join(SETTINGS_FILE),
            r#"{"version": 1, "volume": 30, "from_the_future": true}"#,
        )
        .unwrap();
        let settings = store.load_settings();
        assert_eq!(settings.volume, 30);
        assert!(settings.hardware_decoding);
    }

    #[test]
    fn corrupt_files_fall_back_to_defaults() {
        let store = temp_store("corrupt");
        fs::create_dir_all(store.dir()).unwrap();
        fs::write(store.dir().join(SETTINGS_FILE), "{not json").unwrap();
        assert_eq!(store.load_settings(), Settings::default());
    }

    #[test]
    fn out_of_range_values_are_clamped_on_load() {
        let store = temp_store("clamp");
        fs::create_dir_all(store.dir()).unwrap();
        fs::write(
            store.dir().join(SETTINGS_FILE),
            r#"{"volume": 250, "seek_step_seconds": 0}"#,
        )
        .unwrap();
        let settings = store.load_settings();
        assert_eq!((settings.volume, settings.seek_step_seconds), (100, 1));
    }
}
