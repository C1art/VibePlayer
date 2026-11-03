use eframe::egui::{self, RichText};
use library::Settings;

use crate::theme;
use crate::updates::Updates;
use crate::widgets::{self, Kind, section_title};

/// The small settings window. Returns whether a setting changed.
pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    settings: &mut Settings,
    updates: &mut Updates,
) -> bool {
    let mut changed = false;
    widgets::window("Settings")
        .open(open)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .default_width(380.0)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            section_title(ui, "Playback");
            changed |= widgets::toggle(ui, &mut settings.hardware_decoding, "Hardware decoding")
                .on_hover_text(
                    "Uses the GPU's video decoder and falls back to software automatically. \
                     Applies to the next file.",
                )
                .changed();
            if cfg!(windows) {
                ui.add_enabled_ui(settings.hardware_decoding, |ui| {
                    changed |= widgets::toggle(
                        ui,
                        &mut settings.gpu_frames,
                        "Keep decoded video on the GPU",
                    )
                    .on_hover_text(
                        "Shows hardware-decoded frames without copying them through system \
                         memory: less CPU and memory use, most of all at 4K. Applies to the \
                         next file.",
                    )
                    .changed();
                });
            }
            changed |= widgets::toggle(
                ui,
                &mut settings.resume_playback,
                "Resume files where I left off",
            )
            .changed();
            ui.horizontal(|ui| {
                ui.label("Arrow keys skip");
                let mut step = settings.seek_step_seconds as f32 / 30.0;
                if let Some(value) = widgets::thin_slider(ui, step, 140.0) {
                    step = value;
                    let seconds = ((step * 30.0).round() as u32).clamp(1, 30);
                    if seconds != settings.seek_step_seconds {
                        settings.seek_step_seconds = seconds;
                        changed = true;
                    }
                }
                widgets::chip(ui, &format!("{}s", settings.seek_step_seconds));
            });

            ui.add_space(6.0);
            section_title(ui, "Trim editor");
            changed |= widgets::toggle(
                ui,
                &mut settings.editor.snap_to_keyframes,
                "Snap start to keyframes",
            )
            .changed();
            changed |= widgets::toggle(
                ui,
                &mut settings.editor.smart_cut,
                "Frame-accurate start (smart cut)",
            )
            .on_hover_text("Re-encodes only the frames up to the next keyframe; the rest is copied")
            .changed();

            ui.add_space(6.0);
            section_title(ui, "Updates");
            changed |= widgets::toggle(
                ui,
                &mut settings.updates.check_automatically,
                "Check for updates automatically",
            )
            .changed();
            ui.horizontal(|ui| {
                let check = widgets::button_enabled(
                    ui,
                    Kind::Secondary,
                    None,
                    "Check now",
                    !updates.is_busy(),
                );
                if check.clicked() {
                    updates.check_now();
                }
                if settings.updates.skipped_version.is_some()
                    && widgets::button(ui, Kind::Ghost, None, "Stop skipping").clicked()
                {
                    settings.updates.skipped_version = None;
                    changed = true;
                }
            });
            if let Some(status) = &updates.status {
                ui.add(egui::Label::new(RichText::new(status).color(theme::TEXT_DIM)).wrap());
            }

            ui.add_space(4.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("VibePlayer {}", update::current_version()))
                        .color(theme::TEXT_DIM),
                );
                ui.label(RichText::new("·").color(theme::TEXT_DIM));
                ui.label(RichText::new("GPL-3.0").color(theme::TEXT_DIM));
                ui.label(RichText::new("·").color(theme::TEXT_DIM));
                ui.hyperlink_to(
                    "Releases",
                    format!("https://github.com/{}/releases", update::REPOSITORY),
                );
                if let Some(log) = library::log::path() {
                    ui.label(RichText::new("·").color(theme::TEXT_DIM));
                    if ui
                        .link("Log file")
                        .on_hover_text("Shows the diagnostic log, for reporting a problem")
                        .clicked()
                    {
                        crate::os::show_in_folder(log);
                    }
                }
            });
        });
    changed
}
