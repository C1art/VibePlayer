//! Keeping hardware-decoded pictures on the GPU (Windows): each is copied into a texture the
//! renderer opens, instead of being downloaded to system memory.

use super::VideoDecoder;
use crate::MediaError;
use crate::ffi::{Frame, SurfacePool};
use crate::gpu::SurfaceFormat;
use crate::video_frame::{PlaneLayout, VideoFrame};

/// The decoder's shared textures, once a picture has needed them.
#[derive(Default)]
pub(super) struct Sharing {
    pool: Option<SurfacePool>,
    /// Setting up sharing failed; pictures are downloaded from now on.
    failed: bool,
}

impl VideoDecoder {
    /// The decoded picture, kept on the GPU if it can be: `None` when it must go through the
    /// usual download and filter instead, `Some(None)` when it was a replayed one to skip.
    pub(super) fn share_decoded(&mut self) -> Result<Option<Option<VideoFrame>>, MediaError> {
        if self.options.gpu.is_none() || self.sharing.failed || !self.decoded.is_hardware() {
            return Ok(None);
        }
        let deinterlace =
            self.options.deinterlace && (self.stream_interlaced || self.decoded.is_interlaced());
        let rejected = self
            .sharing
            .pool
            .as_ref()
            .is_some_and(SurfacePool::is_rejected);
        if deinterlace || rejected {
            // Interlaced pictures go through the deinterlacer, which holds one back; mixing
            // the two paths would reorder pictures, so stay with the filter from now on.
            self.sharing = Sharing {
                pool: None,
                failed: true,
            };
            return Ok(None);
        }
        let pool = match &mut self.sharing.pool {
            Some(pool) if pool.fits(&self.decoded) => pool,
            _ => match SurfacePool::for_frame(&self.decoded) {
                Some(pool) => self.sharing.pool.insert(pool),
                None => {
                    self.sharing.failed = true;
                    return Ok(None);
                }
            },
        };
        let format = pool.format();
        if format == SurfaceFormat::P010 && !self.options.high_bit_depth {
            return Ok(None);
        }
        // Every texture still in use: download this one rather than wait.
        let Some(picture) = pool.copy(&self.decoded) else {
            return Ok(None);
        };
        let name = match format {
            SurfaceFormat::Nv12 => "nv12",
            SurfaceFormat::P010 => "p010le",
        };
        let layout = PlaneLayout::from_format_name(name)
            .ok_or_else(|| MediaError::Unsupported(name.into()))?;
        let frame = Frame::properties_of(&self.decoded)?;
        let timebase = self.params.timebase();
        Ok(Some(self.complete(frame, timebase, layout, Some(picture))))
    }
}
