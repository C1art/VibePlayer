use std::time::Instant;

use cut::Ticks;
use eframe::egui;
use media::StreamKind;
use playback::SeekMode;

use crate::actions::{self, Action};
use crate::app::VibePlayerApp;
use crate::chapters;
use crate::controls::format_speed;
use crate::os;
use crate::subtitles::SubtitleOverlay;

impl VibePlayerApp {
    pub(crate) fn apply(&mut self, ctx: &egui::Context, action: Action, now: Instant) {
        match action {
            Action::ShowOpenDialog => {
                if self.open_dialog.is_none() {
                    self.open_dialog = os::open_file_dialog();
                    if self.open_dialog.is_none() {
                        self.osd.show("Drop a file onto the window to open it");
                    }
                }
            }
            Action::Open(path) => {
                if !self.export_in_the_way() {
                    self.open(path, now);
                }
            }
            Action::CloseFile => {
                if !self.export_in_the_way() {
                    self.close_file();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Title("VibePlayer".into()));
                }
            }
            Action::ClearRecents => {
                self.history.clear_watch_history();
                self.save_history();
            }
            Action::ToggleFullscreen => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!self.fullscreen));
            }
            Action::ExitFullscreen => {
                if self.help_open || self.settings_open {
                    self.help_open = false;
                    self.settings_open = false;
                } else if self.info_open {
                    self.info_open = false;
                } else if self.fullscreen {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                }
            }
            Action::ToggleInfo => {
                if self.session.is_some() && self.editor.is_none() {
                    self.info_open = !self.info_open;
                }
            }
            Action::ToggleHelp => self.help_open = !self.help_open,
            Action::ToggleSettings => self.settings_open = !self.settings_open,
            Action::ToggleEditor => self.toggle_editor(ctx),
            Action::TrimStartHere => self.set_trim_point(true),
            Action::TrimEndHere => self.set_trim_point(false),
            Action::Editor(action) => self.apply_editor(action, now),
            other => self.apply_to_player(other, now),
        }
    }

    fn apply_to_player(&mut self, action: Action, now: Instant) {
        let Some(session) = &mut self.session else {
            return;
        };
        if session.player.opened().is_none() {
            return;
        }
        let player = &mut session.player;
        match action {
            Action::TogglePlay => player.toggle_play(now),
            Action::Play => player.play(now),
            Action::Pause => player.pause(),
            Action::SeekBy { seconds } => {
                player.seek_relative(Ticks(seconds * 1_000_000));
                let direction = if seconds < 0 { "−" } else { "+" };
                self.osd.show(format!("{direction}{} s", seconds.abs()));
            }
            Action::SeekTo { position, mode } => player.seek(position, mode),
            Action::SeekToTenth(tenth) => {
                if let Some(duration) = player.duration() {
                    player.seek(Ticks(duration.0 / 10 * i64::from(tenth)), SeekMode::Exact);
                }
            }
            Action::StepForward => player.step_forward(now),
            Action::StepBack => player.step_backward(now),
            Action::NextChapter | Action::PreviousChapter => {
                let marks = &session.chapters;
                let position = player.position();
                let target = if action == Action::NextChapter {
                    chapters::next(marks, position)
                } else {
                    chapters::previous(marks, position)
                };
                match target {
                    Some(index) => {
                        player.seek(marks[index].start, SeekMode::Exact);
                        self.osd.show(chapters::label(marks, index));
                    }
                    None if marks.is_empty() => self.osd.show("No chapters"),
                    None if action == Action::NextChapter => self.osd.show("Last chapter"),
                    // Before the first chapter, or in it: back to the start of the file.
                    None => {
                        player.seek(Ticks::ZERO, SeekMode::Exact);
                        self.osd.show(if marks[0].start <= position {
                            chapters::label(marks, 0)
                        } else {
                            "Start".into()
                        });
                    }
                }
            }
            Action::VolumeBy(delta) => {
                let volume = player.volume().step(delta);
                player.set_volume(volume);
                if player.is_muted() && delta > 0 {
                    player.set_muted(false);
                }
                self.settings.volume = volume.percent();
                self.settings.muted = player.is_muted();
                self.osd.show(format!("Volume {}%", volume.percent()));
                self.save_settings();
            }
            Action::ToggleMute => {
                let muted = !player.is_muted();
                player.set_muted(muted);
                self.settings.muted = muted;
                self.osd.show(if muted { "Muted" } else { "Unmuted" });
                self.save_settings();
            }
            Action::SpeedUp => {
                let speed = actions::faster(player.speed());
                self.set_speed(speed);
            }
            Action::SpeedDown => {
                let speed = actions::slower(player.speed());
                self.set_speed(speed);
            }
            Action::SetSpeed(speed) => self.set_speed(speed),
            Action::SelectAudio(stream) => player.select_audio(stream),
            Action::SelectSubtitle(stream) => {
                player.select_subtitle(stream);
                // Image cues are cached by start time, which another track can share.
                self.subtitles = SubtitleOverlay::default();
            }
            Action::CycleAudio => {
                let current = player.opened().and_then(|o| o.audio);
                if let Some(next) = self.cycle_stream(StreamKind::Audio, current, false)
                    && let Some(session) = &mut self.session
                {
                    session.player.select_audio(next);
                    self.osd.show(stream_osd("Audio", session, next));
                }
            }
            Action::CycleSubtitles => {
                let current = player.opened().and_then(|o| o.subtitle);
                if let Some(next) = self.cycle_stream(StreamKind::Subtitle, current, true)
                    && let Some(session) = &mut self.session
                {
                    session.player.select_subtitle(next);
                    self.subtitles = SubtitleOverlay::default();
                    self.osd.show(stream_osd("Subtitles", session, next));
                }
            }
            Action::ShowOpenDialog
            | Action::Open(_)
            | Action::CloseFile
            | Action::ClearRecents
            | Action::ToggleFullscreen
            | Action::ExitFullscreen
            | Action::ToggleInfo
            | Action::ToggleHelp
            | Action::ToggleSettings
            | Action::ToggleEditor
            | Action::TrimStartHere
            | Action::TrimEndHere
            | Action::Editor(_) => {}
        }
    }

    fn set_speed(&mut self, speed: media::Speed) {
        if let Some(session) = &mut self.session {
            session.player.set_speed(speed);
            self.osd.show(format!("Speed {}×", format_speed(speed)));
        }
    }
}

fn stream_osd(
    kind: &str,
    session: &crate::session::Session,
    stream: Option<cut::StreamIndex>,
) -> String {
    let label = stream
        .and_then(|index| session.player.opened()?.info.stream(index))
        .map_or_else(|| "Off".to_owned(), crate::controls::track_label);
    format!("{kind}: {label}")
}
