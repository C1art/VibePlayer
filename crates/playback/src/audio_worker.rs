use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use audio::{AudioControl, AudioOutput, Volume};
use crossbeam::channel::{Receiver, Sender};
use cut::{SampleRate, StreamIndex, Ticks, Timebase};
use media::{AudioDecoder, AudioEdits, AudioEffects, AudioTarget, Packet, StreamParams};

use crate::messages::{Event, PacketMessage, Shared};
use crate::mixer::Mixer;
use crate::{AnchorPair, AudioAnchor, Generation};

/// How far ahead of the device we decode. Short, so EQ and speed changes are heard quickly.
const QUEUE_AHEAD_MICROS: i64 = 200_000;
/// How often to try reopening a lost output device.
const DEVICE_RETRY: Duration = Duration::from_secs(1);
/// Audio starting later than this after its seek target gets silence in front of it.
const FILL_AFTER: Ticks = Ticks(50_000);
/// Longer silences than this aren't queued; the clock goes by the system then.
const FILL_AT_MOST: Ticks = Ticks(30_000_000);
/// A timestamp jump bigger than this starts a new anchor. Small, so a few dropped packets
/// don't add up to drift; well above the rounding of container timestamps.
const DISCONTINUITY_MICROS: i64 = 10_000;
/// How long the mix waits for a track that falls behind (or hasn't started) before playing
/// the others without it.
const MIX_LAG_LIMIT: Ticks = Ticks(500_000);

pub(crate) struct Setup {
    /// The audio streams to play, mixed together.
    pub params: Vec<(StreamIndex, StreamParams)>,
    pub volume: Volume,
    pub start_time: Ticks,
    pub packets: Receiver<PacketMessage>,
    pub events: Sender<Event>,
    pub shared: Arc<Shared>,
}

pub(crate) fn spawn(setup: Setup) -> bool {
    thread::Builder::new()
        .name("audio".into())
        .spawn(move || run(setup))
        .is_ok()
}

struct Anchored {
    anchor: AudioAnchor,
    pushed_frames: u64,
    expected_pts: Ticks,
}

/// One audio stream being decoded into the mix.
struct Track {
    stream: StreamIndex,
    params: StreamParams,
    decoder: Option<AudioDecoder>,
}

struct Worker {
    target: AudioTarget,
    tracks: Vec<Track>,
    mixer: Mixer,
    edits: AudioEdits,
    edits_version: u64,
    sink: Sink,
    events: Sender<Event>,
    last_device_retry: Option<Instant>,
}

/// Everything on the device side of the mixer.
struct Sink {
    output: AudioOutput,
    control: AudioControl,
    start_time: Ticks,
    shared: Arc<Shared>,
    generation: Generation,
    exact_target: Option<Ticks>,
    /// Where audio should begin after opening or a seek; silence fills up to a later first
    /// sample, so audio that starts late doesn't play early.
    fill_from: Option<Ticks>,
    anchored: Option<Anchored>,
    /// The anchor before the current one, for samples still queued when it was replaced.
    previous_anchor: Option<AudioAnchor>,
}

fn run(setup: Setup) {
    let output = match AudioOutput::open_default(setup.volume) {
        Ok(output) => output,
        Err(error) => {
            let _ = setup.events.send(Event::AudioFailed(error.into()));
            // Keep draining so the reader never blocks on us.
            for _ in setup.packets {}
            return;
        }
    };
    let control = output.control();
    let format = output.format();
    let _ = setup.events.send(Event::AudioReady(control.clone()));
    let mut worker = Worker {
        target: AudioTarget::playback(format.sample_rate.0, format.channels),
        tracks: Vec::new(),
        mixer: Mixer::new(format.channels, format.sample_rate, 0),
        edits: AudioEdits::default(),
        edits_version: 0,
        events: setup.events.clone(),
        last_device_retry: None,
        sink: Sink {
            control,
            output,
            start_time: setup.start_time,
            shared: setup.shared,
            generation: Generation::default(),
            exact_target: None,
            fill_from: Some(Ticks::ZERO),
            anchored: None,
            previous_anchor: None,
        },
    };
    worker.refresh_edits();
    worker.set_streams(setup.params);
    for message in setup.packets {
        if worker.sink.shared.should_quit() {
            return;
        }
        match message {
            PacketMessage::Packet(generation, packet) => {
                let sink = &worker.sink;
                if generation != sink.generation || sink.shared.is_stale(generation) {
                    continue;
                }
                worker.decode(Some(&packet));
            }
            PacketMessage::Flush {
                generation,
                target,
                exact_target,
            } => worker.flush(generation, target, exact_target),
            PacketMessage::End(generation) => {
                if generation == worker.sink.generation {
                    worker.decode(None);
                    let _ = setup.events.send(Event::AudioEnded(generation));
                }
            }
            PacketMessage::SetStreams(params) => worker.set_streams(params),
        }
    }
}

