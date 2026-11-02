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

pub struct Timeline {
    pub duration: Ticks,
    pub position: Ticks,
    pub start: Ticks,
    pub end: Ticks,
    /// Where the cut really starts, when the keyframe is before the requested start.
    pub keyframe_start: Option<Ticks>,
}

impl Timeline {
    pub fn show(&self, ui: &mut egui::Ui) -> TimelineInput {
        let height = 46.0;
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
        let track = rect.shrink2(vec2(0.0, 6.0));
        painter.rect_filled(track, CornerRadius::same(8), theme::PANEL_RAISED);
        let selected = Rect::from_x_y_ranges(x_of(self.start)..=x_of(self.end), track.y_range());
        painter.rect_filled(
            selected,
            CornerRadius::same(6),
            theme::ACCENT.gamma_multiply(0.35),
        );
        painter.rect_stroke(
            selected,
            CornerRadius::same(6),
            Stroke::new(2.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
        if let Some(keyframe) = self.keyframe_start {
            let span = Rect::from_x_y_ranges(x_of(keyframe)..=x_of(self.start), track.y_range());
            painter.rect_filled(span, 0.0, theme::KEYFRAME.gamma_multiply(0.35));
            let x = x_of(keyframe);
            painter.line_segment(
                [pos2(x, track.top()), pos2(x, track.bottom())],
                Stroke::new(2.0, theme::KEYFRAME),
            );
        }
        let mut input = TimelineInput::default();
        let start_handle = handle(ui, rect, x_of(self.start), "start handle");
        let end_handle = handle(ui, rect, x_of(self.end), "end handle");
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
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(2.0, theme::TEXT),
        );
        painter.circle_filled(pos2(x, rect.top() + 2.0), 4.0, theme::TEXT);
        input
    }
}

fn handle(ui: &mut egui::Ui, timeline: Rect, x: f32, name: &str) -> egui::Response {
    let rect = Rect::from_center_size(pos2(x, timeline.center().y), vec2(14.0, timeline.height()));
    let response = ui
        .interact(rect, ui.scope_id().with(name), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let active = response.hovered() || response.dragged();
    let grip = Rect::from_center_size(
        rect.center(),
        vec2(if active { 8.0 } else { 6.0 }, timeline.height() - 4.0),
    );
    let painter = ui.painter();
    painter.rect_filled(
        grip,
        CornerRadius::same(3),
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
            Stroke::new(1.0, Color32::from_black_alpha(160)),
        );
    }
    response
}
