use std::thread;

use crossbeam::channel::{self, Sender};

use crate::{History, Settings, Store};

enum SaveRequest {
    Settings(Settings),
    History(History),
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
                for request in std::iter::once(request).chain(receiver.try_iter()) {
                    match request {
                        SaveRequest::Settings(settings) => latest_settings = Some(settings),
                        SaveRequest::History(history) => latest_history = Some(history),
                    }
                }
                if let Some(settings) = latest_settings {
                    let _ = store.save_settings(&settings);
                }
                if let Some(history) = latest_history {
                    let _ = store.save_history(&history);
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
}