impl Worker {
    fn set_streams(&mut self, params: Vec<(StreamIndex, StreamParams)>) {
        self.tracks = params
            .into_iter()
            .map(|(stream, params)| Track {
                stream,
                params,
                decoder: None,
            })
            .collect();
        self.rebuild_decoders();
    }

    fn rebuild_decoders(&mut self) {
        let speed = self.sink.shared.speed.load();
        for track in &mut self.tracks {
            let edit = self.edits.get(track.stream);
            let effects = AudioEffects {
                speed,
                gain: edit.gain,
                equalizer: edit.equalizer,
                ..AudioEffects::default()
            };
            track.decoder =
                track.params.try_clone().ok().and_then(|params| {
                    AudioDecoder::new(params, self.target.clone(), effects).ok()
                });
        }
        self.mixer.set_track_count(self.tracks.len());
        self.apply_weights();
        self.sink.anchored = None;
        self.sink.previous_anchor = None;
        // Without a decoder no anchor will come; the player must not wait for one.
        let available = self.tracks.iter().any(|track| track.decoder.is_some());
        let _ = self.events.send(Event::AudioDecoder { available });
    }

    /// Picks up new editor edits: removed tracks stay in the mix silently, to keep time.
    fn refresh_edits(&mut self) {
        if let Some(edits) = self.sink.shared.audio_edits_since(&mut self.edits_version) {
            self.edits = edits;
            self.apply_weights();
        }
    }

    fn apply_weights(&mut self) {
        for (slot, track) in self.tracks.iter().enumerate() {
            let removed = self.edits.get(track.stream).is_removed();
            self.mixer.set_weight(slot, if removed { 0.0 } else { 1.0 });
        }
    }

    fn flush(&mut self, generation: Generation, target: Ticks, exact_target: Option<Ticks>) {
        let sink = &mut self.sink;
        sink.generation = generation;
        sink.exact_target = exact_target;
        sink.fill_from = Some(target);
        sink.control.clear();
        sink.anchored = None;
        sink.previous_anchor = None;
        sink.shared.audio_anchor.store(None);
        for decoder in self.tracks.iter_mut().filter_map(|t| t.decoder.as_mut()) {
            decoder.flush();
        }
        self.mixer.reset();
    }

    /// Reopens the default device after the current one disappeared (e.g. headphones
    /// unplugged). Until then audio is dropped and the player follows the system clock.
    fn recover_device(&mut self) {
        let retry_due = self
            .last_device_retry
            .is_none_or(|last| last.elapsed() >= DEVICE_RETRY);
        if !self.sink.control.is_device_lost() || !retry_due {
            return;
        }
        self.last_device_retry = Some(Instant::now());
        let Ok(output) = AudioOutput::open_default(Volume::default()) else {
            return;
        };
        let control = output.control();
        let format = output.format();
        let _ = self.events.send(Event::AudioReady(control.clone()));
        self.sink.output = output;
        self.sink.control = control;
        self.sink.shared.audio_anchor.store(None);
        self.target = AudioTarget::playback(format.sample_rate.0, format.channels);
        self.mixer = Mixer::new(format.channels, format.sample_rate, self.tracks.len());
        self.rebuild_decoders();
    }

    /// Decodes a packet of one track, or the end of every track, and plays whatever the mix
    /// has ready.
    fn decode(&mut self, packet: Option<&Packet>) {
        self.recover_device();
        self.refresh_edits();
        let speed = self.sink.shared.speed.load();
        match packet {
            Some(packet) => {
                let stream = packet.stream_index();
                if let Some(slot) = self.tracks.iter().position(|t| t.stream == stream) {
                    self.decode_track(slot, Some(packet));
                }
            }
            None => {
                for slot in 0..self.tracks.len() {
                    self.decode_track(slot, None);
                }
                self.mixer.finish();
            }
        }
        while let Some(mixed) = self.mixer.pop(MIX_LAG_LIMIT) {
            if !self.sink.play(speed, mixed.pts, &mixed.samples) {
                return;
            }
        }
    }

    fn decode_track(&mut self, slot: usize, packet: Option<&Packet>) {
        let speed = self.sink.shared.speed.load();
        let Some(track) = self.tracks.get_mut(slot) else {
            return;
        };
        let edit = self.edits.get(track.stream);
        let Some(decoder) = &mut track.decoder else {
            return;
        };
        decoder.set_effects(AudioEffects {
            speed,
            gain: edit.gain,
            equalizer: edit.equalizer,
            ..AudioEffects::default()
        });
        loop {
            let accepted = decoder.send(packet).unwrap_or(true);
            while let Ok(Some(frame)) = decoder.receive() {
                if let Some(pts) = frame.pts {
                    let pts = pts - self.sink.start_time;
                    self.mixer.push(slot, pts, frame.samples(), speed);
                }
            }
            if accepted || self.sink.shared.is_stale(self.sink.generation) {
                return;
            }
        }
    }
}

