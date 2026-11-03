use std::ffi::c_int;
use std::marker::PhantomData;
use std::path::Path;
use std::ptr::{self, NonNull};

use cut::{StreamIndex, Ticks, Timebase};
use ffmpeg_sys_next as sys;

use super::{
    Packet, check, codec_name, dict, optional_ts, pixel_format_name, string_from, timebase,
};
use crate::color::ColorInfo;
use crate::{MediaError, Rotation, StreamKind};

use super::output::ChapterSpec;

/// Which packets of a stream the demuxer skips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discard {
    Nothing,
    Everything,
}

/// An open input file.
pub struct InputContext {
    ptr: NonNull<sys::AVFormatContext>,
}

// SAFETY: the context is only used through `&mut self` for I/O, and FFmpeg format contexts have
// no thread affinity.
unsafe impl Send for InputContext {}

impl InputContext {
    pub fn open(path: &Path) -> Result<InputContext, MediaError> {
        let path_text = path
            .to_str()
            .ok_or_else(|| MediaError::PathNotUtf8(path.to_path_buf()))?;
        let path_c = super::c_string(path_text)?;
        let mut raw = ptr::null_mut();
        // SAFETY: `raw` is a valid out-pointer and the path is NUL-terminated. On failure FFmpeg
        // frees the context and leaves `raw` null.
        let code = unsafe {
            sys::avformat_open_input(&mut raw, path_c.as_ptr(), ptr::null(), ptr::null_mut())
        };
        check(code, "avformat_open_input")?;
        let ptr = NonNull::new(raw).ok_or(MediaError::OutOfMemory)?;
        let context = InputContext { ptr };
        // SAFETY: the context was just opened successfully.
        let code = unsafe { sys::avformat_find_stream_info(ptr.as_ptr(), ptr::null_mut()) };
        check(code, "avformat_find_stream_info")?;
        Ok(context)
    }

    fn raw(&self) -> &sys::AVFormatContext {
        // SAFETY: the context is valid for the lifetime of `self`.
        unsafe { self.ptr.as_ref() }
    }

    /// Short demuxer name, e.g. `mov,mp4,m4a,3gp,3g2,mj2` or `matroska,webm`.
    pub fn format_name(&self) -> String {
        // SAFETY: an opened input always has a static input format with static names.
        unsafe { self.raw().iformat.as_ref() }
            .and_then(|format| unsafe { string_from(format.name) })
            .unwrap_or_default()
    }

    pub fn format_long_name(&self) -> String {
        // SAFETY: as in `format_name`.
        unsafe { self.raw().iformat.as_ref() }
            .and_then(|format| unsafe { string_from(format.long_name) })
            .unwrap_or_default()
    }

    /// Duration in microseconds.
    pub fn duration(&self) -> Option<Ticks> {
        optional_ts(self.raw().duration)
            .filter(|&d| d > 0)
            .map(Ticks)
    }

    /// Start time in microseconds.
    pub fn start_time(&self) -> Option<Ticks> {
        optional_ts(self.raw().start_time).map(Ticks)
    }

    pub fn bit_rate(&self) -> Option<u64> {
        u64::try_from(self.raw().bit_rate).ok().filter(|&b| b > 0)
    }

    pub fn metadata(&self) -> Vec<(String, String)> {
        // SAFETY: the dictionary belongs to the context and isn't modified while we read it.
        unsafe { dict::entries(self.raw().metadata) }
    }

    pub fn chapters(&self) -> Vec<ChapterSpec> {
        let raw = self.raw();
        let count = usize::try_from(raw.nb_chapters).unwrap_or(0);
        if raw.chapters.is_null() {
            return Vec::new();
        }
        // SAFETY: `chapters` holds `nb_chapters` valid chapter pointers.
        let chapters = unsafe { std::slice::from_raw_parts(raw.chapters, count) };
        chapters
            .iter()
            // SAFETY: each pointer in the array is a valid chapter owned by the context.
            .filter_map(|&chapter| unsafe { chapter.as_ref() })
            .filter_map(|chapter| {
                Some(ChapterSpec {
                    timebase: timebase(chapter.time_base)?,
                    start: Ticks(chapter.start),
                    end: Ticks(chapter.end),
                    // SAFETY: the chapter's dictionary is owned by the context.
                    metadata: unsafe { dict::entries(chapter.metadata) },
                })
            })
            .collect()
    }

    pub fn stream_count(&self) -> usize {
        usize::try_from(self.raw().nb_streams).unwrap_or(0)
    }

