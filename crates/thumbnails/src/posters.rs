//! One still per file for the recent list, extracted on a worker thread.

use std::collections::HashSet;
use std::path::PathBuf;
use std::thread;

use crossbeam::channel::{self, Receiver, Sender};
use cut::Ticks;
use media::{RgbaImage, ThumbnailExtractor};

const POSTER_WIDTH: u32 = 176;

struct Request {
    path: PathBuf,
    at: Option<Ticks>,
}

pub struct Posters {
    requests: Sender<Request>,
    results: Receiver<(PathBuf, Option<RgbaImage>)>,
    asked: HashSet<PathBuf>,
}

impl Posters {
    pub fn new() -> Posters {
        let (requests, request_receiver) = channel::unbounded();
        let (result_sender, results) = channel::unbounded();
        let _ = thread::Builder::new()
            .name("posters".into())
            .spawn(move || run(&request_receiver, &result_sender));
        Posters {
            requests,
            results,
            asked: HashSet::new(),
        }
    }

    /// Asks once per file for a still at `at`, or a tenth of the way in without one.
    pub fn request(&mut self, path: PathBuf, at: Option<Ticks>) {
        if self.asked.insert(path.clone()) {
            let _ = self.requests.send(Request { path, at });
        }
    }

    /// Takes in finished stills; `None` for files that couldn't give one.
    pub fn poll(&mut self) -> Vec<(PathBuf, Option<RgbaImage>)> {
        self.results.try_iter().collect()
    }
}

impl Default for Posters {
    fn default() -> Posters {
        Posters::new()
    }
}

fn run(requests: &Receiver<Request>, results: &Sender<(PathBuf, Option<RgbaImage>)>) {
    for request in requests {
        let image = poster(&request);
        if results.send((request.path, image)).is_err() {
            return;
        }
    }
}

fn poster(request: &Request) -> Option<RgbaImage> {
    let mut extractor = ThumbnailExtractor::open(&request.path, POSTER_WIDTH).ok()?;
    let at = request
        .at
        .or_else(|| extractor.duration().map(|duration| Ticks(duration.0 / 10)))
        .unwrap_or(Ticks::ZERO);
    match extractor.extract(at).ok().flatten() {
        Some(image) => Some(image),
        None => extractor.extract(Ticks::ZERO).ok().flatten(),
    }
}
