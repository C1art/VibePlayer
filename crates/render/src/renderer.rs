use media::{PlaneLayout, VideoFrame};

use crate::color::{ColorParams, Output};

/// An HDR display's brightness levels, for showing video on an scRGB surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayLevels {
    /// Where the system puts SDR white.
    pub sdr_white_nits: f32,
    /// The brightest the display can go.
    pub peak_nits: f32,
}

const UNIFORM_SIZE: u64 = 10 * 16;

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
    /// What `paint` draws: the uploaded planes, or a picture that stayed on the GPU.
    showing: Option<wgpu::BindGroup>,
    #[cfg(windows)]
    shared: crate::shared::SharedFrames,
    /// The window takes linear values: an sRGB or half-float (scRGB) surface.
    linear_target: bool,
    /// Set for an scRGB surface on an HDR display.
    display: Option<DisplayLevels>,
    /// The shown frame's color description, to redo the color setup when the display changes.
    /// The shown frame's colors, peak brightness and turn, kept to redo its parameters.
    shown_color: Option<(media::ColorInfo, PlaneLayout, Option<f32>, media::Rotation)>,
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
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
            showing: None,
            #[cfg(windows)]
            shared: crate::shared::SharedFrames::default(),
            linear_target: target_format.is_srgb()
                || target_format == wgpu::TextureFormat::Rgba16Float,
            display: None,
            shown_color: None,
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
        self.showing.is_some()
    }

    /// Copies a frame to the GPU. Frames this renderer can't take are ignored.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &VideoFrame) {
        let layout = frame.layout;
        if layout.bits > 8 && !self.high_bit_depth {
            return;
        }
        let limit = device.limits().max_texture_dimension_2d;
        if frame.width() > limit || frame.height() > limit {
            return;
        }
        if frame.gpu.is_some() {
            self.show_shared(device, queue, frame);
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
        // Only now that this frame is up does the shared picture stop being drawn.
        #[cfg(windows)]
        self.shared.release(queue);
        self.showing = Some(planes.bind_group.clone());
        self.write_color(queue, frame);
    }

    /// Shows a picture the decoder left on the GPU.
    #[cfg(windows)]
    fn show_shared(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &VideoFrame) {
        let Some(picture) = &frame.gpu else {
            return;
        };
        let bindings = crate::shared::Bindings {
            layout: &self.layout,
            uniforms: &self.uniforms,
            sampler: &self.sampler,
        };
        if let Some(bind_group) = self.shared.show(device, queue, picture, &bindings) {
            self.showing = Some(bind_group.clone());
            self.write_color(queue, frame);
        }
    }

    #[cfg(not(windows))]
    fn show_shared(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue, _frame: &VideoFrame) {}

    fn write_color(&mut self, queue: &wgpu::Queue, frame: &VideoFrame) {
        let peak = frame
            .luminance
            .max_content
            .or(frame.luminance.max_mastering);
        self.shown_color = Some((frame.color, frame.layout, peak, frame.rotation));
        self.write_shown_color(queue);
    }

    fn write_shown_color(&self, queue: &wgpu::Queue) {
        let Some((color, layout, peak, rotation)) = self.shown_color else {
            return;
        };
        let output = match (self.linear_target, self.display) {
            (true, Some(levels)) => Output::Scrgb {
                sdr_white_nits: levels.sdr_white_nits,
                peak_nits: levels.peak_nits,
            },
            (true, None) => Output::Linear,
            (false, _) => Output::Gamma,
        };
        let mut params = ColorParams::new(color, layout, peak, output);
        params.flags[2] = f32::from(rotation.quarter_turns());
        queue.write_buffer(&self.uniforms, 0, &params.to_bytes());
    }

    /// Follows the display: `Some` for an HDR display behind a half-float surface, `None`
    /// otherwise. A change redoes the shown frame's colors.
    pub fn set_display(&mut self, queue: &wgpu::Queue, display: Option<DisplayLevels>) {
        if display != self.display {
            self.display = display;
            self.write_shown_color(queue);
        }
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
        let Some(bind_group) = &self.showing else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Forgets the uploaded frame, e.g. when a file closes.
    pub fn clear(&mut self, queue: &wgpu::Queue) {
        #[cfg(windows)]
        self.shared.release(queue);
        #[cfg(not(windows))]
        let _ = queue;
        self.planes = None;
        self.showing = None;
        self.shown_color = None;
    }
}
