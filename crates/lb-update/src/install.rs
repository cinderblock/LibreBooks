//! Putting a new binary in place, and deciding whether this copy may.
//!
//! ## Which copies update themselves
//!
//! Only a plain binary that LibreBooks' own release archive put somewhere this
//! process can write. Everything else is told what to do instead
//! ([`InstallKind::advice`]):
//!
//! - in a container: pull the new image (replacing the binary inside a container
//!   is undone the next time it is recreated, and the image is the thing to trust);
//! - a development build (`cargo run`, a git-describe version): rebuild;
//! - a package manager's copy (`/usr/bin`, Nix, Snap): use the package manager;
//! - a folder this process cannot write: run the update as its owner;
//! - `LIBREBOOKS_NO_SELF_UPDATE` set: the packager said no.
//!
//! ## Replacing
//!
//! The current binary is first copied to `<name>.old` beside it (rollback: put it
//! back with [`rollback`]); then the new one replaces it. For the running
//! executable that is `self-replace` (on Windows a running .exe cannot be
//! overwritten, only renamed, which `self-replace` handles); for any other path, a
//! copy to a temporary name in the same folder and a rename over the old file.
//!
//! ## Restarting
//!
//! The new binary runs from the next start. [`restart_strategy`] says how: under a
//! supervisor (systemd, a Windows service, `LIBREBOOKS_SUPERVISED=1`) exit with
//! [`RESTART_EXIT_CODE`] and let it restart the process; otherwise start the new
//! binary with the same arguments, then exit.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::UpdateError;
use crate::feed::ArchiveKind;
use crate::version::Build;

/// Exit code asking a supervisor to restart the process (EX_TEMPFAIL).
pub const RESTART_EXIT_CODE: i32 = 75;

/// Largest binary accepted from an archive.
pub const BINARY_LIMIT: u64 = 256 << 20;

/// Whether, and how, this copy can update itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallKind {
    /// A plain binary in a writable folder.
    SelfUpdatable {
        exe: PathBuf,
    },
    Container,
    Development {
        why: String,
    },
    PackageManaged {
        exe: PathBuf,
    },
    NotWritable {
        dir: PathBuf,
    },
    Disabled,
}

impl InstallKind {
    pub fn can_install(&self) -> bool {
        matches!(self, InstallKind::SelfUpdatable { .. })
    }

    /// What to tell a person whose copy cannot update itself.
    pub fn advice(&self) -> Option<String> {
        Some(match self {
            InstallKind::SelfUpdatable { .. } => return None,
            InstallKind::Container => "LibreBooks is running in a container. Update by pulling the new image (docker pull ghcr.io/cinderblock/librebooks) and recreating the container; your data volume is kept.".into(),
            InstallKind::Development { why } => format!(
                "This is a development build ({why}). Build the new version from source, or install a release."
            ),
            InstallKind::PackageManaged { exe } => format!(
                "LibreBooks at {} was installed by a package manager. Update it with that package manager.",
                exe.display()
            ),
            InstallKind::NotWritable { dir } => format!(
                "LibreBooks can't write to {}. Run the update as the user that owns that folder, or download the release by hand.",
                dir.display()
            ),
            InstallKind::Disabled => "Self-update is turned off for this installation (LIBREBOOKS_NO_SELF_UPDATE).".into(),
        })
    }
}

/// What detection looks at, so tests can describe any machine.
pub trait Probe {
    fn env(&self, name: &str) -> Option<String>;
    fn exists(&self, path: &Path) -> bool;
    fn dir_writable(&self, dir: &Path) -> bool;
}

/// The real machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct System;

