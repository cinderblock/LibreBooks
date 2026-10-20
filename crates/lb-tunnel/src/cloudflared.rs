//! Finding cloudflared, or fetching Cloudflare's own build and keeping it current.
//!
//! ## Where it comes from
//!
//! In order: the path in the settings; `cloudflared` on `PATH` (or where
//! Cloudflare's Windows installer puts it); LibreBooks' managed copy in
//! `<data dir>/bin/`. Only the managed copy is ever downloaded or updated:
//! anything else belongs to the user or a package manager.
//!
//! ## Trust model, honestly
//!
//! cloudflared releases are not signed with a key LibreBooks could check (the
//! Windows binaries carry an Authenticode signature and the macOS ones are
//! notarised, but there is nothing portable). What there is:
//!
//! 1. Cloudflare's release notes list a SHA-256 per asset (`### SHA256 Checksums:`,
//!    lines `name: hex`), parsed by [`parse_checksums`].
//! 2. GitHub records its own SHA-256 of every uploaded asset (the API's `digest`,
//!    `sha256:hex`).
//!
//! A download must match the release notes, and GitHub's digest when present.
//! Both come from the same GitHub API response over TLS, so this proves the bytes
//! are the ones Cloudflare published there: it catches corrupted or truncated
//! downloads and tampering on the download path (CDN, proxy, mirror). It does not
//! protect against a compromise of Cloudflare's GitHub account or of GitHub itself.
//! Users who want more can install cloudflared from Cloudflare's signed package
//! repositories and point `cloudflared_path` at it.
//!
//! **Finding (2026.10.1):** for the macOS `.tgz` assets the release notes list the
//! hash of the `cloudflared` binary *inside* the archive, while GitHub's digest is
//! of the archive. So a `.tgz` is checked against the digest, unpacked, and the
//! binary checked against the notes.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::TunnelError;

/// The executable's file name here.
pub const BINARY: &str = if cfg!(windows) {
    "cloudflared.exe"
} else {
    "cloudflared"
};

/// Largest download accepted (the Windows binary is about 55 MB).
pub const DOWNLOAD_LIMIT: u64 = 200 << 20;

/// Where a cloudflared was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// `cloudflared_path` in the settings.
    Configured,
    /// On `PATH`, or installed by Cloudflare's own installer.
    System,
    /// LibreBooks' own copy, which it keeps updated.
    Managed,
}

/// A cloudflared to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Located {
    pub path: PathBuf,
    pub origin: Origin,
}

/// Where the managed copy lives.
pub fn managed_path(data_dir: &Path) -> PathBuf {
    data_dir.join("bin").join(BINARY)
}

/// Where to look for cloudflared.
#[derive(Debug, Clone, Copy)]
pub struct Sources<'a> {
    /// `cloudflared_path` from the settings.
    pub configured: Option<&'a Path>,
    /// LibreBooks' data folder (the managed copy is in `bin/` under it).
    pub data_dir: &'a Path,
    /// Look on `PATH` and in the installer's folder. Off: only the configured and
    /// managed copies count.
    pub search_system: bool,
}

/// Find a cloudflared (see the module docs for the order).
///
/// # Errors
/// [`TunnelError::NotInstalled`] when `configured` is set but missing: a typo
/// should not silently run some other copy.
pub fn locate(sources: &Sources<'_>) -> Result<Option<Located>, TunnelError> {
    if let Some(p) = sources.configured {
        return if p.is_file() {
            Ok(Some(Located {
                path: p.to_owned(),
                origin: Origin::Configured,
            }))
        } else {
            Err(TunnelError::NotInstalled(format!(
                "the configured cloudflared {} does not exist",
                p.display()
            )))
        };
    }
    if sources.search_system
        && let Some(path) = on_path()
    {
        return Ok(Some(Located {
            path,
            origin: Origin::System,
        }));
    }
    let managed = managed_path(sources.data_dir);
    Ok(managed.is_file().then_some(Located {
        path: managed,
        origin: Origin::Managed,
    }))
}

