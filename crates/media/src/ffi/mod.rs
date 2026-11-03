//! The only place FFmpeg pointers and `unsafe` may live. Everything leaving this module is safe.

mod codec;
mod color;
#[cfg(windows)]
mod d3d11;
mod dict;
mod filter;
mod frame;
mod input;
mod output;
mod packet;
mod params;
mod subtitle;

use std::ffi::{CStr, CString, c_char, c_int};

use cut::Timebase;
use ffmpeg_sys_next as sys;

use crate::MediaError;

pub use codec::{CodecContext, HwDevice, Received};
#[cfg(windows)]
pub use d3d11::{SurfacePool, adapter_index};
pub use filter::{FilterGraph, FilterMedia};
pub use frame::{Frame, MasteringLuminance};
pub use input::{CodecParams, Discard, InputContext, StreamRef};
pub use output::{ChapterSpec, OutputContext};
pub use packet::Packet;
pub use params::StreamParams;
pub use subtitle::RawSubtitleRect;

/// Lowers FFmpeg's own logging (errors only in debug builds, nothing in release); problems
/// are reported through our own types.
/// The FFmpeg build's version string, e.g. `n8.1`.
pub fn version() -> String {
    // SAFETY: av_version_info returns a static NUL-terminated string.
    unsafe { string_from(sys::av_version_info()) }.unwrap_or_default()
}

pub fn quiet_logging() {
    let level = if cfg!(debug_assertions) {
        sys::AV_LOG_ERROR
    } else {
        sys::AV_LOG_QUIET
    };
    // SAFETY: av_log_set_level only stores an integer.
    unsafe { sys::av_log_set_level(level) };
}

fn check(code: c_int, operation: &'static str) -> Result<c_int, MediaError> {
    if code >= 0 {
        Ok(code)
    } else {
        Err(MediaError::Ffmpeg {
            operation,
            code,
            message: error_message(code),
        })
    }
}

fn error_message(code: c_int) -> String {
    let mut buffer = [0 as c_char; 128];
    // SAFETY: the buffer is writable for its full length and av_strerror NUL-terminates within it.
    unsafe {
        sys::av_strerror(code, buffer.as_mut_ptr(), buffer.len());
        CStr::from_ptr(buffer.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

const AGAIN: c_int = sys::AVERROR(sys::EAGAIN);

/// # Safety
/// `ptr` must be null or point to a NUL-terminated string that outlives the call.
unsafe fn string_from(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        None
    } else {
        // SAFETY: the caller guarantees a valid NUL-terminated string.
        Some(
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

fn c_string(text: &str) -> Result<CString, MediaError> {
    CString::new(text).map_err(|_| MediaError::Unsupported(format!("NUL byte in {text:?}")))
}

fn timebase(rational: sys::AVRational) -> Option<Timebase> {
    Timebase::new(i64::from(rational.num), i64::from(rational.den))
}

fn rational(timebase: Timebase) -> sys::AVRational {
    sys::AVRational {
        num: c_int::try_from(timebase.num()).unwrap_or(c_int::MAX),
        den: c_int::try_from(timebase.den()).unwrap_or(c_int::MAX),
    }
}

fn optional_ts(value: i64) -> Option<i64> {
    (value != sys::AV_NOPTS_VALUE).then_some(value)
}

fn codec_name(id: sys::AVCodecID) -> String {
    // SAFETY: avcodec_get_name always returns a static string, "unknown_codec" at worst.
    unsafe { string_from(sys::avcodec_get_name(id)) }.unwrap_or_default()
}

pub(crate) fn pixel_format_name(format: c_int) -> Option<String> {
    let format = pixel_format(format)?;
    // SAFETY: av_get_pix_fmt_name returns a static string or null.
    unsafe { string_from(sys::av_get_pix_fmt_name(format)) }
}

/// Maps a raw format integer to the enum, only for values FFmpeg itself knows.
fn pixel_format(format: c_int) -> Option<sys::AVPixelFormat> {
    if format < 0 || format >= sys::AVPixelFormat::AV_PIX_FMT_NB as c_int {
        return None;
    }
    // SAFETY: AVPixelFormat is a C enum with contiguous discriminants from 0 to AV_PIX_FMT_NB,
    // and `format` was range-checked above.
    Some(unsafe { std::mem::transmute::<c_int, sys::AVPixelFormat>(format) })
}

fn sample_format(format: c_int) -> Option<sys::AVSampleFormat> {
    if format < 0 || format >= sys::AVSampleFormat::AV_SAMPLE_FMT_NB as c_int {
        return None;
    }
    // SAFETY: AVSampleFormat is contiguous from 0 to AV_SAMPLE_FMT_NB and `format` is in range.
    Some(unsafe { std::mem::transmute::<c_int, sys::AVSampleFormat>(format) })
}

pub(crate) fn sample_format_name(format: c_int) -> Option<String> {
    let format = sample_format(format)?;
    // SAFETY: av_get_sample_fmt_name returns a static string or null.
    unsafe { string_from(sys::av_get_sample_fmt_name(format)) }
}

/// # Safety
/// `layout` must point to an initialized channel layout.
unsafe fn describe_layout(layout: *const sys::AVChannelLayout) -> String {
    let mut buffer = [0 as c_char; 128];
    // SAFETY: the caller guarantees `layout` is valid; the buffer is writable and NUL-terminated
    // by av_channel_layout_describe on success.
    unsafe {
        if sys::av_channel_layout_describe(layout, buffer.as_mut_ptr(), buffer.len()) < 0 {
            return String::new();
        }
        CStr::from_ptr(buffer.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}
