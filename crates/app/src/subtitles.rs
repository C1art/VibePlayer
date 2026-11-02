use cut::Ticks;
use eframe::egui::{self, Color32, FontId, Rect, TextureHandle, pos2, vec2};
use media::{Subtitle, SubtitleContent};

/// Paints the active subtitle over the video, keeping image subtitles as textures.
#[derive(Default)]
pub struct SubtitleOverlay {
    cached: Option<(Ticks, Vec<(Rect, TextureHandle)>)>,
}

impl SubtitleOverlay {
    /// `bottom_inset` keeps text subtitles clear of the controls when they're showing.
    pub fn paint(
        &mut self,
        ui: &egui::Ui,
        video: Rect,
        bottom_inset: f32,
        subtitle: Option<&Subtitle>,
    ) {
        let Some(subtitle) = subtitle else {
            self.cached = None;
            return;
        };
        match &subtitle.content {
            SubtitleContent::Text(text) => paint_text(ui, video, bottom_inset, text),
            SubtitleContent::Bitmaps {
                canvas_width,
                canvas_height,
                images,
            } => {
                if self
                    .cached
                    .as_ref()
                    .is_none_or(|(start, _)| *start != subtitle.start)
                {
                    let textures = images
                        .iter()
                        .enumerate()
                        .map(|(index, image)| {
                            let color = egui::ColorImage::from_rgba_unmultiplied(
                                [image.width as usize, image.height as usize],
                                &image.rgba,
                            );
                            let texture = ui.ctx().load_texture(
                                format!("subtitle {index}"),
                                color,
                                egui::TextureOptions::LINEAR,
                            );
                            let area = Rect::from_min_size(
                                pos2(image.x as f32, image.y as f32),
                                vec2(image.width as f32, image.height as f32),
                            );
                            (area, texture)
                        })
                        .collect();
                    self.cached = Some((subtitle.start, textures));
                }
                let scale = vec2(
                    video.width() / (*canvas_width).max(1) as f32,
                    video.height() / (*canvas_height).max(1) as f32,
                );
                if let Some((_, textures)) = &self.cached {
                    for (area, texture) in textures {
                        let target = Rect::from_min_size(
                            video.min + area.min.to_vec2() * scale,
                            area.size() * scale,
                        );
                        ui.painter().image(
                            texture.id(),
                            target,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                }
            }
            SubtitleContent::Clear => {}
        }
    }
}

fn paint_text(ui: &egui::Ui, video: Rect, bottom_inset: f32, text: &str) {
    let size = (video.height() / 18.0).clamp(16.0, 56.0);
    let painter = ui.painter();
    let galley = painter.layout(
        text.to_owned(),
        FontId::proportional(size),
        Color32::WHITE,
        video.width() * 0.9,
    );
    let position = pos2(
        video.center().x - galley.size().x / 2.0,
        (video.bottom() - video.height() * 0.06).min(ui.max_rect().bottom() - bottom_inset)
            - galley.size().y,
    );
    let outline = (size / 14.0).max(1.5);
    for (dx, dy) in [
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (-1.0, 0.0),
        (1.0, 0.0),
        (-1.0, 1.0),
        (0.0, 1.0),
        (1.0, 1.0),
    ] {
        painter.galley_with_override_text_color(
            position + vec2(dx * outline, dy * outline),
            galley.clone(),
            Color32::from_black_alpha(230),
        );
    }
    painter.galley(position, galley, Color32::WHITE);
}
