use std::fmt::Write;
use std::mem;

use cut::{Ticks, Timebase};

use crate::MediaError;
use crate::ffi::{CodecContext, FilterGraph, FilterMedia, Frame, Packet, Received, StreamParams};

/// Playback speed as a percentage; 100 is normal speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Speed(u16);

impl Speed {
    pub const NORMAL: Speed = Speed(100);
    pub const MIN: Speed = Speed(25);
    pub const MAX: Speed = Speed(400);

    pub fn from_percent(percent: u16) -> Speed {
        Speed(percent.clamp(Self::MIN.0, Self::MAX.0))
    }

    pub fn percent(self) -> u16 {
        self.0
    }

    /// Scales a media duration to the wall-clock time it takes at this speed.
    pub fn media_to_wall(self, media: Ticks) -> Ticks {
        Ticks(media.0.saturating_mul(100) / i64::from(self.0))
    }

    /// Scales elapsed wall-clock time to how far media advances at this speed.
    pub fn wall_to_media(self, wall: Ticks) -> Ticks {
        Ticks(wall.0.saturating_mul(i64::from(self.0)) / 100)
    }

    /// `atempo` stages, each kept within the filter's 0.5–2.0 sweet spot.
    fn atempo_chain(self) -> String {
        let mut remaining = f64::from(self.0) / 100.0;
        let mut chain = String::new();
        while remaining < 0.5 {
            chain.push_str("atempo=0.5,");
            remaining /= 0.5;
        }
        while remaining > 2.0 {
            chain.push_str("atempo=2.0,");
            remaining /= 2.0;
        }
        if (remaining - 1.0).abs() > f64::EPSILON {
            let _ = write!(chain, "atempo={remaining:.4},");
        }
        chain
    }
}

/// Gain in tenths of a decibel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Decibels(pub i16);

/// A three-band equalizer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Equalizer {
    pub bass: Decibels,
    pub mid: Decibels,
    pub treble: Decibels,
}

impl Equalizer {
    pub fn is_flat(&self) -> bool {
        *self == Equalizer::default()
    }

    fn filters(&self) -> String {
        let mut chain = String::new();
        let db = |gain: Decibels| f32::from(gain.0) / 10.0;
        if self.bass.0 != 0 {
            let _ = write!(chain, "bass=g={:.1}:f=100,", db(self.bass));
        }
        if self.mid.0 != 0 {
            let _ = write!(chain, "equalizer=f=1000:t=o:w=1.5:g={:.1},", db(self.mid));
        }
        if self.treble.0 != 0 {
            let _ = write!(chain, "treble=g={:.1}:f=6000,", db(self.treble));
        }
        chain
    }
}

/// Changes applied to decoded audio before output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AudioEffects {
    pub speed: Speed,
    /// Fixed gain for export; playback volume is applied live by the output instead.
    pub gain: Decibels,
    pub equalizer: Equalizer,
}

impl Default for AudioEffects {
    fn default() -> AudioEffects {
        AudioEffects {
            speed: Speed::NORMAL,
            gain: Decibels(0),
            equalizer: Equalizer::default(),
        }
    }
}

/// The sample format, rate and channel layout audio must come out in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AudioTarget {
    /// FFmpeg sample format name, e.g. `flt` for interleaved `f32`.
    pub sample_format: String,
    pub sample_rate: u32,
    /// FFmpeg channel layout description, e.g. `stereo` or `5.1`.
    pub layout: String,
    /// Exact samples per frame, for encoders that require it.
    pub frame_size: Option<u32>,
}

impl AudioTarget {
    /// Interleaved `f32` for an output device with `channels` channels.
    pub fn playback(sample_rate: u32, channels: u16) -> AudioTarget {
        let layout = match channels {
            1 => "mono",
            6 => "5.1",
            8 => "7.1",
            _ => "stereo",
        };
        AudioTarget {
            sample_format: "flt".into(),
            sample_rate,
            layout: layout.into(),
            frame_size: None,
        }
    }

    pub fn channel_count(&self) -> u16 {
        match self.layout.as_str() {
            "mono" => 1,
            "5.1" => 6,
            "7.1" => 8,
            _ => 2,
        }
    }
}

/// A block of filtered audio.
pub struct AudioFrame {
    frame: Frame,
    /// Presentation time of the first sample in microseconds, on the media timeline.
    pub pts: Option<Ticks>,
}

impl AudioFrame {
    /// Interleaved samples; empty unless the target format is `flt`.
    pub fn samples(&self) -> &[f32] {
        self.frame.interleaved_f32()
    }

    pub fn sample_count(&self) -> usize {
        self.frame.sample_count()
    }

    /// Restamps the frame, e.g. into an encoder's timebase.
    pub fn set_pts(&mut self, pts: Ticks) {
        self.frame.set_pts(Some(pts));
    }

    pub(crate) fn frame(&self) -> &Frame {
        &self.frame
    }
}

struct ActiveFilter {
    graph: FilterGraph,
    key: (u32, i32, String),
    output_timebase: Timebase,
    /// Media time of the first frame pushed in, in microseconds. `atempo` stamps its output
    /// from here at wall-clock pace, so this is needed to get back to media time.
    origin: Option<Ticks>,
    speed: Speed,
}

impl ActiveFilter {
    /// Converts an output timestamp back to media time, undoing `atempo`'s stretch.
    fn media_time(&self, output: Ticks) -> Ticks {
        match self.origin {
            Some(origin) if self.speed != Speed::NORMAL => {
                origin + self.speed.wall_to_media(output - origin)
            }
            _ => output,
        }
    }
}

