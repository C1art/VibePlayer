use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

pub const ACCENT: Color32 = Color32::from_rgb(124, 92, 255);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(160, 140, 255);
pub const BACKGROUND: Color32 = Color32::from_rgb(10, 10, 12);
pub const PANEL: Color32 = Color32::from_rgb(24, 24, 28);
pub const PANEL_RAISED: Color32 = Color32::from_rgb(34, 34, 40);
pub const TEXT: Color32 = Color32::from_rgb(236, 236, 240);
pub const TEXT_DIM: Color32 = Color32::from_rgb(160, 160, 172);
pub const HOVER: Color32 = Color32::from_rgb(48, 48, 58);
pub const BORDER: Color32 = Color32::from_rgb(52, 52, 62);
pub const TRACK: Color32 = Color32::from_rgba_premultiplied(70, 70, 80, 200);
pub const DANGER: Color32 = Color32::from_rgb(255, 99, 99);
pub const SMART: Color32 = Color32::from_rgb(77, 182, 255);
pub const SUCCESS: Color32 = Color32::from_rgb(88, 214, 141);
pub const KEYFRAME: Color32 = Color32::from_rgb(255, 196, 61);

pub fn apply(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = Color32::from_rgb(16, 16, 20);
    visuals.faint_bg_color = PANEL_RAISED;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, TEXT);
    visuals.hyperlink_color = ACCENT_SOFT;
    visuals.override_text_color = Some(TEXT);
    for widgets in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
        &mut visuals.widgets.noninteractive,
    ] {
        widgets.corner_radius = CornerRadius::same(8);
    }
    visuals.widgets.inactive.weak_bg_fill = PANEL_RAISED;
    visuals.widgets.inactive.bg_fill = PANEL_RAISED;
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(48, 48, 58);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 48, 58);
    visuals.widgets.active.weak_bg_fill = ACCENT;
    // Window title bars use this fill; matching the panel makes each window one card.
    visuals.widgets.open.weak_bg_fill = PANEL;
    visuals.widgets.active.bg_fill = ACCENT;
    ctx.set_visuals(visuals);

    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.slider_rail_height = 4.0;
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.5, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(20.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.5, FontFamily::Monospace),
            ),
        ]
        .into();
    });
}
