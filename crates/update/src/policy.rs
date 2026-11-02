use semver::Version;

/// While the app stays open, it checks again this often.
pub const RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// A release is offered when it's newer than this build and wasn't skipped. A skip only
/// covers that exact version, so a later release is offered again.
pub fn should_offer(current: &Version, latest: &Version, skipped: Option<&str>) -> bool {
    let skipped = skipped.and_then(|v| Version::parse(v).ok());
    latest > current && skipped.as_ref() != Some(latest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn only_newer_versions_are_offered() {
        assert!(should_offer(&v("1.0.0"), &v("1.0.1"), None));
        assert!(!should_offer(&v("1.0.1"), &v("1.0.1"), None));
        assert!(!should_offer(&v("1.1.0"), &v("1.0.9"), None));
    }

    #[test]
    fn skipping_a_version_hides_only_that_version() {
        assert!(!should_offer(&v("1.0.0"), &v("1.1.0"), Some("1.1.0")));
        assert!(should_offer(&v("1.0.0"), &v("1.2.0"), Some("1.1.0")));
    }
}
