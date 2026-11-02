use cut::Ticks;
use eframe::egui::{self, Color32, CornerRadius, Rect, Sense, Stroke, pos2, vec2};

use crate::theme;

/// What the user did on the timeline.
#[derive(Default)]
pub struct TimelineInput {
    pub start: Option<Ticks>,
    pub end: Option<Ticks>,
    pub seek: Option<Ticks>,
    pub scrub: Option<Ticks>,
}

pub struct Timeline<'a> {
    pub duration: Ticks,
    pub position: Ticks,
    pub start: Ticks,
    pub end: Ticks,
    /// Where a lossless cut really starts, when that's before the requested start.
    pub keyframe_start: Option<Ticks>,
    /// Frames re-encoded by a smart cut, from the start to here.
    pub reencoded_until: Option<Ticks>,
    pub keyframes: &'a [Ticks],
}

impl Timeline<'_> {
    pub fn show(&self, ui: &mut egui::Ui) -> TimelineInput {
        let height = 54.0;
        let (rect, background) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
        let duration = self.duration.0.max(1);
        let x_of = |t: Ticks| {
            rect.left() + rect.width() * (t.0.clamp(0, duration) as f32 / duration as f32)
        };
        let time_at = |x: f32| {
            let fraction = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            Ticks((fraction as f64 * duration as f64) as i64)
        };
        let painter = ui.painter().clone();
        let track = Rect::from_min_max(
            pos2(rect.left(), rect.top() + 6.0),
            pos2(rect.right(), rect.bottom() - 14.0),
        );
        painter.rect_filled(track, CornerRadius::same(8), theme::PANEL_RAISED);
        let selected = Rect::from_x_y_ranges(x_of(self.start)..=x_of(self.end), track.y_range());
        painter.rect_filled(
            selected,
            CornerRadius::same(6),
            theme::ACCENT.gamma_multiply(0.30),
        );
        if let Some(keyframe) = self.keyframe_start {
            let span = Rect::from_x_y_ranges(x_of(keyframe)..=x_of(self.start), track.y_range());
            painter.rect_filled(span, 0.0, theme::KEYFRAME.gamma_multiply(0.30));
            let x = x_of(keyframe);
            painter.line_segment(
                [pos2(x, track.top()), pos2(x, track.bottom())],
                Stroke::new(2.0, theme::KEYFRAME),
            );
        }
        if let Some(until) = self.reencoded_until {
            let span = Rect::from_x_y_ranges(x_of(self.start)..=x_of(until), track.y_range());
            painter.rect_filled(span, 0.0, theme::SMART.gamma_multiply(0.35));
        }
        painter.rect_stroke(
            selected,
            CornerRadius::same(6),
            Stroke::new(2.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
        self.paint_keyframes(&painter, rect, &x_of);

        let mut input = TimelineInput::default();
        let start_handle = handle(ui, track, x_of(self.start), "start handle");
        let end_handle = handle(ui, track, x_of(self.end), "end handle");
        if let Some(pointer) = start_handle.interact_pointer_pos()
            && start_handle.dragged()
        {
            input.start = Some(time_at(pointer.x).min(self.end - Ticks(1)));
        }
        if let Some(pointer) = end_handle.interact_pointer_pos()
            && end_handle.dragged()
        {
            input.end = Some(time_at(pointer.x).max(self.start + Ticks(1)));
        }
        if start_handle.drag_stopped() {
            input.seek = Some(self.start);
        }
        if end_handle.drag_stopped() {
            input.seek = Some(self.end);
        }
        if let Some(pointer) = background.interact_pointer_pos() {
            if background.dragged() {
                input.scrub = Some(time_at(pointer.x));
            }
            if background.clicked() || background.drag_stopped() {
                input.seek = Some(time_at(pointer.x));
            }
        }
        let x = x_of(self.position);
        painter.line_segment(
            [pos2(x, track.top() - 4.0), pos2(x, track.bottom() + 4.0)],
            Stroke::new(2.0, theme::TEXT),
        );
        painter.circle_filled(pos2(x, track.top() - 4.0), 4.0, theme::TEXT);
        input
    }

    /// Small ticks under the track, thinned so dense keyframes don't turn into a solid bar.
    fn paint_keyframes(&self, painter: &egui::Painter, rect: Rect, x_of: &impl Fn(Ticks) -> f32) {
        let top = rect.bottom() - 11.0;
        let mut last_x = f32::NEG_INFINITY;
        for &keyframe in self.keyframes {
            let x = x_of(keyframe);
            if x - last_x < 3.0 {
                continue;
            }
            last_x = x;
            painter.line_segment(
                [pos2(x, top), pos2(x, top + 7.0)],
                Stroke::new(1.5, theme::KEYFRAME.gamma_multiply(0.75)),
            );
        }
    }
}

fn handle(ui: &mut egui::Ui, track: Rect, x: f32, name: &str) -> egui::Response {
    let rect = Rect::from_center_size(pos2(x, track.center().y), vec2(16.0, track.height() + 8.0));
    let response = ui
        .interact(rect, ui.scope_id().with(name), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let active = response.hovered() || response.dragged();
    let grip = Rect::from_center_size(
        rect.center(),
        vec2(if active { 9.0 } else { 7.0 }, track.height() + 6.0),
    );
    let painter = ui.painter();
    painter.rect_filled(
        grip,
        CornerRadius::same(4),
        if active {
            theme::TEXT
        } else {
            theme::ACCENT_SOFT
        },
    );
    for dy in [-5.0, 0.0, 5.0] {
        painter.line_segment(
            [
                pos2(grip.center().x - 1.5, grip.center().y + dy),
                pos2(grip.center().x + 1.5, grip.center().y + dy),
            ],
            Stroke::new(1.0, Color32::from_black_alpha(170)),
        );
    }
    response
}
