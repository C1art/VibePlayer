use std::rc::Rc;

use cut::{ClockPrecision, ClockTime, StreamIndex, Ticks, Timebase};
use eframe::egui::{self, Color32, Rect, TextureHandle, pos2, vec2};
use media::{MediaInfo, Speed, StreamKind};
use playback::SeekMode;

use crate::actions::{Action, SPEEDS};
use crate::chapters::ChapterMark;
use crate::icons::Icon;
use crate::seek_bar::{Range, SeekBar};
use crate::theme;
use crate::widgets::{icon_button, pill_button, thin_slider, vertical_fade};

pub const HEIGHT: f32 = 104.0;

/// Everything the control bar shows.
pub struct ControlsState<'a> {
    pub playing: bool,
    pub ended: bool,
    pub position: Ticks,
    pub duration: Ticks,
    pub volume: audio::Volume,
    pub muted: bool,
    pub speed: Speed,
    pub fullscreen: bool,
    pub info: &'a MediaInfo,
    pub audio_stream: Option<StreamIndex>,
    pub subtitle_stream: Option<StreamIndex>,
    pub highlight: Option<Range>,
    pub chapters: Rc<[ChapterMark]>,
}

/// Draws the bottom overlay and returns what the user asked for. Also reports whether the
/// pointer is over the controls, so they stay visible.
pub fn show(
    ui: &mut egui::Ui,
    area: Rect,
    state: &ControlsState<'_>,
    preview: &mut dyn FnMut(Ticks) -> Option<TextureHandle>,
    actions: &mut Vec<Action>,
) -> bool {
    let rect = Rect::from_min_max(pos2(area.left(), area.bottom() - HEIGHT), area.max);
    vertical_fade(
        ui,
        Rect::from_min_max(pos2(rect.left(), rect.top() - 40.0), rect.max),
        Color32::TRANSPARENT,
        Color32::from_black_alpha(215),
    );
    let inner = rect.shrink2(vec2(20.0, 10.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
    let ui = &mut child;
    let seek = SeekBar {
        position: state.position,
        duration: state.duration,
        highlight: state.highlight.as_ref().map(|range| Range {
            start: range.start,
            end: range.end,
        }),
        chapters: &state.chapters,
        preview: Some(preview),
    }
    .show(ui);
    if let Some(position) = seek.scrub {
        actions.push(Action::SeekTo {
            position,
            mode: SeekMode::Keyframe,
        });
    }
    if let Some(position) = seek.commit {
        actions.push(Action::SeekTo {
            position,
            mode: SeekMode::Exact,
        });
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let play_icon = if state.ended {
            Icon::Replay
        } else if state.playing {
            Icon::Pause
        } else {
            Icon::Play
        };
        if icon_button(ui, play_icon, 40.0, "Play / pause (Space)").clicked() {
            actions.push(Action::TogglePlay);
        }
        if icon_button(ui, Icon::SkipBack, 34.0, "Back 10 s (J)").clicked() {
            actions.push(Action::SeekBy { seconds: -10 });
        }
        if icon_button(ui, Icon::SkipForward, 34.0, "Forward 10 s (L)").clicked() {
            actions.push(Action::SeekBy { seconds: 10 });
        }
        ui.add_space(6.0);
        let volume_icon = if state.muted || state.volume.percent() == 0 {
            Icon::Muted
        } else if state.volume.percent() < 50 {
            Icon::VolumeLow
        } else {
            Icon::Volume
        };
        if icon_button(ui, volume_icon, 34.0, "Mute (M)").clicked() {
            actions.push(Action::ToggleMute);
        }
        let shown_volume = if state.muted {
            0.0
        } else {
            f32::from(state.volume.percent()) / 100.0
        };
        if let Some(value) = thin_slider(ui, shown_volume, 90.0) {
            let target = (value * 100.0).round() as i32;
            // Unmute first: raising the volume would unmute too, and a toggle after it re-mutes.
            if state.muted {
                actions.push(Action::ToggleMute);
            }
            actions.push(Action::VolumeBy(target - i32::from(state.volume.percent())));
        }
        ui.add_space(10.0);
        let time = format!(
            "{} / {}",
            ClockTime::new(state.position, Timebase::MICROS, ClockPrecision::Seconds),
            ClockTime::new(state.duration, Timebase::MICROS, ClockPrecision::Seconds)
        );
        ui.label(egui::RichText::new(time).color(theme::TEXT).size(13.5));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let fullscreen_icon = if state.fullscreen {
                Icon::ExitFullscreen
            } else {
                Icon::Fullscreen
            };
            if icon_button(ui, fullscreen_icon, 34.0, "Fullscreen (F)").clicked() {
                actions.push(Action::ToggleFullscreen);
            }
            if icon_button(ui, Icon::Scissors, 34.0, "Trim (E)").clicked() {
                actions.push(Action::ToggleEditor);
            }
            if icon_button(ui, Icon::Info, 34.0, "File info (Tab)").clicked() {
                actions.push(Action::ToggleInfo);
            }
            if icon_button(ui, Icon::Settings, 34.0, "Settings (Ctrl + ,)").clicked() {
                actions.push(Action::ToggleSettings);
            }
            if icon_button(ui, Icon::Folder, 34.0, "Open file (Ctrl + O)").clicked() {
                actions.push(Action::ShowOpenDialog);
            }
            tracks_menu(ui, state, actions);
            let speed = format!("{}×", format_speed(state.speed));
            let response = pill_button(ui, &speed, "Playback speed ([ / ])");
            egui::Popup::menu(&response).show(|ui| {
                ui.set_min_width(90.0);
                for percent in SPEEDS.iter().rev() {
                    let speed = Speed::from_percent(*percent);
                    let label = format!("{}×", format_speed(speed));
                    if ui.selectable_label(speed == state.speed, label).clicked() {
                        actions.push(Action::SetSpeed(speed));
                    }
                }
            });
        });
    });
    ui.ctx()
        .pointer_hover_pos()
        .is_some_and(|pointer| rect.contains(pointer))
}

