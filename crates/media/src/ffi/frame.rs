use std::ffi::c_int;
use std::ptr::NonNull;

use cut::Ticks;
use ffmpeg_sys_next as sys;

use super::{check, optional_ts, pixel_format};
use crate::MediaError;
use crate::color::ColorInfo;

/// An owned, reference-counted decoded video frame or audio buffer.
pub struct Frame {
    ptr: NonNull<sys::AVFrame>,
}

// SAFETY: a frame exclusively owns its fields; its buffers are reference counted with atomic
// counts, so moving the frame to another thread is sound.
unsafe impl Send for Frame {}

/// Peak brightness information from HDR side data, in nits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MasteringLuminance {
    pub max_mastering: Option<f32>,
    pub max_content: Option<f32>,
}

// Mirrors libavutil/mastering_display_metadata.h, which the bindings don't include. The layout
// has been stable since FFmpeg 3.
#[repr(C)]
struct MasteringDisplayMetadata {
    display_primaries: [[sys::AVRational; 2]; 3],
    white_point: [sys::AVRational; 2],
    min_luminance: sys::AVRational,
    max_luminance: sys::AVRational,
    has_primaries: c_int,
    has_luminance: c_int,
}

#[repr(C)]
struct ContentLightMetadata {
    max_cll: std::ffi::c_uint,
    max_fall: std::ffi::c_uint,
}

impl Frame {
    pub fn new() -> Result<Frame, MediaError> {
        // SAFETY: av_frame_alloc has no preconditions; null means allocation failure.
        let ptr = unsafe { sys::av_frame_alloc() };
        NonNull::new(ptr)
            .map(|ptr| Frame { ptr })
            .ok_or(MediaError::OutOfMemory)
    }

    pub(super) fn as_ptr(&self) -> *const sys::AVFrame {
        self.ptr.as_ptr()
    }

    pub(super) fn as_mut_ptr(&mut self) -> *mut sys::AVFrame {
        self.ptr.as_ptr()
    }

    fn raw(&self) -> &sys::AVFrame {
        // SAFETY: the pointer is valid and uniquely owned for the frame's lifetime.
        unsafe { self.ptr.as_ref() }
    }

    pub fn width(&self) -> u32 {
        u32::try_from(self.raw().width).unwrap_or(0)
    }

    pub fn height(&self) -> u32 {
        u32::try_from(self.raw().height).unwrap_or(0)
    }

    /// The raw pixel or sample format.
    pub fn format(&self) -> c_int {
        self.raw().format
    }

    /// Best-effort presentation timestamp in the producer's timebase.
    pub fn pts(&self) -> Option<Ticks> {
        let raw = self.raw();
        optional_ts(raw.best_effort_timestamp)
            .or(optional_ts(raw.pts))
            .map(Ticks)
    }

    pub fn set_pts(&mut self, pts: Option<Ticks>) {
        // SAFETY: the frame is valid and `&mut self` guarantees exclusivity.
        let raw = unsafe { self.ptr.as_mut() };
        raw.pts = pts.map_or(sys::AV_NOPTS_VALUE, |t| t.0);
        raw.best_effort_timestamp = raw.pts;
    }

    pub fn duration(&self) -> Ticks {
        Ticks(self.raw().duration.max(0))
    }

    pub fn sample_count(&self) -> usize {
        usize::try_from(self.raw().nb_samples).unwrap_or(0)
    }

    pub fn sample_rate(&self) -> u32 {
        u32::try_from(self.raw().sample_rate).unwrap_or(0)
    }

    pub fn channel_count(&self) -> usize {
        usize::try_from(self.raw().ch_layout.nb_channels).unwrap_or(0)
    }

    pub fn channel_layout_description(&self) -> String {
        // SAFETY: the frame's channel layout is initialized for audio frames and zeroed otherwise.
        unsafe { super::describe_layout(&self.raw().ch_layout) }
    }

    pub fn is_hardware(&self) -> bool {
        !self.raw().hw_frames_ctx.is_null()
    }

    pub fn is_interlaced(&self) -> bool {
        self.raw().flags & sys::AV_FRAME_FLAG_INTERLACED != 0
    }

    pub fn sample_aspect_ratio(&self) -> (i32, i32) {
        let ratio = self.raw().sample_aspect_ratio;
        (ratio.num, ratio.den)
    }

    pub fn color(&self) -> ColorInfo {
        let raw = self.raw();
        super::color::color_info(
            raw.colorspace,
            raw.color_primaries,
            raw.color_trc,
            raw.color_range,
        )
    }

