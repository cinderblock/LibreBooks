//! Server operations from the command line: backups, self-update, the tunnel and
//! the secret store, plus running the server with its background work.
//!
//! These work on the data folder directly, so they also work while `librebooks
//! serve` is running (SQLite's online backup copies an open file; a restored
//! company appears in the server's list at once). They are the same functions
//! the API calls (`lb_server::ops`).

use std::io::Read as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use lb_core::CompanyId;
use lb_server::ops::backups::{self, UnlockRequest};
use lb_server::ops::tunnel::TOKEN_SECRET;
use lb_server::ops::{self, settings};
use lb_server::{AppState, AuthMode, Mode, OpsOptions, ServerConfig};
use lb_store::Actor;
use lb_tunnel::TokenInfo;
use lb_update::{CheckOutcome, Restart, Updater};

#[derive(Subcommand, Debug)]
pub enum BackupCommand {
    /// Back up now: one company, or every company.
    Now {
        /// A company's id or name; all companies when left out.
        #[arg(long)]
        company: Option<String>,
    },
    /// List backups, newest first.
    List {
        #[arg(long)]
        company: Option<String>,
    },
    /// Check a backup (checksums, and the database inside when it can be opened).
    Verify {
        /// The backup's id (`acme-20261019T023000.123Z`), from `backup list`.
        id: String,
        #[arg(long)]
        company: Option<String>,
        #[command(flatten)]
        unlock: UnlockArgs,
    },
    /// Restore a backup, or import a `.lbk` file, as a new company. Nothing
    /// existing is replaced.
    Restore {
        /// A backup id from `backup list`, or the path of a `.lbk` file.
        what: String,
        #[arg(long)]
        company: Option<String>,
        /// The restored company's name (default: "<name> (restored from <date>)").
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        unlock: UnlockArgs,
    },
}

/// How to open an encrypted backup.
#[derive(clap::Args, Debug)]
pub struct UnlockArgs {
    /// A file holding the age identity (`AGE-SECRET-KEY-1…`) for backups
    /// encrypted to keys.
    #[arg(long)]
    identity: Option<PathBuf>,
    /// An environment variable holding the passphrase (when it isn't the one in
    /// the settings).
    #[arg(long)]
    passphrase_env: Option<String>,
}

impl UnlockArgs {
    fn request(&self) -> Result<UnlockRequest> {
        let mut r = UnlockRequest::default();
        if let Some(path) = &self.identity {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("couldn't read {}", path.display()))?;
            r.identities = text
                .lines()
                .map(str::trim)
                .filter(|l| l.starts_with("AGE-SECRET-KEY-"))
                .map(str::to_owned)
                .collect();
            if r.identities.is_empty() {
                bail!("{} holds no AGE-SECRET-KEY-1… line", path.display());
            }
        }
        if let Some(var) = &self.passphrase_env {
            r.passphrase = Some(std::env::var(var).with_context(|| format!("{var} is not set"))?);
        }
        Ok(r)
    }
}

#[derive(Subcommand, Debug)]
pub enum UpdateCommand {
    /// Look for a newer signed release.
    Check,
    /// Download, verify and install the newest release. Restart the service
    /// afterwards (a running `librebooks serve` with auto-install restarts itself).
    Install,
}

#[derive(Subcommand, Debug)]
pub enum TunnelCommand {
    /// Show the tunnel's settings and the cloudflared that would run. (The live
    /// connection state is the running server's: `GET /api/v1/server/tunnel`.)
    Status,
    /// Turn the tunnel on: the token is read from standard input (or
    /// `LIBREBOOKS_TUNNEL_TOKEN`) and kept in the secret store.
    Setup {
        /// The public hostname the tunnel routes (`books.example.com`).
        #[arg(long)]
        hostname: String,
    },
    /// Turn the tunnel off.
    Disable,
}

#[derive(Subcommand, Debug)]
pub enum SecretCommand {
    /// Save a secret; the value is read from standard input. Refer to it in
    /// `server.json` as `secret:<name>`.
    Set { name: String },
    /// Remove a secret.
    Delete { name: String },
    /// The secrets' names (never their values).
    List,
}

/// The server's state for a command (nothing starts).
pub(crate) fn state(data_dir: &Path, secret_key: Option<PathBuf>) -> Result<AppState> {
    std::fs::create_dir_all(data_dir)?;
    let config = ServerConfig {
        data_dir: data_dir.to_owned(),
        mode: Mode::Server,
        auth: AuthMode::Open,
        sessions: None,
    };
    Ok(lb_server::state_with(
        &config,
        OpsOptions {
            secret_key,
            updater: ops::UpdaterChoice::Off("not used by this command".into()),
            ..OpsOptions::default()
        },
    )?)
}