impl Probe for System {
    fn env(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn dir_writable(&self, dir: &Path) -> bool {
        tempfile::tempfile_in(dir).is_ok()
    }
}

/// How the binary at `exe`, built as `build`, is installed.
pub fn detect(exe: &Path, build: &Build, probe: &dyn Probe) -> InstallKind {
    if probe.env("LIBREBOOKS_NO_SELF_UPDATE").is_some() {
        return InstallKind::Disabled;
    }
    let containerised = probe.env("LIBREBOOKS_CONTAINER").is_some()
        || probe.env("container").is_some()
        || probe.exists(Path::new("/.dockerenv"))
        || probe.exists(Path::new("/run/.containerenv"));
    if containerised {
        return InstallKind::Container;
    }
    if let Some(why) = development(exe, build) {
        return InstallKind::Development { why };
    }
    if package_managed(exe) {
        return InstallKind::PackageManaged {
            exe: exe.to_owned(),
        };
    }
    let dir = exe.parent().unwrap_or(Path::new("."));
    if !probe.dir_writable(dir) {
        return InstallKind::NotWritable {
            dir: dir.to_owned(),
        };
    }
    InstallKind::SelfUpdatable {
        exe: exe.to_owned(),
    }
}

/// [`detect`] for the running process.
pub fn detect_current(build: &Build) -> InstallKind {
    match std::env::current_exe() {
        Ok(exe) => detect(&exe, build, &System),
        Err(e) => InstallKind::NotWritable {
            dir: PathBuf::from(format!("(the program's own path is unknown: {e})")),
        },
    }
}

fn development(exe: &Path, build: &Build) -> Option<String> {
    if !build.is_release() {
        return Some(format!("version {build}"));
    }
    // cargo's output folder: …/target/{debug,release}/ or …/target/<triple>/{debug,release}/
    let parts: Vec<String> = exe
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    let in_target = parts
        .windows(2)
        .any(|w| w[0] == "target" && (w[1] == "debug" || w[1] == "release"))
        || parts
            .windows(3)
            .any(|w| w[0] == "target" && (w[2] == "debug" || w[2] == "release"));
    in_target.then(|| format!("running from cargo's build folder {}", exe.display()))
}

fn package_managed(exe: &Path) -> bool {
    if cfg!(windows) {
        return false;
    }
    let s = exe.to_string_lossy();
    [
        "/usr/bin/",
        "/usr/sbin/",
        "/bin/",
        "/sbin/",
        "/nix/store/",
        "/snap/",
        "/usr/lib/",
    ]
    .iter()
    .any(|p| s.starts_with(p))
        || s.contains("/Cellar/")
}

/// Which file an install replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The running executable.
    RunningExe,
    /// Another file (tests; installing for another process).
    Path(PathBuf),
}

impl Target {
    pub fn path(&self) -> Result<PathBuf, UpdateError> {
        match self {
            Target::RunningExe => {
                std::env::current_exe().map_err(UpdateError::io("find the running program"))
            }
            Target::Path(p) => Ok(p.clone()),
        }
    }
}

/// Where the previous binary is kept: `librebooks.old`, `librebooks.exe.old`.
pub fn backup_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().map(OsString::from).unwrap_or_default();
    name.push(".old");
    exe.with_file_name(name)
}

/// Pull the binary named `binary_name` out of `archive` into a new file at `dest`.
/// It must be at the top level or inside one folder, and appear once.
pub fn extract_binary(
    archive: &Path,
    kind: ArchiveKind,
    binary_name: &str,
    dest: &Path,
) -> Result<(), UpdateError> {
    let file = std::fs::File::open(archive).map_err(UpdateError::io("open the download"))?;
    let mut out = std::fs::File::create(dest).map_err(UpdateError::io("unpack the update"))?;
    let mut found = 0;
    match kind {
        ArchiveKind::Zip => {
            let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
                .map_err(|e| UpdateError::Archive(e.to_string()))?;
            for i in 0..zip.len() {
                let mut entry = zip
                    .by_index(i)
                    .map_err(|e| UpdateError::Archive(e.to_string()))?;
                let Some(path) = entry.enclosed_name() else {
                    continue;
                };
                if entry.is_file() && is_the_binary(&path, binary_name) {
                    found += 1;
                    copy_limited(&mut entry, &mut out)?;
                }
            }
        }
        ArchiveKind::TarGz => {
            let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
            let mut tar = tar::Archive::new(gz);
            let entries = tar
                .entries()
                .map_err(|e| UpdateError::Archive(e.to_string()))?;
            for entry in entries {
                let mut entry = entry.map_err(|e| UpdateError::Archive(e.to_string()))?;
                let path = entry
                    .path()
                    .map_err(|e| UpdateError::Archive(e.to_string()))?
                    .into_owned();
                if entry.header().entry_type().is_file() && is_the_binary(&path, binary_name) {
                    found += 1;
                    copy_limited(&mut entry, &mut out)?;
                }
            }
        }
    }
    out.flush().map_err(UpdateError::io("unpack the update"))?;
    drop(out);
    if found != 1 {
        let _ = std::fs::remove_file(dest);
        return Err(UpdateError::Archive(if found == 0 {
            format!("the archive has no {binary_name}")
        } else {
            format!("the archive has more than one {binary_name}")
        }));
    }
    make_executable(dest)?;
    Ok(())
}

