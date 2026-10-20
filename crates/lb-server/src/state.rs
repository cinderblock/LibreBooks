//! Server state: configuration, the companies it serves, and the live-update bus.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use lb_core::{CompanyId, CompanyInfo};
use lb_store::Books;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::auth::AuthMode;
use crate::error::{ApiError, ApiResult};

/// File extension of a company file.
pub const COMPANY_EXT: &str = "lbdb";

/// How the server runs; reported by `/health` and used for defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Headless `librebooks serve`.
    Server,
    /// Inside the desktop app.
    Desktop,
}

/// A committed change in one company, for `/events`.
#[derive(Clone, Debug, Serialize)]
pub struct ChangeEvent {
    pub company: CompanyId,
    pub kind: &'static str,
    pub id: String,
    pub action: &'static str,
}

/// Company files in a directory, opened on first use and kept open.
#[derive(Debug)]
pub struct Companies {
    dir: PathBuf,
    open: RwLock<HashMap<CompanyId, Books>>,
    events: broadcast::Sender<ChangeEvent>,
}

/// What `/companies` lists.
#[derive(Clone, Debug, Serialize)]
pub struct CompanySummary {
    pub id: CompanyId,
    pub name: String,
}

impl Companies {
    pub fn new(dir: PathBuf, events: broadcast::Sender<ChangeEvent>) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Companies {
            dir,
            open: RwLock::new(HashMap::new()),
            events,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_of(&self, id: CompanyId) -> PathBuf {
        self.dir.join(format!("{id}.{COMPANY_EXT}"))
    }

    /// Every company file in the directory, named `<uuid>.lbdb`.
    pub fn ids(&self) -> std::io::Result<Vec<CompanyId>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some(COMPANY_EXT) {
                continue;
            }
            if let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse().ok())
            {
                out.push(id);
            }
        }
        out.sort();
        Ok(out)
    }

    /// The open company, opening it (and wiring its change events) if needed.
    pub fn get(&self, id: CompanyId) -> ApiResult<Books> {
        if let Some(b) = self
            .open
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
        {
            return Ok(b.clone());
        }
        let path = self.path_of(id);
        if !path.exists() {
            return Err(ApiError::not_found("company"));
        }
        let mut open = self
            .open
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(b) = open.get(&id) {
            return Ok(b.clone());
        }
        let books = Books::open(&path)?;
        self.wire(id, &books);
        open.insert(id, books.clone());
        Ok(books)
    }

    fn wire(&self, id: CompanyId, books: &Books) {
        let tx = self.events.clone();
        books.subscribe(move |changes| {
            for c in changes {
                // No subscribers is fine.
                let _ = tx.send(ChangeEvent {
                    company: id,
                    kind: c.kind,
                    id: c.id.clone(),
                    action: c.action,
                });
            }
        });
    }

    /// Create a new company file.
    pub fn create(&self, info: &CompanyInfo) -> ApiResult<CompanyId> {
        if info.name.trim().is_empty() {
            return Err(ApiError::invalid("a company needs a name"));
        }
        let id = CompanyId::new();
        let books = Books::create(&self.path_of(id), info)?;
        self.wire(id, &books);
        self.open
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, books);
        Ok(id)
    }

    /// Bring an existing company file (e.g. restored or imported) under this
    /// server, copying it in with a fresh id.
    pub fn adopt(&self, file: &Path) -> ApiResult<CompanyId> {
        let probe = Books::open(file)?;
        let id = CompanyId::new();
        probe.backup_to(&self.path_of(id)).map_err(ApiError::from)?;
        Ok(id)
    }

    pub fn list(&self) -> ApiResult<Vec<CompanySummary>> {
        let mut out = Vec::new();
        for id in self.ids().map_err(|e| ApiError::internal(e.to_string()))? {
            match self
                .get(id)
                .and_then(|b| b.read(lb_store::company::info).map_err(ApiError::from))
            {
                Ok(info) => out.push(CompanySummary {
                    id,
                    name: info.name,
                }),
                Err(e) => {
                    tracing::warn!(%id, error = %e.message, "skipping unreadable company file");
                }
            }
        }
        out.sort_by_key(|a| a.name.to_lowercase());
        Ok(out)
    }
}

/// Everything a request handler can reach.
#[derive(Clone, Debug)]
pub struct AppState {
    pub inner: Arc<Inner>,
}

#[derive(Debug)]
pub struct Inner {
    pub mode: Mode,
    pub version: &'static str,
    pub companies: Companies,
    pub events: broadcast::Sender<ChangeEvent>,
    pub auth: AuthMode,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.inner
    }
}
