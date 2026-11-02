use std::thread;
use std::time::Duration;

use crossbeam::channel::{self, Receiver};

use crate::{REPOSITORY, Release, UpdateError};

pub(crate) fn agent() -> ureq::Agent {
    // The system's trust store, so networks that inspect TLS with their own root still work.
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent(concat!("VibePlayer/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Fetches the latest release synchronously. [`UpdateCheck`] runs this on a worker.
pub fn check_now() -> Result<Release, UpdateError> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let mut response = agent()
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()?;
    let json = response.body_mut().read_to_string()?;
    Release::from_github_json(&json)
}

/// A release check running in the background.
pub struct UpdateCheck {
    result: Receiver<Result<Release, UpdateError>>,
}

impl UpdateCheck {
    pub fn start() -> UpdateCheck {
        let (sender, result) = channel::bounded(1);
        let spawned = thread::Builder::new()
            .name("update check".into())
            .spawn(move || {
                let _ = sender.send(check_now());
            });
        if let Err(error) = spawned {
            let (sender, failed) = channel::bounded(1);
            let _ = sender.send(Err(UpdateError::Io(error)));
            return UpdateCheck { result: failed };
        }
        UpdateCheck { result }
    }

    /// `Some` once the check finished.
    pub fn poll(&self) -> Option<Result<Release, UpdateError>> {
        self.result.try_recv().ok()
    }
}
