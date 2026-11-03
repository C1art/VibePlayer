//! Showing pictures the decoder left on the GPU (Windows, DX12): their shared textures and
//! fences are opened in wgpu's own D3D12 device, so nothing goes through system memory.

use std::ffi::c_void;
use std::sync::Arc;

use media::{GpuPicture, GpuSharing, SharedSurfaces, SlotLease, SurfaceFormat};
use wgpu::hal::api::Dx12;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D12::{ID3D12Device, ID3D12Fence, ID3D12Resource};

/// How the decoder should keep pictures for this device, if the device can show them.
pub fn sharing(device: &wgpu::Device) -> Option<GpuSharing> {
    let needed = wgpu::Features::TEXTURE_FORMAT_NV12;
    if !device.features().contains(needed) {
        return None;
    }
    // SAFETY: the raw device is only queried for its adapter's LUID.
    let luid = unsafe {
        let hal = device.as_hal::<Dx12>()?;
        hal.raw_device().GetAdapterLuid()
    };
    Some(GpuSharing {
        adapter_luid: (i64::from(luid.HighPart) << 32) | i64::from(luid.LowPart),
    })
}

/// One set of the decoder's textures, opened here, with a bind group per texture.
struct ImportedSet {
    surfaces: Arc<SharedSurfaces>,
    bind_groups: Vec<wgpu::BindGroup>,
    ready: ID3D12Fence,
    done: ID3D12Fence,
    /// The last value signalled on `done`.
    done_value: u64,
}

/// What a bind group for a picture needs besides its planes.
pub(crate) struct Bindings<'a> {
    pub layout: &'a wgpu::BindGroupLayout,
    pub uniforms: &'a wgpu::Buffer,
    pub sampler: &'a wgpu::Sampler,
}

/// The shared picture on screen and the textures it came from.
#[derive(Default)]
pub(crate) struct SharedFrames {
    set: Option<ImportedSet>,
    shown: Option<Arc<SlotLease>>,
}

impl SharedFrames {
    /// Puts `picture` on screen: the queue waits for the decoder's copy before reading it,
    /// and gives back the previous picture once done with it. Returns its bind group, or
    /// `None` if the textures couldn't be opened (the caller then shows nothing new).
    pub fn show(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        picture: &GpuPicture,
        bindings: &Bindings<'_>,
    ) -> Option<&wgpu::BindGroup> {
        self.release(queue);
        let surfaces = picture.lease.surfaces();
        if self
            .set
            .as_ref()
            .is_none_or(|set| set.surfaces.id != surfaces.id)
        {
            self.set = import(device, surfaces, bindings);
            if self.set.is_none() {
                surfaces.reject();
                return None;
            }
        }
        let set = self.set.as_ref()?;
        // SAFETY: the fence is a live D3D12 fence on this device's queue.
        let waiting = unsafe {
            queue
                .as_hal::<Dx12>()
                .is_some_and(|hal| hal.as_raw().Wait(&set.ready, picture.ready_at).is_ok())
        };
        if !waiting {
            surfaces.reject();
            return None;
        }
        self.shown = Some(Arc::clone(&picture.lease));
        set.bind_groups.get(picture.lease.index())
    }

    /// Lets the decoder reuse the texture on screen once the GPU has finished reading it:
    /// everything submitted so far, including the last draw of it.
    pub fn release(&mut self, queue: &wgpu::Queue) {
        let (Some(lease), Some(set)) = (self.shown.take(), self.set.as_mut()) else {
            return;
        };
        set.done_value += 1;
        // SAFETY: the fence is a live D3D12 fence shared with the decoder's device.
        let signalled = unsafe {
            queue
                .as_hal::<Dx12>()
                .is_some_and(|hal| hal.as_raw().Signal(&set.done, set.done_value).is_ok())
        };
        // Without the signal there's no knowing when the GPU is done; better to retire the
        // texture than to risk overwriting it while it's read.
        lease.done_after(if signalled { set.done_value } else { u64::MAX });
    }
}

fn import(
    device: &wgpu::Device,
    surfaces: &Arc<SharedSurfaces>,
    bindings: &Bindings<'_>,
) -> Option<ImportedSet> {
    let (format, luma, chroma) = match surfaces.format {
        SurfaceFormat::Nv12 => (
            wgpu::TextureFormat::NV12,
            wgpu::TextureFormat::R8Unorm,
            wgpu::TextureFormat::Rg8Unorm,
        ),
        SurfaceFormat::P010 => (
            wgpu::TextureFormat::P010,
            wgpu::TextureFormat::R16Unorm,
            wgpu::TextureFormat::Rg16Unorm,
        ),
    };
    let required = match surfaces.format {
        SurfaceFormat::Nv12 => wgpu::Features::TEXTURE_FORMAT_NV12,
        SurfaceFormat::P010 => {
            wgpu::Features::TEXTURE_FORMAT_P010 | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
        }
    };
    if !device.features().contains(required) {
        return None;
    }
    let size = wgpu::Extent3d {
        width: surfaces.width,
        height: surfaces.height,
        depth_or_array_layers: 1,
    };
    // SAFETY: the handles are NT handles to shared D3D11 textures and fences on this device's
    // adapter, kept open by `surfaces`; opening them adds this device's own references.
    let (raw, ready, done) = unsafe {
        let hal = device.as_hal::<Dx12>()?;
        let raw: ID3D12Device = hal.raw_device().clone();
        let ready: ID3D12Fence = open(&raw, surfaces.ready_fence)?;
        let done: ID3D12Fence = open(&raw, surfaces.done_fence)?;
        (raw, ready, done)
    };
    let descriptor = wgpu::TextureDescriptor {
        label: Some("decoded picture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };
    let mut bind_groups = Vec::with_capacity(surfaces.textures.len());
    for &handle in &surfaces.textures {
        // SAFETY: as above; the resource is a single-mip 2D texture of `format` and `size`,
        // and only ever read here, which promotes it from the common state implicitly.
        let texture = unsafe {
            let resource: ID3D12Resource = open(&raw, handle)?;
            let hal_texture = wgpu::hal::dx12::Device::texture_from_raw(
                resource,
                format,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            );
            device.create_texture_from_hal::<Dx12>(
                hal_texture,
                &descriptor,
                wgpu::TextureUses::RESOURCE,
            )
        };
        let plane = |aspect, format| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(format),
                aspect,
                ..Default::default()
            })
        };
        let luma_view = plane(wgpu::TextureAspect::Plane0, luma);
        let chroma_view = plane(wgpu::TextureAspect::Plane1, chroma);
        bind_groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("decoded picture"),
            layout: bindings.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: bindings.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&luma_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&chroma_view),
                },
                // Interleaved chroma: the shader reads V from the same plane.
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&chroma_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(bindings.sampler),
                },
            ],
        }));
    }
    Some(ImportedSet {
        surfaces: Arc::clone(surfaces),
        bind_groups,
        ready,
        done,
        done_value: 0,
    })
}

/// Opens a shared NT handle as a D3D12 object.
///
/// # Safety
/// `handle` must be a live NT handle to an object of type `T` shareable with `device`.
unsafe fn open<T: windows::core::Interface>(device: &ID3D12Device, handle: isize) -> Option<T> {
    let mut object: Option<T> = None;
    // SAFETY: guaranteed by the caller; the out-pointer is valid.
    unsafe { device.OpenSharedHandle(HANDLE(handle as *mut c_void), &mut object) }.ok()?;
    object
}
