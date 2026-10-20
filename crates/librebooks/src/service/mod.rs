//! `librebooks service`: the headless server as a long-running service that
//! starts with the machine: a native Windows service (`windows.rs`, the
//! Service Control Manager), or a systemd unit on Linux (`systemd.rs`).
//!
//! `install` bakes the data folder and `serve`'s options into the service's
//! command line, so the service runs exactly what `librebooks serve` with the
//! same options would: the same `ServerConfig` code, the same graceful stop.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use crate::ServeArgs;

#[cfg(any(unix, test))]
mod systemd;
#[cfg(windows)]
mod windows;

/// The service's name unless `--name` says otherwise.
#[cfg(windows)]
pub const DEFAULT_NAME: &str = "LibreBooks";
#[cfg(not(windows))]
pub const DEFAULT_NAME: &str = "librebooks";

/// What the service is, for the service manager's listing.
pub const DESCRIPTION: &str = "LibreBooks accounting server";

#[derive(Subcommand, Debug)]
pub enum ServiceCommand {
    /// Register the server as a service that starts with the machine, running
    /// `serve` with the options given here. Nothing starts until `service
    /// start` (or the next boot).
    Install(InstallArgs),
    /// Stop the service if it runs and remove its registration. The data
    /// folder is left alone.
    Uninstall(NameArgs),
    /// Start the service.
    Start(NameArgs),
    /// Stop the service gracefully (as Ctrl+C would): requests in flight
    /// finish, a backup in progress completes, cloudflared stops.
    Stop(NameArgs),
    /// Show the service's state and registration.
    Status(NameArgs),
    /// The entry point the service manager starts; not for a console
    /// (`librebooks serve` is).
    #[command(hide = true)]
    Run {
        #[command(flatten)]
        name: NameArgs,
        #[command(flatten)]
        serve: ServeArgs,
    },
}

#[derive(Args, Debug, Clone)]
pub struct NameArgs {
    /// The service's name.
    #[arg(long, default_value = DEFAULT_NAME)]
    pub name: String,
    /// A system-wide unit (`/etc/systemd/system`, needs root) rather than this
    /// user's (`~/.config/systemd/user`, which stops at logout unless
    /// `loginctl enable-linger` is set).
    #[cfg(unix)]
    #[arg(long)]
    pub system: bool,
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    #[command(flatten)]
    pub name: NameArgs,
    #[command(flatten)]
    pub serve: ServeArgs,
    /// Show what would be registered, without registering anything.
    #[arg(long)]
    pub print: bool,
    /// The account the service runs as: `NT AUTHORITY\LocalService`, a virtual
    /// account, or `.\name` (its password in `LIBREBOOKS_SERVICE_PASSWORD`).
    /// LocalSystem when left out. It must be able to write the data folder and
    /// the program's folder (self-update).
    #[cfg(windows)]
    #[arg(long)]
    pub account: Option<String>,
    /// With `--system`: the user (and group) the unit runs as; root when left
    /// out. The account must own the data folder.
    #[cfg(unix)]
    #[arg(long, value_name = "USER[:GROUP]")]
    pub run_as: Option<String>,
}

/// What the service manager starts: this program, the data folder and
/// `serve`'s options, every path absolute, since a service has neither our
/// working directory nor our environment.
#[derive(Debug, Clone)]
pub(crate) struct Launch {
    pub exe: PathBuf,
    pub data_dir: PathBuf,
    pub secret_key: Option<PathBuf>,
    pub serve: ServeArgs,
}

impl Launch {
    pub fn new(data_dir: &Path, secret_key: Option<PathBuf>, serve: ServeArgs) -> Result<Self> {
        let exe = std::env::current_exe().context("couldn't find this program's path")?;
        Ok(Launch {
            exe: std::path::absolute(&exe)?,
            data_dir: std::path::absolute(data_dir)?,
            secret_key: secret_key.map(|k| std::path::absolute(&k)).transpose()?,
            serve,
        })
    }

    /// The command line after the program: `entry` (`serve`, or `service run
    /// --name …`), then the data folder, the secret key and `serve`'s options.
    pub fn args(&self, entry: &[&str]) -> Vec<OsString> {
        let mut out: Vec<OsString> = entry.iter().map(OsString::from).collect();
        out.push("--data-dir".into());
        out.push(self.data_dir.clone().into_os_string());
        if let Some(key) = &self.secret_key {
            out.push("--secret-key-file".into());
            out.push(key.clone().into_os_string());
        }
        out.extend(self.serve.to_args());
        out
    }
}

pub fn run(data_dir: &Path, secret_key: Option<PathBuf>, command: ServiceCommand) -> Result<()> {
    #[cfg(windows)]
    {
        windows::run(data_dir, secret_key, command)
    }
    #[cfg(unix)]
    {
        systemd::run(data_dir, secret_key, command)
    }
}
