//! A plain-text diagnostic log, `vibeplayer.log` in the data directory. Lines go to a worker
//! thread, so logging never waits on the disk.

use std::fmt::Display;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam::channel::{self, Sender};

const FILE_NAME: &str = "vibeplayer.log";
const PREVIOUS_FILE_NAME: &str = "vibeplayer.previous.log";
/// Past this size at startup, the log is moved aside and a new one begun.
const MAX_BYTES: u64 = 1_000_000;

struct Log {
    path: PathBuf,
    lines: Sender<String>,
}

static LOG: OnceLock<Log> = OnceLock::new();

/// Starts logging to `dir`. Later calls do nothing; without a call, logging does nothing.
pub fn start(dir: &Path) {
    if LOG.get().is_some() {
        return;
    }
    let path = dir.join(FILE_NAME);
    let _ = fs::create_dir_all(dir);
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
        let _ = fs::rename(&path, dir.join(PREVIOUS_FILE_NAME));
    }
    let Ok(mut file) = open(&path) else {
        return;
    };
    let (lines, receiver) = channel::unbounded::<String>();
    let spawned = thread::Builder::new().name("log".into()).spawn(move || {
        for line in receiver {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    });
    if spawned.is_ok() {
        let _ = LOG.set(Log { path, lines });
    }
}

/// The log file, once logging has started.
pub fn path() -> Option<&'static Path> {
    LOG.get().map(|log| log.path.as_path())
}

pub fn info(message: impl Display) {
    send("INFO", message);
}

pub fn warn(message: impl Display) {
    send("WARN", message);
}

pub fn error(message: impl Display) {
    send("ERROR", message);
}

/// Appends at once, for a panic: the process may end before the worker gets to it.
pub fn write_now(message: impl Display) {
    if let Some(log) = LOG.get()
        && let Ok(mut file) = open(&log.path)
    {
        let _ = file.write_all(line("PANIC", message).as_bytes());
    }
}

fn send(level: &str, message: impl Display) {
    if let Some(log) = LOG.get() {
        let _ = log.lines.send(line(level, message));
    }
}

fn open(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn line(level: &str, message: impl Display) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{} {level} {message}\n", utc_timestamp(now.as_millis()))
}

/// `YYYY-MM-DD HH:MM:SS.mmm` in UTC, from milliseconds since the Unix epoch.
fn utc_timestamp(millis: u128) -> String {
    let millis = u64::try_from(millis).unwrap_or(u64::MAX);
    let seconds = millis / 1_000;
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let of_day = seconds % 86_400;
    // Days to civil date (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03}",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60,
        millis % 1_000
    )
}

#[cfg(test)]
mod tests {
    use super::utc_timestamp;

    #[test]
    fn timestamps_are_utc_calendar_dates() {
        assert_eq!(utc_timestamp(0), "1970-01-01 00:00:00.000");
        // 2024-02-29 23:59:59.123, a leap day.
        assert_eq!(utc_timestamp(1_709_251_199_123), "2024-02-29 23:59:59.123");
        assert_eq!(utc_timestamp(1_798_761_600_000), "2027-01-01 00:00:00.000");
    }
}
