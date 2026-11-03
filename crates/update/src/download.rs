use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crossbeam::channel::{self, Receiver};

use crate::check::{Patience, agent};
use crate::{Asset, UpdateError};

pub enum DownloadState {
    /// Bytes received so far, and the total if known.
    Receiving(u64, Option<u64>),
    Finished(Result<PathBuf, UpdateError>),
}

/// An installer download running in the background.
pub struct Download {
    updates: Receiver<DownloadState>,
    cancel: Arc<AtomicBool>,
    received: (u64, Option<u64>),
}

impl Download {
    /// Downloads `asset` into the temp directory.
    pub fn start(asset: Asset) -> Download {
        let (sender, updates) = channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let _ = thread::Builder::new()
            .name("update download".into())
            .spawn(move || {
                let result = fetch(&asset, &worker_cancel, &|received| {
                    let _ = sender.send(DownloadState::Receiving(received, Some(asset.size)));
                });
                let _ = sender.send(DownloadState::Finished(result));
            });
        Download {
            updates,
            cancel,
            received: (0, None),
        }
    }

    /// Bytes received and expected, for a progress bar.
    pub fn progress(&self) -> (u64, Option<u64>) {
        self.received
    }

    /// `Some` with the installer's path once finished.
    pub fn poll(&mut self) -> Option<Result<PathBuf, UpdateError>> {
        while let Ok(update) = self.updates.try_recv() {
            match update {
                DownloadState::Receiving(received, total) => self.received = (received, total),
                DownloadState::Finished(result) => return Some(result),
            }
        }
        None
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn fetch(
    asset: &Asset,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<PathBuf, UpdateError> {
    let file_name = PathBuf::from(&asset.name)
        .file_name()
        .map(PathBuf::from)
        .ok_or(UpdateError::NoInstaller)?;
    let path = std::env::temp_dir().join(file_name);
    let partial = path.with_extension("partial");
    let downloaded = download_to(&partial, asset, cancel, progress)
        .and_then(|()| std::fs::rename(&partial, &path).map_err(UpdateError::Io));
    if downloaded.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    downloaded.map(|()| path)
}

fn download_to(
    partial: &Path,
    asset: &Asset,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<(), UpdateError> {
    let response = agent(Patience::Download).get(&asset.url).call()?;
    let mut reader = response.into_body().into_reader();
    let mut file = File::create(partial)?;
    let mut buffer = vec![0; 64 * 1024];
    let mut received = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(UpdateError::Cancelled);
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        received += read as u64;
        progress(received);
    }
    file.flush()?;
    if asset.size > 0 && received != asset.size {
        return Err(UpdateError::BadResponse(format!(
            "downloaded {received} of {} bytes",
            asset.size
        )));
    }
    Ok(())
}
