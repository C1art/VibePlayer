use eframe::egui::{self, Key, Modifiers};

use crate::actions::Action;

/// Keyboard shortcuts shared by the player and the editor.
pub fn read(ctx: &egui::Context, seek_step: i64) -> Vec<Action> {
    if ctx.egui_wants_keyboard_input() {
        return Vec::new();
    }
    ctx.input_mut(|input| {
        let mut actions = Vec::new();
        let mut pressed = |modifiers: Modifiers, key: Key, action: Action| {
            if input.consume_key(modifiers, key) {
                actions.push(action);
            }
        };
        pressed(Modifiers::COMMAND, Key::O, Action::ShowOpenDialog);
        pressed(Modifiers::COMMAND, Key::W, Action::CloseFile);
        pressed(Modifiers::COMMAND, Key::Comma, Action::ToggleSettings);
        pressed(
            Modifiers::SHIFT,
            Key::ArrowLeft,
            Action::SeekBy { seconds: -1 },
        );
        pressed(
            Modifiers::SHIFT,
            Key::ArrowRight,
            Action::SeekBy { seconds: 1 },
        );
        pressed(
            Modifiers::COMMAND,
            Key::ArrowLeft,
            Action::SeekBy { seconds: -60 },
        );
        pressed(
            Modifiers::COMMAND,
            Key::ArrowRight,
            Action::SeekBy { seconds: 60 },
        );
        pressed(
            Modifiers::NONE,
            Key::ArrowLeft,
            Action::SeekBy {
                seconds: -seek_step,
            },
        );
        pressed(
            Modifiers::NONE,
            Key::ArrowRight,
            Action::SeekBy { seconds: seek_step },
        );
        pressed(Modifiers::NONE, Key::J, Action::SeekBy { seconds: -10 });
        pressed(Modifiers::NONE, Key::L, Action::SeekBy { seconds: 10 });
        pressed(Modifiers::NONE, Key::Space, Action::TogglePlay);
        pressed(Modifiers::NONE, Key::K, Action::TogglePlay);
        pressed(Modifiers::NONE, Key::Comma, Action::StepBack);
        pressed(Modifiers::NONE, Key::Period, Action::StepForward);
        pressed(Modifiers::NONE, Key::ArrowUp, Action::VolumeBy(5));
        pressed(Modifiers::NONE, Key::ArrowDown, Action::VolumeBy(-5));
        pressed(Modifiers::NONE, Key::M, Action::ToggleMute);
        pressed(Modifiers::NONE, Key::OpenBracket, Action::SpeedDown);
        pressed(Modifiers::NONE, Key::CloseBracket, Action::SpeedUp);
        pressed(
            Modifiers::NONE,
            Key::Backspace,
            Action::SetSpeed(media::Speed::NORMAL),
        );
        pressed(Modifiers::NONE, Key::F, Action::ToggleFullscreen);
        pressed(Modifiers::NONE, Key::F11, Action::ToggleFullscreen);
        pressed(Modifiers::NONE, Key::Escape, Action::ExitFullscreen);
        pressed(Modifiers::NONE, Key::Tab, Action::ToggleInfo);
        pressed(Modifiers::NONE, Key::F1, Action::ToggleHelp);
        pressed(Modifiers::SHIFT, Key::Slash, Action::ToggleHelp);
        pressed(Modifiers::NONE, Key::Questionmark, Action::ToggleHelp);
        pressed(Modifiers::NONE, Key::E, Action::ToggleEditor);
        pressed(Modifiers::NONE, Key::I, Action::TrimStartHere);
        pressed(Modifiers::NONE, Key::O, Action::TrimEndHere);
        pressed(Modifiers::NONE, Key::A, Action::CycleAudio);
        pressed(Modifiers::NONE, Key::S, Action::CycleSubtitles);
        pressed(Modifiers::NONE, Key::PageDown, Action::NextChapter);
        pressed(Modifiers::NONE, Key::PageUp, Action::PreviousChapter);
        let digits = [
            Key::Num0,
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (tenth, key) in digits.into_iter().enumerate() {
            pressed(Modifiers::NONE, key, Action::SeekToTenth(tenth as u8));
        }
        actions
    })
}

/// Shown in the help overlay.
pub const HELP: &[(&str, &str)] = &[
    ("Space / K", "Play or pause"),
    ("← / →", "Back / forward 5 seconds"),
    ("Shift + ← / →", "Back / forward 1 second"),
    ("Ctrl + ← / →", "Back / forward 1 minute"),
    ("J / L", "Back / forward 10 seconds"),
    (", / .", "Previous / next frame"),
    ("↑ / ↓", "Volume"),
    ("M", "Mute"),
    ("[ / ]", "Slower / faster"),
    ("Backspace", "Normal speed"),
    ("0 – 9", "Jump to 0% – 90%"),
    ("PgUp / PgDn", "Previous / next chapter"),
    ("A / S", "Next audio / subtitle track"),
    ("F / double-click", "Fullscreen"),
    ("Tab", "File info"),
    ("E", "Trim editor"),
    ("I / O", "Set trim start / end (editor)"),
    ("Ctrl + O", "Open file"),
    ("Ctrl + W", "Close file"),
    ("Ctrl + ,", "Settings"),
    ("F1 / ?", "This list"),
];

pub fn show_help(ctx: &egui::Context, open: &mut bool) {
    crate::widgets::window("Keyboard shortcuts")
        .open(open)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            egui::Grid::new("shortcuts")
                .num_columns(2)
                .spacing([24.0, 6.0])
                .show(ui, |ui| {
                    for (keys, what) in HELP {
                        crate::widgets::chip(ui, keys);
                        ui.label(*what);
                        ui.end_row();
                    }
                });
        });
}
