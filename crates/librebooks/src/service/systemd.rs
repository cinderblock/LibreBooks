//! The systemd unit for Linux. `service install` writes it, per user
//! (`~/.config/systemd/user`) or system-wide (`/etc/systemd/system`), and the
//! other subcommands are `systemctl`. Generating the unit is a pure function,
//! so it is tested here on every platform; only the driver is Unix-only.

#[cfg(unix)]
use std::path::{Path, PathBuf};

#[cfg(unix)]
use anyhow::{Context, Result, anyhow, bail};

#[cfg(unix)]
use super::{DESCRIPTION, InstallArgs, Launch, NameArgs, ServiceCommand};

/// Whose service manager: the user's own, or the system's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    User,
    System,
}

/// What goes into the unit file.
#[derive(Debug)]
pub(crate) struct UnitSpec<'a> {
    pub description: &'a str,
    /// The command: the program's absolute path, then its arguments.
    pub exec: &'a [String],
    /// Folders that stay writable under `ProtectSystem=strict`: the data
    /// folder, and the program's folder for self-update.
    pub writable: &'a [String],
    pub scope: Scope,
    /// `User=` (and `Group=`, after a colon) for a system unit.
    pub run_as: Option<&'a str>,
}

/// One word of a unit-file command line or path list, in systemd's quoting:
/// double quotes around anything beyond the plain characters, `\` and `"`
/// escaped inside, and `%` doubled so it isn't a specifier.
pub(crate) fn quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@+,".contains(c));
    let arg = arg.replace('%', "%%");
    if plain {
        arg
    } else {
        format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// The unit file's text.
pub(crate) fn unit_file(spec: &UnitSpec<'_>) -> String {
    let words = |ws: &[String]| ws.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ");
    let mut s = String::new();
    s.push_str("[Unit]\n");
    s.push_str(&format!("Description={}\n", spec.description));
    s.push_str("After=network-online.target\n");
    s.push_str("Wants=network-online.target\n");
    s.push('\n');
    s.push_str("[Service]\n");
    s.push_str("Type=simple\n");
    s.push_str(&format!("ExecStart={}\n", words(spec.exec)));
    if let Some(run_as) = spec.run_as {
        let (user, group) = run_as
            .split_once(':')
            .map_or((run_as, None), |(u, g)| (u, Some(g)));
        s.push_str(&format!("User={user}\n"));
        if let Some(group) = group.filter(|g| !g.is_empty()) {
            s.push_str(&format!("Group={group}\n"));
        }
    }
    s.push_str("# Exit 75 after a self-update means \"start me again\".\n");
    s.push_str("Restart=on-failure\n");
    s.push_str("RestartForceExitStatus=75\n");
    s.push_str("RestartSec=2\n");
    s.push_str(
        "# A graceful stop: requests finish, a backup in progress completes, cloudflared stops.\n",
    );
    s.push_str("KillSignal=SIGTERM\n");
    s.push_str("TimeoutStopSec=90\n");
    s.push_str(
        "# Hardening. The data folder and the program's folder (self-update) stay writable.\n",
    );
    s.push_str("NoNewPrivileges=true\n");
    s.push_str("ProtectSystem=strict\n");
    s.push_str(&format!("ReadWritePaths={}\n", words(spec.writable)));
    s.push_str("PrivateTmp=true\n");
    s.push('\n');
    s.push_str("[Install]\n");
    s.push_str(match spec.scope {
        Scope::User => "WantedBy=default.target\n",
        Scope::System => "WantedBy=multi-user.target\n",
    });
    s
}

#[cfg(unix)]
pub fn run(data_dir: &Path, secret_key: Option<PathBuf>, command: ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::Install(args) => install(data_dir, secret_key, args),
        ServiceCommand::Uninstall(name) => uninstall(&name),
        ServiceCommand::Start(name) => systemctl_passthrough(&name, "start"),
        ServiceCommand::Stop(name) => systemctl_passthrough(&name, "stop"),
        ServiceCommand::Status(name) => systemctl_passthrough(&name, "status"),
        // systemd runs `serve` itself; `service run` here is the same thing,
        // so a unit written for one platform's command line still works.
        ServiceCommand::Run { serve, .. } => {
            let restart =
                crate::serve(data_dir, &serve, secret_key, crate::ops::Control::console())?;
            crate::ops::restart_or_exit(restart)
        }
    }
}

