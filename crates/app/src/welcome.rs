use std::path::PathBuf;

use eframe::egui::{self, Align, Layout, RichText};
use library::RecentFile;

use crate::actions::Action;
use crate::icons::{self, Icon};
use crate::theme;

/// The screen shown with no file open.
pub fn show(
    ui: &mut egui::Ui,
    recents: &[RecentFile],
    error: Option<&str>,
    actions: &mut Vec<Action>,
) {
    let area = ui.max_rect();
    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add_space((area.height() * 0.22).max(24.0));
            ui.label(
                RichText::new("VibePlayer")
                    .size(34.0)
                    .strong()
                    .color(theme::TEXT),
            );
            ui.label(
                RichText::new("Drop a video here, or open one to start.")
                    .size(15.0)
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(18.0);
            let button = egui::Button::new(RichText::new("   Open file").size(16.0))
                .fill(theme::ACCENT)
                .corner_radius(10.0)
                .min_size(egui::vec2(160.0, 40.0));
            let response = ui.add(button);
            let icon_rect = egui::Rect::from_center_size(
                response.rect.left_center() + egui::vec2(28.0, 0.0),
                egui::vec2(16.0, 16.0),
            );
            icons::paint(ui.painter(), icon_rect, Icon::Folder, theme::TEXT);
            if response.clicked() {
                actions.push(Action::ShowOpenDialog);
            }
            ui.add_space(6.0);
            let help = ui.add(
                egui::Button::new(
                    RichText::new("Keyboard shortcuts  (F1)")
                        .small()
                        .color(theme::TEXT_DIM),
                )
                .frame(false),
            );
            if help.clicked() {
                actions.push(Action::ToggleHelp);
            }
            if let Some(error) = error {
                ui.add_space(12.0);
                ui.label(RichText::new(error).color(theme::DANGER));
            }
            let existing: Vec<&RecentFile> = recents.iter().take(8).collect();
            if !existing.is_empty() {
                ui.add_space(36.0);
                ui.label(RichText::new("Recent").color(theme::TEXT_DIM).small());
                ui.add_space(4.0);
                for recent in existing {
                    let name = recent
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| recent.path.display().to_string());
                    let link = ui
                        .add(egui::Button::new(RichText::new(name).size(14.0)).frame(false))
                        .on_hover_text(recent.path.display().to_string());
                    if link.clicked() {
                        actions.push(Action::Open(PathBuf::from(&recent.path)));
                    }
                }
            }
        });
    });
}
