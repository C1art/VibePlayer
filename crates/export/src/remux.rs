use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use cut::{CutPlan, StreamIndex, Ticks, Timebase};
use media::{
    AudioDecoder, AudioEffects, AudioEncoder, Chapter, Decibels, Demuxer, Equalizer, Muxer, Packet,
    StreamDetails, StreamInfo, StreamKind,
};

use crate::ExportError;
use crate::lead_in::{self, SubtitleOutput};
use crate::smart::SmartHead;

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
    /// Cover art: one packet with no place on the timeline, kept whatever the cut.
    Picture {
        index: StreamIndex,
        written: bool,
    },
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
    pub reencoded_frames: usize,
    pub streams: usize,
    pub reencoded_audio: bool,
    /// The cut-on stream in the output, when it is video.
    pub video: Option<StreamIndex>,
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
                trim: Some(plan.range_in(Timebase::MICROS)),
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
                Ok(index) if is_attached_picture(stream) => Some(Output::Picture {
                    index,
                    written: false,
                }),
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
            Output::Picture { .. } => {}
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
    let mut head = match plan.reencode {
        Some(reencoded) => Some(SmartHead::new(&demuxer, plan, reencoded.until)?),
        None => None,
    };
    let primary_output = match outputs.get(plan.stream.0) {
        Some(Some(Output::Copy {
            index,
            input_timebase,
            output_timebase,
        })) => Some((*index, *input_timebase, *output_timebase)),
        _ => None,
    };
    let mut reencoded_frames = 0;
    let subtitles: Vec<SubtitleOutput> = info
        .streams
        .iter()
        .filter(|stream| stream.kind == StreamKind::Subtitle)
        .filter_map(|stream| match outputs.get(stream.index.0) {
            Some(Some(Output::Copy {
                index,
                input_timebase,
                output_timebase,
            })) => Some(SubtitleOutput {
                input: stream.index,
                output: *index,
                input_timebase: *input_timebase,
                output_timebase: *output_timebase,
            }),
            _ => None,
        })
        .collect();
    lead_in::write_cues_on_screen(source, plan, &subtitles, &mut muxer)?;
    let decode_from = plan.reencode.map_or(plan.start, |head| head.decode_from);
    demuxer.seek_stream(plan.stream, decode_from)?;
    // The seek can land on an earlier keyframe (formats that seek by decode time); everything
    // of the primary stream before the planned one is left out.
    let mut reached_start = false;
    let is_subtitle: Vec<bool> = info
        .streams
        .iter()
        .map(|stream| stream.kind == StreamKind::Subtitle)
        .collect();

    let mut packet = Packet::new()?;
    let mut finished = vec![false; outputs.len()];
    let needs_finish: Vec<bool> = info
        .streams
        .iter()
        .map(|s| matches!(s.kind, StreamKind::Video | StreamKind::Audio) && !is_attached_picture(s))
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
                if input_index == plan.stream && !reached_start {
                    if !(packet.is_keyframe() && pts >= decode_from) {
                        continue;
                    }
                    reached_start = true;
                }
                let keep = if input_index == plan.stream {
                    if let Some(fraction) = fraction_done(plan, pts) {
                        progress(fraction);
                    }
                    if let Some(active) = &mut head {
                        active.feed(&packet, plan)?;
                        if active.is_finished()
                            && let Some(finished_head) = head.take()
                        {
                            reencoded_frames = finished_head.write(&mut muxer, |packet| {
                                restamp(packet, plan, timebase, out_timebase, out_index);
                            })?;
                        }
                        continue;
                    }
                    plan.keeps_primary(pts, dts)
                } else {
                    plan.keeps_other(pts, timebase)
                };
                if keep {
                    if is_subtitle[input_index.0] {
                        // A cue running past the end stops there, as the clip does.
                        let end = plan.range_in(timebase).1;
                        if packet.duration() > Ticks::ZERO && pts + packet.duration() > end {
                            packet.set_duration(end - pts);
                        }
                    }
                    restamp(&mut packet, plan, timebase, out_timebase, out_index);
                    muxer.write(&mut packet)?;
                }
            }
            Output::Picture { index, written } => {
                if !*written {
                    *written = true;
                    packet.set_timestamps(Some(Ticks::ZERO), Some(Ticks::ZERO));
                    packet.set_duration(Ticks::ZERO);
                    packet.set_stream_index(*index);
                    packet.clear_position();
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
    if let (Some(mut unfinished), Some((index, input, output))) = (head.take(), primary_output) {
        unfinished.finish()?;
        reencoded_frames = unfinished.write(&mut muxer, |packet| {
            restamp(packet, plan, input, output, index);
        })?;
    }
    for output in outputs.iter_mut().flatten() {
        if let Output::Encode(reencode) = output {
            reencode.feed(None, plan, &mut muxer)?;
            reencode.drain_encoder(&mut muxer, true)?;
        }
    }
    // Stopping here keeps a cancel from being outlasted by the trailer and verification.
    if cancel.load(Ordering::Relaxed) {
        return Err(ExportError::Cancelled);
    }
    muxer.finish()?;
    progress(1.0);
    let video = info
        .streams
        .get(plan.stream.0)
        .filter(|stream| stream.kind == StreamKind::Video)
        .and(primary_output.map(|(index, _, _)| index));
    Ok(Remuxed {
        reencoded_frames,
        streams,
        reencoded_audio,
        video,
    })
}

pub(crate) fn restamp(
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

fn is_attached_picture(stream: &StreamInfo) -> bool {
    matches!(&stream.details, StreamDetails::Video(video) if video.attached_picture)
}

fn fraction_done(plan: &CutPlan, pts: Ticks) -> Option<f32> {
    let total = plan.duration().0;
    (total > 0).then(|| ((pts - plan.start).0 as f32 / total as f32).clamp(0.0, 1.0))
}

impl Reencode {
    /// Decodes a packet (or drains with `None`) and encodes what the decoder's trim keeps.
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
                let placed = (pts - start)
                    .max(Ticks::ZERO)
                    .rescale(Timebase::MICROS, rate_timebase)
                    .0;
                // Back to back, unless the source has a real gap here (over 20 ms), which
                // must stay or everything after it would play early.
                let gap = self.sample_rate / 50;
                let at = match self.next_sample {
                    Some(next) if placed > next + gap => placed,
                    Some(next) => next,
                    None => placed,
                };
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
