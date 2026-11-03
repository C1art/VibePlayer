//! Taking in what the reader and decoder threads report.

use std::time::Instant;

use audio::AudioControl;
use cut::Ticks;

use super::{FRAMES_AHEAD, LoadState, Player, SeekMode};
use crate::messages::{Event, Opened};
use crate::{AudioReading, PlaybackError};

impl Player {
    pub(super) fn receive_events(&mut self, now: Instant) {
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
                    control.set_paused(!self.playing || self.waiting_for_media);
                    self.audio = Some(control);
                }
                Event::AudioFailed(error) => self.audio_error = Some(error),
                Event::AudioDecoder { available } => self.audio_decodable = available,
                Event::VideoFailed(error) => {
                    if matches!(self.state, LoadState::Ready(_)) {
                        self.drop_video(error);
                    } else {
                        self.video_failed = Some(error);
                    }
                }
                Event::Subtitle(generation, subtitle) => {
                    if generation == self.generation {
                        self.subtitles.add(subtitle);
                    }
                }
                Event::VideoEnded(generation) => self.video_ended |= generation == self.generation,
                Event::AudioEnded(generation) => self.audio_ended |= generation == self.generation,
            }
        }
    }

    fn on_opened(&mut self, opened: Box<Opened>, now: Instant) {
        self.state = LoadState::Ready(opened);
        if let Some(error) = self.video_failed.take() {
            self.drop_video(error);
        }
        if let Some(start) = self.pending_start.take() {
            let within = self.duration().is_none_or(|d| start < d - Ticks(5_000_000));
            if within {
                self.seek_at(now, start, SeekMode::Exact);
            }
        }
    }

    pub(super) fn receive_frames(&mut self) {
        while self.queue.pending() < FRAMES_AHEAD {
            match self.frames.try_recv() {
                Ok(decoded) => {
                    self.queue.push(decoded.generation, decoded.frame);
                }
                Err(_) => break,
            }
        }
    }

    /// Plays on without a video stream that can't be decoded, or fails if nothing is left.
    fn drop_video(&mut self, error: PlaybackError) {
        let LoadState::Ready(opened) = &mut self.state else {
            return;
        };
        opened.video = None;
        if opened.audio.is_none() {
            self.state = LoadState::Failed(error);
        }
    }

    pub(super) fn audio_reading(&self) -> Option<AudioReading> {
        let control = self.audio.as_ref()?;
        if !self.has_audio() || control.is_device_lost() {
            return None;
        }
        if self.audio_ended && control.queued_frames() == 0 {
            return None;
        }
        let anchors = self.shared.audio_anchor.load()?;
        (anchors.current.generation == self.generation).then(|| AudioReading {
            anchors,
            played_frames: control.played_frames(),
            latency: control.latency(),
            sample_rate: control.format().sample_rate,
        })
    }

    pub(super) fn media_ready(&self, audio_anchored: bool) -> bool {
        if self.opened().is_none() {
            return false;
        }
        let video_ready = !self.has_video()
            || !self.queue.is_awaiting_first()
            || self.queue.pending() > 0
            || self.video_ended;
        // A lost device publishes no anchor; the system clock stands in until it's back.
        let device_lost = self
            .audio
            .as_ref()
            .is_some_and(AudioControl::is_device_lost);
        let audio_ready = !self.has_audio()
            || audio_anchored
            || self.audio_ended
            || device_lost
            || (self.audio.is_none() && self.audio_error.is_none() && !self.has_video());
        video_ready && (audio_ready || !self.playing)
    }

    pub(super) fn detect_end(&mut self, now: Instant) {
        if self.ended || self.waiting_for_media || !self.playing || self.opened().is_none() {
            return;
        }
        let video_done = !self.has_video() || (self.video_ended && self.queue.pending() == 0);
        // A device that vanished never drains its queue, so it can't hold up the end.
        let audio_drained =
            |audio: &AudioControl| audio.queued_frames() == 0 || audio.is_device_lost();
        let audio_done = !self.has_audio()
            || (self.audio_ended && self.audio.as_ref().is_none_or(audio_drained));
        let past_end = self
            .duration()
            .is_some_and(|d| self.position() > d + Ticks(1_000_000));
        if video_done && (audio_done || past_end) {
            self.ended = true;
            self.pause();
            let end = self.duration().unwrap_or(self.position());
            self.clock.jump(now, end);
        }
    }
}
