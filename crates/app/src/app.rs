use std::path::PathBuf;

use crossbeam::channel::Receiver;
use std::time::{Duration, Instant};

use cut::{StreamIndex, Ticks};
use eframe::egui::{self, Color32, CursorIcon, Rect, Sense};
use library::{History, Saver, Settings, Store};
use media::{HardwareDecoding, Speed, VideoOptions};
use playback::{LoadState, PlayerOptions};

use crate::actions::Action;
use crate::controls::{self, ControlsState};
use crate::editor::{self, Editor};
use crate::info_panel::{self, PlaybackStats};
use crate::os::{MediaControls, NowPlaying, PendingDialog};
use crate::osd::Osd;
use crate::session::Session;
use crate::subtitles::SubtitleOverlay;
use crate::{shortcuts, theme, video_view, welcome};

/// Controls hide after the pointer rests this long during playback.
const CONTROLS_IDLE: Duration = Duration::from_millis(2200);
/// Keyframe seeks while dragging the seek bar are sent at most this often.
const SCRUB_INTERVAL: Duration = Duration::from_millis(60);

pub struct VibePlayerApp {
    pub(crate) store: Option<Store>,
    pub(crate) saver: Option<Saver>,
    pub(crate) settings: Settings,
    pub(crate) history: History,
    pub(crate) session: Option<Session>,
    pub(crate) error: Option<String>,
    pub(crate) osd: Osd,
    pub(crate) info_open: bool,
    pub(crate) help_open: bool,
    pub(crate) settings_open: bool,
    pub(crate) updates: crate::updates::Updates,
    pub(crate) high_bit_depth: Option<bool>,
    pub(crate) last_activity: Instant,
    pub(crate) last_scrub: Option<Instant>,
    pub(crate) scroll_accumulator: f32,
    pub(crate) subtitles: SubtitleOverlay,
    pub(crate) open_dialog: Option<PendingDialog>,
    pub(crate) media_controls: MediaControls,
    pub(crate) fullscreen: bool,
    pub(crate) editor: Option<Editor>,
    /// Files sent by later launches of the app.
    pub(crate) instance_requests: Option<Receiver<PathBuf>>,
}

impl VibePlayerApp {
    pub fn new(
        creation: &eframe::CreationContext<'_>,
        store: Option<Store>,
        initial_file: Option<PathBuf>,
        instance_requests: Option<Receiver<PathBuf>>,
    ) -> VibePlayerApp {
        theme::apply(&creation.egui_ctx);
        let settings = store.as_ref().map(Store::load_settings).unwrap_or_default();
        let history = store.as_ref().map(Store::load_history).unwrap_or_default();
        let high_bit_depth = creation.wgpu_render_state.as_ref().map(video_view::install);
        let mut app = VibePlayerApp {
            saver: store.clone().map(Saver::spawn),
            store,
            settings,
            history,
            session: None,
            error: None,
            osd: Osd::default(),
            info_open: false,
            help_open: false,
            settings_open: false,
            updates: crate::updates::Updates::default(),
            high_bit_depth,
            last_activity: Instant::now(),
            last_scrub: None,
            scroll_accumulator: 0.0,
            subtitles: SubtitleOverlay::default(),
            open_dialog: None,
            media_controls: MediaControls::new(creation),
            fullscreen: false,
            editor: None,
            instance_requests,
        };
        app.updates.start_automatic(&app.settings);
        if let Some(path) = initial_file {
            app.open(path, Instant::now());
        }
        app
    }

    pub(crate) fn player_options(&self, path: &std::path::Path) -> PlayerOptions {
        let hardware = if self.settings.hardware_decoding {
            HardwareDecoding::Auto
        } else {
            HardwareDecoding::Off
        };
        let start_at = self
            .settings
            .resume_playback
            .then(|| self.history.resume_position(path))
            .flatten();
        PlayerOptions {
            video: VideoOptions {
                hardware,
                high_bit_depth: self.high_bit_depth.unwrap_or(false),
            },
            volume: audio::Volume::from_percent(i32::from(self.settings.volume)),
            muted: self.settings.muted,
            speed: Speed::NORMAL,
            start_at,
            autoplay: true,
        }
    }

    pub(crate) fn open(&mut self, path: PathBuf, now: Instant) {
        self.close_file();
        let options = self.player_options(&path);
        if let Some(start) = options.start_at {
            let resumed =
                cut::ClockTime::new(start, cut::Timebase::MICROS, cut::ClockPrecision::Seconds);
            self.osd.show(format!("Resuming at {resumed}"));
        }
        self.error = None;
        self.session = Some(Session::open(path, options, now));
    }