    pub fn stream(&self, index: StreamIndex) -> Option<StreamRef<'_>> {
        if index.0 >= self.stream_count() {
            return None;
        }
        // SAFETY: `streams` holds `nb_streams` valid pointers and `index` is in range.
        let stream = unsafe { (*self.raw().streams.add(index.0)).as_ref() }?;
        Some(StreamRef {
            raw: stream,
            _context: PhantomData,
        })
    }

    pub fn streams(&self) -> impl Iterator<Item = StreamRef<'_>> {
        (0..self.stream_count()).filter_map(|index| self.stream(StreamIndex(index)))
    }

    /// The stream FFmpeg considers the main one of a kind, skipping cover art.
    pub fn best_stream(&self, kind: StreamKind) -> Option<StreamIndex> {
        let media_type = match kind {
            StreamKind::Video => sys::AVMediaType::AVMEDIA_TYPE_VIDEO,
            StreamKind::Audio => sys::AVMediaType::AVMEDIA_TYPE_AUDIO,
            StreamKind::Subtitle => sys::AVMediaType::AVMEDIA_TYPE_SUBTITLE,
            StreamKind::Other => return None,
        };
        // SAFETY: the context is valid; we pass no decoder out-pointer.
        let index = unsafe {
            sys::av_find_best_stream(self.ptr.as_ptr(), media_type, -1, -1, ptr::null_mut(), 0)
        };
        let index = StreamIndex(usize::try_from(index).ok()?);
        let stream = self.stream(index)?;
        (!stream.is_attached_picture()).then_some(index)
    }

    /// Reads the next packet. Returns `false` at the end of the file.
    pub fn read_packet(&mut self, packet: &mut Packet) -> Result<bool, MediaError> {
        packet.unref();
        // SAFETY: the context and packet are valid; the packet is empty.
        let code = unsafe { sys::av_read_frame(self.ptr.as_ptr(), packet.as_mut_ptr()) };
        if code == sys::AVERROR_EOF {
            return Ok(false);
        }
        check(code, "av_read_frame").map(|_| true)
    }

    /// Seeks so the next packets start at the last keyframe at or before `target`, given in
    /// `stream`'s timebase, or in microseconds when `stream` is `None`.
    pub fn seek(&mut self, stream: Option<StreamIndex>, target: Ticks) -> Result<(), MediaError> {
        let stream_index = stream.map_or(-1, |s| c_int::try_from(s.0).unwrap_or(-1));
        // SAFETY: the context is valid; the timestamp bounds are ordered.
        let code = unsafe {
            sys::avformat_seek_file(
                self.ptr.as_ptr(),
                stream_index,
                i64::MIN,
                target.0,
                target.0,
                0,
            )
        };
        if code >= 0 {
            return Ok(());
        }
        // Some demuxers can't satisfy the upper bound; fall back to the nearest keyframe after.
        // SAFETY: as above.
        let code = unsafe {
            sys::avformat_seek_file(
                self.ptr.as_ptr(),
                stream_index,
                i64::MIN,
                target.0,
                i64::MAX,
                0,
            )
        };
        check(code, "avformat_seek_file").map(drop)
    }

    /// Tells the demuxer whether to skip a stream's packets. Not every demuxer honors it.
    pub fn set_discard(&mut self, stream: StreamIndex, discard: Discard) {
        if stream.0 >= self.stream_count() {
            return;
        }
        // SAFETY: index checked above; we hold `&mut self`, so nobody else reads the stream.
        unsafe {
            let stream = *self.raw().streams.add(stream.0);
            (*stream).discard = match discard {
                Discard::Nothing => sys::AVDiscard::AVDISCARD_DEFAULT,
                Discard::Everything => sys::AVDiscard::AVDISCARD_ALL,
            };
        }
    }

    /// Timestamps of the keyframes in the container's own index of `stream`, as stored there
    /// (presentation time for Matroska cues, decode time for MP4).
    pub fn indexed_keyframes(&self, stream: StreamIndex) -> Vec<Ticks> {
        let Some(raw) = self.raw_stream(stream) else {
            return Vec::new();
        };
        // SAFETY: `raw` is a stream of this open context; the index is only read here.
        let count = unsafe { sys::avformat_index_get_entries_count(raw) };
        (0..count)
            .filter_map(|index| {
                // SAFETY: `index` is below the entry count; the entry is copied out at once.
                let entry =
                    unsafe { sys::avformat_index_get_entry(raw.cast_mut(), index).as_ref() }?;
                let keyframe = entry.flags() & sys::AVINDEX_KEYFRAME != 0;
                keyframe.then_some(Ticks(entry.timestamp))
            })
            .collect()
    }

    pub(super) fn metadata_ptr(&self) -> *const sys::AVDictionary {
        self.raw().metadata
    }

    pub(super) fn raw_stream(&self, index: StreamIndex) -> Option<*const sys::AVStream> {
        self.stream(index).map(|stream| ptr::from_ref(stream.raw))
    }
}

