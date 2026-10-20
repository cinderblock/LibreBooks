//! The update stages, check → download → install, and the status the server shows.
//!
//! Each stage is a separate async call, so the server can expose them as
//! `POST /server/update/check` and `POST /server/update/install`, and [`Updater::tick`]
//! runs whatever the policy says is next, for the server's background timer.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use jiff::Timestamp;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;

use crate::UpdateError;
use crate::feed::{self, FeedSource, GhRelease};
use crate::install::{self, InstallKind, Restart, Target};
use crate::policy::{self, PolicyState, Step, UpdateConfig};
use crate::trust::{self, TrustedKey};
use crate::version::{Build, Version};

/// Largest releases list accepted.
const LIST_LIMIT: u64 = 4 << 20;
/// Largest `SHA256SUMS` / signature accepted.
const SMALL_LIMIT: u64 = 64 << 10;
/// Largest archive accepted by default.
pub const DOWNLOAD_LIMIT: u64 = 256 << 20;

/// Everything the updater works with.
#[derive(Debug, Clone)]
pub struct UpdaterOptions {
    pub source: FeedSource,
    pub key: TrustedKey,
    /// What is running now.
    pub current: Build,
    pub config: UpdateConfig,
    /// From [`install::detect_current`] (or chosen, in tests).
    pub install_kind: InstallKind,
    /// What an install replaces.
    pub target: Target,
    /// Where archives are downloaded and unpacked.
    pub download_dir: PathBuf,
    /// Largest archive accepted.
    pub download_limit: u64,
}

/// A verified release newer than what is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Available {
    pub version: Version,
    pub tag: String,
    /// The release page, for people.
    pub page: String,
    pub asset_name: String,
    pub asset_url: String,
    #[serde(skip)]
    pub sha256: [u8; 32],
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// Nothing newer on the channel. `latest` is the newest there is, if any.
    UpToDate {
        latest: Option<Version>,
    },
    Available(Available),
}

/// A downloaded, verified archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    pub release: Available,
    pub path: PathBuf,
}

/// A completed install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Installed {
    pub version: Version,
    pub exe: PathBuf,
    /// The previous binary, kept for rollback.
    pub previous: PathBuf,
    /// How to start the new version.
    pub restart: Restart,
}

/// What the updater is doing, for `GET /server`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateStatus {
    pub current: String,
    pub can_install: bool,
    pub install_kind: InstallKind,
    /// For a copy that cannot update itself: what to do instead.
    pub advice: Option<String>,
    pub last_check: Option<Timestamp>,
    pub available: Option<Available>,
    pub phase: Phase,
}

/// The stage in progress or last finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Checking,
    Downloading {
        version: Version,
    },
    Installing {
        version: Version,
    },
    /// Installed; the new version runs after a restart.
    RestartPending {
        version: Version,
        restart: Restart,
    },
    Failed {
        stage: Stage,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Check,
    Download,
    Install,
}

struct State {
    config: UpdateConfig,
    phase: Phase,
    last_check: Option<Timestamp>,
    available: Option<Available>,
    failed_install: Option<Version>,
}

/// Checks for, downloads and installs updates for the headless server.
pub struct Updater {
    options: UpdaterOptions,
    http: reqwest::Client,
    state: Mutex<State>,
}

impl std::fmt::Debug for Updater {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updater")
            .field("current", &self.options.current)
            .field("phase", &self.lock().phase)
            .finish_non_exhaustive()
    }
}

