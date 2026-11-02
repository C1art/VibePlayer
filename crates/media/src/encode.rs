use cut::{Ticks, Timebase};

use crate::ffi::{CodecContext, Packet, Received, StreamParams};
use crate::{MediaError, VideoFrame};

/// Re-encodes a few frames so a cut can start between keyframes. Settings favour quality over
/// size, since only the stretch up to the next keyframe passes through it.
pub struct VideoEncoder {
    codec: CodecContext,
}

/// The encoder that can continue a stream of this codec, if FFmpeg has it.
pub fn smart_cut_encoder(codec: &str) -> Option<&'static str> {
    let name = match codec {
        "h264" => "libx264",
        "hevc" => "libx265",
        _ => return None,
    };
    CodecContext::encoder_exists(name).then_some(name)
}

impl VideoEncoder {
    /// An encoder producing frames like `first`, stamped in `timebase`.
    pub fn for_smart_cut(
        source: &StreamParams,
        first: &VideoFrame,
        timebase: Timebase,
        frame_rate: Option<(i32, i32)>,
    ) -> Result<VideoEncoder, MediaError> {
        let view = source.view();
        let name = smart_cut_encoder(&view.codec_name())
            .ok_or_else(|| MediaError::NoEncoder(view.codec_name()))?;
        let options: &[(&str, &str)] = match name {
            "libx264" => &[("preset", "medium"), ("crf", "15")],
            _ => &[
                ("preset", "medium"),
                ("crf", "17"),
                ("x265-params", "log-level=error"),
            ],
        };
        let codec =
            CodecContext::video_encoder(name, &view, first.frame(), timebase, frame_rate, options)?;
        Ok(VideoEncoder { codec })
    }

    /// Feeds a frame stamped `pts` in the encoder's timebase, or `None` to drain. Returns
    /// `false` when packets must be received first.
    pub fn send(&mut self, frame: Option<(&mut VideoFrame, Ticks)>) -> Result<bool, MediaError> {
        match frame {
            Some((frame, pts)) => {
                let raw = frame.frame_mut();
                raw.set_pts(Some(pts));
                raw.clear_picture_type();
                self.codec.send_frame(Some(raw))
            }
            None => self.codec.send_frame(None),
        }
    }

    /// The next packet (Annex B, headers in-band), or `false` when none is ready.
    pub fn receive(&mut self, packet: &mut Packet) -> Result<bool, MediaError> {
        Ok(self.codec.receive_packet(packet)? == Received::Output)
    }
}
