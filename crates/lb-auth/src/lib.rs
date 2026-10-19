//! Authentication: passkeys, enrolment tokens, device keys and sessions.
//!
//! LibreBooks has no passwords. People sign in with **passkeys** (WebAuthn);
//! desktop thin clients whose webview has no WebAuthn (WebKitGTK on Linux) sign
//! in with a **device key** (Ed25519, private half in the OS keychain). Both are
//! enrolled with a short-lived, single-use **enrolment token** that an admin
//! mints (or the server prints on first run for the first admin). Signing in
//! makes a server-side **session** whose random token is the cookie value.
//!
//! This crate is a library with no HTTP in it: `lb-server` maps it onto the
//! `/auth/*`, `/session` and `/users` endpoints in `plans/api.md`. Every method
//! that the network can reach takes the request's [`ClientInfo`] (for rate
//! limiting and the session list) and, where WebAuthn binds to it, the
//! request's `Origin`.
//!
//! [`AuthStore`] is the whole thing: its own SQLite file (`server.db`, never a
//! company file), the in-memory state of sign-in ceremonies in flight, and the
//! failed-attempt rate limiter. Design decisions and their reasons are in
//! `plans/auth.md`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OpenFlags, Transaction};

mod ceremony;
mod clock;
mod config;
mod device;
mod enrol;
mod error;
mod model;
mod passkey;
mod perm;
mod ratelimit;
mod schema;
mod secret;
mod session;
mod users;

pub use ceremony::ChallengeId;
pub use clock::Clock;
pub use config::{AuthConfig, RateLimit, RelyingParty};
pub use device::device_login_message;
pub use error::{AuthError, Result};
pub use model::{
    ClientInfo, CompanyAccess, DeviceChallenge, DeviceEnrol, DeviceKeyId, DeviceKeyInfo,
    DeviceLogin, EnrolFinish, EnrolStart, EnrolStartRequest, EnrolmentId, EnrolmentInfo,
    EnrolmentPurpose, LoginFinish, LoginStart, MintedEnrolment, NewEnrolment, NewSession,
    PasskeyId, PasskeyInfo, Principal, Role, SessionId, SessionInfo, SessionKind, User, UserUpdate,
};
pub use perm::Permission;

pub use lb_core::{CompanyId, UserId};

/// "LBau": marks a SQLite file as a LibreBooks server auth database.
pub const APPLICATION_ID: i32 = 0x4C42_6175;

struct Inner {
    conn: Mutex<Connection>,
    path: PathBuf,
    config: AuthConfig,
    rps: Vec<config::Rp>,
    ceremonies: ceremony::Ceremonies,
    limiter: ratelimit::RateLimiter,
}

/// The server's users, credentials and sessions. Cheap to clone; all clones
/// share one connection, one set of ceremonies in flight and one rate limiter.
#[derive(Clone)]
pub struct AuthStore {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for AuthStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthStore")
            .field("path", &self.inner.path)
            .finish_non_exhaustive()
    }
}

impl AuthStore {
    /// Open (creating if needed) the auth database at `path`, upgrading its
    /// schema if it is from an older LibreBooks.
    pub fn open(path: &Path, config: AuthConfig) -> Result<AuthStore> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        AuthStore::from_connection(conn, path, config)
    }

    /// An auth database in memory, for tests and throwaway servers.
    pub fn in_memory(config: AuthConfig) -> Result<AuthStore> {
        let conn = Connection::open_in_memory()?;
        AuthStore::from_connection(conn, Path::new(":memory:"), config)
    }

    fn from_connection(mut conn: Connection, path: &Path, config: AuthConfig) -> Result<AuthStore> {
        let rps = config.validate()?;
        configure(&conn)?;
        migrate(&mut conn)?;
        let limiter = ratelimit::RateLimiter::new(config.rate_limit);
        let ceremonies = ceremony::Ceremonies::new(config.max_pending_ceremonies);
        Ok(AuthStore {
            inner: Arc::new(Inner {
                conn: Mutex::new(conn),
                path: path.to_owned(),
                config,
                rps,
                ceremonies,
                limiter,
            }),
        })
    }

    /// Where the database lives (`:memory:` for an in-memory one).
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// The configuration this store was opened with.
    pub fn config(&self) -> &AuthConfig {
        &self.inner.config
    }

    pub(crate) fn now(&self) -> jiff::Timestamp {
        self.inner.config.clock.now()
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves SQLite consistent (the transaction
        // rolled back when it dropped), so a poisoned lock is safe to reuse.
        self.inner
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Run `f` inside a read transaction (a consistent snapshot).
    pub(crate) fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
        let out = f(&tx)?;
        tx.finish()?;
        Ok(out)
    }

    /// Run `f` inside a write transaction; commit if it returns `Ok`.
    ///
    /// IMMEDIATE takes SQLite's write lock up front, so check-then-update
    /// sequences inside `f` (redeeming a token) are atomic even against another
    /// process with the same file open.
    pub(crate) fn write<T>(&self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Run an attempt on behalf of `client` under the failed-attempt limit:
    /// refuse up front if the client is over it, and count the attempt if it
    /// fails in a way that could be guessing.
    pub(crate) fn guarded<T>(
        &self,
        client: &ClientInfo,
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let key = client.rate_key();
        self.inner.limiter.check(&key, self.now())?;
        let out = f();
        if let Err(e) = &out
            && e.counts_as_failure()
        {
            tracing::debug!(client = %key, error = %e, "failed auth attempt");
            self.inner.limiter.fail(&key, self.now());
        }
        out
    }

    /// Delete what no longer matters: sessions and enrolment tokens that ended
    /// (expired, used or revoked) more than 30 days ago, and ceremonies in
    /// flight that timed out. The server calls this now and then; nothing
    /// depends on it for correctness, because every check also looks at the
    /// times.
    pub fn prune(&self) -> Result<()> {
        let now = self.now();
        let cutoff = clock::ms(now) - 30 * 24 * 60 * 60 * 1000;
        self.inner.ceremonies.prune(now);
        self.inner.limiter.prune(now);
        self.write(|tx| {
            tx.execute(
                "DELETE FROM session
                 WHERE MIN(expires_at, last_seen_at + idle_timeout_ms) < ?1
                    OR revoked_at < ?1",
                [cutoff],
            )?;
            tx.execute(
                "DELETE FROM enrolment WHERE expires_at < ?1 OR used_at < ?1 OR revoked_at < ?1",
                [cutoff],
            )?;
            Ok(())
        })
    }
}

fn configure(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // In-memory databases ignore WAL; that's fine.
    let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "application_id", APPLICATION_ID)?;
    Ok(())
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let known = schema::MIGRATIONS.len() as i64;
    if current > known {
        return Err(AuthError::TooNew {
            found: current,
            known,
        });
    }
    for (i, sql) in schema::MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Another process may have migrated while we waited for the lock.
        let now: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if now > i as i64 {
            continue;
        }
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_store_can_be_shared_between_threads() {
        fn shareable<T: Clone + Send + Sync>() {}
        shareable::<super::AuthStore>();
    }
}
