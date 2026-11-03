use std::path::PathBuf;

use cut::{StreamIndex, Ticks};
use media::Speed;
use playback::SeekMode;

/// Everything the user can ask the player to do, from keys, buttons or the OS.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    TogglePlay,
    /// From the system media controls.
    #[cfg_attr(not(windows), allow(dead_code))]
    Play,
    #[cfg_attr(not(windows), allow(dead_code))]
    Pause,
    SeekBy {
        seconds: i64,
    },
    SeekTo {
        position: Ticks,
        mode: SeekMode,
    },
    /// From the system media controls.
    NextChapter,
    #[cfg_attr(not(windows), allow(dead_code))]
    PreviousChapter,
    StepForward,
    StepBack,
    VolumeBy(i32),
    ToggleMute,
    SpeedUp,
    SpeedDown,
    SetSpeed(Speed),
    ToggleFullscreen,
    ExitFullscreen,
    ToggleInfo,
    ToggleHelp,
    ToggleSettings,
    ToggleEditor,
    /// Set the trim start or end at the current position (editor only).
    TrimStartHere,
    TrimEndHere,
    ShowOpenDialog,
    Open(PathBuf),
    SelectAudio(Option<StreamIndex>),
    SelectSubtitle(Option<StreamIndex>),
    CycleAudio,
    CycleSubtitles,
    CloseFile,
    ClearRecents,
    Editor(crate::editor::EditorAction),
}

pub const SPEEDS: [u16; 10] = [25, 50, 75, 100, 125, 150, 175, 200, 300, 400];

pub fn faster(speed: Speed) -> Speed {
    let next = SPEEDS.iter().find(|&&s| s > speed.percent()).copied();
    Speed::from_percent(next.unwrap_or(speed.percent()))
}

pub fn slower(speed: Speed) -> Speed {
    let next = SPEEDS.iter().rev().find(|&&s| s < speed.percent()).copied();
    Speed::from_percent(next.unwrap_or(speed.percent()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_steps_through_presets_and_stops_at_the_ends() {
        assert_eq!(faster(Speed::NORMAL), Speed::from_percent(125));
        assert_eq!(slower(Speed::NORMAL), Speed::from_percent(75));
        assert_eq!(faster(Speed::MAX), Speed::MAX);
        assert_eq!(slower(Speed::MIN), Speed::MIN);
    }
}
