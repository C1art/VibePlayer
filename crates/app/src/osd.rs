use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, FontId, Rect};

use crate::theme;

const VISIBLE_FOR: Duration = Duration::from_millis(1400);
const FADE: Duration = Duration::from_millis(300);

/// Short on-screen messages like "Volume 60%".
#[derive(Default)]
pub struct Osd {
    message: Option<(String, Instant)>,
}

impl Osd {
    pub fn show(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), Instant::now()));
    }

    /// Paints the message at the top center of `area`; returns whether it's still visible.
    pub fn paint(&mut self, ui: &egui::Ui, area: Rect) -> bool {
        let Some((text, shown_at)) = &self.message else {
            return false;
        };
        let age = shown_at.elapsed();
        if age > VISIBLE_FOR + FADE {
            self.message = None;
            return false;
        }
        let fade = 1.0 - (age.saturating_sub(VISIBLE_FOR)).as_secs_f32() / FADE.as_secs_f32();
        let painter = ui.painter();
        let galley = painter.layout_no_wrap(
            text.clone(),
            FontId::proportional(18.0),
            theme::TEXT.gamma_multiply(fade),
        );
        let origin = egui::pos2(area.center().x - galley.size().x / 2.0, area.top() + 56.0);
        let background = Rect::from_min_size(origin, galley.size()).expand2(egui::vec2(14.0, 8.0));
        painter.rect_filled(
            background,
            10.0,
            Color32::from_black_alpha((170.0 * fade) as u8),
        );
        painter.galley(origin, galley, theme::TEXT);
        true
    }
}
