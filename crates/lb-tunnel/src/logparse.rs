//! Reading cloudflared's log to know what the tunnel is doing.
//!
//! cloudflared writes one line per event to stderr:
//!
//! ```text
//! 2026-10-19T12:00:01Z INF Registered tunnel connection connIndex=0 connection=… event=0 ip=198.41.200.13 location=sjc06 protocol=quic
//! 2026-10-19T12:00:30Z ERR Serve tunnel error error="timeout: no recent network activity" connIndex=0 event=0 ip=198.41.200.13
//! 2026-10-19T12:00:30Z INF Retrying connection in up to 1s connIndex=0 event=0 ip=198.41.200.13
//! ```
//!
//! A healthy connector holds four connections (`connIndex` 0–3) to two Cloudflare
//! data centres. [`LogParser`] keeps the set that are registered; the supervisor
//! turns that into [`crate::TunnelState`]. A few messages mean nothing will work
//! until a person acts (a bad or revoked token); those become
//! [`LogEvent::Fatal`] with an explanation.
//!
//! Matching is on message prefixes that have been stable across cloudflared
//! releases since 2021; anything unrecognised is kept as a log line and ignored.

use std::collections::BTreeSet;

/// What one log line means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvent {
    /// Connection `n` to the edge is up.
    Registered(u32),
    /// Connection `n` went down (it will be retried by cloudflared).
    Lost(u32),
    /// Nothing will work until a person fixes this.
    Fatal(String),
    /// Some other error, worth showing.
    Error(String),
    /// cloudflared's version, from its start-up banner.
    Version(String),
    Other,
}

/// Classify one line of cloudflared's output.
pub fn classify(line: &str) -> LogEvent {
    let (level, msg) = split_line(line);
    let conn = field(msg, "connIndex").and_then(|v| v.parse::<u32>().ok());
    let error = field(msg, "error");

    if msg.starts_with("Provided Tunnel token is not valid") {
        return LogEvent::Fatal(
            "Cloudflare's tunnel token is not valid. Copy it again from the Cloudflare dashboard (Networks → Tunnels → your tunnel).".into(),
        );
    }
    let unauthorized = error.as_deref().is_some_and(|e| e.contains("Unauthorized"))
        || msg.contains("Unauthorized:");
    if unauthorized
        && (msg.starts_with("Register tunnel error")
            || msg.starts_with("Couldn't start tunnel")
            || msg.starts_with("Unauthorized"))
    {
        return LogEvent::Fatal(
            "Cloudflare refused the tunnel token. The tunnel may have been deleted or its token rotated; copy the current token from the dashboard.".into(),
        );
    }
    if msg.starts_with("Couldn't start tunnel") {
        return LogEvent::Fatal(format!(
            "cloudflared could not start the tunnel: {}",
            error.unwrap_or_else(|| msg.to_owned())
        ));
    }
    if msg.starts_with("Registered tunnel connection")
        && let Some(n) = conn
    {
        return LogEvent::Registered(n);
    }
    const LOST: [&str; 6] = [
        "Unregistered tunnel connection",
        "Connection terminated",
        "Serve tunnel error",
        "Failed to serve",
        "Retrying connection in",
        "Lost connection with the edge",
    ];
    if LOST.iter().any(|p| msg.starts_with(p))
        && let Some(n) = conn
    {
        return LogEvent::Lost(n);
    }
    if let Some(v) = msg.strip_prefix("Version ")
        && v.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        return LogEvent::Version(v.split_whitespace().next().unwrap_or(v).to_owned());
    }
    if matches!(level, Some("ERR" | "FTL")) {
        return LogEvent::Error(match error {
            Some(e) => format!("{}: {e}", msg_head(msg)),
            None => msg.to_owned(),
        });
    }
    LogEvent::Other
}

/// The level and the message, without the timestamp. Lines that are not in the
/// usual shape (usage errors print plain text) are all message.
fn split_line(line: &str) -> (Option<&str>, &str) {
    let line = line.trim();
    let mut parts = line.splitn(3, ' ');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(ts), Some(level), Some(rest))
            if ts.len() >= 20
                && ts.as_bytes()[4] == b'-'
                && ts.contains('T')
                && matches!(level, "DBG" | "INF" | "WRN" | "ERR" | "FTL") =>
        {
            (Some(level), rest)
        }
        _ => (None, line),
    }
}

/// The message before its `key=value` fields.
fn msg_head(msg: &str) -> &str {
    let end = msg
        .match_indices(' ')
        .find(|(i, _)| {
            msg[i + 1..]
                .split(' ')
                .next()
                .is_some_and(|w| w.contains('='))
        })
        .map_or(msg.len(), |(i, _)| i);
    &msg[..end]
}

/// A `key=value` or `key="quoted value"` field of a log message.
fn field(msg: &str, key: &str) -> Option<String> {
    let start = msg
        .match_indices(&format!("{key}="))
        .find(|(i, _)| *i == 0 || msg.as_bytes()[i - 1] == b' ')?
        .0
        + key.len()
        + 1;
    let rest = &msg[start..];
    if let Some(quoted) = rest.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = quoted.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => out.extend(chars.next()),
                '"' => return Some(out),
                c => out.push(c),
            }
        }
        Some(out)
    } else {
        Some(rest.split(' ').next().unwrap_or("").to_owned())
    }
}

/// Follows one cloudflared process's log.
#[derive(Debug, Clone, Default)]
pub struct LogParser {
    connections: BTreeSet<u32>,
    ever_connected: bool,
    fatal: Option<String>,
    last_error: Option<String>,
    version: Option<String>,
}

impl LogParser {
    pub fn new() -> Self {
        LogParser::default()
    }

