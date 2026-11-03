use std::path::{Path, PathBuf};

use cut::{Ticks, Timebase};
use serde::{Deserialize, Serialize};

pub(crate) const HISTORY_VERSION: u32 = 1;
const MAX_RECENTS: usize = 20;
const MAX_RESUME_ENTRIES: usize = 300;
const MAX_CUT_SESSIONS: usize = 100;
/// Files shorter than this start from the beginning every time.
const MIN_RESUMABLE_MICROS: i64 = 120_000_000;
/// Positions this close to either end aren't worth resuming.
const RESUME_MARGIN_MICROS: i64 = 15_000_000;

/// What the player remembers about files. New fields need `#[serde(default)]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub version: u32,
    pub recents: Vec<RecentFile>,
    pub resume: Vec<ResumeEntry>,
    pub cut_sessions: Vec<CutSession>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentFile {
    pub path: PathBuf,
    /// Unix seconds.
    pub opened_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeEntry {
    pub path: PathBuf,
    /// Microseconds.
    pub position: Ticks,
    pub saved_at: u64,
}

/// The editor state for one file, so reopening it restores the trim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CutSession {
    pub path: PathBuf,
    pub timebase: Timebase,
    pub start: Ticks,
    pub end: Ticks,
    /// Tenths of a decibel.
    #[serde(default)]
    pub gain: i16,
    #[serde(default)]
    pub equalizer: [i16; 3],
    pub saved_at: u64,
}

impl Default for History {
    fn default() -> History {
        History {
            version: HISTORY_VERSION,
            recents: Vec::new(),
            resume: Vec::new(),
            cut_sessions: Vec::new(),
        }
    }
}

impl History {
    pub fn add_recent(&mut self, path: &Path, now: u64) {
        self.recents.retain(|recent| recent.path != path);
        self.recents.insert(
            0,
            RecentFile {
                path: path.to_path_buf(),
                opened_at: now,
            },
        );
        self.recents.truncate(MAX_RECENTS);
    }

    /// Forgets which files were opened and where they were left off; saved cuts stay.
    pub fn clear_watch_history(&mut self) {
        self.recents.clear();
        self.resume.clear();
    }

    pub fn resume_position(&self, path: &Path) -> Option<Ticks> {
        self.resume
            .iter()
            .find(|entry| entry.path == path)
            .map(|entry| entry.position)
    }

    /// Remembers where playback stopped, or forgets it if the file was short or nearly done.
    pub fn record_position(
        &mut self,
        path: &Path,
        position: Ticks,
        duration: Option<Ticks>,
        now: u64,
    ) {
        self.resume.retain(|entry| entry.path != path);
        let Some(duration) = duration else {
            return;
        };
        let resumable = duration.0 >= MIN_RESUMABLE_MICROS
            && position.0 >= RESUME_MARGIN_MICROS
            && position.0 <= duration.0 - RESUME_MARGIN_MICROS;
        if !resumable {
            return;
        }
        self.resume.insert(
            0,
            ResumeEntry {
                path: path.to_path_buf(),
                position,
                saved_at: now,
            },
        );
        self.resume.truncate(MAX_RESUME_ENTRIES);
    }

    pub fn cut_session(&self, path: &Path) -> Option<&CutSession> {
        self.cut_sessions
            .iter()
            .find(|session| session.path == path)
    }

    pub fn save_cut_session(&mut self, session: CutSession) {
        self.cut_sessions
            .retain(|existing| existing.path != session.path);
        self.cut_sessions.insert(0, session);
        self.cut_sessions.truncate(MAX_CUT_SESSIONS);
    }

    /// Returns whether there was a session to forget.
    pub fn forget_cut_session(&mut self, path: &Path) -> bool {
        let before = self.cut_sessions.len();
        self.cut_sessions.retain(|session| session.path != path);
        self.cut_sessions.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Ticks = Ticks(3_600_000_000);

    #[test]
    fn recents_move_to_front_without_duplicates() {
        let mut history = History::default();
        history.add_recent(Path::new("a.mp4"), 1);
        history.add_recent(Path::new("b.mp4"), 2);
        history.add_recent(Path::new("a.mp4"), 3);
        let paths: Vec<_> = history.recents.iter().map(|r| r.path.clone()).collect();
        assert_eq!(paths, [PathBuf::from("a.mp4"), PathBuf::from("b.mp4")]);
    }

    #[test]
    fn positions_near_the_ends_or_in_short_files_are_not_resumed() {
        let mut history = History::default();
        let path = Path::new("movie.mkv");
        history.record_position(path, Ticks(5_000_000), Some(HOUR), 1);
        assert_eq!(history.resume_position(path), None);
        history.record_position(path, HOUR - Ticks(1_000_000), Some(HOUR), 1);
        assert_eq!(history.resume_position(path), None);
        history.record_position(path, Ticks(30_000_000), Some(Ticks(60_000_000)), 1);
        assert_eq!(history.resume_position(path), None);
        history.record_position(path, Ticks(600_000_000), Some(HOUR), 1);
        assert_eq!(history.resume_position(path), Some(Ticks(600_000_000)));
    }

    #[test]
    fn finishing_a_file_forgets_its_resume_position() {
        let mut history = History::default();
        let path = Path::new("movie.mkv");
        history.record_position(path, Ticks(600_000_000), Some(HOUR), 1);
        history.record_position(path, HOUR, Some(HOUR), 2);
        assert_eq!(history.resume_position(path), None);
    }
}
