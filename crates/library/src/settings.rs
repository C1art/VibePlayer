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
    /// Show hardware-decoded pictures straight from the GPU, without a copy through memory.
    pub gpu_frames: bool,
    /// Present HDR video above SDR white on HDR displays; takes effect at the next launch.
    pub hdr_output: bool,
    /// The accent color of the interface and the app icon.
    pub theme: ColorTheme,
    pub resume_playback: bool,
    /// Seconds jumped by the arrow keys.
    pub seek_step_seconds: u32,
    pub updates: UpdatePreferences,
    pub editor: EditorPreferences,
    /// Where the window was when the app last closed.
    pub window: Option<WindowPlacement>,
}

/// The window's size and position when not maximized, and whether it was maximized. On
/// Windows the outer rectangle in pixels; elsewhere egui's logical points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            version: SETTINGS_VERSION,
            volume: 80,
            muted: false,
            hardware_decoding: true,
            gpu_frames: true,
            hdr_output: true,
            theme: ColorTheme::default(),
            resume_playback: true,
            seek_step_seconds: 5,
            updates: UpdatePreferences::default(),
            editor: EditorPreferences::default(),
            window: None,
        }
    }
}

/// The interface's accent color, which the app icon follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorTheme {
    #[default]
    Teal,
    Ocean,
    Mint,
    Amber,
    Coral,
    Purple,
    Graphite,
}

impl ColorTheme {
    pub const ALL: [ColorTheme; 7] = [
        ColorTheme::Teal,
        ColorTheme::Ocean,
        ColorTheme::Mint,
        ColorTheme::Amber,
        ColorTheme::Coral,
        ColorTheme::Purple,
        ColorTheme::Graphite,
    ];
}

/// Trim editor choices that carry over between files.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPreferences {
    /// Move the start handle to the nearest keyframe while dragging.
    pub snap_to_keyframes: bool,
    /// Re-encode from the start to the next keyframe instead of moving the start back.
    pub smart_cut: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdatePreferences {
    pub check_automatically: bool,
    /// Unix seconds of the last successful check.
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
