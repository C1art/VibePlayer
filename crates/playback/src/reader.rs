use std::mem;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam::channel::{self, Receiver, SendTimeoutError, Sender};
use cut::{StreamIndex, Ticks, Timebase};
use media::{Demuxer, MediaInfo, Packet, StreamKind, SubtitleDecoder, VideoOptions};

use crate::messages::{Command, DecodedVideo, Event, Opened, PacketMessage, Shared};
use crate::{Generation, PlaybackError, audio_worker, video_worker};

const VIDEO_PACKETS: usize = 96;
const AUDIO_PACKETS: usize = 192;
/// Consecutive read errors after which a file is treated as ended rather than retried forever.
const MAX_READ_ERRORS: u32 = 64;

pub(crate) struct ReaderSetup {
    pub path: PathBuf,
    pub video_options: VideoOptions,
    pub volume: audio::Volume,
    pub shared: Arc<Shared>,
    pub commands: Receiver<Command>,
    pub events: Sender<Event>,
    pub frames: Sender<DecodedVideo>,
}

pub(crate) fn spawn(setup: ReaderSetup) {
    let spawned = thread::Builder::new()
        .name("reader".into())
        .spawn(move || run(setup));
    if let Err(error) = spawned {
        // Without the reader nothing will ever report back; nothing more we can do here.
        drop(error);
    }
}

/// Where a stream's packets go.
struct Route {
    stream: Option<StreamIndex>,
    sender: Option<Sender<PacketMessage>>,
}

impl Route {
    fn none() -> Route {
        Route {
            stream: None,
            sender: None,
        }
    }
}

struct Reader {
    demuxer: Demuxer,
    info: MediaInfo,
    shared: Arc<Shared>,
    commands: Receiver<Command>,
    events: Sender<Event>,
    generation: Generation,
    video: Route,
    audio: Route,
    subtitle: Option<(StreamIndex, SubtitleDecoder)>,
    video_size: (u32, u32),
    volume: audio::Volume,
}

fn run(setup: ReaderSetup) {
    let demuxer = match Demuxer::open(&setup.path) {
        Ok(demuxer) => demuxer,
        Err(error) => {
            let _ = setup.events.send(Event::Failed(error.into()));
            return;
        }
    };
    let info = demuxer.info();
    let video_stream = demuxer.best_stream(StreamKind::Video);
    let audio_stream = demuxer.best_stream(StreamKind::Audio);
    let subtitle_stream = info
        .streams_of(StreamKind::Subtitle)
        .find(|s| s.is_default)
        .map(|s| s.index);
    let video_size = info
        .video()
        .map_or((1920, 1080), |(_, video)| (video.width, video.height));
    let mut reader = Reader {
        demuxer,
        info,
        shared: setup.shared,
        commands: setup.commands,
        events: setup.events,
        generation: Generation::default(),
        video: Route::none(),
        audio: Route::none(),
        subtitle: None,
        video_size,
        volume: setup.volume,
    };
    if let Some(stream) = video_stream {
        reader.start_video(stream, setup.video_options, setup.frames);
    }
    reader.start_audio(audio_stream);
    reader.select_subtitle(subtitle_stream);
    reader.apply_selection();
    let opened = Opened {
        info: reader.info.clone(),
        video: reader.video.stream,
        audio: reader.audio.stream,
        subtitle: reader.subtitle.as_ref().map(|(stream, _)| *stream),
    };
    if reader.events.send(Event::Opened(Box::new(opened))).is_ok() {
        reader.run();
    }
}

impl Reader {
    fn start_video(
        &mut self,
        stream: StreamIndex,
        options: VideoOptions,
        frames: Sender<DecodedVideo>,
    ) {
        let params = match self.demuxer.stream_params(stream) {
            Ok(params) => params,
            Err(error) => {
                let _ = self.events.send(Event::Failed(PlaybackError::Media(error)));
                return;
            }
        };
        let (sender, receiver) = channel::bounded(VIDEO_PACKETS);
        let worker = video_worker::Setup {
            params,
            options,
            start_time: self.info.start_time,
            packets: receiver,
            frames,
            events: self.events.clone(),
            shared: Arc::clone(&self.shared),
        };
        if video_worker::spawn(worker) {
            self.video = Route {
                stream: Some(stream),
                sender: Some(sender),
            };
        }
    }

    fn start_audio(&mut self, stream: Option<StreamIndex>) {
        let params = stream.and_then(|s| self.demuxer.stream_params(s).ok());
        let (sender, receiver) = channel::bounded(AUDIO_PACKETS);
        let worker = audio_worker::Setup {
            params,
            volume: self.volume,
            start_time: self.info.start_time,
            packets: receiver,
            events: self.events.clone(),
            shared: Arc::clone(&self.shared),
        };
        if audio_worker::spawn(worker) {
            self.audio = Route {
                stream,
                sender: Some(sender),
            };
        }
    }

    fn select_audio(&mut self, stream: Option<StreamIndex>) {
        let params = stream.and_then(|s| self.demuxer.stream_params(s).ok());
        self.audio.stream = stream.filter(|_| params.is_some());
        if let Some(sender) = &self.audio.sender {
            let _ = sender.send(PacketMessage::SetStream(params));
        }
    }

