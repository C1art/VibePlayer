use std::path::PathBuf;

use eframe::egui::{self, Align, Layout, RichText, Sense, vec2};
use library::RecentFile;

use crate::actions::Action;
use crate::icons::{self, Icon};
use crate::theme;
use crate::widgets::{self, Kind};

/// The screen shown with no file open.
pub fn show(
    ui: &mut egui::Ui,
    recents: &[RecentFile],
    error: Option<&str>,
    actions: &mut Vec<Action>,
) {
    let area = ui.max_rect();
    let gear = egui::Rect::from_min_size(area.right_top() + vec2(-52.0, 14.0), vec2(36.0, 36.0));
    let mut corner = ui.new_child(egui::UiBuilder::new().max_rect(gear));
    if widgets::icon_button(&mut corner, Icon::Settings, 36.0, "Settings (Ctrl + ,)").clicked() {
        actions.push(Action::ToggleSettings);
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add_space((area.height() * 0.2).max(24.0));
            let (logo, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
            let icon = theme::icon_texture(ui.ctx());
            ui.painter().image(
                icon.id(),
                logo,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            ui.add_space(10.0);
            ui.label(RichText::new("VibePlayer").size(30.0).strong());
            ui.label(
                RichText::new("Drop a video here, or open one to start.")
                    .size(15.0)
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                let primary = 120.0 + 150.0;
                ui.add_space((ui.available_width() - primary) / 2.0);
                if widgets::button(ui, Kind::Primary, Some(Icon::Folder), "Open file").clicked() {
                    actions.push(Action::ShowOpenDialog);
                }
                if widgets::button(ui, Kind::Ghost, None, "Shortcuts  F1").clicked() {
                    actions.push(Action::ToggleHelp);
                }
            });
            if let Some(error) = error {
                ui.add_space(12.0);
                ui.label(RichText::new(error).color(theme::DANGER));
            }
            if !recents.is_empty() {
                ui.add_space(30.0);
                ui.allocate_ui_with_layout(vec2(420.0, 0.0), Layout::top_down(Align::Min), |ui| {
                    widgets::section_title(ui, "Recent");
                    for recent in recents.iter().take(6) {
                        if recent_row(ui, recent).clicked() {
                            actions.push(Action::Open(PathBuf::from(&recent.path)));
                        }
                    }
                });
            }
        });
    });
}

fn recent_row(ui: &mut egui::Ui, recent: &RecentFile) -> egui::Response {
    let name = recent
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| recent.path.display().to_string());
    let folder = recent
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let (rect, response) = ui.allocate_exact_size(vec2(420.0, 44.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, theme::HOVER);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let icon = egui::Rect::from_center_size(rect.left_center() + vec2(20.0, 0.0), vec2(16.0, 16.0));
    icons::paint(ui.painter(), icon, Icon::Play, theme::accent_soft());
    let painter = ui.painter_at(rect.shrink2(vec2(40.0, 0.0)).with_min_x(rect.left() + 40.0));
    painter.text(
        rect.left_top() + vec2(40.0, 6.0),
        egui::Align2::LEFT_TOP,
        name,
        egui::FontId::proportional(14.5),
        theme::TEXT,
    );
    painter.text(
        rect.left_bottom() + vec2(40.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        folder,
        egui::FontId::proportional(12.0),
        theme::TEXT_DIM,
    );
    response.on_hover_text(recent.path.display().to_string())
}
