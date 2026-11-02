use eframe::egui::{self, RichText};
use library::Settings;

use crate::theme;
use crate::updates::Updates;

/// The small settings window. Returns whether a setting changed.
pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    settings: &mut Settings,
    updates: &mut Updates,
) -> bool {
    let mut changed = false;
    egui::Window::new("Settings")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .default_width(360.0)
        .show(ctx, |ui| {
            ui.label(RichText::new("Playback").color(theme::ACCENT_SOFT).strong());
            changed |= ui
                .checkbox(&mut settings.hardware_decoding, "Hardware decoding")
                .on_hover_text("Uses the GPU's video decoder; falls back to software automatically. Applies to the next file.")
                .changed();
            changed |= ui
                .checkbox(&mut settings.resume_playback, "Resume files where I left off")
                .changed();
            ui.horizontal(|ui| {
                ui.label("Arrow keys skip");
                changed |= ui
                    .add(egui::Slider::new(&mut settings.seek_step_seconds, 1..=30).suffix(" s"))
                    .changed();
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Updates").color(theme::ACCENT_SOFT).strong());
            changed |= ui
                .checkbox(&mut settings.updates.check_automatically, "Check for updates once a day")
                .changed();
            let button = ui.add_enabled(!updates.is_busy(), egui::Button::new("Check now"));
            if button.clicked() {
                updates.check_now();
            }
            if let Some(status) = &updates.status {
                ui.add(egui::Label::new(RichText::new(status).color(theme::TEXT_DIM)).wrap());
            }
            if settings.updates.skipped_version.is_some() && ui.small_button("Stop skipping the skipped version").clicked() {
                settings.updates.skipped_version = None;
                changed = true;
            }
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("VibePlayer {}", update::current_version()))
                        .color(theme::TEXT_DIM),
                );
                ui.hyperlink_to(
                    "Releases",
                    format!("https://github.com/{}/releases", update::REPOSITORY),
                );
                ui.label(RichText::new("· GPL-3.0").color(theme::TEXT_DIM));
            });
        });
    changed
}
