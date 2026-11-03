//! Reopening the window where it was left.

use eframe::egui;
use library::WindowPlacement;

use crate::os;

/// Starts the window at `placement` if it's still on a screen.
pub fn restore(
    builder: egui::ViewportBuilder,
    placement: Option<WindowPlacement>,
) -> egui::ViewportBuilder {
    let Some(placement) = placement else {
        return builder;
    };
    let size = [placement.width as f32, placement.height as f32];
    let builder = builder
        .with_inner_size(size)
        .with_maximized(placement.maximized);
    // A monitor may have been unplugged; then let the system place the window.
    if os::is_on_screen(placement.x, placement.y) {
        builder.with_position([placement.x as f32, placement.y as f32])
    } else {
        builder
    }
}

/// Follows the window's normal (not maximized or fullscreen) placement as it changes.
#[derive(Default)]
pub struct WindowTracker {
    placement: Option<WindowPlacement>,
}

impl WindowTracker {
    pub fn new(placement: Option<WindowPlacement>) -> WindowTracker {
        WindowTracker { placement }
    }

    pub fn observe(&mut self, ctx: &egui::Context) {
        ctx.input(|input| {
            let viewport = input.viewport();
            if viewport.fullscreen == Some(true) || viewport.minimized == Some(true) {
                return;
            }
            let maximized = viewport.maximized == Some(true);
            if maximized {
                // Keep the normal size and position to return to from maximized.
                if let Some(placement) = &mut self.placement {
                    placement.maximized = true;
                }
                return;
            }
            let (Some(outer), Some(inner)) = (viewport.outer_rect, viewport.inner_rect) else {
                return;
            };
            self.placement = Some(WindowPlacement {
                x: outer.min.x.round() as i32,
                y: outer.min.y.round() as i32,
                width: inner.width().round().max(1.0) as u32,
                height: inner.height().round().max(1.0) as u32,
                maximized: false,
            });
        });
    }

    pub fn placement(&self) -> Option<WindowPlacement> {
        self.placement
    }
}