fn on_path() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(BINARY)).collect())
        .unwrap_or_default();
    if cfg!(windows) {
        // Cloudflare's MSI installs here and does not always add it to PATH.
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(dir) = std::env::var_os(var) {
                candidates.push(PathBuf::from(dir).join("cloudflared").join(BINARY));
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// `cloudflared version 2026.10.0 (built 2026-10-05T08:39 UTC)` → `2026.10.0`.
pub fn parse_version_output(output: &str) -> Option<String> {
    let rest = output.trim().strip_prefix("cloudflared version ")?;
    let v = rest.split_whitespace().next()?;
    v.chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
        .then(|| v.to_owned())
}

/// Compare `2026.10.0`-style versions numerically, part by part.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| -> Vec<u64> {
        s.split(['.', '-'])
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    parts(a).cmp(&parts(b))
}

/// Ask a cloudflared binary its version.
///
/// # Errors
/// [`TunnelError::Spawn`] if it does not run or does not answer like cloudflared.
pub async fn version_of(path: &Path) -> Result<String, TunnelError> {
    let mut cmd = tokio::process::Command::new(path);
    cmd.arg("--version")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(crate::proc::CREATE_NO_WINDOW);
    let out = tokio::time::timeout(Duration::from_secs(15), cmd.output())
        .await
        .map_err(|_| TunnelError::Spawn(format!("{} --version did not answer", path.display())))?
        .map_err(|e| TunnelError::Spawn(format!("could not run {}: {e}", path.display())))?;
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    parse_version_output(&text).ok_or_else(|| {
        TunnelError::Spawn(format!(
            "{} does not look like cloudflared (its --version said {:?})",
            path.display(),
            text.trim()
        ))
    })
}

/// How an asset is packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packaging {
    /// The executable itself.
    Raw,
    /// A gzipped tar holding `cloudflared` (macOS).
    Tgz,
}

/// The release asset for a platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformAsset {
    pub name: &'static str,
    pub packaging: Packaging,
}

/// The asset for `os`/`arch` as Rust names them (`std::env::consts`).
pub fn asset_for(os: &str, arch: &str) -> Option<PlatformAsset> {
    let raw = |name| {
        Some(PlatformAsset {
            name,
            packaging: Packaging::Raw,
        })
    };
    let tgz = |name| {
        Some(PlatformAsset {
            name,
            packaging: Packaging::Tgz,
        })
    };
    match (os, arch) {
        // No native Windows on ARM build; Windows 11 runs the x64 one.
        ("windows", "x86_64" | "aarch64") => raw("cloudflared-windows-amd64.exe"),
        ("windows", "x86") => raw("cloudflared-windows-386.exe"),
        ("linux", "x86_64") => raw("cloudflared-linux-amd64"),
        ("linux", "aarch64") => raw("cloudflared-linux-arm64"),
        ("linux", "arm") => raw("cloudflared-linux-arm"),
        ("linux", "x86") => raw("cloudflared-linux-386"),
        ("macos", "x86_64") => tgz("cloudflared-darwin-amd64.tgz"),
        ("macos", "aarch64") => tgz("cloudflared-darwin-arm64.tgz"),
        _ => None,
    }
}

/// The asset for the machine this runs on.
pub fn this_platform() -> Option<PlatformAsset> {
    asset_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// A cloudflared release as the GitHub API describes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
    /// GitHub's own hash of the upload, `sha256:<hex>` (newer releases).
    #[serde(default)]
    pub digest: Option<String>,
}

impl Asset {
    fn github_sha256(&self) -> Option<[u8; 32]> {
        let hex = self.digest.as_deref()?.strip_prefix("sha256:")?;
        let mut out = [0u8; 32];
        hex::decode_to_slice(hex, &mut out).ok()?;
        Some(out)
    }
}

/// The `name: sha256` lines of a release's notes.
pub fn parse_checksums(body: &str) -> BTreeMap<String, [u8; 32]> {
    let mut out = BTreeMap::new();
    for line in body.lines() {
        let line = line.trim().trim_matches('`').trim();
        let Some((name, hex)) = line.split_once(':') else {
            continue;
        };
        let (name, hex) = (name.trim().trim_matches('`'), hex.trim().trim_matches('`'));
        let name_ok = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        let mut sum = [0u8; 32];
        if name_ok && hex.len() == 64 && hex::decode_to_slice(hex, &mut sum).is_ok() {
            out.insert(name.to_owned(), sum);
        }
    }
    out
}

