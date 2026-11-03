//! The re-encoded head of a smart cut: frames from the cut's start up to the next keyframe are
//! decoded and encoded again, then the rest of the stream is copied as usual.

use cut::{CutPlan, StreamIndex, Ticks, Timebase};
use media::{
    Demuxer, HardwareDecoding, Muxer, Packet, StreamParams, VideoDecoder, VideoEncoder, VideoFrame,
    VideoOptions,
};

use crate::ExportError;
use crate::nal::{self, NalFormat};

/// Why a stream can't be smart-cut, for the editor to show.
pub fn smart_cut_support(info: &media::MediaInfo, stream: StreamIndex) -> Result<(), String> {
    let stream = info.stream(stream).ok_or("no video stream")?;
    let media::StreamDetails::Video(video) = &stream.details else {
        return Err("only video can be smart-cut".into());
    };
    if video.interlaced {
        return Err("interlaced video can't be smart-cut".into());
    }
    if media::smart_cut_encoder(&stream.codec).is_none() {
        return Err(match stream.codec.as_str() {
            "h264" | "hevc" => format!(
                "this FFmpeg build has no {} encoder",
                stream.codec.to_uppercase()
            ),
            other => format!(
                "smart cut supports H.264 and HEVC, not {}",
                other.to_uppercase()
            ),
        });
    }
    Ok(())
}

pub(crate) struct SmartHead {
    params: StreamParams,
    decoder: VideoDecoder,
    encoder: Option<VideoEncoder>,
    frame_rate: Option<(i32, i32)>,
    timebase: Timebase,
    start: Ticks,
    until: Ticks,
    format: NalFormat,
    parameter_sets: Vec<Vec<u8>>,
    encoded: Vec<Packet>,
    /// Copied packets read while the head was still being encoded.
    held: Vec<Packet>,
    finished: bool,
}

impl SmartHead {
    pub fn new(demuxer: &Demuxer, plan: &CutPlan, until: Ticks) -> Result<SmartHead, ExportError> {
        smart_cut_support(&demuxer.info(), plan.stream).map_err(ExportError::SmartCut)?;
        let params = demuxer.stream_params(plan.stream)?;
        let decoder = VideoDecoder::new(
            demuxer.stream_params(plan.stream)?,
            VideoOptions {
                hardware: HardwareDecoding::Off,
                high_bit_depth: true,
                deinterlace: false,
            },
        )?;
        let view = params.view();
        let (format, parameter_sets) = nal::stream_layout(&view.codec_name(), view.extradata());
        let frame_rate = demuxer
            .info()
            .video()
            .and_then(|(_, video)| video.frame_rate);
        Ok(SmartHead {
            timebase: params.timebase(),
            params,
            decoder,
            encoder: None,
            frame_rate,
            start: plan.start,
            until,
            format,
            parameter_sets,
            encoded: Vec::new(),
            held: Vec::new(),
            finished: false,
        })
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Takes a packet of the primary stream. Packets the copy keeps are held until the head
    /// is written; every packet is decoded until a frame at or past `until` appears.
    pub fn feed(&mut self, packet: &Packet, plan: &CutPlan) -> Result<(), ExportError> {
        if let Some(pts) = packet.pts()
            && plan.keeps_primary(pts, packet.dts())
        {
            self.held.push(packet.try_clone()?);
        }
        if self.finished {
            return Ok(());
        }
        loop {
            let accepted = self.decoder.send(Some(packet))?;
            self.drain_decoder()?;
            if accepted || self.finished {
                return Ok(());
            }
        }
    }

    /// Ends the head at the end of the file.
    pub fn finish(&mut self) -> Result<(), ExportError> {
        if !self.finished {
            self.decoder.send(None)?;
            self.drain_decoder()?;
            self.finished = true;
        }
        Ok(())
    }

    fn drain_decoder(&mut self) -> Result<(), ExportError> {
        while let Some(mut frame) = self.decoder.receive()? {
            if self.finished {
                continue;
            }
            let pts = frame.stream_pts;
            if pts >= self.until {
                self.finished = true;
            } else if pts >= self.start {
                self.encode(&mut frame, pts)?;
            }
        }
        Ok(())
    }

    fn encode(&mut self, frame: &mut VideoFrame, pts: Ticks) -> Result<(), ExportError> {
        if self.encoder.is_none() {
            self.encoder = Some(VideoEncoder::for_smart_cut(
                &self.params,
                frame,
                self.timebase,
                self.frame_rate,
            )?);
        }
        let Some(encoder) = &mut self.encoder else {
            return Ok(());
        };
        while !encoder.send(Some((frame, pts)))? {
            collect(encoder, &mut self.encoded)?;
        }
        collect(encoder, &mut self.encoded)
    }

    /// Writes the re-encoded head, then the held copied packets, all restamped by `restamp`.
    /// Returns how many frames were re-encoded.
    pub fn write(
        mut self,
        muxer: &mut Muxer,
        mut restamp: impl FnMut(&mut Packet),
    ) -> Result<usize, ExportError> {
        if let Some(encoder) = &mut self.encoder {
            encoder.send(None)?;
            collect(encoder, &mut self.encoded)?;
        }
        // Copied packets keep their own decode delay; giving the head the same delay keeps
        // decode timestamps rising across the join, since the head has no B-frames.
        let delay = self
            .held
            .first()
            .and_then(|first| Some(first.pts()? - first.dts()?))
            .unwrap_or(Ticks::ZERO)
            .max(Ticks::ZERO);
        let frames = self.encoded.len();
        for mut packet in std::mem::take(&mut self.encoded) {
            let pts = packet.pts();
            packet.set_timestamps(pts, pts.map(|pts| pts - delay));
            if self.format != NalFormat::AnnexB {
                let units = nal::split_annex_b(packet.data());
                let framed = nal::frame(units, self.format).ok_or_else(too_long)?;
                packet.replace_data(&framed)?;
            }
            restamp(&mut packet);
            muxer.write(&mut packet)?;
        }
        for (index, mut packet) in std::mem::take(&mut self.held).into_iter().enumerate() {
            if index == 0 && !self.parameter_sets.is_empty() {
                // The head switched the decoder to the encoder's parameter sets; switch back.
                let mut data =
                    nal::frame(self.parameter_sets.iter().map(Vec::as_slice), self.format)
                        .ok_or_else(too_long)?;
                data.extend_from_slice(packet.data());
                packet.replace_data(&data)?;
            }
            restamp(&mut packet);
            muxer.write(&mut packet)?;
        }
        Ok(frames)
    }
}

fn too_long() -> ExportError {
    ExportError::SmartCut("a re-encoded frame is too large for this file's NAL length size".into())
}

fn collect(encoder: &mut VideoEncoder, into: &mut Vec<Packet>) -> Result<(), ExportError> {
    loop {
        let mut packet = Packet::new()?;
        if !encoder.receive(&mut packet)? {
            return Ok(());
        }
        into.push(packet);
    }
}
