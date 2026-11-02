use std::ffi::c_int;
use std::mem::MaybeUninit;

use ffmpeg_sys_next as sys;

use super::{CodecContext, Packet, check, string_from};
use crate::MediaError;

/// A decoded subtitle event, copied out of FFmpeg.
#[derive(Clone, Debug, PartialEq)]
pub struct RawSubtitle {
    /// Display window relative to the packet timestamp, in milliseconds.
    pub start_millis: u32,
    pub end_millis: Option<u32>,
    pub rects: Vec<RawSubtitleRect>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RawSubtitleRect {
    /// An ASS dialogue line (`ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,Effect,Text`)
    /// or plain text.
    Text(String),
    /// Straight-alpha RGBA pixels positioned in the video's coordinate space.
    Bitmap {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

impl CodecContext {
    /// Decodes one subtitle packet. `None` if the packet produced no event.
    pub fn decode_subtitle(&mut self, packet: &Packet) -> Result<Option<RawSubtitle>, MediaError> {
        let mut subtitle = MaybeUninit::<sys::AVSubtitle>::zeroed();
        let mut got: c_int = 0;
        // SAFETY: the context is an open subtitle decoder; the zeroed subtitle is a valid output.
        let code = unsafe {
            sys::avcodec_decode_subtitle2(
                self.as_mut_ptr(),
                subtitle.as_mut_ptr(),
                &mut got,
                packet.as_ptr(),
            )
        };
        check(code, "avcodec_decode_subtitle2")?;
        if got == 0 {
            return Ok(None);
        }
        // SAFETY: the decoder filled the subtitle because `got` is set.
        let mut subtitle = unsafe { subtitle.assume_init() };
        let converted = convert(&subtitle);
        // SAFETY: frees what the decoder allocated inside the subtitle, exactly once.
        unsafe { sys::avsubtitle_free(&mut subtitle) };
        Ok(Some(converted))
    }
}

fn convert(subtitle: &sys::AVSubtitle) -> RawSubtitle {
    let count = usize::try_from(subtitle.num_rects).unwrap_or(0);
    let rects = if subtitle.rects.is_null() || count == 0 {
        &[][..]
    } else {
        // SAFETY: `rects` holds `num_rects` pointers to rects owned by the subtitle.
        unsafe { std::slice::from_raw_parts(subtitle.rects, count) }
    };
    RawSubtitle {
        start_millis: subtitle.start_display_time,
        end_millis: (subtitle.end_display_time != 0 && subtitle.end_display_time != u32::MAX)
            .then_some(subtitle.end_display_time),
        rects: rects
            .iter()
            // SAFETY: each non-null rect is valid while the subtitle is.
            .filter_map(|&rect| unsafe { rect.as_ref() })
            .filter_map(convert_rect)
            .collect(),
    }
}

fn convert_rect(rect: &sys::AVSubtitleRect) -> Option<RawSubtitleRect> {
    match rect.type_ {
        sys::AVSubtitleType::SUBTITLE_ASS => {
            // SAFETY: `ass` is null or a NUL-terminated string owned by the rect.
            unsafe { string_from(rect.ass) }.map(RawSubtitleRect::Text)
        }
        sys::AVSubtitleType::SUBTITLE_TEXT => {
            // SAFETY: `text` is null or a NUL-terminated string owned by the rect.
            unsafe { string_from(rect.text) }.map(RawSubtitleRect::Text)
        }
        sys::AVSubtitleType::SUBTITLE_BITMAP => palette_to_rgba(rect),
        _ => None,
    }
}

fn palette_to_rgba(rect: &sys::AVSubtitleRect) -> Option<RawSubtitleRect> {
    let width = usize::try_from(rect.w).ok().filter(|&w| w > 0)?;
    let height = usize::try_from(rect.h).ok().filter(|&h| h > 0)?;
    let stride = usize::try_from(rect.linesize[0])
        .ok()
        .filter(|&s| s >= width)?;
    let colors = usize::try_from(rect.nb_colors).ok()?.min(256);
    if rect.data[0].is_null() || rect.data[1].is_null() {
        return None;
    }
    // SAFETY: bitmap rects hold `linesize[0] * h` palette indices in data[0] and `nb_colors`
    // native-endian 0xAARRGGBB entries in data[1], all owned by the rect.
    let (indices, palette) = unsafe {
        (
            std::slice::from_raw_parts(rect.data[0], stride * height),
            std::slice::from_raw_parts(rect.data[1].cast::<u32>(), colors),
        )
    };
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in indices.chunks_exact(stride) {
        for &index in &row[..width] {
            let argb = palette.get(usize::from(index)).copied().unwrap_or(0);
            let [a, r, g, b] = argb.to_be_bytes();
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    Some(RawSubtitleRect::Bitmap {
        x: rect.x,
        y: rect.y,
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
        rgba,
    })
}
