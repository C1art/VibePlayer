use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use cut::{Ticks, Timebase, TrimRequest};
use export::{AudioChanges, ExportRequest};
use media::{Demuxer, Packet, StreamKind, ThumbnailExtractor, VideoDecoder, VideoOptions};
use playback::{Player, SeekMode};

use super::{Random, drive, files, hardware, millis, options, summary};

#[test]
#[ignore]
fn playback_speed() {
    println!("decode throughput (frames per second, decode + convert, no display)");
    for path in files() {
        let Ok(mut demuxer) = Demuxer::open(&path) else {
            continue;
        };
        let Some(stream) = demuxer.best_stream(StreamKind::Video) else {
            continue;
        };
        demuxer.select_streams(&[stream]);
        let options = VideoOptions {
            hardware: hardware(),
            high_bit_depth: true,
        };
        let Ok(mut decoder) = VideoDecoder::new(demuxer.stream_params(stream).unwrap(), options)
        else {
            continue;
        };
        let mut packet = Packet::new().unwrap();
        let start = Instant::now();
        let mut frames = 0u64;
        loop {
            let more = demuxer.read(&mut packet).unwrap_or(false);
            while !decoder.send(more.then_some(&packet)).unwrap_or(true) {
                while let Ok(Some(_)) = decoder.receive() {
                    frames += 1;
                }
            }
            while let Ok(Some(_)) = decoder.receive() {
                frames += 1;
            }
            if !more {
                break;
            }
        }
        let seconds = start.elapsed().as_secs_f64();
        let decoder_kind = if decoder.is_hardware() { "hw" } else { "sw" };
        println!(
            "  {} [{decoder_kind}]: {frames} frames, {:.0} fps",
            path.display(),
            frames as f64 / seconds
        );
    }
}

#[test]
#[ignore]
fn seek_latency() {
    println!("exact seek latency (ms from request to frame)");
    for path in files() {
        let mut player = Player::open(path.clone(), options(false), Instant::now());
        if drive(&mut player, Duration::from_secs(10), |p| {
            p.current_frame().is_some()
        })
        .is_none()
        {
            continue;
        }
        let duration = player.duration().unwrap_or(Ticks(1_000_000)).0.max(1);
        let mut random = Random::new(7);
        let mut samples = Vec::new();
        for _ in 0..30 {
            let target = Ticks(random.below(duration as u64) as i64);
            player.seek(target, SeekMode::Exact);
            if let Some(took) = drive(&mut player, Duration::from_secs(10), |p| !p.is_seeking()) {
                samples.push(millis(took));
            }
        }
        summary(&path.display().to_string(), &mut samples);
    }
}

#[test]
#[ignore]
fn frame_step_back() {
    println!("frame step back latency (ms)");
    for path in files() {
        let mut player = Player::open(path.clone(), options(false), Instant::now());
        if drive(&mut player, Duration::from_secs(10), |p| {
            p.current_frame().is_some()
        })
        .is_none()
        {
            continue;
        }
        let duration = player.duration().unwrap_or(Ticks(2_000_000));
        player.seek(Ticks(duration.0 / 2), SeekMode::Exact);
        drive(&mut player, Duration::from_secs(10), |p| !p.is_seeking());
        let mut samples = Vec::new();
        for _ in 0..20 {
            let before = player.current_frame().map(|f| f.pts);
            player.step_backward(Instant::now());
            if let Some(took) = drive(&mut player, Duration::from_secs(10), |p| {
                !p.is_seeking() && p.current_frame().map(|f| f.pts) != before
            }) {
                samples.push(millis(took));
            }
        }
        summary(&path.display().to_string(), &mut samples);
    }
}

#[test]
#[ignore]
fn thumbnail_generation() {
    println!("thumbnail extraction (ms each)");
    for path in files() {
        let Ok(info) = media::probe(&path) else {
            continue;
        };
        let Some(duration) = info.duration else {
            continue;
        };
        let Ok(mut extractor) = ThumbnailExtractor::open(&path, 192) else {
            continue;
        };
        let mut samples = Vec::new();
        for step in 0..20 {
            let at = Ticks(duration.0 * step / 20);
            let start = Instant::now();
            if let Ok(Some(_)) = extractor.extract(at) {
                samples.push(millis(start.elapsed()));
            }
        }
        summary(&path.display().to_string(), &mut samples);
    }
}

#[test]
#[ignore]
fn export_speed() {
    let segments: i64 = std::env::var("PLAY_BENCH_SEGMENTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let reencode = std::env::var("PLAY_BENCH_AUDIO_REENCODE").is_ok_and(|v| v == "1");
    let audio = if reencode {
        AudioChanges {
            gain: media::Decibels(30),
            ..AudioChanges::default()
        }
    } else {
        AudioChanges::default()
    };
    println!("export of {segments} trims per file (ms each; audio re-encode: {reencode})");
    let out_dir = std::env::temp_dir().join("vibeplayer-export-bench");
    let _ = std::fs::create_dir_all(&out_dir);
    for path in files() {
        let Ok(mut demuxer) = Demuxer::open(&path) else {
            continue;
        };
        let info = demuxer.info();
        let Some(stream) = demuxer.best_stream(StreamKind::Video) else {
            continue;
        };
        let (Some(timebase), Some(duration)) = (demuxer.timebase(stream), info.duration) else {
            continue;
        };
        let mut samples = Vec::new();
        for segment in 0..segments {
            let at = |t: i64| (Ticks(t) + info.start_time).rescale(Timebase::MICROS, timebase);
            let request = TrimRequest {
                stream,
                timebase,
                start: at(duration.0 * segment / (segments + 1)),
                end: at(duration.0 * (segment + 1) / (segments + 1)),
            };
            let Ok(keyframe) = demuxer.keyframe_at_or_before(stream, request.start) else {
                continue;
            };
            let Ok(plan) = request.plan(keyframe) else {
                continue;
            };
            let extension = path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default();
            let output = out_dir.join(format!("segment-{segment}.{extension}"));
            let _ = std::fs::remove_file(&output);
            let start = Instant::now();
            let request = ExportRequest {
                source: path.clone(),
                output: output.clone(),
                plan,
                audio,
            };
            match export::run(&request, &AtomicBool::new(false), &|_| {}) {
                Ok(_) => samples.push(millis(start.elapsed())),
                Err(error) => println!("  {}: segment {segment} failed: {error}", path.display()),
            }
            let _ = std::fs::remove_file(&output);
        }
        summary(&path.display().to_string(), &mut samples);
    }
}
