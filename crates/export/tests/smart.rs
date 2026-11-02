//! Smart cuts of `tests/fixtures/`, decoded back and compared with the source frame by frame.
//! Needs FFmpeg with libx264 and libx265. Run with `cargo test -p export -- --ignored`.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use cut::{Ticks, Timebase, TrimRequest};
use export::{AudioChanges, ExportRequest, ExportResult};
use media::{Demuxer, HardwareDecoding, Packet, StreamKind, VideoDecoder, VideoOptions};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn output(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vibeplayer-smart-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// Decoded frames as (seconds from the file start, luma plane) in display order.
fn frames(path: &Path) -> (Vec<(f64, Vec<u8>)>, u64) {
    let mut demuxer = Demuxer::open(path).unwrap();
    let origin = demuxer.info().start_time;
    let stream = demuxer.best_stream(StreamKind::Video).unwrap();
    demuxer.select_streams(&[stream]);
    let options = VideoOptions {
        hardware: HardwareDecoding::Off,
        high_bit_depth: true,
    };
    let mut decoder = VideoDecoder::new(demuxer.stream_params(stream).unwrap(), options).unwrap();
    let mut packet = Packet::new().unwrap();
    let mut decoded = Vec::new();
    let mut take = |decoder: &mut VideoDecoder| {
        while let Some(frame) = decoder.receive().unwrap() {
            let (bytes, stride) = frame.plane(0).unwrap();
            let width = frame.width() as usize * frame.layout.bytes_per_sample();
            let luma = bytes
                .chunks(stride)
                .take(frame.height() as usize)
                .flat_map(|row| &row[..width])
                .copied()
                .collect();
            decoded.push(((frame.pts - origin).display_seconds(Timebase::MICROS), luma));
        }
    };
    loop {
        let more = demuxer.read(&mut packet).unwrap();
        while !decoder.send(more.then_some(&packet)).unwrap() {
            take(&mut decoder);
        }
        take(&mut decoder);
        if !more {
            return (decoded, decoder.skipped());
        }
    }
}

fn smart_cut(name: &str, start: f64, end: f64, out: &str) -> (PathBuf, ExportResult) {
    let source = fixture(name);
    let mut demuxer = Demuxer::open(&source).unwrap();
    let stream = demuxer.best_stream(StreamKind::Video).unwrap();
    let timebase = demuxer.timebase(stream).unwrap();
    let origin = demuxer.info().start_time;
    let at = |seconds: f64| {
        (Ticks::from_display_seconds(seconds, Timebase::MICROS) + origin)
            .rescale(Timebase::MICROS, timebase)
    };
    let request = TrimRequest {
        stream,
        timebase,
        start: at(start),
        end: at(end),
    };
    let span = demuxer.keyframes_around(stream, request.start).unwrap();
    let plan = request
        .plan_smart(span.before, span.after, span.frames_until_after)
        .unwrap();
    assert!(plan.reencode.is_some(), "{name}: start is a keyframe");
    let output = output(out);
    let request = ExportRequest {
        source,
        output: output.clone(),
        plan,
        audio: AudioChanges::default(),
    };
    let result = export::run(&request, &AtomicBool::new(false), &|_| {}).unwrap();
    (output, result)
}

fn mean_difference(a: &[u8], b: &[u8]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| f64::from(x.abs_diff(*y)))
        .sum::<f64>()
        / a.len() as f64
}

/// The cut starts exactly at `start`, every frame decodes, re-encoded frames look like the
/// source, and copied frames are identical to it.
fn check(name: &str, start: f64, end: f64, next_keyframe: f64, frame_seconds: f64) {
    let extension = name.rsplit('.').next().unwrap();
    let (output, result) = smart_cut(name, start, end, &format!("{name}.cut.{extension}"));
    let expected_head = ((next_keyframe - start) / frame_seconds).round() as usize;
    assert_eq!(
        result.reencoded_frames, expected_head,
        "{name}: frames re-encoded"
    );
    let (source, _) = frames(&fixture(name));
    let (mut cut, skipped) = frames(&output);
    assert_eq!(skipped, 0, "{name}: undecodable output frames");
    let wanted: Vec<_> = source
        .iter()
        .filter(|(t, _)| *t >= start - 1e-6 && *t < end - 1e-6)
        .collect();
    // Frames shown after the end but needed as references by frames before it stay in.
    let trailing = cut.len().saturating_sub(wanted.len());
    assert!(trailing <= 3, "{name}: {trailing} frames past the end");
    cut.truncate(wanted.len());
    assert_eq!(cut.len(), wanted.len(), "{name}: frame count");
    // Audio encoder priming can start the file a few milliseconds before the first frame.
    assert!(cut[0].0.abs() < 0.03, "{name}: first frame at {}", cut[0].0);
    for (index, ((_, written), (time, original))) in cut.iter().zip(&wanted).enumerate() {
        if *time < next_keyframe - 1e-6 {
            let difference = mean_difference(written, original);
            assert!(
                difference < 3.0,
                "{name}: re-encoded frame {index} differs by {difference}"
            );
        } else {
            assert!(
                written == original,
                "{name}: copied frame {index} at {time}s changed"
            );
        }
    }
}

#[test]
#[ignore]
fn smart_cut_h264_in_mp4_starts_on_the_exact_frame() {
    check("h264_bframes.mp4", 1.32, 3.5, 2.0, 0.04);
}

#[test]
#[ignore]
fn smart_cut_h264_in_matroska_starts_on_the_exact_frame() {
    // Its only keyframe is the first frame, so everything up to the end is re-encoded.
    check("multi_audio.mkv", 0.6, 3.0, 3.0, 0.04);
}

#[test]
#[ignore]
fn smart_cut_open_gop_hevc_replaces_the_leading_frames() {
    check("hevc_open_gop.mkv", 0.52, 2.8, 1.2, 0.04);
}

#[test]
#[ignore]
fn smart_cut_within_one_gop_re_encodes_the_whole_clip() {
    let (output, result) = smart_cut("h264_bframes.mp4", 1.2, 1.6, "short.mp4");
    assert_eq!(result.reencoded_frames, 10);
    let (cut, skipped) = frames(&output);
    assert_eq!((cut.len(), skipped), (10, 0));
}