    /// Closes the current file, remembering where it stopped.
    pub(crate) fn close_file(&mut self) {
        self.close_editor();
        if let Some(session) = self.session.take() {
            let player = &session.player;
            if let LoadState::Ready(_) = player.state() {
                let position = if player.has_ended() {
                    player.duration().unwrap_or(player.position())
                } else {
                    player.position()
                };
                self.history.record_position(
                    player.path(),
                    position,
                    player.duration(),
                    unix_now(),
                );
                self.save_history();
            }
        }
        self.subtitles = SubtitleOverlay::default();
        self.info_open = false;
    }

    pub(crate) fn save_history(&self) {
        if let Some(saver) = &self.saver {
            saver.save_history(&self.history);
        }
    }

    pub(crate) fn save_settings(&self) {
        if let Some(saver) = &self.saver {
            saver.save_settings(&self.settings);
        }
    }

    fn poll_session(&mut self, ctx: &egui::Context, frame: &eframe::Frame, now: Instant) {
        let Some(session) = &mut self.session else {
            return;
        };
        let update = session.player.update(now);
        if update.frame_changed
            && let (Some(render_state), Some(video)) =
                (frame.wgpu_render_state(), session.player.current_frame())
        {
            video_view::upload(render_state, video);
        }
        if session.poll_thumbnails() {
            ctx.request_repaint();
        }
        match session.player.state() {
            LoadState::Failed(error) => {
                self.error = Some(format!("Couldn't open {}: {error}", session.file_name()));
                self.session = None;
                if let Some(render_state) = frame.wgpu_render_state() {
                    video_view::clear(render_state);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Title("VibePlayer".into()));
                return;
            }
            LoadState::Ready(_) if !session.title_shown => {
                session.title_shown = true;
                // Only files that actually opened belong in Recent.
                self.history.add_recent(session.player.path(), unix_now());
                if let Some(saver) = &self.saver {
                    saver.save_history(&self.history);
                }
                let title = format!("{} — VibePlayer", session.file_name());
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
            }
            _ => {}
        }
        if session.player.is_playing() || session.player.is_seeking() {
            ctx.request_repaint();
        } else if let Some(after) = update.repaint_after {
            ctx.request_repaint_after(after);
        }
    }

    fn controls_visible(&self, ctx: &egui::Context, over_controls: bool) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        let idle = self.last_activity.elapsed() > CONTROLS_IDLE;
        let menus_open = egui::Popup::is_any_open(ctx);
        !session.player.is_playing() || !idle || over_controls || menus_open
    }

    fn handle_scrub_throttle(&mut self, actions: &mut Vec<Action>, now: Instant) {
        actions.retain(|action| match action {
            Action::SeekTo {
                mode: playback::SeekMode::Keyframe,
                ..
            } => {
                let due = self
                    .last_scrub
                    .is_none_or(|last| now.duration_since(last) >= SCRUB_INTERVAL);
                if due {
                    self.last_scrub = Some(now);
                }
                due
            }
            _ => true,
        });
    }
}