/// Downloads cloudflared from its GitHub releases.
#[derive(Debug, Clone)]
pub struct Downloader {
    http: reqwest::Client,
    /// `https://api.github.com`, or a test server.
    pub api_base: String,
    /// `cloudflare/cloudflared`.
    pub repo: String,
    pub limit: u64,
}

impl Downloader {
    /// # Errors
    /// [`TunnelError::Network`] if the HTTP client cannot be built.
    pub fn new() -> Result<Downloader, TunnelError> {
        Downloader::with_api("https://api.github.com")
    }

    /// # Errors
    /// [`TunnelError::Network`] if the HTTP client cannot be built.
    pub fn with_api(api_base: &str) -> Result<Downloader, TunnelError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("LibreBooks/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .build()?;
        Ok(Downloader {
            http,
            api_base: api_base.trim_end_matches('/').to_owned(),
            repo: "cloudflare/cloudflared".into(),
            limit: DOWNLOAD_LIMIT,
        })
    }

    /// The latest release.
    ///
    /// # Errors
    /// [`TunnelError::Network`] or an unreadable answer.
    pub async fn latest(&self) -> Result<Release, TunnelError> {
        let url = format!("{}/repos/{}/releases/latest", self.api_base, self.repo);
        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(TunnelError::Network(format!(
                "{url} answered HTTP {}",
                resp.status().as_u16()
            )));
        }
        let bytes = resp.bytes().await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| TunnelError::Network(format!("an unreadable release from {url}: {e}")))
    }

    /// Download `platform`'s asset of `release`, verify it, and put the binary at
    /// `dest` (replacing what is there). Returns the release's version.
    ///
    /// # Errors
    /// [`TunnelError::Checksum`] when it does not match, [`TunnelError::Unverifiable`]
    /// when the release lists no checksum for it, or a network or disk error.
    pub async fn install(
        &self,
        release: &Release,
        platform: PlatformAsset,
        dest: &Path,
    ) -> Result<String, TunnelError> {
        let sums = parse_checksums(release.body.as_deref().unwrap_or(""));
        let expected = *sums
            .get(platform.name)
            .ok_or_else(|| TunnelError::Unverifiable(platform.name.to_owned()))?;
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == platform.name)
            .ok_or_else(|| {
                TunnelError::NotInstalled(format!(
                    "cloudflared {} has no {} to download",
                    release.tag_name, platform.name
                ))
            })?;

        let dir = dest.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(TunnelError::io("create the cloudflared folder"))?;
        let mut download = tempfile::Builder::new()
            .prefix(".cloudflared-download-")
            .tempfile_in(dir)
            .map_err(TunnelError::io("create the download"))?;
        let digest = self
            .fetch(&asset.browser_download_url, download.as_file_mut())
            .await?;
        if asset.github_sha256().is_some_and(|g| g != digest) {
            return Err(TunnelError::Checksum(platform.name.to_owned()));
        }

        let binary = match platform.packaging {
            Packaging::Raw => {
                if digest != expected {
                    return Err(TunnelError::Checksum(platform.name.to_owned()));
                }
                download.into_temp_path()
            }
            Packaging::Tgz => {
                let path = download.into_temp_path();
                let out = tempfile::Builder::new()
                    .prefix(".cloudflared-binary-")
                    .tempfile_in(dir)
                    .map_err(TunnelError::io("unpack cloudflared"))?
                    .into_temp_path();
                let inner = untgz_cloudflared(&path, &out)?;
                if inner != expected {
                    return Err(TunnelError::Checksum(format!(
                        "cloudflared in {}",
                        platform.name
                    )));
                }
                out
            }
        };
        make_executable(&binary)?;
        binary.persist(dest).map_err(|e| TunnelError::Io {
            what: "put cloudflared in place",
            source: e.error,
        })?;
        tracing::info!(version = %release.tag_name, path = %dest.display(), "cloudflared installed");
        Ok(release.tag_name.clone())
    }

    /// GET `url` into `file`; returns the SHA-256 of what was written.
    async fn fetch(&self, url: &str, file: &mut std::fs::File) -> Result<[u8; 32], TunnelError> {
        let mut resp = self.http.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(TunnelError::Network(format!(
                "{url} answered HTTP {}",
                resp.status().as_u16()
            )));
        }
        if resp.content_length().is_some_and(|n| n > self.limit) {
            return Err(TunnelError::Network(format!("{url} is unexpectedly large")));
        }
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        while let Some(chunk) = resp.chunk().await? {
            total += chunk.len() as u64;
            if total > self.limit {
                return Err(TunnelError::Network(format!("{url} is unexpectedly large")));
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .map_err(TunnelError::io("write the download"))?;
        }
        file.flush()
            .map_err(TunnelError::io("write the download"))?;
        Ok(hasher.finalize().into())
    }
}

