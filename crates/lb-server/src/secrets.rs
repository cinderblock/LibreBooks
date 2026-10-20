//! One place for the server's credentials: SimpleFIN access URLs, bank
//! passwords, S3 keys, the Cloudflare tunnel token, backup passphrases.
//!
//! A headless server has no OS keychain, so secrets live in the data folder:
//! `secrets.age` is a JSON map of name → value, encrypted with [age] to an X25519
//! key kept in `secret.key`. The key is made the first time a secret is saved, with
//! permissions for its owner alone (mode 0600 on Unix; on Windows an ACL granting
//! only the current user). It can live somewhere else (`LIBREBOOKS_SECRET_KEY_FILE`,
//! a systemd credential, a mounted secret), so that a copy of the data folder alone
//! reveals nothing. Either file can be read with the stock tool:
//! `age -d -i secret.key secrets.age`.
//!
//! Settings never hold a secret, only a **reference**:
//!
//! - `secret:<name>`: this store;
//! - `env:NAME`: an environment variable (containers, systemd `Environment=`);
//! - `file:/path`: a file's contents, trailing newline removed.
//!
//! The API can set and delete secrets and list their names, never read one back.
//! Feed connections, which bookkeepers manage, see only their own company's
//! namespace ([`CompanySecrets`]).
//!
//! [age]: https://age-encryption.org

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use age::secrecy::{ExposeSecret, SecretString};
use jiff::Timestamp;
use lb_core::CompanyId;
use serde::{Deserialize, Serialize};

/// The encrypted secrets file in the data folder.
pub const SECRETS_FILE: &str = "secrets.age";
/// The key file in the data folder, unless placed elsewhere.
pub const KEY_FILE: &str = "secret.key";
/// The scheme of a reference to this store.
pub const SCHEME: &str = "secret:";

/// Why a secret could not be stored or read. Never carries a secret value.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("{0:?} is not a usable secret name (letters, digits, '.', '_' and '-', at most 128)")]
    BadName(String),
    #[error("there is no secret named {0:?}")]
    Missing(String),
    #[error("{0}")]
    Reference(String),
    #[error("the secrets file could not be decrypted with {key}; was the key replaced?")]
    WrongKey { key: PathBuf },
    #[error("the secrets file is damaged: {0}")]
    Corrupt(String),
    #[error("the secret key {0} is unreadable; it should hold one AGE-SECRET-KEY-1… line")]
    BadKey(PathBuf),
    #[error("could not {what}: {source}")]
    Io {
        what: String,
        #[source]
        source: std::io::Error,
    },
}

fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> SecretError {
    let what = what.into();
    move |source| SecretError::Io { what, source }
}

/// What listing shows: the name and when it was last set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SecretInfo {
    pub name: String,
    pub updated_at: Timestamp,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    value: String,
    updated_at: Timestamp,
}

#[derive(Default, Serialize, Deserialize)]
struct FileFormat {
    version: u32,
    secrets: BTreeMap<String, Entry>,
}

/// The decrypted map, and the file state it was read from (so a change made by
/// another process, such as `librebooks secret set`, is picked up).
#[derive(Default)]
struct Loaded {
    stamp: Option<(SystemTime, u64)>,
    secrets: BTreeMap<String, (SecretString, Timestamp)>,
}

struct Inner {
    file: PathBuf,
    key: PathBuf,
    loaded: Mutex<Loaded>,
}

/// The server's encrypted secrets. Cheap to clone.
#[derive(Clone)]
pub struct SecretStore {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for SecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<String> = self.lock().secrets.keys().cloned().collect();
        f.debug_struct("SecretStore")
            .field("file", &self.inner.file)
            .field("names", &names)
            .finish_non_exhaustive()
    }
}

