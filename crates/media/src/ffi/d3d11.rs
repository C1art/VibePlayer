//! D3D11 decoding on the renderer's GPU, and copying decoded pictures into textures the
//! renderer's D3D12 device can open.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use ffmpeg_sys_next as sys;
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_FENCE_FLAG_SHARED, D3D11_RESOURCE_MISC_SHARED,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device,
    ID3D11Device5, ID3D11DeviceContext4, ID3D11Fence, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_NV12, DXGI_FORMAT_P010, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIFactory1,
    IDXGIResource1,
};
use windows::core::{Interface, PCWSTR};

use super::Frame;
use crate::gpu::{GpuPicture, SharedSurfaces, Slot, SurfaceFormat};

/// Textures in a set: more than the pictures in flight between decoder and screen.
const SLOTS: usize = 12;

static NEXT_SET_ID: AtomicU64 = AtomicU64::new(1);

/// FFmpeg's `AVD3D11VADeviceContext` (libavutil/hwcontext_d3d11va.h), laid out as it has been
/// since FFmpeg 4. Declared here because the bindings only include it when the Windows SDK
/// headers were found at build time.
#[repr(C)]
struct D3d11vaDeviceContext {
    device: *mut c_void,
    device_context: *mut c_void,
    video_device: *mut c_void,
    video_context: *mut c_void,
    lock: Option<unsafe extern "C" fn(*mut c_void)>,
    unlock: Option<unsafe extern "C" fn(*mut c_void)>,
    lock_ctx: *mut c_void,
}

/// The DXGI index of the adapter with `luid`, which FFmpeg takes as the D3D11 device name.
pub fn adapter_index(luid: i64) -> Option<u32> {
    // SAFETY: plain DXGI enumeration; every returned object is owned and released here.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        (0..16).find(|&index| {
            factory
                .EnumAdapters1(index)
                .and_then(|adapter| adapter.GetDesc1())
                .is_ok_and(|desc| {
                    let id = desc.AdapterLuid;
                    (i64::from(id.HighPart) << 32) | i64::from(id.LowPart) == luid
                })
        })
    }
}

/// The D3D11 objects behind a [`SharedSurfaces`] set.
struct Objects {
    device: ID3D11Device,
    context: ID3D11DeviceContext4,
    textures: Vec<ID3D11Texture2D>,
    ready: ID3D11Fence,
    done: ID3D11Fence,
    handles: Vec<HANDLE>,
}

// SAFETY: FFmpeg turns on D3D11 multithread protection for the devices it creates, and the
// context is only used under FFmpeg's device lock.
unsafe impl Send for Objects {}
// SAFETY: as above.
unsafe impl Sync for Objects {}

impl Drop for Objects {
    fn drop(&mut self) {
        for handle in &self.handles {
            // SAFETY: each handle was created by CreateSharedHandle and is closed once.
            let _ = unsafe { CloseHandle(*handle) };
        }
    }
}

/// Copies decoded D3D11 pictures into a set of shared textures.
pub struct SurfacePool {
    surfaces: Arc<SharedSurfaces>,
    objects: Arc<Objects>,
    next_ready: u64,
}

/// A hardware frame's texture, array slice, size, format and device.
struct Source<'a> {
    texture: &'a ID3D11Texture2D,
    slice: u32,
    width: u32,
    height: u32,
    format: SurfaceFormat,
    device: &'a D3d11vaDeviceContext,
}

impl Frame {
    /// The D3D11 texture behind a hardware frame, if it is one.
    fn d3d11_source(&self) -> Option<Source<'_>> {
        let raw = self.raw();
        if raw.format != sys::AVPixelFormat::AV_PIX_FMT_D3D11 as i32 || raw.hw_frames_ctx.is_null()
        {
            return None;
        }
        // SAFETY: a D3D11 frame's `hw_frames_ctx` holds an AVHWFramesContext whose device
        // context's `hwctx` is an AVD3D11VADeviceContext; `data[0]` is the texture and
        // `data[1]` its array slice. All stay valid while the frame holds its references.
        unsafe {
            let frames = (*raw.hw_frames_ctx)
                .data
                .cast::<sys::AVHWFramesContext>()
                .as_ref()?;
            let device = frames.device_ctx.as_ref()?;
            let device = device.hwctx.cast::<D3d11vaDeviceContext>().as_ref()?;
            let format = match frames.sw_format {
                sys::AVPixelFormat::AV_PIX_FMT_NV12 => SurfaceFormat::Nv12,
                sys::AVPixelFormat::AV_PIX_FMT_P010LE => SurfaceFormat::P010,
                _ => return None,
            };
            // The same pointer, typed as the borrowed interface pointer it is.
            let pointer = &*std::ptr::from_ref(&raw.data[0]).cast::<*mut c_void>();
            let texture = ID3D11Texture2D::from_raw_borrowed(pointer)?;
            Some(Source {
                texture,
                slice: u32::try_from(raw.data[1] as usize).ok()?,
                width: u32::try_from(raw.width).ok()?,
                height: u32::try_from(raw.height).ok()?,
                format,
                device,
            })
        }
    }
}

impl SurfacePool {
    pub fn format(&self) -> SurfaceFormat {
        self.surfaces.format
    }

