//! Runs a cut plan through `media`: stream copy, audio-only re-encode for volume/EQ,
//! verification. Writes to a temporary file, probes it, and only then renames it into place.

mod error;
mod job;
mod nal;
mod naming;
mod remux;
mod smart;
mod verify;

pub use error::ExportError;
pub use job::{ExportJob, ExportRequest, ExportResult, Progress, run};
pub use naming::{suggested_output, unique_path};
pub use remux::AudioChanges;
pub use smart::smart_cut_support;
