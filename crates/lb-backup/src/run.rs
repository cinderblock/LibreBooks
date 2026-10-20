//! Making, listing, verifying and restoring backups.

use std::path::{Path, PathBuf};

use age::secrecy::SecretString;
use jiff::{Timestamp, Zoned};
use lb_store::Books;
use serde::{Deserialize, Serialize};

use crate::config::{BackupConfig, Destination, Encryption, Retention, RetentionScope};
use crate::dest::Store;
use crate::dest::local::blocking;
use crate::manifest::{self, EncryptionKind, MANIFEST_SUFFIX, Manifest};
use crate::pipeline::{self, Decrypt, Encrypt};
use crate::retention::{Dated, plan_retention};
use crate::secret::SecretResolver;
use crate::{APP_VERSION, BackupError, Result};

/// What one backup run did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRecord {
    pub manifest: Manifest,
    /// One per configured destination, in order.
    pub destinations: Vec<DestinationOutcome>,
}

impl BackupRecord {
    /// Stored everywhere it should have been.
    pub fn complete(&self) -> bool {
        self.destinations.iter().all(|d| d.error.is_none())
    }
}

/// What happened at one destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationOutcome {
    /// [`Destination::describe`].
    pub destination: String,
    pub stored: bool,
    /// Why it was not stored, or why pruning failed after it was.
    pub error: Option<String>,
    /// Ids of older backups deleted by retention.
    pub pruned: Vec<String>,
}

/// A backup found in a destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupEntry {
    pub destination: Destination,
    pub manifest: Manifest,
}

/// What it takes to open an encrypted backup.
#[derive(Default)]
pub enum Unlock {
    /// Nothing: unencrypted backups only.
    #[default]
    None,
    /// age identities, `AGE-SECRET-KEY-1…`, for backups encrypted to keys.
    Identities(Vec<String>),
    /// The passphrase, for passphrase backups.
    Passphrase(SecretString),
}

impl std::fmt::Debug for Unlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Unlock::None => "Unlock::None",
            Unlock::Identities(_) => "Unlock::Identities(..)",
            Unlock::Passphrase(_) => "Unlock::Passphrase(..)",
        })
    }
}

impl Unlock {
    /// What the configuration itself can unlock: the passphrase of passphrase
    /// encryption. Backups encrypted to keys need the identity from a person.
    pub fn from_config(encryption: &Encryption, secrets: &dyn SecretResolver) -> Result<Unlock> {
        match encryption {
            Encryption::Passphrase { passphrase_ref, .. } => {
                Ok(Unlock::Passphrase(secrets.resolve(passphrase_ref)?))
            }
            Encryption::None | Encryption::Recipients { .. } => Ok(Unlock::None),
        }
    }
}

/// The result of [`verify`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    /// The stored bytes match the manifest's checksum.
    pub checksum_ok: bool,
    /// The contents were decrypted, decompressed, hashed and integrity-checked.
    /// `false` when that was not possible (encrypted, and no key given).
    pub contents_checked: bool,
    /// What is wrong, in words. Empty when all is well.
    pub problems: Vec<String>,
}

impl VerifyReport {
    /// Nothing wrong was found.
    pub fn ok(&self) -> bool {
        self.checksum_ok && self.problems.is_empty()
    }
}