fn tracks_menu(ui: &mut egui::Ui, state: &ControlsState<'_>, actions: &mut Vec<Action>) {
    let audio: Vec<_> = state.info.streams_of(StreamKind::Audio).collect();
    let subtitles: Vec<_> = state.info.streams_of(StreamKind::Subtitle).collect();
    if audio.len() < 2 && subtitles.is_empty() {
        return;
    }
    let response = icon_button(ui, Icon::Subtitles, 34.0, "Audio and subtitles (A / S)");
    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(220.0);
        if !audio.is_empty() {
            crate::widgets::section_title(ui, "Audio");
            for stream in &audio {
                let selected = state.audio_stream == Some(stream.index);
                if ui.selectable_label(selected, track_label(stream)).clicked() {
                    actions.push(Action::SelectAudio(Some(stream.index)));
                }
            }
        }
        if !subtitles.is_empty() {
            ui.separator();
            crate::widgets::section_title(ui, "Subtitles");
            if ui
                .selectable_label(state.subtitle_stream.is_none(), "Off")
                .clicked()
            {
                actions.push(Action::SelectSubtitle(None));
            }
            for stream in &subtitles {
                let selected = state.subtitle_stream == Some(stream.index);
                if ui.selectable_label(selected, track_label(stream)).clicked() {
                    actions.push(Action::SelectSubtitle(Some(stream.index)));
                }
            }
        }
    });
}

pub fn track_label(stream: &media::StreamInfo) -> String {
    let mut parts = vec![format!("#{}", stream.index.0)];
    if let Some(title) = &stream.title {
        parts.push(title.clone());
    }
    if let Some(language) = &stream.language {
        parts.push(language.to_uppercase());
    }
    parts.push(stream.codec.to_uppercase());
    if let media::StreamDetails::Audio(audio) = &stream.details
        && !audio.layout.is_empty()
    {
        parts.push(audio.layout.clone());
    }
    parts.join(" · ")
}

pub fn format_speed(speed: Speed) -> String {
    let percent = speed.percent();
    if percent.is_multiple_of(100) {
        format!("{}", percent / 100)
    } else if percent.is_multiple_of(10) {
        format!("{}.{}", percent / 100, (percent % 100) / 10)
    } else {
        format!("{}.{:02}", percent / 100, percent % 100)
    }
}

#[cfg(test)]
mod tests {
    use super::format_speed;
    use media::Speed;

    #[test]
    fn speeds_read_naturally() {
        assert_eq!(format_speed(Speed::NORMAL), "1");
        assert_eq!(format_speed(Speed::from_percent(150)), "1.5");
        assert_eq!(format_speed(Speed::from_percent(25)), "0.25");
    }
}
