use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use cut::{CutPlan, StreamIndex, Ticks, Timebase};
use media::{
    AudioDecoder, AudioEffects, AudioEncoder, Chapter, Decibels, Demuxer, Equalizer, Muxer, Packet,
    StreamKind,
};

use crate::ExportError;

/// Audio edits that force audio, and only audio, to be re-encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioChanges {
    pub gain: Decibels,
    pub equalizer: Equalizer,
}

impl AudioChanges {
    pub fn is_none(&self) -> bool {
        self.gain.0 == 0 && self.equalizer.is_flat()
    }
}

const DEFAULT_AUDIO_BIT_RATE: u64 = 192_000;
/// Audio packets this close outside the range are still decoded, then trimmed by sample time.
const AUDIO_DECODE_SLACK: Ticks = Ticks(1_000_000);

enum Output {
    Copy {
        index: StreamIndex,
        input_timebase: Timebase,
        output_timebase: Timebase,
    },
    Encode(Box<Reencode>),
}

struct Reencode {
    index: StreamIndex,
    input_timebase: Timebase,
    output_timebase: Timebase,
    decoder: AudioDecoder,
    encoder: AudioEncoder,
    sample_rate: i64,
    next_sample: Option<i64>,
}

pub(crate) struct Remuxed {
    pub streams: usize,
    pub reencoded_audio: bool,
}

pub(crate) fn remux(
    source: &Path,
    temp: &Path,
    plan: &CutPlan,
    audio: AudioChanges,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32),
) -> Result<Remuxed, ExportError> {
    let mut demuxer = Demuxer::open(source)?;
    let info = demuxer.info();
    let mut muxer = Muxer::create(temp)?;
    let mut outputs: Vec<Option<Output>> = Vec::new();
    let mut reencoded_audio = false;
    for stream in &info.streams {
        let Some(input_timebase) = stream.timebase else {
            outputs.push(None);
            continue;
        };
        let output = if stream.kind == StreamKind::Audio && !audio.is_none() {
            reencoded_audio = true;
            let params = demuxer.stream_params(stream.index)?;
            let bit_rate = stream.bit_rate.unwrap_or(DEFAULT_AUDIO_BIT_RATE);
            let global_header = muxer.needs_global_header();
            let mut encoder = AudioEncoder::open(&params, None, bit_rate, global_header)
                .or_else(|_| AudioEncoder::open(&params, Some("aac"), bit_rate, global_header))?;
            let effects = AudioEffects {
                gain: audio.gain,
                equalizer: audio.equalizer,
                ..AudioEffects::default()
            };
            let decoder = AudioDecoder::new(params, encoder.target(), effects)?;
            let index = muxer.add_encoded_stream(&mut encoder, &demuxer, stream.index)?;
            Some(Output::Encode(Box::new(Reencode {
                index,
                input_timebase,
                output_timebase: input_timebase,
                sample_rate: i64::from(encoder.target().sample_rate.max(1)),
                decoder,
                encoder,
                next_sample: None,
            })))
        } else {
            match muxer.add_copied_stream(&demuxer, stream.index) {
                Ok(index) => Some(Output::Copy {
                    index,
                    input_timebase,
                    output_timebase: input_timebase,
                }),
                // Data streams some containers can't carry aren't worth failing the export.
                Err(_) if stream.kind == StreamKind::Other => None,
                Err(error) => return Err(error.into()),
            }
        };
        outputs.push(output);
    }
    muxer.copy_metadata(&demuxer);
    muxer.set_chapters(&trimmed_chapters(&info.chapters, plan))?;
    muxer.write_header()?;
    for output in outputs.iter_mut().flatten() {
        match output {
            Output::Copy {
                index,
                output_timebase,
                ..
            } => *output_timebase = muxer.timebase(*index).unwrap_or(*output_timebase),
            Output::Encode(reencode) => {
                reencode.output_timebase = muxer
                    .timebase(reencode.index)
                    .unwrap_or(reencode.output_timebase);
            }
        }
    }
    let mapped: Vec<StreamIndex> = outputs
        .iter()
        .enumerate()
        .filter(|(_, output)| output.is_some())
        .map(|(index, _)| StreamIndex(index))
        .collect();
    let streams = mapped.len();
    demuxer.select_streams(&mapped);
    demuxer.seek_stream(plan.stream, plan.start)?;

    let mut packet = Packet::new()?;
    let mut finished = vec![false; outputs.len()];
    let needs_finish: Vec<bool> = info
        .streams
        .iter()
        .map(|s| matches!(s.kind, StreamKind::Video | StreamKind::Audio))
        .collect();
    while demuxer.read(&mut packet)? {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        let input_index = packet.stream_index();
        let Some(Some(output)) = outputs.get_mut(input_index.0) else {
            continue;
        };
        match output {
            Output::Copy {
                index,
                input_timebase,
                output_timebase,
            } => {
                let (timebase, out_index, out_timebase) =
                    (*input_timebase, *index, *output_timebase);
                let Some(pts) = packet.pts() else {
                    continue;
                };
                let dts = packet.dts();
                if dts.unwrap_or(pts) >= Ticks::ZERO
                    && plan.is_past_end(dts.unwrap_or(pts), timebase)
                {
                    finished[input_index.0] = true;
                }
                let keep = if input_index == plan.stream {
                    if let Some(fraction) = fraction_done(plan, pts) {
                        progress(fraction);
                    }
                    plan.keeps_primary(pts, dts)
                } else {
                    plan.keeps_other(pts, timebase)
                };
                if keep {
                    restamp(&mut packet, plan, timebase, out_timebase, out_index);
                    muxer.write(&mut packet)?;
                }
            }
            Output::Encode(reencode) => {
                let timebase = reencode.input_timebase;
                let Some(pts) = packet.pts() else {
                    continue;
                };
                if plan.is_past_end(pts, timebase) {
                    finished[input_index.0] = true;
                }
                let (start, end) = plan.range_in(timebase);
                let slack = AUDIO_DECODE_SLACK.rescale(Timebase::MICROS, timebase);
                if pts >= start - slack && pts < end + slack {
                    reencode.feed(Some(&packet), plan, &mut muxer)?;
                }
            }
        }
        let all_done = finished
            .iter()
            .zip(&needs_finish)
            .zip(&outputs)
            .all(|((done, needed), output)| *done || !needed || output.is_none());
        if all_done {
            break;
        }
    }
    for output in outputs.iter_mut().flatten() {
        if let Output::Encode(reencode) = output {
            reencode.feed(None, plan, &mut muxer)?;
            reencode.drain_encoder(&mut muxer, true)?;
        }
    }
    muxer.finish()?;
    progress(1.0);
    Ok(Remuxed {
        streams,
        reencoded_audio,
    })
}