impl Drop for InputContext {
    fn drop(&mut self) {
        let mut raw = self.ptr.as_ptr();
        // SAFETY: the context was opened by avformat_open_input and is closed exactly once.
        unsafe { sys::avformat_close_input(&mut raw) };
    }
}

/// A borrowed view of one stream of an [`InputContext`].
#[derive(Clone, Copy)]
pub struct StreamRef<'a> {
    raw: &'a sys::AVStream,
    _context: PhantomData<&'a InputContext>,
}

impl<'a> StreamRef<'a> {
    pub fn index(&self) -> StreamIndex {
        StreamIndex(usize::try_from(self.raw.index).unwrap_or(usize::MAX))
    }

    pub fn timebase(&self) -> Option<Timebase> {
        timebase(self.raw.time_base)
    }

    pub fn start_time(&self) -> Option<Ticks> {
        optional_ts(self.raw.start_time).map(Ticks)
    }

    pub fn duration(&self) -> Option<Ticks> {
        optional_ts(self.raw.duration).filter(|&d| d > 0).map(Ticks)
    }

    pub fn frame_count(&self) -> Option<u64> {
        u64::try_from(self.raw.nb_frames).ok().filter(|&n| n > 0)
    }

    /// Average frame rate as `(num, den)`, if the container knows it.
    pub fn average_frame_rate(&self) -> Option<(i32, i32)> {
        let rate = self.raw.avg_frame_rate;
        (rate.num > 0 && rate.den > 0).then_some((rate.num, rate.den))
    }

    pub fn real_frame_rate(&self) -> Option<(i32, i32)> {
        let rate = self.raw.r_frame_rate;
        (rate.num > 0 && rate.den > 0).then_some((rate.num, rate.den))
    }

    pub fn metadata(&self) -> Vec<(String, String)> {
        // SAFETY: the dictionary is owned by the stream, which outlives `self`.
        unsafe { dict::entries(self.raw.metadata) }
    }

    pub fn language(&self) -> Option<String> {
        // SAFETY: as in `metadata`.
        unsafe { dict::get(self.raw.metadata, c"language") }.filter(|l| l != "und")
    }

    pub fn title(&self) -> Option<String> {
        // SAFETY: as in `metadata`.
        unsafe { dict::get(self.raw.metadata, c"title") }
    }

    pub fn is_default(&self) -> bool {
        self.raw.disposition & sys::AV_DISPOSITION_DEFAULT != 0
    }

    pub fn is_attached_picture(&self) -> bool {
        self.raw.disposition & sys::AV_DISPOSITION_ATTACHED_PIC != 0
    }

    pub fn params(&self) -> CodecParams<'a> {
        CodecParams {
            // SAFETY: every stream of an opened input has codec parameters, owned by the stream.
            raw: unsafe { &*self.raw.codecpar },
        }
    }
}

/// A borrowed view of a stream's codec parameters.
#[derive(Clone, Copy)]
pub struct CodecParams<'a> {
    raw: &'a sys::AVCodecParameters,
}