fn api(e: lb_server::error::ApiError) -> anyhow::Error {
    anyhow!(e.message)
}

/// A company by id or (case-insensitive) name.
fn find_company(state: &AppState, which: &str) -> Result<(CompanyId, String)> {
    let list = state.companies.list().map_err(api)?;
    if let Ok(id) = which.parse::<CompanyId>() {
        // An id works even for a company that no longer exists (its backups do).
        let name = list
            .iter()
            .find(|c| c.id == id)
            .map_or_else(|| id.to_string(), |c| c.name.clone());
        return Ok((id, name));
    }
    let hits: Vec<_> = list
        .iter()
        .filter(|c| c.name.eq_ignore_ascii_case(which))
        .collect();
    match hits.as_slice() {
        [one] => Ok((one.id, one.name.clone())),
        [] => bail!("no company named {which:?}; `librebooks company list` shows them"),
        _ => bail!("several companies are named {which:?}; use the id"),
    }
}

fn companies(state: &AppState, which: Option<&str>) -> Result<Vec<(CompanyId, String)>> {
    match which {
        Some(w) => Ok(vec![find_company(state, w)?]),
        None => Ok(state
            .companies
            .list()
            .map_err(api)?
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect()),
    }
}

pub fn backup(data_dir: &Path, secret_key: Option<PathBuf>, command: BackupCommand) -> Result<()> {
    let state = state(data_dir, secret_key)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let settings = state.ops.settings();
        match command {
            BackupCommand::Now { company } => {
                if !settings.backups_enabled() {
                    bail!(
                        "no backup destinations are set in {}",
                        data_dir.join(settings::SETTINGS_FILE).display()
                    );
                }
                let mut failed = 0;
                for (id, name) in companies(&state, company.as_deref())? {
                    let books = state.companies.get(id).map_err(api)?;
                    match backups::run_now(&state, id, &books, &name).await {
                        Ok(rec) => {
                            println!("{name}: {}", rec.manifest.id);
                            for d in &rec.destinations {
                                match &d.error {
                                    None => println!(
                                        "  stored in {}{}",
                                        d.destination,
                                        if d.pruned.is_empty() {
                                            String::new()
                                        } else {
                                            format!(" ({} old backup(s) pruned)", d.pruned.len())
                                        }
                                    ),
                                    Some(e) => println!("  {}: {e}", d.destination),
                                }
                            }
                        }
                        Err(e) => {
                            failed += 1;
                            eprintln!("{name}: {e}");
                        }
                    }
                }
                if failed > 0 {
                    bail!("{failed} backup(s) failed");
                }
            }
            BackupCommand::List { company } => {
                for (id, name) in companies(&state, company.as_deref())? {
                    let list = backups::list(&state, id).await;
                    println!("{name} ({id})");
                    if list.backups.is_empty() {
                        println!("  no backups");
                    }
                    for b in &list.backups {
                        let m = &b.manifest;
                        println!(
                            "  {}  {}  {:>9} bytes  {:?}  {}",
                            m.id, m.created_at, m.stored_size, m.encryption, b.destination_name
                        );
                    }
                    for e in &list.errors {
                        println!("  ! {e}");
                    }
                }
            }
            BackupCommand::Verify {
                id,
                company,
                unlock,
            } => {
                let (company, _) = locate(&state, &id, company.as_deref()).await?;
                let entry = backups::find(&state, company, &id, None).await?;
                let unlock = unlock.request()?.unlock(&state)?;
                let report = backups::verify(&state, &entry, &unlock).await?;
                if !report.contents_checked && report.checksum_ok {
                    println!(
                        "{id}: the stored file is intact; it is encrypted, so the database inside was not checked (pass --identity)"
                    );
                } else if report.ok() {
                    println!("{id}: ok");
                } else {
                    for p in &report.problems {
                        println!("{p}");
                    }
                    bail!("{id} has problems");
                }
            }
            BackupCommand::Restore {
                what,
                company,
                name,
                unlock,
            } => {
                let actor = Actor::system();
                let path = Path::new(&what);
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lbk")) && path.is_file()
                {
                    let (id, manifest) =
                        backups::import_lbk(&state, path.to_owned(), name, &actor)
                            .await
                            .map_err(api)?;
                    println!(
                        "imported \"{}\" as a new company ({id})",
                        manifest.company_name
                    );
                    return Ok(());
                }
                let (company, _) = locate(&state, &what, company.as_deref()).await?;
                let entry = backups::find(&state, company, &what, None).await?;
                let unlock = unlock.request()?.unlock(&state)?;
                let restored = backups::restore(&state, &entry, &unlock, name, &actor)
                    .await
                    .map_err(api)?;
                println!("restored as \"{}\" ({})", restored.name, restored.id);
            }
        }
        Ok(())
    })
}