/// Is `name` usable as a secret name?
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl SecretStore {
    /// The store in `data_dir`, with its key at `key` (default
    /// `<data_dir>/secret.key`). Nothing is created until a secret is saved.
    pub fn open(data_dir: &Path, key: Option<PathBuf>) -> SecretStore {
        SecretStore {
            inner: Arc::new(Inner {
                file: data_dir.join(SECRETS_FILE),
                key: key.unwrap_or_else(|| data_dir.join(KEY_FILE)),
                loaded: Mutex::new(Loaded::default()),
            }),
        }
    }

    /// Where the key lives.
    pub fn key_path(&self) -> &Path {
        &self.inner.key
    }

    fn lock(&self) -> MutexGuard<'_, Loaded> {
        self.inner
            .loaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The secrets as they are on disk now (re-read when the file changed).
    fn fresh(&self) -> Result<MutexGuard<'_, Loaded>, SecretError> {
        let mut loaded = self.lock();
        let stamp = file_stamp(&self.inner.file);
        if stamp != loaded.stamp {
            *loaded = self.read(stamp)?;
        }
        Ok(loaded)
    }

    fn read(&self, stamp: Option<(SystemTime, u64)>) -> Result<Loaded, SecretError> {
        let Some(_) = stamp else {
            return Ok(Loaded::default());
        };
        let bytes = std::fs::read(&self.inner.file)
            .map_err(io(format!("read {}", self.inner.file.display())))?;
        let identity = self.identity(false)?.ok_or_else(|| SecretError::WrongKey {
            key: self.inner.key.clone(),
        })?;
        let plain = age::decrypt(&identity, &bytes).map_err(|_| SecretError::WrongKey {
            key: self.inner.key.clone(),
        })?;
        let parsed: FileFormat =
            serde_json::from_slice(&plain).map_err(|e| SecretError::Corrupt(e.to_string()))?;
        Ok(Loaded {
            stamp,
            secrets: parsed
                .secrets
                .into_iter()
                .map(|(k, e)| (k, (SecretString::from(e.value), e.updated_at)))
                .collect(),
        })
    }

    /// The key, made (owner-only) when `create` and there is none yet.
    fn identity(&self, create: bool) -> Result<Option<age::x25519::Identity>, SecretError> {
        let path = &self.inner.key;
        match std::fs::read_to_string(path) {
            Ok(text) => {
                check_permissions(path);
                let line = text
                    .lines()
                    .map(str::trim)
                    .find(|l| l.starts_with("AGE-SECRET-KEY-"))
                    .ok_or_else(|| SecretError::BadKey(path.clone()))?;
                line.parse()
                    .map(Some)
                    .map_err(|_| SecretError::BadKey(path.clone()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
                let identity = age::x25519::Identity::generate();
                write_key(path, &identity)?;
                Ok(Some(identity))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(format!("read {}", path.display()))(e)),
        }
    }

    fn write(&self, loaded: &mut Loaded) -> Result<(), SecretError> {
        let identity = self
            .identity(true)?
            .ok_or_else(|| SecretError::BadKey(self.inner.key.clone()))?;
        let doc = FileFormat {
            version: 1,
            secrets: loaded
                .secrets
                .iter()
                .map(|(k, (v, at))| {
                    (
                        k.clone(),
                        Entry {
                            value: v.expose_secret().to_owned(),
                            updated_at: *at,
                        },
                    )
                })
                .collect(),
        };
        let plain = serde_json::to_vec(&doc).map_err(|e| SecretError::Corrupt(e.to_string()))?;
        let sealed = age::encrypt(&identity.to_public(), &plain)
            .map_err(|e| SecretError::Corrupt(e.to_string()))?;
        write_private(&self.inner.file, &sealed)?;
        loaded.stamp = file_stamp(&self.inner.file);
        Ok(())
    }

    /// Save (or replace) a secret.
    pub fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        if !valid_name(name) {
            return Err(SecretError::BadName(name.to_owned()));
        }
        let mut loaded = self.fresh()?;
        loaded.secrets.insert(
            name.to_owned(),
            (SecretString::from(value.to_owned()), Timestamp::now()),
        );
        self.write(&mut loaded)
    }

    /// Remove a secret; `false` if there was none.
    pub fn delete(&self, name: &str) -> Result<bool, SecretError> {
        let mut loaded = self.fresh()?;
        if loaded.secrets.remove(name).is_none() {
            return Ok(false);
        }
        self.write(&mut loaded)?;
        Ok(true)
    }

    /// Names and update times, sorted by name. Never values.
    pub fn list(&self) -> Result<Vec<SecretInfo>, SecretError> {
        Ok(self
            .fresh()?
            .secrets
            .iter()
            .map(|(name, (_, at))| SecretInfo {
                name: name.clone(),
                updated_at: *at,
            })
            .collect())
    }

    /// Is there a secret with this name?
    pub fn contains(&self, name: &str) -> Result<bool, SecretError> {
        Ok(self.fresh()?.secrets.contains_key(name))
    }

    /// The secret `name`, for the code that sends it.
    pub fn get(&self, name: &str) -> Result<Option<SecretString>, SecretError> {
        Ok(self.fresh()?.secrets.get(name).map(|(v, _)| v.clone()))
    }

    /// Resolve a reference: `secret:<name>`, `env:NAME` or `file:/path`.
    pub fn resolve(&self, reference: &str) -> Result<SecretString, SecretError> {
        match reference.split_once(':') {
            Some(("secret", name)) => self
                .get(name)?
                .ok_or_else(|| SecretError::Missing(name.to_owned())),
            Some(("env", name)) => std::env::var(name).map(SecretString::from).map_err(|_| {
                SecretError::Reference(format!("the environment variable {name} is not set"))
            }),
            Some(("file", path)) => std::fs::read_to_string(path)
                .map(|s| SecretString::from(s.trim_end_matches(['\r', '\n']).to_owned()))
                .map_err(|e| SecretError::Reference(format!("could not read {path}: {e}"))),
            _ => Err(SecretError::Reference(format!(
                "{reference:?} is not a secret reference (secret:<name>, env:NAME or file:/path)"
            ))),
        }
    }

    /// Is `reference` well formed, and (for `secret:`) does it exist? For
    /// validating settings before they are saved.
    pub fn check_reference(&self, reference: &str) -> Result<(), SecretError> {
        match reference.split_once(':') {
            Some(("secret", name)) if valid_name(name) => {
                if self.contains(name)? {
                    Ok(())
                } else {
                    Err(SecretError::Missing(name.to_owned()))
                }
            }
            Some(("env" | "file", rest)) if !rest.is_empty() => Ok(()),
            _ => Err(SecretError::Reference(format!(
                "{reference:?} is not a secret reference (secret:<name>, env:NAME or file:/path)"
            ))),
        }
    }

    /// The view a company's feed connections get.
    pub fn for_company(&self, company: CompanyId) -> CompanySecrets {
        CompanySecrets {
            store: self.clone(),
            company,
        }
    }
}