#[cfg(unix)]
fn scope_of(name: &NameArgs) -> Scope {
    if name.system {
        Scope::System
    } else {
        Scope::User
    }
}

#[cfg(unix)]
fn unit_name(name: &str) -> String {
    format!("{name}.service")
}

/// Where the unit file lives.
#[cfg(unix)]
fn unit_path(scope: Scope, name: &str) -> Result<PathBuf> {
    let dir = match scope {
        Scope::System => PathBuf::from("/etc/systemd/system"),
        Scope::User => {
            let config = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
                .context("neither XDG_CONFIG_HOME nor HOME is set; pass --system or set HOME")?;
            config.join("systemd").join("user")
        }
    };
    Ok(dir.join(unit_name(name)))
}

#[cfg(unix)]
fn systemctl(scope: Scope, args: &[&str]) -> Result<std::process::ExitStatus> {
    let mut cmd = std::process::Command::new("systemctl");
    if scope == Scope::User {
        cmd.arg("--user");
    }
    cmd.args(args)
        .status()
        .context("couldn't run systemctl (is this a systemd system?)")
}

#[cfg(unix)]
fn systemctl_ok(scope: Scope, args: &[&str]) -> Result<()> {
    let status = systemctl(scope, args)?;
    if status.success() {
        Ok(())
    } else {
        bail!("systemctl {} failed ({status})", args.join(" "))
    }
}

/// `systemctl <verb> <unit>`, with its output and exit status as ours.
#[cfg(unix)]
fn systemctl_passthrough(name: &NameArgs, verb: &str) -> Result<()> {
    let status = systemctl(scope_of(name), &[verb, &unit_name(&name.name)])?;
    match status.code() {
        Some(0) => Ok(()),
        Some(code) => std::process::exit(code),
        None => bail!("systemctl {verb} was killed by a signal"),
    }
}

#[cfg(unix)]
fn utf8(s: std::ffi::OsString) -> Result<String> {
    s.into_string().map_err(|s| {
        anyhow!(
            "{} isn't valid UTF-8; a unit file can't hold it",
            s.display()
        )
    })
}

#[cfg(unix)]
fn install(data_dir: &Path, secret_key: Option<PathBuf>, args: InstallArgs) -> Result<()> {
    let scope = scope_of(&args.name);
    let launch = Launch::new(data_dir, secret_key, args.serve)?;
    let mut exec = vec![utf8(launch.exe.clone().into_os_string())?];
    for a in launch.args(&["serve"]) {
        exec.push(utf8(a)?);
    }
    let exe_dir = launch
        .exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let writable = [
        utf8(launch.data_dir.clone().into_os_string())?,
        utf8(exe_dir.into_os_string())?,
    ];
    let text = unit_file(&UnitSpec {
        description: DESCRIPTION,
        exec: &exec,
        writable: &writable,
        scope,
        run_as: args.run_as.as_deref(),
    });
    let path = unit_path(scope, &args.name.name)?;
    if args.print {
        println!("# {}", path.display());
        print!("{text}");
        return Ok(());
    }
    if scope == Scope::System && args.run_as.is_none() {
        eprintln!(
            "warning: without --run-as the service runs as root; better: `useradd --system --home-dir {} librebooks`, chown the data folder, and --run-as librebooks",
            launch.data_dir.display()
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("couldn't create {}", parent.display()))?;
    }
    std::fs::write(&path, &text).with_context(|| format!("couldn't write {}", path.display()))?;
    let unit = unit_name(&args.name.name);
    systemctl_ok(scope, &["daemon-reload"])?;
    systemctl_ok(scope, &["enable", &unit])?;
    let system_flag = if scope == Scope::System {
        " --system"
    } else {
        ""
    };
    println!(
        "wrote {} and enabled it; start it with `librebooks service start{system_flag}`",
        path.display()
    );
    if scope == Scope::User {
        println!(
            "a user service stops at logout unless `loginctl enable-linger {}` is set",
            std::env::var("USER").unwrap_or_else(|_| "<user>".into())
        );
    }
    Ok(())
}

