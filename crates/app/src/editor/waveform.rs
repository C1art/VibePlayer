use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam::channel::{self, Receiver, TryRecvError};
use cut::{StreamIndex, Ticks, Timebase};
use eframe::egui;
use media::{AudioDecoder, AudioEffects, AudioTarget, Demuxer, Packet};

/// Columns of loudness across the whole file; more than any screen is wide.
const BUCKETS: usize = 4_000;
/// Peaks don't need full bandwidth; a low rate makes decoding much cheaper.
const SAMPLE_RATE: u32 = 8_000;
const REPORT_EVERY: Duration = Duration::from_millis(250);

/// Each audio track's loudness over time, computed in the background and filled in as it
/// goes.
pub struct Waveform {
    updates: Receiver<Vec<(StreamIndex, Vec<f32>)>>,
    tracks: Vec<(StreamIndex, Vec<f32>)>,
    complete: bool,
}

impl Waveform {
    pub fn new(
        path: PathBuf,
        streams: Vec<StreamIndex>,
        duration: Ticks,
        repaint: egui::Context,
    ) -> Waveform {
        let (sender, updates) = channel::unbounded();
        let _ = thread::Builder::new()
            .name("waveform".into())
            .spawn(move || {
                let _ = scan(&path, &streams, duration, |tracks| {
                    repaint.request_repaint();
                    sender.send(tracks).is_ok()
                });
                drop(sender);
                repaint.request_repaint();
            });
        Waveform {
            updates,
            tracks: Vec::new(),
            complete: false,
        }
    }

    /// Takes in the latest snapshot; returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.updates.try_recv() {
                Ok(tracks) => {
                    self.tracks = tracks;
                    changed = true;
                }
                Err(TryRecvError::Empty) => return changed,
                Err(TryRecvError::Disconnected) => {
                    changed |= !self.complete;
                    self.complete = true;
                    return changed;
                }
            }
        }
    }

    /// The loudest of `streams` (0–1) per equal slice of the file; empty until the first
    /// report.
    pub fn peaks(&self, streams: &[StreamIndex]) -> Vec<f32> {
        let mut shown = self
            .tracks
            .iter()
            .filter(|(stream, _)| streams.contains(stream))
            .map(|(_, peaks)| peaks);
        let Some(first) = shown.next() else {
            return Vec::new();
        };
        let mut peaks = first.clone();
        for track in shown {
            for (peak, other) in peaks.iter_mut().zip(track) {
                *peak = peak.max(*other);
            }
        }
        peaks
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }
}

/// Decodes every one of `streams` to mono in one pass and reports each one's peaks so far
/// every quarter second, and at the end.
fn scan(
    path: &std::path::Path,
    streams: &[StreamIndex],
    duration: Ticks,
    mut report: impl FnMut(Vec<(StreamIndex, Vec<f32>)>) -> bool,
) -> Result<(), media::MediaError> {
    let mut demuxer = Demuxer::open(path)?;
    let origin = demuxer.info().start_time;
    demuxer.select_streams(streams);
    let target = AudioTarget {
        sample_format: "flt".into(),
        sample_rate: SAMPLE_RATE,
        layout: "mono".into(),
        frame_size: None,
    };
    let mut tracks = Vec::new();
    for &stream in streams {
        // A track that can't be decoded just has no waveform.
        let Ok(decoder) = demuxer
            .stream_params(stream)
            .and_then(|params| AudioDecoder::new(params, target.clone(), AudioEffects::default()))
        else {
            continue;
        };
        tracks.push((
            stream,
            decoder,
            PeakBuckets::new(BUCKETS, duration, SAMPLE_RATE),
        ));
    }
    let snapshot = |tracks: &[(StreamIndex, AudioDecoder, PeakBuckets)]| {
        tracks
            .iter()
            .map(|(stream, _, peaks)| (*stream, peaks.values().to_vec()))
            .collect()
    };
    let mut packet = Packet::new()?;
    let mut last_report = Instant::now();
    loop {
        let more = demuxer.read(&mut packet)?;
        for (stream, decoder, peaks) in &mut tracks {
            if more && packet.stream_index() != *stream {
                continue;
            }
            decoder.send(more.then_some(&packet))?;
            while let Some(frame) = decoder.receive()? {
                if let Some(pts) = frame.pts {
                    peaks.add(pts - origin, frame.samples());
                }
            }
        }
        if !more {
            report(snapshot(&tracks));
            return Ok(());
        }
        if last_report.elapsed() >= REPORT_EVERY {
            last_report = Instant::now();
            if !report(snapshot(&tracks)) {
                return Ok(());
            }
        }
    }
}

/// Folds samples into per-slice peaks by their time on the timeline.
struct PeakBuckets {
    peaks: Vec<f32>,
    duration: Ticks,
    sample_rate: Timebase,
}

impl PeakBuckets {
    fn new(buckets: usize, duration: Ticks, sample_rate: u32) -> PeakBuckets {
        PeakBuckets {
            peaks: vec![0.0; buckets.max(1)],
            duration: Ticks(duration.0.max(1)),
            sample_rate: Timebase::new(1, i64::from(sample_rate)).unwrap_or(Timebase::MICROS),
        }
    }

    /// `start` is the first sample's time in microseconds from the start of the file.
    fn add(&mut self, start: Ticks, samples: &[f32]) {
        let buckets = self.peaks.len() as i64;
        let first = start.rescale(Timebase::MICROS, self.sample_rate).0;
        let total = self
            .duration
            .rescale(Timebase::MICROS, self.sample_rate)
            .0
            .max(1);
        for (offset, sample) in samples.iter().enumerate() {
            let at = first + offset as i64;
            if !(0..total).contains(&at) {
                continue;
            }
            let bucket = (at * buckets / total) as usize;
            let peak = &mut self.peaks[bucket];
            *peak = peak.max(sample.abs().min(1.0));
        }
    }

    fn values(&self) -> &[f32] {
        &self.peaks
    }
}

#[cfg(test)]
mod tests {
    use super::PeakBuckets;
    use cut::Ticks;

    #[test]
    fn samples_land_in_the_slice_of_the_timeline_they_play_at() {
        // Four slices over one second at 8 samples per second: two samples per slice.
        let mut buckets = PeakBuckets::new(4, Ticks(1_000_000), 8);
        buckets.add(Ticks(500_000), &[0.5, -0.9, 0.1, 0.2]);
        assert_eq!(buckets.values(), [0.0, 0.0, 0.9, 0.2]);
    }

    #[test]
    fn samples_outside_the_file_and_overloads_are_ignored_or_clamped() {
        let mut buckets = PeakBuckets::new(2, Ticks(1_000_000), 4);
        buckets.add(Ticks(-500_000), &[1.0, 1.0, 3.0]);
        assert_eq!(buckets.values(), [1.0, 0.0]);
    }
}