fn file_stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// Write `bytes` to `path` atomically, readable by the owner only.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), SecretError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(io(format!("create {}", dir.display())))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".secrets-")
        .tempfile_in(dir)
        .map_err(io(format!("write in {}", dir.display())))?;
    // tempfile creates files 0600 on Unix already.
    tmp.write_all(bytes)
        .and_then(|()| tmp.as_file().sync_all())
        .map_err(io(format!("write {}", path.display())))?;
    restrict(tmp.path());
    tmp.persist(path)
        .map_err(|e| io(format!("replace {}", path.display()))(e.error))?;
    Ok(())
}

fn write_key(path: &Path, identity: &age::x25519::Identity) -> Result<(), SecretError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io(format!("create {}", dir.display())))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .map_err(io(format!("create {}", path.display())))?;
    restrict(path);
    let text = format!(
        "# LibreBooks secret key: decrypts {SECRETS_FILE}. Keep it private; losing it loses the secrets.\n# public key: {}\n{}\n",
        identity.to_public(),
        identity.to_string().expose_secret()
    );
    f.write_all(text.as_bytes())
        .and_then(|()| f.sync_all())
        .map_err(io(format!("write {}", path.display())))?;
    tracing::info!(path = %path.display(), "created the secret key");
    Ok(())
}

