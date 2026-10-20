//! The desktop app's own settings: which backend to run, serving to the network,
//! updates. Kept as JSON in the app's config directory.
//!
//! Company data never lives here; `local.data_dir` only points at it.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use url::Url;

/// Bumped when the file's shape changes incompatibly.
pub const SCHEMA_VERSION: u32 = 1;

/// Default port for serving to the network, the same as `librebooks serve`.
pub const DEFAULT_NETWORK_PORT: u16 = 8740;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    /// The first-run chooser has been completed.
    pub configured: bool,
    pub mode: BackendMode,
    pub local: LocalSettings,
    pub remote: RemoteSettings,
    pub network: NetworkSettings,
    pub updates: UpdateSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: SCHEMA_VERSION,
            configured: false,
            mode: BackendMode::Local,
            local: LocalSettings::default(),
            remote: RemoteSettings::default(),
            network: NetworkSettings::default(),
            updates: UpdateSettings::default(),
        }
    }
}

/// Where the books are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendMode {
    /// On this computer, served by an in-process server.
    #[default]
    Local,
    /// On a LibreBooks server elsewhere; this app is a thin client.
    Remote,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalSettings {
    /// Where company files live. `None` is the default folder (the same one the
    /// `librebooks` command uses).
    pub data_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteSettings {
    /// The server's base URL, e.g. `https://books.example.com/`.
    pub url: Option<String>,
    /// Allow `http://` to a host other than this computer. Off by default: the
    /// session would cross the network in the clear.
    pub allow_insecure_http: bool,
    /// This computer's device key enrolment with that server.
    pub device: Option<DeviceEnrollment>,
}

/// A device key registered with a server. The private half is in the OS
/// keychain, never here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEnrollment {
    /// The server origin it was enrolled with (`https://host[:port]`).
    pub origin: String,
    /// The server's id for the key (`/auth/device/enroll` → `key_id`).
    pub key_id: String,
    /// What the server calls this computer.
    pub name: String,
    /// Base64url public key, for display and diagnostics.
    pub public_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkSettings {
    /// Also serve the books to other devices (phones, other computers).
    pub enabled: bool,
    pub bind: IpAddr,
    pub port: u16,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        NetworkSettings {
            enabled: false,
            bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: DEFAULT_NETWORK_PORT,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    /// Look for new releases (and say so). On by default.
    pub check_automatically: bool,
    /// Download new releases and install them when the app quits, without
    /// asking. Off by default: opt in.
    pub install_automatically: bool,
    /// Hours between automatic checks.
    pub check_interval_hours: u32,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        UpdateSettings {
            check_automatically: true,
            install_automatically: false,
            check_interval_hours: 24,
        }
    }
}

/// Why settings can't be used.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("{0}")]
    Invalid(String),
    #[error("couldn't save settings to {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl Settings {
    /// Read the settings file. A missing file is first run; an unreadable one is
    /// set aside (`settings.json.bad`) and replaced by defaults, so a damaged file
    /// can never stop the app from starting.
    pub fn load(path: &Path) -> Settings {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
            Err(e) => {
                log::error!("couldn't read {}: {e}; using defaults", path.display());
                return Settings::default();
            }
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(s) => s,
            Err(e) => {
                let aside = path.with_extension("json.bad");
                log::error!(
                    "{} is damaged ({e}); moved it to {} and started from defaults",
                    path.display(),
                    aside.display()
                );
                let _ = std::fs::rename(path, &aside);
                Settings::default()
            }
        }
    }

    /// Write the settings file atomically (write a sibling, then rename).
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            let mut text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
            text.push('\n');
            std::fs::write(&tmp, text)?;
            std::fs::rename(&tmp, path)
        };
        write().map_err(|source| SettingsError::Write {
            path: path.to_owned(),
            source,
        })
    }

    /// Check the settings make sense before saving or acting on them.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.mode == BackendMode::Remote {
            let url = self
                .remote
                .url
                .as_deref()
                .filter(|u| !u.trim().is_empty())
                .ok_or_else(|| {
                    SettingsError::Invalid("enter the address of the LibreBooks server".into())
                })?;
            parse_server_url(url, self.remote.allow_insecure_http)?;
        }
        if self.network.enabled {
            if self.mode == BackendMode::Remote {
                return Err(SettingsError::Invalid(
                    "serving to the network is for books on this computer; a thin client has nothing to serve".into(),
                ));
            }
            if self.network.port == 0 {
                return Err(SettingsError::Invalid(
                    "choose a port for the network server".into(),
                ));
            }
            crate::network::available().map_err(|e| SettingsError::Invalid(e.to_string()))?;
        }
        if self.updates.check_interval_hours == 0 {
            return Err(SettingsError::Invalid(
                "the update check interval must be at least an hour".into(),
            ));
        }
        Ok(())
    }

    /// The folder holding company files.
    pub fn data_dir(&self) -> PathBuf {
        self.local.data_dir.clone().unwrap_or_else(default_data_dir)
    }

    /// The device enrolment for the configured server, if it still matches it.
    pub fn device_for_remote(&self) -> Option<&DeviceEnrollment> {
        let url =
            parse_server_url(self.remote.url.as_deref()?, self.remote.allow_insecure_http).ok()?;
        let origin = url.origin().ascii_serialization();
        self.remote.device.as_ref().filter(|d| d.origin == origin)
    }
}

