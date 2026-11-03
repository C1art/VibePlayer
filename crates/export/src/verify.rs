use std::path::Path;

use cut::{StreamIndex, Ticks, Timebase};
use media::{Demuxer, Packet};

use crate::ExportError;

/// Probes the written file and checks it matches the plan: every stream there, about the
/// expected length, and the video starting on a keyframe.
pub(crate) fn verify(
    path: &Path,
    streams: usize,
    video: Option<StreamIndex>,
    expected: Ticks,
) -> Result<Ticks, ExportError> {
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
    if let Some(video) = video
        && !starts_on_a_keyframe(path, video)?
    {
        return Err(ExportError::Verification(
            "the video doesn't start on a keyframe".into(),
        ));
    }
    Ok(duration)
}

fn starts_on_a_keyframe(path: &Path, video: StreamIndex) -> Result<bool, ExportError> {
    let failed = |error: media::MediaError| ExportError::Verification(error.to_string());
    let mut demuxer = Demuxer::open(path).map_err(failed)?;
    demuxer.select_streams(&[video]);
    let mut packet = Packet::new().map_err(failed)?;
    Ok(demuxer.read(&mut packet).map_err(failed)? && packet.is_keyframe())
}
