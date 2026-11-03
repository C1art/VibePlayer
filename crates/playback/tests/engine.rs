//! Drives the real engine headless against `tests/fixtures/`. Run with `--ignored`.

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use cut::Ticks;
use media::{HardwareDecoding, Speed, VideoOptions};
use playback::{LoadState, Player, PlayerOptions, SeekMode};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn options(autoplay: bool) -> PlayerOptions {
    PlayerOptions {
        video: VideoOptions {
            hardware: HardwareDecoding::Off,
            high_bit_depth: true,
            deinterlace: true,
        },
        volume: audio::Volume::default(),
        muted: true,
        speed: Speed::NORMAL,
        start_at: None,
        autoplay,
    }
}

/// Updates the player like a UI would until `done` or the timeout.
fn run_until(
    player: &mut Player,
    timeout: Duration,
    mut done: impl FnMut(&Player) -> bool,
) -> usize {
    let start = Instant::now();
    let mut frames_shown = 0;
    while start.elapsed() < timeout {
        if player.update(Instant::now()).frame_changed {
            frames_shown += 1;
        }
        if done(player) {
            return frames_shown;
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("timed out; position {:?}", player.position());
}

/// When the position first moves; output latency (large on some Linux setups) comes before.
fn started_moving(player: &mut Player) -> Instant {
    run_until(player, Duration::from_secs(10), |p| {
        p.position() > Ticks::ZERO
    });
    Instant::now()
}

#[test]
#[ignore]
fn plays_a_file_to_the_end_in_real_time() {
    let mut player = Player::open(fixture("h264_bframes.mp4"), options(true), Instant::now());
    let started = started_moving(&mut player);
    let shown = run_until(&mut player, Duration::from_secs(10), Player::has_ended);
    let elapsed = started.elapsed().as_secs_f64();
    assert!((3.5..4.5).contains(&elapsed), "took {elapsed}s");
    assert!(shown > 80, "only {shown} frames shown");
    assert!(player.position() >= Ticks(3_900_000));
}

#[test]
#[ignore]
fn exact_seek_shows_the_target_frame_while_paused() {
    let mut player = Player::open(fixture("h264_bframes.mp4"), options(false), Instant::now());
    run_until(&mut player, Duration::from_secs(5), |p| {
        p.current_frame().is_some()
    });
    player.seek(Ticks(2_480_000), SeekMode::Exact);
    run_until(&mut player, Duration::from_secs(5), |p| {
        !p.is_seeking() && p.current_frame().is_some_and(|f| f.pts > Ticks(2_000_000))
    });
    assert_eq!(player.current_frame().unwrap().pts, Ticks(2_480_000));
}

#[test]
#[ignore]
fn stepping_moves_one_frame_each_way() {
    let mut player = Player::open(fixture("h264_bframes.mp4"), options(false), Instant::now());
    run_until(&mut player, Duration::from_secs(5), |p| {
        p.current_frame().is_some()
    });
    player.seek(Ticks(1_000_000), SeekMode::Exact);
    run_until(&mut player, Duration::from_secs(5), |p| {
        !p.is_seeking() && p.current_frame().is_some_and(|f| f.pts == Ticks(1_000_000))
    });
    run_until(&mut player, Duration::from_secs(5), |p| {
        p.current_frame().is_some()
    });
    thread::sleep(Duration::from_millis(100));
    player.update(Instant::now());
    player.step_forward(Instant::now());
    assert_eq!(player.current_frame().unwrap().pts, Ticks(1_040_000));
    player.step_backward(Instant::now());
    run_until(&mut player, Duration::from_secs(5), |p| {
        p.current_frame().is_some_and(|f| f.pts == Ticks(1_000_000))
    });
}

#[test]
#[ignore]
fn seeks_during_playback_never_show_stale_frames() {
    let mut player = Player::open(fixture("h264_bframes.mp4"), options(true), Instant::now());
    run_until(&mut player, Duration::from_secs(5), |p| {
        p.current_frame().is_some()
    });
    for target in [3_000_000, 500_000, 2_000_000, 100_000, 3_500_000] {
        player.seek(Ticks(target), SeekMode::Exact);
        run_until(&mut player, Duration::from_secs(5), |p| !p.is_seeking());
        let pts = player.current_frame().unwrap().pts.0;
        assert!(
            (target - 40_000..=target + 200_000).contains(&pts),
            "target {target}, shown {pts}"
        );
    }
}

#[test]
#[ignore]
fn corrupt_files_fail_cleanly() {
    let mut player = Player::open(fixture("truncated.mp4"), options(true), Instant::now());
    run_until(&mut player, Duration::from_secs(10), |p| {
        matches!(p.state(), LoadState::Failed(_)) || p.has_ended()
    });
}

#[test]
#[ignore]
fn double_speed_finishes_in_half_the_time() {
    let mut fast = options(true);
    fast.speed = Speed::from_percent(200);
    let mut player = Player::open(fixture("h264_bframes.mp4"), fast, Instant::now());
    let started = started_moving(&mut player);
    run_until(&mut player, Duration::from_secs(10), Player::has_ended);
    let elapsed = started.elapsed().as_secs_f64();
    assert!((1.6..2.4).contains(&elapsed), "took {elapsed}s");
}

#[test]
#[ignore]
fn stepping_back_from_a_keyframe_reaches_the_previous_gop() {
    let mut player = Player::open(fixture("h264_bframes.mp4"), options(false), Instant::now());
    run_until(&mut player, Duration::from_secs(5), |p| {
        p.current_frame().is_some()
    });
    player.seek(Ticks(2_000_000), SeekMode::Exact);
    run_until(&mut player, Duration::from_secs(5), |p| {
        !p.is_seeking() && p.current_frame().is_some_and(|f| f.pts == Ticks(2_000_000))
    });
    player.step_backward(Instant::now());
    run_until(&mut player, Duration::from_secs(5), |p| {
        p.current_frame().is_some_and(|f| f.pts == Ticks(1_960_000))
    });
}