/// The default company folder: the same one `librebooks serve` uses, so the app
/// and the command see the same books.
pub fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("org", "LibreBooks", "LibreBooks")
        .map_or_else(|| PathBuf::from("LibreBooks"), |d| d.data_dir().to_owned())
}

/// Parse and check a server address. Accepts a bare host (`books.example.com`,
/// taken as https) or a full URL. Returns the base URL, ending in `/`.
pub fn parse_server_url(input: &str, allow_insecure_http: bool) -> Result<Url, SettingsError> {
    let input = input.trim();
    let with_scheme = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let mut url = Url::parse(&with_scheme)
        .map_err(|e| SettingsError::Invalid(format!("\"{input}\" isn't a web address: {e}")))?;
    let host = url
        .host()
        .ok_or_else(|| SettingsError::Invalid(format!("\"{input}\" has no host name")))?;
    let loopback = match host {
        url::Host::Domain(d) => d.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
    };
    match url.scheme() {
        "https" => {}
        "http" if loopback || allow_insecure_http => {}
        "http" => {
            return Err(SettingsError::Invalid(
                "that server uses http://, which would send your session across the network unencrypted; use https:// or allow insecure connections".into(),
            ));
        }
        other => {
            return Err(SettingsError::Invalid(format!(
                "\"{other}://\" isn't supported; use https://"
            )));
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SettingsError::Invalid(
            "leave user names and passwords out of the address".into(),
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe() {
        let s = Settings::default();
        assert!(!s.configured);
        assert_eq!(s.mode, BackendMode::Local);
        assert!(!s.network.enabled);
        assert!(s.updates.check_automatically);
        assert!(!s.updates.install_automatically, "auto-install is opt-in");
    }

    #[test]
    fn missing_fields_take_defaults() {
        let s: Settings = serde_json::from_str(r#"{"configured":true,"mode":"remote"}"#).unwrap();
        assert!(s.configured);
        assert_eq!(s.mode, BackendMode::Remote);
        assert_eq!(s.network.port, DEFAULT_NETWORK_PORT);
        assert!(s.updates.check_automatically);
    }

    #[test]
    fn round_trips_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
        let s = Settings {
            configured: true,
            local: LocalSettings {
                data_dir: Some(dir.path().join("books")),
            },
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
    }

    #[test]
    fn a_damaged_file_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        assert!(dir.path().join("settings.json.bad").exists());
    }

    #[test]
    fn server_urls() {
        let u = parse_server_url("books.example.com", false).unwrap();
        assert_eq!(u.as_str(), "https://books.example.com/");
        let u = parse_server_url("https://example.com/books?x=1#y", false).unwrap();
        assert_eq!(u.as_str(), "https://example.com/books/");
        assert!(parse_server_url("http://10.0.0.5:8740", false).is_err());
        assert!(parse_server_url("http://10.0.0.5:8740", true).is_ok());
        assert!(parse_server_url("http://127.0.0.1:8740", false).is_ok());
        assert!(parse_server_url("http://localhost:8740", false).is_ok());
        assert!(parse_server_url("ftp://example.com", true).is_err());
        assert!(parse_server_url("https://me:pw@example.com", false).is_err());
    }

    #[test]
    fn remote_mode_needs_a_url() {
        let mut s = Settings {
            mode: BackendMode::Remote,
            ..Settings::default()
        };
        assert!(s.validate().is_err());
        s.remote.url = Some("books.example.com".into());
        s.validate().unwrap();
    }

    #[test]
    fn network_serving_is_refused_until_passkeys() {
        let mut s = Settings::default();
        s.network.enabled = true;
        let err = s.validate().unwrap_err().to_string();
        assert!(err.contains("passkey"), "{err}");
    }

    #[test]
    fn enrolment_must_match_the_server() {
        let mut s = Settings::default();
        s.remote.url = Some("https://a.example.com".into());
        s.remote.device = Some(DeviceEnrollment {
            origin: "https://a.example.com".into(),
            key_id: "k".into(),
            name: "pc".into(),
            public_key: "pk".into(),
        });
        assert!(s.device_for_remote().is_some());
        s.remote.url = Some("https://b.example.com".into());
        assert!(s.device_for_remote().is_none());
    }
}
