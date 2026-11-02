use std::ffi::c_uint;
use std::path::Path;
use std::ptr::{self, NonNull};

use cut::{StreamIndex, Ticks, Timebase};
use ffmpeg_sys_next as sys;

use super::{CodecContext, InputContext, Packet, c_string, check, dict, rational, timebase};
use crate::MediaError;

/// A chapter as read from an input or written to an output.
#[derive(Clone, Debug, PartialEq)]
pub struct ChapterSpec {
    pub timebase: Timebase,
    pub start: Ticks,
    pub end: Ticks,
    pub metadata: Vec<(String, String)>,
}

/// An output file being muxed.
pub struct OutputContext {
    ptr: NonNull<sys::AVFormatContext>,
    header_written: bool,
}

// SAFETY: only used through `&mut self`; format contexts have no thread affinity.
unsafe impl Send for OutputContext {}

impl OutputContext {
    /// Creates `path` with the muxer named `format`, e.g. `mp4` or `matroska`, or the one its
    /// extension implies.
    pub fn create(path: &Path, format: Option<&str>) -> Result<OutputContext, MediaError> {
        let path_text = path
            .to_str()
            .ok_or_else(|| MediaError::PathNotUtf8(path.to_path_buf()))?;
        let path_c = c_string(path_text)?;
        let format_c = format.map(c_string).transpose()?;
        let mut raw = ptr::null_mut();
        // SAFETY: `raw` is a valid out-pointer; strings are NUL-terminated.
        let code = unsafe {
            sys::avformat_alloc_output_context2(
                &mut raw,
                ptr::null(),
                format_c.as_ref().map_or(ptr::null(), |f| f.as_ptr()),
                path_c.as_ptr(),
            )
        };
        check(code, "avformat_alloc_output_context2")?;
        let ptr = NonNull::new(raw).ok_or(MediaError::OutOfMemory)?;
        let output = OutputContext {
            ptr,
            header_written: false,
        };
        // SAFETY: the context is valid; its output format is a static struct.
        let needs_file = unsafe { (*(*raw).oformat).flags } & sys::AVFMT_NOFILE == 0;
        if needs_file {
            // SAFETY: `pb` is a valid out-pointer owned by the context; closed in Drop.
            let code =
                unsafe { sys::avio_open(&mut (*raw).pb, path_c.as_ptr(), sys::AVIO_FLAG_WRITE) };
            check(code, "avio_open")?;
        }
        Ok(output)
    }

    fn raw(&self) -> &sys::AVFormatContext {
        // SAFETY: valid for the lifetime of `self`.
        unsafe { self.ptr.as_ref() }
    }

    fn output_format(&self) -> &sys::AVOutputFormat {
        // SAFETY: an allocated output context always has a static output format.
        unsafe { &*self.raw().oformat }
    }

    pub fn needs_global_header(&self) -> bool {
        self.output_format().flags & sys::AVFMT_GLOBALHEADER != 0
    }

    fn new_stream(&mut self) -> Result<*mut sys::AVStream, MediaError> {
        // SAFETY: the context is valid and its header not yet written.
        let stream = unsafe { sys::avformat_new_stream(self.ptr.as_ptr(), ptr::null()) };
        if stream.is_null() {
            return Err(MediaError::OutOfMemory);
        }
        Ok(stream)
    }

    /// Adds a stream that will receive packets copied unchanged from `input`'s `source` stream.
    pub fn add_copied_stream(
        &mut self,
        input: &InputContext,
        source: StreamIndex,
    ) -> Result<StreamIndex, MediaError> {
        let source_stream = input
            .raw_stream(source)
            .ok_or(MediaError::NoSuchStream(source))?;
        let stream = self.new_stream()?;
        let tags = self.output_format().codec_tag;
        // SAFETY: both streams are valid; the output stream's parameters are ours to fill.
        unsafe {
            let source_stream = &*source_stream;
            check(
                sys::avcodec_parameters_copy((*stream).codecpar, source_stream.codecpar),
                "avcodec_parameters_copy",
            )?;
            let params = &mut *(*stream).codecpar;
            if !tag_is_valid(tags, params.codec_id, params.codec_tag) {
                params.codec_tag = 0;
            }
            (*stream).time_base = source_stream.time_base;
            (*stream).sample_aspect_ratio = source_stream.sample_aspect_ratio;
            (*stream).avg_frame_rate = source_stream.avg_frame_rate;
            (*stream).r_frame_rate = source_stream.r_frame_rate;
            (*stream).disposition = source_stream.disposition;
            dict::copy(&mut (*stream).metadata, source_stream.metadata);
            Ok(StreamIndex(usize::try_from((*stream).index).unwrap_or(0)))
        }
    }

    /// Adds a stream fed by `encoder`, keeping `input`'s `source` stream metadata.
    pub fn add_encoded_stream(
        &mut self,
        encoder: &mut CodecContext,
        input: &InputContext,
        source: StreamIndex,
    ) -> Result<StreamIndex, MediaError> {
        let source_stream = input
            .raw_stream(source)
            .ok_or(MediaError::NoSuchStream(source))?;
        let stream = self.new_stream()?;
        // SAFETY: the encoder is open; both streams are valid.
        unsafe {
            check(
                sys::avcodec_parameters_from_context((*stream).codecpar, encoder.as_ptr()),
                "avcodec_parameters_from_context",
            )?;
            (*stream).time_base = (*encoder.as_ptr()).time_base;
            (*stream).disposition = (*source_stream).disposition;
            dict::copy(&mut (*stream).metadata, (*source_stream).metadata);
            Ok(StreamIndex(usize::try_from((*stream).index).unwrap_or(0)))
        }
    }

