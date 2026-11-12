//! Sums the decoded audio of several tracks, lined up by their timestamps.

use std::collections::VecDeque;

use cut::{SampleRate, Ticks};
use media::Speed;

/// Frames mixed and handed out at most per call.
const MAX_CHUNK_FRAMES: usize = 4096;

/// Where output frame `frame` sits on the media timeline; frames advance at `speed`.
#[derive(Clone, Copy, Debug)]
struct Base {
    frame: i64,
    pts: Ticks,
    speed: Speed,
}

#[derive(Default)]
struct Track {
    /// Output frame index of the first buffered frame.
    start: i64,
    samples: VecDeque<f32>,
    seen: bool,
    ended: bool,
    weight: f32,
}

/// Mixed audio ready for the device.
#[derive(Debug, PartialEq)]
pub(crate) struct Mixed {
    /// Media time of the first frame, in microseconds.
    pub pts: Ticks,
    /// Interleaved samples.
    pub samples: Vec<f32>,
}

pub(crate) struct Mixer {
    channels: usize,
    rate: i64,
    tracks: Vec<Track>,
    base: Option<Base>,
    /// The next output frame to hand out.
    cursor: i64,
}

impl Mixer {
    pub fn new(channels: u16, rate: SampleRate, tracks: usize) -> Mixer {
        let mut mixer = Mixer {
            channels: usize::from(channels.max(1)),
            rate: i64::from(rate.0.max(1)),
            tracks: Vec::new(),
            base: None,
            cursor: 0,
        };
        mixer.set_track_count(tracks);
        mixer
    }

    /// Starts over with `tracks` empty tracks, all at full weight.
    pub fn set_track_count(&mut self, tracks: usize) {
        self.tracks = (0..tracks)
            .map(|_| Track {
                weight: 1.0,
                ..Track::default()
            })
            .collect();
        self.reset();
    }

    /// Forgets everything buffered, e.g. after a seek.
    pub fn reset(&mut self) {
        for track in &mut self.tracks {
            track.samples.clear();
            track.seen = false;
            track.ended = false;
        }
        self.base = None;
        self.cursor = 0;
    }

    /// How loud each track is in the mix, 0 to 1. A silent track still keeps time.
    pub fn set_weight(&mut self, track: usize, weight: f32) {
        if let Some(track) = self.tracks.get_mut(track) {
            track.weight = weight;
        }
    }

    /// Adds `samples` of `track` that start at media time `pts` and were stretched to `speed`.
    pub fn push(&mut self, track: usize, pts: Ticks, samples: &[f32], speed: Speed) {
        let base = self.base_for(pts, speed);
        let position = self.frame_at(base, pts);
        let channels = self.channels;
        let tolerance = self.rate / 100;
        let cursor = self.cursor;
        let Some(buffer) = self.tracks.get_mut(track) else {
            return;
        };
        let end = buffer.start + (buffer.samples.len() / channels) as i64;
        let mut samples = samples;
        let mut position = position;
        if !buffer.seen || buffer.samples.is_empty() && position > end + tolerance {
            buffer.seen = true;
            buffer.samples.clear();
            buffer.start = position.max(cursor);
        } else if position > end + tolerance {
            // A gap in this track: silence until its next samples.
            let gap = (position - end) as usize * channels;
            buffer.samples.extend(std::iter::repeat_n(0.0, gap));
        } else if position < end - tolerance {
            // Overlaps what is already buffered; keep the earlier samples.
            let overlap = ((end - position) as usize * channels).min(samples.len());
            samples = &samples[overlap..];
            position = end;
        }
        // Samples from before the cursor were already handed out (as silence, for this track).
        let late = ((cursor - position).max(0) as usize * channels).min(samples.len());
        if buffer.samples.is_empty() {
            buffer.start = position.max(cursor);
        }
        buffer.samples.extend(&samples[late..]);
    }

    /// Marks every track as ended, so whatever is buffered can be handed out.
    pub fn finish(&mut self) {
        for track in &mut self.tracks {
            track.ended = true;
        }
    }