impl Updater {
    /// # Errors
    /// [`UpdateError::Network`] if the HTTP client cannot be built.
    pub fn new(options: UpdaterOptions) -> Result<Updater, UpdateError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!(
                "LibreBooks/",
                env!("CARGO_PKG_VERSION"),
                " (self-update)"
            ))
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()?;
        let state = State {
            config: options.config.clone(),
            phase: Phase::Idle,
            last_check: None,
            available: None,
            failed_install: None,
        };
        Ok(Updater {
            options,
            http,
            state: Mutex::new(state),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn status(&self) -> UpdateStatus {
        let s = self.lock();
        UpdateStatus {
            current: self.options.current.to_string(),
            can_install: self.options.install_kind.can_install(),
            install_kind: self.options.install_kind.clone(),
            advice: self.options.install_kind.advice(),
            last_check: s.last_check,
            available: s.available.clone(),
            phase: s.phase.clone(),
        }
    }

    pub fn config(&self) -> UpdateConfig {
        self.lock().config.clone()
    }

    pub fn set_config(&self, config: UpdateConfig) {
        self.lock().config = config;
    }

    /// Find the newest release on the channel and verify it.
    ///
    /// # Errors
    /// [`UpdateError`] when the feed cannot be read or the release does not verify.
    pub async fn check(&self) -> Result<CheckOutcome, UpdateError> {
        self.lock().phase = Phase::Checking;
        let result = self.check_inner().await;
        let mut s = self.lock();
        s.last_check = Some(Timestamp::now());
        match &result {
            Ok(CheckOutcome::Available(a)) => {
                s.available = Some(a.clone());
                s.phase = Phase::Idle;
            }
            Ok(CheckOutcome::UpToDate { .. }) => {
                s.available = None;
                s.phase = Phase::Idle;
            }
            Err(e) => {
                s.phase = Phase::Failed {
                    stage: Stage::Check,
                    message: e.to_string(),
                }
            }
        }
        result
    }

    async fn check_inner(&self) -> Result<CheckOutcome, UpdateError> {
        let channel = self.lock().config.channel;
        let list = self
            .get(&self.options.source.releases_url(), LIST_LIMIT, true)
            .await?;
        let releases = feed::parse_releases(&list).map_err(|e| UpdateError::Feed(e.to_string()))?;
        let Some((version, release)) = feed::pick(&releases, channel) else {
            return Ok(CheckOutcome::UpToDate { latest: None });
        };
        if !self.options.current.is_older_than(&version) {
            return Ok(CheckOutcome::UpToDate {
                latest: Some(version),
            });
        }
        let available = self.verify_release(version, release).await?;
        tracing::info!(version = %available.version, "update available");
        Ok(CheckOutcome::Available(available))
    }

    async fn verify_release(
        &self,
        version: Version,
        release: &GhRelease,
    ) -> Result<Available, UpdateError> {
        let tag = &release.tag_name;
        let file = |name: &str| {
            release
                .asset(name)
                .map(|a| a.browser_download_url.clone())
                .ok_or_else(|| UpdateError::NoAsset {
                    tag: tag.clone(),
                    name: name.to_owned(),
                })
        };
        let signature = self
            .get(&file(trust::SIGNATURE)?, SMALL_LIMIT, false)
            .await?;
        let signature = String::from_utf8(signature)
            .map_err(|_| UpdateError::Trust(trust::TrustError::SignatureFile))?;
        let sums = self.get(&file(trust::SUMS)?, SMALL_LIMIT, false).await?;
        let signed = self.options.key.verify(&signature, &sums, tag)?;

        let asset_name = self.options.source.asset_name(tag);
        let sum = signed
            .get(&asset_name)
            .ok_or_else(|| UpdateError::NoAsset {
                tag: tag.clone(),
                name: asset_name.clone(),
            })?;
        Ok(Available {
            version,
            tag: tag.clone(),
            page: release.html_url.clone(),
            asset_url: file(&asset_name)?,
            asset_name,
            sha256: sum.sha256,
        })
    }

    /// Download `release`'s archive and check it against the signed sums.
    ///
    /// # Errors
    /// [`UpdateError::Checksum`] (the file is removed), [`UpdateError::TooLarge`],
    /// or a network or disk error.
    pub async fn download(&self, release: &Available) -> Result<Downloaded, UpdateError> {
        self.lock().phase = Phase::Downloading {
            version: release.version.clone(),
        };
        let result = self.download_inner(release).await;
        if let Err(e) = &result {
            self.lock().phase = Phase::Failed {
                stage: Stage::Download,
                message: e.to_string(),
            };
        }
        result
    }

    async fn download_inner(&self, release: &Available) -> Result<Downloaded, UpdateError> {
        let dir = &self.options.download_dir;
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(UpdateError::io("create the download folder"))?;
        let path = dir.join(&release.asset_name);
        let part = dir.join(format!("{}.part", release.asset_name));
        let mut file = tokio::fs::File::create(&part)
            .await
            .map_err(UpdateError::io("create the download"))?;
        let mut hasher = Sha256::new();
        let fetched = self
            .fetch(
                &release.asset_url,
                self.options.download_limit,
                false,
                |chunk| hasher.update(chunk),
                &mut file,
            )
            .await;
        let flushed = file.flush().await;
        drop(file);
        let digest: [u8; 32] = hasher.finalize().into();
        let outcome = fetched
            .and_then(|()| flushed.map_err(UpdateError::io("write the download")))
            .and_then(|()| {
                if digest == release.sha256 {
                    Ok(())
                } else {
                    Err(UpdateError::Checksum)
                }
            });
        if let Err(e) = outcome {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(e);
        }
        tokio::fs::rename(&part, &path)
            .await
            .map_err(UpdateError::io("move the download into place"))?;
        tracing::info!(path = %path.display(), "update downloaded and verified");
        self.lock().phase = Phase::Idle;
        Ok(Downloaded {
            release: release.clone(),
            path,
        })
    }

    /// Unpack the downloaded archive and replace the binary. The new version runs
    /// after the restart the result describes.
    ///
    /// # Errors
    /// [`UpdateError::NotSelfUpdatable`] for a copy that cannot update itself
    /// (with what to do instead), [`UpdateError::Checksum`] if the archive changed
    /// since it was verified, or an archive or disk error.
    pub async fn install(&self, downloaded: &Downloaded) -> Result<Installed, UpdateError> {
        if let Some(advice) = self.options.install_kind.advice() {
            return Err(UpdateError::NotSelfUpdatable(advice));
        }
        let version = downloaded.release.version.clone();
        self.lock().phase = Phase::Installing {
            version: version.clone(),
        };
        let target = self.options.target.clone();
        let source = self.options.source.clone();
        let dir = self.options.download_dir.clone();
        let downloaded2 = downloaded.clone();
        let result = tokio::task::spawn_blocking(move || {
            install_blocking(&downloaded2, &source, &dir, &target)
        })
        .await
        .map_err(|e| UpdateError::Archive(format!("the install task failed: {e}")))
        .and_then(|r| r);
        let mut s = self.lock();
        match result {
            Ok((exe, previous)) => {
                let restart = install::restart_strategy(&install::System);
                s.phase = Phase::RestartPending {
                    version: version.clone(),
                    restart: restart.clone(),
                };
                Ok(Installed {
                    version,
                    exe,
                    previous,
                    restart,
                })
            }
            Err(e) => {
                s.failed_install = Some(version);
                s.phase = Phase::Failed {
                    stage: Stage::Install,
                    message: e.to_string(),
                };
                Err(e)
            }
        }
    }

    /// Run the policy's next step: check when due, and install when the
    /// configuration says so. Returns the install when one happened (the caller
    /// then restarts).
    ///
    /// # Errors
    /// Whatever the step that ran returned.
    pub async fn tick(&self) -> Result<Option<Installed>, UpdateError> {
        // At most a check and then an install.
        for _ in 0..2 {
            let step = {
                let s = self.lock();
                let state = PolicyState {
                    last_check: s.last_check,
                    available: s.available.as_ref().map(|a| a.version.clone()),
                    can_install: self.options.install_kind.can_install(),
                    restart_pending: matches!(s.phase, Phase::RestartPending { .. }),
                    failed_install: s.failed_install.clone(),
                };
                policy::next_step(&s.config, &state, Timestamp::now())
            };
            match step {
                Step::Nothing | Step::Announce(_) => return Ok(None),
                Step::Check => {
                    self.check().await?;
                }
                Step::Install(_) => {
                    let Some(available) = self.lock().available.clone() else {
                        return Ok(None);
                    };
                    let downloaded = self.download(&available).await?;
                    return self.install(&downloaded).await.map(Some);
                }
            }
        }
        Ok(None)
    }

    async fn get(&self, url: &str, limit: u64, api: bool) -> Result<Vec<u8>, UpdateError> {
        let mut body = Vec::new();
        self.fetch(url, limit, api, |_| {}, &mut body).await?;
        Ok(body)
    }

    /// GET `url` (following redirects), showing each chunk to `observe` and writing
    /// it to `sink`. More than `limit` bytes is an error.
    async fn fetch<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        url: &str,
        limit: u64,
        api: bool,
        mut observe: impl FnMut(&[u8]),
        sink: &mut W,
    ) -> Result<(), UpdateError> {
        let mut req = self.http.get(url);
        if api {
            req = req
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
        }
        let mut resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(UpdateError::Http {
                url: url.to_owned(),
                status: status.as_u16(),
            });
        }
        if resp.content_length().is_some_and(|n| n > limit) {
            return Err(UpdateError::TooLarge {
                url: url.to_owned(),
                limit,
            });
        }
        let mut received = 0u64;
        while let Some(chunk) = resp.chunk().await? {
            received += chunk.len() as u64;
            if received > limit {
                return Err(UpdateError::TooLarge {
                    url: url.to_owned(),
                    limit,
                });
            }
            observe(&chunk);
            sink.write_all(&chunk)
                .await
                .map_err(UpdateError::io("write the download"))?;
        }
        Ok(())
    }
}

fn install_blocking(
    downloaded: &Downloaded,
    source: &FeedSource,
    dir: &std::path::Path,
    target: &Target,
) -> Result<(PathBuf, PathBuf), UpdateError> {
    // Hash once more: the file sat on disk since it was checked.
    let bytes = std::fs::read(&downloaded.path).map_err(UpdateError::io("read the download"))?;
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    if digest != downloaded.release.sha256 {
        return Err(UpdateError::Checksum);
    }
    let staging = tempfile::Builder::new()
        .prefix("staging-")
        .tempdir_in(dir)
        .map_err(UpdateError::io("create a staging folder"))?;
    let binary = staging.path().join(feed::binary_name(&source.target));
    install::extract_binary(
        &downloaded.path,
        feed::archive_kind(&source.target),
        feed::binary_name(&source.target),
        &binary,
    )?;
    let out = install::replace_binary(&binary, target)?;
    let _ = std::fs::remove_file(&downloaded.path);
    Ok(out)
}