#[cfg(unix)]
fn uninstall(name: &NameArgs) -> Result<()> {
    let scope = scope_of(name);
    let unit = unit_name(&name.name);
    let path = unit_path(scope, &name.name)?;
    if !path.exists() {
        bail!("there is no unit at {}", path.display());
    }
    // Stops it too; a unit that isn't loaded makes this fail harmlessly.
    let _ = systemctl(scope, &["disable", "--now", &unit]);
    std::fs::remove_file(&path).with_context(|| format!("couldn't remove {}", path.display()))?;
    systemctl_ok(scope, &["daemon-reload"])?;
    println!("removed {}; the data folder was left alone", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(
            quote("/usr/local/bin/librebooks"),
            "/usr/local/bin/librebooks"
        );
        assert_eq!(quote("--bind"), "--bind");
        assert_eq!(quote("127.0.0.1:8740"), "127.0.0.1:8740");
        assert_eq!(quote("/home/me/My Books"), "\"/home/me/My Books\"");
        assert_eq!(quote("50%"), "\"50%%\"");
        assert_eq!(quote(r#"say "hi" \o/"#), r#""say \"hi\" \\o/""#);
        assert_eq!(quote(""), "\"\"");
    }

    #[test]
    fn user_unit_with_spaces_in_paths() {
        let exec = [
            "/home/me/apps/Libre Books/librebooks".to_owned(),
            "serve".to_owned(),
            "--data-dir".to_owned(),
            "/home/me/My Books".to_owned(),
            "--bind".to_owned(),
            "127.0.0.1:8740".to_owned(),
        ];
        let writable = [
            "/home/me/My Books".to_owned(),
            "/home/me/apps/Libre Books".to_owned(),
        ];
        let text = unit_file(&UnitSpec {
            description: "LibreBooks accounting server",
            exec: &exec,
            writable: &writable,
            scope: Scope::User,
            run_as: None,
        });
        assert_eq!(
            text,
            "[Unit]\n\
             Description=LibreBooks accounting server\n\
             After=network-online.target\n\
             Wants=network-online.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             ExecStart=\"/home/me/apps/Libre Books/librebooks\" serve --data-dir \"/home/me/My Books\" --bind 127.0.0.1:8740\n\
             # Exit 75 after a self-update means \"start me again\".\n\
             Restart=on-failure\n\
             RestartForceExitStatus=75\n\
             RestartSec=2\n\
             # A graceful stop: requests finish, a backup in progress completes, cloudflared stops.\n\
             KillSignal=SIGTERM\n\
             TimeoutStopSec=90\n\
             # Hardening. The data folder and the program's folder (self-update) stay writable.\n\
             NoNewPrivileges=true\n\
             ProtectSystem=strict\n\
             ReadWritePaths=\"/home/me/My Books\" \"/home/me/apps/Libre Books\"\n\
             PrivateTmp=true\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n"
        );
    }

    #[test]
    fn system_unit_runs_as_user_and_group() {
        let exec = [
            "/opt/librebooks/librebooks".to_owned(),
            "serve".to_owned(),
            "--data-dir".to_owned(),
            "/var/lib/librebooks".to_owned(),
        ];
        let writable = [
            "/var/lib/librebooks".to_owned(),
            "/opt/librebooks".to_owned(),
        ];
        let text = unit_file(&UnitSpec {
            description: "LibreBooks accounting server",
            exec: &exec,
            writable: &writable,
            scope: Scope::System,
            run_as: Some("librebooks:books"),
        });
        assert!(text.contains(
            "ExecStart=/opt/librebooks/librebooks serve --data-dir /var/lib/librebooks\n"
        ));
        assert!(text.contains("\nUser=librebooks\nGroup=books\n"));
        assert!(text.contains("ReadWritePaths=/var/lib/librebooks /opt/librebooks\n"));
        assert!(text.ends_with("[Install]\nWantedBy=multi-user.target\n"));

        let user_only = unit_file(&UnitSpec {
            description: "LibreBooks accounting server",
            exec: &exec,
            writable: &writable,
            scope: Scope::System,
            run_as: Some("librebooks"),
        });
        assert!(user_only.contains("\nUser=librebooks\n"));
        assert!(!user_only.contains("Group="));
    }
}
