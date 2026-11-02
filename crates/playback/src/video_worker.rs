use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam::channel::{Receiver, SendTimeoutError, Sender};
use cut::Ticks;
use media::{StreamParams, VideoDecoder, VideoFrame, VideoOptions};

use crate::Generation;
use crate::messages::{DecodedVideo, Event, PacketMessage, Shared};

pub(crate) struct Setup {
    pub params: StreamParams,
    pub options: VideoOptions,
    pub start_time: Ticks,
    pub packets: Receiver<PacketMessage>,
    pub frames: Sender<DecodedVideo>,
    pub events: Sender<Event>,
    pub shared: Arc<Shared>,
}

pub(crate) fn spawn(setup: Setup) -> bool {
    thread::Builder::new()
        .name("video".into())
        .spawn(move || run(setup))
        .is_ok()
}

/// After an exact seek, holds back frames until the one covering the target is known.
struct ExactSeek {
    target: Ticks,
    held: Option<VideoFrame>,
}

struct Worker {
    decoder: VideoDecoder,
    start_time: Ticks,
    frames: Sender<DecodedVideo>,
    shared: Arc<Shared>,
    generation: Generation,
    exact: Option<ExactSeek>,
}

enum Flow {
    Continue,
    Stop,
}

fn run(setup: Setup) {
    let decoder = match VideoDecoder::new(setup.params, setup.options) {
        Ok(decoder) => decoder,
        Err(error) => {
            let _ = setup.events.send(Event::Failed(error.into()));
            return;
        }
    };
    let mut hardware = decoder.is_hardware();
    let _ = setup.events.send(Event::VideoDecoder { hardware });
    let mut worker = Worker {
        decoder,
        start_time: setup.start_time,
        frames: setup.frames,
        shared: setup.shared,
        generation: Generation::default(),
        exact: None,
    };
    for message in setup.packets {
        if worker.shared.should_quit() {
            return;
        }
        let flow = match message {
            PacketMessage::Packet(generation, packet) => {
                if worker.shared.is_stale(generation) || generation != worker.generation {
                    continue;
                }
                worker.decode(Some(&packet))
            }
            PacketMessage::Flush {
                generation,
                exact_target,
            } => {
                worker.generation = generation;
                worker.decoder.flush();
                worker.exact = exact_target.map(|target| ExactSeek { target, held: None });
                Flow::Continue
            }
            PacketMessage::End(generation) => {
                if generation != worker.generation {
                    continue;
                }
                let flow = worker.decode(None);
                worker.release_held();
                let _ = setup.events.send(Event::VideoEnded(generation));
                flow
            }
            PacketMessage::SetStream(_) => Flow::Continue,
        };
        if let Flow::Stop = flow {
            return;
        }
        if worker.decoder.is_hardware() != hardware {
            hardware = worker.decoder.is_hardware();
            let _ = setup.events.send(Event::VideoDecoder { hardware });
        }
    }
}

impl Worker {
    fn decode(&mut self, packet: Option<&media::Packet>) -> Flow {
        loop {
            let accepted = match self.decoder.send(packet) {
                Ok(accepted) => accepted,
                Err(_) => return Flow::Continue,
            };
            if let Flow::Stop = self.drain() {
                return Flow::Stop;
            }
            if accepted || self.shared.is_stale(self.generation) {
                return Flow::Continue;
            }
        }
    }

    fn drain(&mut self) -> Flow {
        loop {
            let mut frame = match self.decoder.receive() {
                Ok(Some(frame)) => frame,
                Ok(None) | Err(_) => return Flow::Continue,
            };
            frame.pts = frame.pts - self.start_time;
            let ready = match &mut self.exact {
                Some(exact) if frame.pts <= exact.target => {
                    exact.held = Some(frame);
                    continue;
                }
                Some(exact) => {
                    let held = exact.held.take();
                    self.exact = None;
                    if let Some(held) = held
                        && let Flow::Stop = self.emit(held)
                    {
                        return Flow::Stop;
                    }
                    frame
                }
                None => frame,
            };
            if let Flow::Stop = self.emit(ready) {
                return Flow::Stop;
            }
        }
    }

    fn release_held(&mut self) {
        if let Some(held) = self.exact.take().and_then(|exact| exact.held) {
            let _ = self.emit(held);
        }
    }

    /// Hands a frame to the player, waiting for room but dropping it if a seek made it stale.
    fn emit(&mut self, frame: VideoFrame) -> Flow {
        let mut message = DecodedVideo {
            generation: self.generation,
            frame,
        };
        loop {
            if self.shared.is_stale(self.generation) {
                return if self.shared.should_quit() {
                    Flow::Stop
                } else {
                    Flow::Continue
                };
            }
            match self.frames.send_timeout(message, Duration::from_millis(20)) {
                Ok(()) => return Flow::Continue,
                Err(SendTimeoutError::Timeout(returned)) => message = returned,
                Err(SendTimeoutError::Disconnected(_)) => return Flow::Stop,
            }
        }
    }
}