fn is_the_binary(path: &Path, binary_name: &str) -> bool {
    let parts: Vec<Component<'_>> = path.components().collect();
    let plain = parts.iter().all(|c| matches!(c, Component::Normal(_)));
    plain && (1..=2).contains(&parts.len()) && path.file_name().is_some_and(|n| n == binary_name)
}

fn copy_limited(from: &mut impl Read, to: &mut std::fs::File) -> Result<(), UpdateError> {
    let n = std::io::copy(&mut from.take(BINARY_LIMIT + 1), to)
        .map_err(UpdateError::io("unpack the update"))?;
    if n > BINARY_LIMIT {
        return Err(UpdateError::Archive(
            "the binary in the archive is implausibly large".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(UpdateError::io("mark the update executable"))
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn make_executable(_: &Path) -> Result<(), UpdateError> {
    Ok(())
}

/// Replace `target` with the binary at `new`, keeping the old one as
/// [`backup_path`]. Returns the replaced path and the backup's.
pub fn replace_binary(new: &Path, target: &Target) -> Result<(PathBuf, PathBuf), UpdateError> {
    let exe = target.path()?;
    let backup = backup_path(&exe);
    std::fs::copy(&exe, &backup).map_err(UpdateError::io("keep a copy of the current version"))?;
    match target {
        Target::RunningExe => {
            self_replace::self_replace(new).map_err(UpdateError::io("replace the program"))?;
        }
        Target::Path(p) => replace_file(new, p)?,
    }
    tracing::info!(exe = %exe.display(), backup = %backup.display(), "binary replaced");
    Ok((exe, backup))
}

/// Put the previous version (kept by [`replace_binary`]) back.
pub fn rollback(target: &Target) -> Result<PathBuf, UpdateError> {
    let exe = target.path()?;
    let backup = backup_path(&exe);
    if !backup.exists() {
        return Err(UpdateError::NothingToRollBack);
    }
    match target {
        Target::RunningExe => {
            self_replace::self_replace(&backup).map_err(UpdateError::io("restore the program"))?;
        }
        Target::Path(p) => replace_file(&backup, p)?,
    }
    Ok(exe)
}

/// Copy `new` beside `dest` and rename it over `dest`, so `dest` is never half
/// written.
fn replace_file(new: &Path, dest: &Path) -> Result<(), UpdateError> {
    let dir = dest.parent().unwrap_or(Path::new("."));
    let tmp = tempfile::Builder::new()
        .prefix(".librebooks-update-")
        .tempfile_in(dir)
        .map_err(UpdateError::io("write beside the program"))?
        .into_temp_path();
    std::fs::copy(new, &tmp).map_err(UpdateError::io("write beside the program"))?;
    make_executable(&tmp)?;
    tmp.persist(dest).map_err(|e| UpdateError::Io {
        what: "replace the program",
        source: e.error,
    })?;
    Ok(())
}

/// How to get the new version running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Restart {
    /// Exit with `code`; the supervisor starts the new binary.
    ExitForSupervisor { code: i32 },
    /// Start `exe` with `args`, then exit.
    Respawn {
        exe: PathBuf,
        #[serde(skip)]
        args: Vec<OsString>,
    },
}

/// The restart that suits this process: a supervisor if one is detected
/// (systemd sets `INVOCATION_ID`; a Windows service or another supervisor sets
/// `LIBREBOOKS_SUPERVISED=1`), otherwise a respawn.
pub fn restart_strategy(probe: &dyn Probe) -> Restart {
    let supervised =
        probe.env("INVOCATION_ID").is_some() || probe.env("LIBREBOOKS_SUPERVISED").is_some();
    if supervised {
        Restart::ExitForSupervisor {
            code: RESTART_EXIT_CODE,
        }
    } else {
        Restart::Respawn {
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("librebooks")),
            args: std::env::args_os().skip(1).collect(),
        }
    }
}

impl Restart {
    /// For [`Restart::Respawn`]: start the new process (the caller then exits).
    /// For [`Restart::ExitForSupervisor`]: nothing; the caller exits with the code.
    pub fn spawn(&self) -> std::io::Result<Option<std::process::Child>> {
        match self {
            Restart::ExitForSupervisor { .. } => Ok(None),
            Restart::Respawn { exe, args } => {
                std::process::Command::new(exe).args(args).spawn().map(Some)
            }
        }
    }

    /// The exit code to leave with after [`Restart::spawn`].
    pub fn exit_code(&self) -> i32 {
        match self {
            Restart::ExitForSupervisor { code } => *code,
            Restart::Respawn { .. } => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake {
        env: HashMap<&'static str, &'static str>,
        files: Vec<&'static str>,
        read_only: bool,
    }

    impl Probe for Fake {
        fn env(&self, name: &str) -> Option<String> {
            self.env.get(name).map(|v| (*v).to_owned())
        }
        fn exists(&self, path: &Path) -> bool {
            self.files.iter().any(|f| Path::new(f) == path)
        }
        fn dir_writable(&self, _: &Path) -> bool {
            !self.read_only
        }
    }

    fn release() -> Build {
        Build::parse("0.3.0").unwrap()
    }

    #[test]
    fn a_plain_binary_in_a_writable_folder_updates_itself() {
        let exe = if cfg!(windows) {
            r"C:\LibreBooks\librebooks.exe"
        } else {
            "/opt/librebooks/librebooks"
        };
        let kind = detect(Path::new(exe), &release(), &Fake::default());
        assert!(kind.can_install(), "{kind:?}");
        assert_eq!(kind.advice(), None);
    }

    #[test]
    fn copies_that_must_not_update_themselves_say_why() {
        let exe = Path::new("/opt/librebooks/librebooks");
        let env = |k, v| Fake {
            env: HashMap::from([(k, v)]),
            ..Fake::default()
        };
        let cases: Vec<(Fake, &Path, Build, &str)> = vec![
            (
                env("LIBREBOOKS_NO_SELF_UPDATE", "1"),
                exe,
                release(),
                "turned off",
            ),
            (
                env("LIBREBOOKS_CONTAINER", "1"),
                exe,
                release(),
                "docker pull",
            ),
            (env("container", "podman"), exe, release(), "container"),
            (
                Fake {
                    files: vec!["/.dockerenv"],
                    ..Fake::default()
                },
                exe,
                release(),
                "container",
            ),
            (
                Fake::default(),
                exe,
                Build::parse("0.3.0-4-gabcdef0-dirty").unwrap(),
                "development build",
            ),
            (
                Fake::default(),
                Path::new("/home/me/LibreBooks/target/debug/librebooks"),
                release(),
                "cargo's build folder",
            ),
            (
                Fake::default(),
                Path::new(
                    "/home/me/LibreBooks/target/x86_64-unknown-linux-musl/release/librebooks",
                ),
                release(),
                "cargo's build folder",
            ),
            (
                Fake {
                    read_only: true,
                    ..Fake::default()
                },
                exe,
                release(),
                "can't write",
            ),
        ];
        for (probe, path, build, needle) in cases {
            let kind = detect(path, &build, &probe);
            assert!(!kind.can_install(), "{kind:?}");
            let advice = kind.advice().unwrap();
            assert!(advice.contains(needle), "{needle:?} not in {advice:?}");
        }
        if !cfg!(windows) {
            let kind = detect(
                Path::new("/usr/bin/librebooks"),
                &release(),
                &Fake::default(),
            );
            assert!(
                matches!(kind, InstallKind::PackageManaged { .. }),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn supervisors_get_an_exit_code_and_others_a_respawn() {
        let systemd = Fake {
            env: HashMap::from([("INVOCATION_ID", "abc")]),
            ..Fake::default()
        };
        let r = restart_strategy(&systemd);
        assert_eq!(r, Restart::ExitForSupervisor { code: 75 });
        assert_eq!(r.exit_code(), 75);
        assert!(r.spawn().unwrap().is_none());
        let service = Fake {
            env: HashMap::from([("LIBREBOOKS_SUPERVISED", "1")]),
            ..Fake::default()
        };
        assert_eq!(restart_strategy(&service).exit_code(), 75);
        let r = restart_strategy(&Fake::default());
        assert!(matches!(r, Restart::Respawn { .. }));
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn backups_sit_beside_the_binary() {
        assert_eq!(
            backup_path(Path::new("/opt/lb/librebooks")),
            Path::new("/opt/lb/librebooks.old")
        );
        assert_eq!(
            backup_path(Path::new("dir/librebooks.exe")),
            Path::new("dir/librebooks.exe.old")
        );
    }

    #[test]
    fn only_a_plain_shallow_path_is_the_binary() {
        assert!(is_the_binary(Path::new("librebooks"), "librebooks"));
        assert!(is_the_binary(
            Path::new("librebooks-v1/librebooks"),
            "librebooks"
        ));
        assert!(!is_the_binary(Path::new("a/b/librebooks"), "librebooks"));
        assert!(!is_the_binary(Path::new("../librebooks"), "librebooks"));
        assert!(!is_the_binary(Path::new("/librebooks"), "librebooks"));
        assert!(!is_the_binary(Path::new("librebooks.sig"), "librebooks"));
    }
}
