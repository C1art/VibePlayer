use media::{PlaneLayout, VideoFrame};

use crate::color::ColorParams;

const UNIFORM_SIZE: u64 = 9 * 16;

struct PlaneTextures {
    key: (u32, u32, PlaneLayout),
    /// Y, U, V; or Y and interleaved UV.
    textures: Vec<wgpu::Texture>,
    bind_group: wgpu::BindGroup,
}

/// Draws decoded video frames with wgpu. Create once per device; upload each new frame, then
/// paint it into whatever viewport the caller sets.
pub struct VideoRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    planes: Option<PlaneTextures>,
    target_is_srgb: bool,
    high_bit_depth: bool,
}

impl VideoRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> VideoRenderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("video"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/video.wgsl").into()),
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video planes"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("video"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("video"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("video planes"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video color"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        VideoRenderer {
            pipeline,
            layout,
            sampler,
            uniforms,
            planes: None,
            target_is_srgb: target_format.is_srgb(),
            high_bit_depth: device
                .features()
                .contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM),
        }
    }

    /// Whether frames with more than 8 bits per sample can be uploaded as-is.
    pub fn supports_high_bit_depth(&self) -> bool {
        self.high_bit_depth
    }

    pub fn has_frame(&self) -> bool {
        self.planes.is_some()
    }

    /// Copies a frame to the GPU. Frames this renderer can't take are ignored.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &VideoFrame) {
        let layout = frame.layout;
        if layout.bits > 8 && !self.high_bit_depth {
            return;
        }
        let key = (frame.width(), frame.height(), layout);
        if self.planes.as_ref().is_none_or(|planes| planes.key != key) {
            self.planes = Some(self.create_planes(device, key));
        }
        let Some(planes) = &self.planes else {
            return;
        };
        for (index, texture) in planes.textures.iter().enumerate() {
            let Some((bytes, stride)) = frame.plane(index) else {
                return;
            };
            let size = texture.size();
            let samples_per_texel = if index == 1 && layout.interleaved_chroma {
                2
            } else {
                1
            };
            let row_bytes = size.width as usize * layout.bytes_per_sample() * samples_per_texel;
            let needed = stride * (size.height as usize - 1) + row_bytes;
            if bytes.len() < needed || stride < row_bytes {
                return;
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes[..needed],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride as u32),
                    rows_per_image: Some(size.height),
                },
                size,
            );
        }
        let peak = frame
            .luminance
            .max_content
            .or(frame.luminance.max_mastering);
        let params = ColorParams::new(frame.color, layout, peak, self.target_is_srgb);
        queue.write_buffer(&self.uniforms, 0, &params.to_bytes());
    }

    fn create_planes(&self, device: &wgpu::Device, key: (u32, u32, PlaneLayout)) -> PlaneTextures {
        let (width, height, layout) = key;
        let high = layout.bits > 8;
        let (luma_format, chroma_format) = match (high, layout.interleaved_chroma) {
            (false, false) => (wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::R8Unorm),
            (false, true) => (wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm),
            (true, false) => (wgpu::TextureFormat::R16Unorm, wgpu::TextureFormat::R16Unorm),
            (true, true) => (
                wgpu::TextureFormat::R16Unorm,
                wgpu::TextureFormat::Rg16Unorm,
            ),
        };
        let chroma = |size: u32, shift: u8| size.div_ceil(1 << shift).max(1);
        let chroma_size = (
            chroma(width, layout.chroma_shift_x),
            chroma(height, layout.chroma_shift_y),
        );
        let mut planes = vec![
            ((width.max(1), height.max(1)), luma_format),
            (chroma_size, chroma_format),
        ];
        if !layout.interleaved_chroma {
            planes.push((chroma_size, chroma_format));
        }
        let textures: Vec<wgpu::Texture> = planes
            .into_iter()
            .map(|((width, height), format)| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("video plane"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            })
            .collect();
        let views: Vec<wgpu::TextureView> = textures
            .iter()
            .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()))
            .collect();
        // With interleaved chroma the shader reads V from plane_u too; bind it twice.
        let v_view = views.get(2).unwrap_or(&views[1]);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video planes"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(v_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        PlaneTextures {
            key,
            textures,
            bind_group,
        }
    }

    /// Draws the last uploaded frame over the render pass's current viewport.
    pub fn paint(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(planes) = &self.planes else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &planes.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Forgets the uploaded frame, e.g. when a file closes.
    pub fn clear(&mut self) {
        self.planes = None;
    }
}
