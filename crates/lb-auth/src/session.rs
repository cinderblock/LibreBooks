//! Sessions: what a signed-in browser or device holds.
//!
//! A session is a 256-bit random token (`lbs_…`), handed out once as the
//! cookie value (or bearer token) and stored only as its SHA-256. Each request
//! presents it; [`AuthStore::validate_session`] checks it hasn't been revoked,
//! hasn't passed its absolute expiry, hasn't sat idle too long, and that its
//! user is still enabled, and answers with the [`Principal`].
//!
//! Defaults (see [`crate::AuthConfig`]): passkey and device sessions end after
//! 14 days unused or 90 days after sign-in; local desktop sessions after a day
//! unused or 30 days. `last_seen_at` is written at most once a minute per
//! session, so a busy page doesn't turn every read into a write.

use std::time::Duration;

use jiff::Timestamp;
use lb_core::UserId;
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::clock::{dur_ms, from_ms, ms};
use crate::error::{AuthError, Result};
use crate::model::{ClientInfo, NewSession, Principal, SessionId, SessionInfo, SessionKind};
use crate::perm::Permission;
use crate::{AuthStore, secret, users};

/// Longest label (user agent) kept with a session.
const MAX_LABEL: usize = 200;

/// Everything needed to make a session, beyond the user.
pub(crate) struct Spec<'a> {
    pub kind: SessionKind,
    /// The passkey or device key that signed in.
    pub credential_id: Option<String>,
    pub label: Option<String>,
    pub client: &'a ClientInfo,
    pub idle: Duration,
    pub absolute: Duration,
}

/// Make a session for `user` inside a write transaction.
pub(crate) fn create(
    c: &Connection,
    user: UserId,
    spec: &Spec<'_>,
    now: Timestamp,
) -> Result<NewSession> {
    let token = secret::new_token::<32>(secret::SESSION_PREFIX);
    let id = SessionId::new();
    let label = spec
        .label
        .as_deref()
        .or(spec.client.user_agent.as_deref())
        .map(|l| {
            l.chars()
                .filter(|c| !c.is_control())
                .take(MAX_LABEL)
                .collect::<String>()
        });
    let ip = spec.client.ip.map(|ip| ip.to_string());
    let expires_at = ms(now).saturating_add(dur_ms(spec.absolute));
    let idle = dur_ms(spec.idle);
    c.execute(
        "INSERT INTO session (id, token_hash, user_id, kind, credential_id, label, ip,
                              created_at, last_seen_at, expires_at, idle_timeout_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, ?10)",
        params![
            id.to_string(),
            secret::hash(&token).as_slice(),
            user.to_string(),
            spec.kind.as_str(),
            spec.credential_id,
            label,
            ip,
            ms(now),
            expires_at,
            idle,
        ],
    )?;
    Ok(NewSession {
        token,
        info: SessionInfo {
            id,
            kind: spec.kind,
            label,
            ip,
            created_at: from_ms(ms(now)),
            last_seen_at: from_ms(ms(now)),
            expires_at: from_ms(expires_at.min(ms(now).saturating_add(idle))),
            current: true,
        },
    })
}

/// End every live session of `user`.
pub(crate) fn revoke_all(c: &Connection, user: UserId, now: Timestamp) -> Result<usize> {
    Ok(c.execute(
        "UPDATE session SET revoked_at = ?2 WHERE user_id = ?1 AND revoked_at IS NULL",
        params![user.to_string(), ms(now)],
    )?)
}

/// End every live session signed in by one credential (passkey or device key).
pub(crate) fn revoke_by_credential(
    c: &Connection,
    credential_id: &str,
    now: Timestamp,
) -> Result<usize> {
    Ok(c.execute(
        "UPDATE session SET revoked_at = ?2 WHERE credential_id = ?1 AND revoked_at IS NULL",
        params![credential_id, ms(now)],
    )?)
}

