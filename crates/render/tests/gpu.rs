//! Renders real frames on a GPU and reads the pixels back. Needs a GPU (or a software one like
//! lavapipe): `cargo test -p render -- --ignored`.

use std::path::PathBuf;
use std::task::{Context, Poll, Waker};

use media::{
    Demuxer, HardwareDecoding, Packet, StreamKind, VideoDecoder, VideoFrame, VideoOptions,
};
use render::{DisplayLevels, VideoRenderer};

const SIZE: u32 = 64;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}

fn first_frame(name: &str) -> VideoFrame {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    let mut demuxer = Demuxer::open(&path).unwrap();
    let stream = demuxer.best_stream(StreamKind::Video).unwrap();
    demuxer.select_streams(&[stream]);
    let options = VideoOptions {
        hardware: HardwareDecoding::Off,
        high_bit_depth: true,
        deinterlace: true,
        gpu: None,
    };
    let mut decoder = VideoDecoder::new(demuxer.stream_params(stream).unwrap(), options).unwrap();
    let mut packet = Packet::new().unwrap();
    loop {
        let more = demuxer.read(&mut packet).unwrap();
        decoder.send(more.then_some(&packet)).unwrap();
        if let Some(frame) = decoder.receive().unwrap() {
            return frame;
        }
        assert!(more, "no frame in {name}");
    }
}

/// Renders `frame` into a small RGBA target and returns the center pixel.
fn render_center(frame: &VideoFrame) -> [u8; 4] {
    let bytes = render_center_as(frame, wgpu::TextureFormat::Rgba8Unorm, None);
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

/// The center pixel's color on an HDR display behind a half-float (scRGB) surface.
fn render_center_scrgb(frame: &VideoFrame, display: DisplayLevels) -> [f32; 3] {
    let bytes = render_center_as(frame, wgpu::TextureFormat::Rgba16Float, Some(display));
    let channel =
        |index: usize| half_to_f32(u16::from_le_bytes([bytes[index * 2], bytes[index * 2 + 1]]));
    [channel(0), channel(1), channel(2)]
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    sign * match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

/// Renders `frame` into a small target of `format` and returns the center pixel's bytes.
fn render_center_as(
    frame: &VideoFrame,
    format: wgpu::TextureFormat,
    display: Option<DisplayLevels>,
) -> Vec<u8> {
    let instance = wgpu::Instance::default();
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .expect("no GPU");
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & render::optional_features(),
        ..Default::default()
    }))
    .unwrap();
    let mut renderer = VideoRenderer::new(&device, format);
    renderer.set_display(&queue, display);
    renderer.upload(&device, &queue, frame);
    let pixel_bytes = format.block_copy_size(None).unwrap();
    let row_bytes = SIZE * pixel_bytes;
    assert!(renderer.has_frame());
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row_bytes * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        renderer.paint(&mut pass);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = readback.slice(..).get_mapped_range().unwrap();
    let at = (SIZE * row_bytes / 2 + SIZE / 2 * pixel_bytes) as usize;
    bytes[at..at + pixel_bytes as usize].to_vec()
}

fn assert_red(pixel: [u8; 4]) {
    let [r, g, b, _] = pixel;
    assert!(
        r >= 245 && g <= 10 && b <= 10,
        "expected red, got {pixel:?}"
    );
}

#[test]
#[ignore]
fn planar_yuv_renders_its_true_color() {
    assert_red(render_center(&first_frame("red_yuv420p.mkv")));
}

#[test]
#[ignore]
fn interleaved_nv12_renders_its_true_color() {
    let frame = first_frame("red_nv12.mkv");
    assert!(frame.layout.interleaved_chroma);
    assert_red(render_center(&frame));
}

#[test]
#[ignore]
fn sdr_video_on_an_hdr_display_sits_at_the_sdr_white_level() {
    let display = DisplayLevels {
        sdr_white_nits: 240.0,
        peak_nits: 1000.0,
    };
    let [r, g, b] = render_center_scrgb(&first_frame("red_yuv420p.mkv"), display);
    // scRGB 1.0 is 80 nits, so full red at a 240-nit white is 3.0.
    assert!((r - 3.0).abs() < 0.1, "red {r}");
    assert!(g.abs() < 0.05 && b.abs() < 0.05, "{g} {b}");
}

#[test]
#[ignore]
fn hdr_video_never_goes_past_the_display_peak() {
    let display = DisplayLevels {
        sdr_white_nits: 200.0,
        peak_nits: 400.0,
    };
    let pixel = render_center_scrgb(&first_frame("hdr10.mkv"), display);
    assert!(
        pixel
            .iter()
            .all(|channel| channel.is_finite() && *channel <= 400.0 / 80.0 + 0.01),
        "{pixel:?}"
    );
}
