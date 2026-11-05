//! The alpha-software notice: which version each caller accepted, and when.
//!
//! The web UI asks before a person's first change and records the answer here.
//! People signed in with a session keep theirs on their user (the auth store).
//! The desktop app's own local session ("This computer") and open mode have no
//! user, so theirs is `notices.json` in the data dir: the desktop app serves its
//! UI from a new loopback port every launch, so the browser can't remember it.
//!
//! Nothing here stops a request: API clients and the CLI aren't people at a
//! screen. It's the UI that holds changes until the notice is accepted.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use lb_auth::NoticeAcceptance;
use serde::{Deserialize, Serialize};

use crate::auth::Principal;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const FILE: &str = "notices.json";

/// What `notices.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct NoticesFile {
    /// The alpha notice, as accepted on this computer without a session.
    alpha: Option<NoticeAcceptance>,
}

/// This computer's own acceptance (no session, no user).
#[derive(Debug)]
pub struct LocalNotices {
    path: PathBuf,
    file: Mutex<NoticesFile>,
}

impl LocalNotices {
    /// Read `notices.json` in `data_dir`. A missing file is nothing accepted yet;
    /// a damaged one is logged and treated the same (the next acceptance
    /// replaces it), so the notice is asked for again rather than the server
    /// refusing to start.
    pub fn open(data_dir: &Path) -> LocalNotices {
        let path = data_dir.join(FILE);
        let file = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!(path = %path.display(), error = %e, "unreadable notices file; asking again");
                NoticesFile::default()
            }),
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path = %path.display(), error = %e, "can't read the notices file; asking again");
                }
                NoticesFile::default()
            }
        };
        LocalNotices {
            path,
            file: Mutex::new(file),
        }
    }

    pub fn alpha(&self) -> Option<NoticeAcceptance> {
        self.lock().alpha
    }

    /// Accept `version` now (see [`NoticeAcceptance::accept`]) and save.
    pub fn accept_alpha(&self, version: u32) -> std::io::Result<NoticeAcceptance> {
        let mut file = self.lock();
        let accepted = NoticeAcceptance::accept(file.alpha, version, jiff::Timestamp::now());
        if file.alpha != Some(accepted) {
            let next = NoticesFile {
                alpha: Some(accepted),
            };
            save(&self.path, &next)?;
            *file = next;
        }
        Ok(accepted)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, NoticesFile> {
        self.file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Write the file in one step (a reader never sees half of it).
fn save(path: &Path, file: &NoticesFile) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut text = serde_json::to_string_pretty(file).map_err(std::io::Error::other)?;
    text.push('\n');
    let mut tmp = tempfile::Builder::new()
        .prefix(".notices-")
        .tempfile_in(dir)?;
    std::io::Write::write_all(&mut tmp, text.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// What `p` accepted: their user's, or this computer's without a session.
pub fn alpha_for(s: &AppState, p: &Principal) -> ApiResult<Option<NoticeAcceptance>> {
    match (&p.session, &s.sessions) {
        (Some(sp), Some(sessions)) => Ok(sessions.store.user(sp.user_id)?.alpha_notice),
        _ => Ok(s.ops.notices.alpha()),
    }
}

/// `p` accepts the alpha notice `version`.
pub fn accept_alpha(s: &AppState, p: &Principal, version: u32) -> ApiResult<NoticeAcceptance> {
    if version == 0 {
        return Err(ApiError::invalid("the notice version starts at 1"));
    }
    match (&p.session, &s.sessions) {
        (Some(sp), Some(sessions)) => Ok(sessions.store.accept_alpha_notice(sp, version)?),
        _ => s
            .ops
            .notices
            .accept_alpha(version)
            .map_err(|e| ApiError::internal(format!("could not save the notice: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_acceptance_survives_a_restart_and_never_goes_back() {
        let dir = tempfile::tempdir().unwrap();
        let notices = LocalNotices::open(dir.path());
        assert_eq!(notices.alpha(), None);
        let first = notices.accept_alpha(2).unwrap();
        assert_eq!(notices.accept_alpha(1).unwrap(), first);

        let reopened = LocalNotices::open(dir.path());
        assert_eq!(reopened.alpha(), Some(first));
    }

    #[test]
    fn a_damaged_file_asks_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), b"{ not json").unwrap();
        let notices = LocalNotices::open(dir.path());
        assert_eq!(notices.alpha(), None);
        assert_eq!(notices.accept_alpha(1).unwrap().version, 1);
        assert_eq!(
            LocalNotices::open(dir.path()).alpha().map(|a| a.version),
            Some(1)
        );
    }
}
