//! The release feed: GitHub's releases API, and the asset naming contract.
//!
//! `GET {api}/repos/{owner}/{repo}/releases` lists releases newest first. The
//! updater takes the highest version on its channel (stable: releases not marked
//! pre-release and without a pre-release part in the version; pre-release: any
//! non-draft) whose tag is `v<semver>`. Nothing in that JSON is trusted beyond
//! "where to look": the release's `SHA256SUMS` must verify ([`crate::trust`]).
//!
//! ## Asset naming contract
//!
//! The release workflow publishes, for each headless build:
//!
//! ```text
//! librebooks-<tag>-<rust target triple>.zip      (Windows targets)
//! librebooks-<tag>-<rust target triple>.tar.gz   (everything else)
//! ```
//!
//! e.g. `librebooks-v0.3.0-x86_64-unknown-linux-musl.tar.gz`, holding the binary
//! `librebooks` (or `librebooks.exe`) at the top level or inside one folder, plus
//! `SHA256SUMS` listing every asset and `SHA256SUMS.minisig`.

use serde::{Deserialize, Serialize};

use crate::version::Version;

/// The Rust target triple this crate was compiled for.
pub const TARGET: &str = env!("LB_UPDATE_TARGET");

/// Which releases to follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// Releases only.
    #[default]
    Stable,
    /// Release candidates and betas too.
    Prerelease,
}

/// Where releases come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedSource {
    /// `https://api.github.com`, or a test server.
    pub api_base: String,
    pub owner: String,
    pub repo: String,
    /// The target triple whose asset to download.
    pub target: String,
}

impl Default for FeedSource {
    /// LibreBooks' own releases, for this build's target.
    fn default() -> Self {
        FeedSource {
            api_base: "https://api.github.com".into(),
            owner: "cinderblock".into(),
            repo: "LibreBooks".into(),
            target: TARGET.into(),
        }
    }
}

impl FeedSource {
    /// The releases list (first page, which holds the newest 30).
    pub fn releases_url(&self) -> String {
        format!(
            "{}/repos/{}/{}/releases?per_page=30",
            self.api_base.trim_end_matches('/'),
            self.owner,
            self.repo
        )
    }

    /// The asset for this source's target in release `tag`.
    pub fn asset_name(&self, tag: &str) -> String {
        asset_name(tag, &self.target)
    }
}

/// The archive name for `tag` and `target`, per the naming contract.
pub fn asset_name(tag: &str, target: &str) -> String {
    let ext = match archive_kind(target) {
        ArchiveKind::Zip => "zip",
        ArchiveKind::TarGz => "tar.gz",
    };
    format!("librebooks-{tag}-{target}.{ext}")
}

/// How a target's release is packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarGz,
}

pub fn archive_kind(target: &str) -> ArchiveKind {
    if target.contains("-windows-") {
        ArchiveKind::Zip
    } else {
        ArchiveKind::TarGz
    }
}

/// The binary's file name on `target`.
pub fn binary_name(target: &str) -> &'static str {
    if target.contains("-windows-") {
        "librebooks.exe"
    } else {
        "librebooks"
    }
}

/// A release as the GitHub API describes it (the fields used).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GhRelease {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    pub html_url: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<GhAsset>,
}

impl GhRelease {
    pub fn asset(&self, name: &str) -> Option<&GhAsset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// One file of a release.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GhAsset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
}

/// Parse the releases list.
///
/// # Errors
/// The JSON error, when it is not a list of releases.
pub fn parse_releases(json: &[u8]) -> Result<Vec<GhRelease>, serde_json::Error> {
    serde_json::from_slice(json)
}

/// The version a tag names: `v0.3.0` → `0.3.0`. Other tags (`nightly`,
/// `desktop-v1`) are not releases of this program.
pub fn tag_version(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v')?).ok()
}

/// The newest release on `channel`, with its version.
pub fn pick(releases: &[GhRelease], channel: Channel) -> Option<(Version, &GhRelease)> {
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| tag_version(&r.tag_name).map(|v| (v, r)))
        .filter(|(v, r)| match channel {
            Channel::Stable => !r.prerelease && v.is_release(),
            Channel::Prerelease => true,
        })
        .max_by(|a, b| a.0.cmp(&b.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASES: &[u8] = include_bytes!("../testdata/releases.json");

    #[test]
    fn the_fixture_parses() {
        let r = parse_releases(RELEASES).unwrap();
        assert_eq!(r.len(), 5);
        let v030 = r.iter().find(|r| r.tag_name == "v0.3.0").unwrap();
        assert_eq!(v030.assets.len(), 4);
        assert_eq!(v030.asset("SHA256SUMS").unwrap().size, 227);
        assert!(parse_releases(b"{\"message\":\"Not Found\"}").is_err());
    }

    #[test]
    fn channels_pick_the_newest_eligible_release() {
        let r = parse_releases(RELEASES).unwrap();
        let (v, rel) = pick(&r, Channel::Stable).unwrap();
        assert_eq!(v.to_string(), "0.3.0");
        assert_eq!(rel.tag_name, "v0.3.0");
        // The draft v0.5.0 and the non-semver "nightly" are never offered.
        let (v, _) = pick(&r, Channel::Prerelease).unwrap();
        assert_eq!(v.to_string(), "0.4.0-rc.1");
        assert!(pick(&[], Channel::Stable).is_none());
    }

    #[test]
    fn a_prerelease_version_is_not_stable_even_if_unflagged() {
        let json = br#"[{"tag_name":"v1.0.0-beta.1","draft":false,"prerelease":false,"html_url":"x","assets":[]},
                        {"tag_name":"v0.9.0","draft":false,"prerelease":false,"html_url":"y","assets":[]}]"#;
        let r = parse_releases(json).unwrap();
        assert_eq!(pick(&r, Channel::Stable).unwrap().0.to_string(), "0.9.0");
        assert_eq!(
            pick(&r, Channel::Prerelease).unwrap().0.to_string(),
            "1.0.0-beta.1"
        );
    }

    #[test]
    fn asset_names_follow_the_contract() {
        assert_eq!(
            asset_name("v0.3.0", "x86_64-pc-windows-msvc"),
            "librebooks-v0.3.0-x86_64-pc-windows-msvc.zip"
        );
        assert_eq!(
            asset_name("v0.3.0", "aarch64-unknown-linux-musl"),
            "librebooks-v0.3.0-aarch64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            asset_name("v1.0.0-rc.1", "aarch64-apple-darwin"),
            "librebooks-v1.0.0-rc.1-aarch64-apple-darwin.tar.gz"
        );
        assert_eq!(binary_name("x86_64-pc-windows-gnu"), "librebooks.exe");
        assert_eq!(binary_name("x86_64-unknown-linux-gnu"), "librebooks");
        assert_ne!(TARGET, "");
        assert_eq!(
            FeedSource::default().releases_url(),
            "https://api.github.com/repos/cinderblock/LibreBooks/releases?per_page=30"
        );
    }

    #[test]
    fn only_v_semver_tags_are_versions() {
        assert_eq!(tag_version("v0.3.0").unwrap().to_string(), "0.3.0");
        for t in ["0.3.0", "nightly", "v0.3", "desktop-v1.0.0", "v"] {
            assert!(tag_version(t).is_none(), "{t}");
        }
    }
}
