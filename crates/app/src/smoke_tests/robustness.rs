use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use cut::Ticks;
use playback::{LoadState, Player, SeekMode};

use super::{Random, drive, files, fixtures, options};

/// Random seeks, steps and track changes while playing; must neither panic nor hang.
fn random_seeks(mut on_frame: impl FnMut(&media::VideoFrame)) {
    for path in files() {
        let mut player = Player::open(path.clone(), options(true), Instant::now());
        if drive(&mut player, Duration::from_secs(10), |p| {
            p.opened().is_some() || matches!(p.state(), LoadState::Failed(_))
        })
        .is_none()
        {
            panic!("{} never opened", path.display());
        }
        let duration = player.duration().unwrap_or(Ticks(1_000_000)).0.max(1);
        let mut random = Random::new(42);
        for _ in 0..200 {
            match random.below(6) {
                0 => player.seek(
                    Ticks(random.below(duration as u64) as i64),
                    SeekMode::Keyframe,
                ),
                1 => player.seek(Ticks(random.below(duration as u64) as i64), SeekMode::Exact),
                2 => player.step_forward(Instant::now()),
                3 => player.step_backward(Instant::now()),
                4 => player.toggle_play(Instant::now()),
                _ => player.seek_relative(Ticks(random.below(20_000_000) as i64 - 10_000_000)),
            }
            for _ in 0..random.below(20) {
                if player.update(Instant::now()).frame_changed
                    && let Some(frame) = player.current_frame()
                {
                    on_frame(frame);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        player.seek(Ticks(duration / 3), SeekMode::Exact);
        let settled = drive(&mut player, Duration::from_secs(15), |p| !p.is_seeking());
        assert!(
            settled.is_some(),
            "{} hung after random seeks",
            path.display()
        );
        println!("  {}: ok", path.display());
    }
}

#[test]
#[ignore]
fn random_seeks_never_panic() {
    random_seeks(|_| {});
}

#[test]
#[ignore]
fn random_seeks_never_panic_with_a_gpu() {
    use eframe::wgpu;
    let instance = wgpu::Instance::default();
    let adapter = pollster_block(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .expect("no GPU adapter");
    let (device, queue) = pollster_block(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & render::optional_features(),
        ..Default::default()
    }))
    .expect("no GPU device");
    let mut renderer = render::VideoRenderer::new(&device, wgpu::TextureFormat::Bgra8Unorm);
    random_seeks(|frame| renderer.upload(&device, &queue, frame));
    assert!(renderer.has_frame());
}

/// Blocks on a future without an async runtime; wgpu's native futures complete on poll.
fn pollster_block<F: std::future::Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, Waker};
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Copies `source` with bytes overwritten at pseudo-random places.
fn corrupt_copy(source: &Path, seed: u64, out_dir: &Path) -> Option<PathBuf> {
    let mut bytes = std::fs::read(source).ok()?;
    let mut random = Random::new(seed);
    let header = bytes.len().min(4096) as u64;
    for _ in 0..(bytes.len() / 2000).max(8) {
        let at = (header + random.below(bytes.len() as u64 - header)) as usize;
        bytes[at] = random.next() as u8;
    }
    let output = out_dir.join(format!("{seed}-{}", source.file_name()?.to_string_lossy()));
    std::fs::write(&output, bytes).ok()?;
    Some(output)
}

#[test]
#[ignore]
fn corrupt_files_never_panic() {
    let out_dir = std::env::temp_dir().join("vibeplayer-corrupt");
    let _ = std::fs::create_dir_all(&out_dir);
    for source in fixtures() {
        for seed in 1..=4 {
            let Some(path) = corrupt_copy(&source, seed, &out_dir) else {
                continue;
            };
            let _ = media::probe(&path);
            if let Ok(mut thumbnails) = media::ThumbnailExtractor::open(&path, 96) {
                let _ = thumbnails.extract(Ticks(500_000));
            }
            let mut player = Player::open(path.clone(), options(true), Instant::now());
            let finished = drive(&mut player, Duration::from_secs(20), |p| {
                p.has_ended() || matches!(p.state(), LoadState::Failed(_))
            });
            assert!(finished.is_some(), "{} hung", path.display());
            export_whatever_we_can(&path, &out_dir);
            let _ = std::fs::remove_file(&path);
        }
        println!("  {}: ok", source.display());
    }
}

fn export_whatever_we_can(path: &Path, out_dir: &Path) {
    let Ok(mut demuxer) = media::Demuxer::open(path) else {
        return;
    };
    let Some(stream) = demuxer.best_stream(media::StreamKind::Video) else {
        return;
    };
    let Some(timebase) = demuxer.timebase(stream) else {
        return;
    };
    let request = cut::TrimRequest {
        stream,
        timebase,
        start: Ticks(0),
        end: Ticks(i64::from(timebase.den())),
    };
    let keyframe = demuxer
        .keyframe_at_or_before(stream, request.start)
        .ok()
        .flatten();
    let Ok(plan) = request.plan(keyframe) else {
        return;
    };
    let output = out_dir.join(format!(
        "export-{}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let _ = std::fs::remove_file(&output);
    let request = export::ExportRequest {
        source: path.to_path_buf(),
        output: output.clone(),
        plan,
        audio: export::AudioChanges::default(),
    };
    let _ = export::run(&request, &AtomicBool::new(false), &|_| {});
    let _ = std::fs::remove_file(&output);
}

/// Resident memory in bytes, where the platform reports it cheaply.
fn resident_memory() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

fn thread_count() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))?
        .trim()
        .parse()
        .ok()
}

#[test]
#[ignore]
fn a_long_session_levels_off() {
    let paths = files();
    let mut readings = Vec::new();
    for round in 0..12 {
        for path in &paths {
            let mut player = Player::open(path.clone(), options(true), Instant::now());
            drive(&mut player, Duration::from_secs(5), |p| {
                p.position() > Ticks(300_000)
                    || p.has_ended()
                    || matches!(p.state(), LoadState::Failed(_))
            });
            if let Some(duration) = player.duration() {
                player.seek(Ticks(duration.0 / 2), SeekMode::Exact);
                drive(&mut player, Duration::from_secs(5), |p| !p.is_seeking());
            }
        }
        // Workers exit asynchronously after the player drops.
        std::thread::sleep(Duration::from_millis(300));
        if let Some(memory) = resident_memory() {
            readings.push(memory);
            let threads = thread_count().map_or_else(String::new, |n| format!(", {n} threads"));
            println!("  round {round}: {:.1} MB{threads}", memory as f64 / 1e6);
        }
    }
    if readings.len() >= 12 {
        let early = readings[3] as f64;
        let late = readings[11] as f64;
        assert!(
            late < early * 1.25 + 20e6,
            "memory kept growing: {early} -> {late}"
        );
    }
}
