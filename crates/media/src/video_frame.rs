//! What the video decoder takes and gives: options, plane layouts and decoded frames.

use cut::Ticks;

use crate::ffi::{Frame, MasteringLuminance};
use crate::gpu::{GpuPicture, GpuSharing};
use crate::{ColorInfo, Rotation};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareDecoding {
    Auto,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoOptions {
    pub hardware: HardwareDecoding,
    /// Whether frames may come out with more than 8 bits per sample; otherwise they're reduced.
    pub high_bit_depth: bool,
    /// Whether interlaced frames are deinterlaced, as for display; re-encoding keeps fields.
    pub deinterlace: bool,
    /// Keep hardware-decoded pictures on this GPU instead of copying them to system memory.
    pub gpu: Option<GpuSharing>,
}

/// How the three planes of a [`VideoFrame`] are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlaneLayout {
    /// Significant bits per sample: 8 (one byte) or 10/12/16 (two little-endian bytes).
    pub bits: u8,
    /// Chroma planes are this many powers of two narrower and shorter than luma.
    pub chroma_shift_x: u8,
    pub chroma_shift_y: u8,
    /// U and V share plane 1, interleaved (NV12, P010), as hardware decoders produce them.
    pub interleaved_chroma: bool,
}

impl PlaneLayout {
    pub fn bytes_per_sample(self) -> usize {
        if self.bits > 8 { 2 } else { 1 }
    }

    pub(crate) fn from_format_name(name: &str) -> Option<PlaneLayout> {
        let interleaved = |bits| PlaneLayout {
            bits,
            chroma_shift_x: 1,
            chroma_shift_y: 1,
            interleaved_chroma: true,
        };
        match name {
            "nv12" => return Some(interleaved(8)),
            // P010 keeps its 10 bits at the top of each 16-bit sample, so it reads as 16-bit.
            "p010le" => return Some(interleaved(16)),
            _ => {}
        }
        let (bits, rest) = match name.strip_suffix("le") {
            Some(base) => {
                let digits_at = base.rfind('p')? + 1;
                (base[digits_at..].parse().ok()?, &base[..digits_at])
            }
            None => (8, name),
        };
        let (chroma_shift_x, chroma_shift_y) = match rest {
            "yuv420p" => (1, 1),
            "yuv422p" => (1, 0),
            "yuv444p" => (0, 0),
            _ => return None,
        };
        Some(PlaneLayout {
            bits,
            chroma_shift_x,
            chroma_shift_y,
            interleaved_chroma: false,
        })
    }
}

/// A decoded, display-ready frame in planar YUV.
pub struct VideoFrame {
    pub(crate) frame: Frame,
    /// Presentation time in microseconds.
    pub pts: Ticks,
    /// Presentation time in the stream's own timebase.
    pub stream_pts: Ticks,
    /// In microseconds; zero when unknown.
    pub duration: Ticks,
    pub layout: PlaneLayout,
    pub color: ColorInfo,
    pub luminance: MasteringLuminance,
    pub rotation: Rotation,
    /// Set when the picture stayed on the GPU; the planes are then empty.
    pub gpu: Option<GpuPicture>,
}

impl VideoFrame {
    pub fn width(&self) -> u32 {
        self.frame.width()
    }

    pub fn height(&self) -> u32 {
        self.frame.height()
    }

    /// Plane `index` (0 = Y, 1 = U or interleaved UV, 2 = V) as `(bytes, stride)`.
    pub fn plane(&self, index: usize) -> Option<(&[u8], usize)> {
        if index > 2 {
            return None;
        }
        self.frame.plane(index)
    }

    pub(crate) fn frame(&self) -> &Frame {
        &self.frame
    }

    pub(crate) fn frame_mut(&mut self) -> &mut Frame {
        &mut self.frame
    }

    /// Display width over height, including non-square pixels and the display rotation.
    pub fn display_aspect(&self) -> f32 {
        let (num, den) = self.frame.sample_aspect_ratio();
        let pixel_aspect = if num > 0 && den > 0 {
            num as f32 / den as f32
        } else {
            1.0
        };
        let aspect = self.width() as f32 * pixel_aspect / self.height().max(1) as f32;
        if self.rotation.is_sideways() {
            1.0 / aspect
        } else {
            aspect
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PlaneLayout;

    #[test]
    fn plane_layout_reads_bit_depth_and_subsampling_from_format_names() {
        let layout = |name| {
            PlaneLayout::from_format_name(name)
                .map(|l| (l.bits, l.chroma_shift_x, l.chroma_shift_y))
        };
        assert_eq!(layout("yuv420p"), Some((8, 1, 1)));
        assert_eq!(layout("yuv422p10le"), Some((10, 1, 0)));
        assert_eq!(layout("yuv444p12le"), Some((12, 0, 0)));
        assert_eq!(layout("nv12"), Some((8, 1, 1)));
        assert!(
            PlaneLayout::from_format_name("p010le")
                .unwrap()
                .interleaved_chroma
        );
        assert_eq!(layout("rgb24"), None);
    }
}