    /// Mixes and hands out what every track has reached, up to a chunk at a time. A track
    /// holds the others back by at most `lag_limit`; past that, its missing part plays as
    /// silence.
    pub fn pop(&mut self, lag_limit: Ticks) -> Option<Mixed> {
        let base = self.base?;
        let channels = self.channels;
        let ends: Vec<(i64, bool)> = self
            .tracks
            .iter()
            .map(|track| {
                let end = if track.seen {
                    track.start + (track.samples.len() / channels) as i64
                } else {
                    self.cursor
                };
                (end, track.ended)
            })
            .collect();
        let furthest = ends.iter().map(|(end, _)| *end).max()?;
        let lag = lag_limit.0.saturating_mul(self.rate) / 1_000_000;
        let ready = ends
            .iter()
            .filter(|(_, ended)| !ended)
            .map(|(end, _)| (*end).max(furthest - lag))
            .min()
            .unwrap_or(furthest);
        let frames = (ready - self.cursor).clamp(0, MAX_CHUNK_FRAMES as i64) as usize;
        if frames == 0 {
            return None;
        }
        let mut mixed = vec![0.0f32; frames * channels];
        for track in &mut self.tracks {
            let skip =
                ((self.cursor - track.start).max(0) as usize * channels).min(track.samples.len());
            track.samples.drain(..skip);
            track.start = track.start.max(self.cursor);
            // A track whose samples start after this chunk adds nothing to it.
            let offset = ((track.start - self.cursor) as usize * channels).min(mixed.len());
            let available = track.samples.len().min(mixed.len() - offset);
            if track.weight > 0.0 {
                for (out, sample) in mixed[offset..offset + available]
                    .iter_mut()
                    .zip(track.samples.iter())
                {
                    *out += sample * track.weight;
                }
            }
            track.samples.drain(..available);
            track.start += (available / channels) as i64;
        }
        for sample in &mut mixed {
            *sample = sample.clamp(-1.0, 1.0);
        }
        let pts = self.pts_at(base, self.cursor);
        self.cursor += frames as i64;
        Some(Mixed {
            pts,
            samples: mixed,
        })
    }

    /// The timeline mapping, started at the first samples and restarted at the cursor when
    /// the speed changes.
    fn base_for(&mut self, pts: Ticks, speed: Speed) -> Base {
        let base = match self.base {
            None => Base {
                frame: self.cursor,
                pts,
                speed,
            },
            Some(base) if base.speed != speed => Base {
                frame: self.cursor,
                pts: self.pts_at(base, self.cursor),
                speed,
            },
            Some(base) => base,
        };
        self.base = Some(base);
        base
    }

    fn frame_at(&self, base: Base, pts: Ticks) -> i64 {
        let wall = base.speed.media_to_wall(pts - base.pts);
        base.frame + rounded_div(i128::from(wall.0) * i128::from(self.rate), 1_000_000)
    }

    fn pts_at(&self, base: Base, frame: i64) -> Ticks {
        let wall = rounded_div(
            i128::from(frame - base.frame) * 1_000_000,
            i128::from(self.rate),
        );
        base.pts + base.speed.wall_to_media(Ticks(wall))
    }
}

