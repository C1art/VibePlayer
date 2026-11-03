//! Decoded pictures that stay on the GPU: the decoder copies each one into a texture another
//! graphics API can open, so the renderer can show it without a trip through system memory.
//! Only Windows (D3D11 decoding, D3D12 rendering) produces these; elsewhere they never occur.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Asks the decoder to keep pictures on the GPU the renderer uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuSharing {
    /// The renderer's adapter, so decoding happens on the same GPU (Windows LUID).
    pub adapter_luid: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceFormat {
    /// 8-bit luma plane, then interleaved half-size chroma.
    Nv12,
    /// The same with 16-bit samples, the top 10 bits used.
    P010,
}

/// A set of same-sized textures the decoder writes and the renderer reads, and the two fences
/// that order those accesses. Handles are Windows NT handles, valid while this lives.
pub struct SharedSurfaces {
    /// Different for every set, so a renderer knows when to open them again.
    pub id: u64,
    pub format: SurfaceFormat,
    pub width: u32,
    pub height: u32,
    pub textures: Vec<isize>,
    /// Reaches a picture's `ready_at` once the decoder's copy into it is done.
    pub ready_fence: isize,
    /// Advanced by the renderer once the GPU has finished reading a texture.
    pub done_fence: isize,
    pub(crate) slots: Vec<Slot>,
    /// Set by a renderer that can't open these textures; the decoder then stops using them.
    pub(crate) rejected: AtomicBool,
    /// The platform objects behind the handles, kept alive as long as any picture is.
    #[allow(dead_code)]
    pub(crate) owner: Option<Box<dyn std::any::Any + Send + Sync>>,
}

/// One texture's bookkeeping.
#[derive(Default)]
pub(crate) struct Slot {
    pub in_use: AtomicBool,
    /// The texture may be written again once `done_fence` reaches this.
    pub done_after: AtomicU64,
}

impl SharedSurfaces {
    /// Tells the decoder these textures can't be shown, so it sends pictures through memory.
    pub fn reject(&self) {
        self.rejected.store(true, Ordering::Release);
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn is_rejected(&self) -> bool {
        self.rejected.load(Ordering::Acquire)
    }

    /// A free slot whose last reader is done according to `completed` (the done fence's
    /// value), now marked in use.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn claim(self: &Arc<Self>, completed: u64) -> Option<SlotLease> {
        self.slots.iter().enumerate().find_map(|(index, slot)| {
            let finished = slot.done_after.load(Ordering::Acquire) <= completed;
            (finished
                && slot
                    .in_use
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok())
            .then(|| SlotLease {
                surfaces: Arc::clone(self),
                index,
            })
        })
    }
}

/// Holds a texture for one picture; frees it when the last holder lets go.
pub struct SlotLease {
    surfaces: Arc<SharedSurfaces>,
    index: usize,
}

impl SlotLease {
    pub fn surfaces(&self) -> &Arc<SharedSurfaces> {
        &self.surfaces
    }

    pub fn index(&self) -> usize {
        self.index
    }

    /// The renderer is done with the texture once the done fence reaches `value`.
    pub fn done_after(&self, value: u64) {
        self.surfaces.slots[self.index]
            .done_after
            .fetch_max(value, Ordering::AcqRel);
    }
}

impl Drop for SlotLease {
    fn drop(&mut self) {
        self.surfaces.slots[self.index]
            .in_use
            .store(false, Ordering::Release);
    }
}

/// A decoded picture in one of the shared textures.
#[derive(Clone)]
pub struct GpuPicture {
    pub lease: Arc<SlotLease>,
    /// Wait for the ready fence to reach this before reading.
    pub ready_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surfaces(slots: usize) -> Arc<SharedSurfaces> {
        Arc::new(SharedSurfaces {
            id: 1,
            format: SurfaceFormat::Nv12,
            width: 16,
            height: 16,
            textures: vec![0; slots],
            ready_fence: 0,
            done_fence: 0,
            slots: (0..slots).map(|_| Slot::default()).collect(),
            rejected: AtomicBool::new(false),
            owner: None,
        })
    }

    #[test]
    fn a_texture_is_reused_only_after_its_lease_ends_and_the_gpu_is_done() {
        let surfaces = surfaces(1);
        let lease = surfaces.claim(0).unwrap();
        assert!(surfaces.claim(0).is_none());
        lease.done_after(5);
        drop(lease);
        assert!(surfaces.claim(4).is_none());
        assert!(surfaces.claim(5).is_some());
    }
}
