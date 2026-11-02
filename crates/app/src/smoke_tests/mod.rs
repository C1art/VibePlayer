//! Ignored benchmarks and robustness runs that print a report and need no screen.
//! `cargo test --release -p app <name> -- --ignored --nocapture`, with `PLAY_BENCH_FILES`
//! (`;`-separated) or the bundled fixtures.

mod benchmarks;
mod robustness;

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use media::{HardwareDecoding, Speed, VideoOptions};
use playback::{Player, PlayerOptions};

pub(crate) fn files() -> Vec<PathBuf> {
    match std::env::var("PLAY_BENCH_FILES") {
        Ok(list) if !list.trim().is_empty() => list
            .split(';')
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .collect(),
        _ => fixtures()
            .into_iter()
            .filter(|p| !p.ends_with("truncated.mp4"))
            .collect(),
    }
}

pub(crate) fn fixtures() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e != "sh"));
    files.sort();
    files
}

pub(crate) fn hardware() -> HardwareDecoding {
    if std::env::var("PLAY_BENCH_SOFTWARE_DECODE").is_ok_and(|v| v == "1") {
        HardwareDecoding::Off
    } else {
        HardwareDecoding::Auto
    }
}

pub(crate) fn options(autoplay: bool) -> PlayerOptions {
    PlayerOptions {
        video: VideoOptions {
            hardware: hardware(),
            high_bit_depth: true,
        },
        volume: audio::Volume::from_percent(0),
        muted: true,
        speed: Speed::NORMAL,
        start_at: None,
        autoplay,
    }
}

/// Updates the player like the UI would until `done`; `None` on timeout.
pub(crate) fn drive(
    player: &mut Player,
    timeout: Duration,
    mut done: impl FnMut(&Player) -> bool,
) -> Option<Duration> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        player.update(Instant::now());
        if done(player) {
            return Some(start.elapsed());
        }
        thread::sleep(Duration::from_millis(1));
    }
    None
}

/// A tiny deterministic generator, so runs are repeatable without a dependency.
pub(crate) struct Random(u64);

impl Random {
    pub fn new(seed: u64) -> Random {
        Random(seed.max(1))
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound.max(1)
    }
}

pub(crate) fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

pub(crate) fn summary(label: &str, samples: &mut [f64]) {
    if samples.is_empty() {
        println!("  {label}: no samples");
        return;
    }
    samples.sort_by(f64::total_cmp);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let p95 = samples[(samples.len() * 95 / 100).min(samples.len() - 1)];
    println!(
        "  {label}: n={} mean={mean:.1} p95={p95:.1} max={:.1}",
        samples.len(),
        samples[samples.len() - 1]
    );
}
