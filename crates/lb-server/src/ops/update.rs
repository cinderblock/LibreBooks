//! Self-update for the headless server: one [`lb_update::Updater`] per process,
//! a timer that lets its policy decide (check daily, install only when the
//! settings say so), and the restart after an install.
//!
//! An updater exists only when this build carries a release signing key; a build
//! without one says so in `GET /server/update` rather than trusting anything.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use lb_update::{
    Build, CheckOutcome, FeedSource, Installed, Target, TrustedKey, UpdateConfig, UpdateError,
    UpdateStatus, Updater, UpdaterOptions,
};
use serde::Serialize;
use tokio::sync::watch;

use super::UpdaterChoice;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// How often the timer asks the policy what to do (it checks only when the
/// configured interval has passed).
const TICK_EVERY: Duration = Duration::from_secs(15 * 60);
/// The first tick, a little after start so a crash-looping server doesn't hammer
/// GitHub.
const FIRST_TICK: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub(crate) struct UpdateSlot {
    updater: Option<Arc<Updater>>,
    unavailable: Option<String>,
    /// One check or install at a time.
    busy: tokio::sync::Mutex<()>,
}

/// The options for LibreBooks' own releases, or why there are none.
pub fn release_options(data_dir: &Path, config: &UpdateConfig) -> Result<UpdaterOptions, String> {
    let key = TrustedKey::built_in().map_err(|e| e.to_string())?;
    let current = Build::parse(env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())?;
    let install_kind = lb_update::install::detect_current(&current);
    Ok(UpdaterOptions {
        source: FeedSource::default(),
        key,
        current,
        config: config.clone(),
        install_kind,
        target: Target::RunningExe,
        download_dir: data_dir.join("updates"),
        download_limit: lb_update::DOWNLOAD_LIMIT,
        smoke_test: true,
    })
}

impl UpdateSlot {
    pub(crate) fn new(choice: UpdaterChoice, data_dir: &Path, config: &UpdateConfig) -> Self {
        let options = match choice {
            UpdaterChoice::Releases => release_options(data_dir, config),
            UpdaterChoice::Custom(o) => Ok(UpdaterOptions {
                config: config.clone(),
                ..*o
            }),
            UpdaterChoice::Off(why) => Err(why),
        };
        let (updater, unavailable) =
            match options.and_then(|o| Updater::new(o).map_err(|e| e.to_string())) {
                Ok(u) => (Some(Arc::new(u)), None),
                Err(why) => (None, Some(why)),
            };
        UpdateSlot {
            updater,
            unavailable,
            busy: tokio::sync::Mutex::new(()),
        }
    }

    pub(crate) fn set_config(&self, config: &UpdateConfig) {
        if let Some(u) = &self.updater {
            u.set_config(config.clone());
        }
    }

    fn get(&self) -> ApiResult<&Arc<Updater>> {
        self.updater.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "unavailable",
                self.unavailable
                    .clone()
                    .unwrap_or_else(|| "self-update is off".into()),
            )
        })
    }
}

/// What `GET /server/update` shows.
#[derive(Clone, Debug, Serialize)]
pub struct UpdateView {
    pub config: UpdateConfig,
    /// `None` when this build cannot update itself; `unavailable` says why.
    pub status: Option<UpdateStatus>,
    pub unavailable: Option<String>,
}

pub fn view(state: &AppState) -> UpdateView {
    let slot = &state.ops.update;
    UpdateView {
        config: state.ops.settings().updates,
        status: slot.updater.as_ref().map(|u| u.status()),
        unavailable: slot.unavailable.clone(),
    }
}

/// Check now.
pub async fn check(state: &AppState) -> ApiResult<UpdateView> {
    let slot = &state.ops.update;
    let updater = slot.get()?;
    let _one = slot.busy.lock().await;
    updater.check().await.map_err(ApiError::from)?;
    Ok(view(state))
}

/// Download and install the newest release (checking first if nothing is
/// known), then restart if asked.
pub async fn install(state: &AppState, restart: bool) -> ApiResult<Installed> {
    let slot = &state.ops.update;
    let updater = slot.get()?;
    let _one = slot.busy.lock().await;
    let available = match updater.status().available {
        Some(a) => a,
        None => match updater.check().await.map_err(ApiError::from)? {
            CheckOutcome::Available(a) => a,
            CheckOutcome::UpToDate { .. } => {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "up_to_date",
                    "this is the newest release",
                ));
            }
        },
    };
    let downloaded = updater.download(&available).await.map_err(ApiError::from)?;
    let installed = updater.install(&downloaded).await.map_err(ApiError::from)?;
    if restart {
        state.ops.request_restart(installed.restart.clone());
    }
    Ok(installed)
}

/// The timer.
pub(crate) async fn run(state: AppState, mut stop: watch::Receiver<bool>) {
    let Some(updater) = state.ops.update.updater.clone() else {
        if let Some(why) = &state.ops.update.unavailable {
            tracing::info!("self-update is unavailable: {why}");
        }
        return;
    };
    let mut wait = FIRST_TICK;
    loop {
        if super::nap(wait, &mut stop).await {
            break;
        }
        wait = TICK_EVERY;
        let result = {
            let _one = state.ops.update.busy.lock().await;
            updater.tick().await
        };
        match result {
            Ok(Some(installed)) => {
                tracing::info!(version = %installed.version, "update installed; restarting");
                state.ops.request_restart(installed.restart);
                break;
            }
            Ok(None) => {
                if let Some(a) = updater.status().available {
                    tracing::info!(version = %a.version, page = %a.page, "an update is available");
                }
            }
            Err(e) => tracing::warn!(error = %e, "update check failed"),
        }
    }
}

impl From<UpdateError> for ApiError {
    fn from(e: UpdateError) -> Self {
        let message = e.to_string();
        match e {
            UpdateError::Network(_) | UpdateError::Http { .. } | UpdateError::Feed(_) => {
                ApiError::new(StatusCode::BAD_GATEWAY, "network", message)
            }
            UpdateError::Trust(_)
            | UpdateError::Checksum
            | UpdateError::NoAsset { .. }
            | UpdateError::SmokeTest(_) => {
                ApiError::new(StatusCode::BAD_GATEWAY, "untrusted", message)
            }
            UpdateError::NotSelfUpdatable(_) | UpdateError::NothingToRollBack => {
                ApiError::new(StatusCode::CONFLICT, "conflict", message)
            }
            UpdateError::TooLarge { .. } | UpdateError::Archive(_) | UpdateError::Io { .. } => {
                ApiError::internal(message)
            }
        }
    }
}
