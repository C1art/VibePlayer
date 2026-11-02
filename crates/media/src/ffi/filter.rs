use std::ffi::{CStr, c_int};
use std::ptr::{self, NonNull};

use cut::Timebase;
use ffmpeg_sys_next as sys;

use super::{AGAIN, Frame, Received, c_string, check, timebase};
use crate::MediaError;

/// A configured libavfilter graph with one buffer source and one buffer sink.
pub struct FilterGraph {
    graph: NonNull<sys::AVFilterGraph>,
    source: NonNull<sys::AVFilterContext>,
    sink: NonNull<sys::AVFilterContext>,
}

// SAFETY: the graph and its filters are owned exclusively by this value and only used through
// `&mut self`; libavfilter graphs have no thread affinity.
unsafe impl Send for FilterGraph {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMedia {
    Video,
    Audio,
}

impl FilterGraph {
    /// Builds `source -> description -> sink`. `source_args` describe the input frames in
    /// buffersrc syntax, e.g. `video_size=1920x1080:pix_fmt=yuv420p:time_base=1/25`.
    pub fn new(
        media: FilterMedia,
        source_args: &str,
        description: &str,
    ) -> Result<FilterGraph, MediaError> {
        // SAFETY: no preconditions; null means allocation failure.
        let graph =
            NonNull::new(unsafe { sys::avfilter_graph_alloc() }).ok_or(MediaError::OutOfMemory)?;
        let mut filter_graph = FilterGraph {
            graph,
            source: NonNull::dangling(),
            sink: NonNull::dangling(),
        };
        let (source_name, sink_name) = match media {
            FilterMedia::Video => (c"buffer", c"buffersink"),
            FilterMedia::Audio => (c"abuffer", c"abuffersink"),
        };
        let source_args = c_string(source_args)?;
        filter_graph.source = filter_graph.create(source_name, c"in", Some(&source_args))?;
        filter_graph.sink = filter_graph.create(sink_name, c"out", None)?;
        filter_graph.link(description)?;
        Ok(filter_graph)
    }

    fn create(
        &mut self,
        filter: &CStr,
        name: &CStr,
        args: Option<&CStr>,
    ) -> Result<NonNull<sys::AVFilterContext>, MediaError> {
        // SAFETY: the name is NUL-terminated; returns a static filter or null.
        let definition = unsafe { sys::avfilter_get_by_name(filter.as_ptr()) };
        if definition.is_null() {
            return Err(MediaError::Unsupported(format!(
                "filter {}",
                filter.to_string_lossy()
            )));
        }
        let mut context = ptr::null_mut();
        // SAFETY: the graph is valid; strings are NUL-terminated; the graph owns the new filter.
        let code = unsafe {
            sys::avfilter_graph_create_filter(
                &mut context,
                definition,
                name.as_ptr(),
                args.map_or(ptr::null(), CStr::as_ptr),
                ptr::null_mut(),
                self.graph.as_ptr(),
            )
        };
        check(code, "avfilter_graph_create_filter")?;
        NonNull::new(context).ok_or(MediaError::OutOfMemory)
    }

    fn link(&mut self, description: &str) -> Result<(), MediaError> {
        let description = c_string(description)?;
        // SAFETY: the in/out lists are allocated here, handed to avfilter_graph_parse_ptr, and
        // freed below whatever happens. Names are av_strdup'd because FFmpeg frees them.
        unsafe {
            let mut outputs = sys::avfilter_inout_alloc();
            let mut inputs = sys::avfilter_inout_alloc();
            let result = if outputs.is_null() || inputs.is_null() {
                Err(MediaError::OutOfMemory)
            } else {
                (*outputs).name = sys::av_strdup(c"in".as_ptr());
                (*outputs).filter_ctx = self.source.as_ptr();
                (*outputs).pad_idx = 0;
                (*outputs).next = ptr::null_mut();
                (*inputs).name = sys::av_strdup(c"out".as_ptr());
                (*inputs).filter_ctx = self.sink.as_ptr();
                (*inputs).pad_idx = 0;
                (*inputs).next = ptr::null_mut();
                check(
                    sys::avfilter_graph_parse_ptr(
                        self.graph.as_ptr(),
                        description.as_ptr(),
                        &mut inputs,
                        &mut outputs,
                        ptr::null_mut(),
                    ),
                    "avfilter_graph_parse_ptr",
                )
                .and_then(|_| {
                    check(
                        sys::avfilter_graph_config(self.graph.as_ptr(), ptr::null_mut()),
                        "avfilter_graph_config",
                    )
                })
                .map(drop)
            };
            sys::avfilter_inout_free(&mut inputs);
            sys::avfilter_inout_free(&mut outputs);
            result
        }
    }

    /// Makes the sink return audio frames of exactly `samples` samples, as encoders require.
    pub fn set_sink_frame_size(&mut self, samples: u32) {
        // SAFETY: the sink is a configured abuffersink of this graph.
        unsafe { sys::av_buffersink_set_frame_size(self.sink.as_ptr(), samples) };
    }

    pub fn sink_timebase(&self) -> Option<Timebase> {
        // SAFETY: the sink is a configured filter of this graph.
        timebase(unsafe { sys::av_buffersink_get_time_base(self.sink.as_ptr()) })
    }

    /// Feeds a frame (kept by the caller), or `None` to signal end of input.
    pub fn push(&mut self, frame: Option<&Frame>) -> Result<(), MediaError> {
        let frame = frame.map_or(ptr::null_mut(), |f| f.as_ptr().cast_mut());
        // SAFETY: the source is a configured buffer filter. With KEEP_REF FFmpeg takes a new
        // reference and never modifies the frame, so passing a shared frame is sound.
        let code = unsafe {
            sys::av_buffersrc_add_frame_flags(
                self.source.as_ptr(),
                frame,
                sys::AV_BUFFERSRC_FLAG_KEEP_REF as c_int,
            )
        };
        check(code, "av_buffersrc_add_frame_flags").map(drop)
    }

    pub fn pull(&mut self, frame: &mut Frame) -> Result<Received, MediaError> {
        frame.unref();
        // SAFETY: the sink is configured and the frame is valid and empty.
        let code = unsafe { sys::av_buffersink_get_frame(self.sink.as_ptr(), frame.as_mut_ptr()) };
        match code {
            AGAIN => Ok(Received::Again),
            sys::AVERROR_EOF => Ok(Received::Eof),
            _ => check(code, "av_buffersink_get_frame").map(|_| Received::Output),
        }
    }
}

impl Drop for FilterGraph {
    fn drop(&mut self) {
        let mut graph = self.graph.as_ptr();
        // SAFETY: the graph was allocated by avfilter_graph_alloc and is freed exactly once,
        // together with every filter it owns.
        unsafe { sys::avfilter_graph_free(&mut graph) };
    }
}
