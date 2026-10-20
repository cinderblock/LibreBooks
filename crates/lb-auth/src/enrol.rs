//! Enrolment tokens: the one way in.
//!
//! An enrolment token is a 160-bit random secret (`lbe_…`) that lets its holder
//! register one passkey or device key: as a new user (`new_user`, with the
//! role the admin chose), as another device of an existing user
//! (`add_device`), or as the server's first admin (`first_admin`, minted by
//! [`AuthStore::bootstrap`] and printed at startup). Tokens expire quickly (15
//! minutes by default) and work once: redeeming one is a single
//! `UPDATE … WHERE used_at IS NULL AND expires_at > now` inside the same
//! IMMEDIATE transaction that creates the user, credential and session, and
//! only the caller whose update changed the row gets through. Only the hash is
//! stored.
//!
//! Starting a passkey enrolment only *checks* the token (the browser ceremony
//! may still be abandoned); finishing it redeems the token.

use std::time::Duration;

use jiff::Timestamp;
use lb_core::{CompanyId, UserId};
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::clock::{self, from_ms, ms};
use crate::error::{AuthError, Result};
use crate::model::{
    ClientInfo, EnrolmentId, EnrolmentInfo, EnrolmentPurpose, MintedEnrolment, NewEnrolment,
    Principal, Role, User, clean_name,
};
use crate::perm::Permission;
use crate::{AuthStore, secret, users};

/// An outstanding token, as read back from the database.
#[derive(Clone, Debug)]
pub(crate) struct TokenRow {
    pub id: EnrolmentId,
    pub purpose: EnrolmentPurpose,
    pub role: Option<Role>,
    pub user_id: Option<UserId>,
    pub company_id: Option<CompanyId>,
    pub display_name: Option<String>,
    pub created_by: Option<UserId>,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

impl TokenRow {
    fn info(&self) -> EnrolmentInfo {
        EnrolmentInfo {
            id: self.id,
            purpose: self.purpose,
            role: self.role,
            user_id: self.user_id,
            company_id: self.company_id,
            display_name: self.display_name.clone(),
            created_by: self.created_by,
            created_at: self.created_at,
            expires_at: self.expires_at,
        }
    }
}

const COLUMNS: &str = "id, purpose, role, user_id, company_id, display_name, created_by, \
                       created_at, expires_at, used_at, revoked_at";

type RawRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<i64>,
    Option<i64>,
);

fn raw(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
    ))
}

/// A parsed row plus its `used_at` / `revoked_at`.
fn parse(row: RawRow) -> Result<(TokenRow, Option<i64>, Option<i64>)> {
    let (
        id,
        purpose,
        role,
        user_id,
        company_id,
        display_name,
        created_by,
        created,
        expires,
        used,
        revoked,
    ) = row;
    let company_id = company_id
        .map(|c| {
            c.parse::<CompanyId>()
                .map_err(|_| AuthError::Corrupt(format!("bad company id {c:?}")))
        })
        .transpose()?;
    Ok((
        TokenRow {
            id: id
                .parse()
                .map_err(|_| AuthError::Corrupt(format!("bad enrolment id {id:?}")))?,
            purpose: purpose.parse()?,
            role: role.map(|r| r.parse()).transpose()?,
            user_id: user_id.as_deref().map(users::parse_user_id).transpose()?,
            company_id,
            display_name,
            created_by: created_by
                .as_deref()
                .map(users::parse_user_id)
                .transpose()?,
            created_at: from_ms(created),
            expires_at: from_ms(expires),
        },
        used,
        revoked,
    ))
}

/// The token with this hash, if it may still be used; otherwise the reason it
/// can't.
pub(crate) fn check(c: &Connection, hash: &[u8; 32], now: Timestamp) -> Result<TokenRow> {
    let row = c
        .query_row(
            &format!("SELECT {COLUMNS} FROM enrolment WHERE token_hash = ?1"),
            [hash.as_slice()],
            raw,
        )
        .optional()?;
    let Some(row) = row else {
        return Err(AuthError::TokenNotFound);
    };
    let (token, used, revoked) = parse(row)?;
    if used.is_some() {
        return Err(AuthError::TokenUsed);
    }
    if revoked.is_some() {
        return Err(AuthError::TokenRevoked);
    }
    if ms(now) >= ms(token.expires_at) {
        return Err(AuthError::TokenExpired);
    }
    Ok(token)
}

/// What redeeming a token produced.
pub(crate) struct Redeemed {
    pub user: User,
    pub token: TokenRow,
}

