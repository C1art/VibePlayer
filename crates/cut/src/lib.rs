//! Cut planning: in/out points, keyframe snapping and timestamp rebasing. Pure logic, no I/O.
//! Also owns the time types every other crate shares.

pub mod time;

pub use time::{ClockPrecision, ClockTime, FrameIndex, SampleRate, StreamIndex, Ticks, Timebase};