fn restamp(
    packet: &mut Packet,
    plan: &CutPlan,
    input: Timebase,
    output: Timebase,
    index: StreamIndex,
) {
    let shift = |ticks: Ticks| plan.rebase(ticks, input).rescale(input, output);
    let pts = packet.pts().map(shift);
    let dts = packet.dts().map(shift);
    packet.set_timestamps(pts, dts);
    let duration = packet.duration().rescale(input, output);
    packet.set_duration(duration);
    packet.set_stream_index(index);
    packet.clear_position();
}

fn fraction_done(plan: &CutPlan, pts: Ticks) -> Option<f32> {
    let total = plan.duration().0;
    (total > 0).then(|| ((pts - plan.start).0 as f32 / total as f32).clamp(0.0, 1.0))
}

impl Reencode {
    /// Decodes a packet (or drains with `None`), trims to the cut by sample time, and encodes.
    fn feed(
        &mut self,
        packet: Option<&Packet>,
        plan: &CutPlan,
        muxer: &mut Muxer,
    ) -> Result<(), ExportError> {
        let (start, end) = plan.range_in(Timebase::MICROS);
        let rate_timebase = Timebase::new(1, self.sample_rate).unwrap_or(Timebase::MICROS);
        loop {
            let accepted = self.decoder.send(packet)?;
            while let Some(mut frame) = self.decoder.receive()? {
                let Some(pts) = frame.pts else {
                    continue;
                };
                let samples = frame.sample_count() as i64;
                let frame_end = pts + Ticks(samples).rescale(rate_timebase, Timebase::MICROS);
                if frame_end <= start || pts >= end {
                    continue;
                }
                let at = *self.next_sample.get_or_insert_with(|| {
                    (pts - start)
                        .max(Ticks::ZERO)
                        .rescale(Timebase::MICROS, rate_timebase)
                        .0
                });
                frame.set_pts(Ticks(at));
                self.next_sample = Some(at + samples);
                while !self.encoder.send(Some(&frame))? {
                    self.drain_encoder(muxer, false)?;
                }
                self.drain_encoder(muxer, false)?;
            }
            if accepted {
                return Ok(());
            }
        }
    }

    fn drain_encoder(&mut self, muxer: &mut Muxer, flush: bool) -> Result<(), ExportError> {
        if flush {
            self.encoder.send(None)?;
        }
        let encoder_timebase = self.encoder.timebase().unwrap_or(Timebase::MICROS);
        let mut packet = Packet::new()?;
        while self.encoder.receive(&mut packet)? {
            let shift = |ticks: Ticks| ticks.rescale(encoder_timebase, self.output_timebase);
            let (pts, dts) = (packet.pts().map(shift), packet.dts().map(shift));
            packet.set_timestamps(pts, dts);
            packet.set_duration(
                packet
                    .duration()
                    .rescale(encoder_timebase, self.output_timebase),
            );
            packet.set_stream_index(self.index);
            muxer.write(&mut packet)?;
        }
        Ok(())
    }
}

/// Chapters overlapping the cut, clipped to it and moved to start at zero.
fn trimmed_chapters(chapters: &[Chapter], plan: &CutPlan) -> Vec<Chapter> {
    chapters
        .iter()
        .filter_map(|chapter| {
            let (start, end) = plan.range_in(chapter.timebase);
            if chapter.end <= start || chapter.start >= end {
                return None;
            }
            Some(Chapter {
                timebase: chapter.timebase,
                start: chapter.start.max(start) - start,
                end: chapter.end.min(end) - start,
                metadata: chapter.metadata.clone(),
            })
        })
        .collect()
}
