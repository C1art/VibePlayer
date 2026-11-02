use std::fmt;
use std::ops::{Add, Sub};

use serde::{Deserialize, Serialize};

/// A positive rational timebase: one tick lasts `num / den` seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timebase {
    num: u32,
    den: u32,
}

impl Timebase {
    /// The timeline the player runs on.
    pub const MICROS: Timebase = Timebase {
        num: 1,
        den: 1_000_000,
    };
    pub const MILLIS: Timebase = Timebase { num: 1, den: 1_000 };

    /// Returns `None` unless both parts are positive, so corrupt stream headers can't divide by zero.
    pub fn new(num: i64, den: i64) -> Option<Timebase> {
        let num = u32::try_from(num).ok().filter(|&n| n > 0)?;
        let den = u32::try_from(den).ok().filter(|&d| d > 0)?;
        Some(Timebase { num, den })
    }

    pub fn num(self) -> u32 {
        self.num
    }

    pub fn den(self) -> u32 {
        self.den
    }
}

/// An integer count of ticks in some [`Timebase`].
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Ticks(pub i64);

impl Ticks {
    pub const ZERO: Ticks = Ticks(0);

    /// Converts between timebases, rounding to nearest (ties away from zero) and saturating.
    pub fn rescale(self, from: Timebase, to: Timebase) -> Ticks {
        if from == to {
            return self;
        }
        let numerator = i128::from(self.0) * i128::from(from.num) * i128::from(to.den);
        let denominator = i128::from(from.den) * i128::from(to.num);
        let half = denominator / 2;
        let rounded = if numerator >= 0 {
            (numerator + half) / denominator
        } else {
            (numerator - half) / denominator
        };
        Ticks(rounded.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
    }

    /// Seconds as a float, for display and UI geometry only.
    pub fn display_seconds(self, timebase: Timebase) -> f64 {
        self.0 as f64 * f64::from(timebase.num) / f64::from(timebase.den)
    }

    /// The nearest tick to a display position, e.g. where the user clicked on the seek bar.
    pub fn from_display_seconds(seconds: f64, timebase: Timebase) -> Ticks {
        let ticks = seconds * f64::from(timebase.den) / f64::from(timebase.num);
        Ticks(ticks.round() as i64)
    }

    pub fn saturating_add(self, other: Ticks) -> Ticks {
        Ticks(self.0.saturating_add(other.0))
    }

    pub fn saturating_sub(self, other: Ticks) -> Ticks {
        Ticks(self.0.saturating_sub(other.0))
    }
}

impl Add for Ticks {
    type Output = Ticks;
    fn add(self, other: Ticks) -> Ticks {
        self.saturating_add(other)
    }
}

impl Sub for Ticks {
    type Output = Ticks;
    fn sub(self, other: Ticks) -> Ticks {
        self.saturating_sub(other)
    }
}

/// The index of a stream inside its container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StreamIndex(pub usize);

/// A zero-based frame number within a stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameIndex(pub u64);

/// Audio samples per second per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SampleRate(pub u32);

impl SampleRate {
    pub fn timebase(self) -> Option<Timebase> {
        Timebase::new(1, i64::from(self.0))
    }
}

/// Formats a timestamp as `m:ss`, `h:mm:ss`, or with `.mmm` when `precision` asks for it.
pub struct ClockTime {
    millis: i64,
    precision: ClockPrecision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockPrecision {
    Seconds,
    Millis,
}

impl ClockTime {
    pub fn new(ticks: Ticks, timebase: Timebase, precision: ClockPrecision) -> ClockTime {
        ClockTime {
            millis: ticks.rescale(timebase, Timebase::MILLIS).0,
            precision,
        }
    }
}

impl fmt::Display for ClockTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.millis < 0 { "-" } else { "" };
        let millis = self.millis.unsigned_abs();
        let total_seconds = millis / 1000;
        let (hours, minutes, seconds) = (
            total_seconds / 3600,
            total_seconds / 60 % 60,
            total_seconds % 60,
        );
        if hours > 0 {
            write!(f, "{sign}{hours}:{minutes:02}:{seconds:02}")?;
        } else {
            write!(f, "{sign}{minutes}:{seconds:02}")?;
        }
        if self.precision == ClockPrecision::Millis {
            write!(f, ".{:03}", millis % 1000)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tb(num: i64, den: i64) -> Timebase {
        Timebase::new(num, den).unwrap()
    }

    #[test]
    fn rescale_rounds_to_nearest() {
        // 1/90000 s → µs: 1 tick = 11.11 µs, 2 ticks = 22.22 µs, 5 ticks = 55.56 µs.
        assert_eq!(Ticks(1).rescale(tb(1, 90_000), Timebase::MICROS), Ticks(11));
        assert_eq!(Ticks(5).rescale(tb(1, 90_000), Timebase::MICROS), Ticks(56));
    }

    #[test]
    fn rescale_rounds_negative_ticks_symmetrically() {
        assert_eq!(
            Ticks(-5).rescale(tb(1, 90_000), Timebase::MICROS),
            Ticks(-56)
        );
    }

    #[test]
    fn rescale_round_trips_ntsc_frame_durations() {
        let ntsc = tb(1001, 30_000);
        let micros = Ticks(300).rescale(ntsc, Timebase::MICROS);
        assert_eq!(micros, Ticks(10_010_000));
        assert_eq!(micros.rescale(Timebase::MICROS, ntsc), Ticks(300));
    }

    #[test]
    fn rescale_saturates_instead_of_overflowing() {
        let huge = Ticks(i64::MAX).rescale(tb(1, 1), Timebase::MICROS);
        assert_eq!(huge, Ticks(i64::MAX));
    }

    #[test]
    fn timebase_rejects_non_positive_parts() {
        assert_eq!(Timebase::new(0, 1), None);
        assert_eq!(Timebase::new(1, 0), None);
        assert_eq!(Timebase::new(-1, 25), None);
    }

    #[test]
    fn clock_time_formats_minutes_hours_and_millis() {
        let show = |millis, precision| {
            ClockTime::new(Ticks(millis), Timebase::MILLIS, precision).to_string()
        };
        assert_eq!(show(65_432, ClockPrecision::Seconds), "1:05");
        assert_eq!(show(3_725_004, ClockPrecision::Seconds), "1:02:05");
        assert_eq!(show(65_432, ClockPrecision::Millis), "1:05.432");
        assert_eq!(show(-1_500, ClockPrecision::Millis), "-0:01.500");
    }
}
