//! Reopening the window where it was left.
//!
//! On Windows the placement is the window's own rectangle in pixels, read and set through
//! Win32: egui's logical points can't be turned back into a place on screen when monitors
//! use different scale factors. Elsewhere (development only) it is egui's logical points.

use eframe::egui;
use library::WindowPlacement;

use crate::os::{self, NativeWindow};

/// Sizes the window to `placement` before it exists (not on Windows, which places it in
/// [`place`]).
pub fn restore(
    builder: egui::ViewportBuilder,
    placement: Option<WindowPlacement>,
) -> egui::ViewportBuilder {
    match placement {
        Some(placement) if !cfg!(windows) => builder
            .with_inner_size([placement.width as f32, placement.height as f32])
            .with_position([placement.x as f32, placement.y as f32])
            .with_maximized(placement.maximized),
        _ => builder,
    }
}

/// Moves the newly created, still hidden window to `placement` (Windows), if that is on a
/// monitor that is still there; then maximizes it there if it was maximized.
pub fn place(
    window: Option<NativeWindow>,
    placement: Option<WindowPlacement>,
    ctx: &egui::Context,
) {
    let (Some(window), Some(placement)) = (window, placement) else {
        return;
    };
    if os::place_window(window, placement) && placement.maximized {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
    }
}

/// Follows the window's normal (not maximized or fullscreen) placement as it changes.
pub struct WindowTracker {
    window: Option<NativeWindow>,
    placement: Option<WindowPlacement>,
}

impl WindowTracker {
    pub fn new(window: Option<NativeWindow>, placement: Option<WindowPlacement>) -> WindowTracker {
        WindowTracker { window, placement }
    }

    pub fn observe(&mut self, ctx: &egui::Context) {
        let (fullscreen, minimized) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.fullscreen == Some(true),
                viewport.minimized == Some(true),
            )
        });
        if fullscreen || minimized {
            return;
        }
        if let Some(window) = self.window {
            if let Some(placement) = os::window_placement(window, self.placement) {
                self.placement = Some(placement);
            }
            return;
        }
        self.observe_logical(ctx);
    }

    fn observe_logical(&mut self, ctx: &egui::Context) {
        ctx.input(|input| {
            let viewport = input.viewport();
            if viewport.maximized == Some(true) {
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
