//! Real decoding against `tests/fixtures/`. Run with `cargo test -p media -- --ignored`.

use std::path::PathBuf;

use cut::{Ticks, Timebase};
use media::{
    AudioDecoder, AudioEffects, AudioTarget, Demuxer, HardwareDecoding, Packet, Rotation, Speed,
    StreamDetails, StreamKind, SubtitleContent, SubtitleDecoder, ThumbnailExtractor, Transfer,
    VideoDecoder, VideoFrame, VideoOptions,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

const SOFTWARE: VideoOptions = VideoOptions {
    hardware: HardwareDecoding::Off,
    high_bit_depth: true,
    deinterlace: true,
    gpu: None,
};

fn decode_video(name: &str, options: VideoOptions) -> Vec<VideoFrame> {
    let mut demuxer = Demuxer::open(&fixture(name)).unwrap();
    let stream = demuxer.best_stream(StreamKind::Video).unwrap();
    demuxer.select_streams(&[stream]);
    let mut decoder = VideoDecoder::new(demuxer.stream_params(stream).unwrap(), options).unwrap();
    let mut packet = Packet::new().unwrap();
    let mut frames = Vec::new();
    loop {
        let more = demuxer.read(&mut packet).unwrap();
        let input = more.then_some(&packet);
        while !decoder.send(input).unwrap() {
            frames.extend(std::iter::from_fn(|| decoder.receive().unwrap()));
        }
        frames.extend(std::iter::from_fn(|| decoder.receive().unwrap()));
        if !more {
            return frames;
        }
    }
}

#[test]
#[ignore]
fn probe_reports_streams_and_duration() {
    let info = media::probe(&fixture("h264_bframes.mp4")).unwrap();
    let (_, video) = info.video().unwrap();
    assert_eq!((video.width, video.height), (320, 180));
    assert_eq!(video.frame_rate, Some((25, 1)));
    let audio = info.streams_of(StreamKind::Audio).next().unwrap();
    assert!(matches!(&audio.details, StreamDetails::Audio(a) if a.sample_rate == 48_000));
    let seconds = info.duration.unwrap().display_seconds(Timebase::MICROS);
    assert!((3.9..4.2).contains(&seconds), "{seconds}");
}

#[test]
#[ignore]
fn b_frames_decode_in_presentation_order() {
    let frames = decode_video("h264_bframes.mp4", SOFTWARE);
    assert_eq!(frames.len(), 100);
    assert!(frames.windows(2).all(|w| w[0].pts < w[1].pts));
    assert_eq!(frames[0].layout.bits, 8);
    assert!(frames[0].plane(0).unwrap().0.len() >= 320 * 180);
}

#[test]
#[ignore]
fn open_gop_hevc_decodes_every_frame() {
    assert_eq!(decode_video("hevc_open_gop.mkv", SOFTWARE).len(), 75);
}

#[test]
#[ignore]
fn interlaced_video_is_deinterlaced_frame_for_frame() {
    let frames = decode_video("interlaced.ts", SOFTWARE);
    assert!((49..=51).contains(&frames.len()), "{}", frames.len());
}

#[test]
#[ignore]
fn hdr_frames_keep_ten_bits_and_pq_transfer() {
    let frames = decode_video("hdr10.mkv", SOFTWARE);
    assert_eq!(frames[0].layout.bits, 10);
    assert_eq!(frames[0].color.transfer, Transfer::Pq);
    assert_eq!(frames[0].luminance.max_content, Some(1000.0));
    let eight_bit = VideoOptions {
        high_bit_depth: false,
        ..SOFTWARE
    };
    assert_eq!(decode_video("hdr10.mkv", eight_bit)[0].layout.bits, 8);
}

#[test]
#[ignore]
fn variable_frame_rate_timestamps_are_kept() {
    let frames = decode_video("vfr.mkv", SOFTWARE);
    let gaps: Vec<i64> = frames.windows(2).map(|w| (w[1].pts - w[0].pts).0).collect();
    assert!(gaps.iter().any(|&g| g < 50_000) && gaps.iter().any(|&g| g > 100_000));
}

fn decoded_samples(effects: AudioEffects) -> usize {
    let mut demuxer = Demuxer::open(&fixture("h264_bframes.mp4")).unwrap();
    let stream = demuxer.best_stream(StreamKind::Audio).unwrap();
    demuxer.select_streams(&[stream]);
    let params = demuxer.stream_params(stream).unwrap();
    let mut decoder = AudioDecoder::new(params, AudioTarget::playback(44_100, 2), effects).unwrap();
    let mut packet = Packet::new().unwrap();
    let mut samples = 0;
    loop {
        let more = demuxer.read(&mut packet).unwrap();
        decoder.send(more.then_some(&packet)).unwrap();
        while let Some(frame) = decoder.receive().unwrap() {
            assert_eq!(frame.samples().len(), frame.sample_count() * 2);
            samples += frame.sample_count();
        }
        if !more {
            return samples;
        }
    }
}

#[test]
#[ignore]
fn audio_is_resampled_to_the_target_and_tempo_changes_length() {
    let normal = decoded_samples(AudioEffects::default());
    assert!(
        (4 * 44_100 - 2048..4 * 44_100 + 2048).contains(&normal),
        "{normal}"
    );
    let double = decoded_samples(AudioEffects {
        speed: Speed::from_percent(200),
        ..AudioEffects::default()
    });
    assert!(
        (normal / 2 - 2048..normal / 2 + 2048).contains(&double),
        "{double}"
    );
}

#[test]
#[ignore]
fn multiple_audio_tracks_and_languages_are_listed() {
    let info = media::probe(&fixture("multi_audio.mkv")).unwrap();
    let languages: Vec<_> = info
        .streams_of(StreamKind::Audio)
        .map(|s| s.language.clone())
        .collect();
    assert_eq!(languages, [Some("eng".into()), Some("ger".into())]);
    assert_eq!(info.streams_of(StreamKind::Subtitle).count(), 1);
}

#[test]
#[ignore]
fn embedded_text_subtitles_decode_to_plain_text() {
    let mut demuxer = Demuxer::open(&fixture("multi_audio.mkv")).unwrap();
    let stream = demuxer.best_stream(StreamKind::Subtitle).unwrap();
    demuxer.select_streams(&[stream]);
    let mut decoder =
        SubtitleDecoder::new(demuxer.stream_params(stream).unwrap(), (320, 180)).unwrap();
    let mut packet = Packet::new().unwrap();
    let mut texts = Vec::new();
    while demuxer.read(&mut packet).unwrap() {
        if let Some(subtitle) = decoder.decode(&packet).unwrap() {
            texts.push((subtitle.start, subtitle.content));
        }
    }
    assert_eq!(texts[0].0, Ticks(500_000));
    assert_eq!(texts[0].1, SubtitleContent::Text("First line".into()));
    assert_eq!(texts[1].1, SubtitleContent::Text("Second line".into()));
}

#[test]
#[ignore]
fn seeking_lands_on_the_keyframe_before_the_target() {
    let mut demuxer = Demuxer::open(&fixture("h264_bframes.mp4")).unwrap();
    let stream = demuxer.best_stream(StreamKind::Video).unwrap();
    demuxer.select_streams(&[stream]);
    demuxer.seek(Ticks(2_500_000)).unwrap();
    let mut packet = Packet::new().unwrap();
    assert!(demuxer.read(&mut packet).unwrap());
    assert!(packet.is_keyframe());
    let timebase = demuxer.timebase(stream).unwrap();
    let pts = packet.pts().unwrap().rescale(timebase, Timebase::MICROS);
    assert_eq!(pts, Ticks(2_000_000));
}

#[test]
#[ignore]
fn thumbnails_are_scaled_to_the_requested_width() {
    let mut extractor = ThumbnailExtractor::open(&fixture("h264_bframes.mp4"), 160).unwrap();
    let image = extractor.extract(Ticks(1_000_000)).unwrap().unwrap();
    assert_eq!((image.width, image.height), (160, 90));
    assert_eq!(image.pixels.len(), 160 * 90 * 4);
}

#[test]
#[ignore]
fn sideways_videos_report_their_turn_and_get_upright_thumbnails() {
    let demuxer = Demuxer::open(&fixture("rotated.mp4")).unwrap();
    let info = demuxer.info();
    assert_eq!(info.video().unwrap().1.rotation, Rotation::Clockwise90);
    let mut extractor = ThumbnailExtractor::open(&fixture("rotated.mp4"), 160).unwrap();
    let image = extractor.extract(Ticks(1_000_000)).unwrap().unwrap();
    assert_eq!((image.width, image.height), (90, 160));
}

#[test]
#[ignore]
fn truncated_files_fail_or_decode_partially_without_panicking() {
    if let Ok(mut demuxer) = Demuxer::open(&fixture("truncated.mp4")) {
        let mut packet = Packet::new().unwrap();
        while let Ok(true) = demuxer.read(&mut packet) {}
    }
}

#[test]
#[ignore]
fn keyframe_lookup_finds_the_gop_start_before_a_target() {
    let mut demuxer = Demuxer::open(&fixture("h264_bframes.mp4")).unwrap();
    let stream = demuxer.best_stream(StreamKind::Video).unwrap();
    let timebase = demuxer.timebase(stream).unwrap();
    let at = |seconds: f64| Ticks::from_display_seconds(seconds, timebase);
    let keyframe = demuxer.keyframe_at_or_before(stream, at(2.7)).unwrap();
    assert_eq!(keyframe, Some(at(2.0)));
    assert_eq!(
        demuxer.keyframe_at_or_before(stream, at(1.0)).unwrap(),
        Some(at(1.0))
    );
}

#[test]
#[ignore]
fn keyframe_span_counts_frames_until_the_next_keyframe() {
    let mut demuxer = Demuxer::open(&fixture("h264_bframes.mp4")).unwrap();
    let stream = demuxer.best_stream(StreamKind::Video).unwrap();
    let timebase = demuxer.timebase(stream).unwrap();
    let at = |seconds: f64| Ticks::from_display_seconds(seconds, timebase);
    let span = demuxer.keyframes_around(stream, at(2.6)).unwrap();
    assert_eq!(span.before, Some(at(2.0)));
    assert_eq!(span.after, Some(at(3.0)));
    // 2.60, 2.64, … 2.96: ten frames at 25 fps.
    assert_eq!(span.frames_until_after(), 10);
}

#[test]
#[ignore]
fn a_full_scan_finds_every_keyframe() {
    let mut demuxer = Demuxer::open(&fixture("h264_bframes.mp4")).unwrap();
    let stream = demuxer.best_stream(StreamKind::Video).unwrap();
    let mut keyframes = Vec::new();
    demuxer
        .scan_all_keyframes(stream, |batch| {
            keyframes.extend(batch);
            true
        })
        .unwrap();
    let timebase = demuxer.timebase(stream).unwrap();
    let seconds: Vec<f64> = keyframes
        .iter()
        .map(|k| k.display_seconds(timebase))
        .collect();
    assert_eq!(seconds, [0.0, 1.0, 2.0, 3.0]);
}

/// First and last audio timestamps (seconds) after converting to `rate` at `speed`.
fn audio_time_span(rate: u32, speed: u16) -> (f64, f64) {
    let mut demuxer = Demuxer::open(&fixture("h264_bframes.mp4")).unwrap();
    let stream = demuxer.best_stream(StreamKind::Audio).unwrap();
    demuxer.select_streams(&[stream]);
    let effects = AudioEffects {
        speed: Speed::from_percent(speed),
        ..AudioEffects::default()
    };
    let params = demuxer.stream_params(stream).unwrap();
    let mut decoder = AudioDecoder::new(params, AudioTarget::playback(rate, 2), effects).unwrap();
    let mut packet = Packet::new().unwrap();
    let mut times = Vec::new();
    loop {
        let more = demuxer.read(&mut packet).unwrap();
        decoder.send(more.then_some(&packet)).unwrap();
        while let Some(frame) = decoder.receive().unwrap() {
            times.push(frame.pts.unwrap().display_seconds(Timebase::MICROS));
        }
        if !more {
            return (times[0], *times.last().unwrap());
        }
    }
}

#[test]
#[ignore]
fn audio_timestamps_stay_in_media_time_when_resampled_or_stretched() {
    for (rate, speed) in [
        (48_000, 100),
        (8_000, 100),
        (44_100, 100),
        (44_100, 200),
        (48_000, 50),
    ] {
        let (first, last) = audio_time_span(rate, speed);
        assert!(
            first.abs() < 0.05,
            "{rate} Hz at {speed}%: starts at {first}"
        );
        assert!(
            (3.85..4.05).contains(&last),
            "{rate} Hz at {speed}%: ends at {last}"
        );
    }
}
