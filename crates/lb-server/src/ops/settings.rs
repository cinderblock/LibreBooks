//! The server's own settings, `server.json` in the data folder.
//!
//! Plain JSON, so a headless server can be set up with an editor before it first
//! starts. It never holds a secret: credentials are references (`secret:<name>`,
//! `env:NAME`, `file:/path`) resolved by [`crate::secrets::SecretStore`].

use std::path::{Path, PathBuf};

use jiff::SignedDuration;
use lb_backup::{BackupConfig, Destination, Encryption};
use lb_core::CompanyId;
use lb_tunnel::{AccessConfig, TunnelConfig};
use lb_update::UpdateConfig;
use serde::{Deserialize, Serialize};

use crate::secrets::SecretStore;

/// The settings file in the data folder.
pub const SETTINGS_FILE: &str = "server.json";

/// Everything `GET /server` shows and `PUT /server` replaces.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerSettings {
    /// Addresses people reach this server at, besides `--public-url`
    /// (`https://books.example.com`). Passkeys are bound to them, so a change
    /// takes effect at the next start.
    pub public_urls: Vec<String>,
    /// Backups of every company, each into its own folder (named by company id)
    /// in every destination.
    pub backups: BackupConfig,
    pub updates: UpdateConfig,
    pub tunnel: TunnelConfig,
    /// Verify Cloudflare Access's assertion on requests that come through
    /// Cloudflare. Off when `None`.
    pub access: Option<AccessConfig>,
    pub feeds: FeedSyncSettings,
}

impl Default for ServerSettings {
    /// What a settings file missing a section gets. A brand-new server gets
    /// [`ServerSettings::initial`] instead, which also turns on local backups.
    fn default() -> Self {
        ServerSettings {
            public_urls: Vec::new(),
            backups: BackupConfig::default(),
            updates: UpdateConfig::default(),
            tunnel: TunnelConfig::default(),
            access: None,
            feeds: FeedSyncSettings::default(),
        }
    }
}

/// Downloading bank feeds in the background.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedSyncSettings {
    /// Download online connections (SimpleFIN, Direct Connect) by themselves.
    pub auto_sync: bool,
    /// How long after a successful download the next one is due (ISO 8601:
    /// `"PT6H"`). At least an hour: banks and bridges limit how often they may be
    /// asked, and most refresh once or twice a day.
    pub every: SignedDuration,
}

impl Default for FeedSyncSettings {
    fn default() -> Self {
        FeedSyncSettings {
            auto_sync: true,
            every: SignedDuration::from_hours(6),
        }
    }
}

impl ServerSettings {
    /// A new server's settings: the defaults, plus daily backups into
    /// `<data dir>/backups` so that a server nobody configured still keeps copies
    /// (on the same disk: a second destination is what survives losing it).
    pub fn initial(data_dir: &Path) -> ServerSettings {
        ServerSettings {
            backups: BackupConfig {
                destinations: vec![Destination::Local {
                    dir: data_dir.join("backups"),
                }],
                ..BackupConfig::default()
            },
            ..ServerSettings::default()
        }
    }

    /// Backups are configured (a destination, and the schedule on).
    pub fn backups_enabled(&self) -> bool {
        !self.backups.destinations.is_empty()
    }

    /// What must be fixed before these settings can be saved, in words.
    pub fn problems(&self, secrets: &SecretStore) -> Vec<String> {
        let mut out = Vec::new();
        for url in &self.public_urls {
            if let Err(e) =
                lb_auth::RelyingParty::for_origin(url.trim_end_matches('/'), "LibreBooks")
            {
                out.push(format!("public URL {url:?}: {e}"));
            }
        }
        if self.backups_enabled() {
            out.extend(self.backups.problems());
        } else if !matches!(self.backups.encryption, Encryption::None) {
            // Harmless, but the person probably meant to add a destination.
            out.push("backups are encrypted but have no destination".into());
        }
        for d in &self.backups.destinations {
            if let Destination::S3 {
                secret_access_key_ref,
                ..
            } = d
                && let Err(e) = secrets.check_reference(&secret_access_key_ref.0)
            {
                out.push(format!("{}: secret access key: {e}", d.describe()));
            }
        }
        if let Encryption::Passphrase { passphrase_ref, .. } = &self.backups.encryption
            && let Err(e) = secrets.check_reference(&passphrase_ref.0)
        {
            out.push(format!("backup passphrase: {e}"));
        }
        out.extend(self.tunnel.problems());
        if self.tunnel.enabled
            && !self.tunnel.token_ref.trim().is_empty()
            && let Err(e) = secrets.check_reference(&self.tunnel.token_ref)
        {
            out.push(format!("tunnel token: {e}"));
        }
        if self.updates.interval < SignedDuration::from_hours(1) {
            out.push("check for updates at most once an hour".into());
        }
        if let Some(a) = &self.access {
            if a.team_domain.trim().is_empty() {
                out.push(
                    "Cloudflare Access needs the team name (myteam.cloudflareaccess.com)".into(),
                );
            }
            if a.audience.iter().all(|s| s.trim().is_empty()) {
                out.push("Cloudflare Access needs the application's audience (AUD) tag".into());
            }
        }
        if self.feeds.every < SignedDuration::from_hours(1) {
            out.push("download bank feeds at most once an hour".into());
        }
        out
    }

