use std::collections::VecDeque;

use cut::Ticks;

use crate::Generation;

pub trait Timed {
    fn pts(&self) -> Ticks;
}

/// Decoded frames waiting to be shown, and the one on screen.
pub struct FrameQueue<F> {
    pending: VecDeque<F>,
    current: Option<F>,
    generation: Generation,
    awaiting_first: bool,
}

/// What changed on screen after an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    Unchanged,
    /// A new frame is current; `dropped` late frames were skipped to get there.
    NewFrame {
        dropped: usize,
    },
}

impl<F: Timed> FrameQueue<F> {
    pub fn new() -> FrameQueue<F> {
        FrameQueue {
            pending: VecDeque::new(),
            current: None,
            generation: Generation::default(),
            awaiting_first: true,
        }
    }

    /// Starts a new generation: pending frames are discarded, and the first frame of the new
    /// generation will be shown as soon as it arrives. The current frame stays on screen.
    pub fn reset(&mut self, generation: Generation) {
        self.generation = generation;
        self.pending.clear();
        self.awaiting_first = true;
    }

    /// Accepts a frame only if it belongs to the current generation.
    pub fn push(&mut self, generation: Generation, frame: F) -> bool {
        if generation != self.generation {
            return false;
        }
        self.pending.push_back(frame);
        true
    }

    pub fn is_awaiting_first(&self) -> bool {
        self.awaiting_first
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn current(&self) -> Option<&F> {
        self.current.as_ref()
    }

    pub fn next_pts(&self) -> Option<Ticks> {
        self.pending.front().map(Timed::pts)
    }

    /// Shows the first frame after a reset, whatever its time.
    pub fn show_first(&mut self) -> Shown {
        if !self.awaiting_first {
            return Shown::Unchanged;
        }
        match self.pending.pop_front() {
            Some(frame) => {
                self.current = Some(frame);
                self.awaiting_first = false;
                Shown::NewFrame { dropped: 0 }
            }
            None => Shown::Unchanged,
        }
    }

    /// Shows the latest frame due at `position`, skipping frames that are already late.
    pub fn advance_to(&mut self, position: Ticks) -> Shown {
        if self.awaiting_first {
            return self.show_first();
        }
        let mut dropped = 0;
        let mut shown = None;
        while self.pending.front().is_some_and(|f| f.pts() <= position) {
            if shown.is_some() {
                dropped += 1;
            }
            shown = self.pending.pop_front();
        }
        match shown {
            Some(frame) => {
                self.current = Some(frame);
                Shown::NewFrame { dropped }
            }
            None => Shown::Unchanged,
        }
    }

    /// Shows the next frame regardless of time, for stepping while paused.
    pub fn step(&mut self) -> Shown {
        match self.pending.pop_front() {
            Some(frame) => {
                self.current = Some(frame);
                self.awaiting_first = false;
                Shown::NewFrame { dropped: 0 }
            }
            None => Shown::Unchanged,
        }
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.current = None;
        self.awaiting_first = true;
    }
}

impl<F: Timed> Default for FrameQueue<F> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Fake(i64);

    impl Timed for Fake {
        fn pts(&self) -> Ticks {
            Ticks(self.0)
        }
    }

    fn queue_with(frames: &[i64]) -> FrameQueue<Fake> {
        let mut queue = FrameQueue::new();
        queue.reset(Generation(1));
        for &pts in frames {
            queue.push(Generation(1), Fake(pts));
        }
        queue
    }

    #[test]
    fn frames_from_a_stale_generation_are_rejected() {
        let mut queue = queue_with(&[]);
        queue.reset(Generation(2));
        assert!(!queue.push(Generation(1), Fake(0)));
        assert!(queue.push(Generation(2), Fake(0)));
    }

    #[test]
    fn first_frame_after_a_reset_shows_immediately() {
        let mut queue = queue_with(&[5_000_000]);
        assert_eq!(queue.advance_to(Ticks(0)), Shown::NewFrame { dropped: 0 });
        assert_eq!(queue.current(), Some(&Fake(5_000_000)));
    }

    #[test]
    fn late_frames_are_dropped_to_catch_up() {
        let mut queue = queue_with(&[0, 40, 80, 120, 160]);
        queue.show_first();
        assert_eq!(queue.advance_to(Ticks(125)), Shown::NewFrame { dropped: 2 });
        assert_eq!(queue.current(), Some(&Fake(120)));
        assert_eq!(queue.next_pts(), Some(Ticks(160)));
    }

    #[test]
    fn early_frames_wait_for_their_time() {
        let mut queue = queue_with(&[0, 40]);
        queue.show_first();
        assert_eq!(queue.advance_to(Ticks(39)), Shown::Unchanged);
        assert_eq!(queue.current(), Some(&Fake(0)));
    }

    #[test]
    fn stepping_ignores_time() {
        let mut queue = queue_with(&[0, 40]);
        queue.show_first();
        assert_eq!(queue.step(), Shown::NewFrame { dropped: 0 });
        assert_eq!(queue.current(), Some(&Fake(40)));
        assert_eq!(queue.step(), Shown::Unchanged);
    }

    #[test]
    fn reset_keeps_the_current_frame_until_a_new_one_arrives() {
        let mut queue = queue_with(&[0]);
        queue.show_first();
        queue.reset(Generation(2));
        assert_eq!(queue.current(), Some(&Fake(0)));
        assert!(queue.is_awaiting_first());
    }
}