    pub fn copy_metadata_from(&mut self, input: &InputContext) {
        let source = input.metadata_ptr();
        // SAFETY: both dictionaries are valid; the output's metadata is ours to modify.
        unsafe { dict::copy(&mut (*self.ptr.as_ptr()).metadata, source) };
    }

    /// Replaces the output's chapters.
    pub fn set_chapters(&mut self, chapters: &[ChapterSpec]) -> Result<(), MediaError> {
        let raw = self.ptr.as_ptr();
        // SAFETY: we build the chapter array exactly as libavformat's own demuxers do: an
        // av_malloc'd array of av_mallocz'd chapters, which avformat_free_context frees.
        unsafe {
            if !(*raw).chapters.is_null() || (*raw).nb_chapters != 0 {
                return Err(MediaError::Unsupported("chapters already set".into()));
            }
            if chapters.is_empty() {
                return Ok(());
            }
            let array = sys::av_mallocz(size_of::<*mut sys::AVChapter>() * chapters.len())
                .cast::<*mut sys::AVChapter>();
            if array.is_null() {
                return Err(MediaError::OutOfMemory);
            }
            (*raw).chapters = array;
            for (id, spec) in chapters.iter().enumerate() {
                let chapter = sys::av_mallocz(size_of::<sys::AVChapter>()).cast::<sys::AVChapter>();
                if chapter.is_null() {
                    return Err(MediaError::OutOfMemory);
                }
                (*chapter).id = i64::try_from(id).unwrap_or(i64::MAX);
                (*chapter).time_base = rational(spec.timebase);
                (*chapter).start = spec.start.0;
                (*chapter).end = spec.end.0;
                for (key, value) in &spec.metadata {
                    let (key, value) = (c_string(key)?, c_string(value)?);
                    sys::av_dict_set(&mut (*chapter).metadata, key.as_ptr(), value.as_ptr(), 0);
                }
                *array.add(id) = chapter;
                (*raw).nb_chapters = c_uint::try_from(id + 1).unwrap_or(c_uint::MAX);
            }
        }
        Ok(())
    }

    /// Writes the header. Timestamps are shifted so the earliest one is zero.
    pub fn write_header(&mut self) -> Result<(), MediaError> {
        // SAFETY: the context is valid; streams are set up; options are null.
        unsafe {
            (*self.ptr.as_ptr()).avoid_negative_ts = sys::AVFMT_AVOID_NEG_TS_MAKE_ZERO;
            check(
                sys::avformat_write_header(self.ptr.as_ptr(), ptr::null_mut()),
                "avformat_write_header",
            )?;
        }
        self.header_written = true;
        Ok(())
    }

    /// The timebase the muxer chose for a stream; only final after `write_header`.
    pub fn stream_timebase(&self, stream: StreamIndex) -> Option<Timebase> {
        let raw = self.raw();
        if stream.0 >= usize::try_from(raw.nb_streams).unwrap_or(0) {
            return None;
        }
        // SAFETY: index checked; the stream pointer is valid.
        timebase(unsafe { (**raw.streams.add(stream.0)).time_base })
    }

    /// Writes a packet already stamped in the output stream's timebase. The packet is left empty.
    pub fn write_packet(&mut self, packet: &mut Packet) -> Result<(), MediaError> {
        // SAFETY: the header was written; av_interleaved_write_frame takes the packet's reference.
        let code =
            unsafe { sys::av_interleaved_write_frame(self.ptr.as_ptr(), packet.as_mut_ptr()) };
        check(code, "av_interleaved_write_frame").map(drop)
    }

    pub fn finish(mut self) -> Result<(), MediaError> {
        if !self.header_written {
            return Err(MediaError::Unsupported("trailer before header".into()));
        }
        self.header_written = false;
        // SAFETY: the header was written (checked above) and the trailer is written once.
        let code = unsafe { sys::av_write_trailer(self.ptr.as_ptr()) };
        check(code, "av_write_trailer").map(drop)
    }
}

impl Drop for OutputContext {
    fn drop(&mut self) {
        let raw = self.ptr.as_ptr();
        // SAFETY: the context was allocated by avformat_alloc_output_context2; `pb` is closed if
        // we opened it, then the context and everything it owns are freed exactly once.
        unsafe {
            if (*(*raw).oformat).flags & sys::AVFMT_NOFILE == 0 {
                sys::avio_closep(&mut (*raw).pb);
            }
            sys::avformat_free_context(raw);
        }
    }
}

/// Mirrors ffmpeg's own rule for keeping a stream-copied codec tag.
fn tag_is_valid(tags: *const *const sys::AVCodecTag, codec: sys::AVCodecID, tag: u32) -> bool {
    if tags.is_null() {
        return true;
    }
    let mut preferred: c_uint = 0;
    // SAFETY: `tags` is a valid tag table of the static output format.
    unsafe {
        sys::av_codec_get_id(tags, tag) == codec
            || sys::av_codec_get_tag2(tags, codec, &mut preferred) == 0
    }
}