    /// The hostnames passkeys should work at: each public URL's and the tunnel's.
    pub fn passkey_hosts(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .public_urls
            .iter()
            .filter_map(|u| url::Url::parse(u).ok())
            .filter_map(|u| u.host_str().map(str::to_ascii_lowercase))
            .collect();
        if let Some(h) = &self.tunnel.public_hostname {
            out.push(h.trim().to_ascii_lowercase());
        }
        out.sort();
        out.dedup();
        out
    }

    /// Every public URL, the tunnel's included.
    pub fn all_public_urls(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .public_urls
            .iter()
            .map(|u| u.trim_end_matches('/').to_owned())
            .collect();
        if let Some(u) = self.tunnel.public_url() {
            out.push(u);
        }
        out.dedup();
        out
    }
}

/// `path`'s settings, or `None` when there is no file yet.
pub fn load(path: &Path) -> std::io::Result<Option<ServerSettings>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{} is not valid settings: {e}", path.display()),
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Write `settings` to `path` in one step (a reader never sees half a file).
pub fn save(path: &Path, settings: &ServerSettings) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut text = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
    text.push('\n');
    let mut tmp = tempfile::Builder::new()
        .prefix(".server-")
        .tempfile_in(dir)?;
    std::io::Write::write_all(&mut tmp, text.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// The settings in `data_dir`, writing [`ServerSettings::initial`] there first if
/// there are none.
pub fn load_or_create(data_dir: &Path) -> std::io::Result<(PathBuf, ServerSettings)> {
    let path = data_dir.join(SETTINGS_FILE);
    if let Some(s) = load(&path)? {
        return Ok((path, s));
    }
    let s = ServerSettings::initial(data_dir);
    save(&path, &s)?;
    Ok((path, s))
}

/// One company's backup configuration: the server's, with every destination
/// narrowed to that company's own folder (named by id, which never changes and
/// never collides, unlike the name).
pub fn company_backup_config(config: &BackupConfig, company: CompanyId) -> BackupConfig {
    let id = company.to_string();
    let mut out = config.clone();
    for d in &mut out.destinations {
        match d {
            Destination::Local { dir } => *dir = dir.join(&id),
            Destination::S3 { prefix, .. } => {
                let base = prefix.trim_matches('/');
                *prefix = if base.is_empty() {
                    format!("{id}/")
                } else {
                    format!("{base}/{id}/")
                };
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_server_backs_up_locally_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let (path, s) = load_or_create(dir.path()).unwrap();
        assert!(s.backups_enabled());
        assert!(s.updates.check && !s.updates.auto_install);
        assert!(!s.tunnel.enabled);
        assert_eq!(load(&path).unwrap().unwrap(), s);
        let secrets = SecretStore::open(dir.path(), None);
        assert_eq!(s.problems(&secrets), Vec::<String>::new());
        // A file with a section missing gets that section's defaults.
        std::fs::write(&path, r#"{"public_urls":["https://books.example.com"]}"#).unwrap();
        let s = load(&path).unwrap().unwrap();
        assert!(!s.backups_enabled());
        assert_eq!(s.passkey_hosts(), ["books.example.com"]);
    }

    #[test]
    fn each_company_backs_up_into_its_own_folder() {
        let c = CompanyId::new();
        let config = BackupConfig {
            destinations: vec![
                Destination::Local {
                    dir: PathBuf::from("/b"),
                },
                Destination::S3 {
                    endpoint: String::new(),
                    region: "us-east-1".into(),
                    bucket: "x".into(),
                    prefix: "/lb/".into(),
                    access_key_id: "AK".into(),
                    secret_access_key_ref: lb_backup::SecretRef::new("secret:s3"),
                    path_style: false,
                },
            ],
            ..BackupConfig::default()
        };
        let per = company_backup_config(&config, c);
        assert_eq!(
            per.destinations[0],
            Destination::Local {
                dir: PathBuf::from("/b").join(c.to_string())
            }
        );
        let Destination::S3 { prefix, .. } = &per.destinations[1] else {
            panic!()
        };
        assert_eq!(prefix, &format!("lb/{c}/"));
    }

    #[test]
    fn problems_name_missing_secrets_and_bad_urls() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = SecretStore::open(dir.path(), None);
        let mut s = ServerSettings::initial(dir.path());
        s.public_urls = vec!["books.example.com".into()];
        s.tunnel.enabled = true;
        s.tunnel.token_ref = "secret:tunnel-token".into();
        s.feeds.every = SignedDuration::from_mins(5);
        let p = s.problems(&secrets).join("\n");
        for needle in ["public URL", "tunnel token", "bank feeds"] {
            assert!(p.contains(needle), "{needle} missing from {p}");
        }
    }
}
