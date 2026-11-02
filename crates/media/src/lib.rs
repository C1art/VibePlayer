//! Safe FFmpeg wrapper: probe, demux, decode, filter, remux.

mod audio;
mod color;
mod demux;
mod error;
mod ffi;
mod info;
mod remux;
mod subtitle;
mod thumbnail;
mod video;

pub use audio::{AudioDecoder, AudioEffects, AudioFrame, AudioTarget, Decibels, Equalizer, Speed};
pub use color::{ColorInfo, ColorRange, Matrix, Primaries, Transfer};
pub use demux::Demuxer;
pub use error::MediaError;
pub use ffi::{MasteringLuminance, Packet, StreamParams};
pub use info::{AudioInfo, Chapter, MediaInfo, StreamDetails, StreamInfo, VideoInfo};
pub use remux::{AudioEncoder, Muxer};
pub use subtitle::{Subtitle, SubtitleBitmap, SubtitleContent, SubtitleDecoder};
pub use thumbnail::{RgbaImage, ThumbnailExtractor};
pub use video::{HardwareDecoding, PlaneLayout, VideoDecoder, VideoFrame, VideoOptions};

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

/// Reads a file's streams and metadata.
pub fn probe(path: &std::path::Path) -> Result<MediaInfo, MediaError> {
    Demuxer::open(path).map(|demuxer| demuxer.info())
}