/// Unpack the `cloudflared` file from a `.tgz` into `out`; returns its SHA-256.
fn untgz_cloudflared(tgz: &Path, out: &Path) -> Result<[u8; 32], TunnelError> {
    let file = std::fs::File::open(tgz).map_err(TunnelError::io("open the download"))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let entries = archive
        .entries()
        .map_err(|e| TunnelError::Archive(e.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| TunnelError::Archive(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| TunnelError::Archive(e.to_string()))?
            .into_owned();
        let is_it = entry.header().entry_type().is_file()
            && path.file_name().is_some_and(|n| n == "cloudflared")
            && path.components().count() <= 2;
        if is_it {
            let mut dest =
                std::fs::File::create(out).map_err(TunnelError::io("unpack cloudflared"))?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 64 << 10];
            let mut total = 0u64;
            loop {
                let n = std::io::Read::read(&mut entry, &mut buf)
                    .map_err(|e| TunnelError::Archive(e.to_string()))?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > DOWNLOAD_LIMIT {
                    return Err(TunnelError::Archive(
                        "cloudflared is implausibly large".into(),
                    ));
                }
                hasher.update(&buf[..n]);
                dest.write_all(&buf[..n])
                    .map_err(TunnelError::io("unpack cloudflared"))?;
            }
            return Ok(hasher.finalize().into());
        }
    }
    Err(TunnelError::Archive(
        "the archive has no cloudflared".into(),
    ))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), TunnelError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(TunnelError::io("mark cloudflared executable"))
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn make_executable(_: &Path) -> Result<(), TunnelError> {
    Ok(())
}

/// A cloudflared ready to run, and its version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ready {
    pub located: Located,
    pub version: String,
}

