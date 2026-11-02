use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use audio::{AudioControl, Volume};
use crossbeam::channel::{self, Receiver, Sender};
use cut::{StreamIndex, Ticks};
use media::{
    AudioEffects, Decibels, Equalizer, Speed, Subtitle, SubtitleContent, VideoFrame, VideoOptions,
};

use crate::messages::{Command, DecodedVideo, Event, Opened, Shared};
use crate::reader::{self, ReaderSetup};
use crate::{AudioReading, Clock, FrameQueue, Generation, PlaybackError, Shown, Timed};

/// Decoded frames the video thread may get ahead of the screen.
const FRAMES_AHEAD: usize = 4;
const MAX_SUBTITLES: usize = 256;

impl Timed for VideoFrame {
    fn pts(&self) -> Ticks {
        self.pts
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeekMode {
    /// Decode from the prior keyframe and show the exact target frame.
    Exact,
    /// Show the prior keyframe; fast, for scrubbing.
    Keyframe,
}

#[derive(Clone, Copy, Debug)]
pub struct PlayerOptions {
    pub video: VideoOptions,
    pub volume: Volume,
    pub muted: bool,
    pub speed: Speed,
    /// Where to start, in microseconds; e.g. a resume position.
    pub start_at: Option<Ticks>,
    pub autoplay: bool,
}

#[derive(Debug)]
pub enum LoadState {
    Loading,
    Ready(Box<Opened>),
    Failed(PlaybackError),
}

/// What the UI should do after [`Player::update`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Update {
    pub frame_changed: bool,
    /// When the next update is due, if playback is moving.
    pub repaint_after: Option<Duration>,
}

/// One open file. Lives on the UI thread; never blocks it.
pub struct Player {
    path: PathBuf,
    shared: Arc<Shared>,
    commands: Sender<Command>,
    events: Receiver<Event>,
    frames: Receiver<DecodedVideo>,
    state: LoadState,
    queue: FrameQueue<VideoFrame>,
    clock: Clock,
    generation: Generation,
    playing: bool,
    waiting_for_media: bool,
    ended: bool,
    video_ended: bool,
    audio_ended: bool,
    audio: Option<AudioControl>,
    audio_error: Option<PlaybackError>,
    hardware_decoding: Option<bool>,
    volume: Volume,
    muted: bool,
    effects: AudioEffects,
    subtitles: Vec<Subtitle>,
    pending_start: Option<Ticks>,
    dropped_frames: u64,
}

impl Player {
    /// Starts opening `path` in the background; check [`Player::state`] for the result.
    pub fn open(path: PathBuf, options: PlayerOptions, now: Instant) -> Player {
        let effects = AudioEffects {
            speed: options.speed,
            ..AudioEffects::default()
        };
        let shared = Arc::new(Shared::new(effects));
        let (command_sender, command_receiver) = channel::unbounded();
        let (event_sender, event_receiver) = channel::unbounded();
        let (frame_sender, frame_receiver) = channel::bounded(FRAMES_AHEAD);
        reader::spawn(ReaderSetup {
            path: path.clone(),
            video_options: options.video,
            volume: options.volume,
            shared: Arc::clone(&shared),
            commands: command_receiver,
            events: event_sender,
            frames: frame_sender,
        });
        let mut clock = Clock::new(now);
        clock.set_speed(now, options.speed);
        Player {
            path,
            shared,
            commands: command_sender,
            events: event_receiver,
            frames: frame_receiver,
            state: LoadState::Loading,
            queue: FrameQueue::new(),
            clock,
            generation: Generation::default(),
            playing: options.autoplay,
            waiting_for_media: true,
            ended: false,
            video_ended: false,
            audio_ended: false,
            audio: None,
            audio_error: None,
            hardware_decoding: None,
            volume: options.volume,
            muted: options.muted,
            effects,
            subtitles: Vec::new(),
            pending_start: options.start_at.filter(|t| t.0 > 0),
            dropped_frames: 0,
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn state(&self) -> &LoadState {
        &self.state
    }

    pub fn opened(&self) -> Option<&Opened> {
        match &self.state {
            LoadState::Ready(opened) => Some(opened),
            _ => None,
        }
    }

    /// Duration in microseconds, when known.
    pub fn duration(&self) -> Option<Ticks> {
        self.opened()?.info.duration
    }

    pub fn position(&self) -> Ticks {
        self.clock.position()
    }

    pub fn current_frame(&self) -> Option<&VideoFrame> {
        self.queue.current()
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn has_ended(&self) -> bool {
        self.ended
    }

    /// True between a seek and the first frame or sample of the new position.
    pub fn is_seeking(&self) -> bool {
        self.waiting_for_media
    }

    pub fn speed(&self) -> Speed {
        self.clock.speed()
    }

    pub fn volume(&self) -> Volume {
        self.volume
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn hardware_decoding(&self) -> Option<bool> {
        self.hardware_decoding
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames
    }

    /// Why audio isn't playing, if the output device couldn't be used.
    pub fn audio_error(&self) -> Option<&PlaybackError> {
        self.audio_error.as_ref()
    }

    pub fn equalizer(&self) -> Equalizer {
        self.effects.equalizer
    }

    /// Call once per UI frame: takes in worker results and picks the frame to show.
    pub fn update(&mut self, now: Instant) -> Update {
        self.receive_events(now);
        self.receive_frames();
        let audio = self.audio_reading();
        if self.waiting_for_media && self.media_ready(audio.is_some()) {
            self.waiting_for_media = false;
            if self.playing {
                self.clock.play(now);
            }
        }
        let position = self.clock.tick(now, audio);
        let shown = if self.waiting_for_media || !self.playing {
            self.queue.show_first()
        } else {
            self.queue.advance_to(position)
        };
        if let Shown::NewFrame { dropped } = shown {
            self.dropped_frames += dropped as u64;
        }
        self.detect_end(now);
        Update {
            frame_changed: shown != Shown::Unchanged,
            repaint_after: self.repaint_after(position),
        }
    }

    fn receive_events(&mut self, now: Instant) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Opened(opened) => self.on_opened(opened, now),
                Event::Failed(error) => {
                    if matches!(self.state, LoadState::Loading) {
                        self.state = LoadState::Failed(error);
                    }
                }
                Event::VideoDecoder { hardware } => self.hardware_decoding = Some(hardware),
                Event::AudioReady(control) => {
                    control.set_volume(self.volume, self.muted);
                    control.set_paused(!self.playing);
                    self.audio = Some(control);
                }
                Event::AudioFailed(error) => self.audio_error = Some(error),
                Event::Subtitle(generation, subtitle) => {
                    if generation == self.generation {
                        self.add_subtitle(subtitle);
                    }
                }
                Event::VideoEnded(generation) => self.video_ended |= generation == self.generation,
                Event::AudioEnded(generation) => self.audio_ended |= generation == self.generation,
            }
        }
    }

    fn on_opened(&mut self, opened: Box<Opened>, now: Instant) {
        self.state = LoadState::Ready(opened);
        if let Some(start) = self.pending_start.take() {
            let within = self.duration().is_none_or(|d| start < d - Ticks(5_000_000));
            if within {
                self.seek_at(now, start, SeekMode::Exact);
            }
        }
    }

    fn receive_frames(&mut self) {
        while self.queue.pending() < FRAMES_AHEAD {
            match self.frames.try_recv() {
                Ok(decoded) => {
                    self.queue.push(decoded.generation, decoded.frame);
                }
                Err(_) => break,
            }
        }
    }

    fn has_video(&self) -> bool {
        self.opened().is_some_and(|o| o.video.is_some())
    }

    fn has_audio(&self) -> bool {
        self.opened().is_some_and(|o| o.audio.is_some()) && self.audio_error.is_none()
    }

    fn audio_reading(&self) -> Option<AudioReading> {
        let control = self.audio.as_ref()?;
        if !self.has_audio() || control.is_device_lost() {
            return None;
        }
        if self.audio_ended && control.queued_frames() == 0 {
            return None;
        }
        let anchor = self.shared.audio_anchor.load()?;
        (anchor.generation == self.generation).then(|| AudioReading {
            anchor,
            played_frames: control.played_frames(),
            latency: control.latency(),
            sample_rate: control.format().sample_rate,
        })
    }

    fn media_ready(&self, audio_anchored: bool) -> bool {
        if self.opened().is_none() {
            return false;
        }
        let video_ready = !self.has_video()
            || !self.queue.is_awaiting_first()
            || self.queue.pending() > 0
            || self.video_ended;
        let audio_ready = !self.has_audio()
            || audio_anchored
            || self.audio_ended
            || (self.audio.is_none() && self.audio_error.is_none() && !self.has_video());
        video_ready && (audio_ready || !self.playing)
    }

    fn detect_end(&mut self, now: Instant) {
        if self.ended || self.waiting_for_media || !self.playing || self.opened().is_none() {
            return;
        }
        let video_done = !self.has_video() || (self.video_ended && self.queue.pending() == 0);
        let audio_done = !self.has_audio()
            || (self.audio_ended && self.audio.as_ref().is_none_or(|a| a.queued_frames() == 0));
        if video_done && audio_done {
            self.ended = true;
            self.pause_at(now);
            if let Some(duration) = self.duration() {
                let end = self
                    .current_frame()
                    .map_or(duration, |f| f.pts.max(self.position()));
                self.clock.jump(now, end.min(duration));
            }
        }
    }

    fn repaint_after(&self, position: Ticks) -> Option<Duration> {
        if self.waiting_for_media {
            return Some(Duration::from_millis(5));
        }
        if !self.playing {
            return None;
        }
        let wait = match self.queue.next_pts() {
            Some(next) => self
                .clock
                .speed()
                .media_to_wall(next - position)
                .0
                .clamp(1_000, 20_000),
            None => 5_000,
        };
        Some(Duration::from_micros(wait as u64))
    }

    pub fn play(&mut self, now: Instant) {
        if self.ended {
            self.seek_at(now, Ticks::ZERO, SeekMode::Exact);
        }
        self.playing = true;
        if !self.waiting_for_media {
            self.clock.play(now);
        }
        if let Some(audio) = &self.audio {
            audio.set_paused(false);
        }
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.clock.pause();
        if let Some(audio) = &self.audio {
            audio.set_paused(true);
        }
    }

    fn pause_at(&mut self, _now: Instant) {
        self.pause();
    }

    pub fn toggle_play(&mut self, now: Instant) {
        if self.playing {
            self.pause();
        } else {
            self.play(now);
        }
    }

    /// Seeks to `target` microseconds from the start.
    pub fn seek(&mut self, target: Ticks, mode: SeekMode) {
        self.seek_at(Instant::now(), target, mode);
    }

    fn seek_at(&mut self, now: Instant, target: Ticks, mode: SeekMode) {
        let end = self.duration().unwrap_or(Ticks(i64::MAX));
        let target = target.max(Ticks::ZERO).min(end);
        self.generation = Generation(self.generation.0 + 1);
        self.shared.set_generation(self.generation);
        self.shared.audio_anchor.store(None);
        self.queue.reset(self.generation);
        self.subtitles.clear();
        self.video_ended = false;
        self.audio_ended = false;
        self.ended = false;
        self.waiting_for_media = true;
        self.clock.pause();
        self.clock.jump(now, target);
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        let _ = self.commands.send(Command::Seek {
            generation: self.generation,
            target,
            exact: mode == SeekMode::Exact,
        });
    }

    /// Moves `delta` microseconds from the current position.
    pub fn seek_relative(&mut self, delta: Ticks) {
        self.seek(self.position() + delta, SeekMode::Exact);
    }

    /// Pauses and shows the next frame.
    pub fn step_forward(&mut self, now: Instant) {
        self.pause();
        if let Shown::NewFrame { .. } = self.queue.step()
            && let Some(frame) = self.queue.current()
        {
            let pts = frame.pts;
            self.clock.jump(now, pts);
        }
    }

    /// Pauses and shows the previous frame, decoding again from the prior keyframe.
    pub fn step_backward(&mut self, now: Instant) {
        self.pause();
        let current = self.queue.current().map_or(self.position(), |f| f.pts);
        if current > Ticks::ZERO {
            self.seek_at(now, current - Ticks(1), SeekMode::Exact);
        }
    }

    pub fn set_speed(&mut self, speed: Speed) {
        self.clock.set_speed(Instant::now(), speed);
        self.effects.speed = speed;
        self.shared.audio_effects.store(self.effects);
    }

    /// Live EQ and gain for the editor preview.
    pub fn set_audio_edits(&mut self, gain: Decibels, equalizer: Equalizer) {
        self.effects.gain = gain;
        self.effects.equalizer = equalizer;
        self.shared.audio_effects.store(self.effects);
    }

    pub fn set_volume(&mut self, volume: Volume) {
        self.volume = volume;
        self.apply_volume();
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.apply_volume();
    }

    fn apply_volume(&self) {
        if let Some(audio) = &self.audio {
            audio.set_volume(self.volume, self.muted);
        }
    }

    pub fn select_audio(&mut self, stream: Option<StreamIndex>) {
        if let LoadState::Ready(opened) = &mut self.state {
            opened.audio = stream;
        }
        self.audio_ended = false;
        let _ = self.commands.send(Command::SelectAudio(stream));
        self.seek(self.position(), SeekMode::Exact);
    }

    pub fn select_subtitle(&mut self, stream: Option<StreamIndex>) {
        if let LoadState::Ready(opened) = &mut self.state {
            opened.subtitle = stream;
        }
        let _ = self.commands.send(Command::SelectSubtitle(stream));
        self.seek(self.position(), SeekMode::Exact);
    }

    fn add_subtitle(&mut self, subtitle: Subtitle) {
        let at = self
            .subtitles
            .partition_point(|existing| existing.start <= subtitle.start);
        self.subtitles.insert(at, subtitle);
        if self.subtitles.len() > MAX_SUBTITLES {
            self.subtitles.remove(0);
        }
    }

    /// The subtitle to show at the current position.
    pub fn active_subtitle(&self) -> Option<&Subtitle> {
        let position = self.position();
        let index = self
            .subtitles
            .partition_point(|subtitle| subtitle.start <= position);
        let subtitle = self.subtitles[..index].last()?;
        let next_start = self.subtitles.get(index).map(|next| next.start);
        let end = subtitle.end.or(next_start);
        let visible = end.is_none_or(|end| position < end);
        (visible && subtitle.content != SubtitleContent::Clear).then_some(subtitle)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shared.quit();
        if let Some(audio) = &self.audio {
            audio.set_paused(true);
        }
    }
}