/// The company holding backup `id`: the one given, or whichever has it.
async fn locate(state: &AppState, id: &str, company: Option<&str>) -> Result<(CompanyId, String)> {
    if let Some(c) = company {
        return find_company(state, c);
    }
    for (c, name) in companies(state, None)? {
        if backups::find(state, c, id, None).await.is_ok() {
            return Ok((c, name));
        }
    }
    bail!("no company has a backup {id:?}; pass --company <id> for a company that no longer exists")
}

pub fn update(data_dir: &Path, command: &UpdateCommand) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let (_, settings) = settings::load_or_create(data_dir)?;
    let options =
        ops::update::release_options(data_dir, &settings.updates).map_err(|e| anyhow!(e))?;
    let updater = Updater::new(options)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let status = updater.status();
        if let Some(advice) = &status.advice
            && matches!(command, UpdateCommand::Install)
        {
            bail!("{advice}");
        }
        let available = match updater.check().await? {
            CheckOutcome::UpToDate { latest } => {
                println!(
                    "LibreBooks {} is up to date{}",
                    status.current,
                    latest.map(|l| format!(" (newest release: {l})")).unwrap_or_default()
                );
                return Ok(());
            }
            CheckOutcome::Available(a) => a,
        };
        println!(
            "LibreBooks {} is available (this is {}): {}",
            available.version, status.current, available.page
        );
        match command {
            UpdateCommand::Check => {
                if let Some(advice) = &status.advice {
                    println!("{advice}");
                } else {
                    println!("install it with `librebooks update install`");
                }
            }
            UpdateCommand::Install => {
                let downloaded = updater.download(&available).await?;
                let installed = updater.install(&downloaded).await?;
                println!(
                    "installed {} at {} (the previous version is kept as {})",
                    installed.version,
                    installed.exe.display(),
                    installed.previous.display()
                );
                println!(
                    "restart LibreBooks to run it (systemctl restart librebooks, or restart the Windows service)"
                );
            }
        }
        Ok(())
    })
}

pub fn tunnel(data_dir: &Path, secret_key: Option<PathBuf>, command: TunnelCommand) -> Result<()> {
    let state = state(data_dir, secret_key)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        match command {
            TunnelCommand::Status => {
                let t = state.ops.settings().tunnel;
                println!("enabled:          {}", t.enabled);
                println!(
                    "public hostname:  {}",
                    t.public_hostname.as_deref().unwrap_or("(not set)")
                );
                let token = (!t.token_ref.is_empty())
                    .then(|| lb_tunnel::TokenSource::resolve(&state.ops.secrets, &t.token_ref));
                match token {
                    None => println!("token:            (not set)"),
                    Some(Err(e)) => println!("token:            {} ({e})", t.token_ref),
                    Some(Ok(tok)) => match TokenInfo::parse(&tok) {
                        Ok(info) => println!(
                            "token:            {} (tunnel {})",
                            t.token_ref, info.tunnel_id
                        ),
                        Err(e) => println!("token:            {} ({e})", t.token_ref),
                    },
                }
                let sources = lb_tunnel::Sources {
                    configured: t.cloudflared_path.as_deref(),
                    data_dir,
                    search_system: true,
                };
                match lb_tunnel::cloudflared::locate(&sources) {
                    Ok(Some(found)) => {
                        let version = lb_tunnel::cloudflared::version_of(&found.path)
                            .await
                            .unwrap_or_else(|e| format!("unknown: {e}"));
                        println!(
                            "cloudflared:      {} ({:?}, {version})",
                            found.path.display(),
                            found.origin
                        );
                    }
                    Ok(None) if t.auto_download => {
                        println!("cloudflared:      not installed; downloaded when the tunnel starts");
                    }
                    Ok(None) => println!("cloudflared:      not installed (auto_download is off)"),
                    Err(e) => println!("cloudflared:      {e}"),
                }
                for p in t.problems() {
                    println!("! {p}");
                }
                println!(
                    "The running server's live status: GET /api/v1/server/tunnel (Settings → Tunnel)."
                );
            }
            TunnelCommand::Setup { hostname } => {
                let token = if let Ok(t) = std::env::var("LIBREBOOKS_TUNNEL_TOKEN") {
                    t
                } else {
                    eprintln!("paste the tunnel token (or the whole `cloudflared service install …` command), then end input (Ctrl+D, or Ctrl+Z Enter on Windows):");
                    read_stdin()?
                };
                let token = token.trim();
                let token = token.rsplit(' ').next().unwrap_or(token).to_owned();
                let info = TokenInfo::parse(&token)?;
                state.ops.secrets.set(TOKEN_SECRET, &token)?;
                ops::edit_settings(&state, |s| {
                    s.tunnel.enabled = true;
                    s.tunnel.public_hostname = Some(hostname.trim().to_owned());
                    s.tunnel.token_ref = format!("{}{TOKEN_SECRET}", lb_server::secrets::SCHEME);
                })
                .await
                .map_err(api)?;
                println!(
                    "tunnel {} on for https://{}; restart `librebooks serve` to start it (and so passkeys work at that hostname)",
                    info.tunnel_id,
                    hostname.trim()
                );
            }
            TunnelCommand::Disable => {
                ops::edit_settings(&state, |s| s.tunnel.enabled = false)
                    .await
                    .map_err(api)?;
                println!("tunnel off; restart `librebooks serve` if it is running");
            }
        }
        Ok(())
    })
}