impl AuthStore {
    /// Check a session token (the cookie value) and say who it belongs to.
    /// Every failure is [`AuthError::Unauthenticated`] (or `UserDisabled`), so
    /// callers answer 401 and the client signs in again.
    pub fn validate_session(&self, token: &str) -> Result<Principal> {
        let token = token.trim();
        if !token.starts_with(secret::SESSION_PREFIX) {
            return Err(AuthError::Unauthenticated);
        }
        let now = ms(self.now());
        let hash = secret::hash(token);
        let row = self.read(|c| {
            Ok(c.query_row(
                "SELECT s.id, s.user_id, s.kind, s.last_seen_at, s.expires_at, s.idle_timeout_ms,
                        s.revoked_at, u.display_name, u.role, u.disabled_at
                 FROM session s JOIN user u ON u.id = s.user_id
                 WHERE s.token_hash = ?1",
                [hash.as_slice()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, Option<i64>>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, String>(8)?,
                        r.get::<_, Option<i64>>(9)?,
                    ))
                },
            )
            .optional()?)
        })?;
        let Some((id, user_id, kind, last_seen, expires, idle, revoked, name, role, disabled)) =
            row
        else {
            return Err(AuthError::Unauthenticated);
        };
        if revoked.is_some() || now >= expires || now >= last_seen.saturating_add(idle) {
            return Err(AuthError::Unauthenticated);
        }
        if disabled.is_some() {
            return Err(AuthError::UserDisabled);
        }
        if now - last_seen >= dur_ms(self.inner.config.touch_interval) {
            self.write(|tx| {
                tx.execute(
                    "UPDATE session SET last_seen_at = ?2 WHERE id = ?1 AND last_seen_at < ?2",
                    params![id, now],
                )?;
                Ok(())
            })?;
        }
        Ok(Principal {
            user_id: users::parse_user_id(&user_id)?,
            display_name: name,
            role: role.parse()?,
            session_id: id
                .parse()
                .map_err(|_| AuthError::Corrupt(format!("bad session id {id:?}")))?,
            kind: kind.parse()?,
        })
    }

    /// Sign out: end the session with this token. Unknown or already-ended
    /// tokens are fine (signing out twice isn't an error).
    pub fn logout(&self, token: &str) -> Result<()> {
        let now = ms(self.now());
        let hash = secret::hash(token);
        self.write(|tx| {
            tx.execute(
                "UPDATE session SET revoked_at = ?2 WHERE token_hash = ?1 AND revoked_at IS NULL",
                params![hash.as_slice(), now],
            )?;
            Ok(())
        })
    }

    /// `user`'s live sessions, most recently used first. `current` marks the
    /// one making the request.
    pub fn list_sessions(
        &self,
        user: UserId,
        current: Option<SessionId>,
    ) -> Result<Vec<SessionInfo>> {
        let now = ms(self.now());
        self.read(|c| {
            let mut stmt = c.prepare(
                "SELECT id, kind, label, ip, created_at, last_seen_at, expires_at, idle_timeout_ms
                 FROM session
                 WHERE user_id = ?1 AND revoked_at IS NULL
                   AND expires_at > ?2 AND last_seen_at + idle_timeout_ms > ?2
                 ORDER BY last_seen_at DESC, id DESC",
            )?;
            let rows = stmt.query_map(params![user.to_string(), now], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, kind, label, ip, created, seen, expires, idle) = row?;
                let id: SessionId = id
                    .parse()
                    .map_err(|_| AuthError::Corrupt(format!("bad session id {id:?}")))?;
                out.push(SessionInfo {
                    id,
                    kind: kind.parse()?,
                    label,
                    ip,
                    created_at: from_ms(created),
                    last_seen_at: from_ms(seen),
                    expires_at: from_ms(expires.min(seen.saturating_add(idle))),
                    current: current == Some(id),
                });
            }
            Ok(out)
        })
    }

    /// End one session. You may end your own; admins may end anyone's.
    pub fn revoke_session(&self, by: &Principal, id: SessionId) -> Result<()> {
        let now = ms(self.now());
        self.write(|tx| {
            let owner: Option<String> = tx
                .query_row(
                    "SELECT user_id FROM session WHERE id = ?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            let not_found = || AuthError::NotFound {
                kind: "session",
                id: id.to_string(),
            };
            let owner = users::parse_user_id(&owner.ok_or_else(not_found)?)?;
            if owner != by.user_id && !by.can(Permission::ManageUsers) {
                // Don't confirm other people's session ids exist.
                return Err(not_found());
            }
            tx.execute(
                "UPDATE session SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![id.to_string(), now],
            )?;
            Ok(())
        })
    }

    /// End every session of `user` ("sign out everywhere"). You may do it to
    /// yourself; admins to anyone. Returns how many ended.
    pub fn revoke_user_sessions(&self, by: &Principal, user: UserId) -> Result<usize> {
        if by.user_id != user {
            by.require(Permission::ManageUsers)?;
        }
        let now = self.now();
        self.write(|tx| revoke_all(tx, user, now))
    }

    /// A session for the desktop app's in-process server (`Authorization:
    /// Bearer`), made without any sign-in because the app and the server are
    /// one process. Never reachable from the network: only the desktop app
    /// calls this.
    pub fn create_local_session(&self, user: UserId, label: Option<&str>) -> Result<NewSession> {
        let now = self.now();
        let config = &self.inner.config;
        let client = ClientInfo::default();
        self.write(|tx| {
            users::get_enabled(tx, user)?;
            create(
                tx,
                user,
                &Spec {
                    kind: SessionKind::Local,
                    credential_id: None,
                    label: label.map(str::to_owned),
                    client: &client,
                    idle: config.local_session_idle,
                    absolute: config.local_session_absolute,
                },
                now,
            )
        })
    }
}
