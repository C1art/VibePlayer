#[cfg(windows)]
mod shared;

use std::mem;

use cut::{Ticks, Timebase};

use crate::ffi::{
    CodecContext, FilterGraph, FilterMedia, Frame, HwDevice, Packet, Received, StreamParams,
    pixel_format_name,
};
use crate::gpu::GpuPicture;
use crate::replay::ReplayBuffer;
use crate::video_frame::{HardwareDecoding, PlaneLayout, VideoFrame, VideoOptions};
use crate::{ColorInfo, ColorRange, Matrix, MediaError, Primaries, Rotation};

// NV12 and P010 are what hardware decoders download as; listing them avoids a CPU conversion.
const EIGHT_BIT_FORMATS: &str = "nv12|yuv420p|yuv422p|yuv444p";
const HIGH_BIT_FORMATS: &str = "nv12|p010le|yuv420p|yuv422p|yuv444p|yuv420p10le|yuv422p10le|\
yuv444p10le|yuv420p12le|yuv422p12le|yuv444p12le";

struct ActiveFilter {
    graph: FilterGraph,
    key: FilterKey,
    output_timebase: Timebase,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FilterKey {
    width: u32,
    height: u32,
    format: i32,
    deinterlace: bool,
}

/// Decodes one video stream into [`VideoFrame`]s, with hardware decoding that silently falls
/// back to software, deinterlacing, and conversion to a planar format the renderer understands.
pub struct VideoDecoder {
    params: StreamParams,
    codec: CodecContext,
    options: VideoOptions,
    stream_color: ColorInfo,
    stream_interlaced: bool,
    rotation: Rotation,
    filter: Option<ActiveFilter>,
    /// A deinterlacing graph replaced mid-stream, still giving out the frame it held back.
    retiring: Option<ActiveFilter>,
    filter_drained: bool,
    decoded: Frame,
    downloaded: Frame,
    filtered: Frame,
    skipped: u64,
    replay: ReplayBuffer,
    /// The last frame given out, in stream ticks.
    last_output: Option<Ticks>,
    /// After a fallback replays the GOP: frames up to here were given out already.
    skip_through: Option<Ticks>,
    /// The end of the stream was sent, so a decoder taking over must be drained too.
    end_sent: bool,
    #[cfg(windows)]
    sharing: shared::Sharing,
}

impl VideoDecoder {
    pub fn new(params: StreamParams, options: VideoOptions) -> Result<VideoDecoder, MediaError> {
        let codec = open_codec(&params, options)?;
        let view = params.view();
        Ok(VideoDecoder {
            stream_color: view.color(),
            stream_interlaced: view.is_interlaced(),
            rotation: view.rotation(),
            params,
            codec,
            options,
            filter: None,
            retiring: None,
            filter_drained: false,
            decoded: Frame::new()?,
            downloaded: Frame::new()?,
            filtered: Frame::new()?,
            skipped: 0,
            replay: ReplayBuffer::default(),
            #[cfg(windows)]
            sharing: shared::Sharing::default(),
            last_output: None,
            end_sent: false,
            skip_through: None,
        })
    }

    pub fn is_hardware(&self) -> bool {
        self.codec.is_hardware()
    }

    /// Packets or frames dropped as undecodable so far.
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Feeds a packet, or `None` at the end of the stream. Returns `false` when frames must be
    /// received first; send the same packet again afterwards. Bad packets are skipped.
    pub fn send(&mut self, packet: Option<&Packet>) -> Result<bool, MediaError> {
        let hardware = self.codec.is_hardware();
        match self.codec.send_packet(packet) {
            Ok(accepted) => {
                if accepted && packet.is_none() {
                    self.end_sent = true;
                }
                if accepted
                    && hardware
                    && let Some(packet) = packet
                {
                    self.replay.record(packet)?;
                }
                Ok(accepted)
            }
            Err(_) if hardware => {
                if let Some(packet) = packet {
                    self.replay.record(packet)?;
                }
                self.fall_back_to_software(packet.is_none())?;
                Ok(true)
            }
            Err(_) => {
                self.skipped += 1;
                Ok(true)
            }
        }
    }

