//! Running the server over the API: settings, write-only secrets, backups,
//! self-update, the tunnel and the Cloudflare Access check. No real internet:
//! GitHub is a local stand-in, and cloudflared is never started.

mod support;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use base64::Engine as _;
use lb_auth::Role;
use lb_server::ops::{self, OpsOptions, UpdaterChoice};
use lb_update::{Build, FeedSource, InstallKind, Target, TrustedKey, UpdateConfig, UpdaterOptions};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use support::{Server, eventually};

const SECRET_VALUE: &str = "wJalrXUtnFEMI-K7MDENG-bPxRfiCYEXAMPLEKEY";

fn tunnel_token() -> String {
    base64::engine::general_purpose::STANDARD.encode(
        br#"{"a":"0123456789abcdef0123456789abcdef","t":"6ff42ae2-765d-4adf-8112-31c55c1551ef","s":"c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0"}"#,
    )
}

#[tokio::test]
async fn a_new_server_has_settings_and_secrets_are_write_only() {
    let s = Server::open(OpsOptions::default());
    let (st, v) = s.call("GET", "/api/v1/server", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["settings"]["backups"]["destinations"][0]["kind"], "local");
    assert_eq!(v["settings"]["updates"]["check"], true);
    assert_eq!(v["settings"]["updates"]["auto_install"], false);
    assert_eq!(v["settings"]["tunnel"]["enabled"], false);
    assert_eq!(v["running"], false);
    assert!(s.dir.path().join("server.json").exists());

    // Set a secret: 204, then only its name is ever shown.
    let (st, _) = s
        .call(
            "PUT",
            "/api/v1/server/secrets/s3-backup",
            Some(json!({ "value": SECRET_VALUE })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let r = s.raw("GET", "/api/v1/server/secrets", None).await;
    assert_eq!(r.json()[0]["name"], "s3-backup");
    for uri in ["/api/v1/server/secrets", "/api/v1/server"] {
        let body = s.raw("GET", uri, None).await.text();
        assert!(!body.contains(SECRET_VALUE), "{uri} leaked the secret");
    }
    for f in ["secrets.age", "server.json"] {
        let on_disk = std::fs::read(s.dir.path().join(f)).unwrap();
        assert!(
            !String::from_utf8_lossy(&on_disk).contains(SECRET_VALUE),
            "{f} holds the secret in clear"
        );
    }
    let (st, v) = s
        .call(
            "PUT",
            "/api/v1/server/secrets/bad%20name",
            Some(json!({ "value": "x" })),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");

    // Settings that refer to it can be saved; a missing one is named.
    let (_, view) = s.call("GET", "/api/v1/server", None).await;
    let mut settings = view["settings"].clone();
    settings["backups"]["destinations"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "kind": "s3", "region": "us-east-1", "bucket": "books", "access_key_id": "AKID",
            "secret_access_key_ref": "secret:nope", "endpoint": "http://127.0.0.1:9"
        }));
    let (st, v) = s
        .call("PUT", "/api/v1/server", Some(settings.clone()))
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"]["detail"]["problems"][0]
            .as_str()
            .unwrap()
            .contains("nope"),
        "{v}"
    );
    settings["backups"]["destinations"][1]["secret_access_key_ref"] = json!("secret:s3-backup");
    let (st, v) = s.call("PUT", "/api/v1/server", Some(settings)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v["settings"]["backups"]["destinations"][1]["secret_access_key_ref"],
        "secret:s3-backup"
    );
    // In use: can't be deleted.
    let (st, v) = s
        .call("DELETE", "/api/v1/server/secrets/s3-backup", None)
        .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "in_use");
    let (st, _) = s
        .call("DELETE", "/api/v1/server/secrets/never-set", None)
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_server_admins_run_the_server() {
    let mut s = Server::with_sessions(OpsOptions::default());
    s.cookie = Some(s.sign_in("Ada", Role::Admin).await);
    let company = s.company("Admin Co").await;
    s.cookie = Some(s.sign_in("Bob", Role::Bookkeeper).await);
    for (method, uri) in [
        ("GET", "/api/v1/server".to_owned()),
        ("GET", "/api/v1/server/secrets".to_owned()),
        ("POST", "/api/v1/server/update/check".to_owned()),
        ("GET", "/api/v1/server/tunnel".to_owned()),
        ("GET", "/api/v1/server/backups/status".to_owned()),
        ("POST", format!("/api/v1/c/{company}/backups/run")),
    ] {
        let (st, v) = s.call(method, &uri, None).await;
        assert!(
            st == StatusCode::FORBIDDEN || st == StatusCode::NOT_FOUND,
            "{method} {uri}: {st} {v}"
        );
    }
    let (st, _) = s
        .call(
            "PUT",
            "/api/v1/server/secrets/x",
            Some(json!({ "value": "y" })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn back_up_list_verify_restore_and_export() {
    let s = Server::open(OpsOptions::default());
    let c = s.company("Backup Co").await;
    s.account(&c, "Checking", "Bank").await;

    let (st, rec) = s
        .call("POST", &format!("/api/v1/c/{c}/backups/run"), None)
        .await;
    assert_eq!(st, StatusCode::OK, "{rec}");
    assert_eq!(rec["destinations"][0]["stored"], true, "{rec}");
    let id = rec["manifest"]["id"].as_str().unwrap().to_owned();
    // Into this company's own folder, by id.
    assert!(
        s.dir.path().join("backups").join(&c).is_dir(),
        "backups/<company id>/"
    );

    let (st, list) = s.call("GET", &format!("/api/v1/c/{c}/backups"), None).await;
    assert_eq!(st, StatusCode::OK, "{list}");
    assert_eq!(list["backups"].as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list["backups"][0]["manifest"]["id"], id.as_str());
    assert_eq!(list["errors"], json!([]));

    let (st, report) = s
        .call("POST", &format!("/api/v1/c/{c}/backups/{id}/verify"), None)
        .await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(report["checksum_ok"], true);
    assert_eq!(report["contents_checked"], true);
    assert_eq!(report["problems"], json!([]));

    let (st, v) = s
        .call("POST", &format!("/api/v1/c/{c}/backups/nope/verify"), None)
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{v}");

    let (st, restored) = s
        .call(
            "POST",
            &format!("/api/v1/c/{c}/backups/{id}/restore"),
            Some(json!({})),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{restored}");
    let new_id = restored["id"].as_str().unwrap();
    assert_ne!(new_id, c, "restore makes a new company");
    let name = restored["name"].as_str().unwrap();
    assert!(name.starts_with("Backup Co (restored from "), "{name}");
    let (_, companies) = s.call("GET", "/api/v1/companies", None).await;
    let names: Vec<&str> = companies
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"Backup Co") && names.contains(&name));
    // The restored books are the backed-up books.
    let (_, accounts) = s
        .call("GET", &format!("/api/v1/c/{new_id}/accounts"), None)
        .await;
    assert!(
        accounts
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["name"] == "Checking"),
        "{accounts}"
    );

    let r = s
        .raw("GET", &format!("/api/v1/c/{c}/backups/export"), None)
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.headers["content-type"], "application/zip");
    assert!(
        r.headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("backup-co.lbk")
    );
    assert_eq!(&r.bytes[..2], b"PK");
}

