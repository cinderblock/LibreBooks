//! Users: the people who can sign in, and admin management of them.
//!
//! A user is a name and a role; their credentials (passkeys, device keys) and
//! sessions hang off them. There is always at least one enabled admin once the
//! server is set up: the last one can't be disabled, demoted or deleted, so a
//! server can't lock itself out of user management.

use jiff::Timestamp;
use lb_core::UserId;
use rusqlite::{Connection, OptionalExtension as _, Row, params};

use crate::clock::{from_ms, ms, opt_from_ms};
use crate::error::{AuthError, Result};
use crate::model::{NoticeAcceptance, Principal, Role, User, UserUpdate, clean_name};
use crate::perm::Permission;
use crate::{AuthStore, session};

const COLUMNS: &str =
    "id, display_name, role, created_at, disabled_at, alpha_notice_version, alpha_notice_at";

/// A `user` row, as [`COLUMNS`] reads it.
struct UserRow {
    id: String,
    display_name: String,
    role: String,
    created_at: i64,
    disabled_at: Option<i64>,
    alpha_notice_version: Option<u32>,
    alpha_notice_at: Option<i64>,
}

fn from_row(r: &Row<'_>) -> rusqlite::Result<UserRow> {
    Ok(UserRow {
        id: r.get(0)?,
        display_name: r.get(1)?,
        role: r.get(2)?,
        created_at: r.get(3)?,
        disabled_at: r.get(4)?,
        alpha_notice_version: r.get(5)?,
        alpha_notice_at: r.get(6)?,
    })
}

fn to_user(row: UserRow) -> Result<User> {
    let alpha_notice = match (row.alpha_notice_version, row.alpha_notice_at) {
        (Some(version), Some(at)) => Some(NoticeAcceptance {
            version,
            accepted_at: from_ms(at),
        }),
        _ => None,
    };
    Ok(User {
        id: parse_user_id(&row.id)?,
        display_name: row.display_name,
        role: row.role.parse()?,
        created_at: from_ms(row.created_at),
        disabled_at: opt_from_ms(row.disabled_at),
        alpha_notice,
    })
}

pub(crate) fn parse_user_id(s: &str) -> Result<UserId> {
    s.parse()
        .map_err(|_| AuthError::Corrupt(format!("bad user id {s:?}")))
}

/// The user with this id.
pub(crate) fn get(c: &Connection, id: UserId) -> Result<User> {
    find(c, id)?.ok_or_else(|| AuthError::NotFound {
        kind: "user",
        id: id.to_string(),
    })
}

pub(crate) fn find(c: &Connection, id: UserId) -> Result<Option<User>> {
    c.query_row(
        &format!("SELECT {COLUMNS} FROM user WHERE id = ?1"),
        [id.to_string()],
        from_row,
    )
    .optional()?
    .map(to_user)
    .transpose()
}

/// The user with this id, who must be enabled.
pub(crate) fn get_enabled(c: &Connection, id: UserId) -> Result<User> {
    let user = get(c, id)?;
    if user.disabled_at.is_some() {
        return Err(AuthError::UserDisabled);
    }
    Ok(user)
}

pub(crate) fn insert(
    c: &Connection,
    id: UserId,
    display_name: &str,
    role: Role,
    now: Timestamp,
) -> Result<User> {
    let display_name = clean_name("a name", display_name)?;
    c.execute(
        "INSERT INTO user (id, display_name, role, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id.to_string(), display_name, role.as_str(), ms(now)],
    )?;
    get(c, id)
}

pub(crate) fn enabled_admins(c: &Connection) -> Result<i64> {
    Ok(c.query_row(
        "SELECT COUNT(*) FROM user WHERE role = 'admin' AND disabled_at IS NULL",
        [],
        |r| r.get(0),
    )?)
}

/// Refuse to remove `user`'s only remaining credential when they do it
/// themselves: they would lock themselves out with no way back short of an
/// admin. (Admins removing someone else's are not stopped; that's on purpose
/// when a device is lost or a person leaves.)
pub(crate) fn ensure_not_last_credential(c: &Connection, user: UserId) -> Result<()> {
    let n: i64 = c.query_row(
        "SELECT (SELECT COUNT(*) FROM passkey WHERE user_id = ?1)
              + (SELECT COUNT(*) FROM device_key WHERE user_id = ?1 AND revoked_at IS NULL)",
        [user.to_string()],
        |r| r.get(0),
    )?;
    if n <= 1 {
        return Err(AuthError::Conflict(
            "this is your only way to sign in; add another passkey or device first".to_owned(),
        ));
    }
    Ok(())
}

