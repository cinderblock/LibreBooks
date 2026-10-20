//! The `librebooks` command: run the server, manage company files, check them.
//!
//! Backups, self-update, the tunnel and secrets are in `ops.rs`
//! (`plans/server-ops.md`); running the server as a Windows service or a
//! systemd unit is in `service/`; later subcommands (QuickBooks import) arrive
//! as their crates do; see `plans/librebooks.md`.

mod ops;
mod service;

mod import;

use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use lb_auth::{AuthConfig, AuthStore, EnrolmentPurpose, NewEnrolment, RelyingParty, Role};
use lb_core::CompanyInfo;
use lb_server::{AuthMode, Mode, ServerConfig, Sessions};

#[derive(Parser, Debug)]
#[command(
    name = "librebooks",
    version,
    about = "Open-source double-entry accounting: a drop-in replacement for QuickBooks Desktop."
)]
struct Cli {
    /// Where company files and server data live.
    #[arg(long, global = true, env = "LIBREBOOKS_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// The secret store's key (default `<data dir>/secret.key`). Keep it apart
    /// from the data (a systemd credential, a mounted secret) so a copy of the
    /// data folder alone reveals no credentials.
    #[arg(long, global = true, env = "LIBREBOOKS_SECRET_KEY_FILE")]
    secret_key_file: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the server: the API and the web UI.
    Serve {
        #[command(flatten)]
        args: ServeArgs,
    },
    /// Make an enrolment link: a new person, or another passkey or device for
    /// someone who has one. Single use, short-lived.
    Enroll {
        /// The new person's role.
        #[arg(long, default_value = "bookkeeper")]
        role: String,
        /// Their name, suggested on the enrolment page.
        #[arg(long)]
        name: Option<String>,
        /// Minutes until the link expires.
        #[arg(long, default_value_t = 15)]
        minutes: u32,
        /// The address the link points at.
        #[arg(
            long,
            env = "LIBREBOOKS_PUBLIC_URL",
            default_value = "http://localhost:8740"
        )]
        public_url: String,
    },
    /// Import from another system (QuickBooks Desktop), read-only.
    Import {
        #[command(subcommand)]
        command: import::ImportCommand,
    },
    /// Company files.
    Company {
        #[command(subcommand)]
        command: CompanyCommand,
    },
    /// Check a company file: SQLite's integrity check and that every transaction balances.
    Check {
        /// A `.lbdb` file.
        file: PathBuf,
    },
    /// Backups: make one now, list, verify, restore.
    Backup {
        #[command(subcommand)]
        command: ops::BackupCommand,
    },
    /// Self-update from signed releases.
    Update {
        #[command(subcommand)]
        command: ops::UpdateCommand,
    },
    /// The Cloudflare tunnel.
    Tunnel {
        #[command(subcommand)]
        command: ops::TunnelCommand,
    },
    /// The server's encrypted secret store (credentials for backups, feeds,
    /// the tunnel).
    Secret {
        #[command(subcommand)]
        command: ops::SecretCommand,
    },
    /// Run the server as a system service: a Windows service, or a systemd
    /// unit on Linux.
    Service {
        #[command(subcommand)]
        command: service::ServiceCommand,
    },
}

/// `serve`'s options. `service install` takes the same ones and bakes them into
/// the service's command line, so a service runs exactly what `serve` would.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServeArgs {
    /// Addresses to listen on. Repeat for several.
    #[arg(
        long = "bind",
        default_value = "127.0.0.1:8740",
        env = "LIBREBOOKS_BIND"
    )]
    binds: Vec<SocketAddr>,
    /// The address people reach this server at, e.g.
    /// `https://books.example.com`. Passkeys are bound to it. Repeat for
    /// several. `http://localhost:<port>` is always accepted too.
    #[arg(
        long = "public-url",
        env = "LIBREBOOKS_PUBLIC_URL",
        value_delimiter = ','
    )]
    public_urls: Vec<String>,
    /// A header carrying the real client address from a proxy on this
    /// machine (`cf-connecting-ip` behind cloudflared).
    #[arg(long, env = "LIBREBOOKS_CLIENT_IP_HEADER")]
    client_ip_header: Option<String>,
    /// Serve without any sign-in. Only allowed on loopback addresses; for
    /// development.
    #[arg(long)]
    no_auth: bool,
}

