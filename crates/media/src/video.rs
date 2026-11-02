use std::mem;

use cut::{Ticks, Timebase};

use crate::ffi::{
    CodecContext, FilterGraph, FilterMedia, Frame, HwDevice, MasteringLuminance, Packet, Received,
    StreamParams, pixel_format_name,
};
use crate::{ColorInfo, ColorRange, Matrix, MediaError, Primaries};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareDecoding {
    Auto,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoOptions {
    pub hardware: HardwareDecoding,
    /// Whether frames may come out with more than 8 bits per sample; otherwise they're reduced.
    pub high_bit_depth: bool,
}

/// How the three planes of a [`VideoFrame`] are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlaneLayout {
    /// Significant bits per sample: 8 (one byte) or 10/12/16 (two little-endian bytes).
    pub bits: u8,
    /// Chroma planes are this many powers of two narrower and shorter than luma.
    pub chroma_shift_x: u8,
    pub chroma_shift_y: u8,
    /// U and V share plane 1, interleaved (NV12, P010), as hardware decoders produce them.
    pub interleaved_chroma: bool,
}

impl PlaneLayout {
    pub fn bytes_per_sample(self) -> usize {
        if self.bits > 8 { 2 } else { 1 }
    }

    fn from_format_name(name: &str) -> Option<PlaneLayout> {
        let interleaved = |bits| PlaneLayout {
            bits,
            chroma_shift_x: 1,
            chroma_shift_y: 1,
            interleaved_chroma: true,
        };
        match name {
            "nv12" => return Some(interleaved(8)),
            // P010 keeps its 10 bits at the top of each 16-bit sample, so it reads as 16-bit.
            "p010le" => return Some(interleaved(16)),
            _ => {}
        }
        let (bits, rest) = match name.strip_suffix("le") {
            Some(base) => {
                let digits_at = base.rfind('p')? + 1;
                (base[digits_at..].parse().ok()?, &base[..digits_at])
            }
            None => (8, name),
        };
        let (chroma_shift_x, chroma_shift_y) = match rest {
            "yuv420p" => (1, 1),
            "yuv422p" => (1, 0),
            "yuv444p" => (0, 0),
            _ => return None,
        };
        Some(PlaneLayout {
            bits,
            chroma_shift_x,
            chroma_shift_y,
            interleaved_chroma: false,
        })
    }
}

// NV12 and P010 are what hardware decoders download as; listing them avoids a CPU conversion.
const EIGHT_BIT_FORMATS: &str = "nv12|yuv420p|yuv422p|yuv444p";
const HIGH_BIT_FORMATS: &str = "nv12|p010le|yuv420p|yuv422p|yuv444p|yuv420p10le|yuv422p10le|\
yuv444p10le|yuv420p12le|yuv422p12le|yuv444p12le";

/// A decoded, display-ready frame in planar YUV.
pub struct VideoFrame {
    frame: Frame,
    /// Presentation time in microseconds.
    pub pts: Ticks,
    /// Presentation time in the stream's own timebase.
    pub stream_pts: Ticks,
    /// In microseconds; zero when unknown.
    pub duration: Ticks,
    pub layout: PlaneLayout,
    pub color: ColorInfo,
    pub luminance: MasteringLuminance,
}

impl VideoFrame {
    pub fn width(&self) -> u32 {
        self.frame.width()
    }

    pub fn height(&self) -> u32 {
        self.frame.height()
    }

    /// Plane `index` (0 = Y, 1 = U or interleaved UV, 2 = V) as `(bytes, stride)`.
    pub fn plane(&self, index: usize) -> Option<(&[u8], usize)> {
        if index > 2 {
            return None;
        }
        self.frame.plane(index)
    }

    pub(crate) fn frame(&self) -> &Frame {
        &self.frame
    }

    pub(crate) fn frame_mut(&mut self) -> &mut Frame {
        &mut self.frame
    }

    /// Display width over height, including non-square pixels.
    pub fn display_aspect(&self) -> f32 {
        let (num, den) = self.frame.sample_aspect_ratio();
        let pixel_aspect = if num > 0 && den > 0 {
            num as f32 / den as f32
        } else {
            1.0
        };
        self.width() as f32 * pixel_aspect / self.height().max(1) as f32
    }
}

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
    filter: Option<ActiveFilter>,
    filter_drained: bool,
    decoded: Frame,
    downloaded: Frame,
    filtered: Frame,
    skipped: u64,
}

