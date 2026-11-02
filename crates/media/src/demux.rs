use std::path::{Path, PathBuf};

use cut::{StreamIndex, Ticks, Timebase};

use crate::ffi::{InputContext, Packet, StreamParams};
use crate::{MediaError, MediaInfo, StreamKind};

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
            self.input.set_discard(index, !streams.contains(&index));
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

    /// Repositions to the last keyframe of `stream` at or before `target` in its own timebase.
    pub fn seek_stream(&mut self, stream: StreamIndex, target: Ticks) -> Result<(), MediaError> {
        self.input.seek(Some(stream), target)
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

/// How many packets a keyframe lookup may read before giving up.
const KEYFRAME_SCAN_LIMIT: usize = 5_000;
/// How far back a lookup retries when the first seek lands after the target.
const KEYFRAME_RETRY_BACK: Ticks = Ticks(10_000_000);

impl Demuxer {
    /// The timestamp of the last keyframe of `stream` at or before `target`, both in the
    /// stream's timebase. Changes the stream selection and read position.
    pub fn keyframe_at_or_before(
        &mut self,
        stream: StreamIndex,
        target: Ticks,
    ) -> Result<Option<Ticks>, MediaError> {
        let timebase = self
            .timebase(stream)
            .ok_or(MediaError::NoSuchStream(stream))?;
        self.select_streams(&[stream]);
        if let Some(found) = self.scan_keyframes(stream, target, target)? {
            return Ok(Some(found));
        }
        let back = KEYFRAME_RETRY_BACK.rescale(Timebase::MICROS, timebase);
        let earlier = (target - back).max(Ticks::ZERO);
        self.scan_keyframes(stream, earlier, target)
    }

    fn scan_keyframes(
        &mut self,
        stream: StreamIndex,
        seek_to: Ticks,
        target: Ticks,
    ) -> Result<Option<Ticks>, MediaError> {
        self.seek_stream(stream, seek_to)?;
        let mut packet = Packet::new()?;
        let mut best = None;
        for _ in 0..KEYFRAME_SCAN_LIMIT {
            if !self.read(&mut packet)? {
                break;
            }
            let Some(pts) = packet.pts() else {
                continue;
            };
            if packet.is_keyframe() && pts <= target {
                best = best.max(Some(pts));
            }
            let decoded_after = packet.dts().unwrap_or(pts) > target;
            if decoded_after && pts > target {
                break;
            }
        }
        Ok(best)
    }
}
