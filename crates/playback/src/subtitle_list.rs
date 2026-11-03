use cut::Ticks;
use media::{Subtitle, SubtitleContent};

const MAX_SUBTITLES: usize = 256;

/// Decoded subtitle events for the current seek generation, in start order.
#[derive(Default)]
pub(crate) struct SubtitleList {
    events: Vec<Subtitle>,
}

impl SubtitleList {
    pub fn add(&mut self, subtitle: Subtitle) {
        let at = self
            .events
            .partition_point(|existing| existing.start <= subtitle.start);
        self.events.insert(at, subtitle);
        if self.events.len() > MAX_SUBTITLES {
            self.events.remove(0);
        }
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// The latest-starting event showing at `position`; an earlier one still on screen shows
    /// once a shorter one over it ends. Events without an end last until the next one.
    pub fn active(&self, position: Ticks) -> Option<&Subtitle> {
        let started = self
            .events
            .partition_point(|subtitle| subtitle.start <= position);
        // A clear event wipes the screen: nothing from before it shows again.
        let visible = (0..started).rev().find(|&index| {
            let subtitle = &self.events[index];
            let next_start = self.events.get(index + 1).map(|next| next.start);
            subtitle.content == SubtitleContent::Clear
                || subtitle.end.or(next_start).is_none_or(|end| position < end)
        })?;
        let subtitle = &self.events[visible];
        (subtitle.content != SubtitleContent::Clear).then_some(subtitle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(start: i64, end: Option<i64>, text: &str) -> Subtitle {
        Subtitle {
            start: Ticks(start),
            end: end.map(Ticks),
            content: SubtitleContent::Text(text.into()),
        }
    }

    #[test]
    fn a_longer_event_shows_again_after_a_shorter_one_inside_it() {
        let mut list = SubtitleList::default();
        list.add(text(10, Some(20), "long"));
        list.add(text(12, Some(14), "short"));
        let shown = |at| match &list.active(Ticks(at))?.content {
            SubtitleContent::Text(text) => Some(text.clone()),
            _ => None,
        };
        assert_eq!(shown(13).as_deref(), Some("short"));
        assert_eq!(shown(15).as_deref(), Some("long"));
        assert_eq!(shown(20), None);
    }

    #[test]
    fn a_clear_event_hides_earlier_ones_for_good() {
        let mut list = SubtitleList::default();
        list.add(text(10, Some(20), "long"));
        list.add(Subtitle {
            start: Ticks(12),
            end: Some(Ticks(13)),
            content: SubtitleContent::Clear,
        });
        assert!(list.active(Ticks(15)).is_none());
    }

    #[test]
    fn events_show_between_start_and_end() {
        let mut list = SubtitleList::default();
        list.add(text(100, Some(200), "a"));
        assert!(list.active(Ticks(99)).is_none());
        assert!(list.active(Ticks(150)).is_some());
        assert!(list.active(Ticks(200)).is_none());
    }

    #[test]
    fn open_ended_events_last_until_the_next_one() {
        let mut list = SubtitleList::default();
        list.add(text(300, None, "b"));
        list.add(text(100, None, "a"));
        let shown = |at| match &list.active(Ticks(at))?.content {
            SubtitleContent::Text(t) => Some(t.clone()),
            _ => None,
        };
        assert_eq!(shown(250).as_deref(), Some("a"));
        assert_eq!(shown(350).as_deref(), Some("b"));
    }

    #[test]
    fn clear_events_hide_what_was_showing() {
        let mut list = SubtitleList::default();
        list.add(text(100, None, "a"));
        list.add(Subtitle {
            start: Ticks(200),
            end: None,
            content: SubtitleContent::Clear,
        });
        assert!(list.active(Ticks(250)).is_none());
    }
}
