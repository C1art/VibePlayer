//! wgpu presentation, color conversion and HDR. Shaders live in `shaders/*.wgsl`.

mod color;
mod renderer;
#[cfg(windows)]
mod shared;

pub use renderer::VideoRenderer;

/// GPU features worth requesting when the adapter has them.
pub fn optional_features() -> wgpu::Features {
    wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
        | wgpu::Features::TEXTURE_FORMAT_NV12
        | wgpu::Features::TEXTURE_FORMAT_P010
}

/// How the decoder should keep hardware-decoded pictures so this device can show them without
/// a copy through system memory. `None` where that isn't supported (anything but DX12).
pub fn gpu_sharing(device: &wgpu::Device) -> Option<media::GpuSharing> {
    #[cfg(windows)]
    {
        shared::sharing(device)
    }
    #[cfg(not(windows))]
    {
        let _ = device;
        None
    }
}
