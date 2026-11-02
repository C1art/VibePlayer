use std::sync::Arc;

use cut::{ClockPrecision, ClockTime, Ticks, Timebase};
use eframe::egui::{self, Color32, CornerRadius, Rect, Sense, Stroke, TextureHandle, pos2, vec2};
use media::RgbaImage;

use crate::theme;

/// What the user did with the seek bar this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeekInput {
    /// Being dragged to this position: show something fast.
    pub scrub: Option<Ticks>,
    /// Released or clicked here: seek exactly.
    pub commit: Option<Ticks>,
    /// Hovered without pressing.
    pub hover: Option<Ticks>,
}

/// Marks drawn on the bar, e.g. the editor's trim range.
pub struct Range {
    pub start: Ticks,
    pub end: Ticks,
}

pub struct SeekBar<'a> {
    pub position: Ticks,
    pub duration: Ticks,
    pub highlight: Option<Range>,
    pub preview: Option<&'a mut dyn FnMut(Ticks) -> Option<TextureHandle>>,
}

impl SeekBar<'_> {
    pub fn show(self, ui: &mut egui::Ui) -> SeekInput {
        let height = 22.0;
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
        let duration = self.duration.0.max(1);
        let to_x = |t: Ticks| {
            rect.left() + rect.width() * (t.0.clamp(0, duration) as f32 / duration as f32)
        };
        let to_time = |x: f32| {
            let fraction = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            Ticks((fraction as f64 * duration as f64) as i64)
        };
        let active = response.hovered() || response.dragged();
        let track_height = if active { 6.0 } else { 4.0 };
        let track = Rect::from_center_size(rect.center(), vec2(rect.width(), track_height));
        let painter = ui.painter();
        painter.rect_filled(track, CornerRadius::same(3), theme::TRACK);
        if let Some(range) = &self.highlight {
            let span = Rect::from_x_y_ranges(to_x(range.start)..=to_x(range.end), track.y_range());
            painter.rect_filled(
                span,
                CornerRadius::same(3),
                theme::ACCENT.gamma_multiply(0.45),
            );
        }
        let mut input = SeekInput::default();
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        let displayed = match (response.dragged(), pointer) {
            (true, Some(pointer)) => to_time(pointer.x),
            _ => self.position,
        };
        let played = Rect::from_x_y_ranges(track.left()..=to_x(displayed), track.y_range());
        painter.rect_filled(played, CornerRadius::same(3), theme::ACCENT);
        if active {
            painter.circle_filled(pos2(to_x(displayed), rect.center().y), 7.0, theme::TEXT);
        }
        if let Some(pointer) = pointer {
            let at = to_time(pointer.x);
            if response.dragged() {
                input.scrub = Some(at);
            }
            if response.drag_stopped() || response.clicked() {
                input.commit = Some(at);
            }
            if response.hovered() || response.dragged() {
                input.hover = Some(at);
                if let Some(preview) = self.preview {
                    paint_hover(ui, rect, to_x(at), at, preview(at));
                }
            }
        }
        input
    }
}

fn paint_hover(ui: &egui::Ui, bar: Rect, x: f32, at: Ticks, thumbnail: Option<TextureHandle>) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::unique("seek preview"),
    ));
    let label = ClockTime::new(at, Timebase::MICROS, ClockPrecision::Seconds).to_string();
    let galley = painter.layout_no_wrap(label, egui::FontId::proportional(13.0), theme::TEXT);
    let thumb_size = thumbnail
        .as_ref()
        .map(|t| {
            let size = t.size_vec2();
            size * (160.0 / size.x.max(1.0))
        })
        .unwrap_or(egui::Vec2::ZERO);
    let width = thumb_size.x.max(galley.size().x + 16.0);
    let height = thumb_size.y + galley.size().y + 12.0;
    let screen = ui.ctx().content_rect();
    let left = (x - width / 2.0).clamp(screen.left() + 4.0, screen.right() - width - 4.0);
    let frame = Rect::from_min_size(pos2(left, bar.top() - height - 10.0), vec2(width, height));
    painter.rect(
        frame,
        CornerRadius::same(8),
        Color32::from_black_alpha(220),
        Stroke::new(1.0, Color32::from_white_alpha(30)),
        egui::StrokeKind::Outside,
    );
    if let Some(texture) = thumbnail {
        let image = Rect::from_min_size(frame.left_top(), thumb_size);
        painter.image(
            texture.id(),
            image,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    let text_pos = pos2(
        frame.center().x - galley.size().x / 2.0,
        frame.bottom() - galley.size().y - 6.0,
    );
    painter.galley(text_pos, galley, theme::TEXT);
}

/// Converts a decoded thumbnail into an egui texture.
pub fn texture(ctx: &egui::Context, name: &str, image: &Arc<RgbaImage>) -> TextureHandle {
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width as usize, image.height as usize],
        &image.pixels,
    );
    ctx.load_texture(name, color, egui::TextureOptions::LINEAR)
}