#[tokio::test]
async fn schedulers_follow_the_companies_and_stop_at_shutdown() {
    let s = Server::open(OpsOptions::default());
    let c = s.company("Scheduled Co").await;
    let background = ops::start(&s.state);
    eventually("the company's first backup", || async {
        let (_, v) = s.call("GET", "/api/v1/server/backups/status", None).await;
        v["companies"].as_array().is_some_and(|a| {
            a.iter().any(|x| {
                x["company_id"] == c.as_str() && x["status"]["last_success_at"].is_string()
            })
        })
    })
    .await;
    let (_, v) = s.call("GET", "/api/v1/server/backups/status", None).await;
    assert_eq!(v["running"], true);
    // Turning backups off stops the schedulers.
    let mut settings = s.state.ops.settings();
    settings.backups.schedule.enabled = false;
    ops::put_settings(&s.state, settings).await.unwrap();
    eventually("the schedulers to stop", || async {
        let (_, v) = s.call("GET", "/api/v1/server/backups/status", None).await;
        v["companies"].as_array().is_some_and(Vec::is_empty)
    })
    .await;
    background.shutdown().await;
    assert!(!s.state.ops.is_running());
}

// ---------------------------------------------------------------------------
// Self-update

fn update_testdata(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../lb-update/testdata")
            .join(name),
    )
    .unwrap()
}