impl eframe::App for VibePlayerApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        if self.high_bit_depth.is_none() {
            self.high_bit_depth = frame.wgpu_render_state().map(video_view::install);
        }
        let mut actions = self.media_controls.poll();
        let input_activity = ctx.input(|input| {
            input.pointer.delta() != egui::Vec2::ZERO
                || input.pointer.any_down()
                || !input.events.is_empty()
        });
        if input_activity {
            self.last_activity = now;
        }
        if let Some(path) = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .find(|path| !path.as_os_str().is_empty())
        }) {
            actions.push(Action::Open(path));
        }
        if let Some(requests) = &self.instance_requests {
            for path in requests.try_iter() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                if !path.as_os_str().is_empty() {
                    actions.push(Action::Open(path));
                }
            }
        }
        if let Some(dialog) = &self.open_dialog
            && let Some(choice) = dialog.poll()
        {
            self.open_dialog = None;
            if let Some(path) = choice {
                actions.push(Action::Open(path));
            }
        }
        self.fullscreen = ctx.input(|input| input.viewport().fullscreen.unwrap_or(false));
        actions.extend(shortcuts::read(
            &ctx,
            i64::from(self.settings.seek_step_seconds),
        ));
        self.poll_session(&ctx, frame, now);
        if let Some(editor) = &mut self.editor
            && editor.poll()
        {
            ctx.request_repaint();
        }

        let area = ui.max_rect();
        ui.painter().rect_filled(area, 0.0, Color32::BLACK);
        let over_controls = match &mut self.session {
            None => {
                welcome::show(
                    ui,
                    &self.history.recents,
                    self.error.as_deref(),
                    &mut actions,
                );
                false
            }
            Some(_) => self.show_player(ui, area, &mut actions),
        };
        if self.help_open {
            shortcuts::show_help(&ctx, &mut self.help_open);
        }
        let mut settings_changed = false;
        if self.settings_open {
            settings_changed |= crate::settings_window::show(
                &ctx,
                &mut self.settings_open,
                &mut self.settings,
                &mut self.updates,
            );
        }
        self.updates.tick(&self.settings);
        match self.updates.poll(&mut self.settings) {
            crate::updates::UpdateOutcome::SettingsChanged => settings_changed = true,
            crate::updates::UpdateOutcome::InstallerLaunched => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            crate::updates::UpdateOutcome::None => {}
        }
        settings_changed |= self.updates.show(&ctx, &mut self.settings);
        if settings_changed {
            self.save_settings();
        }
        if self.updates.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(200));
        } else {
            // Wakes an idle window now and then so the daily re-check can run.
            ctx.request_repaint_after(Duration::from_secs(3600));
        }
        if self.osd.paint(ui, area) {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        let hide_cursor = self.session.is_some()
            && self.fullscreen
            && !self.controls_visible(&ctx, over_controls);
        if hide_cursor {
            ctx.set_cursor_icon(CursorIcon::None);
        }
        self.handle_scrub_throttle(&mut actions, now);
        for action in actions {
            self.apply(&ctx, action, now);
        }
        self.update_media_controls();
        if self.session.as_ref().is_some_and(|s| s.player.is_playing()) {
            ctx.request_repaint_after(CONTROLS_IDLE);
        }
    }

    fn on_exit(&mut self) {
        self.close_file();
        if let Some(store) = &self.store {
            let _ = store.save_settings(&self.settings);
            let _ = store.save_history(&self.history);
        }
    }
}

impl VibePlayerApp {
    /// Draws the video, subtitles and controls. Returns whether the pointer is over controls.
    fn show_player(&mut self, ui: &mut egui::Ui, area: Rect, actions: &mut Vec<Action>) -> bool {
        let ctx = ui.ctx().clone();
        let pointer_near_controls = ctx
            .pointer_hover_pos()
            .is_some_and(|p| p.y > area.bottom() - controls::HEIGHT);
        let visible = self.controls_visible(&ctx, pointer_near_controls);
        let Some(session) = &mut self.session else {
            return false;
        };
        let response = ui.interact(area, egui::Id::unique("video area"), Sense::click());
        if response.double_clicked() {
            actions.push(Action::ToggleFullscreen);
        } else if response.clicked() {
            actions.push(Action::TogglePlay);
        }
        if response.hovered() {
            self.scroll_accumulator += wheel_notches(&ctx);
            while self.scroll_accumulator.abs() >= 1.0 {
                let step = self.scroll_accumulator.signum();
                actions.push(Action::VolumeBy(5 * step as i32));
                self.scroll_accumulator -= step;
            }
        }
        let Some(opened) = session.player.opened().cloned() else {
            paint_loading(ui, area, &session.file_name());
            return false;
        };
        let full_area = area;
        let area = if self.editor.is_some() {
            Rect::from_min_max(
                area.min,
                egui::pos2(area.right(), area.bottom() - editor::HEIGHT),
            )
        } else {
            area
        };
        let video_rect = session
            .player
            .current_frame()
            .map(|frame| video_view::fit(area, frame.display_aspect()));
        if let Some(rect) = video_rect {
            video_view::paint(ui, rect);
        } else if opened.video.is_none() {
            paint_audio_only(ui, area, &session.file_name());
        }
        let subtitle = session.player.active_subtitle().cloned();
        let inset = if visible {
            controls::HEIGHT + 8.0
        } else {
            24.0
        };
        self.subtitles
            .paint(ui, video_rect.unwrap_or(area), inset, subtitle.as_ref());
        let duration = session.player.duration().unwrap_or(Ticks::ZERO);
        let mut over = false;
        if let Some(editor) = &mut self.editor {
            let editor_actions = editor::show_panel(
                ui,
                full_area,
                editor,
                session.player.position(),
                session.player.is_playing(),
            );
            actions.extend(editor_actions.into_iter().map(Action::Editor));
            over = true;
        } else if visible {
            let state = ControlsState {
                playing: session.player.is_playing(),
                ended: session.player.has_ended(),
                position: session.player.position(),
                duration,
                volume: session.player.volume(),
                muted: session.player.is_muted(),
                speed: session.player.speed(),
                fullscreen: self.fullscreen,
                info: &opened.info,
                audio_stream: opened.audio,
                subtitle_stream: opened.subtitle,
                highlight: None,
            };
            let mut preview = |at: Ticks| session.thumbnail(&ctx, at);
            over = controls::show(ui, area, &state, &mut preview, actions);
            paint_title(ui, area, &opened.info.file_name());
        }
        if self.info_open {
            let stats = PlaybackStats {
                hardware_decoding: session.player.hardware_decoding(),
                dropped_frames: session.player.dropped_frames(),
                audio_error: session.player.audio_error().map(ToString::to_string),
            };
            info_panel::show(&ctx, &mut self.info_open, &opened.info, &stats);
        }
        over
    }

