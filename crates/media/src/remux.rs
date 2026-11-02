use std::path::Path;

use cut::{StreamIndex, Timebase};

use crate::ffi::{CodecContext, OutputContext, Packet, Received, StreamParams};
use crate::{AudioFrame, AudioTarget, Chapter, Demuxer, MediaError};

/// Writes a new file from copied and re-encoded streams.
pub struct Muxer {
    output: OutputContext,
}

impl Muxer {
    /// The container format follows the file extension.
    pub fn create(path: &Path) -> Result<Muxer, MediaError> {
        crate::init();
        Ok(Muxer {
            output: OutputContext::create(path, None)?,
        })
    }

    pub fn needs_global_header(&self) -> bool {
        self.output.needs_global_header()
    }

    /// Adds an output stream whose packets are copied unchanged from `source`.
    pub fn add_copied_stream(
        &mut self,
        demuxer: &Demuxer,
        source: StreamIndex,
    ) -> Result<StreamIndex, MediaError> {
        self.output.add_copied_stream(demuxer.input(), source)
    }

    /// Adds an output stream fed by `encoder`, keeping `source`'s language and disposition.
    pub fn add_encoded_stream(
        &mut self,
        encoder: &mut AudioEncoder,
        demuxer: &Demuxer,
        source: StreamIndex,
    ) -> Result<StreamIndex, MediaError> {
        self.output
            .add_encoded_stream(&mut encoder.codec, demuxer.input(), source)
    }

    pub fn copy_metadata(&mut self, demuxer: &Demuxer) {
        self.output.copy_metadata_from(demuxer.input());
    }

    pub fn set_chapters(&mut self, chapters: &[Chapter]) -> Result<(), MediaError> {
        self.output.set_chapters(chapters)
    }

    pub fn write_header(&mut self) -> Result<(), MediaError> {
        self.output.write_header()
    }

    /// The timebase packets for `stream` must use; only final after [`Muxer::write_header`].
    pub fn timebase(&self, stream: StreamIndex) -> Option<Timebase> {
        self.output.stream_timebase(stream)
    }

    /// Writes a packet whose stream index and timestamps are already in output terms.
    pub fn write(&mut self, packet: &mut Packet) -> Result<(), MediaError> {
        self.output.write_packet(packet)
    }

    pub fn finish(self) -> Result<(), MediaError> {
        self.output.finish()
    }
}

/// Re-encodes one audio stream, for exports that change volume or EQ.
pub struct AudioEncoder {
    codec: CodecContext,
}

impl AudioEncoder {
    /// Encodes like `source`, or with the encoder named `codec` when given (e.g. `aac`).
    pub fn open(
        source: &StreamParams,
        codec: Option<&str>,
        bit_rate: u64,
        global_header: bool,
    ) -> Result<AudioEncoder, MediaError> {
        let codec = CodecContext::audio_encoder(&source.view(), codec, bit_rate, global_header)?;
        Ok(AudioEncoder { codec })
    }

    pub fn codec_name(&self) -> String {
        self.codec.codec_name()
    }

    /// The format decoded audio must be converted to before [`AudioEncoder::send`].
    pub fn target(&self) -> AudioTarget {
        AudioTarget {
            sample_format: self
                .codec
                .sample_format_name()
                .unwrap_or_else(|| "fltp".into()),
            sample_rate: self.codec.sample_rate(),
            layout: self.codec.channel_layout_description(),
            frame_size: self.codec.frame_size(),
        }
    }

    pub fn timebase(&self) -> Option<Timebase> {
        self.codec.timebase()
    }

    /// Feeds a frame stamped in the encoder's timebase, or `None` to drain. Returns `false`
    /// when packets must be received first.
    pub fn send(&mut self, frame: Option<&AudioFrame>) -> Result<bool, MediaError> {
        self.codec.send_frame(frame.map(AudioFrame::frame))
    }

    /// The next encoded packet, stamped in the encoder's timebase. `false` when none is ready.
    pub fn receive(&mut self, packet: &mut Packet) -> Result<bool, MediaError> {
        Ok(self.codec.receive_packet(packet)? == Received::Output)
    }
}
