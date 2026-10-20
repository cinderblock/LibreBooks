//! A stand-in for cloudflared, for lb-tunnel's supervision tests only.
//!
//! It answers `--version` like cloudflared, insists on being run as
//! `tunnel --no-autoupdate run` with the token in `TUNNEL_TOKEN` (and not on the
//! command line), and then plays a script chosen by `FAKE_CF_MODE`:
//!
//! - `connect` (default): registers four connections and stays up;
//! - `flap`: registers two, loses one, gets it back, stays up;
//! - `crash`: registers one, then exits with an error;
//! - `leak`: prints the token in a log line, then registers one.
//!
//! `FAKE_CF_EXPECT_TOKEN` is the token it accepts (anything else: "not valid",
//! as the real one says). With `FAKE_CF_HEARTBEAT=<file>` it writes the time to
//! that file every 50 ms while it runs, so a test can tell when it has died.

use std::io::Write as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn log(level: &str, msg: &str) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "2026-10-19T12:00:00Z {level} {msg}");
    let _ = err.flush();
}

fn register(i: u32) {
    log(
        "INF",
        &format!(
            "Registered tunnel connection connIndex={i} connection=00000000-0000-4000-8000-00000000000{i} event=0 ip=198.41.200.13 location=sjc06 protocol=quic"
        ),
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        println!("cloudflared version 2026.10.0 (built 2026-10-05T08:39 UTC)");
        return;
    }
    let token = std::env::var("TUNNEL_TOKEN").unwrap_or_default();
    if !token.is_empty() && args.iter().any(|a| a.contains(&token)) {
        eprintln!("FAKE: the token was on the command line");
        std::process::exit(3);
    }
    if args != ["tunnel", "--no-autoupdate", "run"] {
        eprintln!("FAKE: unexpected arguments {args:?}");
        std::process::exit(2);
    }
    let expected = std::env::var("FAKE_CF_EXPECT_TOKEN").unwrap_or_default();
    if token.is_empty() || token != expected {
        eprintln!("Provided Tunnel token is not valid.");
        eprintln!("See 'cloudflared tunnel run --help'.");
        std::process::exit(255);
    }
    let heartbeat = std::env::var("FAKE_CF_HEARTBEAT").ok();
    let beat = || {
        if let Some(path) = &heartbeat {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let _ = std::fs::write(path, now.to_string());
        }
    };
    let pause = |ms| std::thread::sleep(Duration::from_millis(ms));

    log(
        "INF",
        "Starting tunnel tunnelID=6ff42ae2-765d-4adf-8112-31c55c1551ef",
    );
    log("INF", "Version 2026.10.0 (Checksum 0000)");
    log("INF", "Settings: map[no-autoupdate:true token:*****]");
    beat();
    match std::env::var("FAKE_CF_MODE")
        .as_deref()
        .unwrap_or("connect")
    {
        "crash" => {
            register(0);
            pause(300);
            log(
                "ERR",
                r#"Serve tunnel error error="connection reset by peer" connIndex=0 event=0 ip=198.41.200.13"#,
            );
            std::process::exit(1);
        }
        "flap" => {
            register(0);
            register(1);
            pause(200);
            log(
                "ERR",
                r#"Serve tunnel error error="timeout: no recent network activity" connIndex=1 event=0 ip=198.41.200.13"#,
            );
            log(
                "INF",
                "Retrying connection in up to 1s connIndex=1 event=0 ip=198.41.200.13",
            );
            pause(200);
            register(1);
        }
        "leak" => {
            // A careless log line: the supervisor must not keep the token.
            log("INF", &format!("debug: using token {token}"));
            register(0);
        }
        _ => {
            for i in 0..4 {
                pause(30);
                register(i);
            }
        }
    }
    loop {
        beat();
        pause(50);
    }
}