/// Back up `books` now: snapshot, check, pack, store everywhere, prune.
///
/// Succeeds when the backup reached at least one destination; the record says how
/// each destination fared. Retention runs per destination, only where the new
/// backup was stored.
///
/// # Errors
/// [`BackupError::NoDestinations`], [`BackupError::Integrity`] (the copy failed
/// its checks and was not stored), [`BackupError::AllDestinationsFailed`], or a
/// configuration, secret or I/O problem.
pub async fn backup_now(
    books: &Books,
    company_name: &str,
    config: &BackupConfig,
    secrets: &dyn SecretResolver,
) -> Result<BackupRecord> {
    if config.destinations.is_empty() {
        return Err(BackupError::NoDestinations);
    }
    let encrypt = resolve_encryption(&config.encryption, secrets)?;
    let tz = config.tz()?;
    let company_slug = manifest::slug(company_name);
    let created_at = Timestamp::now();
    let id = manifest::backup_id(&company_slug, created_at);

    let work_parent = work_dir(config, books);
    let level = config.compression_level;
    let kind = encrypt.kind();
    let books = books.clone();
    let (work, packed, schema_version) = blocking(move || {
        std::fs::create_dir_all(&work_parent)
            .map_err(BackupError::io(format!("create {}", work_parent.display())))?;
        let work = tempfile::Builder::new()
            .prefix(".lb-backup-")
            .tempdir_in(&work_parent)
            .map_err(BackupError::io(format!(
                "create a working folder in {}",
                work_parent.display()
            )))?;
        let db = work.path().join("company.lbdb");
        let schema = pipeline::snapshot(&books, &db)?;
        let packed = pipeline::pack(&db, &work.path().join("stored"), level, &encrypt)?;
        std::fs::remove_file(&db).map_err(BackupError::io("remove the working copy"))?;
        Ok((work, packed, schema))
    })
    .await?;
    let stored_path = work.path().join("stored");

    let manifest = Manifest {
        format: manifest::FORMAT_VERSION,
        app_version: APP_VERSION.to_owned(),
        company_name: company_name.to_owned(),
        company_slug: company_slug.clone(),
        schema_version,
        created_at,
        artifact: manifest::artifact_name(&company_slug, &id, kind != EncryptionKind::None),
        id,
        database_size: packed.database_size,
        database_sha256: packed.database_sha256,
        stored_size: packed.stored_size,
        stored_sha256: packed.stored_sha256,
        compression: "zstd".to_owned(),
        encryption: kind,
    };

    let mut outcomes = Vec::new();
    for destination in &config.destinations {
        let mut outcome = DestinationOutcome {
            destination: destination.describe(),
            stored: false,
            error: None,
            pruned: Vec::new(),
        };
        match store_one(destination, secrets, &manifest, &stored_path).await {
            Err(e) => {
                tracing::warn!(destination = %outcome.destination, error = %e, "backup not stored");
                outcome.error = Some(e.to_string());
            }
            Ok(store) => {
                outcome.stored = true;
                let now = Zoned::new(Timestamp::now(), tz.clone());
                let scope = match config.retention_scope {
                    RetentionScope::CompanySlug => Some(company_slug.as_str()),
                    RetentionScope::Destination => None,
                };
                match prune(&store, scope, &config.retention, &now).await {
                    Ok(pruned) => outcome.pruned = pruned,
                    Err(e) => {
                        tracing::warn!(destination = %outcome.destination, error = %e, "pruning failed");
                        outcome.error =
                            Some(format!("stored, but pruning old backups failed: {e}"));
                    }
                }
            }
        }
        outcomes.push(outcome);
    }
    drop(work);

    if outcomes.iter().all(|o| !o.stored) {
        return Err(BackupError::AllDestinationsFailed(
            outcomes
                .into_iter()
                .map(|o| format!("{}: {}", o.destination, o.error.unwrap_or_default()))
                .collect(),
        ));
    }
    tracing::info!(id = %manifest.id, size = manifest.stored_size, "backup made");
    Ok(BackupRecord {
        manifest,
        destinations: outcomes,
    })
}

/// The artifact, then the manifest: once the manifest exists, the backup is whole.
async fn store_one(
    destination: &Destination,
    secrets: &dyn SecretResolver,
    manifest: &Manifest,
    stored: &Path,
) -> Result<Store> {
    let store = Store::open(destination, secrets)?;
    store
        .put_file(
            &manifest.artifact,
            stored,
            &manifest.stored_sha256,
            manifest.stored_size,
        )
        .await?;
    store
        .put_bytes(&manifest.object_name(), manifest.to_json())
        .await?;
    Ok(store)
}

/// Apply retention to one company's backups in one destination. Returns the ids
/// deleted.
/// Apply `retention` to the backups under `company_slug` (or, with `None`, to
/// every backup in the store: a destination that holds one company).
async fn prune(
    store: &Store,
    company_slug: Option<&str>,
    retention: &Retention,
    now: &Zoned,
) -> Result<Vec<String>> {
    let manifests = read_manifests(store, company_slug).await?;
    let dated: Vec<Dated> = manifests
        .iter()
        .map(|m| Dated::new(m.id.clone(), m.created_at))
        .collect();
    let doomed = plan_retention(&dated, retention, now);
    for id in &doomed {
        let Some(m) = manifests.iter().find(|m| &m.id == id) else {
            continue;
        };
        // Manifest first: an interrupted prune leaves an unlisted artifact, never a
        // listed backup with nothing behind it.
        store.delete(&m.object_name()).await?;
        store.delete(&m.artifact).await?;
        tracing::info!(id = %m.id, "old backup pruned");
    }
    Ok(doomed)
}

