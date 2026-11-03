//! Safe FFmpeg wrapper: probe, demux, decode, filter, remux.

mod audio;
mod color;
mod demux;
mod encode;
mod error;
mod ffi;
mod gpu;
mod info;
mod keyframe_scan;
mod keyframes;
mod remux;
mod replay;
mod rotation;
mod subtitle;
mod thumbnail;
mod video;
mod video_frame;

pub use audio::{AudioDecoder, AudioEffects, AudioFrame, AudioTarget, Decibels, Equalizer, Speed};
pub use color::{ColorInfo, ColorRange, Matrix, Primaries, Transfer};
pub use demux::Demuxer;
pub use encode::{VideoEncoder, smart_cut_encoder};
pub use error::MediaError;
pub use ffi::{MasteringLuminance, Packet, StreamParams};
pub use gpu::{GpuPicture, GpuSharing, SharedSurfaces, SlotLease, SurfaceFormat};
pub use info::{AudioInfo, Chapter, MediaInfo, StreamDetails, StreamInfo, VideoInfo};
pub use keyframes::KeyframeSpan;
pub use remux::{AudioEncoder, Muxer};
pub use rotation::Rotation;
pub use subtitle::{Subtitle, SubtitleBitmap, SubtitleContent, SubtitleDecoder};
pub use thumbnail::{RgbaImage, ThumbnailExtractor};
pub use video::VideoDecoder;
pub use video_frame::{HardwareDecoding, PlaneLayout, VideoFrame, VideoOptions};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    Other,
}

/// One-time FFmpeg setup; safe to call repeatedly.
pub fn init() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(ffi::quiet_logging);
}

/// The FFmpeg build in use, for diagnostics.
pub fn ffmpeg_version() -> String {
    ffi::version()
}

/// Reads a file's streams and metadata.
pub fn probe(path: &std::path::Path) -> Result<MediaInfo, MediaError> {
    Demuxer::open(path).map(|demuxer| demuxer.info())
}