impl ServeArgs {
    /// These options as command-line arguments again, for a service's command
    /// line. Everything is spelled out (defaults and values that came from the
    /// environment included), since a service has neither.
    pub(crate) fn to_args(&self) -> Vec<OsString> {
        let mut out: Vec<OsString> = Vec::new();
        for b in &self.binds {
            out.push("--bind".into());
            out.push(b.to_string().into());
        }
        for u in &self.public_urls {
            out.push("--public-url".into());
            out.push(u.into());
        }
        if let Some(h) = &self.client_ip_header {
            out.push("--client-ip-header".into());
            out.push(h.into());
        }
        if self.no_auth {
            out.push("--no-auth".into());
        }
        out
    }
}

#[derive(Subcommand, Debug)]
enum CompanyCommand {
    /// Create a new, empty company.
    Create {
        /// The company's name.
        name: String,
        /// First month of the fiscal year (1 = January).
        #[arg(long, default_value_t = 1)]
        fiscal_year_start: i8,
    },
    /// List the companies in the data directory.
    List,
}

fn default_data_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("org", "LibreBooks", "LibreBooks")
        .context("couldn't find a home directory for the data; pass --data-dir")?;
    Ok(dirs.data_dir().to_owned())
}

/// Where tracing output goes.
#[derive(Debug, Clone, Copy)]
enum LogTarget<'a> {
    /// Standard error, as a command run from a terminal expects.
    Console,
    /// Daily files in `<data dir>/logs` (`server.<date>.log`, the newest 14
    /// kept), for a service that has no console.
    DataDir(&'a Path),
}

/// Install the tracing subscriber. The guard (for a file target) flushes what
/// is still buffered when it is dropped, so it lives until the process ends.
fn init_logging(
    target: LogTarget<'_>,
) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info,tower_http=info".into())
    };
    match target {
        LogTarget::Console => {
            tracing_subscriber::fmt().with_env_filter(filter()).init();
            Ok(None)
        }
        LogTarget::DataDir(data_dir) => {
            let dir = data_dir.join("logs");
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("couldn't create {}", dir.display()))?;
            let file = tracing_appender::rolling::RollingFileAppender::builder()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("server")
                .filename_suffix("log")
                .max_log_files(14)
                .build(&dir)
                .with_context(|| format!("couldn't open a log file in {}", dir.display()))?;
            let (writer, guard) = tracing_appender::non_blocking(file);
            tracing_subscriber::fmt()
                .with_env_filter(filter())
                .with_ansi(false)
                .with_writer(writer)
                .init();
            Ok(Some(guard))
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let data_dir = match cli.data_dir {
        Some(d) => d,
        None => default_data_dir()?,
    };
    // A Windows service has no console; its log goes to the data folder. (Under
    // systemd `service run` is `serve`, and the journal keeps standard error.)
    let as_service = cfg!(windows)
        && matches!(
            cli.command,
            Command::Service {
                command: service::ServiceCommand::Run { .. }
            }
        );
    let _log_guard = init_logging(if as_service {
        LogTarget::DataDir(&data_dir)
    } else {
        LogTarget::Console
    })?;
    let result = match cli.command {
        Command::Serve { args } => {
            let restart = serve(
                &data_dir,
                &args,
                cli.secret_key_file,
                ops::Control::console(),
            )?;
            ops::restart_or_exit(restart)
        }
        Command::Enroll {
            role,
            name,
            minutes,
            public_url,
        } => enroll(&data_dir, &role, name, minutes, &public_url),
        Command::Company { command } => company(&data_dir, command),
        Command::Import { command } => import::run(&data_dir, cli.secret_key_file, command),
        Command::Check { file } => check(&file),
        Command::Backup { command } => ops::backup(&data_dir, cli.secret_key_file, command),
        Command::Update { command } => ops::update(&data_dir, &command),
        Command::Tunnel { command } => ops::tunnel(&data_dir, cli.secret_key_file, command),
        Command::Secret { command } => ops::secret(&data_dir, cli.secret_key_file, command),
        Command::Service { command } => service::run(&data_dir, cli.secret_key_file, command),
    };
    if as_service && let Err(e) = &result {
        // Nobody reads a service's standard error; its log is in the data folder.
        tracing::error!("{e:#}");
    }
    result
}

