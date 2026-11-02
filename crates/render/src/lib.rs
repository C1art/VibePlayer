//! wgpu presentation, color conversion and HDR. Shaders live in `shaders/*.wgsl`.

mod color;
mod renderer;

pub use renderer::VideoRenderer;

/// GPU features worth requesting when the adapter has them.
pub fn optional_features() -> wgpu::Features {
    wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
}