fn read_stdin() -> Result<String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    Ok(s.trim_end_matches(['\r', '\n']).to_owned())
}

pub fn secret(data_dir: &Path, secret_key: Option<PathBuf>, command: SecretCommand) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let store = lb_server::secrets::SecretStore::open(data_dir, secret_key);
    match command {
        SecretCommand::Set { name } => {
            let value = read_stdin()?;
            if value.is_empty() {
                bail!("nothing was read from standard input");
            }
            store.set(&name, &value)?;
            println!("saved; refer to it as {}{name}", lb_server::secrets::SCHEME);
        }
        SecretCommand::Delete { name } => {
            if !store.delete(&name)? {
                bail!("there is no secret named {name:?}");
            }
            println!("deleted {name}");
        }
        SecretCommand::List => {
            for s in store.list()? {
                println!("{}  (set {})", s.name, s.updated_at);
            }
            println!("key: {}", store.key_path().display());
        }
    }
    Ok(())
}

/// `serve`'s runtime: listen on every address, run the background work, stop
/// gracefully on Ctrl+C / SIGTERM or after an update was installed, then start
/// the new version (or exit for the supervisor) if one was.
pub fn run_server(
    config: &ServerConfig,
    binds: &[SocketAddr],
    secret_key: Option<PathBuf>,
) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    let restart = rt.block_on(async move {
        let state = lb_server::state_with(
            config,
            OpsOptions {
                secret_key,
                ..OpsOptions::default()
            },
        )?;
        let background = ops::start(&state);
        // Due memorized transactions and expired sign-in records, hourly.
        let housekeeping = lb_server::housekeeping::spawn(state.clone());
        let mut restart_rx = state.ops.restart_requested();
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            tokio::select! {
                () = shutdown_signal() => {}
                r = restart_rx.wait_for(Option::is_some) => {
                    if r.is_ok() {
                        tracing::info!("restarting into the new version");
                    }
                }
            }
            let _ = stop_tx.send(true);
        });
        let app = lb_server::router(state.clone());
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
            let mut stop = stop_rx.clone();
            tasks.push(tokio::spawn(async move {
                // Connect info gives sign-in rate limiting the client's address.
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .with_graceful_shutdown(async move {
                    let _ = stop.wait_for(|s| *s).await;
                })
                .await
            }));
        }
        if !lb_server::assets::has_ui() {
            tracing::warn!("this build has no web UI embedded; build web/ first (bun run build)");
        }
        tracing::info!("company files in {}", config.companies_dir().display());
        let mut served = Ok(());
        for t in tasks {
            if let Err(e) = t.await? {
                served = Err(e);
            }
        }
        housekeeping.abort();
        background.shutdown().await;
        served?;
        let restart: Option<Restart> = state.ops.restart_requested().borrow().clone();
        anyhow::Ok(restart)
    })?;
    // The runtime is gone (cloudflared stopped, files closed, ports free).
    drop(rt);
    if let Some(restart) = restart {
        if let Err(e) = restart.spawn() {
            bail!(
                "the update is installed but the new version didn't start ({e}); start LibreBooks again"
            );
        }
        std::process::exit(restart.exit_code());
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            () = async {
                match term.as_mut() {
                    Some(t) => { t.recv().await; }
                    None => std::future::pending().await,
                }
            } => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    tracing::info!("shutting down");
}