/// Where the server keeps its users, credentials and sessions.
fn auth_db(data_dir: &Path) -> PathBuf {
    data_dir.join("server.db")
}

/// Passkey relying parties: each public URL, plus `localhost` on every
/// loopback port the server listens on.
fn auth_config(binds: &[SocketAddr], public_urls: &[String]) -> Result<AuthConfig> {
    let mut rps = Vec::new();
    for url in public_urls {
        rps.push(RelyingParty::for_origin(
            url.trim_end_matches('/'),
            "LibreBooks",
        )?);
    }
    let local: Vec<String> = binds
        .iter()
        .filter(|a| a.ip().is_loopback() || a.ip().is_unspecified())
        .map(|a| format!("http://localhost:{}", a.port()))
        .collect();
    if !local.is_empty() {
        rps.push(RelyingParty::new("localhost", "LibreBooks", local));
    }
    // A LAN address can't hold a passkey (browsers refuse IP addresses), but a
    // device key works there.
    let extra_device_origins = binds
        .iter()
        .filter(|a| !a.ip().is_loopback() && !a.ip().is_unspecified())
        .map(|a| format!("http://{a}"))
        .collect();
    Ok(AuthConfig {
        relying_parties: rps,
        extra_device_origins,
        ..AuthConfig::default()
    })
}

/// Build the server's configuration and run it until `control` says stop (or
/// an update was installed). The same code serves `librebooks serve` and the
/// Windows service; only the `Control` differs. Returns the restart request,
/// if an update asked for one.
pub(crate) fn serve(
    data_dir: &Path,
    args: &ServeArgs,
    secret_key: Option<PathBuf>,
    control: ops::Control,
) -> Result<Option<lb_update::Restart>> {
    let binds = args.binds.as_slice();
    std::fs::create_dir_all(data_dir)?;
    // Passkeys are bound to hostnames when sign-in starts, so the settings'
    // public URLs and the tunnel's hostname join the command line's here.
    let (_, settings) = lb_server::ops::settings::load_or_create(data_dir)?;
    let mut public_urls: Vec<String> = args
        .public_urls
        .iter()
        .map(|u| u.trim_end_matches('/').to_owned())
        .collect();
    for u in settings.all_public_urls() {
        if !public_urls.contains(&u) {
            public_urls.push(u);
        }
    }
    let public_urls = public_urls.as_slice();
    let sessions = if args.no_auth {
        None
    } else {
        let store = AuthStore::open(&auth_db(data_dir), auth_config(binds, public_urls)?)?;
        let public_origin = public_urls
            .first()
            .map(|u| u.trim_end_matches('/').to_owned())
            .or_else(|| {
                binds
                    .first()
                    .map(|a| format!("http://localhost:{}", a.port()))
            });
        if let Some(first) = store.bootstrap()? {
            let base = public_origin.clone().unwrap_or_default();
            let link = format!("{base}/enroll#token={}", first.token);
            if control.console {
                println!();
                println!(
                    "  No one can sign in yet. Open this link within the hour to create the first admin:"
                );
                println!();
                println!("    {link}");
                println!();
            } else {
                // A service's only voice is its log.
                tracing::warn!(
                    "no one can sign in yet; open this link within the hour to create the first admin: {link}"
                );
            }
        }
        Some(Sessions {
            store,
            public_origin,
            client_ip_header: args.client_ip_header.clone(),
        })
    };
    let config = ServerConfig {
        data_dir: data_dir.to_owned(),
        mode: Mode::Server,
        auth: AuthMode::Open,
        sessions,
    };
    for addr in binds {
        lb_server::check_bind(&config, *addr)?;
    }
    ops::run_server(&config, binds, secret_key, control)
}

