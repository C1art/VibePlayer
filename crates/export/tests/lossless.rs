//! Real exports of `tests/fixtures/`. Run with `cargo test -p export -- --ignored`.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use cut::StreamIndex;
use cut::{Ticks, TrimRequest};
use export::{ExportError, ExportRequest};
use media::{AudioEdits, Decibels, Demuxer, Packet, StreamKind, TrackEdit, TrackKeep};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn output(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vibeplayer-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn plan(source: &Path, start_seconds: f64, end_seconds: f64) -> cut::CutPlan {
    let mut demuxer = Demuxer::open(source).unwrap();
    let stream = demuxer.best_stream(StreamKind::Video).unwrap();
    let timebase = demuxer.timebase(stream).unwrap();
    let origin = demuxer.info().start_time;
    let at = |seconds: f64| {
        (Ticks::from_display_seconds(seconds, cut::Timebase::MICROS) + origin)
            .rescale(cut::Timebase::MICROS, timebase)
    };
    let request = TrimRequest {
        stream,
        timebase,
        start: at(start_seconds),
        end: at(end_seconds),
    };
    let keyframe = demuxer
        .keyframe_at_or_before(stream, request.start)
        .unwrap();
    request.plan(keyframe).unwrap()
}

/// The same edit on every audio track of `source`.
fn every_track(source: &Path, edit: TrackEdit) -> AudioEdits {
    let info = media::probe(source).unwrap();
    let mut edits = AudioEdits::default();
    for stream in info.streams_of(StreamKind::Audio) {
        edits.set(stream.index, edit);
    }
    edits
}

fn gain(tenths: i16) -> TrackEdit {
    TrackEdit {
        gain: Decibels(tenths),
        ..TrackEdit::default()
    }
}

fn export(
    source: &Path,
    out: &Path,
    plan: cut::CutPlan,
    audio: AudioEdits,
) -> Result<export::ExportResult, ExportError> {
    let request = ExportRequest {
        source: source.to_path_buf(),
        output: out.to_path_buf(),
        plan,
        audio,
    };
    export::run(&request, &AtomicBool::new(false), &|_| {})
}

/// Packet payloads of the main video stream, in file order.
fn video_payloads(path: &Path, keep: impl Fn(&Packet) -> bool) -> Vec<Vec<u8>> {
    let mut demuxer = Demuxer::open(path).unwrap();
    let stream = demuxer.best_stream(StreamKind::Video).unwrap();
    demuxer.select_streams(&[stream]);
    let mut packet = Packet::new().unwrap();
    let mut payloads = Vec::new();
    while demuxer.read(&mut packet).unwrap() {
        if keep(&packet) {
            payloads.push(packet.data().to_vec());
        }
    }
    payloads
}

#[test]
#[ignore]
fn copied_video_packets_are_byte_identical_to_the_source() {
    let source = fixture("h264_bframes.mp4");
    let out = output("bytes.mp4");
    let plan = plan(&source, 1.3, 3.0);
    let result = export(&source, &out, plan, AudioEdits::default()).unwrap();
    assert!(!result.reencoded_audio);
    let expected = video_payloads(&source, |p| plan.keeps_primary(p.pts().unwrap(), p.dts()));
    let written = video_payloads(&out, |_| true);
    assert!(!written.is_empty());
    assert_eq!(written, expected);
    let seconds = result.duration.display_seconds(cut::Timebase::MICROS);
    assert!((1.9..2.3).contains(&seconds), "{seconds}");
    let info = media::probe(&out).unwrap();
    assert_eq!(info.video().unwrap().0.start_time, Some(Ticks::ZERO));
}

#[test]
#[ignore]
fn transport_streams_with_b_frames_start_on_the_keyframe() {
    // MPEG-TS seeks by decode time, which for a keyframe comes before its presentation time.
    let source = fixture("h264_bframes.ts");
    let out = output("bframes.ts");
    let plan = plan(&source, 1.3, 2.6);
    export(&source, &out, plan, AudioEdits::default()).unwrap();
    let expected = video_payloads(&source, |p| plan.keeps_primary(p.pts().unwrap(), p.dts()));
    assert_eq!(video_payloads(&out, |_| true), expected);
}

#[test]
#[ignore]
fn a_subtitle_running_past_the_end_stops_there() {
    // The fixture's second cue runs from 2.0 s to 3.0 s; the cut ends at 2.5 s.
    let source = fixture("multi_audio.mkv");
    let out = output("subtitle_end.mkv");
    export(
        &source,
        &out,
        plan(&source, 0.5, 2.5),
        AudioEdits::default(),
    )
    .unwrap();
    let mut demuxer = Demuxer::open(&out).unwrap();
    let stream = demuxer.best_stream(StreamKind::Subtitle).unwrap();
    let timebase = demuxer.timebase(stream).unwrap();
    demuxer.select_streams(&[stream]);
    let mut packet = Packet::new().unwrap();
    let mut last_end = Ticks::ZERO;
    while demuxer.read(&mut packet).unwrap() {
        last_end = packet.pts().unwrap() + packet.duration();
    }
    let seconds = last_end.display_seconds(timebase);
    assert!((2.4..=2.55).contains(&seconds), "{seconds}");
}

#[test]
#[ignore]
fn every_stream_survives_the_cut() {
    let source = fixture("multi_audio.mkv");
    let out = output("streams.mkv");
    export(
        &source,
        &out,
        plan(&source, 1.0, 3.5),
        AudioEdits::default(),
    )
    .unwrap();
    let info = media::probe(&out).unwrap();
    assert_eq!(info.streams.len(), 4);
    let languages: Vec<_> = info
        .streams_of(StreamKind::Audio)
        .map(|s| s.language.clone())
        .collect();
    assert_eq!(languages, [Some("eng".into()), Some("ger".into())]);
}

#[test]
#[ignore]
fn cover_art_survives_a_cut_that_starts_later() {
    let source = fixture("cover_art.mp4");
    let out = output("cover_art.mp4");
    export(
        &source,
        &out,
        plan(&source, 1.0, 1.8),
        AudioEdits::default(),
    )
    .unwrap();
    let info = media::probe(&out).unwrap();
    let pictures = info.streams.iter().filter(|stream| {
        matches!(&stream.details, media::StreamDetails::Video(video) if video.attached_picture)
    });
    assert_eq!(pictures.count(), 1);
}

#[test]
#[ignore]
fn volume_changes_reencode_only_the_audio() {
    let source = fixture("h264_bframes.mp4");
    let out = output("louder.mp4");
    let plan = plan(&source, 0.0, 2.0);
    let changes = every_track(&source, gain(60));
    let result = export(&source, &out, plan, changes).unwrap();
    assert!(result.reencoded_audio);
    let expected = video_payloads(&source, |p| plan.keeps_primary(p.pts().unwrap(), p.dts()));
    assert_eq!(video_payloads(&out, |_| true), expected);
    let info = media::probe(&out).unwrap();
    assert_eq!(
        info.streams_of(StreamKind::Audio).next().unwrap().codec,
        "aac"
    );
}

#[test]
#[ignore]
fn removed_audio_leaves_only_the_other_streams_copied() {
    let source = fixture("multi_audio.mkv");
    let out = output("silent.mkv");
    let plan = plan(&source, 0.5, 3.0);
    let removed = TrackEdit {
        keep: TrackKeep::Remove,
        ..gain(60)
    };
    let changes = every_track(&source, removed);
    let result = export(&source, &out, plan, changes).unwrap();
    assert!(!result.reencoded_audio);
    let info = media::probe(&out).unwrap();
    assert_eq!(info.streams_of(StreamKind::Audio).count(), 0);
    let expected = video_payloads(&source, |p| plan.keeps_primary(p.pts().unwrap(), p.dts()));
    assert_eq!(video_payloads(&out, |_| true), expected);
}

#[test]
#[ignore]
fn removing_one_track_keeps_the_others() {
    let source = fixture("multi_audio.mkv");
    let out = output("one_track.mkv");
    let mut changes = AudioEdits::default();
    let removed = TrackEdit {
        keep: TrackKeep::Remove,
        ..TrackEdit::default()
    };
    changes.set(StreamIndex(1), removed);
    let result = export(&source, &out, plan(&source, 0.5, 3.0), changes).unwrap();
    assert!(!result.reencoded_audio);
    let info = media::probe(&out).unwrap();
    let languages: Vec<_> = info
        .streams_of(StreamKind::Audio)
        .map(|s| s.language.clone())
        .collect();
    assert_eq!(languages, [Some("ger".into())]);
}

#[test]
#[ignore]
fn editing_one_track_leaves_the_others_copied() {
    let source = fixture("multi_audio.mkv");
    let out = output("one_edited.mkv");
    let mut changes = AudioEdits::default();
    changes.set(StreamIndex(1), gain(-60));
    let result = export(&source, &out, plan(&source, 0.5, 3.0), changes).unwrap();
    assert!(result.reencoded_audio);
    let source_german = payloads(&source, StreamIndex(2));
    let copied_german = payloads(&out, StreamIndex(2));
    assert!(!copied_german.is_empty());
    assert!(copied_german.iter().all(|p| source_german.contains(p)));
    let source_english = payloads(&source, StreamIndex(1));
    let reencoded_english = payloads(&out, StreamIndex(1));
    assert!(!reencoded_english.iter().all(|p| source_english.contains(p)));
}

/// Packet payloads of one stream, in file order.
fn payloads(path: &Path, stream: StreamIndex) -> Vec<Vec<u8>> {
    let mut demuxer = Demuxer::open(path).unwrap();
    demuxer.select_streams(&[stream]);
    let mut packet = Packet::new().unwrap();
    let mut payloads = Vec::new();
    while demuxer.read(&mut packet).unwrap() {
        payloads.push(packet.data().to_vec());
    }
    payloads
}

#[test]
#[ignore]
fn reencoded_audio_is_cut_to_the_sample() {
    let source = fixture("h264_bframes.mp4");
    let out = output("trimmed_audio.mp4");
    let changes = every_track(&source, gain(-30));
    export(&source, &out, plan(&source, 1.0, 3.0), changes).unwrap();
    let stream = Demuxer::open(&out)
        .unwrap()
        .best_stream(StreamKind::Audio)
        .unwrap();
    let samples = decoded_audio_samples(&out, stream);
    // Two seconds at 48 kHz; AAC can't end mid-frame, so allow the padding of one frame.
    assert!((96_000..96_000 + 1_024).contains(&samples), "{samples}");
}

#[test]
#[ignore]
fn reencoded_audio_in_matroska_keeps_its_length() {
    // Matroska's millisecond timebase differs from the sample rate, unlike MP4's audio.
    let source = fixture("multi_audio.mkv");
    let out = output("trimmed_audio.mkv");
    let changes = every_track(&source, gain(-30));
    export(&source, &out, plan(&source, 0.5, 3.0), changes).unwrap();
    let info = media::probe(&out).unwrap();
    for stream in info.streams_of(StreamKind::Audio) {
        let samples = decoded_audio_samples(&out, stream.index);
        assert!((143_000..146_000).contains(&samples), "{samples}");
    }
}

fn decoded_audio_samples(path: &Path, stream: StreamIndex) -> u64 {
    let mut demuxer = Demuxer::open(path).unwrap();
    demuxer.select_streams(&[stream]);
    let target = media::AudioTarget {
        sample_format: "flt".into(),
        sample_rate: 48_000,
        layout: "mono".into(),
        frame_size: None,
    };
    let params = demuxer.stream_params(stream).unwrap();
    let mut decoder =
        media::AudioDecoder::new(params, target, media::AudioEffects::default()).unwrap();
    let mut packet = Packet::new().unwrap();
    let mut samples = 0;
    loop {
        let more = demuxer.read(&mut packet).unwrap();
        decoder.send(more.then_some(&packet)).unwrap();
        while let Some(frame) = decoder.receive().unwrap() {
            samples += frame.samples().len() as u64;
        }
        if !more {
            return samples;
        }
    }
}

#[test]
#[ignore]
fn interlaced_transport_streams_and_open_gop_hevc_export() {
    for (name, out) in [
        ("interlaced.ts", "cut.ts"),
        ("hevc_open_gop.mkv", "cut.mkv"),
    ] {
        let source = fixture(name);
        let out = output(out);
        export(
            &source,
            &out,
            plan(&source, 0.5, 1.8),
            AudioEdits::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
#[ignore]
fn the_source_is_never_overwritten() {
    let source = fixture("h264_bframes.mp4");
    let result = export(
        &source,
        &source,
        plan(&source, 0.0, 1.0),
        AudioEdits::default(),
    );
    assert!(matches!(result, Err(ExportError::WouldOverwriteSource(_))));
}
