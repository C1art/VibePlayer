use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};
use library::ColorTheme;

pub const BACKGROUND: Color32 = Color32::from_rgb(10, 10, 12);
pub const PANEL: Color32 = Color32::from_rgb(24, 24, 28);
pub const PANEL_RAISED: Color32 = Color32::from_rgb(34, 34, 40);
pub const TEXT: Color32 = Color32::from_rgb(236, 236, 240);
pub const TEXT_DIM: Color32 = Color32::from_rgb(160, 160, 172);
pub const HOVER: Color32 = Color32::from_rgb(48, 48, 58);
pub const BORDER: Color32 = Color32::from_rgb(52, 52, 62);
pub const TRACK: Color32 = Color32::from_rgba_premultiplied(70, 70, 80, 200);
pub const DANGER: Color32 = Color32::from_rgb(255, 99, 99);
pub const SUCCESS: Color32 = Color32::from_rgb(88, 214, 141);

const DARK_TEXT: Color32 = Color32::from_rgb(16, 16, 20);
const SKY: Color32 = Color32::from_rgb(77, 182, 255);
const GOLD: Color32 = Color32::from_rgb(255, 196, 61);

/// The colors that change with the theme.
struct Palette {
    accent: Color32,
    accent_soft: Color32,
    /// Text and icons drawn on the accent color.
    on_accent: Color32,
    /// Marks the smart cut's re-encoded head; kept apart from the accent.
    smart: Color32,
    /// Marks keyframes on the timeline; kept apart from the accent.
    keyframe: Color32,
    /// The app icon, 64 × 64 RGBA.
    icon: &'static [u8],
}

fn palette(theme: ColorTheme) -> Palette {
    let rgb = Color32::from_rgb;
    match theme {
        ColorTheme::Teal => Palette {
            accent: rgb(20, 184, 166),
            accent_soft: rgb(94, 234, 212),
            on_accent: DARK_TEXT,
            smart: SKY,
            keyframe: GOLD,
            icon: include_bytes!("../resources/icons/teal-64.rgba"),
        },
        ColorTheme::Ocean => Palette {
            accent: rgb(59, 130, 246),
            accent_soft: rgb(147, 197, 253),
            on_accent: Color32::WHITE,
            smart: rgb(45, 212, 191),
            keyframe: GOLD,
            icon: include_bytes!("../resources/icons/ocean-64.rgba"),
        },
        ColorTheme::Mint => Palette {
            accent: rgb(52, 211, 153),
            accent_soft: rgb(110, 231, 183),
            on_accent: DARK_TEXT,
            smart: SKY,
            keyframe: GOLD,
            icon: include_bytes!("../resources/icons/mint-64.rgba"),
        },
        ColorTheme::Amber => Palette {
            accent: rgb(245, 158, 11),
            accent_soft: rgb(252, 211, 77),
            on_accent: DARK_TEXT,
            smart: SKY,
            keyframe: rgb(196, 181, 253),
            icon: include_bytes!("../resources/icons/amber-64.rgba"),
        },
        ColorTheme::Coral => Palette {
            accent: rgb(242, 84, 91),
            accent_soft: rgb(252, 165, 165),
            on_accent: Color32::WHITE,
            smart: SKY,
            keyframe: GOLD,
            icon: include_bytes!("../resources/icons/coral-64.rgba"),
        },
        ColorTheme::Purple => Palette {
            accent: rgb(124, 92, 255),
            accent_soft: rgb(160, 140, 255),
            on_accent: Color32::WHITE,
            smart: SKY,
            keyframe: GOLD,
            icon: include_bytes!("../resources/icons/purple-64.rgba"),
        },
        ColorTheme::Graphite => Palette {
            accent: rgb(230, 230, 234),
            accent_soft: Color32::WHITE,
            on_accent: DARK_TEXT,
            smart: SKY,
            keyframe: GOLD,
            icon: include_bytes!("../resources/icons/graphite-64.rgba"),
        },
    }
}

/// The theme in use; read while painting, so kept where every widget can reach it.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

fn current() -> Palette {
    let index = CURRENT.load(Ordering::Relaxed);
    palette(ColorTheme::ALL.get(index).copied().unwrap_or_default())
}

pub fn accent() -> Color32 {
    current().accent
}

pub fn accent_soft() -> Color32 {
    current().accent_soft
}

pub fn on_accent() -> Color32 {
    current().on_accent
}

pub fn smart() -> Color32 {
    current().smart
}

pub fn keyframe() -> Color32 {
    current().keyframe
}

/// `theme`'s accent, for its swatch in Settings.
pub fn swatch(theme: ColorTheme) -> Color32 {
    palette(theme).accent
}

pub fn name(theme: ColorTheme) -> &'static str {
    match theme {
        ColorTheme::Teal => "Teal",
        ColorTheme::Ocean => "Ocean",
        ColorTheme::Mint => "Mint",
        ColorTheme::Amber => "Amber",
        ColorTheme::Coral => "Coral",
        ColorTheme::Purple => "Purple",
        ColorTheme::Graphite => "Graphite",
    }
}

/// The app icon in `theme`'s colors, as the window icon.
pub fn icon(theme: ColorTheme) -> Arc<egui::IconData> {
    Arc::new(egui::IconData {
        rgba: palette(theme).icon.to_vec(),
        width: 64,
        height: 64,
    })
}

/// The app icon in the current theme, for drawing in the interface.
pub fn icon_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let index = CURRENT.load(Ordering::Relaxed);
    let id = egui::Id::unique(("app icon", index));
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return texture;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([64, 64], current().icon);
    let texture = ctx.load_texture("app icon", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

/// Styles the interface in `theme`; call again to switch.
pub fn apply(ctx: &egui::Context, theme: ColorTheme) {
    let index = ColorTheme::ALL
        .iter()
        .position(|&t| t == theme)
        .unwrap_or(0);
    CURRENT.store(index, Ordering::Relaxed);
    let mut visuals = Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = Color32::from_rgb(16, 16, 20);
    visuals.faint_bg_color = PANEL_RAISED;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.selection.bg_fill = accent();
    visuals.selection.stroke = Stroke::new(1.0, on_accent());
    visuals.hyperlink_color = accent_soft();
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
    visuals.widgets.active.weak_bg_fill = accent();
    // Window title bars use this fill; matching the panel makes each window one card.
    visuals.widgets.open.weak_bg_fill = PANEL;
    visuals.widgets.active.bg_fill = accent();
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

#[cfg(test)]
mod tests {
    use library::ColorTheme;

    #[test]
    fn the_exe_icons_are_in_theme_order() {
        let names: Vec<String> = ColorTheme::ALL
            .iter()
            .map(|&theme| super::name(theme).to_lowercase())
            .collect();
        assert_eq!(names.join(","), env!("VIBEPLAYER_ICON_ORDER"));
    }
}
