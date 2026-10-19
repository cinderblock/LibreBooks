//! The `librebooks` command: run the server, manage company files, check them.
//!
//! Later subcommands (enrolment, backups, QuickBooks import, updates) arrive as
//! their crates do; see `plans/librebooks.md`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use lb_core::CompanyInfo;
use lb_server::{AuthMode, Mode, ServerConfig};

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
        /// Serve without any authentication. Only allowed on loopback addresses;
        /// for development until passkeys are wired in.
        #[arg(long)]
        no_auth: bool,
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
        Command::Serve { binds, no_auth } => serve(&data_dir, &binds, no_auth),
        Command::Company { command } => company(&data_dir, command),
        Command::Check { file } => check(&file),
    }
}

fn serve(data_dir: &Path, binds: &[SocketAddr], no_auth: bool) -> Result<()> {
    if !no_auth {
        bail!(
            "passkey sign-in isn't wired into the server yet; for now run with --no-auth (loopback only)"
        );
    }
    let config = ServerConfig {
        data_dir: data_dir.to_owned(),
        mode: Mode::Server,
        auth: AuthMode::Open,
    };
    for addr in binds {
        lb_server::check_bind(&config, *addr)?;
    }
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let state = lb_server::state(&config)?;
        let app = lb_server::router(state);
        let mut tasks = Vec::new();
        for addr in binds {
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("couldn't listen on {addr}"))?;
            tracing::info!(
                "LibreBooks {} listening on http://{}",
                env!("CARGO_PKG_VERSION"),
                listener.local_addr()?
            );
            let app = app.clone();
            tasks.push(tokio::spawn(async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(shutdown_signal())
                    .await
            }));
        }
        if !lb_server::assets::has_ui() {
            tracing::warn!("this build has no web UI embedded; build web/ first (bun run build)");
        }
        tracing::info!("company files in {}", config.companies_dir().display());
        for t in tasks {
            t.await??;
        }
        anyhow::Ok(())
    })
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
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
