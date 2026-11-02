use std::path::{Path, PathBuf};

use cut::{StreamIndex, Ticks, Timebase};

use crate::ffi::{ChapterSpec, InputContext, StreamRef};
use crate::{ColorInfo, StreamKind};

/// Everything the file info panel shows, read once when a file is opened.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub file_size: Option<u64>,
    /// Demuxer name, e.g. `matroska,webm`.
    pub format: String,
    pub format_long: String,
    /// In microseconds.
    pub duration: Option<Ticks>,
    /// In microseconds.
    pub start_time: Ticks,
    pub bit_rate: Option<u64>,
    pub metadata: Vec<(String, String)>,
    pub chapters: Vec<Chapter>,
    pub streams: Vec<StreamInfo>,
}

pub type Chapter = ChapterSpec;

#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub index: StreamIndex,
    pub kind: StreamKind,
    pub codec: String,
    pub codec_long: Option<String>,
    pub profile: Option<String>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub is_default: bool,
    pub bit_rate: Option<u64>,
    pub timebase: Option<Timebase>,
    pub start_time: Option<Ticks>,
    pub duration: Option<Ticks>,
    pub frame_count: Option<u64>,
    pub metadata: Vec<(String, String)>,
    pub details: StreamDetails,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamDetails {
    Video(VideoInfo),
    Audio(AudioInfo),
    Subtitle {
        bitmap: bool,
        width: u32,
        height: u32,
    },
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    /// Average frame rate as `(num, den)`.
    pub frame_rate: Option<(i32, i32)>,
    pub variable_frame_rate: bool,
    pub pixel_format: Option<String>,
    pub bit_depth: Option<u32>,
    pub color: ColorInfo,
    pub interlaced: bool,
    pub sample_aspect_ratio: Option<(i32, i32)>,
    /// Cover art rather than a real video stream.
    pub attached_picture: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u32,
    pub layout: String,
    pub sample_format: Option<String>,
}

impl MediaInfo {
    pub(crate) fn read(path: &Path, input: &InputContext) -> MediaInfo {
        MediaInfo {
            path: path.to_path_buf(),
            file_size: std::fs::metadata(path).ok().map(|m| m.len()),
            format: input.format_name(),
            format_long: input.format_long_name(),
            duration: input.duration(),
            start_time: input.start_time().unwrap_or(Ticks::ZERO),
            bit_rate: input.bit_rate(),
            metadata: input.metadata(),
            chapters: input.chapters(),
            streams: input.streams().map(|stream| stream_info(&stream)).collect(),
        }
    }

    pub fn stream(&self, index: StreamIndex) -> Option<&StreamInfo> {
        self.streams.iter().find(|s| s.index == index)
    }

    pub fn streams_of(&self, kind: StreamKind) -> impl Iterator<Item = &StreamInfo> {
        self.streams.iter().filter(move |s| s.kind == kind)
    }

    pub fn video(&self) -> Option<(&StreamInfo, &VideoInfo)> {
        self.streams
            .iter()
            .find_map(|stream| match &stream.details {
                StreamDetails::Video(video) if !video.attached_picture => Some((stream, video)),
                _ => None,
            })
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

fn stream_info(stream: &StreamRef<'_>) -> StreamInfo {
    let params = stream.params();
    let kind = params.kind();
    let details = match kind {
        StreamKind::Video => {
            let average = stream.average_frame_rate();
            let real = stream.real_frame_rate();
            StreamDetails::Video(VideoInfo {
                width: params.width(),
                height: params.height(),
                frame_rate: average.or(real),
                variable_frame_rate: matches!((average, real), (Some(a), Some(r))
                    if i64::from(a.0) * i64::from(r.1) != i64::from(r.0) * i64::from(a.1)),
                pixel_format: params.pixel_format_name(),
                bit_depth: params.bits_per_raw_sample(),
                color: params.color(),
                interlaced: params.is_interlaced(),
                sample_aspect_ratio: params.sample_aspect_ratio(),
                attached_picture: stream.is_attached_picture(),
            })
        }
        StreamKind::Audio => StreamDetails::Audio(AudioInfo {
            sample_rate: params.sample_rate(),
            channels: params.channel_count(),
            layout: params.channel_layout_description(),
            sample_format: params.sample_format_name(),
        }),
        StreamKind::Subtitle => StreamDetails::Subtitle {
            bitmap: params.is_bitmap_subtitle(),
            width: params.width(),
            height: params.height(),
        },
        StreamKind::Other => StreamDetails::Other,
    };
    StreamInfo {
        index: stream.index(),
        kind,
        codec: params.codec_name(),
        codec_long: params.codec_long_name(),
        profile: params.profile_name(),
        language: stream.language(),
        title: stream.title(),
        is_default: stream.is_default(),
        bit_rate: params.bit_rate(),
        timebase: stream.timebase(),
        start_time: stream.start_time(),
        duration: stream.duration(),
        frame_count: stream.frame_count(),
        metadata: stream.metadata(),
        details,
    }
}
