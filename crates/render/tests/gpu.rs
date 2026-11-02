//! Renders real frames on a GPU and reads the pixels back. Needs a GPU (or a software one like
//! lavapipe): `cargo test -p render -- --ignored`.

use std::path::PathBuf;
use std::task::{Context, Poll, Waker};

use media::{
    Demuxer, HardwareDecoding, Packet, StreamKind, VideoDecoder, VideoFrame, VideoOptions,
};
use render::VideoRenderer;

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
    let instance = wgpu::Instance::default();
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .expect("no GPU");
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & render::optional_features(),
        ..Default::default()
    }))
    .unwrap();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = VideoRenderer::new(&device, format);
    renderer.upload(&device, &queue, frame);
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
        size: u64::from(256 * SIZE),
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
                bytes_per_row: Some(256),
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
    let at = (SIZE as usize / 2) * 256 + (SIZE as usize / 2) * 4;
    [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
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
