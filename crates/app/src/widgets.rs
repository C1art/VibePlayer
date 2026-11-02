//! The app's shared widgets, so every screen uses the same buttons, toggles, sliders and cards.

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Margin, Rect, Response, RichText, Sense, Stroke, pos2,
    vec2,
};

use crate::icons::{self, Icon};
use crate::theme;

pub const CONTROL_HEIGHT: f32 = 32.0;
const RADIUS: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The one main action on a screen.
    Primary,
    Secondary,
    /// Text-only until hovered.
    Ghost,
}

/// A text button with an optional leading icon, in one of the three styles.
pub fn button(ui: &mut egui::Ui, kind: Kind, icon: Option<Icon>, text: &str) -> Response {
    button_enabled(ui, kind, icon, text, true)
}

pub fn button_enabled(
    ui: &mut egui::Ui,
    kind: Kind,
    icon: Option<Icon>,
    text: &str,
    enabled: bool,
) -> Response {
    let font = FontId::proportional(14.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, theme::TEXT);
    let icon_space = if icon.is_some() { 22.0 } else { 0.0 };
    let width = galley.size().x + icon_space + 28.0;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), sense);
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let (fill, stroke, text_color) = match kind {
        Kind::Primary => (
            if pressed {
                theme::ACCENT.gamma_multiply(0.8)
            } else if hovered {
                theme::ACCENT_SOFT
            } else {
                theme::ACCENT
            },
            Stroke::NONE,
            Color32::WHITE,
        ),
        Kind::Secondary => (
            if hovered {
                theme::HOVER
            } else {
                theme::PANEL_RAISED
            },
            Stroke::new(1.0, theme::BORDER),
            theme::TEXT,
        ),
        Kind::Ghost => (
            if hovered {
                theme::HOVER
            } else {
                Color32::TRANSPARENT
            },
            Stroke::NONE,
            theme::TEXT,
        ),
    };
    let alpha = if enabled { 1.0 } else { 0.4 };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(RADIUS),
        fill.gamma_multiply(alpha),
        stroke,
        egui::StrokeKind::Inside,
    );
    let mut x = rect.left() + 14.0;
    if let Some(icon) = icon {
        let icon_rect = Rect::from_center_size(pos2(x + 8.0, rect.center().y), vec2(16.0, 16.0));
        icons::paint(painter, icon_rect, icon, text_color.gamma_multiply(alpha));
        x += icon_space;
    }
    painter.galley(
        pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color.gamma_multiply(alpha),
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A round, borderless icon button.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, size: f32, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let color = if response.hovered() {
        ui.painter()
            .circle_filled(rect.center(), size / 2.0, Color32::from_white_alpha(28));
        theme::TEXT
    } else {
        theme::TEXT.gamma_multiply(0.9)
    };
    icons::paint(ui.painter(), rect.shrink(size * 0.24), icon, color);
    response.on_hover_text(tooltip)
}

/// A compact pill, e.g. the speed selector.
pub fn pill_button(ui: &mut egui::Ui, text: &str, tooltip: &str) -> Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), FontId::proportional(13.5), theme::TEXT);
    let size = vec2(galley.size().x + 20.0, 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() {
        Color32::from_white_alpha(40)
    } else {
        Color32::from_white_alpha(20)
    };
    ui.painter().rect_filled(rect, 14.0, fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, theme::TEXT);
    response.on_hover_text(tooltip)
}

/// A switch with its label. Returns a response that reports `changed()`.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str) -> Response {
    toggle_enabled(ui, on, label, true)
}

pub fn toggle_enabled(ui: &mut egui::Ui, on: &mut bool, label: &str, enabled: bool) -> Response {
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), FontId::proportional(14.0), theme::TEXT);
    let switch = vec2(34.0, 18.0);
    let size = vec2(switch.x + 10.0 + galley.size().x, CONTROL_HEIGHT.min(26.0));
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, mut response) = ui.allocate_exact_size(size, sense);
    if enabled && response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let alpha = if enabled { 1.0 } else { 0.4 };
    let how_on = ui.ctx().animate_bool(response.id, *on);
    let track = Rect::from_min_size(pos2(rect.left(), rect.center().y - switch.y / 2.0), switch);
    let fill = theme::BORDER
        .lerp_to_gamma(theme::ACCENT, how_on)
        .gamma_multiply(alpha);
    let painter = ui.painter();
    painter.rect_filled(track, 9.0, fill);
    let knob_x = egui::lerp(track.left() + 9.0..=track.right() - 9.0, how_on);
    painter.circle_filled(
        pos2(knob_x, track.center().y),
        7.0,
        Color32::WHITE.gamma_multiply(alpha),
    );
    painter.galley(
        pos2(
            track.right() + 10.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        theme::TEXT.gamma_multiply(alpha),
    );
    response
}

