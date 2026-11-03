use std::time::Instant;

use cut::{SampleRate, Ticks, Timebase};
use media::Speed;

/// Counts seeks; frames, samples and anchors from an older generation are stale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(pub u64);

/// Ties a device sample count to a media timestamp: the sample played as frame
/// `played_frames` of the device stream has presentation time `pts`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioAnchor {
    pub generation: Generation,
    pub played_frames: u64,
    pub pts: Ticks,
    pub speed: Speed,
}

/// The latest anchor and the one before it. A new anchor is published as soon as its samples
/// are queued, but the samples queued ahead of them still belong to the previous one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnchorPair {
    pub current: AudioAnchor,
    pub previous: Option<AudioAnchor>,
}

/// A snapshot of the audio device, taken when reading the clock.
#[derive(Clone, Copy, Debug)]
pub struct AudioReading {
    pub anchors: AnchorPair,
    pub played_frames: u64,
    pub latency: Ticks,
    pub sample_rate: SampleRate,
}

impl AudioReading {
    /// The media time currently audible.
    pub fn position(&self) -> Ticks {
        let timebase = self.sample_rate.timebase().unwrap_or(Timebase::MICROS);
        let latency_frames = self.latency.rescale(Timebase::MICROS, timebase).0;
        let audible = self.played_frames as i64 - latency_frames;
        let current = self.anchors.current;
        let anchor = match self.anchors.previous {
            Some(previous) if audible < current.played_frames as i64 => previous,
            _ => current,
        };
        let frames = audible - anchor.played_frames as i64;
        let wall = Ticks(frames).rescale(timebase, Timebase::MICROS);
        anchor.pts + anchor.speed.wall_to_media(wall)
    }
}

/// The playback position. Follows the audio device when there is one, otherwise the system
/// clock, and never runs backwards except on a seek.
#[derive(Debug)]
pub struct Clock {
    position: Ticks,
    playing: bool,
    speed: Speed,
    system_anchor: (Ticks, Instant),
}

impl Clock {
    pub fn new(now: Instant) -> Clock {
        Clock {
            position: Ticks::ZERO,
            playing: false,
            speed: Speed::NORMAL,
            system_anchor: (Ticks::ZERO, now),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn speed(&self) -> Speed {
        self.speed
    }

    /// Advances and returns the position. `audio` is `None` when there's no usable audio clock.
    pub fn tick(&mut self, now: Instant, audio: Option<AudioReading>) -> Ticks {
        if !self.playing {
            return self.position;
        }
        let position = match audio {
            Some(reading) => {
                let position = reading.position();
                self.system_anchor = (position.max(self.position), now);
                position
            }
            None => {
                let (base, since) = self.system_anchor;
                let elapsed = now.saturating_duration_since(since);
                let elapsed = Ticks(i64::try_from(elapsed.as_micros()).unwrap_or(i64::MAX));
                base + self.speed.wall_to_media(elapsed)
            }
        };
        self.position = self.position.max(position);
        self.position
    }

    pub fn position(&self) -> Ticks {
        self.position
    }

    pub fn play(&mut self, now: Instant) {
        self.playing = true;
        self.system_anchor = (self.position, now);
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Moves to `position`, e.g. after a seek or a frame step.
    pub fn jump(&mut self, now: Instant, position: Ticks) {
        self.position = position;
        self.system_anchor = (position, now);
    }

    pub fn set_speed(&mut self, now: Instant, speed: Speed) {
        self.system_anchor = (self.position, now);
        self.speed = speed;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn reading(played: u64, latency: i64) -> AudioReading {
        AudioReading {
            anchors: AnchorPair {
                current: AudioAnchor {
                    generation: Generation(1),
                    played_frames: 48_000,
                    pts: Ticks(10_000_000),
                    speed: Speed::NORMAL,
                },
                previous: None,
            },
            played_frames: played,
            latency: Ticks(latency),
            sample_rate: SampleRate(48_000),
        }
    }

    #[test]
    fn audio_position_counts_played_frames_from_the_anchor() {
        assert_eq!(reading(48_000 + 24_000, 0).position(), Ticks(10_500_000));
    }

    #[test]
    fn audio_position_subtracts_output_latency() {
        assert_eq!(
            reading(48_000 + 24_000, 20_000).position(),
            Ticks(10_480_000)
        );
    }

    #[test]
    fn audio_position_scales_with_the_anchor_speed() {
        let mut fast = reading(48_000 + 24_000, 0);
        fast.anchors.current.speed = Speed::from_percent(200);
        assert_eq!(fast.position(), Ticks(11_000_000));
    }

    #[test]
    fn samples_queued_before_a_new_anchor_keep_the_old_one() {
        // At 1x from 0 s; the new 2x anchor starts at device frame 48 000 (1 s in).
        let mut changed = reading(24_000, 0);
        changed.anchors.previous = Some(AudioAnchor {
            played_frames: 0,
            pts: Ticks::ZERO,
            ..changed.anchors.current
        });
        changed.anchors.current.pts = Ticks(1_000_000);
        changed.anchors.current.speed = Speed::from_percent(200);
        assert_eq!(changed.position(), Ticks(500_000));
        changed.played_frames = 72_000;
        assert_eq!(changed.position(), Ticks(2_000_000));
    }

    #[test]
    fn system_clock_advances_at_playback_speed() {
        let start = Instant::now();
        let mut clock = Clock::new(start);
        clock.jump(start, Ticks(1_000_000));
        clock.set_speed(start, Speed::from_percent(150));
        clock.play(start);
        let position = clock.tick(start + Duration::from_secs(2), None);
        assert_eq!(position, Ticks(4_000_000));
    }

    #[test]
    fn clock_never_runs_backwards_while_playing() {
        let start = Instant::now();
        let mut clock = Clock::new(start);
        clock.jump(start, Ticks(10_600_000));
        clock.play(start);
        assert_eq!(
            clock.tick(start, Some(reading(48_000 + 24_000, 0))),
            Ticks(10_600_000)
        );
    }

    #[test]
    fn paused_clock_holds_its_position() {
        let start = Instant::now();
        let mut clock = Clock::new(start);
        clock.jump(start, Ticks(5));
        assert_eq!(clock.tick(start + Duration::from_secs(9), None), Ticks(5));
    }
}
