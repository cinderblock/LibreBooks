//! The whole update against a local stand-in for GitHub: check, download, verify,
//! install into a temporary folder (never over the test binary), roll back.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use lb_update::{
    Build, Channel, CheckOutcome, FeedSource, InstallKind, Phase, Target, TrustError, TrustedKey,
    UpdateConfig, UpdateError, Updater, UpdaterOptions,
};

const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata");
const LINUX: &str = "x86_64-unknown-linux-gnu";
const WINDOWS: &str = "x86_64-pc-windows-msvc";

fn testdata(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(TESTDATA).join(name)).unwrap()
}

/// Files served by path; requests counted.
struct Server {
    files: Mutex<HashMap<String, Vec<u8>>>,
    hits: Mutex<Vec<String>>,
}

async fn serve(State(s): State<Arc<Server>>, uri: Uri) -> Response {
    s.hits.lock().unwrap().push(uri.path().to_owned());
    match s.files.lock().unwrap().get(uri.path()) {
        Some(body) => (StatusCode::OK, body.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// A stand-in for api.github.com and the release downloads.
async fn github() -> (String, Arc<Server>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut files = HashMap::new();
    let releases = String::from_utf8(testdata("releases.json"))
        .unwrap()
        .replace("{BASE}", &base);
    files.insert(
        "/repos/cinderblock/LibreBooks/releases".to_owned(),
        releases.into_bytes(),
    );
    for name in [
        "SHA256SUMS",
        "SHA256SUMS.minisig",
        "librebooks-v0.3.0-x86_64-pc-windows-msvc.zip",
        "librebooks-v0.3.0-x86_64-unknown-linux-gnu.tar.gz",
    ] {
        files.insert(format!("/download/v0.3.0/{name}"), testdata(name));
    }
    let server = Arc::new(Server {
        files: Mutex::new(files),
        hits: Mutex::new(Vec::new()),
    });
    let app = axum::Router::new()
        .fallback(serve)
        .with_state(Arc::clone(&server));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, server)
}

struct Rig {
    updater: Updater,
    exe: PathBuf,
    _dir: tempfile::TempDir,
}

fn rig(base: &str, target: &str, current: &str, kind: Option<InstallKind>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("bin").join(if target.contains("windows") {
        "librebooks.exe"
    } else {
        "librebooks"
    });
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"the old version").unwrap();
    let updater = Updater::new(UpdaterOptions {
        source: FeedSource {
            api_base: base.to_owned(),
            owner: "cinderblock".into(),
            repo: "LibreBooks".into(),
            target: target.to_owned(),
        },
        key: TrustedKey::decode(&String::from_utf8(testdata("test.pub")).unwrap()).unwrap(),
        current: Build::parse(current).unwrap(),
        config: UpdateConfig::default(),
        install_kind: kind.unwrap_or(InstallKind::SelfUpdatable { exe: exe.clone() }),
        target: Target::Path(exe.clone()),
        download_dir: dir.path().join("updates"),
        download_limit: lb_update::DOWNLOAD_LIMIT,
    })
    .unwrap();
    Rig {
        updater,
        exe,
        _dir: dir,
    }
}

async fn full_update(target: &str) {
    let (base, _server) = github().await;
    let rig = rig(&base, target, "0.2.9", None);

    let CheckOutcome::Available(available) = rig.updater.check().await.unwrap() else {
        panic!("v0.3.0 should be offered to v0.2.9");
    };
    assert_eq!(available.version.to_string(), "0.3.0");
    assert_eq!(available.tag, "v0.3.0");
    assert!(available.asset_name.contains(target));
    assert_eq!(
        available.page,
        "https://github.com/cinderblock/LibreBooks/releases/tag/v0.3.0"
    );
    let status = rig.updater.status();
    assert_eq!(status.available.as_ref(), Some(&available));
    assert!(status.last_check.is_some());

    let downloaded = rig.updater.download(&available).await.unwrap();
    assert!(downloaded.path.exists());

    let installed = rig.updater.install(&downloaded).await.unwrap();
    assert_eq!(installed.exe, rig.exe);
    assert_eq!(
        std::fs::read(&rig.exe).unwrap(),
        testdata("fake-binary.bin")
    );
    assert_eq!(
        std::fs::read(&installed.previous).unwrap(),
        b"the old version"
    );
    assert!(matches!(
        rig.updater.status().phase,
        Phase::RestartPending { .. }
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&rig.exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "the new binary is executable");
    }

    lb_update::install::rollback(&Target::Path(rig.exe.clone())).unwrap();
    assert_eq!(std::fs::read(&rig.exe).unwrap(), b"the old version");
}

#[tokio::test]
async fn a_linux_tarball_checks_downloads_and_installs() {
    full_update(LINUX).await;
}

#[tokio::test]
async fn a_windows_zip_checks_downloads_and_installs() {
    full_update(WINDOWS).await;
}

#[tokio::test]
async fn an_up_to_date_build_is_told_so() {
    let (base, _server) = github().await;
    for current in ["0.3.0", "0.3.0-7-gabcdef0-dirty", "0.4.0"] {
        let rig = rig(&base, LINUX, current, None);
        let outcome = rig.updater.check().await.unwrap();
        assert!(
            matches!(outcome, CheckOutcome::UpToDate { .. }),
            "{current}: {outcome:?}"
        );
    }
}

#[tokio::test]
async fn the_prerelease_channel_needs_its_own_signed_sums() {
    let (base, _server) = github().await;
    let rig = rig(&base, LINUX, "0.3.0", None);
    rig.updater.set_config(UpdateConfig {
        channel: Channel::Prerelease,
        ..UpdateConfig::default()
    });
    // v0.4.0-rc.1 is newer, but its sums are not served: refused, not trusted.
    let err = rig.updater.check().await.unwrap_err();
    assert!(
        matches!(err, UpdateError::Http { status: 404, .. }),
        "{err}"
    );
    assert!(matches!(rig.updater.status().phase, Phase::Failed { .. }));
}

#[tokio::test]
async fn a_tampered_archive_is_refused_and_removed() {
    let (base, server) = github().await;
    let mut evil = testdata("librebooks-v0.3.0-x86_64-unknown-linux-gnu.tar.gz");
    let n = evil.len();
    evil[n - 3] ^= 0xFF;
    server.files.lock().unwrap().insert(
        "/download/v0.3.0/librebooks-v0.3.0-x86_64-unknown-linux-gnu.tar.gz".into(),
        evil,
    );
    let rig = rig(&base, LINUX, "0.2.9", None);
    let CheckOutcome::Available(a) = rig.updater.check().await.unwrap() else {
        panic!("offered");
    };
    let err = rig.updater.download(&a).await.unwrap_err();
    assert!(matches!(err, UpdateError::Checksum), "{err}");
    let left = std::fs::read_dir(rig.exe.parent().unwrap().parent().unwrap().join("updates"))
        .unwrap()
        .count();
    assert_eq!(left, 0, "the bad download is gone");
    assert_eq!(std::fs::read(&rig.exe).unwrap(), b"the old version");
}

#[tokio::test]
async fn tampered_sums_are_refused() {
    let (base, server) = github().await;
    let mut sums = testdata("SHA256SUMS");
    sums[5] ^= 1;
    server
        .files
        .lock()
        .unwrap()
        .insert("/download/v0.3.0/SHA256SUMS".into(), sums);
    let rig = rig(&base, LINUX, "0.2.9", None);
    let err = rig.updater.check().await.unwrap_err();
    assert!(
        matches!(err, UpdateError::Trust(TrustError::BadSignature)),
        "{err}"
    );
}

#[tokio::test]
async fn downloads_have_a_size_limit() {
    let (base, _server) = github().await;
    let mut r = rig(&base, LINUX, "0.2.9", None);
    // Rebuild with a tiny limit.
    let dir = tempfile::tempdir().unwrap();
    r.updater = Updater::new(UpdaterOptions {
        source: FeedSource {
            api_base: base.clone(),
            owner: "cinderblock".into(),
            repo: "LibreBooks".into(),
            target: LINUX.into(),
        },
        key: TrustedKey::decode(&String::from_utf8(testdata("test.pub")).unwrap()).unwrap(),
        current: Build::parse("0.2.9").unwrap(),
        config: UpdateConfig::default(),
        install_kind: InstallKind::SelfUpdatable { exe: r.exe.clone() },
        target: Target::Path(r.exe.clone()),
        download_dir: dir.path().to_owned(),
        download_limit: 100,
    })
    .unwrap();
    let CheckOutcome::Available(a) = r.updater.check().await.unwrap() else {
        panic!("offered");
    };
    let err = r.updater.download(&a).await.unwrap_err();
    assert!(
        matches!(err, UpdateError::TooLarge { limit: 100, .. }),
        "{err}"
    );
}

#[tokio::test]
async fn a_copy_that_cannot_update_itself_is_never_touched() {
    let (base, _server) = github().await;
    let rig = rig(&base, LINUX, "0.2.9", Some(InstallKind::Container));
    let CheckOutcome::Available(a) = rig.updater.check().await.unwrap() else {
        panic!("still told about it");
    };
    let status = rig.updater.status();
    assert!(!status.can_install);
    assert!(status.advice.unwrap().contains("docker pull"));
    let d = rig.updater.download(&a).await.unwrap();
    let err = rig.updater.install(&d).await.unwrap_err();
    assert!(matches!(err, UpdateError::NotSelfUpdatable(_)), "{err}");
    assert_eq!(std::fs::read(&rig.exe).unwrap(), b"the old version");
}

#[tokio::test]
async fn tick_follows_the_policy() {
    let (base, server) = github().await;
    // Default: check and announce, never install.
    let quiet = rig(&base, LINUX, "0.2.9", None);
    assert!(quiet.updater.tick().await.unwrap().is_none());
    assert!(quiet.updater.status().available.is_some());
    assert_eq!(std::fs::read(&quiet.exe).unwrap(), b"the old version");
    // A second tick soon after does not hit the network again.
    let before = server.hits.lock().unwrap().len();
    assert!(quiet.updater.tick().await.unwrap().is_none());
    assert_eq!(server.hits.lock().unwrap().len(), before);

    // Opted in: one tick checks, downloads, verifies and installs.
    let rig = rig(&base, LINUX, "0.2.9", None);
    rig.updater.set_config(UpdateConfig {
        auto_install: true,
        ..UpdateConfig::default()
    });
    let installed = rig.updater.tick().await.unwrap().expect("installed");
    assert_eq!(installed.version.to_string(), "0.3.0");
    assert_eq!(
        std::fs::read(&rig.exe).unwrap(),
        testdata("fake-binary.bin")
    );
    // Then it waits for the restart.
    assert!(rig.updater.tick().await.unwrap().is_none());
}
