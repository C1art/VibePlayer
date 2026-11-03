# Vendored crates

## egui-wgpu 0.37.0

egui-wgpu always picks an 8-bit surface in the default (SDR) color space, and eframe
offers no way to change that, so HDR output needs a patched copy. The first commit that
adds `vendor/egui-wgpu` is the unmodified crate from crates.io; the commit after it holds
every change, each marked `VibePlayer:` in the source:

- `WgpuConfiguration::high_dynamic_range`: when set and the surface supports it, the
  surface is `Rgba16Float` in the extended linear sRGB (scRGB) color space.
- `RenderState::color_space` and `RenderState::display`: the surface's color space and the
  display's latest `DisplayHdrInfo`, refreshed once a second.
- On an HDR display, egui's output (and the clear color) is scaled so its white matches
  the system's SDR white level.

When upgrading egui, copy the new egui-wgpu here as its own commit and reapply that diff.