impl<'a> CodecParams<'a> {
    pub(super) fn from_raw(raw: &'a sys::AVCodecParameters) -> CodecParams<'a> {
        CodecParams { raw }
    }

    pub fn kind(&self) -> StreamKind {
        match self.raw.codec_type {
            sys::AVMediaType::AVMEDIA_TYPE_VIDEO => StreamKind::Video,
            sys::AVMediaType::AVMEDIA_TYPE_AUDIO => StreamKind::Audio,
            sys::AVMediaType::AVMEDIA_TYPE_SUBTITLE => StreamKind::Subtitle,
            _ => StreamKind::Other,
        }
    }

    pub fn codec_name(&self) -> String {
        codec_name(self.raw.codec_id)
    }

    pub fn codec_long_name(&self) -> Option<String> {
        // SAFETY: avcodec_descriptor_get returns a static descriptor or null.
        let descriptor = unsafe { sys::avcodec_descriptor_get(self.raw.codec_id).as_ref() }?;
        // SAFETY: descriptor names are static strings.
        unsafe { string_from(descriptor.long_name) }
    }

    pub fn profile_name(&self) -> Option<String> {
        // SAFETY: avcodec_profile_name returns a static string or null.
        unsafe {
            string_from(sys::avcodec_profile_name(
                self.raw.codec_id,
                self.raw.profile,
            ))
        }
    }

    pub fn is_bitmap_subtitle(&self) -> bool {
        // SAFETY: avcodec_descriptor_get returns a static descriptor or null.
        unsafe { sys::avcodec_descriptor_get(self.raw.codec_id).as_ref() }
            .is_some_and(|d| d.props & sys::AV_CODEC_PROP_BITMAP_SUB != 0)
    }

    pub fn width(&self) -> u32 {
        u32::try_from(self.raw.width).unwrap_or(0)
    }

    pub fn height(&self) -> u32 {
        u32::try_from(self.raw.height).unwrap_or(0)
    }

    pub fn pixel_format_name(&self) -> Option<String> {
        if self.kind() != StreamKind::Video {
            return None;
        }
        pixel_format_name(self.raw.format)
    }

    pub fn bits_per_raw_sample(&self) -> Option<u32> {
        u32::try_from(self.raw.bits_per_raw_sample)
            .ok()
            .filter(|&b| b > 0)
    }

    pub fn sample_aspect_ratio(&self) -> Option<(i32, i32)> {
        let ratio = self.raw.sample_aspect_ratio;
        (ratio.num > 0 && ratio.den > 0).then_some((ratio.num, ratio.den))
    }

    pub fn is_interlaced(&self) -> bool {
        !matches!(
            self.raw.field_order,
            sys::AVFieldOrder::AV_FIELD_PROGRESSIVE | sys::AVFieldOrder::AV_FIELD_UNKNOWN
        )
    }

    /// The turn the stream's display matrix asks for, if it has one.
    pub fn rotation(&self) -> Rotation {
        let count = self.raw.nb_coded_side_data;
        // SAFETY: `coded_side_data` holds `nb_coded_side_data` entries owned by the parameters;
        // the lookup returns one of them or null.
        let side_data = unsafe {
            sys::av_packet_side_data_get(
                self.raw.coded_side_data,
                count,
                sys::AVPacketSideDataType::AV_PKT_DATA_DISPLAYMATRIX,
            )
            .as_ref()
        };
        let Some(side_data) = side_data else {
            return Rotation::None;
        };
        // A display matrix is nine 32-bit integers.
        if side_data.data.is_null() || side_data.size < 9 * size_of::<i32>() {
            return Rotation::None;
        }
        // SAFETY: the buffer is at least nine `i32`s long, checked above; FFmpeg allocates
        // side data suitably aligned.
        let degrees = unsafe { sys::av_display_rotation_get(side_data.data.cast::<i32>()) };
        Rotation::from_counterclockwise_degrees(degrees)
    }

    pub fn color(&self) -> ColorInfo {
        super::color::color_info(
            self.raw.color_space,
            self.raw.color_primaries,
            self.raw.color_trc,
            self.raw.color_range,
        )
    }

    pub fn sample_rate(&self) -> u32 {
        u32::try_from(self.raw.sample_rate).unwrap_or(0)
    }

    pub fn channel_count(&self) -> u32 {
        u32::try_from(self.raw.ch_layout.nb_channels).unwrap_or(0)
    }

    pub fn channel_layout_description(&self) -> String {
        // SAFETY: the parameters' channel layout is initialized (zeroed for non-audio streams).
        unsafe { super::describe_layout(&self.raw.ch_layout) }
    }

    pub fn sample_format_name(&self) -> Option<String> {
        if self.kind() != StreamKind::Audio {
            return None;
        }
        super::sample_format_name(self.raw.format)
    }

    pub fn bit_rate(&self) -> Option<u64> {
        u64::try_from(self.raw.bit_rate).ok().filter(|&b| b > 0)
    }

    /// Codec setup data, e.g. an H.264 `avcC` record with SPS and PPS.
    pub fn extradata(&self) -> &'a [u8] {
        let len = usize::try_from(self.raw.extradata_size).unwrap_or(0);
        if self.raw.extradata.is_null() || len == 0 {
            return &[];
        }
        // SAFETY: FFmpeg guarantees `extradata_size` readable bytes at `extradata`, owned by
        // the parameters, which outlive `'a`.
        unsafe { std::slice::from_raw_parts(self.raw.extradata, len) }
    }

    pub(super) fn raw(&self) -> &'a sys::AVCodecParameters {
        self.raw
    }
}