/// A stand-in for api.github.com and its downloads, serving lb-update's fixtures.
async fn github() -> String {
    async fn serve(
        State(files): State<Arc<HashMap<String, Vec<u8>>>>,
        uri: Uri,
    ) -> axum::response::Response {
        match files.get(uri.path()) {
            Some(b) => (StatusCode::OK, b.clone()).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut files = HashMap::new();
    let releases = String::from_utf8(update_testdata("releases.json"))
        .unwrap()
        .replace("{BASE}", &base);
    files.insert(
        "/repos/cinderblock/LibreBooks/releases".to_owned(),
        releases.into_bytes(),
    );
    for name in [
        "SHA256SUMS",
        "SHA256SUMS.minisig",
        "librebooks-v0.3.0-x86_64-unknown-linux-gnu.tar.gz",
    ] {
        files.insert(format!("/download/v0.3.0/{name}"), update_testdata(name));
    }
    let app = axum::Router::new()
        .fallback(serve)
        .with_state(Arc::new(files));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

fn test_updater(base: &str, exe: &Path) -> UpdaterOptions {
    UpdaterOptions {
        source: FeedSource {
            api_base: base.to_owned(),
            owner: "cinderblock".into(),
            repo: "LibreBooks".into(),
            target: "x86_64-unknown-linux-gnu".into(),
        },
        key: TrustedKey::decode(&String::from_utf8(update_testdata("test.pub")).unwrap()).unwrap(),
        current: Build::parse("0.2.9").unwrap(),
        config: UpdateConfig::default(),
        install_kind: InstallKind::SelfUpdatable {
            exe: exe.to_owned(),
        },
        target: Target::Path(exe.to_owned()),
        download_dir: exe.parent().unwrap().join("updates"),
        download_limit: lb_update::DOWNLOAD_LIMIT,
    }
}

#[tokio::test]
async fn a_build_without_a_release_key_says_so() {
    let s = Server::open(OpsOptions::default());
    let (st, v) = s.call("GET", "/api/v1/server/update", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["status"], Value::Null);
    assert!(
        v["unavailable"].as_str().unwrap().contains("signing key"),
        "{v}"
    );
    let (st, v) = s.call("POST", "/api/v1/server/update/check", None).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "unavailable");
}

#[tokio::test]
async fn check_install_and_ask_for_a_restart() {
    let base = github().await;
    let bin = tempfile::tempdir().unwrap();
    let exe: PathBuf = bin.path().join("librebooks");
    std::fs::write(&exe, b"the old version").unwrap();
    let s = Server::open(OpsOptions {
        updater: UpdaterChoice::Custom(Box::new(test_updater(&base, &exe))),
        ..OpsOptions::default()
    });
    let mut restart = s.state.ops.restart_requested();

    let (st, v) = s.call("POST", "/api/v1/server/update/check", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"]["available"]["version"], "0.3.0", "{v}");
    assert_eq!(v["status"]["current"], "0.2.9");

    let (st, v) = s
        .call(
            "POST",
            "/api/v1/server/update/install",
            Some(json!({ "restart": true })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["installed"]["version"], "0.3.0");
    assert_eq!(v["restarting"], true);
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        update_testdata("fake-binary.bin"),
        "the binary was replaced"
    );
    restart.changed().await.unwrap();
    assert!(restart.borrow().is_some(), "a restart was requested");
    let (_, v) = s.call("GET", "/api/v1/server/update", None).await;
    assert_eq!(v["status"]["phase"]["phase"], "restart_pending", "{v}");
}

// ---------------------------------------------------------------------------
// Tunnel

#[tokio::test]
async fn the_tunnel_token_goes_into_the_secret_store() {
    let s = Server::open(OpsOptions {
        cloudflared: ops::CloudflaredOptions {
            search_system: false,
            // Nothing listens here: no download can happen.
            api_base: Some("http://127.0.0.1:9".into()),
        },
        ..OpsOptions::default()
    });
    let (st, v) = s.call("GET", "/api/v1/server/tunnel", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["status"]["state"]["state"], "stopped");
    assert_eq!(v["token_set"], false);

    let (st, v) = s
        .call(
            "PUT",
            "/api/v1/server/tunnel",
            Some(json!({ "token": "not a token" })),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not a tunnel token")
    );

    let missing = s.dir.path().join("no-such-cloudflared");
    let token = tunnel_token();
    let (st, v) = s
        .call(
            "PUT",
            "/api/v1/server/tunnel",
            Some(json!({
                "enabled": true,
                "public_hostname": "books.example.com",
                "token": format!("sudo cloudflared service install {token}"),
                "cloudflared_path": missing,
            })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["token_set"], true);
    assert_eq!(v["token_ref"], "secret:tunnel-token");
    assert_eq!(v["tunnel_id"], "6ff42ae2-765d-4adf-8112-31c55c1551ef");
    assert_eq!(v["public_url"], "https://books.example.com");
    assert!(
        v["problems"][0]
            .as_str()
            .unwrap()
            .contains("librebooks serve"),
        "{v}"
    );
    // Never the token itself.
    for uri in ["/api/v1/server/tunnel", "/api/v1/server"] {
        assert!(
            !s.raw("GET", uri, None).await.text().contains(&token),
            "{uri}"
        );
    }
    let settings = std::fs::read_to_string(s.dir.path().join("server.json")).unwrap();
    assert!(!settings.contains(&token));
    // While on, cloudflared's forwarded address is trusted from loopback.
    assert_eq!(s.state.ops.forwarded_ip_header(), Some("cf-connecting-ip"));

    // Started: the configured cloudflared is missing, which is reported.
    let background = ops::start(&s.state);
    eventually("the tunnel to fail", || async {
        let (_, v) = s.call("GET", "/api/v1/server/tunnel", None).await;
        v["status"]["state"]["state"] == "failed"
    })
    .await;
    let (_, v) = s.call("GET", "/api/v1/server/tunnel", None).await;
    assert!(
        v["status"]["state"]["reason"]
            .as_str()
            .unwrap()
            .contains("does not exist"),
        "{v}"
    );
    let (st, v) = s
        .call(
            "PUT",
            "/api/v1/server/tunnel",
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"]["state"]["state"], "stopped");
    assert_eq!(s.state.ops.forwarded_ip_header(), None);
    background.shutdown().await;
}

#[tokio::test]
async fn a_new_tunnel_hostname_needs_a_restart_for_passkeys() {
    let mut s = Server::with_sessions(OpsOptions::default());
    s.cookie = Some(s.sign_in("Ada", Role::Admin).await);
    let (_, v) = s.call("GET", "/api/v1/server", None).await;
    assert_eq!(v["restart_required"], json!([]));
    let (st, v) = s
        .call(
            "PUT",
            "/api/v1/server/tunnel",
            Some(json!({ "public_hostname": "books.example.com" })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(
        v["restart_required"][0]
            .as_str()
            .unwrap()
            .contains("books.example.com"),
        "{v}"
    );
}

// ---------------------------------------------------------------------------
// Cloudflare Access

#[tokio::test]
async fn requests_through_cloudflare_need_an_access_assertion() {
    let s = Server::open(OpsOptions {
        // Nothing listens here; no request below gets as far as fetching keys.
        access_certs_url: Some("http://127.0.0.1:9/certs".into()),
        ..OpsOptions::default()
    });
    let mut settings = s.state.ops.settings();
    settings.access = Some(lb_tunnel::AccessConfig {
        team_domain: "myteam".into(),
        audience: vec!["aud-tag".into()],
    });
    ops::put_settings(&s.state, settings).await.unwrap();

    // Directly (no cf-ray): unaffected.
    let (st, _) = s.call("GET", "/api/v1/companies", None).await;
    assert_eq!(st, StatusCode::OK);
    // Through Cloudflare without an assertion, or with a malformed one: refused.
    for assertion in [None, Some("not.a.jwt")] {
        let mut req = axum::http::Request::builder()
            .uri("/api/v1/companies")
            .header("cf-ray", "8c0ffee-SJC");
        if let Some(a) = assertion {
            req = req.header("cf-access-jwt-assertion", a);
        }
        let r = s.send(req, axum::body::Body::empty()).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
        assert_eq!(r.json()["error"]["detail"]["reason"], "cloudflare_access");
    }
    // Off again: through Cloudflare is fine.
    let mut settings = s.state.ops.settings();
    settings.access = None;
    ops::put_settings(&s.state, settings).await.unwrap();
    let r = s
        .send(
            axum::http::Request::builder()
                .uri("/api/v1/companies")
                .header("cf-ray", "8c0ffee-SJC"),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
}
