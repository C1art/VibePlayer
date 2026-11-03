use std::path::PathBuf;
use std::rc::Rc;

use crossbeam::channel::Receiver;
use std::time::{Duration, Instant};

use cut::{StreamIndex, Ticks};
use eframe::egui::{self, Color32, CursorIcon, Rect, Sense};
use library::{History, Saver, Settings, Store, log};
use media::{HardwareDecoding, Speed, VideoOptions};
use playback::{LoadState, PlayerOptions};

use crate::actions::Action;
use crate::controls::{self, ControlsState};
use crate::editor::{self, Editor};
use crate::info_panel::{self, PlaybackStats};
use crate::os::{MediaControls, NowPlaying, PendingDialog, Taskbar, TaskbarState};
use crate::osd::Osd;
use crate::session::Session;
use crate::subtitles::SubtitleOverlay;
use crate::window_state::WindowTracker;
use crate::{chapters, diagnostics, shortcuts, theme, video_view, welcome};

/// Controls hide after the pointer rests this long during playback.
const CONTROLS_IDLE: Duration = Duration::from_millis(2200);
/// Keyframe seeks while dragging the seek bar are sent at most this often.
const SCRUB_INTERVAL: Duration = Duration::from_millis(60);

pub struct VibePlayerApp {
    pub(crate) saver: Option<Saver>,
    window: WindowTracker,
    native_window: Option<crate::os::NativeWindow>,
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
    render_state: Option<eframe::egui_wgpu::RenderState>,
    pub(crate) last_activity: Instant,
    pub(crate) last_scrub: Option<Instant>,
    pending_scrub: Option<Action>,
    pub(crate) scroll_accumulator: f32,
    pub(crate) subtitles: SubtitleOverlay,
    pub(crate) open_dialog: Option<PendingDialog>,
    pub(crate) media_controls: MediaControls,
    taskbar: Taskbar,
    /// How decoded pictures can stay on the renderer's GPU, where supported.
    gpu_sharing: Option<media::GpuSharing>,
    pub(crate) fullscreen: bool,
    pub(crate) editor: Option<Editor>,
    /// Files sent by later launches of the app.
    pub(crate) instance_requests: Option<Receiver<PathBuf>>,
}

