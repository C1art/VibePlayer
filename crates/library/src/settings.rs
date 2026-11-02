use serde::{Deserialize, Serialize};

pub(crate) const SETTINGS_VERSION: u32 = 1;

/// User preferences. New fields need `#[serde(default)]` so older files keep loading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    /// Percent, 0–100.
    pub volume: u8,
    pub muted: bool,
    pub hardware_decoding: bool,
    pub resume_playback: bool,
    /// Seconds jumped by the arrow keys.
    pub seek_step_seconds: u32,
    pub updates: UpdatePreferences,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            version: SETTINGS_VERSION,
            volume: 80,
            muted: false,
            hardware_decoding: true,
            resume_playback: true,
            seek_step_seconds: 5,
            updates: UpdatePreferences::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdatePreferences {
    pub check_automatically: bool,
    /// Unix seconds of the last completed check.
    pub last_check: Option<u64>,
    /// A version the user chose to skip, e.g. `1.2.0`.
    pub skipped_version: Option<String>,
}

impl Default for UpdatePreferences {
    fn default() -> UpdatePreferences {
        UpdatePreferences {
            check_automatically: true,
            last_check: None,
            skipped_version: None,
        }
    }
}
