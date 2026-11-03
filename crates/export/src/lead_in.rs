//! Subtitle cues that begin before the cut and are still on screen when it starts.

use std::path::Path;

use cut::{CutPlan, StreamIndex, Ticks, Timebase};
use media::{Demuxer, Muxer, Packet};

use crate::ExportError;
use crate::remux::restamp;

/// How far before the cut to look for a cue that is still showing.
const LOOKBACK: Ticks = Ticks(10_000_000);

/// A copied subtitle stream: where it comes from and goes to.
pub(crate) struct SubtitleOutput {
    pub input: StreamIndex,
    pub output: StreamIndex,
    pub input_timebase: Timebase,
    pub output_timebase: Timebase,
}

/// Writes the part of each cue that is still on screen at the start, starting at zero. Seeking
/// to the cut lands after such cues, so this reads the stretch before it on its own.
pub(crate) fn write_cues_on_screen(
    source: &Path,
    plan: &CutPlan,
    subtitles: &[SubtitleOutput],
    muxer: &mut Muxer,
) -> Result<(), ExportError> {
    if subtitles.is_empty() {
        return Ok(());
    }
    let mut demuxer = Demuxer::open(source)?;
    let lookback = LOOKBACK.rescale(Timebase::MICROS, plan.timebase);
    demuxer.seek_stream(plan.stream, plan.start - lookback)?;
    let inputs: Vec<StreamIndex> = subtitles.iter().map(|s| s.input).collect();
    demuxer.select_streams(&inputs);
    let start = plan.range_in(Timebase::MICROS).0;
    let mut showing = Vec::new();
    let mut packet = Packet::new()?;
    while demuxer.read(&mut packet)? {
        let Some(stream) = subtitles.iter().find(|s| s.input == packet.stream_index()) else {
            continue;
        };
        let Some(pts) = packet.pts() else {
            continue;
        };
        if pts.rescale(stream.input_timebase, Timebase::MICROS) >= start {
            break;
        }
        if let Some((cue_start, duration)) =
            plan.on_screen_at_start(pts, packet.duration(), stream.input_timebase)
        {
            let mut cue = packet.try_clone()?;
            cue.set_timestamps(Some(cue_start), Some(cue_start));
            cue.set_duration(duration);
            restamp(
                &mut cue,
                plan,
                stream.input_timebase,
                stream.output_timebase,
                stream.output,
            );
            showing.push(cue);
        }
    }
    for mut cue in showing {
        muxer.write(&mut cue)?;
    }
    Ok(())
}