    fn select_subtitle(&mut self, stream: Option<StreamIndex>) {
        self.subtitle = stream.and_then(|stream| {
            let params = self.demuxer.stream_params(stream).ok()?;
            let decoder = SubtitleDecoder::new(params, self.video_size).ok()?;
            Some((stream, decoder))
        });
    }

    fn apply_selection(&mut self) {
        let streams: Vec<StreamIndex> = [
            self.video.stream,
            self.audio.stream,
            self.subtitle.as_ref().map(|(stream, _)| *stream),
        ]
        .into_iter()
        .flatten()
        .collect();
        self.demuxer.select_streams(&streams);
    }

    fn run(mut self) {
        let mut packet = match Packet::new() {
            Ok(packet) => packet,
            Err(error) => {
                let _ = self.events.send(Event::Failed(error.into()));
                return;
            }
        };
        let mut ended = false;
        let mut read_errors = 0;
        loop {
            if self.shared.should_quit() {
                return;
            }
            let command = if ended {
                match self.commands.recv_timeout(Duration::from_millis(100)) {
                    Ok(command) => Some(command),
                    Err(channel::RecvTimeoutError::Timeout) => None,
                    Err(channel::RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match self.commands.try_recv() {
                    Ok(command) => Some(command),
                    Err(channel::TryRecvError::Empty) => None,
                    Err(channel::TryRecvError::Disconnected) => return,
                }
            };
            if let Some(command) = command {
                self.handle(command);
                ended = false;
                read_errors = 0;
                continue;
            }
            if ended {
                continue;
            }
            match self.demuxer.read(&mut packet) {
                Ok(true) => {
                    read_errors = 0;
                    self.route(&mut packet);
                }
                Ok(false) => {
                    self.end();
                    ended = true;
                }
                Err(_) => {
                    read_errors += 1;
                    if read_errors >= MAX_READ_ERRORS {
                        self.end();
                        ended = true;
                    }
                }
            }
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Seek {
                generation,
                target,
                exact,
            } => self.seek(generation, target, exact),
            Command::SelectAudio(stream) => {
                self.select_audio(stream);
                self.apply_selection();
            }
            Command::SelectSubtitle(stream) => {
                self.select_subtitle(stream);
                self.apply_selection();
            }
        }
    }

    fn seek(&mut self, generation: Generation, target: Ticks, exact: bool) {
        self.generation = generation;
        let start = self.info.start_time;
        if self.seek_demuxer(start + target).is_err() {
            let _ = self.demuxer.seek(start);
        }
        if let Some((_, decoder)) = &mut self.subtitle {
            decoder.flush();
        }
        let flush = || PacketMessage::Flush {
            generation,
            target,
            exact_target: exact.then_some(target),
        };
        for sender in [&self.video.sender, &self.audio.sender]
            .into_iter()
            .flatten()
        {
            let _ = sender.send(flush());
        }
    }

    /// Seeks in the video stream's own timebase, rounding down: rounding to the nearest tick
    /// could land on the keyframe just after the target and skip the frame being asked for.
    fn seek_demuxer(&mut self, absolute: Ticks) -> Result<(), media::MediaError> {
        let video = self
            .video
            .stream
            .and_then(|stream| Some((stream, self.demuxer.timebase(stream)?)));
        match video {
            Some((stream, timebase)) => {
                let mut ticks = absolute.rescale(Timebase::MICROS, timebase);
                if ticks.rescale(timebase, Timebase::MICROS) > absolute {
                    ticks = ticks - Ticks(1);
                }
                self.demuxer.seek_stream(stream, ticks)
            }
            None => self.demuxer.seek(absolute),
        }
    }

    fn end(&mut self) {
        let generation = self.generation;
        for sender in [&self.video.sender, &self.audio.sender]
            .into_iter()
            .flatten()
        {
            let _ = sender.send(PacketMessage::End(generation));
        }
    }

    fn route(&mut self, packet: &mut Packet) {
        let stream = packet.stream_index();
        if let Some((subtitle_stream, decoder)) = &mut self.subtitle
            && *subtitle_stream == stream
        {
            if let Ok(Some(mut subtitle)) = decoder.decode(packet) {
                let start = self.info.start_time;
                subtitle.start = subtitle.start - start;
                subtitle.end = subtitle.end.map(|end| end - start);
                let _ = self.events.send(Event::Subtitle(self.generation, subtitle));
            }
            return;
        }
        let route = if Some(stream) == self.video.stream {
            &self.video
        } else if Some(stream) == self.audio.stream {
            &self.audio
        } else {
            return;
        };
        let Some(sender) = &route.sender else {
            return;
        };
        let Ok(fresh) = Packet::new() else {
            return;
        };
        let message = PacketMessage::Packet(self.generation, mem::replace(packet, fresh));
        self.send_responsively(sender.clone(), message);
    }

    /// Waits for room in a decoder's queue, but gives up as soon as a command or quit arrives,
    /// so seeks are never stuck behind a full queue.
    fn send_responsively(&self, sender: Sender<PacketMessage>, mut message: PacketMessage) {
        loop {
            match sender.send_timeout(message, Duration::from_millis(10)) {
                Ok(()) | Err(SendTimeoutError::Disconnected(_)) => return,
                Err(SendTimeoutError::Timeout(returned)) => {
                    if self.shared.should_quit()
                        || !self.commands.is_empty()
                        || self.shared.is_stale(self.generation)
                    {
                        return;
                    }
                    message = returned;
                }
            }
        }
    }
}
