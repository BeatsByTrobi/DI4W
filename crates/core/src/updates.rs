//! Looks for updates in the repository's GitHub releases (used by the island and the settings app).
//!
//! A release counts when its tag is a version ("v1.2.0") and it has a `*-win-x64.zip` or `*-win-x64.msi` asset
//! (or `-win-arm64` ones for the Arm64 build, so each build stays on its own architecture).

use serde_json::Value;
use std::fmt;

pub const REPOSITORY: &str = "aangeraakt/DI4W";

/// Signalled by the settings app to make a running island quit cleanly before files are replaced.
pub const ISLAND_QUIT_EVENT: &str = "DynamicIsland4Windows.Quit";

/// GitHub's API rejects requests without a User-Agent.
pub const USER_AGENT: &str = "DI4W-Updater/1.0";

/// major.minor.patch; "1.2" and "1.2.0.0" both read as 1.2.0, so versions compare the way people read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    /// "1.2", "1.2.3" or "1.2.3.4" (the fourth part is dropped); `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        let parts: Vec<&str> = text.trim().split('.').collect();
        if !(2..=4).contains(&parts.len()) {
            return None;
        }
        let mut numbers = [0u32; 3];
        for (i, part) in parts.iter().enumerate() {
            let n: u32 = part.parse().ok()?;
            if i < 3 {
                numbers[i] = n;
            }
        }
        Some(Self::new(numbers[0], numbers[1], numbers[2]))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// This build's version (the workspace version in Cargo.toml).
pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version::new(0, 0, 0))
}

/// "x64" or "arm64": the build this is (an x64 build running emulated on Arm stays x64).
pub fn architecture() -> &'static str {
    if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" }
}

/// A downloadable file of a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Hex SHA-256 as reported by GitHub, when available.
    pub sha256: Option<String>,
}

/// A newer release found on GitHub: the zip (copy-over update) and/or the installer (.msi).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: Version,
    pub notes: String,
    pub page_url: String,
    pub zip: Option<ReleaseAsset>,
    pub msi: Option<ReleaseAsset>,
}

/// The latest release when it is newer than this build, otherwise `None`. Errors on network or API problems.
pub fn check() -> Result<Option<UpdateInfo>, String> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    match response.status().as_u16() {
        404 => Ok(None), // no published releases yet
        200..=299 => {
            let json = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
            Ok(parse(&json, current_version(), architecture()))
        }
        status => Err(format!("GitHub answered {status}")),
    }
}

/// Reads GitHub's "latest release" JSON; `None` when it isn't newer than `current` or has no package for
/// `architecture` ("x64" or "arm64").
pub fn parse(json: &str, current: Version, architecture: &str) -> Option<UpdateInfo> {
    let release: Value = serde_json::from_str(json).ok()?;
    let tag = release.get("tag_name")?.as_str()?;
    let version = Version::parse(tag.trim_start_matches(['v', 'V']))?;
    if version <= current {
        return None;
    }

    let (mut zip, mut msi) = (None, None);
    let zip_suffix = format!("-win-{architecture}.zip");
    let msi_suffix = format!("-win-{architecture}.msi");
    for asset in release.get("assets")?.as_array()? {
        let name = asset.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        // GitHub reports "sha256:<hex>" for uploaded assets; used to verify the download.
        let sha256 = asset
            .get("digest")
            .and_then(Value::as_str)
            .and_then(|d| d.get(..7).filter(|p| p.eq_ignore_ascii_case("sha256:")).map(|_| d[7..].to_string()));
        let file = ReleaseAsset {
            url: asset.get("browser_download_url").and_then(Value::as_str)?.to_string(),
            size: asset.get("size").and_then(Value::as_u64).unwrap_or(0),
            sha256,
            name: name.clone(),
        };
        let lower = name.to_lowercase();
        if lower.ends_with(&zip_suffix) {
            zip = Some(file);
        } else if lower.ends_with(&msi_suffix) {
            msi = Some(file);
        }
    }
    if zip.is_none() && msi.is_none() {
        return None; // a release without a Windows package
    }

    Some(UpdateInfo {
        version,
        notes: release.get("body").and_then(Value::as_str).unwrap_or_default().to_string(),
        page_url: release
            .get("html_url")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://github.com/{REPOSITORY}/releases")),
        zip,
        msi,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[(&str, Option<&str>)]) -> String {
        let assets: Vec<String> = assets
            .iter()
            .map(|(name, digest)| {
                let digest = digest.map(|d| format!(r#", "digest": "{d}""#)).unwrap_or_default();
                format!(r#"{{ "name": "{name}", "size": 1234, "browser_download_url": "https://example.test/{name}"{digest} }}"#)
            })
            .collect();
        format!(
            r#"{{ "tag_name": "{tag}", "html_url": "https://github.com/aangeraakt/DI4W/releases/tag/{tag}", "body": "What's new", "assets": [{}] }}"#,
            assets.join(",")
        )
    }

    const V120: Version = Version::new(1, 2, 0);

    #[test]
    fn finds_a_newer_release_with_both_packages() {
        let json = release("v1.3.0", &[("DI4W-v1.3.0-win-x64.zip", Some("sha256:abc123")), ("DI4W-v1.3.0-win-x64.msi", Some("sha256:def456"))]);
        let update = parse(&json, V120, "x64").unwrap();
        assert_eq!(update.version, Version::new(1, 3, 0));
        assert_eq!(update.zip.as_ref().unwrap().sha256.as_deref(), Some("abc123"));
        assert_eq!(update.msi.as_ref().unwrap().sha256.as_deref(), Some("def456"));
        assert_eq!(update.zip.as_ref().unwrap().size, 1234);
        assert_eq!(update.notes, "What's new");
    }

    #[test]
    fn ignores_releases_that_are_not_newer() {
        for tag in ["v1.2.0", "v1.1.9", "nightly"] {
            assert_eq!(parse(&release(tag, &[("DI4W-win-x64.zip", None)]), V120, "x64"), None, "{tag}");
        }
    }

    #[test]
    fn ignores_a_release_without_a_windows_package() {
        assert_eq!(parse(&release("v2.0.0", &[("source.tar.gz", None)]), V120, "x64"), None);
    }

    #[test]
    fn missing_digest_means_no_checksum() {
        let update = parse(&release("v2.0.0", &[("DI4W-v2.0.0-win-x64.zip", None)]), Version::new(1, 0, 0), "x64").unwrap();
        assert_eq!(update.zip.unwrap().sha256, None);
        assert_eq!(update.msi, None);
    }

    #[test]
    fn normalizes_versions() {
        assert_eq!(Version::parse("1.2"), Some(Version::new(1, 2, 0)));
        assert_eq!(Version::parse("1.2.0.0"), Some(Version::new(1, 2, 0)));
        assert_eq!(Version::parse("1.2.3.4"), Some(Version::new(1, 2, 3)));
        assert_eq!(Version::parse("1"), None);
        assert_eq!(Version::parse("nightly"), None);
    }

    #[test]
    fn each_build_updates_to_its_own_architecture() {
        let json = release("v9.0.0", &[("DI4W-v9.0.0-win-x64.msi", None), ("DI4W-v9.0.0-win-arm64.msi", None)]);
        assert_eq!(parse(&json, Version::new(1, 0, 0), "x64").unwrap().msi.unwrap().name, "DI4W-v9.0.0-win-x64.msi");
        assert_eq!(parse(&json, Version::new(1, 0, 0), "arm64").unwrap().msi.unwrap().name, "DI4W-v9.0.0-win-arm64.msi");
    }
}