    pub(crate) fn cycle_stream(
        &self,
        kind: media::StreamKind,
        current: Option<StreamIndex>,
        allow_none: bool,
    ) -> Option<Option<StreamIndex>> {
        let opened = self.session.as_ref()?.player.opened()?;
        let mut choices: Vec<Option<StreamIndex>> = opened
            .info
            .streams_of(kind)
            .map(|s| Some(s.index))
            .collect();
        if allow_none {
            choices.push(None);
        }
        if choices.len() < 2 && !(allow_none && choices.len() == 2) {
            return None;
        }
        let at = choices.iter().position(|c| *c == current).unwrap_or(0);
        Some(choices[(at + 1) % choices.len()])
    }

    fn update_media_controls(&mut self) {
        let now_playing = self.session.as_ref().and_then(|session| {
            session.player.opened()?;
            Some((session.file_name(), session.player.is_playing()))
        });
        self.media_controls
            .update(now_playing.as_ref().map(|(title, playing)| NowPlaying {
                title,
                playing: *playing,
            }));
    }
}

fn paint_loading(ui: &egui::Ui, area: Rect, name: &str) {
    ui.painter().text(
        area.center(),
        egui::Align2::CENTER_CENTER,
        format!("Opening {name}…"),
        egui::FontId::proportional(16.0),
        theme::TEXT_DIM,
    );
}

fn paint_audio_only(ui: &egui::Ui, area: Rect, name: &str) {
    let painter = ui.painter();
    painter.circle_filled(
        area.center() - egui::vec2(0.0, 30.0),
        54.0,
        theme::PANEL_RAISED,
    );
    crate::icons::paint(
        painter,
        Rect::from_center_size(
            area.center() - egui::vec2(0.0, 30.0),
            egui::vec2(52.0, 52.0),
        ),
        crate::icons::Icon::Volume,
        theme::ACCENT_SOFT,
    );
    painter.text(
        area.center() + egui::vec2(0.0, 52.0),
        egui::Align2::CENTER_CENTER,
        name,
        egui::FontId::proportional(17.0),
        theme::TEXT,
    );
}

fn paint_title(ui: &egui::Ui, area: Rect, name: &str) {
    let band = Rect::from_min_max(area.min, egui::pos2(area.right(), area.top() + 70.0));
    crate::widgets::vertical_fade(
        ui,
        band,
        Color32::from_black_alpha(170),
        Color32::TRANSPARENT,
    );
    ui.painter().text(
        area.left_top() + egui::vec2(22.0, 20.0),
        egui::Align2::LEFT_TOP,
        name,
        egui::FontId::proportional(15.5),
        theme::TEXT,
    );
}

/// Mouse wheel movement this frame in notches; trackpads count every 40 points as one.
fn wheel_notches(ctx: &egui::Context) -> f32 {
    ctx.input(|input| {
        input
            .events
            .iter()
            .map(|event| match event {
                egui::Event::MouseWheel { unit, delta, .. } => match unit {
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Page => delta.y * 3.0,
                    egui::MouseWheelUnit::Point => delta.y / 40.0,
                },
                _ => 0.0,
            })
            .sum()
    })
}

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
