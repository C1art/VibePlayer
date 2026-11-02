//! Release check against GitHub, version comparison, ask-later and skipped-version rules,
//! installer download.

mod check;
mod download;
mod error;
mod policy;
mod release;

pub use check::{UpdateCheck, check_now};
pub use download::{Download, DownloadState};
pub use error::UpdateError;
pub use policy::{RECHECK_INTERVAL, should_offer};
pub use release::{Asset, Release};

/// Where releases are published.
pub const REPOSITORY: &str = "c1art/vibeplayer";

/// This build's version.
pub fn current_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| semver::Version::new(0, 0, 0))
}