    /// The next decoded frame, or `None` when more input is needed or the stream has ended.
    pub fn receive(&mut self) -> Result<Option<VideoFrame>, MediaError> {
        loop {
            if let Some(retiring) = &mut self.retiring {
                if retiring.graph.pull(&mut self.filtered)? == Received::Output {
                    let timebase = retiring.output_timebase;
                    if let Some(frame) = self.finish_frame(timebase)? {
                        return Ok(Some(frame));
                    }
                    continue;
                }
                self.retiring = None;
            }
            if let Some(filter) = &mut self.filter
                && filter.graph.pull(&mut self.filtered)? == Received::Output
            {
                let timebase = filter.output_timebase;
                if let Some(frame) = self.finish_frame(timebase)? {
                    return Ok(Some(frame));
                }
                continue;
            }
            match self.codec.receive_frame(&mut self.decoded) {
                Ok(Received::Output) => {
                    match self.share_decoded() {
                        Ok(Some(Some(frame))) => return Ok(Some(frame)),
                        Ok(Some(None)) => continue,
                        Ok(None) => {}
                        Err(error) => return Err(error),
                    }
                    if self.filter_decoded().is_err() {
                        if self.codec.is_hardware() {
                            self.fall_back_to_software(self.end_sent)?;
                        } else {
                            self.skipped += 1;
                            self.filter = None;
                        }
                    }
                }
                Ok(Received::Again) => return Ok(None),
                Ok(Received::Eof) => {
                    if self.filter_drained {
                        return Ok(None);
                    }
                    self.filter_drained = true;
                    if let Some(filter) = &mut self.filter {
                        filter.graph.push(None)?;
                    }
                }
                Err(_) if self.codec.is_hardware() => self.fall_back_to_software(self.end_sent)?,
                Err(_) => self.skipped += 1,
            }
        }
    }

    /// Forgets buffered frames, e.g. after a seek.
    pub fn flush(&mut self) {
        self.codec.flush();
        self.filter = None;
        self.retiring = None;
        self.filter_drained = false;
        self.replay.clear();
        self.last_output = None;
        self.skip_through = None;
        self.end_sent = false;
    }

    /// Switches to software decoding and feeds it the current GOP again from its keyframe, so
    /// the picture carries on instead of breaking up until the next keyframe.
    fn fall_back_to_software(&mut self, draining: bool) -> Result<(), MediaError> {
        let software = VideoOptions {
            hardware: HardwareDecoding::Off,
            ..self.options
        };
        self.codec = open_codec(&self.params, software)?;
        #[cfg(windows)]
        {
            // Software pictures never use the shared textures; let them go.
            self.sharing = shared::Sharing::default();
        }
        self.options.hardware = HardwareDecoding::Off;
        self.filter = None;
        self.retiring = None;
        self.filter_drained = false;
        self.skip_through = self.last_output;
        for packet in self.replay.take() {
            self.feed_software(Some(&packet));
        }
        if draining {
            self.feed_software(None);
        }
        Ok(())
    }

    /// Sends a packet, taking frames out into the filter whenever the decoder is full.
    fn feed_software(&mut self, packet: Option<&Packet>) {
        loop {
            match self.codec.send_packet(packet) {
                Ok(true) => return,
                Ok(false) => match self.codec.receive_frame(&mut self.decoded) {
                    Ok(Received::Output) => {
                        // Pictures already shown are dropped here rather than piling up in
                        // the filter: a whole GOP of them can be gigabytes at 4K.
                        let shown = self.decoded.filtered_pts().is_some_and(|pts| {
                            self.skip_through.is_some_and(|through| pts <= through)
                        });
                        if !shown && self.filter_decoded().is_err() {
                            self.skipped += 1;
                            self.filter = None;
                        }
                    }
                    Ok(Received::Again | Received::Eof) => return,
                    Err(_) => self.skipped += 1,
                },
                Err(_) => {
                    self.skipped += 1;
                    return;
                }
            }
        }
    }

    fn filter_decoded(&mut self) -> Result<(), MediaError> {
        let source = if self.decoded.is_hardware() {
            self.decoded.download_into(&mut self.downloaded)?;
            &self.downloaded
        } else {
            &self.decoded
        };
        let key = FilterKey {
            width: source.width(),
            height: source.height(),
            format: source.format(),
            deinterlace: self.options.deinterlace
                && (self.stream_interlaced || source.is_interlaced()),
        };
        if self.filter.as_ref().is_none_or(|f| f.key != key) {
            // The deinterlacer holds a frame back; drain it rather than drop it.
            if let Some(mut old) = self.filter.take()
                && old.key.deinterlace
                && old.graph.push(None).is_ok()
            {
                self.retiring = Some(old);
            }
            self.filter = Some(build_filter(
                key,
                source,
                self.params.timebase(),
                self.options.high_bit_depth,
            )?);
        }
        let Some(filter) = &mut self.filter else {
            return Ok(());
        };
        filter.graph.push(Some(source))
    }

