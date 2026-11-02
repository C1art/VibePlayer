use eframe::egui::{self, Color32, Response, Sense, vec2};

use crate::icons::{self, Icon};
use crate::theme;

/// A round, borderless icon button.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, size: f32, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let color = if response.hovered() {
        ui.painter()
            .circle_filled(rect.center(), size / 2.0, Color32::from_white_alpha(28));
        theme::TEXT
    } else {
        theme::TEXT.gamma_multiply(0.92)
    };
    icons::paint(ui.painter(), rect.shrink(size * 0.24), icon, color);
    response.on_hover_text(tooltip)
}

/// A text pill button, e.g. the speed selector.
pub fn pill_button(ui: &mut egui::Ui, text: &str, tooltip: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(13.5),
        theme::TEXT,
    );
    let size = vec2(galley.size().x + 18.0, 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() {
        Color32::from_white_alpha(36)
    } else {
        Color32::from_white_alpha(18)
    };
    ui.painter().rect_filled(rect, 14.0, fill);
    let text_pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(text_pos, galley, theme::TEXT);
    response.on_hover_text(tooltip)
}

/// A thin horizontal slider for 0..=1 values. Returns the new value while dragged or clicked.
pub fn thin_slider(ui: &mut egui::Ui, value: f32, width: f32) -> Option<f32> {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click_and_drag());
    let track = egui::Rect::from_center_size(rect.center(), vec2(width, 4.0));
    let painter = ui.painter();
    painter.rect_filled(track, 2.0, theme::TRACK);
    let filled = egui::Rect::from_min_max(
        track.min,
        egui::pos2(
            track.left() + track.width() * value.clamp(0.0, 1.0),
            track.max.y,
        ),
    );
    painter.rect_filled(filled, 2.0, theme::TEXT);
    let knob_radius = if response.hovered() || response.dragged() {
        6.0
    } else {
        4.5
    };
    painter.circle_filled(
        egui::pos2(filled.right(), track.center().y),
        knob_radius,
        theme::TEXT,
    );
    let pointer = response.interact_pointer_pos()?;
    if response.dragged() || response.clicked() {
        Some(((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0))
    } else {
        None
    }
}

/// Paints a vertical fade from transparent to `bottom` over `rect`.
pub fn vertical_fade(ui: &egui::Ui, rect: egui::Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}
