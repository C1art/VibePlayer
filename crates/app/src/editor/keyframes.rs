use std::path::PathBuf;
use std::thread;

use crossbeam::channel::{self, Receiver, Sender};
use cut::{StreamIndex, Ticks};
use media::Demuxer;

/// Looks up keyframes on a worker, so dragging the start handle never blocks the UI.
pub struct KeyframeFinder {
    requests: Sender<Ticks>,
    results: Receiver<(Ticks, Option<Ticks>)>,
    latest: Option<(Ticks, Option<Ticks>)>,
    asked: Option<Ticks>,
}

impl KeyframeFinder {
    pub fn new(path: PathBuf, stream: StreamIndex) -> KeyframeFinder {
        let (requests, request_receiver) = channel::unbounded::<Ticks>();
        let (result_sender, results) = channel::unbounded();
        let _ = thread::Builder::new()
            .name("keyframes".into())
            .spawn(move || {
                let mut demuxer = Demuxer::open(&path).ok();
                while let Ok(first) = request_receiver.recv() {
                    let target = request_receiver.try_iter().last().unwrap_or(first);
                    let keyframe = demuxer
                        .as_mut()
                        .and_then(|d| d.keyframe_at_or_before(stream, target).ok())
                        .flatten();
                    if result_sender.send((target, keyframe)).is_err() {
                        return;
                    }
                }
            });
        KeyframeFinder {
            requests,
            results,
            latest: None,
            asked: None,
        }
    }

    /// The keyframe at or before `target` (stream ticks): `Some(answer)` once known.
    pub fn lookup(&mut self, target: Ticks) -> Option<Option<Ticks>> {
        while let Ok(result) = self.results.try_recv() {
            self.latest = Some(result);
        }
        match self.latest {
            Some((asked, keyframe)) if asked == target => Some(keyframe),
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
