use std::path::Path;

use cut::{StreamIndex, Ticks, Timebase};

use crate::ffi::{
    CodecContext, FilterGraph, FilterMedia, Frame, Packet, Received, pixel_format_name,
};
use crate::{Demuxer, MediaError, Rotation, StreamKind};

/// A small straight-alpha RGBA image with tightly packed rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Width, height, pixel format and sample aspect: a change needs a new filter.
type FilterKey = (u32, u32, i32, (i32, i32));

/// Grabs keyframe previews from a file, independently of playback.
pub struct ThumbnailExtractor {
    demuxer: Demuxer,
    stream: StreamIndex,
    timebase: Timebase,
    codec: CodecContext,
    /// The file's start time in microseconds; positions asked for are relative to it.
    origin: Ticks,
    max_width: u32,
    rotation: Rotation,
    filter: Option<(FilterKey, FilterGraph)>,
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
        let origin = demuxer.input().start_time().unwrap_or(Ticks::ZERO);
        Ok(ThumbnailExtractor {
            origin,
            timebase: params.timebase(),
            codec: CodecContext::decoder(&params, None)?,
            demuxer,
            stream,
            max_width: max_width.max(16),
            rotation: params.view().rotation(),
            filter: None,
            packet: Packet::new()?,
            decoded: Frame::new()?,
            scaled: Frame::new()?,
        })
    }

    /// The file's duration in microseconds, if the container says.
    pub fn duration(&self) -> Option<Ticks> {
        self.demuxer.input().duration()
    }

    /// The keyframe at or before `at` (microseconds from the start of the file), scaled to fit
    /// the maximum width at the picture's display aspect, then turned upright.
    pub fn extract(&mut self, at: Ticks) -> Result<Option<RgbaImage>, MediaError> {
        let target = (at + self.origin).rescale(Timebase::MICROS, self.timebase);
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
            self.decoded.sample_aspect_ratio(),
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
            let (width, height) = thumbnail_size(key.0, key.1, key.3, self.max_width);
            let turn = match self.rotation {
                Rotation::None => "",
                Rotation::Clockwise90 => ",transpose=clock",
                Rotation::Half => ",hflip,vflip",
                Rotation::Clockwise270 => ",transpose=cclock",
            };
            let description = format!("scale={width}:{height}:flags=bilinear{turn},format=rgba");
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

/// The size to scale a `width` × `height` picture with `sample_aspect` pixels to so it shows at
/// its display aspect, no wider than `max_width`. Even, for chroma-subsampled formats.
fn thumbnail_size(
    width: u32,
    height: u32,
    sample_aspect: (i32, i32),
    max_width: u32,
) -> (u32, u32) {
    let (num, den) = sample_aspect;
    let aspect = if num > 0 && den > 0 {
        f64::from(num) / f64::from(den)
    } else {
        1.0
    };
    let display_width = (f64::from(width.max(1)) * aspect).max(1.0);
    let even = |value: f64| ((value / 2.0).round() as u32 * 2).max(2);
    let out_width = even(display_width.min(f64::from(max_width)));
    let out_height = even(f64::from(out_width) * f64::from(height.max(1)) / display_width);
    (out_width, out_height)
}

#[cfg(test)]
mod tests {
    use super::thumbnail_size;

    #[test]
    fn anamorphic_pictures_are_scaled_to_their_display_aspect() {
        // DVD: 720×480 with 32:27 pixels shows at 16:9.
        assert_eq!(thumbnail_size(720, 480, (32, 27), 160), (160, 90));
        assert_eq!(thumbnail_size(320, 180, (0, 1), 160), (160, 90));
        assert_eq!(thumbnail_size(100, 50, (1, 1), 160), (100, 50));
    }
}
