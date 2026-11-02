//! Keeps one VibePlayer window: a second launch hands its file to the running one and exits.
//!
//! Requests travel over a per-user local channel (a named pipe on Windows, a Unix socket
//! elsewhere) as one UTF-8 line holding the path, or an empty line to just bring the window
//! forward.

use std::path::{Path, PathBuf};
use std::thread;

use crossbeam::channel::{self, Receiver, Sender};
use eframe::egui;

/// The longest request we read; a path never comes close.
pub(super) const MAX_REQUEST: u64 = 64 * 1024;

pub enum Claim {
    /// This is the first instance. `None` if listening failed; it then just runs alone.
    Primary(Option<Listener>),
    /// Another instance took the request; this process should exit.
    Forwarded,
}

/// Becomes the primary instance, or forwards `file` to the existing one.
pub fn claim(file: Option<&Path>) -> Claim {
    let message = match file {
        Some(path) => match path.to_str() {
            Some(text) => text.to_owned(),
            // A path that can't travel as UTF-8 opens in its own window instead.
            None => return Claim::Primary(None),
        },
        None => String::new(),
    };
    platform::claim(&message)
}

/// Accepts requests from later launches.
pub struct Listener {
    inner: platform::Listener,
}

impl Listener {
    #[cfg(windows)]
    pub(super) fn windows(inner: platform::Listener) -> Listener {
        Listener { inner }
    }

    /// Starts listening on a worker. Each received path (empty for "just focus") arrives on the
    /// returned channel, and the UI is woken to handle it.
    pub fn start(self, ctx: egui::Context) -> Receiver<PathBuf> {
        let (sender, receiver) = channel::unbounded();
        let inner = self.inner;
        let _ = thread::Builder::new()
            .name("single instance".into())
            .spawn(move || platform::serve(inner, &Deliver { sender, ctx }));
        receiver
    }
}

pub(super) struct Deliver {
    sender: Sender<PathBuf>,
    ctx: egui::Context,
}

impl Deliver {
    /// Returns `false` once the app is gone.
    pub(super) fn deliver(&self, request: &[u8]) -> bool {
        let text = String::from_utf8_lossy(request);
        let path = PathBuf::from(text.trim_end_matches(['\r', '\n']));
        let delivered = self.sender.send(path).is_ok();
        self.ctx.request_repaint();
        delivered
    }
}

#[cfg(windows)]
use super::windows::single_instance as platform;

#[cfg(unix)]
mod platform {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{Claim, Deliver, MAX_REQUEST};

    pub type Listener = UnixListener;

    fn socket_path() -> Option<PathBuf> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .or_else(library::data_dir)?;
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir.join("vibeplayer.sock"))
    }

    pub fn claim(message: &str) -> Claim {
        let Some(path) = socket_path() else {
            return Claim::Primary(None);
        };
        if let Ok(mut stream) = UnixStream::connect(&path)
            && stream.write_all(format!("{message}\n").as_bytes()).is_ok()
        {
            return Claim::Forwarded;
        }
        // Nobody answered, so any socket file left behind is stale.
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).ok();
        Claim::Primary(listener.map(|inner| super::Listener { inner }))
    }

    pub fn serve(listener: UnixListener, deliver: &Deliver) {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut request = Vec::new();
            if stream.take(MAX_REQUEST).read_to_end(&mut request).is_ok()
                && !deliver.deliver(&request)
            {
                return;
            }
        }
    }
}