/// Decodes one audio stream and converts it to an [`AudioTarget`] with [`AudioEffects`].
pub struct AudioDecoder {
    params: StreamParams,
    codec: CodecContext,
    target: AudioTarget,
    effects: AudioEffects,
    filter: Option<ActiveFilter>,
    filter_drained: bool,
    decoded: Frame,
    filtered: Frame,
    skipped: u64,
}

impl AudioDecoder {
    pub fn new(
        params: StreamParams,
        target: AudioTarget,
        effects: AudioEffects,
    ) -> Result<AudioDecoder, MediaError> {
        Ok(AudioDecoder {
            codec: CodecContext::decoder(&params, None)?,
            params,
            target,
            effects,
            filter: None,
            filter_drained: false,
            decoded: Frame::new()?,
            filtered: Frame::new()?,
            skipped: 0,
        })
    }

    pub fn target(&self) -> &AudioTarget {
        &self.target
    }

    pub fn effects(&self) -> AudioEffects {
        self.effects
    }

    /// Takes effect from the next decoded frame; buffered samples keep the old effects.
    pub fn set_effects(&mut self, effects: AudioEffects) {
        if effects != self.effects {
            self.effects = effects;
            self.filter = None;
        }
    }

    /// Feeds a packet, or `None` at the end of the stream. Returns `false` when frames must be
    /// received first. Bad packets are skipped.
    pub fn send(&mut self, packet: Option<&Packet>) -> Result<bool, MediaError> {
        match self.codec.send_packet(packet) {
            Ok(accepted) => Ok(accepted),
            Err(_) => {
                self.skipped += 1;
                Ok(true)
            }
        }
    }

    pub fn receive(&mut self) -> Result<Option<AudioFrame>, MediaError> {
        loop {
            if let Some(filter) = &mut self.filter
                && filter.graph.pull(&mut self.filtered)? == Received::Output
            {
                let frame = mem::replace(&mut self.filtered, Frame::new()?);
                let pts = frame.filtered_pts().map(|pts| {
                    filter.media_time(pts.rescale(filter.output_timebase, Timebase::MICROS))
                });
                return Ok(Some(AudioFrame { frame, pts }));
            }
            match self.codec.receive_frame(&mut self.decoded) {
                Ok(Received::Output) => {
                    if self.filter_decoded().is_err() {
                        self.skipped += 1;
                        self.filter = None;
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
                Err(_) => self.skipped += 1,
            }
        }
    }

    pub fn flush(&mut self) {
        self.codec.flush();
        self.filter = None;
        self.filter_drained = false;
    }

    fn filter_decoded(&mut self) -> Result<(), MediaError> {
        let key = (
            self.decoded.sample_rate(),
            self.decoded.format(),
            self.decoded.channel_layout_description(),
        );
        if self.filter.as_ref().is_none_or(|f| f.key != key) {
            self.filter = Some(self.build_filter(key)?);
        }
        let timebase = self.params.timebase();
        match &mut self.filter {
            Some(filter) => {
                if filter.origin.is_none() {
                    filter.origin = self
                        .decoded
                        .pts()
                        .map(|pts| pts.rescale(timebase, Timebase::MICROS));
                }
                filter.graph.push(Some(&self.decoded))
            }
            None => Ok(()),
        }
    }

    fn build_filter(&self, key: (u32, i32, String)) -> Result<ActiveFilter, MediaError> {
        let (rate, format, layout) = &key;
        let format_name = crate::ffi::sample_format_name(*format)
            .ok_or_else(|| MediaError::Unsupported(format!("sample format #{format}")))?;
        let layout = if layout.is_empty() { "stereo" } else { layout };
        let timebase = self.params.timebase();
        let source_args = format!(
            "time_base={}/{}:sample_rate={rate}:sample_fmt={format_name}:channel_layout={layout}",
            timebase.num(),
            timebase.den()
        );
        let mut description = self.effects.speed.atempo_chain();
        description.push_str(&self.effects.equalizer.filters());
        if self.effects.gain.0 != 0 {
            let _ = write!(
                description,
                "volume={:.1}dB,",
                f32::from(self.effects.gain.0) / 10.0
            );
        }
        let _ = write!(
            description,
            "aresample={},aformat=sample_fmts={}:sample_rates={}:channel_layouts={}",
            self.target.sample_rate,
            self.target.sample_format,
            self.target.sample_rate,
            self.target.layout
        );
        let mut graph = FilterGraph::new(FilterMedia::Audio, &source_args, &description)?;
        if let Some(size) = self.target.frame_size {
            graph.set_sink_frame_size(size);
        }
        Ok(ActiveFilter {
            output_timebase: graph.sink_timebase().unwrap_or(timebase),
            graph,
            key,
            origin: None,
            speed: self.effects.speed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_conversions_are_inverse_at_any_speed() {
        let fast = Speed::from_percent(150);
        assert_eq!(fast.media_to_wall(Ticks(3_000_000)), Ticks(2_000_000));
        assert_eq!(fast.wall_to_media(Ticks(2_000_000)), Ticks(3_000_000));
    }

    #[test]
    fn speed_is_clamped_to_supported_range() {
        assert_eq!(Speed::from_percent(10), Speed::MIN);
        assert_eq!(Speed::from_percent(1000), Speed::MAX);
    }

    #[test]
    fn atempo_chain_splits_extreme_speeds_into_stages() {
        assert_eq!(Speed::NORMAL.atempo_chain(), "");
        assert_eq!(
            Speed::from_percent(25).atempo_chain(),
            "atempo=0.5,atempo=0.5000,"
        );
        assert_eq!(
            Speed::from_percent(400).atempo_chain(),
            "atempo=2.0,atempo=2.0000,"
        );
        assert_eq!(Speed::from_percent(150).atempo_chain(), "atempo=1.5000,");
    }
}
