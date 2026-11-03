//! Finding every keyframe of a stream, cheaply where the container's index allows it.

use cut::{StreamIndex, Ticks};

use crate::{Demuxer, MediaError, Packet};

/// Packets between progress reports of a full read.
const REPORT_EVERY: usize = 2_000;
/// Keyframes between progress reports when visiting them one by one.
const REPORT_EVERY_KEYFRAMES: usize = 50;
/// Packets read from the start to check the index against.
const INDEX_CHECK_PACKETS: usize = 3_000;

/// What the container's index turned out to hold, checked against the start of the stream.
enum Index {
    /// Every keyframe at its presentation time (Matroska cues, MP4 without B-frames).
    PresentationTimes(Vec<Ticks>),
    /// Every keyframe at its decode time (MP4): each must be visited to learn when it shows.
    DecodeTimes(Vec<Ticks>),
    /// Not every keyframe, or none: the stream must be read, rewound to here if the check
    /// read some of it.
    Incomplete(Option<Ticks>),
}

impl Demuxer {
    /// Every keyframe of `stream` at its presentation time, reported in batches so a long file
    /// shows progress. Stops early when `report` returns `false`.
    ///
    /// Reading a long file end to end is slow on a hard disk, so this uses the container's
    /// index where it lists every keyframe, and reads the whole stream only otherwise.
    pub fn scan_all_keyframes(
        &mut self,
        stream: StreamIndex,
        mut report: impl FnMut(Vec<Ticks>) -> bool,
    ) -> Result<(), MediaError> {
        self.select_streams(&[stream]);
        match self.check_index(stream)? {
            Index::PresentationTimes(keyframes) => {
                report(keyframes);
                Ok(())
            }
            Index::DecodeTimes(keyframes) => {
                if self.visit_keyframes(stream, &keyframes, &mut report)? {
                    return Ok(());
                }
                self.seek_stream(stream, keyframes[0].min(Ticks::ZERO))?;
                self.read_all_keyframes(report)
            }
            Index::Incomplete(rewind) => {
                if let Some(from) = rewind {
                    self.seek_stream(stream, from)?;
                }
                self.read_all_keyframes(report)
            }
        }
    }

    /// Compares the keyframes at the start of the stream with the index. Muxers may index only
    /// some keyframes (FFmpeg's Matroska cues mark one per cluster), so it can't be taken on
    /// trust.
    fn check_index(&mut self, stream: StreamIndex) -> Result<Index, MediaError> {
        let mut indexed = self.input().indexed_keyframes(stream);
        indexed.sort_unstable();
        indexed.dedup();
        let Some(&first) = indexed.first() else {
            return Ok(Index::Incomplete(None));
        };
        if indexed.len() < 2 {
            return Ok(Index::Incomplete(None));
        }
        let incomplete = Index::Incomplete(Some(first.min(Ticks::ZERO)));
        let listed =
            |ticks: Option<Ticks>| ticks.is_some_and(|t| indexed.binary_search(&t).is_ok());
        let (mut checked, mut by_pts, mut by_dts) = (0, true, true);
        let mut packet = Packet::new()?;
        for _ in 0..INDEX_CHECK_PACKETS {
            if !self.read(&mut packet)? {
                break;
            }
            if packet.is_keyframe() && packet.pts().is_some() {
                checked += 1;
                by_pts &= listed(packet.pts());
                by_dts &= listed(packet.dts());
            }
        }
        let enough = checked >= 2 || indexed.len() <= checked;
        Ok(match (enough, by_pts, by_dts) {
            (true, true, _) => Index::PresentationTimes(indexed),
            (true, false, true) => Index::DecodeTimes(indexed),
            _ => incomplete,
        })
    }

    /// Seeks to each indexed keyframe and reads just that packet, which carries its
    /// presentation time. Only keyframe data comes off the disk. Returns `false` if a seek
    /// lands anywhere but the indexed packet, so the caller can read the stream instead.
    fn visit_keyframes(
        &mut self,
        stream: StreamIndex,
        indexed: &[Ticks],
        report: &mut impl FnMut(Vec<Ticks>) -> bool,
    ) -> Result<bool, MediaError> {
        let mut packet = Packet::new()?;
        let mut batch = Vec::new();
        // Seeking just short of the next keyframe lands on this one, whether the demuxer
        // searches by decode time or (as MP4 does) by presentation time, which comes later.
        let next = indexed.iter().skip(1).map(|&next| next - Ticks(1));
        let visits: Vec<(Ticks, Ticks)> = indexed
            .iter()
            .copied()
            .zip(next.chain([Ticks(i64::MAX)]))
            .collect();
        for chunk in visits.chunks(REPORT_EVERY_KEYFRAMES) {
            for &(at, seek_to) in chunk {
                self.seek_stream(stream, seek_to)?;
                let landed =
                    self.read(&mut packet)? && packet.is_keyframe() && packet.dts() == Some(at);
                match packet.pts() {
                    Some(pts) if landed => batch.push(pts),
                    _ => return Ok(false),
                }
            }
            if !report(std::mem::take(&mut batch)) {
                return Ok(true);
            }
        }
        Ok(true)
    }

    fn read_all_keyframes(
        &mut self,
        mut report: impl FnMut(Vec<Ticks>) -> bool,
    ) -> Result<(), MediaError> {
        let mut packet = Packet::new()?;
        let mut batch = Vec::new();
        let mut since_report = 0usize;
        loop {
            match self.read(&mut packet) {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    // Keep what was found before the damage.
                    report(batch);
                    return Err(error);
                }
            }
            since_report += 1;
            if packet.is_keyframe()
                && let Some(pts) = packet.pts()
            {
                batch.push(pts);
            }
            if since_report >= REPORT_EVERY {
                since_report = 0;
                if !report(std::mem::take(&mut batch)) {
                    return Ok(());
                }
            }
        }
        report(batch);
        Ok(())
    }
}