/// Make `path` readable by its owner alone. Best effort: a failure is logged.
#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!(path = %path.display(), error = %e, "could not make the file private");
    }
}

/// Make `path` readable by the current user alone: no inherited entries, full
/// control for this user's SID. Uses the tools every Windows ships (no `unsafe`).
#[cfg(windows)]
fn restrict(path: &Path) {
    let system32 = std::env::var_os("SystemRoot")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("System32");
    let run = || -> Result<(), String> {
        let out = std::process::Command::new(system32.join("whoami.exe"))
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout);
        // "machine\user","S-1-5-21-…"
        let sid = text
            .trim()
            .rsplit(',')
            .next()
            .map(|s| s.trim().trim_matches('"').to_owned())
            .filter(|s| s.starts_with("S-1-"))
            .ok_or_else(|| format!("whoami gave no SID: {}", text.trim()))?;
        let out = std::process::Command::new(system32.join("icacls.exe"))
            .arg(path)
            .args(["/inheritance:r", "/grant:r"])
            .arg(format!("*{sid}:F"))
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stdout).trim().to_owned())
        }
    };
    if let Err(e) = run() {
        tracing::warn!(path = %path.display(), error = %e, "could not make the file private to this user");
    }
}

#[cfg(not(any(unix, windows)))]
fn restrict(_: &Path) {}

/// Warn when the key is readable by others (Unix), and fix it.
#[cfg(unix)]
fn check_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(m) = std::fs::metadata(path)
        && m.permissions().mode() & 0o077 != 0
    {
        tracing::warn!(path = %path.display(), "the secret key was readable by others; making it private");
        restrict(path);
    }
}

#[cfg(not(unix))]
fn check_permissions(_: &Path) {}

impl lb_backup::SecretResolver for SecretStore {
    fn resolve(&self, reference: &lb_backup::SecretRef) -> lb_backup::Result<SecretString> {
        SecretStore::resolve(self, &reference.0).map_err(|e| lb_backup::BackupError::Secret {
            reference: reference.to_string(),
            reason: e.to_string(),
        })
    }
}

impl lb_tunnel::TokenSource for SecretStore {
    fn resolve(&self, reference: &str) -> Result<String, lb_tunnel::TunnelError> {
        SecretStore::resolve(self, reference)
            .map(|s| s.expose_secret().trim().to_owned())
            .map_err(|e| lb_tunnel::TunnelError::Token(e.to_string()))
    }
}

/// One company's feed credentials: only `secret:feeds.<company-id>.*` resolve.
#[derive(Clone, Debug)]
pub struct CompanySecrets {
    store: SecretStore,
    company: CompanyId,
}

impl CompanySecrets {
    /// The prefix every reference must start with: `secret:feeds.<id>.`.
    pub fn prefix(&self) -> String {
        format!("{SCHEME}{}", self.name_prefix())
    }

    /// The secret-name prefix: `feeds.<id>.`.
    pub fn name_prefix(&self) -> String {
        format!("feeds.{}.", self.company)
    }

    /// Is `reference` one of this company's?
    pub fn owns(&self, reference: &str) -> bool {
        reference
            .strip_prefix(&self.prefix())
            .is_some_and(valid_name)
    }

    /// Store a credential under `feeds.<company>.<suffix>`; returns its reference.
    pub fn set(&self, suffix: &str, value: &str) -> Result<lb_feeds::SecretRef, SecretError> {
        let name = format!("{}{suffix}", self.name_prefix());
        self.store.set(&name, value)?;
        Ok(lb_feeds::SecretRef::new(format!("{SCHEME}{name}")))
    }