fn rounded_div(value: i128, divisor: i128) -> i64 {
    let half = divisor / 2;
    let rounded = if value >= 0 {
        (value + half) / divisor
    } else {
        (value - half) / divisor
    };
    i64::try_from(rounded).unwrap_or(if value >= 0 { i64::MAX } else { i64::MIN })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAG: Ticks = Ticks(1_000_000);

    /// Mono at 1 kHz, so one frame is a millisecond.
    fn mixer(tracks: usize) -> Mixer {
        Mixer::new(1, SampleRate(1000), tracks)
    }

    fn millis(ms: i64) -> Ticks {
        Ticks(ms * 1000)
    }

    fn drain(mixer: &mut Mixer) -> Vec<f32> {
        let mut out = Vec::new();
        while let Some(mixed) = mixer.pop(LAG) {
            out.extend(mixed.samples);
        }
        out
    }

    #[test]
    fn overlapping_tracks_are_summed_sample_by_sample() {
        let mut mixer = mixer(2);
        mixer.push(0, millis(0), &[0.25; 10], Speed::NORMAL);
        mixer.push(1, millis(0), &[0.5; 10], Speed::NORMAL);
        assert_eq!(drain(&mut mixer), vec![0.75; 10]);
    }

    #[test]
    fn output_waits_for_the_track_that_is_behind() {
        let mut mixer = mixer(2);
        mixer.push(0, millis(0), &[0.25; 10], Speed::NORMAL);
        mixer.push(1, millis(0), &[0.5; 4], Speed::NORMAL);
        let first = mixer.pop(LAG).unwrap();
        assert_eq!(first.samples, vec![0.75; 4]);
        assert_eq!(mixer.pop(LAG), None);
        mixer.push(1, millis(4), &[0.5; 6], Speed::NORMAL);
        let second = mixer.pop(LAG).unwrap();
        assert_eq!(second.pts, millis(4));
        assert_eq!(second.samples, vec![0.75; 6]);
    }

    #[test]
    fn a_track_that_starts_later_is_silent_until_then() {
        let mut mixer = mixer(2);
        mixer.push(0, millis(0), &[0.25; 10], Speed::NORMAL);
        mixer.push(1, millis(6), &[0.5; 4], Speed::NORMAL);
        let mut expected = vec![0.25; 6];
        expected.extend([0.75; 4]);
        assert_eq!(drain(&mut mixer), expected);
    }

    #[test]
    fn a_track_starting_after_the_mixed_stretch_is_kept_for_later() {
        let mut mixer = mixer(2);
        mixer.push(0, millis(0), &[0.25; 4], Speed::NORMAL);
        mixer.push(1, millis(6), &[0.5; 4], Speed::NORMAL);
        assert_eq!(mixer.pop(LAG).unwrap().samples, vec![0.25; 4]);
        mixer.push(0, millis(4), &[0.25; 6], Speed::NORMAL);
        let mut expected = vec![0.25; 2];
        expected.extend([0.75; 4]);
        assert_eq!(drain(&mut mixer), expected);
    }

    #[test]
    fn a_track_far_behind_does_not_hold_the_others_back() {
        let mut mixer = mixer(2);
        mixer.push(0, millis(0), &[0.25; 1500], Speed::NORMAL);
        let out = drain(&mut mixer);
        assert_eq!(out.len(), 500);
        assert!(out.iter().all(|&s| s == 0.25));
    }

    #[test]
    fn a_gap_inside_a_track_plays_as_silence() {
        let mut mixer = mixer(1);
        mixer.push(0, millis(0), &[0.5; 5], Speed::NORMAL);
        mixer.push(0, millis(20), &[0.5; 5], Speed::NORMAL);
        mixer.finish();
        let out = drain(&mut mixer);
        assert_eq!(out.len(), 25);
        assert!(out[5..20].iter().all(|&s| s == 0.0));
        assert!(out[20..].iter().all(|&s| s == 0.5));
    }

    #[test]
    fn finishing_hands_out_whatever_is_left() {
        let mut mixer = mixer(2);
        mixer.push(0, millis(0), &[0.25; 10], Speed::NORMAL);
        mixer.push(1, millis(0), &[0.5; 4], Speed::NORMAL);
        drain(&mut mixer);
        mixer.finish();
        assert_eq!(drain(&mut mixer), vec![0.25; 6]);
    }

    #[test]
    fn a_zero_weight_track_is_silent_but_still_keeps_time() {
        let mut mixer = mixer(2);
        mixer.set_weight(1, 0.0);
        mixer.push(0, millis(0), &[0.25; 10], Speed::NORMAL);
        mixer.push(1, millis(0), &[0.5; 4], Speed::NORMAL);
        assert_eq!(drain(&mut mixer), vec![0.25; 4]);
    }

    #[test]
    fn the_sum_is_kept_within_full_scale() {
        let mut mixer = mixer(2);
        mixer.push(0, millis(0), &[0.75; 3], Speed::NORMAL);
        mixer.push(1, millis(0), &[-0.75, 0.75, 0.0], Speed::NORMAL);
        assert_eq!(drain(&mut mixer), vec![0.0, 1.0, 0.75]);
    }

    #[test]
    fn at_double_speed_a_frame_covers_twice_the_media_time() {
        let fast = Speed::from_percent(200);
        let mut mixer = mixer(2);
        mixer.push(0, millis(0), &[0.25; 10], fast);
        mixer.push(1, millis(0), &[0.5; 5], fast);
        mixer.push(1, millis(10), &[0.5; 5], fast);
        let first = mixer.pop(LAG).unwrap();
        assert_eq!(first.samples, vec![0.75; 10]);
        mixer.push(0, millis(20), &[0.25; 2], fast);
        mixer.push(1, millis(20), &[0.5; 2], fast);
        assert_eq!(mixer.pop(LAG).unwrap().pts, millis(20));
    }
}
