use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Layout, RichText, Sense, TextureHandle, vec2};
use library::RecentFile;

use crate::actions::Action;
use crate::icons::{self, Icon};
use crate::theme;
use crate::widgets::{self, Kind};

/// The screen shown with no file open.
pub fn show(
    ui: &mut egui::Ui,
    recents: &[RecentFile],
    poster: &mut dyn FnMut(&Path) -> Option<TextureHandle>,
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
                    ui.horizontal(|ui| {
                        widgets::section_title(ui, "Recent");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if text_link(ui, "Clear history").clicked() {
                                actions.push(Action::ClearRecents);
                            }
                        });
                    });
                    for recent in recents.iter().take(6) {
                        let still = poster(&recent.path);
                        if recent_row(ui, recent, still.as_ref()).clicked() {
                            actions.push(Action::Open(PathBuf::from(&recent.path)));
                        }
                    }
                });
            }
        });
    });
}

const ROW: egui::Vec2 = vec2(420.0, 58.0);
const STILL: egui::Vec2 = vec2(80.0, 45.0);

fn recent_row(
    ui: &mut egui::Ui,
    recent: &RecentFile,
    still: Option<&TextureHandle>,
) -> egui::Response {
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
    let (rect, response) = ui.allocate_exact_size(ROW, Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, theme::HOVER);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let frame =
        egui::Rect::from_min_size(rect.left_top() + vec2(6.0, (ROW.y - STILL.y) / 2.0), STILL);
    paint_still(ui, frame, still);
    let text_left = frame.right() + 12.0;
    let painter = ui.painter_at(rect.with_min_x(text_left).shrink2(vec2(0.0, 2.0)));
    painter.text(
        egui::pos2(text_left, rect.top() + 10.0),
        egui::Align2::LEFT_TOP,
        name,
        egui::FontId::proportional(14.5),
        theme::TEXT,
    );
    painter.text(
        egui::pos2(text_left, rect.bottom() - 10.0),
        egui::Align2::LEFT_BOTTOM,
        folder,
        egui::FontId::proportional(12.0),
        theme::TEXT_DIM,
    );
    response.on_hover_text(recent.path.display().to_string())
}

/// A small dim text button that brightens on hover.
fn text_link(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(12.0),
        egui::Color32::PLACEHOLDER,
    );
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
    let color = if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };
    ui.painter().galley(rect.min, galley, color);
    response
}

/// The file's still, letterboxed in `frame`, or a play mark while there is none.
fn paint_still(ui: &egui::Ui, frame: egui::Rect, still: Option<&TextureHandle>) {
    let painter = ui.painter();
    painter.rect_filled(frame, 6.0, theme::PANEL_RAISED);
    let Some(still) = still else {
        let icon = egui::Rect::from_center_size(frame.center(), vec2(16.0, 16.0));
        icons::paint(painter, icon, Icon::Play, theme::accent_soft());
        return;
    };
    let [width, height] = still.size();
    let aspect = width as f32 / height.max(1) as f32;
    let fitted = crate::video_view::fit(frame, aspect);
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.add(
        egui::epaint::RectShape::filled(fitted, 6.0, egui::Color32::WHITE)
            .with_texture(still.id(), uv),
    );
}
