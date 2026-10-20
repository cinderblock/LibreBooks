//! What the server should do about updates right now, as a pure function.
//!
//! Defaults follow the master plan (decision 9): checking is on and only reports;
//! installing automatically is opt-in. Installing needs checking (the switches
//! chain), a copy that can update itself, and is not retried for a version whose
//! install already failed (a person should look first).

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::feed::Channel;
use crate::version::Version;

/// Update settings, saved with the server's configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    /// Look for new releases (and say so). On by default.
    pub check: bool,
    /// Install them without asking, then restart. Off by default.
    pub auto_install: bool,
    pub channel: Channel,
    /// How often to look (ISO 8601 in JSON: `"PT24H"`).
    pub interval: SignedDuration,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        UpdateConfig {
            check: true,
            auto_install: false,
            channel: Channel::Stable,
            interval: SignedDuration::from_hours(24),
        }
    }
}

/// What has happened so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyState {
    pub last_check: Option<Timestamp>,
    /// A verified release newer than this build, if one was found.
    pub available: Option<Version>,
    /// This copy can replace itself ([`crate::InstallKind::can_install`]).
    pub can_install: bool,
    /// A new version is installed and waits for a restart.
    pub restart_pending: bool,
    /// The version whose install last failed.
    pub failed_install: Option<Version>,
}

/// The next thing to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Nothing,
    /// Look for a new release.
    Check,
    /// A newer release exists; show it (and how to get it).
    Announce(Version),
    /// Download, verify and install it.
    Install(Version),
}

/// Decide the next step.
pub fn next_step(config: &UpdateConfig, state: &PolicyState, now: Timestamp) -> Step {
    if state.restart_pending || !config.check {
        return Step::Nothing;
    }
    let due = state
        .last_check
        .is_none_or(|t| t.duration_until(now) >= config.interval);
    if due {
        return Step::Check;
    }
    match &state.available {
        Some(v)
            if config.auto_install
                && state.can_install
                && state.failed_install.as_ref() != Some(v) =>
        {
            Step::Install(v.clone())
        }
        Some(v) => Step::Announce(v.clone()),
        None => Step::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + h * 3600).unwrap()
    }

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn the_defaults_check_and_only_announce() {
        let c = UpdateConfig::default();
        assert!(c.check && !c.auto_install);
        assert_eq!(c.channel, Channel::Stable);
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"interval\":\"PT24H\""), "{json}");
        let back: UpdateConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn decisions() {
        let auto = UpdateConfig {
            auto_install: true,
            ..UpdateConfig::default()
        };
        let off = UpdateConfig {
            check: false,
            auto_install: true,
            ..UpdateConfig::default()
        };
        let base = UpdateConfig::default();
        let checked = |available: Option<&str>, can_install| PolicyState {
            last_check: Some(t(0)),
            available: available.map(v),
            can_install,
            ..PolicyState::default()
        };
        let table: Vec<(&str, &UpdateConfig, PolicyState, Timestamp, Step)> = vec![
            (
                "never checked",
                &base,
                PolicyState::default(),
                t(0),
                Step::Check,
            ),
            (
                "checked recently, nothing new",
                &base,
                checked(None, true),
                t(1),
                Step::Nothing,
            ),
            (
                "a day later",
                &base,
                checked(None, true),
                t(24),
                Step::Check,
            ),
            (
                "found, default config",
                &base,
                checked(Some("0.3.0"), true),
                t(1),
                Step::Announce(v("0.3.0")),
            ),
            (
                "found, auto",
                &auto,
                checked(Some("0.3.0"), true),
                t(1),
                Step::Install(v("0.3.0")),
            ),
            (
                "found, auto, cannot install",
                &auto,
                checked(Some("0.3.0"), false),
                t(1),
                Step::Announce(v("0.3.0")),
            ),
            (
                "auto, but that version failed before",
                &auto,
                PolicyState {
                    failed_install: Some(v("0.3.0")),
                    ..checked(Some("0.3.0"), true)
                },
                t(1),
                Step::Announce(v("0.3.0")),
            ),
            (
                "auto, a newer one than the failure",
                &auto,
                PolicyState {
                    failed_install: Some(v("0.3.0")),
                    ..checked(Some("0.3.1"), true)
                },
                t(1),
                Step::Install(v("0.3.1")),
            ),
            (
                "waiting for a restart",
                &auto,
                PolicyState {
                    restart_pending: true,
                    ..checked(Some("0.3.0"), true)
                },
                t(48),
                Step::Nothing,
            ),
            (
                "checking off",
                &off,
                PolicyState::default(),
                t(0),
                Step::Nothing,
            ),
            (
                "found, recheck due first",
                &auto,
                checked(Some("0.3.0"), true),
                t(30),
                Step::Check,
            ),
        ];
        for (name, config, state, now, want) in table {
            assert_eq!(next_step(config, &state, now), want, "{name}");
        }
    }
}