async fn read_manifests(store: &Store, company_slug: Option<&str>) -> Result<Vec<Manifest>> {
    let prefix = company_slug.map(|s| format!("{s}/")).unwrap_or_default();
    let mut out = Vec::new();
    for name in store.list(&prefix).await? {
        if !name.ends_with(MANIFEST_SUFFIX) {
            continue;
        }
        let parsed = store
            .get_bytes(&name)
            .await
            .and_then(|b| Manifest::from_json(&b));
        match parsed {
            // Only manifests where they belong: a copied-in file elsewhere is not
            // trusted to name objects for deletion.
            Ok(m) if m.object_name() == name => out.push(m),
            Ok(_) => tracing::warn!(%name, "manifest does not match its name; ignored"),
            Err(e) => tracing::warn!(%name, error = %e, "unreadable manifest ignored"),
        }
    }
    Ok(out)
}

/// The backups in `destination`, newest first; only `company_slug`'s when given.
/// Unreadable manifests are skipped (and logged).
pub async fn list_backups(
    destination: &Destination,
    secrets: &dyn SecretResolver,
    company_slug: Option<&str>,
) -> Result<Vec<BackupEntry>> {
    let store = Store::open(destination, secrets)?;
    let mut manifests = read_manifests(&store, company_slug).await?;
    manifests.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    Ok(manifests
        .into_iter()
        .map(|manifest| BackupEntry {
            destination: destination.clone(),
            manifest,
        })
        .collect())
}

/// Check a backup without restoring it: the stored bytes against the manifest,
/// then (when it can be unlocked) the database inside: its hash, and the store's
/// integrity checks.
pub async fn verify(
    entry: &BackupEntry,
    secrets: &dyn SecretResolver,
    unlock: &Unlock,
) -> Result<VerifyReport> {
    let store = Store::open(&entry.destination, secrets)?;
    let work = tempfile::Builder::new()
        .prefix("lb-verify-")
        .tempdir()
        .map_err(BackupError::io("create a temporary folder"))?;
    let stored = work.path().join("stored");
    store.get_file(&entry.manifest.artifact, &stored).await?;
    let manifest = entry.manifest.clone();
    let can_unlock = can_unlock(manifest.encryption, unlock);
    let identities = parse_identities(unlock)?;
    let passphrase = match unlock {
        Unlock::Passphrase(p) => Some(p.clone()),
        _ => None,
    };
    let report = blocking(move || {
        let mut report = VerifyReport {
            checksum_ok: false,
            contents_checked: false,
            problems: Vec::new(),
        };
        let (size, sha) = pipeline::hash_file(&stored)?;
        if size != manifest.stored_size || sha != manifest.stored_sha256 {
            report.problems.push(
                "the stored file does not match its checksum; it was changed or damaged".into(),
            );
            return Ok(report);
        }
        report.checksum_ok = true;
        if !can_unlock {
            return Ok(report);
        }
        let decrypt = decrypt_for(&identities, passphrase.as_ref());
        let db = work.path().join("company.lbdb");
        match pipeline::unpack(&stored, &db, manifest.encryption, &decrypt) {
            Err(e) => report.problems.push(e.to_string()),
            Ok((size, sha)) => {
                if size != manifest.database_size || sha != manifest.database_sha256 {
                    report
                        .problems
                        .push("the database inside does not match its checksum".into());
                } else {
                    match pipeline::check_database(&db) {
                        Ok(_) => {}
                        Err(BackupError::Integrity(p)) => report.problems.extend(p),
                        Err(e) => report.problems.push(e.to_string()),
                    }
                }
                report.contents_checked = true;
            }
        }
        drop(work);
        Ok(report)
    })
    .await?;
    Ok(report)
}

