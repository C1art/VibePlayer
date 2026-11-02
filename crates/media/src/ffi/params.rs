use std::ptr::NonNull;

use cut::Timebase;
use ffmpeg_sys_next as sys;

use super::{CodecParams, StreamRef, check};
use crate::MediaError;

/// An owned copy of a stream's codec parameters and timebase, so decoders can live on other
/// threads than the demuxer that found them.
pub struct StreamParams {
    ptr: NonNull<sys::AVCodecParameters>,
    timebase: Timebase,
}

// SAFETY: the parameters are a plain owned struct with owned buffers; nothing else aliases them.
unsafe impl Send for StreamParams {}

impl StreamParams {
    pub fn copy_from(stream: &StreamRef<'_>) -> Result<StreamParams, MediaError> {
        let timebase = stream
            .timebase()
            .ok_or_else(|| MediaError::Unsupported("stream without a timebase".into()))?;
        // SAFETY: no preconditions; null means allocation failure.
        let ptr = NonNull::new(unsafe { sys::avcodec_parameters_alloc() })
            .ok_or(MediaError::OutOfMemory)?;
        let params = StreamParams { ptr, timebase };
        // SAFETY: both parameter structs are valid; the destination is exclusively ours.
        let code = unsafe { sys::avcodec_parameters_copy(ptr.as_ptr(), stream.params().raw()) };
        check(code, "avcodec_parameters_copy")?;
        Ok(params)
    }

    pub fn view(&self) -> CodecParams<'_> {
        // SAFETY: the pointer is valid for the lifetime of `self`.
        CodecParams::from_raw(unsafe { self.ptr.as_ref() })
    }

    pub fn timebase(&self) -> Timebase {
        self.timebase
    }

    pub(super) fn raw_timebase(&self) -> sys::AVRational {
        super::rational(self.timebase)
    }
}

impl Drop for StreamParams {
    fn drop(&mut self) {
        let mut raw = self.ptr.as_ptr();
        // SAFETY: allocated by avcodec_parameters_alloc and freed exactly once.
        unsafe { sys::avcodec_parameters_free(&mut raw) };
    }
}