/// Redeem a token inside a write transaction: check it, make or find the user
/// it's for, and mark it used. `new_user_id` is the id a new user gets (a
/// passkey ceremony fixed it at the start, as the WebAuthn user handle);
/// `display_name` is what the person typed, if anything.
///
/// The caller adds the credential and session in the same transaction, so
/// either all of it happens or none of it does.
pub(crate) fn redeem(
    c: &Connection,
    hash: &[u8; 32],
    now: Timestamp,
    new_user_id: UserId,
    display_name: Option<&str>,
) -> Result<Redeemed> {
    let token = check(c, hash, now)?;
    let typed = display_name.map(str::trim).filter(|n| !n.is_empty());
    let user = match token.purpose {
        EnrolmentPurpose::FirstAdmin => {
            if users::enabled_admins(c)? > 0 {
                return Err(AuthError::AdminExists);
            }
            let name = typed
                .or(token.display_name.as_deref())
                .unwrap_or("Administrator");
            users::insert(c, new_user_id, name, Role::Admin, now)?
        }
        EnrolmentPurpose::NewUser => {
            let name = typed
                .or(token.display_name.as_deref())
                .ok_or_else(|| AuthError::Invalid("please enter your name".to_owned()))?;
            let role = token.role.unwrap_or(Role::Bookkeeper);
            let user = users::insert(c, new_user_id, name, role, now)?;
            if let Some(company) = token.company_id {
                c.execute(
                    "INSERT INTO company_access (user_id, company_id, role) VALUES (?1, ?2, ?3)",
                    params![user.id.to_string(), company.to_string(), role.as_str()],
                )?;
            }
            user
        }
        EnrolmentPurpose::AddDevice => {
            let id = token
                .user_id
                .ok_or_else(|| AuthError::Corrupt("add_device token without a user".to_owned()))?;
            users::get_enabled(c, id)?
        }
    };
    let changed = c.execute(
        "UPDATE enrolment SET used_at = ?2, used_by = ?3
         WHERE token_hash = ?1 AND used_at IS NULL AND revoked_at IS NULL AND expires_at > ?2",
        params![hash.as_slice(), ms(now), user.id.to_string()],
    )?;
    if changed != 1 {
        // Unreachable inside an IMMEDIATE transaction that just checked it, but
        // this update is the real guard, so honour it.
        return Err(AuthError::TokenUsed);
    }
    Ok(Redeemed { user, token })
}

impl AuthStore {
    /// First-run setup: if the server has no enabled admin, mint a
    /// `first_admin` token (valid for an hour) for the server to print, and
    /// cancel any earlier unused ones so only the one just printed works.
    /// Returns `None`, and mints nothing, once an admin exists.
    pub fn bootstrap(&self) -> Result<Option<MintedEnrolment>> {
        let now = self.now();
        let ttl = self.inner.config.first_admin_ttl;
        self.write(|tx| {
            if users::enabled_admins(tx)? > 0 {
                return Ok(None);
            }
            tx.execute(
                "UPDATE enrolment SET revoked_at = ?1
                 WHERE purpose = 'first_admin' AND used_at IS NULL AND revoked_at IS NULL",
                [ms(now)],
            )?;
            let minted = insert(
                tx,
                &Validated {
                    purpose: EnrolmentPurpose::FirstAdmin,
                    role: Some(Role::Admin),
                    user_id: None,
                    company_id: None,
                    display_name: None,
                    ttl,
                },
                None,
                now,
            )?;
            tracing::info!(id = %minted.id, "minted the first-admin enrolment token");
            Ok(Some(minted))
        })
    }

    /// Mint an enrolment token for someone (`POST /auth/enrollments`). Admins
    /// may mint any `new_user` or `add_device` token; anyone may mint an
    /// `add_device` token for themselves (to add their phone, say).
    pub fn create_enrolment(&self, by: &Principal, req: &NewEnrolment) -> Result<MintedEnrolment> {
        let for_self = req.purpose == EnrolmentPurpose::AddDevice
            && req.user_id.is_none_or(|u| u == by.user_id);
        if !for_self {
            by.require(Permission::ManageUsers)?;
        }
        let mut req = req.clone();
        if req.purpose == EnrolmentPurpose::AddDevice && req.user_id.is_none() {
            req.user_id = Some(by.user_id);
        }
        self.mint(&req, Some(by.user_id))
    }

    /// Mint an enrolment token as the server's operator (someone with a shell
    /// on the server: `librebooks enrol`). No permission check: having the
    /// database file is already everything. Never wire this to the network.
    pub fn operator_enrolment(&self, req: &NewEnrolment) -> Result<MintedEnrolment> {
        self.mint(req, None)
    }

    fn mint(&self, req: &NewEnrolment, created_by: Option<UserId>) -> Result<MintedEnrolment> {
        let now = self.now();
        let v = self.validate_request(req)?;
        self.write(|tx| {
            if let Some(user) = v.user_id {
                users::get_enabled(tx, user)?;
            }
            insert(tx, &v, created_by, now)
        })
    }

