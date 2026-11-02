//! Cut planning: in/out points, keyframe snapping and timestamp rebasing. Pure logic, no I/O.
//! Also owns the time types every other crate shares.

mod plan;
pub mod time;

pub use plan::{CutError, CutPlan, ReencodedHead, TrimRequest};

pub use time::{ClockPrecision, ClockTime, FrameIndex, SampleRate, StreamIndex, Ticks, Timebase};
