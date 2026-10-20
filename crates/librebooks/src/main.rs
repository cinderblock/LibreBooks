//! The `librebooks` command: run the server, manage company files, check them.
//!
//! Backups, self-update, the tunnel and secrets are in `ops.rs`
//! (`plans/server-ops.md`); later subcommands (QuickBooks import) arrive as
//! their crates do; see `plans/librebooks.md`.

mod ops;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();
    let data_dir = match cli.data_dir {
        Some(d) => d,
        None => default_data_dir()?,
    };
    match cli.command {
        Command::Serve {
            binds,
            public_urls,
            client_ip_header,
            no_auth,
        } => serve(
            &data_dir,
            &binds,
            &public_urls,
            client_ip_header,
            no_auth,
            cli.secret_key_file,
        ),
        Command::Enroll {
            role,
            name,
            minutes,
            public_url,
        } => enroll(&data_dir, &role, name, minutes, &public_url),
        Command::Company { command } => company(&data_dir, command),
        Command::Check { file } => check(&file),
        Command::Backup { command } => ops::backup(&data_dir, cli.secret_key_file, command),
        Command::Update { command } => ops::update(&data_dir, &command),
        Command::Tunnel { command } => ops::tunnel(&data_dir, cli.secret_key_file, command),
        Command::Secret { command } => ops::secret(&data_dir, cli.secret_key_file, command),
    }
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

fn serve(
    data_dir: &Path,
    binds: &[SocketAddr],
    public_urls: &[String],
    client_ip_header: Option<String>,
    no_auth: bool,
    secret_key: Option<PathBuf>,
) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    // Passkeys are bound to hostnames when sign-in starts, so the settings'
    // public URLs and the tunnel's hostname join the command line's here.
    let (_, settings) = lb_server::ops::settings::load_or_create(data_dir)?;
    let mut public_urls: Vec<String> = public_urls
        .iter()
        .map(|u| u.trim_end_matches('/').to_owned())
        .collect();
    for u in settings.all_public_urls() {
        if !public_urls.contains(&u) {
            public_urls.push(u);
        }
    }
    let public_urls = public_urls.as_slice();
    let sessions = if no_auth {
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
            println!();
            println!(
                "  No one can sign in yet. Open this link within the hour to create the first admin:"
            );
            println!();
            println!("    {}/enroll#token={}", base, first.token);
            println!();
        }
        Some(Sessions {
            store,
            public_origin,
            client_ip_header,
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
    ops::run_server(&config, binds, secret_key)
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