/// Restore a backup to `dest_path`, which must not exist. Every check [`verify`]
/// makes must pass first; then the file appears at `dest_path` in one step.
/// Nothing open is touched: the caller closes the old company and swaps.
///
/// # Errors
/// [`BackupError::Exists`] when `dest_path` exists, [`BackupError::Checksum`] or
/// [`BackupError::Integrity`] when the backup is damaged, [`BackupError::Decrypt`]
/// when it cannot be unlocked.
pub async fn restore(
    entry: &BackupEntry,
    dest_path: &Path,
    secrets: &dyn SecretResolver,
    unlock: &Unlock,
) -> Result<Manifest> {
    if dest_path.exists() {
        return Err(BackupError::Exists(dest_path.to_owned()));
    }
    let parent = parent_dir(dest_path);
    std::fs::create_dir_all(&parent)
        .map_err(BackupError::io(format!("create {}", parent.display())))?;
    let store = Store::open(&entry.destination, secrets)?;
    let work = tempfile::Builder::new()
        .prefix(".lb-restore-")
        .tempdir_in(&parent)
        .map_err(BackupError::io(format!(
            "create a working folder in {}",
            parent.display()
        )))?;
    let stored = work.path().join("stored");
    store.get_file(&entry.manifest.artifact, &stored).await?;

    let manifest = entry.manifest.clone();
    let identities = parse_identities(unlock)?;
    let passphrase = match unlock {
        Unlock::Passphrase(p) => Some(p.clone()),
        _ => None,
    };
    let dest_path = dest_path.to_owned();
    blocking(move || {
        let (size, sha) = pipeline::hash_file(&stored)?;
        if size != manifest.stored_size || sha != manifest.stored_sha256 {
            return Err(BackupError::Checksum {
                what: "the stored backup",
            });
        }
        let db = tempfile::Builder::new()
            .prefix(".lb-restore-")
            .suffix(".lbdb")
            .tempfile_in(&parent)
            .map_err(BackupError::io(format!(
                "create a file in {}",
                parent.display()
            )))?
            .into_temp_path();
        let decrypt = decrypt_for(&identities, passphrase.as_ref());
        let (size, sha) = pipeline::unpack(&stored, &db, manifest.encryption, &decrypt)?;
        if size != manifest.database_size || sha != manifest.database_sha256 {
            return Err(BackupError::Checksum {
                what: "the restored database",
            });
        }
        pipeline::check_database(&db)?;
        db.persist_noclobber(&dest_path).map_err(|e| {
            if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                BackupError::Exists(dest_path.clone())
            } else {
                BackupError::Io {
                    what: format!("move the restored file to {}", dest_path.display()),
                    source: e.error,
                }
            }
        })?;
        drop(work);
        Ok(manifest)
    })
    .await
}

fn resolve_encryption(encryption: &Encryption, secrets: &dyn SecretResolver) -> Result<Encrypt> {
    Ok(match encryption {
        Encryption::None => Encrypt::None,
        Encryption::Recipients { recipients } => {
            if recipients.is_empty() {
                return Err(BackupError::Config(
                    "encryption to keys needs at least one public key".into(),
                ));
            }
            let parsed = recipients
                .iter()
                .map(|r| {
                    r.trim().parse::<age::x25519::Recipient>().map_err(|_| {
                        BackupError::Config(format!("{r:?} is not an age public key (age1…)"))
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Encrypt::Recipients(parsed)
        }
        Encryption::Passphrase {
            passphrase_ref,
            work_factor,
        } => Encrypt::Passphrase {
            passphrase: secrets.resolve(passphrase_ref)?,
            work_factor: *work_factor,
        },
    })
}

fn can_unlock(kind: EncryptionKind, unlock: &Unlock) -> bool {
    match kind {
        EncryptionKind::None => true,
        EncryptionKind::X25519 => matches!(unlock, Unlock::Identities(ids) if !ids.is_empty()),
        EncryptionKind::Passphrase => matches!(unlock, Unlock::Passphrase(_)),
    }
}

fn parse_identities(unlock: &Unlock) -> Result<Vec<age::x25519::Identity>> {
    match unlock {
        Unlock::Identities(ids) => ids
            .iter()
            .map(|s| {
                s.trim().parse::<age::x25519::Identity>().map_err(|_| {
                    BackupError::Config("that is not an age identity (AGE-SECRET-KEY-1…)".into())
                })
            })
            .collect(),
        _ => Ok(Vec::new()),
    }
}

fn decrypt_for<'a>(
    identities: &'a [age::x25519::Identity],
    passphrase: Option<&'a SecretString>,
) -> Decrypt<'a> {
    match passphrase {
        Some(p) => Decrypt::Passphrase(p),
        None if !identities.is_empty() => Decrypt::Identities(identities),
        None => Decrypt::None,
    }
}

/// Where the working copy goes: configured, else beside the company file, else the
/// system's temporary folder (an in-memory company).
fn work_dir(config: &BackupConfig, books: &Books) -> PathBuf {
    if let Some(dir) = &config.work_dir {
        return dir.clone();
    }
    match books.path().parent() {
        Some(p) if !p.as_os_str().is_empty() && books.path() != Path::new(":memory:") => {
            p.to_owned()
        }
        _ => std::env::temp_dir(),
    }
}

fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_owned(),
        _ => PathBuf::from("."),
    }
}
