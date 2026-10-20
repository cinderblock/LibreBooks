//! Fetching cloudflared from a local stand-in for GitHub: checksums from the
//! release notes, GitHub's digests, the macOS archive quirk, and keeping the
//! managed copy current.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use lb_tunnel::TunnelError;
use lb_tunnel::cloudflared::{
    self, Downloader, Origin, Packaging, PlatformAsset, Release, Sources, managed_path,
};
use sha2::{Digest, Sha256};

#[derive(Default)]
struct Server {
    files: Mutex<HashMap<String, Vec<u8>>>,
    hits: Mutex<Vec<String>>,
}

async fn serve(State(s): State<Arc<Server>>, uri: Uri) -> Response {
    s.hits.lock().unwrap().push(uri.path().to_owned());
    match s.files.lock().unwrap().get(uri.path()) {
        Some(b) => (StatusCode::OK, b.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn github() -> (String, Arc<Server>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = Arc::new(Server::default());
    let app = axum::Router::new()
        .fallback(serve)
        .with_state(Arc::clone(&server));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, server)
}

fn sha(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

struct Published {
    name: String,
    bytes: Vec<u8>,
    /// What the release notes say (None: not listed).
    notes_sha: Option<String>,
    /// GitHub's digest (None: not given).
    digest: Option<String>,
}

/// Publish a release on the stand-in; returns it as the API would.
fn publish(base: &str, server: &Server, tag: &str, assets: &[Published]) -> Release {
    let mut body = String::from("Release notes.\n### SHA256 Checksums:\n```\n");
    let mut list = Vec::new();
    for a in assets {
        if let Some(s) = &a.notes_sha {
            body.push_str(&format!("{}: {s}\n", a.name));
        }
        let path = format!("/download/{tag}/{}", a.name);
        server
            .files
            .lock()
            .unwrap()
            .insert(path.clone(), a.bytes.clone());
        list.push(serde_json::json!({
            "name": a.name,
            "size": a.bytes.len(),
            "browser_download_url": format!("{base}{path}"),
            "digest": a.digest.as_ref().map(|d| format!("sha256:{d}")),
        }));
    }
    body.push_str("```\n");
    let release = serde_json::json!({ "tag_name": tag, "body": body, "assets": list });
    server.files.lock().unwrap().insert(
        "/repos/cloudflare/cloudflared/releases/latest".into(),
        serde_json::to_vec(&release).unwrap(),
    );
    serde_json::from_value(release).unwrap()
}

fn raw(name: &str, bytes: &[u8]) -> Published {
    Published {
        name: name.into(),
        bytes: bytes.to_vec(),
        notes_sha: Some(sha(bytes)),
        digest: Some(sha(bytes)),
    }
}

const LINUX: PlatformAsset = PlatformAsset {
    name: "cloudflared-linux-amd64",
    packaging: Packaging::Raw,
};
const MAC: PlatformAsset = PlatformAsset {
    name: "cloudflared-darwin-arm64.tgz",
    packaging: Packaging::Tgz,
};

#[tokio::test]
async fn a_raw_binary_is_verified_and_installed() {
    let (base, server) = github().await;
    let release = publish(
        &base,
        &server,
        "2026.10.1",
        &[raw(LINUX.name, b"cloudflared!")],
    );
    let dl = Downloader::with_api(&base).unwrap();
    assert_eq!(dl.latest().await.unwrap(), release);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("bin").join("cloudflared");
    let v = dl.install(&release, LINUX, &dest).await.unwrap();
    assert_eq!(v, "2026.10.1");
    assert_eq!(std::fs::read(&dest).unwrap(), b"cloudflared!");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o111,
            0o111
        );
    }
    // Nothing else left in the folder.
    assert_eq!(
        std::fs::read_dir(dir.path().join("bin")).unwrap().count(),
        1
    );
}

#[tokio::test]
async fn mismatches_and_missing_checksums_are_refused() {
    let (base, server) = github().await;
    let dl = Downloader::with_api(&base).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("cloudflared");

    let mut wrong_notes = raw(LINUX.name, b"cloudflared!");
    wrong_notes.notes_sha = Some(sha(b"something else"));
    wrong_notes.digest = None;
    let r = publish(&base, &server, "2026.10.1", &[wrong_notes]);
    let err = dl.install(&r, LINUX, &dest).await.unwrap_err();
    assert!(matches!(err, TunnelError::Checksum(_)), "{err}");

    let mut wrong_digest = raw(LINUX.name, b"cloudflared!");
    wrong_digest.digest = Some(sha(b"something else"));
    let r = publish(&base, &server, "2026.10.1", &[wrong_digest]);
    let err = dl.install(&r, LINUX, &dest).await.unwrap_err();
    assert!(matches!(err, TunnelError::Checksum(_)), "{err}");

    let mut unlisted = raw(LINUX.name, b"cloudflared!");
    unlisted.notes_sha = None;
    let r = publish(&base, &server, "2026.10.1", &[unlisted]);
    let err = dl.install(&r, LINUX, &dest).await.unwrap_err();
    assert!(matches!(err, TunnelError::Unverifiable(_)), "{err}");
    assert!(err.to_string().contains("not installed"));

    assert!(!dest.exists());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "no leftovers"
    );
}