    /// Take in one line; returns what it meant.
    pub fn feed(&mut self, line: &str) -> LogEvent {
        let event = classify(line);
        match &event {
            LogEvent::Registered(n) => {
                self.connections.insert(*n);
                self.ever_connected = true;
            }
            LogEvent::Lost(n) => {
                self.connections.remove(n);
            }
            LogEvent::Fatal(reason) => self.fatal = Some(reason.clone()),
            LogEvent::Error(e) => self.last_error = Some(e.clone()),
            LogEvent::Version(v) => self.version = Some(v.clone()),
            LogEvent::Other => {}
        }
        event
    }

    /// Connections up now.
    pub fn connections(&self) -> u32 {
        self.connections.len() as u32
    }

    pub fn ever_connected(&self) -> bool {
        self.ever_connected
    }

    pub fn fatal(&self) -> Option<&str> {
        self.fatal.as_deref()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TS: &str = "2026-10-19T12:00:01Z";

    fn l(level: &str, msg: &str) -> String {
        format!("{TS} {level} {msg}")
    }

    #[test]
    fn connections_come_and_go() {
        let mut p = LogParser::new();
        assert_eq!(
            p.feed(&l(
                "INF",
                "Starting tunnel tunnelID=6ff42ae2-765d-4adf-8112-31c55c1551ef"
            )),
            LogEvent::Other
        );
        assert_eq!(
            p.feed(&l("INF", "Version 2026.10.0 (Checksum 1234)")),
            LogEvent::Version("2026.10.0".into())
        );
        for i in 0..4 {
            let line = l(
                "INF",
                &format!(
                    "Registered tunnel connection connIndex={i} connection=1b0d2c9e-0000-4000-8000-00000000000{i} event=0 ip=198.41.200.13 location=sjc06 protocol=quic"
                ),
            );
            assert_eq!(p.feed(&line), LogEvent::Registered(i));
        }
        assert_eq!(p.connections(), 4);
        assert_eq!(
            p.feed(&l("ERR", r#"Serve tunnel error error="timeout: no recent network activity" connIndex=2 event=0 ip=198.41.200.13"#)),
            LogEvent::Lost(2)
        );
        assert_eq!(
            p.feed(&l(
                "INF",
                "Retrying connection in up to 1s connIndex=2 event=0 ip=198.41.200.13"
            )),
            LogEvent::Lost(2)
        );
        assert_eq!(
            p.feed(&l(
                "INF",
                "Unregistered tunnel connection connIndex=1 event=0 ip=198.41.200.13"
            )),
            LogEvent::Lost(1)
        );
        assert_eq!(p.connections(), 2);
        p.feed(&l("INF", "Registered tunnel connection connIndex=2 connection=x event=1 ip=198.41.192.7 location=lax01 protocol=quic"));
        assert_eq!(p.connections(), 3);
        assert!(p.ever_connected());
        assert_eq!(p.version(), Some("2026.10.0"));
    }

    #[test]
    fn a_bad_token_is_fatal_and_explained() {
        // What cloudflared 2026.10.0 prints for TUNNEL_TOKEN=garbage (checked by hand).
        let e = classify("Provided Tunnel token is not valid.");
        assert!(
            matches!(&e, LogEvent::Fatal(m) if m.contains("not valid")),
            "{e:?}"
        );
        let e = classify(&l(
            "ERR",
            r#"Register tunnel error from server side error="Unauthorized: Invalid tunnel secret" connIndex=0 event=0 ip=198.41.200.13"#,
        ));
        assert!(
            matches!(&e, LogEvent::Fatal(m) if m.contains("refused")),
            "{e:?}"
        );
        let e = classify(&l(
            "ERR",
            r#"Couldn't start tunnel error="Unauthorized: Tunnel not found""#,
        ));
        assert!(
            matches!(&e, LogEvent::Fatal(m) if m.contains("refused")),
            "{e:?}"
        );
        let e = classify(&l(
            "ERR",
            r#"Couldn't start tunnel error="no route to host""#,
        ));
        assert_eq!(
            e,
            LogEvent::Fatal("cloudflared could not start the tunnel: no route to host".into())
        );
        let mut p = LogParser::new();
        p.feed("Provided Tunnel token is not valid.");
        assert!(p.fatal().is_some() && !p.ever_connected());
    }

    #[test]
    fn other_errors_are_kept_in_words() {
        let e = classify(&l(
            "ERR",
            r#"Failed to fetch features error="lookup cfd-features.argotunnel.com: no such host""#,
        ));
        assert_eq!(
            e,
            LogEvent::Error(
                "Failed to fetch features: lookup cfd-features.argotunnel.com: no such host".into()
            )
        );
        assert_eq!(
            classify(&l("WRN", "Your version 2026.1.0 is outdated")),
            LogEvent::Other
        );
        assert_eq!(classify(""), LogEvent::Other);
        assert_eq!(
            classify("See 'cloudflared tunnel run --help'."),
            LogEvent::Other
        );
        // A connection event without an index is not counted.
        assert_eq!(
            classify(&l("INF", "Registered tunnel connection")),
            LogEvent::Other
        );
    }

    #[test]
    fn fields_parse_with_and_without_quotes() {
        let m = r#"x error="a \"quoted\" thing" connIndex=3 ip=1.2.3.4"#;
        assert_eq!(field(m, "error").as_deref(), Some(r#"a "quoted" thing"#));
        assert_eq!(field(m, "connIndex").as_deref(), Some("3"));
        assert_eq!(field(m, "ip").as_deref(), Some("1.2.3.4"));
        assert_eq!(field(m, "Index"), None);
        assert_eq!(
            msg_head("Serve tunnel error error=\"x\" connIndex=0"),
            "Serve tunnel error"
        );
    }
}
