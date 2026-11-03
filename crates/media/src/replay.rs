//! The packets of the current GOP, kept while decoding in hardware so a software decoder that
//! takes over mid-GOP can start from its keyframe.

use crate::MediaError;
use crate::ffi::Packet;

/// A GOP of high-bitrate 4K can be tens of megabytes; past this, give up on replaying it.
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct ReplayBuffer {
    packets: Vec<Packet>,
    bytes: usize,
    overflowed: bool,
}

impl ReplayBuffer {
    pub fn record(&mut self, packet: &Packet) -> Result<(), MediaError> {
        if packet.is_keyframe() {
            self.clear();
        }
        if self.overflowed {
            return Ok(());
        }
        self.bytes += packet.data().len();
        if self.bytes > MAX_BYTES {
            self.packets.clear();
            self.overflowed = true;
        } else {
            self.packets.push(packet.try_clone()?);
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.packets.clear();
        self.bytes = 0;
        self.overflowed = false;
    }

    /// The packets from the last keyframe on, unless they didn't all fit.
    pub fn take(&mut self) -> Vec<Packet> {
        let packets = if self.overflowed {
            Vec::new()
        } else {
            std::mem::take(&mut self.packets)
        };
        self.clear();
        packets
    }
}