impl VibePlayerApp {
    pub fn new(
        creation: &eframe::CreationContext<'_>,
        store: Option<Store>,
        settings: Settings,
        initial_file: Option<PathBuf>,
        instance_requests: Option<Receiver<PathBuf>>,
    ) -> VibePlayerApp {
        theme::apply(&creation.egui_ctx, settings.theme);
        let native_window = crate::os::NativeWindow::of(creation);
        crate::window_state::place(native_window, settings.window, &creation.egui_ctx);
        let window = WindowTracker::new(native_window, settings.window);
        if let Some(native_window) = native_window {
            // The taskbar shows the window's large icon, which only the exe's resources give.
            crate::os::show_app_icon(native_window, settings.theme);
        }
        let history = store.as_ref().map(Store::load_history).unwrap_or_default();
        let high_bit_depth = creation.wgpu_render_state.as_ref().map(video_view::install);
        diagnostics::log_startup(creation, high_bit_depth);
        let mut app = VibePlayerApp {
            saver: store.map(Saver::spawn),
            settings,
            window,
            native_window,
            history,
            session: None,
            error: None,
            osd: Osd::default(),
            info_open: false,
            help_open: false,
            settings_open: false,
            updates: crate::updates::Updates::default(),
            high_bit_depth,
            render_state: creation.wgpu_render_state.clone(),
            last_activity: Instant::now(),
            last_scrub: None,
            pending_scrub: None,
            scroll_accumulator: 0.0,
            subtitles: SubtitleOverlay::default(),
            open_dialog: None,
            media_controls: MediaControls::new(creation),
            taskbar: Taskbar::new(&creation.egui_ctx),
            gpu_sharing: creation
                .wgpu_render_state
                .as_ref()
                .and_then(|state| render::gpu_sharing(&state.device)),
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
                deinterlace: true,
                gpu: self.gpu_sharing.filter(|_| self.settings.gpu_frames),
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
        // The last picture may hold GPU textures shared with the decoder; let them go now.
        if let Some(render_state) = &self.render_state {
            video_view::clear(render_state);
        }
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

    /// Recolors the interface and the app icon: the window's, and on Windows the icon the
    /// shell shows for shortcuts and VibePlayer's videos.
    fn apply_theme(&self, ctx: &egui::Context) {
        let theme = self.settings.theme;
        theme::apply(ctx, theme);
        ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(theme::icon(theme))));
        if let Some(window) = self.native_window {
            crate::os::show_app_icon(window, theme);
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
        if let Some(render_state) = frame.wgpu_render_state() {
            video_view::follow_display(render_state);
        }
        let update = session.player.update(now);
        session.log.observe(&session.player);
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
                log::error(format!(
                    "couldn't open {}: {error}",
                    session.player.path().display()
                ));
                self.error = Some(format!("Couldn't open {}: {error}", session.file_name()));
                self.session = None;
                if let Some(render_state) = frame.wgpu_render_state() {
                    video_view::clear(render_state);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Title("VibePlayer".into()));
                return;
            }
            LoadState::Ready(opened) if !session.title_shown => {
                session.title_shown = true;
                log::info(format!("opened {}", diagnostics::describe(&opened.info)));
                session.chapters = chapters::marks(&opened.info).into();
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

    /// Lets through at most one scrub (keyframe seek) per interval. The latest one held back
    /// is sent once the interval is up, so the picture matches where the pointer stopped.
    fn handle_scrub_throttle(
        &mut self,
        ctx: &egui::Context,
        actions: &mut Vec<Action>,
        now: Instant,
    ) {
        let mut latest = None;
        actions.retain(|action| {
            let scrub = is_scrub(action);
            if scrub {
                latest = Some(action.clone());
            } else if moves_playhead(action) {
                // A committed seek supersedes a held-back scrub.
                self.pending_scrub = None;
            }
            !scrub
        });
        if latest.is_some() {
            self.pending_scrub = latest;
        }
        if self.pending_scrub.is_none() {
            return;
        }
        let wait = self.last_scrub.map_or(Duration::ZERO, |last| {
            SCRUB_INTERVAL.saturating_sub(now - last)
        });
        if wait.is_zero() {
            self.last_scrub = Some(now);
            actions.extend(self.pending_scrub.take());
        } else {
            ctx.request_repaint_after(wait);
        }
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
        actions.extend(self.taskbar.poll());
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
        self.window.observe(&ctx);
        actions.extend(shortcuts::read(
            &ctx,
            i64::from(self.settings.seek_step_seconds),
        ));
        self.poll_session(&ctx, frame, now);
        self.end_preview();
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
            let theme = self.settings.theme;
            settings_changed |= crate::settings_window::show(
                &ctx,
                &mut self.settings_open,
                &mut self.settings,
                &mut self.updates,
            );
            if self.settings.theme != theme {
                self.apply_theme(&ctx);
            }
        }
        self.updates.tick(&self.settings);
        let exporting = self.editor.as_ref().is_some_and(Editor::is_exporting);
        match self.updates.poll(&mut self.settings, exporting) {
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
            && self.editor.is_none()
            && !self.help_open
            && !self.settings_open
            && !self.controls_visible(&ctx, over_controls);
        if hide_cursor {
            ctx.set_cursor_icon(CursorIcon::None);
        }
        self.handle_scrub_throttle(&ctx, &mut actions, now);
        for action in actions {
            // Any seek ends a selection preview (starting one sets it again).
            if moves_playhead(&action)
                && let Some(editor) = &mut self.editor
            {
                editor.previewing = false;
            }
            self.apply(&ctx, action, now);
        }
        self.update_media_controls();
        if self.session.as_ref().is_some_and(|s| s.player.is_playing()) {
            ctx.request_repaint_after(CONTROLS_IDLE);
        }
    }

    fn on_exit(&mut self) {
        // Dropping the editor cancels a running export and removes its temp file.
        self.close_file();
        self.settings.window = self.window.placement();
        if let Some(saver) = &self.saver {
            saver.save_settings(&self.settings);
            saver.save_history(&self.history);
            saver.flush();
        }
        log::info("closed");
        log::flush(Duration::from_secs(1));
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
        // Only the picture takes clicks: blank parts of the panels below must not toggle play.
        let panel_height = if self.editor.is_some() {
            editor::HEIGHT
        } else if visible {
            controls::HEIGHT
        } else {
            0.0
        };
        let clickable = Rect::from_min_max(
            area.min,
            egui::pos2(area.right(), (area.bottom() - panel_height).max(area.top())),
        );
        let response = ui.interact(clickable, egui::Id::unique("video area"), Sense::click());
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
                seek_step_seconds: self.settings.seek_step_seconds,
                info: &opened.info,
                audio_stream: opened.audio,
                subtitle_stream: opened.subtitle,
                highlight: None,
                chapters: Rc::clone(&session.chapters),
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
        let export = self
            .editor
            .as_ref()
            .filter(|editor| editor.is_exporting())
            .map(Editor::export_fraction);
        self.taskbar.update(TaskbarState {
            playing: now_playing.map(|(_, playing)| playing),
            progress: export.or(self.updates.download_progress()),
        });
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
        theme::accent_soft(),
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
fn is_scrub(action: &Action) -> bool {
    matches!(
        action,
        Action::SeekTo {
            mode: playback::SeekMode::Keyframe,
            ..
        } | Action::Editor(editor::EditorAction::Seek(_, playback::SeekMode::Keyframe))
    )
}

fn moves_playhead(action: &Action) -> bool {
    use editor::EditorAction;
    matches!(
        action,
        Action::SeekTo { .. }
            | Action::SeekBy { .. }
            | Action::SeekToTenth(_)
            | Action::NextChapter
            | Action::PreviousChapter
            | Action::StepBack
            | Action::StepForward
            | Action::Editor(
                EditorAction::Seek(..)
                    | EditorAction::PreviewSelection
                    | EditorAction::StepBack
                    | EditorAction::StepForward
            )
    )
}

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