/// A thin horizontal slider for 0..=1 values. Returns the new value while dragged or clicked.
pub fn thin_slider(ui: &mut egui::Ui, value: f32, width: f32) -> Option<f32> {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click_and_drag());
    let track = Rect::from_center_size(rect.center(), vec2(width, 4.0));
    let painter = ui.painter();
    painter.rect_filled(track, 2.0, theme::TRACK);
    let filled = Rect::from_min_max(
        track.min,
        pos2(
            track.left() + track.width() * value.clamp(0.0, 1.0),
            track.max.y,
        ),
    );
    painter.rect_filled(filled, 2.0, theme::TEXT);
    let knob = if response.hovered() || response.dragged() {
        6.5
    } else {
        5.0
    };
    painter.circle_filled(pos2(filled.right(), track.center().y), knob, theme::TEXT);
    let pointer = response.interact_pointer_pos()?;
    (response.dragged() || response.clicked())
        .then(|| ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0))
}

/// A centered slider for a signed value in `-limit..=limit` (e.g. dB), with its label and
/// formatted value. Double-click resets to zero. Returns the new value when it changed.
pub fn balance_slider(
    ui: &mut egui::Ui,
    label: &str,
    value: f32,
    limit: f32,
    format: impl Fn(f32) -> String,
) -> Option<f32> {
    let mut result = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).color(theme::TEXT_DIM).size(12.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format(value)).monospace().size(12.0));
            });
        });
        let width = ui.available_width().min(170.0);
        let (rect, response) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click_and_drag());
        let track = Rect::from_center_size(rect.center(), vec2(width, 4.0));
        let to_x =
            |v: f32| track.left() + track.width() * ((v / limit + 1.0) / 2.0).clamp(0.0, 1.0);
        let painter = ui.painter();
        painter.rect_filled(track, 2.0, theme::TRACK);
        let (from, to) = (to_x(0.0).min(to_x(value)), to_x(0.0).max(to_x(value)));
        painter.rect_filled(
            Rect::from_x_y_ranges(from..=to, track.y_range()),
            2.0,
            theme::ACCENT,
        );
        painter.line_segment(
            [
                pos2(to_x(0.0), track.top() - 4.0),
                pos2(to_x(0.0), track.bottom() + 4.0),
            ],
            Stroke::new(1.0, theme::TEXT_DIM),
        );
        let knob = if response.hovered() || response.dragged() {
            7.0
        } else {
            6.0
        };
        painter.circle_filled(pos2(to_x(value), track.center().y), knob, theme::TEXT);
        if response.double_clicked() {
            result = Some(0.0);
        } else if let Some(pointer) = response.interact_pointer_pos()
            && (response.dragged() || response.clicked())
        {
            let fraction = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
            result = Some((fraction * 2.0 - 1.0) * limit);
        }
        response.on_hover_text("Double-click to reset");
    });
    result
}

/// A small rounded value display, e.g. a timestamp.
pub fn chip(ui: &mut egui::Ui, text: &str) -> Response {
    egui::Frame::new()
        .fill(theme::PANEL_RAISED)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .monospace()
                    .size(13.0)
                    .color(theme::TEXT),
            )
        })
        .inner
}

/// A small colored label, e.g. "Lossless".
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.18))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(color).strong());
        });
}

/// A dim, small heading for a group of controls.
pub fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.5)
            .color(theme::TEXT_DIM)
            .strong(),
    );
}

/// The frame every floating window uses.
pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(18))
        .shadow(egui::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(140),
        })
}

/// A floating window with the shared look and a close button.
pub fn window(title: &'static str) -> egui::Window<'static> {
    egui::Window::new(RichText::new(title).size(16.0).strong())
        .frame(card_frame())
        .collapsible(false)
}

/// Paints a vertical fade from `top` to `bottom` over `rect`.
pub fn vertical_fade(ui: &egui::Ui, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}
