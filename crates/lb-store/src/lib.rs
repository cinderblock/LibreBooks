//! LibreBooks storage: one SQLite file per company.
//!
//! [`Books`] is an open company file. Reads and writes go through closures that
//! receive a connection inside a SQLite transaction, so every save is atomic:
//! a transaction, its postings, the open balances it changes and its audit
//! record commit together or not at all.
//!
//! Concurrency: one connection behind a mutex today, WAL mode. Readers never
//! block on SQLite's side; the mutex serialises Rust callers. When that becomes a
//! bottleneck, readers move to a pool (`Books::read` is already separate from
//! `Books::write` for that reason).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OpenFlags, Transaction};

pub mod accounts;
pub mod attachments;
pub mod audit;
pub mod balances;
pub mod codec;
pub mod company;
pub mod context;
pub mod entities;
pub mod extref;
pub mod hier;
pub mod items;
pub mod lists;
pub mod merge;
pub mod reconcile;
pub mod schema;
pub mod txns;

pub use balances::{AccountBalance, RegisterLine};
pub use context::StoreContext;
pub use txns::{SaveOutcome, TxnFilter, TxnSummary};

/// What a change was, for live updates and caches.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Change {
    pub kind: &'static str,
    pub id: String,
    pub action: &'static str,
}

/// Everything that can go wrong in the store. Messages are for people where the
/// person can do something about it.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Posting(#[from] lb_core::PostingError),
    #[error("{kind} not found")]
    NotFound { kind: &'static str, id: String },
    #[error("someone else changed this {kind} since you opened it; reopen it to see their change")]
    Conflict {
        kind: &'static str,
        id: String,
        expected: i64,
        actual: i64,
    },
    #[error("{0}")]
    Invalid(String),
    #[error("\"{name}\" is already used by another {kind}")]
    Duplicate { kind: &'static str, name: String },
    #[error("the books are closed through {closing_date}; this date is on or before it")]
    ClosedPeriod { closing_date: jiff::civil::Date },
    #[error("{kind} is in use and can't be deleted (make it inactive instead)")]
    InUse { kind: &'static str },
    #[error("this file is from a newer LibreBooks (schema {found}, this build knows {known})")]
    TooNew { found: i64, known: i64 },
    #[error("bad data in the file: {0}")]
    Corrupt(String),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// Who is making a change, for the audit trail and permissions.
#[derive(Clone, Debug, Default)]
pub struct Actor {
    pub user_id: Option<String>,
    /// Admins may post into a closed period (QBD's closing-date password).
    pub may_override_closing_date: bool,
}

impl Actor {
    pub fn system() -> Actor {
        Actor {
            user_id: None,
            may_override_closing_date: true,
        }
    }
}

type Listener = Box<dyn Fn(&[Change]) + Send + Sync>;

struct Inner {
    conn: Mutex<Connection>,
    path: PathBuf,
    listeners: Mutex<Vec<Listener>>,
}

/// An open company file. Cheap to clone; all clones share one connection.
#[derive(Clone)]
pub struct Books {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Books {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Books")
            .field("path", &self.inner.path)
            .finish_non_exhaustive()
    }
}

/// A write in progress: the SQLite transaction plus the changes it has made, so
/// listeners hear about them only once it commits.
#[derive(Debug)]
pub struct Writer<'c> {
    pub tx: Transaction<'c>,
    pub actor: Actor,
    changes: Vec<Change>,
}

impl Writer<'_> {
    pub fn changed(&mut self, kind: &'static str, id: impl Into<String>, action: &'static str) {
        self.changes.push(Change {
            kind,
            id: id.into(),
            action,
        });
    }
}

impl std::ops::Deref for Writer<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.tx
    }
}

