use cut::{StreamIndex, Ticks, Timebase};

use crate::{Demuxer, MediaError, Packet};

/// How many packets a keyframe lookup may read before giving up.
const SCAN_LIMIT: usize = 10_000;
/// How far back a lookup retries when the first seek lands after the target.
const RETRY_BACK: Ticks = Ticks(10_000_000);

/// The keyframes on either side of a position, in the stream's timebase.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyframeSpan {
    /// The last keyframe at or before the position.
    pub before: Option<Ticks>,
    /// The first keyframe after the position, if one was found.
    pub after: Option<Ticks>,
    /// When each frame from the position up to `after` (or as far as the scan went) is shown,
    /// in order.
    pub frame_times: Vec<Ticks>,
}

impl KeyframeSpan {
    /// Frames shown from the position up to the next keyframe.
    pub fn frames_until_after(&self) -> usize {
        self.frame_times.len()
    }
}

impl Demuxer {
    /// The last keyframe of `stream` at or before `target`, both in the stream's timebase.
    /// Changes the stream selection and read position.
    pub fn keyframe_at_or_before(
        &mut self,
        stream: StreamIndex,
        target: Ticks,
    ) -> Result<Option<Ticks>, MediaError> {
        Ok(self.keyframes_around(stream, target)?.before)
    }

    /// The keyframes around `target` and how many frames lie between it and the next one.
    /// Changes the stream selection and read position.
    pub fn keyframes_around(
        &mut self,
        stream: StreamIndex,
        target: Ticks,
    ) -> Result<KeyframeSpan, MediaError> {
        let timebase = self
            .timebase(stream)
            .ok_or(MediaError::NoSuchStream(stream))?;
        self.select_streams(&[stream]);
        let span = self.scan_around(stream, target, target)?;
        if span.before.is_some() {
            return Ok(span);
        }
        let back = RETRY_BACK.rescale(Timebase::MICROS, timebase);
        self.scan_around(stream, (target - back).max(Ticks::ZERO), target)
    }

    fn scan_around(
        &mut self,
        stream: StreamIndex,
        seek_to: Ticks,
        target: Ticks,
    ) -> Result<KeyframeSpan, MediaError> {
        self.seek_stream(stream, seek_to)?;
        let mut packet = Packet::new()?;
        let mut before = None;
        let mut after: Option<Ticks> = None;
        let mut shown = Vec::new();
        for _ in 0..SCAN_LIMIT {
            if !self.read(&mut packet)? {
                break;
            }
            let Some(pts) = packet.pts() else {
                continue;
            };
            let dts = packet.dts().unwrap_or(pts);
            if packet.is_keyframe() {
                if pts <= target {
                    before = before.max(Some(pts));
                } else if after.is_none_or(|known| pts < known) {
                    after = Some(pts);
                }
            }
            if pts >= target {
                shown.push(pts);
            }
            // Past the next keyframe, and past frames shown before it (open-GOP leading frames
            // follow their keyframe in decode order).
            if let Some(next) = after
                && pts > next
                && dts >= next
            {
                break;
            }
        }
        let limit = after.unwrap_or(Ticks(i64::MAX));
        shown.retain(|&pts| pts < limit);
        shown.sort_unstable();
        Ok(KeyframeSpan {
            before,
            after,
            frame_times: shown,
        })
    }
}
