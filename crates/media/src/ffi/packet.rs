use std::ptr::NonNull;

use cut::{StreamIndex, Ticks};
use ffmpeg_sys_next as sys;

use super::optional_ts;
use crate::MediaError;

/// An owned, reference-counted compressed packet.
pub struct Packet {
    ptr: NonNull<sys::AVPacket>,
}

// SAFETY: a packet exclusively owns its fields; its data buffer is reference counted with atomic
// counts, so moving the packet to another thread is sound.
unsafe impl Send for Packet {}

impl Packet {
    pub fn new() -> Result<Packet, MediaError> {
        // SAFETY: av_packet_alloc has no preconditions; null means allocation failure.
        let ptr = unsafe { sys::av_packet_alloc() };
        NonNull::new(ptr)
            .map(|ptr| Packet { ptr })
            .ok_or(MediaError::OutOfMemory)
    }

    pub(super) fn as_ptr(&self) -> *const sys::AVPacket {
        self.ptr.as_ptr()
    }

    pub(super) fn as_mut_ptr(&mut self) -> *mut sys::AVPacket {
        self.ptr.as_ptr()
    }

    fn raw(&self) -> &sys::AVPacket {
        // SAFETY: the pointer is valid and uniquely owned for the packet's lifetime.
        unsafe { self.ptr.as_ref() }
    }

    fn raw_mut(&mut self) -> &mut sys::AVPacket {
        // SAFETY: as in `raw`, and `&mut self` guarantees exclusivity.
        unsafe { self.ptr.as_mut() }
    }

    pub fn stream_index(&self) -> StreamIndex {
        StreamIndex(usize::try_from(self.raw().stream_index).unwrap_or(usize::MAX))
    }

    pub fn set_stream_index(&mut self, index: StreamIndex) {
        self.raw_mut().stream_index = i32::try_from(index.0).unwrap_or(i32::MAX);
    }

    pub fn pts(&self) -> Option<Ticks> {
        optional_ts(self.raw().pts).map(Ticks)
    }

    pub fn dts(&self) -> Option<Ticks> {
        optional_ts(self.raw().dts).map(Ticks)
    }

    pub fn duration(&self) -> Ticks {
        Ticks(self.raw().duration.max(0))
    }

    pub fn set_timestamps(&mut self, pts: Option<Ticks>, dts: Option<Ticks>) {
        let raw = self.raw_mut();
        raw.pts = pts.map_or(sys::AV_NOPTS_VALUE, |t| t.0);
        raw.dts = dts.map_or(sys::AV_NOPTS_VALUE, |t| t.0);
    }

    pub fn set_duration(&mut self, duration: Ticks) {
        self.raw_mut().duration = duration.0;
    }

    /// Clears the byte position so a muxer doesn't trust the source file's offsets.
    pub fn clear_position(&mut self) {
        self.raw_mut().pos = -1;
    }

    pub fn is_keyframe(&self) -> bool {
        self.raw().flags & sys::AV_PKT_FLAG_KEY != 0
    }

    pub fn is_corrupt(&self) -> bool {
        self.raw().flags & sys::AV_PKT_FLAG_CORRUPT != 0
    }

    pub fn data(&self) -> &[u8] {
        let raw = self.raw();
        let len = usize::try_from(raw.size).unwrap_or(0);
        if raw.data.is_null() || len == 0 {
            return &[];
        }
        // SAFETY: FFmpeg guarantees `data` points to at least `size` readable bytes, alive as
        // long as the packet holds its reference.
        unsafe { std::slice::from_raw_parts(raw.data, len) }
    }

    pub fn try_clone(&self) -> Result<Packet, MediaError> {
        // SAFETY: the source packet is valid; av_packet_clone returns a new reference or null.
        let ptr = unsafe { sys::av_packet_clone(self.as_ptr()) };
        NonNull::new(ptr)
            .map(|ptr| Packet { ptr })
            .ok_or(MediaError::OutOfMemory)
    }

    pub fn unref(&mut self) {
        // SAFETY: the packet is valid; unref resets it to an empty state.
        unsafe { sys::av_packet_unref(self.as_mut_ptr()) };
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        let mut ptr = self.ptr.as_ptr();
        // SAFETY: the packet was allocated by av_packet_alloc and is freed exactly once here.
        unsafe { sys::av_packet_free(&mut ptr) };
    }
}
