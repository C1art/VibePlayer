//! Background seek-bar previews with a bounded cache, and stills for the recent list.

mod cache;
mod posters;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use crossbeam::channel::{self, Receiver, Sender};
use cut::Ticks;
use media::{RgbaImage, ThumbnailExtractor};

pub use cache::BoundedCache;
pub use posters::Posters;

const THUMBNAIL_WIDTH: u32 = 192;
const CACHE_ENTRIES: usize = 160;
/// Previews are taken every this many microseconds at most…
const MIN_STEP_MICROS: i64 = 1_000_000;
/// …and at most this many per file.
const MAX_BUCKETS: i64 = 400;

/// Which preview slot a position falls into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bucket(i64);

/// Seek-bar previews for one file, extracted on a worker thread.
pub struct Thumbnails {
    requests: Sender<Bucket>,
    results: Receiver<(Bucket, Option<RgbaImage>)>,
    cache: BoundedCache<Bucket, Option<Arc<RgbaImage>>>,
    step: Ticks,
    in_flight: Option<Bucket>,
}

impl Thumbnails {
    pub fn open(path: PathBuf, duration: Ticks) -> Thumbnails {
        let step = Ticks((duration.0 / MAX_BUCKETS).max(MIN_STEP_MICROS));
        let (requests, request_receiver) = channel::unbounded::<Bucket>();
        let (result_sender, results) = channel::unbounded();
        let _ = thread::Builder::new()
            .name("thumbnails".into())
            .spawn(move || run(path, step, request_receiver, result_sender));
        Thumbnails {
            requests,
            results,
            cache: BoundedCache::new(CACHE_ENTRIES),
            step,
            in_flight: None,
        }
    }

    pub fn bucket(&self, position: Ticks) -> Bucket {
        Bucket(position.0.max(0) / self.step.0)
    }

    /// The preview for `position` if ready; otherwise asks for it and returns `None`.
    pub fn get(&mut self, position: Ticks) -> Option<Arc<RgbaImage>> {
        let bucket = self.bucket(position);
        if let Some(cached) = self.cache.get(&bucket) {
            return cached.clone();
        }
        if self.in_flight != Some(bucket) {
            self.in_flight = Some(bucket);
            let _ = self.requests.send(bucket);
        }
        None
    }

    /// Takes in finished previews. Returns `true` if any arrived.
    pub fn poll(&mut self) -> bool {
        let mut arrived = false;
        while let Ok((bucket, image)) = self.results.try_recv() {
            if self.in_flight == Some(bucket) {
                self.in_flight = None;
            }
            self.cache.insert(bucket, image.map(Arc::new));
            arrived = true;
        }
        arrived
    }
}

fn run(
    path: PathBuf,
    step: Ticks,
    requests: Receiver<Bucket>,
    results: Sender<(Bucket, Option<RgbaImage>)>,
) {
    let Ok(mut extractor) = ThumbnailExtractor::open(&path, THUMBNAIL_WIDTH) else {
        for bucket in requests {
            let _ = results.send((bucket, None));
        }
        return;
    };
    while let Ok(first) = requests.recv() {
        // While the user scrubs, only the newest request matters.
        let bucket = requests.try_iter().last().unwrap_or(first);
        let image = extractor.extract(Ticks(bucket.0 * step.0)).ok().flatten();
        if results.send((bucket, image)).is_err() {
            return;
        }
    }
}
