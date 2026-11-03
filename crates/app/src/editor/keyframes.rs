use std::path::PathBuf;
use std::thread;

use crossbeam::channel::{self, Receiver, Sender, TryRecvError};
use cut::{StreamIndex, Ticks};
use eframe::egui;
use media::{Demuxer, KeyframeSpan};

/// Looks up the keyframes around the trim start on a worker, so dragging never blocks the UI.
pub struct KeyframeFinder {
    requests: Sender<Ticks>,
    results: Receiver<(Ticks, KeyframeSpan)>,
    latest: Option<(Ticks, KeyframeSpan)>,
    asked: Option<Ticks>,
}

impl KeyframeFinder {
    pub fn new(path: PathBuf, stream: StreamIndex, repaint: egui::Context) -> KeyframeFinder {
        let (requests, request_receiver) = channel::unbounded::<Ticks>();
        let (result_sender, results) = channel::unbounded();
        let _ = thread::Builder::new()
            .name("keyframe lookup".into())
            .spawn(move || {
                let mut demuxer = Demuxer::open(&path).ok();
                while let Ok(first) = request_receiver.recv() {
                    let target = request_receiver.try_iter().last().unwrap_or(first);
                    let span = demuxer
                        .as_mut()
                        .and_then(|d| d.keyframes_around(stream, target).ok())
                        .unwrap_or(KeyframeSpan {
                            before: None,
                            after: None,
                            frames_until_after: 0,
                        });
                    if result_sender.send((target, span)).is_err() {
                        return;
                    }
                    repaint.request_repaint();
                }
            });
        KeyframeFinder {
            requests,
            results,
            latest: None,
            asked: None,
        }
    }

    /// The keyframes around `target` (stream ticks), once known.
    pub fn lookup(&mut self, target: Ticks) -> Option<KeyframeSpan> {
        while let Ok(result) = self.results.try_recv() {
            self.latest = Some(result);
        }
        match self.latest {
            Some((asked, span)) if asked == target => Some(span),
            _ => {
                if self.asked != Some(target) {
                    self.asked = Some(target);
                    let _ = self.requests.send(target);
                }
                None
            }
        }
    }
}

/// Every keyframe of the primary stream, found by a background scan that fills in as it goes.
pub struct KeyframeIndex {
    batches: Receiver<Vec<Ticks>>,
    keyframes: Vec<Ticks>,
    complete: bool,
}

impl KeyframeIndex {
    pub fn new(path: PathBuf, stream: StreamIndex, repaint: egui::Context) -> KeyframeIndex {
        let (sender, batches) = channel::unbounded();
        let _ = thread::Builder::new()
            .name("keyframe index".into())
            .spawn(move || {
                if let Ok(mut demuxer) = Demuxer::open(&path) {
                    let _ = demuxer.scan_all_keyframes(stream, |batch| {
                        repaint.request_repaint();
                        sender.send(batch).is_ok()
                    });
                }
                drop(sender);
                repaint.request_repaint();
            });
        KeyframeIndex {
            batches,
            keyframes: Vec::new(),
            complete: false,
        }
    }

    /// Takes in new batches; returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.batches.try_recv() {
                Ok(batch) => {
                    self.keyframes.extend(batch);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    changed |= !self.complete;
                    self.complete = true;
                    break;
                }
            }
        }
        if changed {
            self.keyframes.sort_unstable();
            self.keyframes.dedup();
        }
        changed
    }

    /// Sorted keyframe timestamps in stream ticks.
    pub fn keyframes(&self) -> &[Ticks] {
        &self.keyframes
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }
}

/// The keyframe closest to `target`.
pub fn nearest(keyframes: &[Ticks], target: Ticks) -> Option<Ticks> {
    let at = keyframes.partition_point(|&k| k < target);
    let candidates = [
        at.checked_sub(1).and_then(|i| keyframes.get(i)),
        keyframes.get(at),
    ];
    candidates
        .into_iter()
        .flatten()
        .copied()
        .min_by_key(|k| (k.0 - target.0).unsigned_abs())
}

/// Where a start dragged to `target` snaps: the nearest keyframe, unless that would land at or
/// past `end` while `target` doesn't, which would move the end too; then the one before.
pub fn snap_start(keyframes: &[Ticks], target: Ticks, end: Ticks) -> Option<Ticks> {
    let snapped = nearest(keyframes, target)?;
    if snapped >= end && target < end {
        previous(keyframes, target + Ticks(1))
    } else {
        Some(snapped)
    }
}

/// The last keyframe strictly before `target`.
pub fn previous(keyframes: &[Ticks], target: Ticks) -> Option<Ticks> {
    let at = keyframes.partition_point(|&k| k < target);
    at.checked_sub(1).and_then(|i| keyframes.get(i)).copied()
}

/// The first keyframe strictly after `target`.
pub fn next(keyframes: &[Ticks], target: Ticks) -> Option<Ticks> {
    let at = keyframes.partition_point(|&k| k <= target);
    keyframes.get(at).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: [Ticks; 3] = [Ticks(0), Ticks(1_000), Ticks(2_000)];

    #[test]
    fn snapping_picks_the_closer_keyframe() {
        assert_eq!(nearest(&KEYS, Ticks(1_400)), Some(Ticks(1_000)));
        assert_eq!(nearest(&KEYS, Ticks(1_600)), Some(Ticks(2_000)));
        assert_eq!(nearest(&KEYS, Ticks(9_000)), Some(Ticks(2_000)));
        assert_eq!(nearest(&[], Ticks(5)), None);
    }

    #[test]
    fn a_snapped_start_stays_before_the_end() {
        assert_eq!(snap_start(&KEYS, Ticks(1_800), Ticks(1_900)), Some(Ticks(1_000)));
        assert_eq!(snap_start(&KEYS, Ticks(1_800), Ticks(5_000)), Some(Ticks(2_000)));
    }

    #[test]
    fn stepping_between_keyframes_skips_the_current_one() {
        assert_eq!(previous(&KEYS, Ticks(1_000)), Some(Ticks(0)));
        assert_eq!(next(&KEYS, Ticks(1_000)), Some(Ticks(2_000)));
        assert_eq!(previous(&KEYS, Ticks(0)), None);
        assert_eq!(next(&KEYS, Ticks(2_000)), None);
    }
}
