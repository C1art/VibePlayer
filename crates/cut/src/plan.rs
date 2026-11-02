use std::fmt;

use crate::{StreamIndex, Ticks, Timebase};

/// The user's trim, in the primary stream's timebase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrimRequest {
    pub stream: StreamIndex,
    pub timebase: Timebase,
    pub start: Ticks,
    pub end: Ticks,
}

/// Where a lossless cut actually lands. The start moves back to a keyframe because copied
/// video can only begin on one; the UI shows this before export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CutPlan {
    pub stream: StreamIndex,
    pub timebase: Timebase,
    pub requested_start: Ticks,
    pub start: Ticks,
    pub end: Ticks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutError {
    /// The end is not after the start.
    EmptyRange,
    /// No keyframe at or before the start, so nothing can be copied from there.
    NoKeyframe,
}

impl fmt::Display for CutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CutError::EmptyRange => f.write_str("the end must be after the start"),
            CutError::NoKeyframe => f.write_str("no keyframe at or before the start"),
        }
    }
}

impl std::error::Error for CutError {}

impl TrimRequest {
    /// Plans the cut given the keyframe found at or before the start.
    pub fn plan(&self, keyframe: Option<Ticks>) -> Result<CutPlan, CutError> {
        if self.end <= self.start {
            return Err(CutError::EmptyRange);
        }
        let start = keyframe
            .filter(|&keyframe| keyframe <= self.start)
            .ok_or(CutError::NoKeyframe)?;
        Ok(CutPlan {
            stream: self.stream,
            timebase: self.timebase,
            requested_start: self.start,
            start,
            end: self.end,
        })
    }
}

impl CutPlan {
    /// How far the start moved back to reach a keyframe.
    pub fn start_shift(&self) -> Ticks {
        self.requested_start - self.start
    }

    pub fn duration(&self) -> Ticks {
        self.end - self.start
    }

    /// This plan's range expressed in another stream's timebase.
    pub fn range_in(&self, timebase: Timebase) -> (Ticks, Ticks) {
        (
            self.start.rescale(self.timebase, timebase),
            self.end.rescale(self.timebase, timebase),
        )
    }

    /// Whether a packet of the primary (video) stream belongs in the output. Frames shown
    /// before the start keyframe (open-GOP leading frames) can't be decoded without the
    /// previous GOP, so they go. Frames shown after the end are kept only if they're decoded
    /// before it, because frames inside the range may reference them.
    pub fn keeps_primary(&self, pts: Ticks, dts: Option<Ticks>) -> bool {
        if pts < self.start {
            return false;
        }
        pts < self.end || dts.is_some_and(|dts| dts < self.end)
    }

    /// Whether a packet of any other stream, stamped in `timebase`, belongs in the output.
    pub fn keeps_other(&self, pts: Ticks, timebase: Timebase) -> bool {
        let (start, end) = self.range_in(timebase);
        pts >= start && pts < end
    }

    /// Moves a timestamp so the cut starts at zero.
    pub fn rebase(&self, ticks: Ticks, timebase: Timebase) -> Ticks {
        ticks - self.start.rescale(self.timebase, timebase)
    }

    /// Whether reading can stop: a packet decoded at `dts` in `timebase` is past every packet
    /// the output needs.
    pub fn is_past_end(&self, dts: Ticks, timebase: Timebase) -> bool {
        // B-frame reordering spans well under a second; give it two to be safe.
        let margin = Ticks(2_000).rescale(Timebase::MILLIS, timebase);
        dts > self.range_in(timebase).1 + margin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms() -> Timebase {
        Timebase::MILLIS
    }

    fn request(start: i64, end: i64) -> TrimRequest {
        TrimRequest {
            stream: StreamIndex(0),
            timebase: ms(),
            start: Ticks(start),
            end: Ticks(end),
        }
    }

    #[test]
    fn start_snaps_back_to_the_keyframe() {
        let plan = request(5_300, 9_000).plan(Some(Ticks(5_000))).unwrap();
        assert_eq!(plan.start, Ticks(5_000));
        assert_eq!(plan.start_shift(), Ticks(300));
        assert_eq!(plan.duration(), Ticks(4_000));
    }

    #[test]
    fn start_on_a_keyframe_does_not_move() {
        let plan = request(5_000, 9_000).plan(Some(Ticks(5_000))).unwrap();
        assert_eq!(plan.start_shift(), Ticks::ZERO);
    }

    #[test]
    fn empty_or_reversed_ranges_are_rejected() {
        assert_eq!(
            request(5_000, 5_000).plan(Some(Ticks(0))),
            Err(CutError::EmptyRange)
        );
        assert_eq!(
            request(6_000, 5_000).plan(Some(Ticks(0))),
            Err(CutError::EmptyRange)
        );
    }

    #[test]
    fn a_keyframe_after_the_start_cannot_be_used() {
        assert_eq!(
            request(100, 900).plan(Some(Ticks(200))),
            Err(CutError::NoKeyframe)
        );
        assert_eq!(request(100, 900).plan(None), Err(CutError::NoKeyframe));
    }

    #[test]
    fn open_gop_leading_frames_before_the_keyframe_are_dropped() {
        let plan = request(5_000, 9_000).plan(Some(Ticks(5_000))).unwrap();
        assert!(!plan.keeps_primary(Ticks(4_960), Some(Ticks(5_000))));
        assert!(plan.keeps_primary(Ticks(5_000), Some(Ticks(4_920))));
    }

    #[test]
    fn reference_frames_decoded_before_the_end_are_kept() {
        let plan = request(0, 9_000).plan(Some(Ticks(0))).unwrap();
        assert!(plan.keeps_primary(Ticks(9_080), Some(Ticks(8_960))));
        assert!(!plan.keeps_primary(Ticks(9_080), Some(Ticks(9_000))));
    }

    #[test]
    fn other_streams_are_cut_in_their_own_timebase() {
        let plan = request(5_000, 9_000).plan(Some(Ticks(5_000))).unwrap();
        let audio = Timebase::new(1, 48_000).unwrap();
        assert!(!plan.keeps_other(Ticks(239_999), audio));
        assert!(plan.keeps_other(Ticks(240_000), audio));
        assert!(!plan.keeps_other(Ticks(432_000), audio));
    }

    #[test]
    fn rebasing_puts_the_start_at_zero_in_any_timebase() {
        let plan = request(5_300, 9_000).plan(Some(Ticks(5_000))).unwrap();
        let audio = Timebase::new(1, 48_000).unwrap();
        assert_eq!(plan.rebase(Ticks(240_000), audio), Ticks::ZERO);
        assert_eq!(plan.rebase(Ticks(5_040), ms()), Ticks(40));
    }

    #[test]
    fn reading_stops_two_seconds_past_the_end() {
        let plan = request(0, 9_000).plan(Some(Ticks(0))).unwrap();
        assert!(!plan.is_past_end(Ticks(11_000), ms()));
        assert!(plan.is_past_end(Ticks(11_001), ms()));
    }
}
