//! The `.qwc` file the user adds to the QuickBooks Web Connector once. It tells
//! the Web Connector where LibreBooks is (`AppURL`), who it is (`UserName`,
//! `OwnerID`, `FileID`) and, with `IsReadOnly`, to ask QuickBooks for read-only
//! access, so QuickBooks itself enforces what LibreBooks already guarantees.
//!
//! The Web Connector requires HTTPS for `AppURL` unless the host is `localhost`
//! (a plain-HTTP LAN address is refused when the file is added). `AppSupport`
//! must be on the same domain as `AppURL`.

use quick_xml::escape::escape;
use uuid::Uuid;

/// Everything a `.qwc` holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QwcFile {
    /// Shown in the Web Connector's list: "LibreBooks (Company)".
    pub app_name: String,
    /// Intuit's application id; LibreBooks has none (empty is accepted).
    pub app_id: String,
    /// `https://host/qbwc`.
    pub app_url: String,
    pub app_description: String,
    /// A URL on the same domain as `app_url`.
    pub app_support: String,
    pub user_name: String,
    /// Identifies the application (constant across companies).
    pub owner_id: Uuid,
    /// Identifies this company file pairing.
    pub file_id: Uuid,
    /// Have the Web Connector run on its own every N minutes.
    pub run_every_minutes: Option<u32>,
    /// Ask QuickBooks for read-only access. LibreBooks always sets this.
    pub read_only: bool,
}

/// Why a `.qwc` can't be made.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QwcError {
    #[error("{0:?} isn't a usable URL for the Web Connector")]
    BadUrl(String),
    #[error(
        "the Web Connector only accepts https:// URLs (or http://localhost): {0}. Publish LibreBooks through the Cloudflare tunnel or an HTTPS proxy, or run the Web Connector on the same machine"
    )]
    NotHttps(String),
    #[error("the Web Connector needs a user name")]
    NoUserName,
}

impl QwcFile {
    /// Check what the Web Connector will check when the file is added.
    pub fn validate(&self) -> Result<(), QwcError> {
        let url =
            url::Url::parse(&self.app_url).map_err(|_| QwcError::BadUrl(self.app_url.clone()))?;
        let host = url.host_str().unwrap_or("");
        let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1");
        if url.scheme() != "https" && !(url.scheme() == "http" && local) {
            return Err(QwcError::NotHttps(self.app_url.clone()));
        }
        if self.user_name.trim().is_empty() {
            return Err(QwcError::NoUserName);
        }
        Ok(())
    }

    /// The file's text. The Web Connector expects GUIDs in braces.
    pub fn to_xml(&self) -> String {
        fn el(out: &mut String, name: &str, value: &str) {
            out.push_str(&format!("  <{name}>{}</{name}>\r\n", escape(value)));
        }
        let mut out = String::from("<?xml version=\"1.0\"?>\r\n<QBWCXML>\r\n");
        el(&mut out, "AppName", &self.app_name);
        el(&mut out, "AppID", &self.app_id);
        el(&mut out, "AppURL", &self.app_url);
        el(&mut out, "AppDescription", &self.app_description);
        el(&mut out, "AppSupport", &self.app_support);
        el(&mut out, "UserName", &self.user_name);
        el(
            &mut out,
            "OwnerID",
            &format!("{{{}}}", self.owner_id.as_hyphenated()).to_uppercase(),
        );
        el(
            &mut out,
            "FileID",
            &format!("{{{}}}", self.file_id.as_hyphenated()).to_uppercase(),
        );
        el(&mut out, "QBType", "QBFS");
        if let Some(n) = self.run_every_minutes {
            out.push_str(&format!(
                "  <Scheduler>\r\n    <RunEveryNMinutes>{n}</RunEveryNMinutes>\r\n  </Scheduler>\r\n"
            ));
        }
        el(
            &mut out,
            "IsReadOnly",
            if self.read_only { "true" } else { "false" },
        );
        out.push_str("</QBWCXML>\r\n");
        out
    }

    /// `AppSupport` on the same host as `app_url`: its origin.
    pub fn support_url_for(app_url: &str) -> String {
        url::Url::parse(app_url)
            .ok()
            .map(|u| {
                let mut o = u.origin().ascii_serialization();
                o.push('/');
                o
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> QwcFile {
        QwcFile {
            app_name: "LibreBooks (Larkspur)".into(),
            app_id: String::new(),
            app_url: "https://books.example.com/qbwc".into(),
            app_description: "Read-only import into LibreBooks".into(),
            app_support: "https://books.example.com/".into(),
            user_name: "librebooks-1a2b3c4d".into(),
            owner_id: Uuid::parse_str("57F3B9B1-86F1-4FCC-B1EE-566DE1813D20").unwrap(),
            file_id: Uuid::parse_str("90A44FB5-33D9-4815-AC85-BC87A7E7D1EB").unwrap(),
            run_every_minutes: Some(60),
            read_only: true,
        }
    }

    #[test]
    fn renders_what_the_web_connector_expects() {
        let f = file();
        f.validate().unwrap();
        let xml = f.to_xml();
        assert!(xml.starts_with(
            "<?xml version=\"1.0\"?>\r\n<QBWCXML>\r\n  <AppName>LibreBooks (Larkspur)</AppName>"
        ));
        assert!(xml.contains("<OwnerID>{57F3B9B1-86F1-4FCC-B1EE-566DE1813D20}</OwnerID>"));
        assert!(xml.contains("<FileID>{90A44FB5-33D9-4815-AC85-BC87A7E7D1EB}</FileID>"));
        assert!(xml.contains("<QBType>QBFS</QBType>"));
        assert!(xml.contains("<RunEveryNMinutes>60</RunEveryNMinutes>"));
        assert!(xml.contains("<IsReadOnly>true</IsReadOnly>"));
        crate::qbxml::xml::to_json(&xml).unwrap();
        assert_eq!(
            QwcFile::support_url_for("https://books.example.com:8443/qbwc"),
            "https://books.example.com:8443/"
        );
    }

    #[test]
    fn refuses_plain_http_except_localhost() {
        let mut f = file();
        f.app_url = "http://10.0.0.5:8740/qbwc".into();
        assert!(matches!(f.validate(), Err(QwcError::NotHttps(_))));
        f.app_url = "http://localhost:8740/qbwc".into();
        f.validate().unwrap();
        f.app_url = "not a url".into();
        assert!(matches!(f.validate(), Err(QwcError::BadUrl(_))));
        f.app_url = "https://x.example/qbwc".into();
        f.user_name = " ".into();
        assert_eq!(f.validate(), Err(QwcError::NoUserName));
    }
}
