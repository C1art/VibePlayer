use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use audio::{AudioControl, Volume};
use crossbeam::channel::{self, Receiver, Sender};
use cut::{StreamIndex, Ticks};
use media::{AudioEffects, Decibels, Equalizer, Speed, Subtitle, VideoFrame, VideoOptions};

use crate::messages::{Command, DecodedVideo, Event, Opened, Shared};
use crate::reader::{self, ReaderSetup};
use crate::subtitle_list::SubtitleList;
use crate::{Clock, FrameQueue, Generation, PlaybackError, Shown};

mod events;

/// Decoded frames the video thread may get ahead of the screen.
const FRAMES_AHEAD: usize = 4;

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
    /// Whether the selected audio stream has a working decoder.
    audio_decodable: bool,
    /// The video decoder failed before the file was reported open.
    video_failed: Option<PlaybackError>,
    /// The pending seek shows the keyframe before its target, so the clock goes there.
    keyframe_seek: bool,
    /// Step-backs pressed while the previous one was still decoding.
    queued_steps_back: u32,
    stepping_back: bool,
    /// Frames were stepped forward while paused; audio is still where it was.
    stepped_forward: bool,
    hardware_decoding: Option<bool>,
    volume: Volume,
    muted: bool,
    effects: AudioEffects,
    subtitles: SubtitleList,
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
            audio_decodable: true,
            video_failed: None,
            keyframe_seek: false,
            queued_steps_back: 0,
            stepping_back: false,
            stepped_forward: false,
            hardware_decoding: None,
            volume: options.volume,
            muted: options.muted,
            effects,
            subtitles: SubtitleList::default(),
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
            self.media_arrived(now);
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
        self.continue_stepping_back(now);
        self.detect_end(now);
        Update {
            frame_changed: shown != Shown::Unchanged,
            repaint_after: self.repaint_after(position),
        }
    }

    fn media_arrived(&mut self, now: Instant) {
        self.waiting_for_media = false;
        if std::mem::take(&mut self.keyframe_seek)
            && let Some(keyframe) = self.queue.next_pts()
        {
            // Audio starts at the keyframe too; a clock at the target would wait for it.
            self.clock.jump(now, keyframe);
        }
        if self.playing {
            self.clock.play(now);
            if let Some(audio) = &self.audio {
                audio.set_paused(false);
            }
        }
    }

    /// Takes the next queued step back once the previous one's frame is on screen.
    fn continue_stepping_back(&mut self, now: Instant) {
        if self.waiting_for_media || self.queued_steps_back == 0 {
            return;
        }
        let remaining = self.queued_steps_back - 1;
        self.step_backward(now);
        self.queued_steps_back = remaining;
    }

    fn has_video(&self) -> bool {
        self.opened().is_some_and(|o| o.video.is_some())
    }

    fn has_audio(&self) -> bool {
        self.opened().is_some_and(|o| o.audio.is_some())
            && self.audio_error.is_none()
            && self.audio_decodable
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
        } else if self.stepped_forward {
            // Bring the audio to the stepped-to frame rather than hold the picture until the
            // audio catches up.
            self.seek_at(now, self.position(), SeekMode::Exact);
        }
        self.playing = true;
        if !self.waiting_for_media {
            self.clock.play(now);
            if let Some(audio) = &self.audio {
                audio.set_paused(false);
            }
        }
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.clock.pause();
        if let Some(audio) = &self.audio {
            audio.set_paused(true);
        }
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
        self.keyframe_seek = mode == SeekMode::Keyframe;
        self.queued_steps_back = 0;
        self.stepping_back = false;
        self.stepped_forward = false;
        self.clock.pause();
        self.clock.jump(now, target);
        if let Some(audio) = &self.audio {
            // Silent until the picture is ready too.
            audio.set_paused(true);
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
            self.stepped_forward = true;
        }
    }

    /// Pauses and shows the previous frame, decoding again from the prior keyframe.
    pub fn step_backward(&mut self, now: Instant) {
        self.pause();
        if self.waiting_for_media && self.stepping_back {
            // Until that frame arrives the one before it isn't known; step again after.
            self.queued_steps_back += 1;
            return;
        }
        let current = self.queue.current().map_or(self.position(), |f| f.pts);
        if current > Ticks::ZERO {
            self.seek_at(now, current - Ticks(1), SeekMode::Exact);
            self.stepping_back = true;
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

    /// The subtitle to show at the current position.
    pub fn active_subtitle(&self) -> Option<&Subtitle> {
        self.subtitles.active(self.position())
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
