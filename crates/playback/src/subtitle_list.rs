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

    /// The event showing at `position`. Events without an end last until the next one.
    pub fn active(&self, position: Ticks) -> Option<&Subtitle> {
        let index = self
            .events
            .partition_point(|subtitle| subtitle.start <= position);
        let subtitle = self.events[..index].last()?;
        let next_start = self.events.get(index).map(|next| next.start);
        let end = subtitle.end.or(next_start);
        let visible = end.is_none_or(|end| position < end);
        (visible && subtitle.content != SubtitleContent::Clear).then_some(subtitle)
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
