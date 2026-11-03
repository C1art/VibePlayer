use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use audio::{AudioControl, AudioOutput, Volume};
use crossbeam::channel::{Receiver, Sender};
use cut::{SampleRate, Ticks, Timebase};
use media::{AudioDecoder, AudioEffects, AudioFrame, AudioTarget, StreamParams};

use crate::messages::{Event, PacketMessage, Shared};
use crate::{AnchorPair, AudioAnchor, Generation};

/// How far ahead of the device we decode. Short, so EQ and speed changes are heard quickly.
const QUEUE_AHEAD_MICROS: i64 = 200_000;
/// How often to try reopening a lost output device.
const DEVICE_RETRY: Duration = Duration::from_secs(1);
/// A timestamp jump bigger than this starts a new anchor. Small, so a few dropped packets
/// don't add up to drift; well above the rounding of container timestamps.
const DISCONTINUITY_MICROS: i64 = 10_000;

pub(crate) struct Setup {
    pub params: Option<StreamParams>,
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

struct Worker {
    target: AudioTarget,
    params: Option<StreamParams>,
    decoder: Option<AudioDecoder>,
    sink: Sink,
    events: Sender<Event>,
    last_device_retry: Option<Instant>,
}

/// Everything on the device side of the decoder.
struct Sink {
    output: AudioOutput,
    control: AudioControl,
    start_time: Ticks,
    shared: Arc<Shared>,
    generation: Generation,
    exact_target: Option<Ticks>,
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
    let target = AudioTarget::playback(format.sample_rate.0, format.channels);
    let mut worker = Worker {
        decoder: None,
        params: None,
        events: setup.events.clone(),
        last_device_retry: None,
        target,
        sink: Sink {
            control,
            output,
            start_time: setup.start_time,
            shared: setup.shared,
            generation: Generation::default(),
            exact_target: None,
            anchored: None,
            previous_anchor: None,
        },
    };
    worker.set_stream(setup.params);
    for message in setup.packets {
        let sink = &mut worker.sink;
        if sink.shared.should_quit() {
            return;
        }
        match message {
            PacketMessage::Packet(generation, packet) => {
                if generation != sink.generation || sink.shared.is_stale(generation) {
                    continue;
                }
                worker.decode(Some(&packet));
            }
            PacketMessage::Flush {
                generation,
                exact_target,
            } => {
                sink.generation = generation;
                sink.exact_target = exact_target;
                sink.control.clear();
                sink.anchored = None;
                sink.previous_anchor = None;
                sink.shared.audio_anchor.store(None);
                if let Some(decoder) = &mut worker.decoder {
                    decoder.flush();
                }
            }
            PacketMessage::End(generation) => {
                if generation == sink.generation {
                    worker.decode(None);
                    let _ = setup.events.send(Event::AudioEnded(generation));
                }
            }
            PacketMessage::SetStream(params) => worker.set_stream(params),
        }
    }
}

impl Worker {
    fn set_stream(&mut self, params: Option<StreamParams>) {
        self.params = params;
        self.rebuild_decoder();
    }

    fn rebuild_decoder(&mut self) {
        let effects = self.sink.shared.audio_effects.load();
        self.decoder = self
            .params
            .as_ref()
            .and_then(|p| p.try_clone().ok())
            .and_then(|p| AudioDecoder::new(p, self.target.clone(), effects).ok());
        self.sink.anchored = None;
        self.sink.previous_anchor = None;
        // Without a decoder no anchor will come; the player must not wait for one.
        let available = self.decoder.is_some();
        let _ = self.events.send(Event::AudioDecoder { available });
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
        self.rebuild_decoder();
    }

    fn decode(&mut self, packet: Option<&media::Packet>) {
        self.recover_device();
        let Some(decoder) = &mut self.decoder else {
            return;
        };
        let effects: AudioEffects = self.sink.shared.audio_effects.load();
        decoder.set_effects(effects);
        loop {
            let accepted = decoder.send(packet).unwrap_or(true);
            while let Ok(Some(frame)) = decoder.receive() {
                if !self.sink.play(effects, &frame) {
                    return;
                }
            }
            if accepted || self.sink.shared.is_stale(self.sink.generation) {
                return;
            }
        }
    }
}

impl Sink {
    /// Queues one frame for the device. Returns `false` if a seek made the work stale.
    fn play(&mut self, effects: AudioEffects, frame: &AudioFrame) -> bool {
        let channels = usize::from(self.control.format().channels);
        let rate = self.control.format().sample_rate;
        let speed = effects.speed;
        let Some(mut pts) = frame.pts.map(|pts| pts - self.start_time) else {
            return true;
        };
        let mut samples = frame.samples();
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
    /// Sleeps for `wall` microseconds, waking early for a seek.
    fn wait_out(&self, wall: Ticks) {
        let deadline = Instant::now() + Duration::from_micros(u64::try_from(wall.0).unwrap_or(0));
        while Instant::now() < deadline && !self.shared.is_stale(self.generation) {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

fn frames_for(rate: SampleRate, wall: Ticks) -> u64 {
    let Some(timebase) = rate.timebase() else {
        return 0;
    };
    u64::try_from(wall.rescale(Timebase::MICROS, timebase).0).unwrap_or(0)
}