impl Sink {
    /// Queues mixed samples that start at media time `pts`. Returns `false` if a seek made
    /// the work stale.
    fn play(&mut self, speed: media::Speed, pts: Ticks, samples: &[f32]) -> bool {
        let channels = usize::from(self.control.format().channels).max(1);
        let rate = self.control.format().sample_rate;
        let mut pts = pts;
        let mut samples = samples;
        if let Some(target) = self.exact_target {
            let frames = samples.len() / channels;
            let skip = frames_for(rate, speed.media_to_wall(target - pts)).min(frames as u64);
            if skip as usize >= frames {
                return true;
            }
            samples = &samples[skip as usize * channels..];
            pts = pts.max(target);
            self.exact_target = None;
        }
        let frames = (samples.len() / channels) as u64;
        if self.control.is_device_lost() {
            // Keep to real time while there's no device, so the file isn't raced through.
            self.wait_out(self.control.frames_to_micros(frames));
            return true;
        }
        if let Some(from) = self.fill_from.take() {
            let late = pts - from;
            if late > FILL_AFTER && late < FILL_AT_MOST {
                let silence =
                    vec![0.0; frames_for(rate, speed.media_to_wall(late)) as usize * channels];
                if !self.queue(&silence, from, speed) {
                    return false;
                }
            }
        }
        self.queue(samples, pts, speed)
    }

    /// Queues samples that start at `pts`, anchoring them if needed. Returns `false` if a seek
    /// made the work stale.
    fn queue(&mut self, samples: &[f32], pts: Ticks, speed: media::Speed) -> bool {
        let channels = usize::from(self.control.format().channels).max(1);
        let frames = (samples.len() / channels) as u64;
        self.anchor_if_needed(pts, speed);
        let mut remaining = samples;
        while !remaining.is_empty() {
            if self.shared.is_stale(self.generation) {
                return false;
            }
            if self.control.is_device_lost() {
                return true;
            }
            let ahead = self
                .control
                .frames_to_micros(self.control.queued_frames() as u64);
            if ahead.0 > QUEUE_AHEAD_MICROS {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            let pushed = self.output.push(remaining);
            remaining = &remaining[pushed..];
            if pushed == 0 {
                thread::sleep(Duration::from_millis(5));
            }
        }
        if let Some(current) = &mut self.anchored {
            current.pushed_frames += frames;
            let wall = self.control.frames_to_micros(frames);
            current.expected_pts = pts + speed.wall_to_media(wall);
        }
        true
    }

    /// Publishes a new anchor at the start of playback, after a timestamp jump, or when the
    /// speed changes.
    fn anchor_if_needed(&mut self, pts: Ticks, speed: media::Speed) {
        let needs_anchor = self.anchored.as_ref().is_none_or(|current| {
            (pts - current.expected_pts).0.abs() > DISCONTINUITY_MICROS
                || current.anchor.speed != speed
        });
        if !needs_anchor {
            return;
        }
        let played_frames = match &self.anchored {
            Some(current) => current.anchor.played_frames + current.pushed_frames,
            None => self.control.played_frames() + self.control.queued_frames() as u64,
        };
        let anchor = AudioAnchor {
            generation: self.generation,
            played_frames,
            pts,
            speed,
        };
        self.previous_anchor = self.anchored.as_ref().map(|current| current.anchor);
        self.shared.audio_anchor.store(Some(AnchorPair {
            current: anchor,
            previous: self.previous_anchor,
        }));
        self.anchored = Some(Anchored {
            anchor,
            pushed_frames: 0,
            expected_pts: pts,
        });
    }
}

impl Sink {
    /// Lets `wall` microseconds of playing time pass, as a device would: paused time doesn't
    /// count. Wakes early for a seek.
    fn wait_out(&self, wall: Ticks) {
        let mut left = Duration::from_micros(u64::try_from(wall.0).unwrap_or(0));
        while !left.is_zero() && !self.shared.is_stale(self.generation) {
            let step = left.min(Duration::from_millis(5));
            thread::sleep(step);
            if !self.control.is_paused() {
                left -= step;
            }
        }
    }
}

fn frames_for(rate: SampleRate, wall: Ticks) -> u64 {
    let Some(timebase) = rate.timebase() else {
        return 0;
    };
    u64::try_from(wall.rescale(Timebase::MICROS, timebase).0).unwrap_or(0)
}
