use std::path::{Path, PathBuf};

use cut::{StreamIndex, Ticks, Timebase};

use crate::ffi::{Discard, InputContext, Packet, StreamParams};
use crate::{MediaError, MediaInfo, StreamKind};

/// How far before a presentation time to seek in formats that seek by decode time; far more
/// than any decoder's reordering delay.
const DECODE_TIME_MARGIN: Ticks = Ticks(1_000_000);

/// Reads packets from a file.
pub struct Demuxer {
    input: InputContext,
    path: PathBuf,
    selected: Option<Vec<StreamIndex>>,
}

impl Demuxer {
    pub fn open(path: &Path) -> Result<Demuxer, MediaError> {
        crate::init();
        Ok(Demuxer {
            input: InputContext::open(path)?,
            path: path.to_path_buf(),
            selected: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn info(&self) -> MediaInfo {
        MediaInfo::read(&self.path, &self.input)
    }

    pub fn best_stream(&self, kind: StreamKind) -> Option<StreamIndex> {
        self.input.best_stream(kind)
    }

    pub fn timebase(&self, stream: StreamIndex) -> Option<Timebase> {
        self.input.stream(stream)?.timebase()
    }

    /// Only packets of `streams` are read from now on.
    pub fn select_streams(&mut self, streams: &[StreamIndex]) {
        for index in 0..self.input.stream_count() {
            let index = StreamIndex(index);
            let discard = if streams.contains(&index) {
                Discard::Nothing
            } else {
                Discard::Everything
            };
            self.input.set_discard(index, discard);
        }
        self.selected = Some(streams.to_vec());
    }

    /// Reads the next packet of a selected stream into `packet`. Returns `false` at the end of
    /// the file.
    pub fn read(&mut self, packet: &mut Packet) -> Result<bool, MediaError> {
        // Some demuxers (Matroska) ignore the discard flag, so filter here too.
        loop {
            if !self.input.read_packet(packet)? {
                return Ok(false);
            }
            let wanted = self
                .selected
                .as_ref()
                .is_none_or(|streams| streams.contains(&packet.stream_index()));
            if wanted {
                return Ok(true);
            }
        }
    }

    /// Repositions to the last keyframe at or before `target` (microseconds).
    pub fn seek(&mut self, target: Ticks) -> Result<(), MediaError> {
        self.input.seek(None, target)
    }

    /// Repositions to a keyframe of `stream` at or before `target` (a presentation time in
    /// its own timebase): the last one, except in formats that seek by decode time, which go
    /// back a second further to be sure not to land just after it.
    pub fn seek_stream(&mut self, stream: StreamIndex, target: Ticks) -> Result<(), MediaError> {
        let target = match self.timebase(stream) {
            Some(timebase) if self.seeks_by_decode_time() => {
                target - DECODE_TIME_MARGIN.rescale(Timebase::MICROS, timebase)
            }
            _ => target,
        };
        self.input.seek(Some(stream), target)
    }

    /// MPEG-TS and program streams seek by decode time, which for a keyframe followed by
    /// B-frames comes before its presentation time.
    fn seeks_by_decode_time(&self) -> bool {
        matches!(self.input.format_name().as_str(), "mpegts" | "mpeg")
    }

    pub(crate) fn input(&self) -> &InputContext {
        &self.input
    }
}

impl Demuxer {
    /// An owned copy of a stream's decoding parameters, for decoders on other threads.
    pub fn stream_params(&self, stream: StreamIndex) -> Result<StreamParams, MediaError> {
        let stream = self
            .input
            .stream(stream)
            .ok_or(MediaError::NoSuchStream(stream))?;
        StreamParams::copy_from(&stream)
    }
}
