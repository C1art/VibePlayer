use semver::Version;

/// Automatic checks happen at most once a day.
pub const CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;

pub fn is_check_due(last_check: Option<u64>, now: u64) -> bool {
    last_check.is_none_or(|last| now.saturating_sub(last) >= CHECK_INTERVAL_SECONDS || last > now)
}

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
    fn checks_run_once_a_day() {
        assert!(is_check_due(None, 1_000));
        assert!(!is_check_due(
            Some(1_000),
            1_000 + CHECK_INTERVAL_SECONDS - 1
        ));
        assert!(is_check_due(Some(1_000), 1_000 + CHECK_INTERVAL_SECONDS));
    }

    #[test]
    fn a_clock_set_backwards_does_not_block_checks() {
        assert!(is_check_due(Some(5_000), 1_000));
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
