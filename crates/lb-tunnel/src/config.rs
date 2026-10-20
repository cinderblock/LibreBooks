//! Tunnel settings, and the tunnel token.
//!
//! The whole point is that publishing LibreBooks needs only what the Cloudflare
//! dashboard hands out when a tunnel is created: the **token** (and the public
//! hostname, which the dashboard routes to `http://localhost:<port>`, and which
//! LibreBooks needs as the passkey relying-party id).
//!
//! The token itself never sits in the settings: `token_ref` names where it lives
//! (`env:LB_TUNNEL_TOKEN`, `file:/run/secrets/tunnel`, or a scheme the caller's
//! [`TokenSource`] understands, such as the server's secret store).

use std::path::PathBuf;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::TunnelError;

/// Whether and how to run the tunnel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TunnelConfig {
    /// Off unless the user turns it on.
    pub enabled: bool,
    /// Where the tunnel token is (see the module docs).
    pub token_ref: String,
    /// `books.example.com`: the hostname the tunnel's public route uses.
    pub public_hostname: Option<String>,
    /// A cloudflared to use instead of finding one.
    pub cloudflared_path: Option<PathBuf>,
    /// Download (and keep current) Cloudflare's official build when none is
    /// installed.
    pub auto_download: bool,
}

impl Default for TunnelConfig {
    fn default() -> Self {
        TunnelConfig {
            enabled: false,
            token_ref: String::new(),
            public_hostname: None,
            cloudflared_path: None,
            auto_download: true,
        }
    }
}

impl TunnelConfig {
    /// `https://<public_hostname>`, if one is set.
    pub fn public_url(&self) -> Option<String> {
        self.public_hostname
            .as_deref()
            .map(|h| format!("https://{}", h.trim()))
    }

    /// What a person must fix before the tunnel can start.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.enabled && self.token_ref.trim().is_empty() {
            out.push("paste the tunnel token from the Cloudflare dashboard".to_owned());
        }
        if let Some(h) = &self.public_hostname
            && !valid_hostname(h)
        {
            out.push(format!(
                "{h:?} is not a hostname (just the name, like books.example.com: no https://, no path)"
            ));
        }
        out
    }
}

fn valid_hostname(h: &str) -> bool {
    let h = h.trim();
    !h.is_empty()
        && h.len() <= 253
        && h.contains('.')
        && h.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Looks the token up when the tunnel starts.
pub trait TokenSource: Send + Sync {
    /// The token `reference` names.
    ///
    /// # Errors
    /// [`TunnelError::Token`] when it is missing or unreadable.
    fn resolve(&self, reference: &str) -> Result<String, TunnelError>;
}

/// Resolves `env:NAME` and `file:/path` references.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvAndFiles;

impl TokenSource for EnvAndFiles {
    fn resolve(&self, reference: &str) -> Result<String, TunnelError> {
        match reference.split_once(':') {
            Some(("env", name)) => std::env::var(name).map_err(|_| {
                TunnelError::Token(format!("the environment variable {name} is not set"))
            }),
            Some(("file", path)) => std::fs::read_to_string(path)
                .map(|s| s.trim().to_owned())
                .map_err(|e| TunnelError::Token(format!("could not read {path}: {e}"))),
            _ => Err(TunnelError::Token(
                "the token reference must be env:NAME or file:/path".into(),
            )),
        }
    }
}

/// What a tunnel token says about itself. The token is base64 of
/// `{"a": account tag, "t": tunnel id, "s": secret}`; the secret is not kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenInfo {
    pub account_tag: String,
    pub tunnel_id: String,
}

impl TokenInfo {
    /// Check that `token` looks like a tunnel token, so a pasting mistake is
    /// caught before cloudflared starts.
    ///
    /// # Errors
    /// [`TunnelError::Token`] when it is not one.
    pub fn parse(token: &str) -> Result<TokenInfo, TunnelError> {
        #[derive(Deserialize)]
        struct Raw {
            a: String,
            t: String,
            s: String,
        }
        let bad = || {
            TunnelError::Token(
                "that is not a tunnel token; copy it again from the Cloudflare dashboard (Networks → Tunnels → your tunnel → the install command's long value)".into(),
            )
        };
        let token = token.trim();
        // People paste the whole install command; take the last word.
        let token = token.rsplit(' ').next().unwrap_or(token);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(token)
            .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(token))
            .map_err(|_| bad())?;
        let raw: Raw = serde_json::from_slice(&bytes).map_err(|_| bad())?;
        if raw.a.is_empty() || raw.t.is_empty() || raw.s.is_empty() {
            return Err(bad());
        }
        Ok(TokenInfo {
            account_tag: raw.a,
            tunnel_id: raw.t,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn token() -> String {
        base64::engine::general_purpose::STANDARD.encode(
            br#"{"a":"0123456789abcdef0123456789abcdef","t":"6ff42ae2-765d-4adf-8112-31c55c1551ef","s":"c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0"}"#,
        )
    }

    #[test]
    fn tokens_are_recognised_and_mistakes_explained() {
        let info = TokenInfo::parse(&token()).unwrap();
        assert_eq!(info.tunnel_id, "6ff42ae2-765d-4adf-8112-31c55c1551ef");
        // The whole install command pasted.
        let cmd = format!("sudo cloudflared service install {}", token());
        assert_eq!(TokenInfo::parse(&cmd).unwrap(), info);
        for bad in ["", "hello", "eyJhIjoiIn0=", "not base64 !!"] {
            let e = TokenInfo::parse(bad).unwrap_err().to_string();
            assert!(e.contains("not a tunnel token"), "{bad}: {e}");
        }
    }

    #[test]
    fn config_defaults_and_problems() {
        let c: TunnelConfig = serde_json::from_str("{}").unwrap();
        assert!(!c.enabled && c.auto_download);
        assert_eq!(c.problems(), Vec::<String>::new());
        let c = TunnelConfig {
            enabled: true,
            public_hostname: Some("https://books.example.com/".into()),
            ..TunnelConfig::default()
        };
        let p = c.problems().join("\n");
        assert!(
            p.contains("tunnel token") && p.contains("not a hostname"),
            "{p}"
        );
        let c = TunnelConfig {
            public_hostname: Some("books.example.com".into()),
            ..TunnelConfig::default()
        };
        assert_eq!(c.problems(), Vec::<String>::new());
        assert_eq!(c.public_url().as_deref(), Some("https://books.example.com"));
    }

    #[test]
    fn token_references_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t");
        std::fs::write(&f, format!("{}\n", token())).unwrap();
        let got = EnvAndFiles
            .resolve(&format!("file:{}", f.display()))
            .unwrap();
        assert_eq!(got, token());
        assert!(EnvAndFiles.resolve("env:LB_TUNNEL_SURELY_UNSET").is_err());
        assert!(
            EnvAndFiles.resolve(&token()).is_err(),
            "a raw token is not a reference"
        );
    }
}