    /// The filtered frame, or `None` if a fallback's replay already gave it out.
    fn finish_frame(&mut self, timebase: Timebase) -> Result<Option<VideoFrame>, MediaError> {
        let frame = mem::replace(&mut self.filtered, Frame::new()?);
        let format = pixel_format_name(frame.format()).unwrap_or_default();
        let layout = PlaneLayout::from_format_name(&format)
            .ok_or_else(|| MediaError::Unsupported(format!("filtered pixel format {format}")))?;
        Ok(self.complete(frame, timebase, layout, None))
    }

    /// Stamps a finished frame, or `None` if a fallback's replay already gave it out.
    fn complete(
        &mut self,
        frame: Frame,
        timebase: Timebase,
        layout: PlaneLayout,
        gpu: Option<GpuPicture>,
    ) -> Option<VideoFrame> {
        let pts = frame.filtered_pts().unwrap_or(Ticks::ZERO);
        let stream_pts = pts.rescale(timebase, self.params.timebase());
        if self.skip_through.is_some_and(|shown| stream_pts <= shown) {
            return None;
        }
        self.skip_through = None;
        self.last_output = Some(stream_pts);
        let color = merge_color(frame.color(), self.stream_color).resolved(frame.height());
        Some(VideoFrame {
            pts: pts.rescale(timebase, Timebase::MICROS),
            stream_pts,
            duration: frame.duration().rescale(timebase, Timebase::MICROS),
            layout,
            color,
            luminance: frame.mastering_luminance(),
            rotation: self.rotation,
            frame,
            gpu,
        })
    }
}

/// Pictures stay on the GPU only on Windows.
#[cfg(not(windows))]
impl VideoDecoder {
    fn share_decoded(&mut self) -> Result<Option<Option<VideoFrame>>, MediaError> {
        Ok(None)
    }
}

fn open_codec(params: &StreamParams, options: VideoOptions) -> Result<CodecContext, MediaError> {
    if options.hardware == HardwareDecoding::Auto {
        // Pictures kept on the GPU must be decoded on the renderer's adapter.
        #[cfg(windows)]
        let adapter = options
            .gpu
            .and_then(|gpu| crate::ffi::adapter_index(gpu.adapter_luid));
        #[cfg(not(windows))]
        let adapter = None;
        for &device in HwDevice::preferred() {
            // The index is DXGI's; DXVA2 numbers its adapters differently.
            let adapter = adapter.filter(|_| device == HwDevice::D3d11va);
            if let Ok(codec) = CodecContext::decoder_on(params, Some(device), adapter)
                && codec.is_hardware()
            {
                return Ok(codec);
            }
        }
    }
    CodecContext::decoder(params, None)
}

fn build_filter(
    key: FilterKey,
    source: &Frame,
    timebase: Timebase,
    high_bit_depth: bool,
) -> Result<ActiveFilter, MediaError> {
    let format = pixel_format_name(key.format)
        .ok_or_else(|| MediaError::Unsupported(format!("pixel format #{}", key.format)))?;
    let (aspect_num, aspect_den) = match source.sample_aspect_ratio() {
        (num, den) if num > 0 && den > 0 => (num, den),
        _ => (1, 1),
    };
    let source_args = format!(
        "video_size={}x{}:pix_fmt={format}:time_base={}/{}:pixel_aspect={aspect_num}/{aspect_den}",
        key.width,
        key.height,
        timebase.num(),
        timebase.den()
    );
    let formats = if high_bit_depth {
        HIGH_BIT_FORMATS
    } else {
        EIGHT_BIT_FORMATS
    };
    let deinterlace = if key.deinterlace {
        "bwdif=mode=send_frame:parity=auto:deint=interlaced,"
    } else {
        ""
    };
    let graph = FilterGraph::new(
        FilterMedia::Video,
        &source_args,
        &format!("{deinterlace}format=pix_fmts={formats}"),
    )?;
    Ok(ActiveFilter {
        output_timebase: graph.sink_timebase().unwrap_or(timebase),
        graph,
        key,
    })
}

fn merge_color(frame: ColorInfo, stream: ColorInfo) -> ColorInfo {
    ColorInfo {
        matrix: if frame.matrix == Matrix::Unspecified {
            stream.matrix
        } else {
            frame.matrix
        },
        primaries: if frame.primaries == Primaries::Unspecified {
            stream.primaries
        } else {
            frame.primaries
        },
        transfer: if frame.transfer == crate::Transfer::Sdr {
            stream.transfer
        } else {
            frame.transfer
        },
        range: if frame.range == ColorRange::Unspecified {
            stream.range
        } else {
            frame.range
        },
    }
}
