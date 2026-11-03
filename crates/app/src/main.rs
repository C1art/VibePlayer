// Release builds are GUI-only; debug builds keep a console for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod app;
mod app_actions;
mod chapters;
mod controls;
mod diagnostics;
mod editor;
mod editor_actions;
mod icons;
mod info_panel;
mod os;
mod osd;
mod seek_bar;
mod session;
mod settings_window;
mod shortcuts;
#[cfg(test)]
mod smoke_tests;
mod subtitles;
mod theme;
mod updates;
mod video_view;
mod welcome;
mod widgets;
mod window_state;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;

fn main() -> eframe::Result {
    // Absolute, so a running instance with another working directory can open it.
    let initial_file = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .map(|path| std::path::absolute(&path).unwrap_or(path));
    let listener = match os::single_instance::claim(initial_file.as_deref()) {
        os::single_instance::Claim::Forwarded => return Ok(()),
        os::single_instance::Claim::Primary(listener) => listener,
    };
    let store = library::Store::open_default().ok();
    if let Some(store) = &store {
        library::log::start(store.dir());
    }
    diagnostics::install_panic_hook();
    let settings = store
        .as_ref()
        .map(library::Store::load_settings)
        .unwrap_or_default();
    let viewport = egui::ViewportBuilder::default()
        .with_title("VibePlayer")
        .with_app_id("VibePlayer")
        .with_inner_size([1280.0, 760.0])
        .with_min_inner_size([480.0, 300.0])
        .with_drag_and_drop(true)
        .with_icon(Arc::new(egui::IconData {
            rgba: include_bytes!("../resources/icon-64.rgba").to_vec(),
            width: 64,
            height: 64,
        }));
    let options = eframe::NativeOptions {
        viewport: window_state::restore(viewport, settings.window),
        wgpu_options: wgpu_configuration(),
        ..Default::default()
    };
    eframe::run_native(
        "VibePlayer",
        options,
        Box::new(move |creation| {
            let requests = listener.map(|listener| listener.start(creation.egui_ctx.clone()));
            Ok(Box::new(app::VibePlayerApp::new(
                creation,
                store,
                settings,
                initial_file,
                requests,
            )))
        }),
    )
}

/// Prefers DX12, asks for 16-bit textures when available, and allows 8K frames.
fn wgpu_configuration() -> WgpuConfiguration {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends =
        wgpu::Backends::DX12 | wgpu::Backends::VULKAN | wgpu::Backends::METAL;
    setup.native_adapter_selector = Some(Arc::new(choose_adapter));
    setup.device_descriptor = Arc::new(|adapter| wgpu::DeviceDescriptor {
        label: Some("VibePlayer"),
        required_features: adapter.features() & render::optional_features(),
        required_limits: wgpu::Limits {
            max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d.clamp(8192, 16384),
            ..wgpu::Limits::default()
        },
        ..Default::default()
    });
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}

fn choose_adapter(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<wgpu::Adapter, String> {
    let usable = adapters
        .iter()
        .filter(|adapter| surface.is_none_or(|surface| adapter.is_surface_supported(surface)));
    let score = |adapter: &wgpu::Adapter| {
        let info = adapter.get_info();
        let backend = match info.backend {
            wgpu::Backend::Dx12 => 2,
            wgpu::Backend::Vulkan | wgpu::Backend::Metal => 1,
            _ => 0,
        };
        let device = match info.device_type {
            wgpu::DeviceType::DiscreteGpu => 3,
            wgpu::DeviceType::IntegratedGpu => 2,
            wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Other => 1,
            wgpu::DeviceType::Cpu => 0,
        };
        (device, backend)
    };
    usable
        .max_by_key(|adapter| score(adapter))
        .cloned()
        .ok_or_else(|| "no GPU adapter can present to this window".to_owned())
}
