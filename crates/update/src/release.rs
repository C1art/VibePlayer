use semver::Version;
use serde::Deserialize;

use crate::UpdateError;

/// A published release, as far as the updater cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub name: String,
    pub notes: String,
    pub page: String,
    pub installer: Option<Asset>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub url: String,
    pub size: u64,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

impl Release {
    /// Parses GitHub's `releases/latest` response.
    pub fn from_github_json(json: &str) -> Result<Release, UpdateError> {
        let release: GitHubRelease =
            serde_json::from_str(json).map_err(|e| UpdateError::BadResponse(e.to_string()))?;
        if release.draft || release.prerelease {
            return Err(UpdateError::BadResponse("not a stable release".into()));
        }
        let tag = release.tag_name.trim();
        let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag))
            .map_err(|e| UpdateError::BadResponse(format!("tag {tag}: {e}")))?;
        let installer = release
            .assets
            .into_iter()
            .find(|asset| asset.name.to_ascii_lowercase().ends_with("-setup.exe"));
        Ok(Release {
            name: release
                .name
                .unwrap_or_else(|| format!("VibePlayer {version}")),
            version,
            notes: release.body.unwrap_or_default(),
            page: release.html_url,
            installer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "tag_name": "v1.2.0",
        "name": "VibePlayer 1.2.0",
        "body": "Faster seeking",
        "html_url": "https://github.com/c1art/vibeplayer/releases/tag/v1.2.0",
        "draft": false,
        "prerelease": false,
        "assets": [
            {"name": "notes.txt", "browser_download_url": "https://example/notes.txt", "size": 10},
            {"name": "VibePlayer-1.2.0-setup.exe", "browser_download_url": "https://example/setup.exe", "size": 9000000}
        ]
    }"#;

    #[test]
    fn github_releases_parse_with_their_installer() {
        let release = Release::from_github_json(SAMPLE).unwrap();
        assert_eq!(release.version, Version::new(1, 2, 0));
        assert_eq!(release.installer.unwrap().url, "https://example/setup.exe");
    }

    #[test]
    fn prereleases_are_not_offered() {
        let json = SAMPLE.replace(r#""prerelease": false"#, r#""prerelease": true"#);
        assert!(Release::from_github_json(&json).is_err());
    }

    #[test]
    fn tags_that_are_not_versions_are_rejected() {
        let json = SAMPLE.replace("v1.2.0\"", "nightly\"");
        assert!(Release::from_github_json(&json).is_err());
    }
}