    fn validate_request(&self, req: &NewEnrolment) -> Result<Validated> {
        let config = &self.inner.config;
        let ttl = match req.ttl_minutes {
            None => config.enrolment_ttl,
            Some(0) => {
                return Err(AuthError::Invalid(
                    "a link must last at least a minute".to_owned(),
                ));
            }
            Some(m) => Duration::from_secs(u64::from(m) * 60),
        };
        if ttl > config.max_enrolment_ttl {
            return Err(AuthError::Invalid(format!(
                "a link can last at most {} minutes",
                config.max_enrolment_ttl.as_secs() / 60
            )));
        }
        let display_name = req
            .display_name
            .as_deref()
            .filter(|n| !n.trim().is_empty())
            .map(|n| clean_name("a name", n))
            .transpose()?;
        match req.purpose {
            EnrolmentPurpose::FirstAdmin => Err(AuthError::Invalid(
                "first-admin links are only made when the server starts with no admin".to_owned(),
            )),
            EnrolmentPurpose::NewUser => {
                if req.user_id.is_some() {
                    return Err(AuthError::Invalid(
                        "a new-user link can't name an existing user".to_owned(),
                    ));
                }
                Ok(Validated {
                    purpose: req.purpose,
                    role: Some(req.role.unwrap_or(Role::Bookkeeper)),
                    user_id: None,
                    company_id: req.company_id,
                    display_name,
                    ttl,
                })
            }
            EnrolmentPurpose::AddDevice => {
                let user_id = req.user_id.ok_or_else(|| {
                    AuthError::Invalid("say whose device this link adds".to_owned())
                })?;
                if req.role.is_some() || req.company_id.is_some() {
                    return Err(AuthError::Invalid(
                        "an add-device link can't change roles or company access".to_owned(),
                    ));
                }
                Ok(Validated {
                    purpose: req.purpose,
                    role: None,
                    user_id: Some(user_id),
                    company_id: None,
                    display_name: None,
                    ttl,
                })
            }
        }
    }

    /// Tokens that can still be used, newest first (for the admin page).
    pub fn list_enrolments(&self) -> Result<Vec<EnrolmentInfo>> {
        let now = ms(self.now());
        self.read(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {COLUMNS} FROM enrolment
                 WHERE used_at IS NULL AND revoked_at IS NULL AND expires_at > ?1
                 ORDER BY created_at DESC, id DESC"
            ))?;
            let rows = stmt.query_map([now], raw)?;
            rows.map(|r| Ok(parse(r?)?.0.info())).collect()
        })
    }

    /// Cancel an outstanding token. Admins may cancel any; others only the
    /// ones they made.
    pub fn revoke_enrolment(&self, by: &Principal, id: EnrolmentId) -> Result<()> {
        let now = ms(self.now());
        self.write(|tx| {
            let created_by: Option<Option<String>> = tx
                .query_row(
                    "SELECT created_by FROM enrolment WHERE id = ?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            let not_found = || AuthError::NotFound {
                kind: "enrolment link",
                id: id.to_string(),
            };
            let created_by = created_by.ok_or_else(not_found)?;
            let mine = created_by.as_deref() == Some(by.user_id.to_string().as_str());
            if !mine && !by.can(Permission::ManageUsers) {
                return Err(not_found());
            }
            tx.execute(
                "UPDATE enrolment SET revoked_at = ?2 WHERE id = ?1 AND used_at IS NULL AND revoked_at IS NULL",
                params![id.to_string(), now],
            )?;
            Ok(())
        })
    }

    /// What a token is for, without using it: lets a client decide whether to
    /// ask for a name before enrolling. Failures count against the client's
    /// rate limit.
    pub fn inspect_enrolment(&self, token: &str, client: &ClientInfo) -> Result<EnrolmentInfo> {
        self.guarded(client, || {
            let now = self.now();
            let hash = secret::hash(token);
            self.read(|c| check(c, &hash, now)).map(|t| t.info())
        })
    }
}

/// A checked mint request.
struct Validated {
    purpose: EnrolmentPurpose,
    role: Option<Role>,
    user_id: Option<UserId>,
    company_id: Option<CompanyId>,
    display_name: Option<String>,
    ttl: Duration,
}

fn insert(
    c: &Connection,
    v: &Validated,
    created_by: Option<UserId>,
    now: Timestamp,
) -> Result<MintedEnrolment> {
    let token = secret::new_token::<20>(secret::ENROLMENT_PREFIX);
    let id = EnrolmentId::new();
    let expires_at = clock::add(now, v.ttl);
    c.execute(
        "INSERT INTO enrolment (id, token_hash, purpose, role, user_id, company_id, display_name,
                                created_by, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id.to_string(),
            secret::hash(&token).as_slice(),
            v.purpose.as_str(),
            v.role.map(Role::as_str),
            v.user_id.map(|u| u.to_string()),
            v.company_id.map(|c| c.to_string()),
            v.display_name,
            created_by.map(|u| u.to_string()),
            ms(now),
            ms(expires_at),
        ],
    )?;
    Ok(MintedEnrolment {
        id,
        token,
        purpose: v.purpose,
        expires_at: from_ms(ms(expires_at)),
    })
}