impl Books {
    /// Create a new company file at `path`. Fails if the file exists.
    pub fn create(path: &Path, info: &lb_core::CompanyInfo) -> Result<Books> {
        if path.exists() {
            return Err(StoreError::Invalid(format!(
                "{} already exists",
                path.display()
            )));
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        let books = Books::from_connection(conn, path)?;
        books.write(&Actor::system(), |w| company::init(w, info))?;
        Ok(books)
    }

    /// Open an existing company file, upgrading its schema if needed.
    pub fn open(path: &Path) -> Result<Books> {
        if !path.exists() {
            return Err(StoreError::NotFound {
                kind: "company file",
                id: path.display().to_string(),
            });
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        Books::from_connection(conn, path)
    }

    /// A company in memory, for tests.
    pub fn in_memory(info: &lb_core::CompanyInfo) -> Result<Books> {
        let conn = Connection::open_in_memory()?;
        let books = Books::from_connection(conn, Path::new(":memory:"))?;
        books.write(&Actor::system(), |w| company::init(w, info))?;
        Ok(books)
    }

    fn from_connection(mut conn: Connection, path: &Path) -> Result<Books> {
        configure(&conn)?;
        migrate(&mut conn)?;
        Ok(Books {
            inner: Arc::new(Inner {
                conn: Mutex::new(conn),
                path: path.to_owned(),
                listeners: Mutex::new(Vec::new()),
            }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
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
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
        let out = f(&tx)?;
        tx.finish()?;
        Ok(out)
    }

    /// Run `f` inside a write transaction; commit if it returns `Ok`.
    pub fn write<T>(
        &self,
        actor: &Actor,
        f: impl FnOnce(&mut Writer<'_>) -> Result<T>,
    ) -> Result<T> {
        let (out, changes) = {
            let mut conn = self.lock();
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let mut w = Writer {
                tx,
                actor: actor.clone(),
                changes: Vec::new(),
            };
            let out = f(&mut w)?;
            let Writer { tx, changes, .. } = w;
            tx.commit()?;
            (out, changes)
        };
        // Listeners run after the lock is released, so they may read the books.
        self.notify(&changes);
        Ok(out)
    }

    fn notify(&self, changes: &[Change]) {
        if !changes.is_empty() {
            let listeners = self
                .inner
                .listeners
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for l in listeners.iter() {
                l(changes);
            }
        }
    }

    /// Hear about every committed change (for live updates).
    pub fn subscribe(&self, f: impl Fn(&[Change]) + Send + Sync + 'static) {
        self.inner
            .listeners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Box::new(f));
    }

    /// Copy the whole file to `dest` while it stays open (SQLite's online backup).
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        let conn = self.lock();
        let mut out = Connection::open(dest)?;
        let backup = rusqlite::backup::Backup::new(&conn, &mut out)?;
        backup.run_to_completion(256, std::time::Duration::from_millis(0), None)?;
        Ok(())
    }

    /// SQLite's integrity check plus the ledger's own: every transaction balances.
    pub fn check_integrity(&self) -> Result<Vec<String>> {
        self.read(|c| {
            let mut problems = Vec::new();
            let mut stmt = c.prepare("PRAGMA integrity_check")?;
            for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
                let row = row?;
                if row != "ok" {
                    problems.push(row);
                }
            }
            let mut stmt = c.prepare(
                "SELECT txn_id, SUM(amount) FROM posting GROUP BY txn_id HAVING SUM(amount) <> 0",
            )?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
                let (id, sum) = row?;
                problems.push(format!("transaction {id} is out of balance by {sum}"));
            }
            let mut stmt = c.prepare("PRAGMA foreign_key_check")?;
            let n = stmt.query_map([], |_| Ok(()))?.count();
            if n > 0 {
                problems.push(format!("{n} broken references"));
            }
            Ok(problems)
        })
    }
}

fn configure(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // In-memory databases ignore WAL; that's fine.
    let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    // Identify the file as a LibreBooks company (`file` and friends show this).
    conn.pragma_update(None, "application_id", APPLICATION_ID)?;
    Ok(())
}

/// "LBks" — marks a SQLite file as a LibreBooks company file.
pub const APPLICATION_ID: i32 = 0x4C42_6B73;

fn migrate(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let known = schema::MIGRATIONS.len() as i64;
    if current > known {
        return Err(StoreError::TooNew {
            found: current,
            known,
        });
    }
    for (i, sql) in schema::MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

/// Now, as stored: RFC 3339 UTC.
pub fn now() -> String {
    jiff::Timestamp::now().to_string()
}