impl AuthStore {
    /// Every user, oldest first.
    pub fn list_users(&self) -> Result<Vec<User>> {
        self.read(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {COLUMNS} FROM user ORDER BY created_at, id"
            ))?;
            let rows = stmt.query_map([], from_row)?;
            rows.map(|r| to_user(r?)).collect()
        })
    }

    /// One user.
    pub fn user(&self, id: UserId) -> Result<User> {
        self.read(|c| get(c, id))
    }

    /// Whether any user exists (a fresh server has none).
    pub fn has_users(&self) -> Result<bool> {
        self.read(|c| Ok(c.query_row("SELECT EXISTS (SELECT 1 FROM user)", [], |r| r.get(0))?))
    }

    /// Make a user directly, without an enrolment: for the server operator's
    /// CLI and the desktop app's local owner. They can't sign in remotely
    /// until they enrol a passkey or device with an `add_device` token.
    pub fn create_user(&self, display_name: &str, role: Role) -> Result<User> {
        let now = self.now();
        self.write(|tx| insert(tx, UserId::new(), display_name, role, now))
    }

    /// Change a user (`PATCH /users/:id`). Admins may change anything; anyone
    /// may change their own display name. Disabling ends every session the
    /// user has and cancels their outstanding enrolment links.
    pub fn update_user(&self, by: &Principal, id: UserId, update: &UserUpdate) -> Result<User> {
        let own_name_only = by.user_id == id && update.role.is_none() && update.disabled.is_none();
        if !own_name_only {
            by.require(Permission::ManageUsers)?;
        }
        let now = self.now();
        self.write(|tx| {
            let user = get(tx, id)?;
            if let Some(name) = &update.display_name {
                let name = clean_name("a name", name)?;
                tx.execute(
                    "UPDATE user SET display_name = ?2 WHERE id = ?1",
                    params![id.to_string(), name],
                )?;
            }
            let was_active_admin = user.role == Role::Admin && user.disabled_at.is_none();
            let demoted = update.role.is_some_and(|r| r != Role::Admin);
            let disabling = update.disabled == Some(true);
            if was_active_admin && (demoted || disabling) && enabled_admins(tx)? <= 1 {
                return Err(AuthError::Conflict(
                    "this is the only admin; make someone else an admin first".to_owned(),
                ));
            }
            if let Some(role) = update.role {
                tx.execute(
                    "UPDATE user SET role = ?2 WHERE id = ?1",
                    params![id.to_string(), role.as_str()],
                )?;
            }
            match update.disabled {
                Some(true) if user.disabled_at.is_none() => {
                    tx.execute(
                        "UPDATE user SET disabled_at = ?2 WHERE id = ?1",
                        params![id.to_string(), ms(now)],
                    )?;
                    session::revoke_all(tx, id, now)?;
                    tx.execute(
                        "UPDATE enrolment SET revoked_at = ?2
                         WHERE user_id = ?1 AND used_at IS NULL AND revoked_at IS NULL",
                        params![id.to_string(), ms(now)],
                    )?;
                }
                Some(false) => {
                    tx.execute(
                        "UPDATE user SET disabled_at = NULL WHERE id = ?1",
                        [id.to_string()],
                    )?;
                }
                _ => {}
            }
            get(tx, id)
        })
    }

    /// Record that `by` accepted the web UI's alpha-software notice `version`.
    /// The stored version never goes down, and accepting the same one again
    /// keeps the first time. Anyone signed in may; it is only about themselves.
    pub fn accept_alpha_notice(&self, by: &Principal, version: u32) -> Result<NoticeAcceptance> {
        // As stored (milliseconds), so the answer matches what's read back later.
        let now = from_ms(ms(self.now()));
        self.write(|tx| {
            let user = get(tx, by.user_id)?;
            let accepted = NoticeAcceptance::accept(user.alpha_notice, version, now);
            if Some(accepted) != user.alpha_notice {
                tx.execute(
                    "UPDATE user SET alpha_notice_version = ?2, alpha_notice_at = ?3 WHERE id = ?1",
                    params![
                        by.user_id.to_string(),
                        accepted.version,
                        ms(accepted.accepted_at)
                    ],
                )?;
            }
            Ok(accepted)
        })
    }

    /// Delete a user and everything of theirs here (credentials, sessions,
    /// company access). Their name stays in company audit trails, which keep
    /// their own copy. You can't delete yourself or the last admin.
    pub fn delete_user(&self, by: &Principal, id: UserId) -> Result<()> {
        by.require(Permission::ManageUsers)?;
        if by.user_id == id {
            return Err(AuthError::Conflict(
                "you can't delete your own account; ask another admin".to_owned(),
            ));
        }
        self.write(|tx| {
            let user = get(tx, id)?;
            if user.role == Role::Admin && user.disabled_at.is_none() && enabled_admins(tx)? <= 1 {
                return Err(AuthError::Conflict(
                    "this is the only admin; make someone else an admin first".to_owned(),
                ));
            }
            tx.execute("DELETE FROM user WHERE id = ?1", [id.to_string()])?;
            Ok(())
        })
    }
}