/// Find cloudflared; when allowed, download it if missing, and update the managed
/// copy if a newer release exists. A failed update check keeps the current copy.
///
/// # Errors
/// [`TunnelError::NotInstalled`] when there is none and downloading is off, or the
/// download's error.
pub async fn ensure(
    sources: &Sources<'_>,
    auto_download: bool,
    downloader: &Downloader,
) -> Result<Ready, TunnelError> {
    let found = locate(sources)?;
    match found {
        Some(located) if located.origin != Origin::Managed || !auto_download => {
            let version = version_of(&located.path).await?;
            Ok(Ready { located, version })
        }
        Some(located) => {
            let current = version_of(&located.path).await.ok();
            match downloader.latest().await {
                Ok(release)
                    if current.as_deref().is_none_or(|c| {
                        compare_versions(&release.tag_name, c) == std::cmp::Ordering::Greater
                    }) =>
                {
                    let platform = this_platform().ok_or(TunnelError::UnsupportedPlatform)?;
                    match downloader.install(&release, platform, &located.path).await {
                        Ok(version) => Ok(Ready { located, version }),
                        Err(e) if current.is_some() => {
                            tracing::warn!(error = %e, "cloudflared update failed; keeping the current one");
                            Ok(Ready {
                                located,
                                version: current.unwrap_or_default(),
                            })
                        }
                        Err(e) => Err(e),
                    }
                }
                Ok(_) => Ok(Ready {
                    version: current.unwrap_or_default(),
                    located,
                }),
                Err(e) => {
                    tracing::warn!(error = %e, "could not check for a cloudflared update");
                    match current {
                        Some(version) => Ok(Ready { located, version }),
                        None => Err(e),
                    }
                }
            }
        }
        None if auto_download => {
            let platform = this_platform().ok_or(TunnelError::UnsupportedPlatform)?;
            let release = downloader.latest().await?;
            let path = managed_path(sources.data_dir);
            let version = downloader.install(&release, platform, &path).await?;
            Ok(Ready {
                located: Located {
                    path,
                    origin: Origin::Managed,
                },
                version,
            })
        }
        None => Err(TunnelError::NotInstalled(
            "cloudflared is not installed. Install it from Cloudflare, or let LibreBooks download it (auto_download).".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../testdata/cloudflared-release-2026.10.1.json");

    #[test]
    fn checksums_parse_from_real_release_notes() {
        let r: Release = serde_json::from_slice(FIXTURE).unwrap();
        let sums = parse_checksums(r.body.as_deref().unwrap());
        assert_eq!(sums.len(), 26);
        assert_eq!(r.assets.len(), 26);
        assert_eq!(
            hex::encode(sums["cloudflared-windows-amd64.exe"]),
            "4d2490e74082c5d48dbfedaf92576fa5bd1702379ab88eb7faa0cfbe8dc891b6"
        );
        // Every platform we support has a checksum and an asset.
        for (os, arch) in [
            ("windows", "x86_64"),
            ("windows", "x86"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("linux", "arm"),
            ("linux", "x86"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
        ] {
            let a = asset_for(os, arch).unwrap();
            assert!(sums.contains_key(a.name), "{os}/{arch}");
            assert!(r.assets.iter().any(|x| x.name == a.name), "{os}/{arch}");
        }
        // Raw binaries: the notes and GitHub agree.
        let linux = r
            .assets
            .iter()
            .find(|a| a.name == "cloudflared-linux-amd64")
            .unwrap();
        assert_eq!(linux.github_sha256(), Some(sums["cloudflared-linux-amd64"]));
        // macOS archives: they do not (the notes hash the binary inside).
        let mac = r
            .assets
            .iter()
            .find(|a| a.name == "cloudflared-darwin-arm64.tgz")
            .unwrap();
        assert_ne!(
            mac.github_sha256(),
            Some(sums["cloudflared-darwin-arm64.tgz"])
        );
    }

    #[test]
    fn checksum_lines_are_strict() {
        let h = "ab".repeat(32);
        let body = format!(
            "notes\n### SHA256 Checksums:\n```\nok-name.exe: {h}\n`ticked`: {h}\nbad name: {h}\nshort: abcd\n```\n"
        );
        let s = parse_checksums(&body);
        assert_eq!(s.keys().collect::<Vec<_>>(), ["ok-name.exe", "ticked"]);
        assert!(parse_checksums("").is_empty());
    }

    #[test]
    fn platforms_and_versions() {
        assert_eq!(asset_for("linux", "riscv64"), None);
        assert_eq!(
            asset_for("macos", "aarch64").unwrap().packaging,
            Packaging::Tgz
        );
        assert_eq!(
            parse_version_output("cloudflared version 2026.10.0 (built 2026-10-05T08:39 UTC)\n")
                .as_deref(),
            Some("2026.10.0")
        );
        assert_eq!(parse_version_output("something else"), None);
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("2026.10.1", "2026.10.0"), Greater);
        assert_eq!(compare_versions("2026.9.0", "2026.10.0"), Less);
        assert_eq!(compare_versions("2026.10.0", "2026.10.0"), Equal);
    }

    #[test]
    fn a_missing_configured_binary_is_an_error_not_a_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        let sources = |configured| Sources {
            configured,
            data_dir: dir.path(),
            search_system: false,
        };
        let err = locate(&sources(Some(&missing))).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
        let managed = managed_path(dir.path());
        std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
        std::fs::write(&managed, b"x").unwrap();
        let found = locate(&sources(Some(&managed))).unwrap().unwrap();
        assert_eq!(found.origin, Origin::Configured);
        let found = locate(&sources(None)).unwrap().unwrap();
        assert_eq!(found.origin, Origin::Managed);
    }
}