    pub fn mastering_luminance(&self) -> MasteringLuminance {
        let mut luminance = MasteringLuminance::default();
        if let Some(display) = self.side_data::<MasteringDisplayMetadata>(
            sys::AVFrameSideDataType::AV_FRAME_DATA_MASTERING_DISPLAY_METADATA,
        ) && display.has_luminance != 0
            && display.max_luminance.den != 0
        {
            luminance.max_mastering =
                Some(display.max_luminance.num as f32 / display.max_luminance.den as f32);
        }
        if let Some(light) = self.side_data::<ContentLightMetadata>(
            sys::AVFrameSideDataType::AV_FRAME_DATA_CONTENT_LIGHT_LEVEL,
        ) && light.max_cll > 0
        {
            luminance.max_content = Some(light.max_cll as f32);
        }
        luminance
    }

    fn side_data<T>(&self, kind: sys::AVFrameSideDataType) -> Option<&T> {
        // SAFETY: the frame is valid; the returned side data lives as long as the frame.
        let side = unsafe { sys::av_frame_get_side_data(self.as_ptr(), kind).as_ref() }?;
        if side.data.is_null() || side.size < size_of::<T>() {
            return None;
        }
        // SAFETY: FFmpeg stores a `T`-shaped struct for this side data type; the size was
        // checked, and av_malloc'd side data is suitably aligned.
        unsafe { side.data.cast::<T>().as_ref() }
    }

    /// One plane of a video frame as `(bytes, stride)`. `None` for missing planes, hardware
    /// frames, and bottom-up (negative stride) layouts.
    pub fn plane(&self, index: usize) -> Option<(&[u8], usize)> {
        let raw = self.raw();
        if index >= raw.data.len() || raw.data[index].is_null() || self.is_hardware() {
            return None;
        }
        let stride = usize::try_from(raw.linesize[index]).ok()?;
        let format = pixel_format(raw.format)?;
        // SAFETY: av_pix_fmt_desc_get returns a static descriptor or null.
        let descriptor = unsafe { sys::av_pix_fmt_desc_get(format).as_ref() }?;
        let height = usize::try_from(raw.height).ok()?;
        let is_chroma = (index == 1 || index == 2) && descriptor.nb_components >= 3;
        let rows = if is_chroma {
            let shift = descriptor.log2_chroma_h;
            (height + (1 << shift) - 1) >> shift
        } else {
            height
        };
        // SAFETY: FFmpeg allocates at least `stride * rows` bytes for each plane of a frame of this
        // format and height, alive while the frame holds its buffer references.
        let bytes = unsafe { std::slice::from_raw_parts(raw.data[index], stride * rows) };
        Some((bytes, stride))
    }

    /// Interleaved `f32` samples. Empty unless the frame is packed float audio.
    pub fn interleaved_f32(&self) -> &[f32] {
        let raw = self.raw();
        if raw.format != sys::AVSampleFormat::AV_SAMPLE_FMT_FLT as c_int || raw.data[0].is_null() {
            return &[];
        }
        let len = self.sample_count() * self.channel_count();
        // SAFETY: packed float audio stores nb_samples * channels f32 values in data[0], which
        // av_malloc aligns for f32.
        unsafe { std::slice::from_raw_parts(raw.data[0].cast::<f32>(), len) }
    }

    /// Copies a hardware frame into `destination` as a software frame.
    pub fn download_into(&self, destination: &mut Frame) -> Result<(), MediaError> {
        destination.unref();
        // SAFETY: both frames are valid and distinct; the destination is empty so FFmpeg picks
        // the software format and allocates its buffers.
        let code =
            unsafe { sys::av_hwframe_transfer_data(destination.as_mut_ptr(), self.as_ptr(), 0) };
        check(code, "av_hwframe_transfer_data")?;
        // SAFETY: both frames are valid; this copies timestamps and color properties.
        let code = unsafe { sys::av_frame_copy_props(destination.as_mut_ptr(), self.as_ptr()) };
        check(code, "av_frame_copy_props").map(drop)
    }

    pub fn unref(&mut self) {
        // SAFETY: the frame is valid; unref resets it to an empty state.
        unsafe { sys::av_frame_unref(self.as_mut_ptr()) };
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        let mut ptr = self.ptr.as_ptr();
        // SAFETY: the frame was allocated by av_frame_alloc and is freed exactly once here.
        unsafe { sys::av_frame_free(&mut ptr) };
    }
}