fn tgz(inner: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut tar = tar::Builder::new(&mut gz);
        let mut h = tar::Header::new_gnu();
        h.set_size(inner.len() as u64);
        h.set_mode(0o755);
        h.set_cksum();
        tar.append_data(&mut h, "cloudflared", inner).unwrap();
        tar.finish().unwrap();
    }
    gz.flush().unwrap();
    gz.finish().unwrap()
}

#[tokio::test]
async fn a_macos_archive_is_checked_as_cloudflare_publishes_it() {
    let (base, server) = github().await;
    let dl = Downloader::with_api(&base).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("cloudflared");
    let inner = b"mach-o cloudflared";
    let archive = tgz(inner);

    // As 2026.10.1 does it: the notes hash the binary inside, GitHub the archive.
    let r = publish(
        &base,
        &server,
        "2026.10.1",
        &[Published {
            name: MAC.name.into(),
            bytes: archive.clone(),
            notes_sha: Some(sha(inner)),
            digest: Some(sha(&archive)),
        }],
    );
    dl.install(&r, MAC, &dest).await.unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), inner);

    // A different binary inside the same-looking archive is caught.
    let evil = tgz(b"something else");
    let r = publish(
        &base,
        &server,
        "2026.10.1",
        &[Published {
            name: MAC.name.into(),
            bytes: evil.clone(),
            notes_sha: Some(sha(inner)),
            digest: Some(sha(&evil)),
        }],
    );
    let err = dl
        .install(&r, MAC, &dir.path().join("other"))
        .await
        .unwrap_err();
    assert!(matches!(err, TunnelError::Checksum(_)), "{err}");
}

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lb-tunnel-fake-cloudflared"))
}

#[tokio::test]
async fn a_configured_binary_is_used_as_is() {
    let (base, server) = github().await;
    let dl = Downloader::with_api(&base).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let fake = fake();
    let sources = Sources {
        configured: Some(&fake),
        data_dir: dir.path(),
        search_system: false,
    };
    let ready = cloudflared::ensure(&sources, true, &dl).await.unwrap();
    assert_eq!(ready.located.origin, Origin::Configured);
    assert_eq!(ready.version, "2026.10.0");
    assert!(server.hits.lock().unwrap().is_empty(), "never updated");
}

#[tokio::test]
async fn the_managed_copy_is_fetched_when_missing_and_updated_when_old() {
    let Some(platform) = cloudflared::this_platform() else {
        return; // Cloudflare publishes nothing for this machine.
    };
    let (base, server) = github().await;
    let dl = Downloader::with_api(&base).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let sources = Sources {
        configured: None,
        data_dir: dir.path(),
        search_system: false,
    };

    // Not allowed to download: a clear error.
    let err = cloudflared::ensure(&sources, false, &dl).await.unwrap_err();
    assert!(matches!(err, TunnelError::NotInstalled(_)), "{err}");

    // Missing, allowed: the fake is "released" as this platform's asset.
    let fake_bytes = std::fs::read(fake()).unwrap();
    let asset = |bytes: &[u8]| {
        if platform.packaging == Packaging::Tgz {
            let archive = tgz(bytes);
            Published {
                name: platform.name.into(),
                notes_sha: Some(sha(bytes)),
                digest: Some(sha(&archive)),
                bytes: archive,
            }
        } else {
            raw(platform.name, bytes)
        }
    };
    publish(&base, &server, "2026.10.0", &[asset(&fake_bytes)]);
    let ready = cloudflared::ensure(&sources, true, &dl).await.unwrap();
    assert_eq!(ready.located.origin, Origin::Managed);
    assert_eq!(ready.located.path, managed_path(dir.path()));
    assert_eq!(ready.version, "2026.10.0");

    // Up to date: checked, not downloaded again.
    let downloads = |s: &Server| {
        s.hits
            .lock()
            .unwrap()
            .iter()
            .filter(|h| h.starts_with("/download/"))
            .count()
    };
    let before = downloads(&server);
    let ready = cloudflared::ensure(&sources, true, &dl).await.unwrap();
    assert_eq!(ready.version, "2026.10.0");
    assert_eq!(downloads(&server), before);

    // A newer release replaces the managed copy.
    publish(&base, &server, "2026.11.0", &[asset(b"the new one")]);
    let ready = cloudflared::ensure(&sources, true, &dl).await.unwrap();
    assert_eq!(ready.version, "2026.11.0");
    assert_eq!(
        std::fs::read(managed_path(dir.path())).unwrap(),
        b"the new one"
    );
}
