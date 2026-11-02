use eframe::egui::{self, Rect};
use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, RenderState};
use media::VideoFrame;
use render::VideoRenderer;

/// Creates the renderer inside egui's wgpu resources. Returns whether 10/12-bit frames can
/// be shown without reducing them to 8 bits.
pub fn install(render_state: &RenderState) -> bool {
    let renderer = VideoRenderer::new(&render_state.device, render_state.target_format);
    let high_bit_depth = renderer.supports_high_bit_depth();
    render_state
        .renderer
        .write()
        .callback_resources
        .insert(renderer);
    high_bit_depth
}

pub fn upload(render_state: &RenderState, frame: &VideoFrame) {
    let mut egui_renderer = render_state.renderer.write();
    if let Some(renderer) = egui_renderer.callback_resources.get_mut::<VideoRenderer>() {
        renderer.upload(&render_state.device, &render_state.queue, frame);
    }
}

pub fn clear(render_state: &RenderState) {
    let mut egui_renderer = render_state.renderer.write();
    if let Some(renderer) = egui_renderer.callback_resources.get_mut::<VideoRenderer>() {
        renderer.clear();
    }
}

/// Draws the last uploaded frame into `rect`.
pub fn paint(ui: &egui::Ui, rect: Rect) {
    ui.painter()
        .add(egui_wgpu::Callback::new_paint_callback(rect, VideoCallback));
}

/// The largest rect of `aspect` (width / height) centered in `area`.
pub fn fit(area: Rect, aspect: f32) -> Rect {
    if !aspect.is_finite() || aspect <= 0.0 || area.height() <= 0.0 {
        return area;
    }
    let size = if area.width() / area.height() > aspect {
        egui::vec2(area.height() * aspect, area.height())
    } else {
        egui::vec2(area.width(), area.width() / aspect)
    };
    Rect::from_center_size(area.center(), size)
}

struct VideoCallback;

impl CallbackTrait for VideoCallback {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut eframe::wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if let Some(renderer) = resources.get::<VideoRenderer>() {
            renderer.paint(render_pass);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fit;
    use eframe::egui::{Rect, pos2, vec2};

    #[test]
    fn wide_video_in_a_tall_area_is_letterboxed() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 1000.0));
        let fitted = fit(area, 2.0);
        assert_eq!(fitted.size(), vec2(1000.0, 500.0));
        assert_eq!(fitted.center(), area.center());
    }

    #[test]
    fn tall_video_in_a_wide_area_is_pillarboxed() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 900.0));
        assert_eq!(fit(area, 0.5).size(), vec2(450.0, 900.0));
    }
}
