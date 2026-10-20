//! A company's QuickBooks import settings: which transport reads its QuickBooks
//! and how to reach it. Stored by the server in the company file (one JSON
//! setting, [`SETTING_KEY`]) and shown in the UI; credentials are never in here,
//! only references to the server's secret store.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// The company-file setting the JSON lives under.
pub const SETTING_KEY: &str = "qbd.import";

/// How LibreBooks reads this company's QuickBooks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum QbdTransport {
    /// The QB Bridge's REST API, from anywhere.
    #[default]
    Bridge,
    /// The QuickBooks Web Connector polls this server.
    Qbwc,
    /// The QuickBooks SDK over COM: LibreBooks runs on the QuickBooks machine.
    Sdk,
}

impl QbdTransport {
    pub const fn as_str(self) -> &'static str {
        match self {
            QbdTransport::Bridge => "bridge",
            QbdTransport::Qbwc => "qbwc",
            QbdTransport::Sdk => "sdk",
        }
    }
}

impl std::str::FromStr for QbdTransport {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bridge" => Ok(QbdTransport::Bridge),
            "qbwc" | "web-connector" | "webconnector" => Ok(QbdTransport::Qbwc),
            "sdk" | "com" => Ok(QbdTransport::Sdk),
            other => Err(format!("{other:?} isn't a transport (bridge, qbwc or sdk)")),
        }
    }
}

/// The QB Bridge.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct BridgeSettings {
    /// `http://10.255.0.18:8743`.
    pub url: String,
    /// A reference to the API key (`secret:qbd.<company>.bridge-key`); set
    /// through the import settings' write-only `bridge_key`.
    pub key_ref: Option<String>,
    /// Milliseconds between requests, to go easy on a QuickBooks in use.
    #[ts(type = "number")]
    pub delay_ms: u64,
}

/// The Web Connector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct QbwcSettings {
    /// What the Web Connector signs in as (unique across this server's
    /// companies). The password is a secret, set through its own route.
    pub user_name: String,
    /// Override of the server's public URL for `AppURL` (must be HTTPS, or
    /// localhost).
    pub app_url: Option<String>,
    /// Have the Web Connector run on its own every N minutes (the `.qwc`'s
    /// scheduler). `None`: only when the user clicks *Update Selected*.
    pub run_every_minutes: Option<u32>,
    /// The `.qwc`'s `OwnerID` and `FileID`, minted once per company.
    pub owner_id: Uuid,
    pub file_id: Uuid,
}

impl Default for QbwcSettings {
    fn default() -> Self {
        QbwcSettings {
            user_name: String::new(),
            app_url: None,
            run_every_minutes: None,
            owner_id: Uuid::new_v4(),
            file_id: Uuid::new_v4(),
        }
    }
}

/// How the SDK opens the company file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum FileMode {
    /// Whatever mode QuickBooks is in (or single-user if it has to open it).
    #[default]
    DontCare,
    SingleUser,
    MultiUser,
}

/// The SDK.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct SdkSettings {
    pub file_mode: FileMode,
}

/// Everything about one company's import.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct ImportSettings {
    pub transport: QbdTransport,
    pub bridge: BridgeSettings,
    pub qbwc: QbwcSettings,
    pub sdk: SdkSettings,
    /// The `.QBW` path on the QuickBooks machine. The Web Connector and the SDK
    /// use it when QuickBooks has no company open; empty means "whatever
    /// QuickBooks has open".
    pub company_file: String,
}

impl Default for ImportSettings {
    fn default() -> Self {
        ImportSettings {
            transport: QbdTransport::Bridge,
            bridge: BridgeSettings {
                url: String::new(),
                key_ref: None,
                delay_ms: 100,
            },
            qbwc: QbwcSettings::default(),
            sdk: SdkSettings::default(),
            company_file: String::new(),
        }
    }
}

impl ImportSettings {
    /// Fresh settings for a company, with a Web Connector user name derived from
    /// its id so it is unique on this server.
    pub fn new_for(company_id: &str) -> ImportSettings {
        let short: String = company_id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(8)
            .collect();
        ImportSettings {
            qbwc: QbwcSettings {
                user_name: format!("librebooks-{short}"),
                ..QbwcSettings::default()
            },
            ..ImportSettings::default()
        }
    }

    /// What's wrong with these settings for their transport, in words.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        match self.transport {
            QbdTransport::Bridge => {
                if self.bridge.url.trim().is_empty() {
                    out.push("the bridge URL is empty".into());
                } else if let Err(e) = url::Url::parse(&self.bridge.url) {
                    out.push(format!(
                        "the bridge URL {:?} is invalid: {e}",
                        self.bridge.url
                    ));
                }
                if self.bridge.key_ref.is_none() {
                    out.push("the bridge's API key hasn't been set".into());
                }
            }
            QbdTransport::Qbwc => {
                if self.qbwc.user_name.trim().is_empty() {
                    out.push("the Web Connector user name is empty".into());
                }
                if let Some(u) = &self.qbwc.app_url
                    && url::Url::parse(u).is_err()
                {
                    out.push(format!("the Web Connector app URL {u:?} is invalid"));
                }
                if self.qbwc.run_every_minutes == Some(0) {
                    out.push("the Web Connector schedule can't be every 0 minutes".into());
                }
            }
            QbdTransport::Sdk => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_problems() {
        let s = ImportSettings::new_for("0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b");
        assert_eq!(s.qbwc.user_name, "librebooks-0192a1b2");
        assert_eq!(s.transport, QbdTransport::Bridge);
        assert_eq!(s.problems().len(), 2);
        let mut q = ImportSettings {
            transport: QbdTransport::Qbwc,
            ..s.clone()
        };
        assert_eq!(q.problems(), Vec::<String>::new());
        q.qbwc.app_url = Some("nope".into());
        assert_eq!(q.problems().len(), 1);
        assert_eq!(
            "Web-Connector".parse::<QbdTransport>(),
            Ok(QbdTransport::Qbwc)
        );
        assert!("ftp".parse::<QbdTransport>().is_err());
        let json = serde_json::to_string(&s).unwrap();
        let back: ImportSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        // Older settings without newer fields still load.
        let partial: ImportSettings = serde_json::from_str(r#"{"transport":"sdk"}"#).unwrap();
        assert_eq!(partial.transport, QbdTransport::Sdk);
        assert_eq!(partial.bridge.delay_ms, 100);
    }
}
