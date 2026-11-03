use std::time::Instant;

use eframe::egui::{self, RichText};
use library::{Settings, log};
use update::{Download, Release, UpdateCheck, UpdateError};

use crate::app::unix_now;
use crate::theme;

/// The update flow: background check, the "Update / Ask later / Skip" prompt, the download,
/// and launching the installer.
#[derive(Default)]
pub struct Updates {
    check: Option<(UpdateCheck, bool)>,
    /// When the last automatic check started, for the daily re-check while running.
    last_automatic: Option<Instant>,
    offer: Option<Release>,
    download: Option<Download>,
    /// A downloaded installer waiting for an export to finish, since installing closes the app.
    installer: Option<std::path::PathBuf>,
    /// Feedback for checks the user started from Settings.
    pub status: Option<String>,
}

pub enum UpdateOutcome {
    None,
    SettingsChanged,
    /// The installer is running; the app should close so it can replace files.
    InstallerLaunched,
}

impl Updates {
    /// Checks in the background at launch, so a new release is offered right away.
    pub fn start_automatic(&mut self, settings: &Settings) {
        if settings.updates.check_automatically && self.check.is_none() {
            self.last_automatic = Some(Instant::now());
            self.check = Some((UpdateCheck::start(), false));
        }
    }

    /// Checks again once a day for people who leave the player open, and as soon as automatic
    /// checks are turned on if they were off at launch.
    pub fn tick(&mut self, settings: &Settings) {
        let due = self
            .last_automatic
            .is_none_or(|last| last.elapsed() >= update::RECHECK_INTERVAL);
        if due && self.offer.is_none() && self.download.is_none() {
            self.start_automatic(settings);
        }
    }

    pub fn check_now(&mut self) {
        if self.check.is_none() {
            self.status = Some("Checking…".into());
            self.check = Some((UpdateCheck::start(), true));
        }
    }

    /// How far the installer download has come, while one runs.
    pub fn download_progress(&self) -> Option<f32> {
        self.download.as_ref().map(fraction_of)
    }

    pub fn is_busy(&self) -> bool {
        self.check.is_some() || self.download.is_some()
    }

    /// `exporting` holds back the installer, which closes the app, until the export is done.
    pub fn poll(&mut self, settings: &mut Settings, exporting: bool) -> UpdateOutcome {
        let mut outcome = UpdateOutcome::None;
        if let Some((check, manual)) = &self.check
            && let Some(result) = check.poll()
        {
            let manual = *manual;
            self.check = None;
            self.status = None;
            match result {
                Ok(release) => {
                    log::info(format!(
                        "update check: latest release is {}",
                        release.version
                    ));
                    settings.updates.last_check = Some(unix_now());
                    outcome = UpdateOutcome::SettingsChanged;
                    let skipped = if manual {
                        None
                    } else {
                        settings.updates.skipped_version.as_deref()
                    };
                    if update::should_offer(&update::current_version(), &release.version, skipped) {
                        self.offer = Some(release);
                    } else if manual {
                        self.status = Some("You're on the latest version.".into());
                    }
                }
                Err(error) => {
                    log::warn(format!("update check failed: {error}"));
                    if manual {
                        self.status = Some(describe(&error));
                    }
                }
            }
        }
        if let Some(download) = &mut self.download
            && let Some(result) = download.poll()
        {
            self.download = None;
            match result {
                Ok(installer) => self.installer = Some(installer),
                Err(error) => {
                    log::error(format!("update failed: {error}"));
                    self.status = Some(describe(&error));
                }
            }
        }
        if exporting {
            if self.installer.is_some() {
                self.status = Some("The update installs when the export finishes.".into());
            }
            return outcome;
        }
        if let Some(installer) = self.installer.take() {
            match launch_installer(installer) {
                Ok(()) => {
                    log::info("update downloaded; starting the installer");
                    return UpdateOutcome::InstallerLaunched;
                }
                Err(error) => {
                    log::error(format!("update failed: {error}"));
                    self.status = Some(describe(&error));
                }
            }
        }
        outcome
    }

    /// The prompt card, while there's an update to offer. Returns true if settings changed.
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings) -> bool {
        let Some(release) = self.offer.clone() else {
            return false;
        };
        let mut changed = false;
        crate::widgets::window("Update available")
            .resizable(false)
            .title_bar(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-20.0, -124.0))
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("VibePlayer {} is available", release.version))
                        .strong()
                        .size(16.0),
                );
                ui.label(
                    RichText::new(format!("You have {}.", update::current_version()))
                        .color(theme::TEXT_DIM)
                        .small(),
                );
                if !release.notes.trim().is_empty() {
                    egui::CollapsingHeader::new("What's new").show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(140.0)
                            .show(ui, |ui| {
                                ui.label(release.notes.trim());
                            });
                    });
                }
                ui.add_space(6.0);
                if let Some(download) = &self.download {
                    let fraction = fraction_of(download);
                    ui.add(egui::ProgressBar::new(fraction).text("Downloading…"));
                    if crate::widgets::button(ui, crate::widgets::Kind::Secondary, None, "Cancel")
                        .clicked()
                    {
                        download.cancel();
                    }
                    ctx.request_repaint();
                    return;
                }
                if let Some(status) = &self.status {
                    ui.label(RichText::new(status).color(theme::DANGER));
                }
                ui.horizontal(|ui| {
                    use crate::widgets::{Kind, button};
                    if button(ui, Kind::Primary, None, "Update").clicked() {
                        match release.installer.clone() {
                            Some(asset) if cfg!(windows) => {
                                self.status = None;
                                self.download = Some(Download::start(asset));
                            }
                            _ => ctx.open_url(egui::OpenUrl::new_tab(&release.page)),
                        }
                    }
                    if button(ui, Kind::Secondary, None, "Ask later").clicked() {
                        self.offer = None;
                    }
                    if button(ui, Kind::Ghost, None, "Skip this version").clicked() {
                        settings.updates.skipped_version = Some(release.version.to_string());
                        self.offer = None;
                        changed = true;
                    }
                });
            });
        changed
    }
}

fn fraction_of(download: &Download) -> f32 {
    let (received, total) = download.progress();
    total.map_or(0.0, |total| received as f32 / total.max(1) as f32)
}

fn describe(error: &UpdateError) -> String {
    match error {
        UpdateError::Network(message) if message.contains("404") => {
            "No releases have been published yet.".into()
        }
        other => format!("Update failed: {other}"),
    }
}

/// Starts the installer quietly; it closes and restarts VibePlayer itself.
fn launch_installer(path: std::path::PathBuf) -> Result<(), UpdateError> {
    std::process::Command::new(path)
        .args([
            "/SILENT",
            "/SP-",
            "/NOCANCEL",
            "/CLOSEAPPLICATIONS",
            "/RESTARTAPPLICATIONS",
        ])
        .spawn()
        .map(drop)
        .map_err(UpdateError::Io)
}
