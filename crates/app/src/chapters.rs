//! A file's chapters on the player's timeline, and stepping between them.

use cut::{Ticks, Timebase};
use media::MediaInfo;

/// Going to the previous chapter within this long of a chapter's start goes to the one before;
/// later, it goes back to the start of the current one.
const RESTART_WINDOW: Ticks = Ticks(3_000_000);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChapterMark {
    /// Microseconds from the start of the file.
    pub start: Ticks,
    pub title: Option<String>,
}

/// The chapters of `info`, in order, on the player's timeline.
pub fn marks(info: &MediaInfo) -> Vec<ChapterMark> {
    let mut marks: Vec<ChapterMark> = info
        .chapters
        .iter()
        .map(|chapter| ChapterMark {
            start: (chapter.start.rescale(chapter.timebase, Timebase::MICROS) - info.start_time)
                .max(Ticks::ZERO),
            title: chapter
                .metadata
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("title"))
                .map(|(_, title)| title.trim().to_owned())
                .filter(|title| !title.is_empty()),
        })
        .collect();
    marks.sort_by_key(|mark| mark.start);
    marks
}

/// The index of the chapter playing at `position`.
pub fn current(marks: &[ChapterMark], position: Ticks) -> Option<usize> {
    marks
        .partition_point(|mark| mark.start <= position)
        .checked_sub(1)
}

/// The chapter "next" goes to: the first that starts after `position`.
pub fn next(marks: &[ChapterMark], position: Ticks) -> Option<usize> {
    let index = marks.partition_point(|mark| mark.start <= position);
    (index < marks.len()).then_some(index)
}

/// The chapter "previous" goes to: back to the start of this one, unless it only just began.
pub fn previous(marks: &[ChapterMark], position: Ticks) -> Option<usize> {
    let index = current(marks, position)?;
    if position - marks[index].start > RESTART_WINDOW {
        Some(index)
    } else {
        index.checked_sub(1)
    }
}

/// How the OSD names a chapter.
pub fn label(marks: &[ChapterMark], index: usize) -> String {
    let number = index + 1;
    match marks.get(index).and_then(|mark| mark.title.as_deref()) {
        Some(title) => format!("Chapter {number}: {title}"),
        None => format!("Chapter {number}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(starts: &[i64]) -> Vec<ChapterMark> {
        starts
            .iter()
            .map(|&seconds| ChapterMark {
                start: Ticks(seconds * 1_000_000),
                title: None,
            })
            .collect()
    }

    const S: i64 = 1_000_000;

    #[test]
    fn next_skips_to_the_following_chapter() {
        let marks = at(&[0, 60, 120]);
        assert_eq!(next(&marks, Ticks(30 * S)), Some(1));
        assert_eq!(next(&marks, Ticks(60 * S)), Some(2));
        assert_eq!(next(&marks, Ticks(130 * S)), None);
    }

    #[test]
    fn previous_restarts_the_chapter_unless_it_just_began() {
        let marks = at(&[0, 60, 120]);
        assert_eq!(previous(&marks, Ticks(90 * S)), Some(1));
        assert_eq!(previous(&marks, Ticks(61 * S)), Some(0));
        assert_eq!(previous(&marks, Ticks(S)), None);
    }

    #[test]
    fn chapter_times_are_moved_onto_the_player_timeline() {
        let info = MediaInfo {
            path: "x.mkv".into(),
            file_size: None,
            format: String::new(),
            format_long: String::new(),
            duration: None,
            start_time: Ticks(2 * S),
            bit_rate: None,
            metadata: Vec::new(),
            chapters: vec![media::Chapter {
                timebase: Timebase::MILLIS,
                start: Ticks(62_000),
                end: Ticks(90_000),
                metadata: vec![("title".into(), " Intro ".into())],
            }],
            streams: Vec::new(),
        };
        let marks = marks(&info);
        assert_eq!(marks[0].start, Ticks(60 * S));
        assert_eq!(label(&marks, 0), "Chapter 1: Intro");
    }
}
