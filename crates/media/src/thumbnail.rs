use std::path::Path;

use cut::{StreamIndex, Ticks, Timebase};

use crate::ffi::{
    CodecContext, FilterGraph, FilterMedia, Frame, Packet, Received, pixel_format_name,
};
use crate::{Demuxer, MediaError, StreamKind};

/// A small straight-alpha RGBA image with tightly packed rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Grabs keyframe previews from a file, independently of playback.
pub struct ThumbnailExtractor {
    demuxer: Demuxer,
    stream: StreamIndex,
    timebase: Timebase,
    codec: CodecContext,
    max_width: u32,
    filter: Option<((u32, u32, i32), FilterGraph)>,
    packet: Packet,
    decoded: Frame,
    scaled: Frame,
}

impl ThumbnailExtractor {
    pub fn open(path: &Path, max_width: u32) -> Result<ThumbnailExtractor, MediaError> {
        let mut demuxer = Demuxer::open(path)?;
        let stream = demuxer
            .best_stream(StreamKind::Video)
            .ok_or_else(|| MediaError::Unsupported("no video stream".into()))?;
        demuxer.select_streams(&[stream]);
        let params = demuxer.stream_params(stream)?;
        Ok(ThumbnailExtractor {
            timebase: params.timebase(),
            codec: CodecContext::decoder(&params, None)?,
            demuxer,
            stream,
            max_width: max_width.max(16),
            filter: None,
            packet: Packet::new()?,
            decoded: Frame::new()?,
            scaled: Frame::new()?,
        })
    }

    /// The keyframe at or before `at` (microseconds), scaled to fit the maximum width.
    pub fn extract(&mut self, at: Ticks) -> Result<Option<RgbaImage>, MediaError> {
        let target = at.rescale(Timebase::MICROS, self.timebase);
        self.demuxer.seek_stream(self.stream, target)?;
        self.codec.flush();
        let mut packets_read = 0;
        while packets_read < 500 {
            if !self.demuxer.read(&mut self.packet)? {
                self.codec.send_packet(None)?;
            } else {
                packets_read += 1;
                if self.packet.stream_index() != self.stream {
                    continue;
                }
                if self.codec.send_packet(Some(&self.packet)).is_err() {
                    continue;
                }
            }
            match self.codec.receive_frame(&mut self.decoded) {
                Ok(Received::Output) => return self.scale().map(Some),
                Ok(Received::Eof) => return Ok(None),
                Ok(Received::Again) | Err(_) => {}
            }
        }
        Ok(None)
    }

    fn scale(&mut self) -> Result<RgbaImage, MediaError> {
        let key = (
            self.decoded.width(),
            self.decoded.height(),
            self.decoded.format(),
        );
        if self.filter.as_ref().is_none_or(|(k, _)| *k != key) {
            let format = pixel_format_name(key.2)
                .ok_or_else(|| MediaError::Unsupported("thumbnail pixel format".into()))?;
            let args = format!(
                "video_size={}x{}:pix_fmt={format}:time_base={}/{}:pixel_aspect=1/1",
                key.0,
                key.1,
                self.timebase.num(),
                self.timebase.den()
            );
            let width = self.max_width.min(key.0.max(2));
            let description = format!("scale={width}:-2:flags=bilinear,format=rgba");
            self.filter = Some((
                key,
                FilterGraph::new(FilterMedia::Video, &args, &description)?,
            ));
        }
        let Some((_, graph)) = &mut self.filter else {
            return Err(MediaError::Unsupported("thumbnail filter".into()));
        };
        graph.push(Some(&self.decoded))?;
        if graph.pull(&mut self.scaled)? != Received::Output {
            return Err(MediaError::Unsupported(
                "thumbnail filter produced nothing".into(),
            ));
        }
        let (width, height) = (self.scaled.width(), self.scaled.height());
        let (bytes, stride) = self
            .scaled
            .plane(0)
            .ok_or_else(|| MediaError::Unsupported("thumbnail plane".into()))?;
        let row = width as usize * 4;
        let pixels = bytes
            .chunks(stride)
            .take(height as usize)
            .flat_map(|line| &line[..row.min(line.len())])
            .copied()
            .collect();
        Ok(RgbaImage {
            width,
            height,
            pixels,
        })
    }
}