fn enroll(
    data_dir: &Path,
    role: &str,
    name: Option<String>,
    minutes: u32,
    public_url: &str,
) -> Result<()> {
    let role = match role {
        "admin" => Role::Admin,
        "bookkeeper" => Role::Bookkeeper,
        "viewer" => Role::Viewer,
        other => bail!("--role is admin, bookkeeper or viewer, not {other:?}"),
    };
    let path = auth_db(data_dir);
    if !path.exists() {
        bail!(
            "{} doesn't exist yet; run `librebooks serve` once first",
            path.display()
        );
    }
    let store = AuthStore::open(&path, AuthConfig::default())?;
    let minted = store.operator_enrolment(&NewEnrolment {
        purpose: EnrolmentPurpose::NewUser,
        role: Some(role),
        user_id: None,
        company_id: None,
        display_name: name,
        ttl_minutes: Some(minutes),
    })?;
    println!(
        "{}/enroll#token={}",
        public_url.trim_end_matches('/'),
        minted.token
    );
    eprintln!("single use; expires {}", minted.expires_at);
    Ok(())
}

fn company(data_dir: &Path, command: CompanyCommand) -> Result<()> {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let companies = lb_server::Companies::new(data_dir.join("companies"), events)?;
    match command {
        CompanyCommand::Create {
            name,
            fiscal_year_start,
        } => {
            if !(1..=12).contains(&fiscal_year_start) {
                bail!("--fiscal-year-start is a month, 1 to 12");
            }
            let info = CompanyInfo {
                name: name.clone(),
                fiscal_year_start_month: fiscal_year_start,
                tax_year_start_month: fiscal_year_start,
                ..CompanyInfo::default()
            };
            let id = companies
                .create(&info)
                .map_err(|e| anyhow::anyhow!(e.message))?;
            println!("created \"{name}\" ({id})");
        }
        CompanyCommand::List => {
            let list = companies.list().map_err(|e| anyhow::anyhow!(e.message))?;
            if list.is_empty() {
                println!("no companies in {}", companies.dir().display());
            }
            for c in list {
                println!("{}  {}", c.id, c.name);
            }
        }
    }
    Ok(())
}

fn check(file: &Path) -> Result<()> {
    let books =
        lb_store::Books::open(file).with_context(|| format!("couldn't open {}", file.display()))?;
    let problems = books.check_integrity()?;
    if problems.is_empty() {
        println!("{}: ok", file.display());
        Ok(())
    } else {
        for p in &problems {
            println!("{p}");
        }
        bail!("{} problem(s) in {}", problems.len(), file.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("librebooks").chain(argv.iter().copied()))
            .expect("parses")
    }

    fn serve_args(cli: Cli) -> ServeArgs {
        match cli.command {
            Command::Serve { args } => args,
            Command::Service {
                command: service::ServiceCommand::Install(install),
            } => install.serve,
            other => panic!("not a serve command: {other:?}"),
        }
    }

    /// What `service install` was given comes back out of `to_args` as
    /// arguments `serve` parses to the same thing.
    #[test]
    fn serve_args_round_trip() {
        let given = serve_args(parse(&[
            "service",
            "install",
            "--bind",
            "127.0.0.1:9000",
            "--bind",
            "[::1]:9001",
            "--public-url",
            "https://books.example.com/,https://b.example.org",
            "--client-ip-header",
            "cf-connecting-ip",
            "--no-auth",
        ]));
        let rendered = given.to_args();
        let strings: Vec<String> = rendered
            .iter()
            .map(|s| s.to_str().expect("utf-8").to_owned())
            .collect();
        assert_eq!(
            strings,
            [
                "--bind",
                "127.0.0.1:9000",
                "--bind",
                "[::1]:9001",
                "--public-url",
                "https://books.example.com/",
                "--public-url",
                "https://b.example.org",
                "--client-ip-header",
                "cf-connecting-ip",
                "--no-auth",
            ]
        );
        let mut argv = vec!["serve"];
        argv.extend(strings.iter().map(String::as_str));
        assert_eq!(serve_args(parse(&argv)), given);
    }

    /// Defaults are spelled out too: a service's command line stands alone.
    #[test]
    fn serve_args_spell_out_defaults() {
        let given = serve_args(parse(&["service", "install"]));
        let rendered = given.to_args();
        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], "--bind");
        assert_eq!(rendered[1], "127.0.0.1:8740");
    }
}
