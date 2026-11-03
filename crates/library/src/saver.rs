use std::thread;

use crossbeam::channel::{self, Sender};

use crate::{History, Settings, Store};

enum SaveRequest {
    Settings(Settings),
    History(History),
    /// Answered once everything queued before it is on disk.
    Flush(Sender<()>),
}

/// Writes settings and history on a background thread so the UI never waits on disk.
#[derive(Clone)]
pub struct Saver {
    requests: Sender<SaveRequest>,
}

impl Saver {
    pub fn spawn(store: Store) -> Saver {
        let (requests, receiver) = channel::unbounded::<SaveRequest>();
        let _ = thread::Builder::new().name("saver".into()).spawn(move || {
            while let Ok(request) = receiver.recv() {
                // Only the newest pending snapshot of each kind matters.
                let mut latest_settings = None;
                let mut latest_history = None;
                let mut flushes = Vec::new();
                for request in std::iter::once(request).chain(receiver.try_iter()) {
                    match request {
                        SaveRequest::Settings(settings) => latest_settings = Some(settings),
                        SaveRequest::History(history) => latest_history = Some(history),
                        SaveRequest::Flush(done) => flushes.push(done),
                    }
                }
                if let Some(settings) = latest_settings {
                    let _ = store.save_settings(&settings);
                }
                if let Some(history) = latest_history {
                    let _ = store.save_history(&history);
                }
                for done in flushes {
                    let _ = done.send(());
                }
            }
        });
        Saver { requests }
    }

    pub fn save_settings(&self, settings: &Settings) {
        let _ = self.requests.send(SaveRequest::Settings(settings.clone()));
    }

    pub fn save_history(&self, history: &History) {
        let _ = self.requests.send(SaveRequest::History(history.clone()));
    }

    /// Blocks until every save queued so far is written. Only for shutdown: the saver is the
    /// one writer of each file, so a direct save could race it.
    pub fn flush(&self) {
        let (done, finished) = channel::bounded(1);
        if self.requests.send(SaveRequest::Flush(done)).is_ok() {
            let _ = finished.recv();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_returns_once_the_latest_queued_snapshot_is_on_disk() {
        let dir = std::env::temp_dir().join(format!("vibeplayer-saver-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir);
        let saver = Saver::spawn(store.clone());
        for volume in 1..=50 {
            saver.save_settings(&Settings {
                volume,
                ..Settings::default()
            });
        }
        saver.flush();
        assert_eq!(store.load_settings().volume, 50);
    }
}
