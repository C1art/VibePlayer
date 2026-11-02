use std::path::Path;

use cut::{Ticks, Timebase};

use crate::ExportError;

/// Probes the written file and checks it matches the plan.
pub(crate) fn verify(path: &Path, streams: usize, expected: Ticks) -> Result<Ticks, ExportError> {
    let info = media::probe(path).map_err(|error| ExportError::Verification(error.to_string()))?;
    if info.streams.len() != streams {
        return Err(ExportError::Verification(format!(
            "expected {streams} streams, found {}",
            info.streams.len()
        )));
    }
    let duration = info
        .duration
        .ok_or_else(|| ExportError::Verification("no duration".into()))?;
    // Keyframe and audio-frame boundaries make the result a little longer or shorter.
    let tolerance = Ticks((expected.0 / 10).max(1_500_000));
    if (duration - expected).0.abs() > tolerance.0 {
        let show = |t: Ticks| format!("{:.2}s", t.display_seconds(Timebase::MICROS));
        return Err(ExportError::Verification(format!(
            "duration {} instead of about {}",
            show(duration),
            show(expected)
        )));
    }
    Ok(duration)
}
