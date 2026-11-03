//! H.264/HEVC NAL unit framing: Annex B start codes vs. the length prefixes MP4 and Matroska
//! use, and the parameter sets stored in `avcC`/`hvcC` codec records.

/// How NAL units are framed inside packets of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NalFormat {
    AnnexB,
    /// Each unit is preceded by its length in this many big-endian bytes.
    LengthPrefixed(u8),
}

/// Reads a stream's framing and parameter sets from its codec record (`extradata`).
pub fn stream_layout(codec: &str, extradata: &[u8]) -> (NalFormat, Vec<Vec<u8>>) {
    let is_record = extradata.first() == Some(&1);
    match (codec, is_record) {
        ("h264", true) => parse_avcc(extradata),
        ("hevc", true) => parse_hvcc(extradata),
        _ => (
            NalFormat::AnnexB,
            split_annex_b(extradata)
                .into_iter()
                .map(<[u8]>::to_vec)
                .collect(),
        ),
    }
}

fn parse_avcc(record: &[u8]) -> (NalFormat, Vec<Vec<u8>>) {
    let length_size = record.get(4).map_or(4, |b| (b & 3) + 1);
    let mut units = Vec::new();
    let mut at = 5;
    for _ in 0..2 {
        let Some(&count_byte) = record.get(at) else {
            break;
        };
        // The SPS count carries three reserved bits; the PPS count doesn't.
        let count = if units.is_empty() {
            count_byte & 0x1f
        } else {
            count_byte
        };
        at += 1;
        for _ in 0..count {
            let Some(unit) = read_sized(record, &mut at) else {
                return (NalFormat::LengthPrefixed(length_size), units);
            };
            units.push(unit);
        }
    }
    (NalFormat::LengthPrefixed(length_size), units)
}

fn parse_hvcc(record: &[u8]) -> (NalFormat, Vec<Vec<u8>>) {
    let length_size = record.get(21).map_or(4, |b| (b & 3) + 1);
    let mut units = Vec::new();
    let mut at = 23;
    let arrays = record.get(22).copied().unwrap_or(0);
    for _ in 0..arrays {
        // One byte of array type, then a 16-bit unit count.
        let Some(count) = record.get(at + 1..at + 3) else {
            break;
        };
        let count = u16::from_be_bytes([count[0], count[1]]);
        at += 3;
        for _ in 0..count {
            let Some(unit) = read_sized(record, &mut at) else {
                return (NalFormat::LengthPrefixed(length_size), units);
            };
            units.push(unit);
        }
    }
    (NalFormat::LengthPrefixed(length_size), units)
}

fn read_sized(record: &[u8], at: &mut usize) -> Option<Vec<u8>> {
    let size = record.get(*at..*at + 2)?;
    let size = usize::from(u16::from_be_bytes([size[0], size[1]]));
    let unit = record.get(*at + 2..*at + 2 + size)?.to_vec();
    *at += 2 + size;
    Some(unit)
}

/// The NAL units in Annex B data, without their start codes.
pub fn split_annex_b(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push((i, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(n, &(_, begin))| {
            let mut end = starts.get(n + 1).map_or(data.len(), |&(code, _)| code);
            // A four-byte start code's leading zero belongs to it, not to this unit.
            while end > begin && data[end - 1] == 0 {
                end -= 1;
            }
            &data[begin..end]
        })
        .filter(|unit| !unit.is_empty())
        .collect()
}

/// Frames `units` for a stream of the given format.
/// `None` when a unit is too long for the length field (sources may use 1 or 2 bytes).
pub fn frame<'a>(units: impl IntoIterator<Item = &'a [u8]>, format: NalFormat) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for unit in units {
        match format {
            NalFormat::AnnexB => out.extend_from_slice(&[0, 0, 0, 1]),
            NalFormat::LengthPrefixed(size) => {
                let size = usize::from(size).min(8);
                let length = unit.len() as u64;
                if size < 8 && length >> (size * 8) != 0 {
                    return None;
                }
                out.extend_from_slice(&length.to_be_bytes()[8 - size..]);
            }
        }
        out.extend_from_slice(unit);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_too_long_for_the_length_field_are_refused() {
        let unit = vec![0x65; 70_000];
        assert!(frame([unit.as_slice()], NalFormat::LengthPrefixed(2)).is_none());
        assert!(frame([unit.as_slice()], NalFormat::LengthPrefixed(4)).is_some());
    }

    #[test]
    fn annex_b_splits_on_three_and_four_byte_start_codes() {
        let data = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4,
        ];
        assert_eq!(
            split_annex_b(&data),
            [&[0x67, 1, 2][..], &[0x68, 3], &[0x65, 4]]
        );
    }

    #[test]
    fn units_are_reframed_with_length_prefixes() {
        let framed = frame([&[0x67, 1, 2][..], &[0x68]], NalFormat::LengthPrefixed(4)).unwrap();
        assert_eq!(framed, [0, 0, 0, 3, 0x67, 1, 2, 0, 0, 0, 1, 0x68]);
    }

    #[test]
    fn avcc_records_yield_their_length_size_and_parameter_sets() {
        let record = [
            1, 0x64, 0, 0x1f, 0xff, 0xe1, 0, 3, 0x67, 1, 2, 1, 0, 2, 0x68, 9,
        ];
        let (format, units) = stream_layout("h264", &record);
        assert_eq!(format, NalFormat::LengthPrefixed(4));
        assert_eq!(units, [vec![0x67, 1, 2], vec![0x68, 9]]);
    }

    #[test]
    fn hvcc_records_yield_every_array() {
        let mut record = vec![1; 21];
        record.push(0xff);
        record.push(2);
        record.extend([0x20, 0, 1, 0, 2, 0x40, 1]);
        record.extend([0x21, 0, 1, 0, 1, 0x42]);
        let (format, units) = stream_layout("hevc", &record);
        assert_eq!(format, NalFormat::LengthPrefixed(4));
        assert_eq!(units, [vec![0x40, 1], vec![0x42]]);
    }

    #[test]
    fn annex_b_extradata_is_kept_as_annex_b() {
        let (format, units) = stream_layout("h264", &[0, 0, 0, 1, 0x67, 5]);
        assert_eq!((format, units), (NalFormat::AnnexB, vec![vec![0x67, 5]]));
    }
}
