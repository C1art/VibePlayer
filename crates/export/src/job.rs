use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crossbeam::channel::{self, Receiver};
use cut::{CutPlan, Ticks, Timebase};

use crate::ExportError;
use crate::naming::temp_path;
use crate::remux::{AudioChanges, remux};
use crate::verify::verify;

#[derive(Clone, Debug)]
pub struct ExportRequest {
    pub source: PathBuf,
    pub output: PathBuf,
    pub plan: CutPlan,
    pub audio: AudioChanges,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportResult {
    pub output: PathBuf,
    pub reencoded_audio: bool,
    /// Video frames re-encoded at the start by a smart cut.
    pub reencoded_frames: usize,
    /// Of the written file, in microseconds.
    pub duration: Ticks,
}

pub enum Progress {
    Working(f32),
    Done(Result<ExportResult, ExportError>),
}

/// An export running on its own thread.
pub struct ExportJob {
    progress: Receiver<Progress>,
    cancel: Arc<AtomicBool>,
    fraction: f32,
    worker: Option<thread::JoinHandle<()>>,
}

impl ExportJob {
    pub fn start(request: ExportRequest) -> ExportJob {
        let (sender, progress) = channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let spawned = thread::Builder::new().name("export".into()).spawn({
            let sender = sender.clone();
            move || {
                let report = |fraction| {
                    let _ = sender.send(Progress::Working(fraction));
                };
                let result = run(&request, &worker_cancel, &report);
                let _ = sender.send(Progress::Done(result));
            }
        });
        let worker = match spawned {
            Ok(worker) => Some(worker),
            Err(error) => {
                let _ = sender.send(Progress::Done(Err(ExportError::Io(error))));
                None
            }
        };
        ExportJob {
            progress,
            cancel,
            fraction: 0.0,
            worker,
        }
    }

    /// The latest progress, or the result once finished.
    pub fn poll(&mut self) -> Option<Result<ExportResult, ExportError>> {
        while let Ok(update) = self.progress.try_recv() {
            match update {
                Progress::Working(fraction) => self.fraction = fraction,
                Progress::Done(result) => return Some(result),
            }
        }
        None
    }

    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Dropping a running job cancels it and waits for the worker to remove its temp file, which
/// takes no longer than one packet.
impl Drop for ExportJob {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Exports synchronously; [`ExportJob`] runs this on a worker.
pub fn run(
    request: &ExportRequest,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32),
) -> Result<ExportResult, ExportError> {
    if same_file(&request.source, &request.output) {
        return Err(ExportError::WouldOverwriteSource(request.output.clone()));
    }
    if request.output.exists() {
        return Err(ExportError::OutputExists(request.output.clone()));
    }
    let temp = temp_path(&request.output);
    let result = remux(
        &request.source,
        &temp,
        &request.plan,
        request.audio,
        cancel,
        progress,
    )
    .and_then(|remuxed| {
        let expected = request
            .plan
            .duration()
            .rescale(request.plan.timebase, Timebase::MICROS);
        let duration = verify(&temp, remuxed.streams, expected)?;
        if request.output.exists() {
            return Err(ExportError::OutputExists(request.output.clone()));
        }
        fs::rename(&temp, &request.output)?;
        Ok(ExportResult {
            output: request.output.clone(),
            reencoded_audio: remuxed.reencoded_audio,
            reencoded_frames: remuxed.reencoded_frames,
            duration,
        })
    });
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