    /// Delete a credential this company owns (others are left alone).
    pub fn delete(&self, reference: &str) -> Result<bool, SecretError> {
        match reference.strip_prefix(SCHEME) {
            Some(name) if self.owns(reference) => self.store.delete(name),
            _ => Ok(false),
        }
    }
}

impl lb_feeds::Secrets for CompanySecrets {
    fn get(&self, r: &lb_feeds::SecretRef) -> Option<lb_feeds::Secret> {
        if !self.owns(&r.0) {
            return None;
        }
        let name = r.0.strip_prefix(SCHEME)?;
        match self.store.get(name) {
            Ok(v) => v.map(|s| lb_feeds::Secret::new(s.expose_secret())),
            Err(e) => {
                tracing::warn!(error = %e, "could not read the secrets file");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_round_trip_encrypted_and_never_print() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::open(dir.path(), None);
        assert_eq!(store.list().unwrap(), Vec::new());
        assert!(!dir.path().join(KEY_FILE).exists(), "nothing made yet");

        store.set("s3-key", "hunter2-very-secret").unwrap();
        assert!(dir.path().join(KEY_FILE).exists());
        let sealed = std::fs::read(dir.path().join(SECRETS_FILE)).unwrap();
        assert!(
            !String::from_utf8_lossy(&sealed).contains("hunter2"),
            "the file is encrypted"
        );
        assert_eq!(
            store.resolve("secret:s3-key").unwrap().expose_secret(),
            "hunter2-very-secret"
        );
        assert!(!format!("{store:?}").contains("hunter2"));
        assert_eq!(store.list().unwrap()[0].name, "s3-key");

        // Another process (a second store on the same files) sees it, and its
        // change is seen here.
        let other = SecretStore::open(dir.path(), None);
        assert!(other.contains("s3-key").unwrap());
        other.set("tunnel-token", "abc").unwrap();
        assert!(store.contains("tunnel-token").unwrap());

        assert!(store.delete("s3-key").unwrap());
        assert!(!store.delete("s3-key").unwrap());
        assert!(matches!(
            store.resolve("secret:s3-key"),
            Err(SecretError::Missing(_))
        ));
        assert!(matches!(
            store.set("bad name", "x"),
            Err(SecretError::BadName(_))
        ));
    }

    #[test]
    fn a_replaced_key_is_reported_not_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::open(dir.path(), None);
        store.set("a", "1").unwrap();
        std::fs::remove_file(dir.path().join(KEY_FILE)).unwrap();
        let fresh = SecretStore::open(dir.path(), None);
        assert!(matches!(fresh.list(), Err(SecretError::WrongKey { .. })));
    }

    #[test]
    fn env_and_file_references_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::open(dir.path(), None);
        let f = dir.path().join("pp");
        std::fs::write(&f, "passphrase\r\n").unwrap();
        assert_eq!(
            store
                .resolve(&format!("file:{}", f.display()))
                .unwrap()
                .expose_secret(),
            "passphrase"
        );
        assert!(store.resolve("env:PATH").is_ok());
        assert!(store.resolve("vault:x").is_err());
        assert!(store.check_reference("secret:nope").is_err());
        assert!(store.check_reference("env:X").is_ok());
    }

    #[test]
    fn a_company_sees_only_its_own_feed_secrets() {
        use lb_feeds::Secrets as _;
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::open(dir.path(), None);
        store.set("tunnel-token", "server-only").unwrap();
        let a = store.for_company(CompanyId::new());
        let b = store.for_company(CompanyId::new());
        let r = a.set("conn-1", "bank-password").unwrap();
        assert_eq!(a.get(&r).unwrap().expose(), "bank-password");
        assert!(b.get(&r).is_none(), "another company's");
        for bad in ["secret:tunnel-token", "env:PATH", "file:/etc/passwd"] {
            assert!(a.get(&lb_feeds::SecretRef::new(bad)).is_none(), "{bad}");
        }
        assert!(!b.delete(&r.0).unwrap());
        assert!(a.delete(&r.0).unwrap());
    }
}