impl VideoDecoder {
    pub fn new(params: StreamParams, options: VideoOptions) -> Result<VideoDecoder, MediaError> {
        let codec = open_codec(&params, options.hardware)?;
        let view = params.view();
        Ok(VideoDecoder {
            stream_color: view.color(),
            stream_interlaced: view.is_interlaced(),
            params,
            codec,
            options,
            filter: None,
            filter_drained: false,
            decoded: Frame::new()?,
            downloaded: Frame::new()?,
            filtered: Frame::new()?,
            skipped: 0,
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
        match self.codec.send_packet(packet) {
            Ok(accepted) => Ok(accepted),
            Err(_) if self.codec.is_hardware() => {
                self.fall_back_to_software()?;
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
            if let Some(filter) = &mut self.filter
                && filter.graph.pull(&mut self.filtered)? == Received::Output
            {
                let timebase = filter.output_timebase;
                return self.finish_frame(timebase).map(Some);
            }
            match self.codec.receive_frame(&mut self.decoded) {
                Ok(Received::Output) => {
                    if self.filter_decoded().is_err() {
                        if self.codec.is_hardware() {
                            self.fall_back_to_software()?;
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
                Err(_) if self.codec.is_hardware() => self.fall_back_to_software()?,
                Err(_) => self.skipped += 1,
            }
        }
    }

    /// Forgets buffered frames, e.g. after a seek.
    pub fn flush(&mut self) {
        self.codec.flush();
        self.filter = None;
        self.filter_drained = false;
    }

    fn fall_back_to_software(&mut self) -> Result<(), MediaError> {
        self.codec = open_codec(&self.params, HardwareDecoding::Off)?;
        self.options.hardware = HardwareDecoding::Off;
        self.filter = None;
        self.filter_drained = false;
        Ok(())
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
            deinterlace: self.stream_interlaced || source.is_interlaced(),
        };
        if self.filter.as_ref().is_none_or(|f| f.key != key) {
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

    fn finish_frame(&mut self, timebase: Timebase) -> Result<VideoFrame, MediaError> {
        let frame = mem::replace(&mut self.filtered, Frame::new()?);
        let format = pixel_format_name(frame.format()).unwrap_or_default();
        let layout = PlaneLayout::from_format_name(&format)
            .ok_or_else(|| MediaError::Unsupported(format!("filtered pixel format {format}")))?;
        let pts = frame.filtered_pts().unwrap_or(Ticks::ZERO);
        let color = merge_color(frame.color(), self.stream_color).resolved(frame.height());
        Ok(VideoFrame {
            pts: pts.rescale(timebase, Timebase::MICROS),
            stream_pts: pts.rescale(timebase, self.params.timebase()),
            duration: frame.duration().rescale(timebase, Timebase::MICROS),
            layout,
            color,
            luminance: frame.mastering_luminance(),
            frame,
        })
    }
}

fn open_codec(
    params: &StreamParams,
    hardware: HardwareDecoding,
) -> Result<CodecContext, MediaError> {
    if hardware == HardwareDecoding::Auto {
        for &device in HwDevice::preferred() {
            if let Ok(codec) = CodecContext::decoder(params, Some(device))
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

#[cfg(test)]
mod tests {
    use super::PlaneLayout;

    #[test]
    fn plane_layout_reads_bit_depth_and_subsampling_from_format_names() {
        let layout = |name| {
            PlaneLayout::from_format_name(name)
                .map(|l| (l.bits, l.chroma_shift_x, l.chroma_shift_y))
        };
        assert_eq!(layout("yuv420p"), Some((8, 1, 1)));
        assert_eq!(layout("yuv422p10le"), Some((10, 1, 0)));
        assert_eq!(layout("yuv444p12le"), Some((12, 0, 0)));
        assert_eq!(layout("nv12"), Some((8, 1, 1)));
        assert!(
            PlaneLayout::from_format_name("p010le")
                .unwrap()
                .interleaved_chroma
        );
        assert_eq!(layout("rgb24"), None);
    }
}
