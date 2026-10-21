//! Supervising a fake cloudflared (a real child process) on every OS.

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use lb_tunnel::{Backoff, SupervisorOptions, Tunnel, TunnelState, TunnelStatus};

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lb-tunnel-fake-cloudflared"))
}

fn token() -> String {
    base64::engine::general_purpose::STANDARD.encode(
        br#"{"a":"0123456789abcdef0123456789abcdef","t":"6ff42ae2-765d-4adf-8112-31c55c1551ef","s":"c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0"}"#,
    )
}

fn options(mode: &str, accepted_token: &str) -> SupervisorOptions {
    let mut o = SupervisorOptions::new(fake(), token());
    o.env = vec![
        ("FAKE_CF_MODE".into(), mode.into()),
        ("FAKE_CF_EXPECT_TOKEN".into(), accepted_token.into()),
    ];
    o.backoff = Backoff {
        initial: Duration::from_millis(50),
        max: Duration::from_millis(400),
        healthy_after: Duration::from_secs(30),
    };
    o.stop_grace = Duration::from_secs(5);
    o.public_url = Some("https://books.example.com".into());
    o
}

async fn wait_for(t: &Tunnel, what: &str, pred: impl Fn(&TunnelStatus) -> bool) -> TunnelStatus {
    let mut rx = t.subscribe();
    let r = tokio::time::timeout(Duration::from_secs(20), rx.wait_for(|s| pred(s))).await;
    match r {
        Ok(Ok(s)) => s.clone(),
        _ => panic!("timed out waiting for {what}; status: {:#?}", t.status()),
    }
}

#[tokio::test]
async fn it_connects_reports_and_stops_cleanly() {
    let t = Tunnel::start(options("connect", &token()));
    let s = wait_for(&t, "four connections", |s| {
        s.state == TunnelState::Connected { connections: 4 }
    })
    .await;
    assert_eq!(s.version.as_deref(), Some("2026.10.0"), "read from the log");
    assert_eq!(
        s.tunnel_id.as_deref(),
        Some("6ff42ae2-765d-4adf-8112-31c55c1551ef")
    );
    assert_eq!(s.public_url.as_deref(), Some("https://books.example.com"));
    assert!(
        s.last_log_lines
            .iter()
            .any(|l| l.contains("Registered tunnel connection"))
    );
    assert_eq!(s.restarts, 0);
    let end = t.stop().await;
    assert_eq!(end.state, TunnelState::Stopped);
}

#[tokio::test]
async fn a_bad_token_fails_with_a_reason_and_keeps_retrying_slowly() {
    let t = Tunnel::start(options("connect", "some-other-token"));
    let s = wait_for(
        &t,
        "the failure",
        |s| matches!(&s.state, TunnelState::Failed { reason } if reason.contains("not valid")),
    )
    .await;
    assert!(s.last_log_lines.iter().any(|l| l.contains("not valid")));
    wait_for(&t, "a retry", |s| s.restarts >= 2).await;
    assert_eq!(t.stop().await.state, TunnelState::Stopped);
}

#[tokio::test]
async fn a_crash_after_connecting_is_reconnecting_then_connected_again() {
    let t = Tunnel::start(options("crash", &token()));
    wait_for(&t, "the first connection", |s| {
        s.state == TunnelState::Connected { connections: 1 }
    })
    .await;
    // The fake exits; the supervisor restarts it and it connects again.
    let s = wait_for(&t, "a reconnection after a restart", |s| {
        s.restarts >= 1 && s.state == TunnelState::Connected { connections: 1 }
    })
    .await;
    assert!(
        s.last_log_lines
            .iter()
            .any(|l| l.contains("connection reset by peer"))
    );
    t.stop().await;
}

#[tokio::test]
async fn connection_changes_are_followed() {
    let t = Tunnel::start(options("flap", &token()));
    wait_for(&t, "two connections after the flap", |s| {
        s.state == TunnelState::Connected { connections: 2 }
            && s.last_log_lines
                .iter()
                .any(|l| l.contains("Retrying connection"))
    })
    .await;
    t.stop().await;
}

#[tokio::test]
async fn the_token_never_reaches_the_status() {
    let t = Tunnel::start(options("leak", &token()));
    let s = wait_for(&t, "one connection", |s| {
        s.state == TunnelState::Connected { connections: 1 }
    })
    .await;
    let all = s.last_log_lines.join("\n");
    assert!(all.contains("debug: using token [token]"), "{all}");
    assert!(!all.contains(&token()));
    assert!(!serde_json::to_string(&s).unwrap().contains(&token()));
    t.stop().await;
}

#[tokio::test]
async fn dropping_the_tunnel_ends_cloudflared() {
    let dir = tempfile::tempdir().unwrap();
    let heartbeat = dir.path().join("beat");
    let mut o = options("connect", &token());
    o.env
        .push(("FAKE_CF_HEARTBEAT".into(), heartbeat.display().to_string()));
    let t = Tunnel::start(o);
    wait_for(&t, "four connections", |s| {
        s.state == TunnelState::Connected { connections: 4 }
    })
    .await;
    // Two distinct beats prove it is alive. A loaded machine (CI) may not have
    // written the first one yet, so wait for them rather than assume a cadence.
    async fn beat(path: &std::path::Path, after: &str) -> String {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let now = std::fs::read_to_string(path).unwrap_or_default();
            if !now.is_empty() && now != after {
                return now;
            }
            assert!(tokio::time::Instant::now() < deadline, "no heartbeat");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    let alive = beat(&heartbeat, "").await;
    beat(&heartbeat, &alive).await;

    drop(t);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let last = std::fs::read_to_string(&heartbeat).unwrap_or_default();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let later = std::fs::read_to_string(&heartbeat).unwrap_or_default();
    assert_eq!(
        last, later,
        "cloudflared still running after the tunnel was dropped"
    );
}

#[tokio::test]
async fn a_missing_binary_is_a_failure_in_words() {
    let mut o = options("connect", &token());
    o.binary = PathBuf::from("definitely-not-cloudflared-here");
    let t = Tunnel::start(o);
    wait_for(&t, "the failure", |s| {
        matches!(&s.state, TunnelState::Failed { reason } if reason.contains("could not start cloudflared"))
    })
    .await;
    t.stop().await;
}

#[tokio::test]
async fn the_version_comes_from_the_binary() {
    let v = lb_tunnel::cloudflared::version_of(&fake()).await.unwrap();
    assert_eq!(v, "2026.10.0");
}