    /// Whether the renderer couldn't open these textures.
    pub fn is_rejected(&self) -> bool {
        self.surfaces.is_rejected()
    }

    /// Whether `frame` can be copied into this set: same device, size and format.
    pub fn fits(&self, frame: &Frame) -> bool {
        frame.d3d11_source().is_some_and(|source| {
            even(source.width) == self.surfaces.width
                && even(source.height) == self.surfaces.height
                && source.format == self.surfaces.format
                && source.device.device == self.objects.device.as_raw()
        })
    }

    /// A set of textures for pictures like `frame`, on its device.
    pub fn for_frame(frame: &Frame) -> Option<SurfacePool> {
        let source = frame.d3d11_source()?;
        let (width, height) = (even(source.width), even(source.height));
        // SAFETY: `device` is FFmpeg's live D3D11 device; everything created here is owned by
        // `Objects`, and the shared handles are closed when it drops.
        unsafe {
            let device = ID3D11Device::from_raw_borrowed(&source.device.device)?.clone();
            let device5: ID3D11Device5 = device.cast().ok()?;
            let context: ID3D11DeviceContext4 =
                ID3D11DeviceContext4::from_raw_borrowed(&source.device.device_context)?
                    .cast()
                    .ok()?;
            let format = match source.format {
                SurfaceFormat::Nv12 => DXGI_FORMAT_NV12,
                SurfaceFormat::P010 => DXGI_FORMAT_P010,
            };
            let mut handles = Vec::new();
            let mut textures = Vec::new();
            for _ in 0..SLOTS {
                let texture = shared_texture(&device, width, height, format)?;
                let resource: IDXGIResource1 = texture.cast().ok()?;
                let access = DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0;
                handles.push(
                    resource
                        .CreateSharedHandle(None, access, PCWSTR::null())
                        .ok()?,
                );
                textures.push(texture);
            }
            let fence = || -> Option<(ID3D11Fence, HANDLE)> {
                let mut fence: Option<ID3D11Fence> = None;
                device5
                    .CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut fence)
                    .ok()?;
                let fence = fence?;
                let handle = fence
                    .CreateSharedHandle(None, GENERIC_ALL.0, PCWSTR::null())
                    .ok()?;
                Some((fence, handle))
            };
            let (ready, ready_handle) = fence()?;
            let (done, done_handle) = fence()?;
            let texture_handles: Vec<isize> = handles.iter().map(|h| h.0 as isize).collect();
            handles.extend([ready_handle, done_handle]);
            let objects = Arc::new(Objects {
                device,
                context,
                textures,
                ready,
                done,
                handles,
            });
            let surfaces = Arc::new(SharedSurfaces {
                id: NEXT_SET_ID.fetch_add(1, Ordering::Relaxed),
                format: source.format,
                width,
                height,
                textures: texture_handles,
                ready_fence: ready_handle.0 as isize,
                done_fence: done_handle.0 as isize,
                slots: (0..SLOTS).map(|_| Slot::default()).collect(),
                rejected: AtomicBool::new(false),
                owner: Some(Box::new(Arc::clone(&objects))),
            });
            Some(SurfacePool {
                surfaces,
                objects,
                next_ready: 0,
            })
        }
    }

    /// Copies `frame` into a free texture on the GPU. `None` when every texture is still in use
    /// (the caller then downloads the frame instead) or the frame doesn't fit.
    pub fn copy(&mut self, frame: &Frame) -> Option<GpuPicture> {
        let source = frame.d3d11_source()?;
        // SAFETY: GetCompletedValue only reads the fence.
        let completed = unsafe { self.objects.done.GetCompletedValue() };
        let lease = self.surfaces.claim(completed)?;
        let target = &self.objects.textures[lease.index()];
        self.next_ready += 1;
        let region = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: self.surfaces.width,
            bottom: self.surfaces.height,
            back: 1,
        };
        // SAFETY: FFmpeg's device lock is held around every use of its immediate context; the
        // source texture belongs to `frame`, which outlives this call, and the box lies within
        // both textures (decoder textures are at least the frame's even-rounded size).
        unsafe {
            if let Some(lock) = source.device.lock {
                lock(source.device.lock_ctx);
            }
            self.objects.context.CopySubresourceRegion(
                target,
                0,
                0,
                0,
                0,
                source.texture,
                source.slice,
                Some(&region),
            );
            let signalled = self
                .objects
                .context
                .Signal(&self.objects.ready, self.next_ready);
            self.objects.context.Flush();
            if let Some(unlock) = source.device.unlock {
                unlock(source.device.lock_ctx);
            }
            signalled.ok()?;
        }
        Some(GpuPicture {
            lease: Arc::new(lease),
            ready_at: self.next_ready,
        })
    }
}

/// A texture the renderer's D3D12 device can open through an NT handle.
///
/// # Safety
/// `device` must be a live D3D11 device.
unsafe fn shared_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
) -> Option<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32,
    };
    let mut texture = None;
    // SAFETY: `desc` is fully initialized and the out-pointer is valid.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }.ok()?;
    texture
}

fn even(size: u32) -> u32 {
    size.div_ceil(2) * 2
}
