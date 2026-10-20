//! `librebooks import qbd`: read a company out of QuickBooks Desktop, read-only,
//! into a company file, through the QB Bridge or (on the QuickBooks machine)
//! the QuickBooks SDK; `import parity` to compare the two; and the Web
//! Connector's set-up pieces (`import qwc`, `import qbwc-password`), since a
//! Web Connector sync itself is started on the server (QuickBooks polls it).
//!
//! Transport settings saved on the company (by the UI, the API or `--save`)
//! are the defaults, so a routine `librebooks import qbd --company <id>` needs
//! nothing else.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use lb_core::{CompanyId, CompanyInfo};
use lb_qbd::client::{ListQuery, QbdSource, ReportQuery};
use lb_qbd::config::{ImportSettings, QbdTransport};
use lb_qbd::{
    BridgeClient, BridgeConfig, ParityOptions, QbdEntity, QbdError, SyncOptions, SyncProgress,
};
use lb_server::AppState;
use lb_server::import as server_import;
use serde_json::Value;

#[derive(Subcommand, Debug)]
pub enum ImportCommand {
    /// Copy a company out of QuickBooks Desktop (read-only), or bring an
    /// imported one up to date. Run it again any time: only what changed in
    /// QuickBooks is fetched.
    Qbd {
        #[command(flatten)]
        transport: TransportArgs,
        /// The company to import into (an id from `company list`). Without it, a
        /// new company is created.
        #[arg(long)]
        company: Option<CompanyId>,
        /// Save the transport settings (and bridge key) on the company, so the
        /// server, the UI and later runs use them.
        #[arg(long)]
        save: bool,
        /// List everything again rather than what changed since the last sync.
        #[arg(long)]
        full: bool,
        /// Also find what was deleted in QuickBooks and delete it here.
        #[arg(long)]
        sweep: bool,
        /// At most this many of each transaction type (a light trial run).
        #[arg(long)]
        limit: Option<usize>,
        /// Only transactions on or after this date.
        #[arg(long)]
        from: Option<jiff::civil::Date>,
        /// Only transactions on or before this date.
        #[arg(long)]
        to: Option<jiff::civil::Date>,
        /// Milliseconds to wait between bridge requests, to go easy on QuickBooks.
        #[arg(long)]
        delay_ms: Option<u64>,
        /// Compare balances with QuickBooks afterwards.
        #[arg(long)]
        parity: bool,
    },
    /// Compare an imported company's balances with QuickBooks' own.
    Parity {
        #[command(flatten)]
        transport: TransportArgs,
        #[arg(long)]
        company: CompanyId,
        /// The trial balance's date (default today).
        #[arg(long)]
        as_of: Option<jiff::civil::Date>,
    },
    /// Write the QuickBooks Web Connector file (.qwc) for a company, to add to
    /// the Web Connector on the QuickBooks machine.
    Qwc {
        #[arg(long)]
        company: CompanyId,
        /// LibreBooks' public URL as the Web Connector will reach it
        /// (https://…, or http://localhost); default: the saved app URL, else
        /// the server's public URL.
        #[arg(long)]
        url: Option<String>,
        /// Write to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Set the password the Web Connector signs in with (read from stdin), or
    /// generate one (--generate prints it once).
    QbwcPassword {
        #[arg(long)]
        company: CompanyId,
        #[arg(long)]
        generate: bool,
    },
}

/// How to reach QuickBooks. Anything not given comes from the company's saved
/// settings.
#[derive(Args, Debug)]
pub struct TransportArgs {
    /// bridge (the QB Bridge's REST API), sdk (the QuickBooks SDK on this
    /// Windows machine) or qbwc (the Web Connector; started on the server).
    #[arg(long)]
    transport: Option<QbdTransport>,
    /// The QB Bridge, e.g. http://10.255.0.18:8743 (an IPv4 literal: the bridge
    /// refuses public IPv6).
    #[arg(long, env = "QB_API_URL")]
    bridge_url: Option<String>,
    /// The bridge's API key. Better: leave it out and use --key-file.
    #[arg(long, env = "QB_API_KEY", hide_env_values = true)]
    key: Option<String>,
    /// A .env file holding QB_API_KEY=… (read without printing it).
    #[arg(long)]
    key_file: Option<PathBuf>,
    /// The .QBW path for the SDK (default: the company QuickBooks has open).
    #[arg(long)]
    company_file: Option<String>,
}

impl TransportArgs {
    /// The saved settings with these arguments laid over them.
    fn settings(&self, saved: ImportSettings) -> ImportSettings {
        let mut s = saved;
        if let Some(t) = self.transport {
            s.transport = t;
        } else if self.bridge_url.is_some() && s.transport != QbdTransport::Bridge {
            s.transport = QbdTransport::Bridge;
        }
        if let Some(u) = &self.bridge_url {
            s.bridge.url = u.clone();
        }
        if let Some(f) = &self.company_file {
            s.company_file = f.clone();
        }
        s
    }

    /// The bridge key from the arguments, if given here.
    fn given_key(&self) -> Result<Option<String>> {
        Ok(match (&self.key, &self.key_file) {
            (Some(k), _) => Some(k.clone()),
            (None, Some(f)) => Some(
                lb_qbd::client::read_env_value(f, "QB_API_KEY")
                    .with_context(|| format!("couldn't read {}", f.display()))?
                    .with_context(|| format!("{} has no QB_API_KEY", f.display()))?,
            ),
            (None, None) => None,
        })
    }
}

/// The source a run uses.
enum Source {
    Bridge(Box<BridgeClient>),
    #[cfg(all(windows, feature = "qbsdk"))]
    Sdk(lb_qbd::sdk::SdkSource),
}

impl QbdSource for Source {
    async fn list(&self, entity: QbdEntity, query: &ListQuery) -> Result<Vec<Value>, QbdError> {
        match self {
            Source::Bridge(s) => s.list(entity, query).await,
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.list(entity, query).await,
        }
    }

    async fn get(&self, entity: QbdEntity, id: &str) -> Result<Value, QbdError> {
        match self {
            Source::Bridge(s) => s.get(entity, id).await,
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.get(entity, id).await,
        }
    }

    async fn company(&self) -> Result<Value, QbdError> {
        match self {
            Source::Bridge(s) => s.company().await,
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.company().await,
        }
    }

    async fn preferences(&self) -> Result<Value, QbdError> {
        match self {
            Source::Bridge(s) => s.preferences().await,
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.preferences().await,
        }
    }

    async fn report(&self, slug: &str, query: &ReportQuery) -> Result<Value, QbdError> {
        match self {
            Source::Bridge(s) => s.report(slug, query).await,
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.report(slug, query).await,
        }
    }

    fn requests_made(&self) -> Option<u64> {
        match self {
            Source::Bridge(s) => s.requests_made(),
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.requests_made(),
        }
    }
}

/// Build the source for `settings`, with the bridge key from the arguments or
/// the secret store.
fn open_source(
    state: &AppState,
    company: Option<CompanyId>,
    settings: &ImportSettings,
    args: &TransportArgs,
    delay: Option<Duration>,
) -> Result<Source> {
    match settings.transport {
        QbdTransport::Bridge => {
            if settings.bridge.url.trim().is_empty() {
                bail!("give the bridge with --bridge-url (or save it on the company first)");
            }
            let key = if let Some(k) = args.given_key()? {
                k
            } else {
                let reference = settings
                    .bridge
                    .key_ref
                    .clone()
                    .or_else(|| {
                        company.map(|c| format!("secret:{}", server_import::bridge_key_name(c)))
                    })
                    .context("give the bridge's key with --key-file or QB_API_KEY")?;
                use age::secrecy::ExposeSecret as _;
                state
                    .ops
                    .secrets
                    .resolve(&reference)
                    .with_context(|| {
                        format!(
                            "the bridge key ({reference}) can't be read; give it with --key-file or QB_API_KEY"
                        )
                    })?
                    .expose_secret()
                    .to_owned()
            };
            let mut config = BridgeConfig::new(&settings.bridge.url, key)?;
            config.politeness_delay =
                delay.unwrap_or(Duration::from_millis(settings.bridge.delay_ms));
            Ok(Source::Bridge(Box::new(BridgeClient::new(config)?)))
        }
        QbdTransport::Sdk => {
            #[cfg(all(windows, feature = "qbsdk"))]
            {
                eprintln!(
                    "connecting to QuickBooks through the SDK (the first time, QuickBooks asks you to allow LibreBooks: choose \"Yes, always\")"
                );
                let config = lb_qbd::sdk::SdkConfig {
                    company_file: settings.company_file.clone(),
                    file_mode: settings.sdk.file_mode,
                    ..lb_qbd::sdk::SdkConfig::default()
                };
                let source = lb_qbd::sdk::sdk_source(&config)?;
                eprintln!("QuickBooks opened {}", source.transport().company_file());
                Ok(Source::Sdk(source))
            }
            #[cfg(not(all(windows, feature = "qbsdk")))]
            {
                bail!(
                    "the SDK transport needs LibreBooks on the Windows machine that runs QuickBooks (this build has no SDK support)"
                )
            }
        }
        QbdTransport::Qbwc => bail!(
            "a Web Connector sync is started on the running server (the UI, or POST /api/v1/c/<company>/import/qbd/sync), because QuickBooks' Web Connector polls the server; \
             this command can write the .qwc (`import qwc`) and set the password (`import qbwc-password`)"
        ),
    }
}

fn books_of(state: &AppState, id: CompanyId) -> Result<lb_store::Books> {
    state
        .companies
        .get(id)
        .map_err(|e| anyhow::anyhow!(e.message))
}

fn saved_settings(state: &AppState, company: Option<CompanyId>) -> Result<ImportSettings> {
    match company {
        Some(id) => server_import::settings_of(&books_of(state, id)?, id)
            .map_err(|e| anyhow::anyhow!(e.message)),
        None => Ok(ImportSettings::default()),
    }
}

pub fn run(data_dir: &Path, secret_key: Option<PathBuf>, command: ImportCommand) -> Result<()> {
    let state = crate::ops::state(data_dir, secret_key)?;
    let rt = tokio::runtime::Runtime::new()?;
    match command {
        ImportCommand::Qbd {
            transport,
            company,
            save,
            full,
            sweep,
            limit,
            from,
            to,
            delay_ms,
            parity,
        } => {
            let settings = transport.settings(saved_settings(&state, company)?);
            let delay = delay_ms.map(Duration::from_millis);
            let source = open_source(&state, company, &settings, &transport, delay)?;
            let id = if let Some(id) = company {
                id
            } else {
                let info = CompanyInfo {
                    name: "Imported from QuickBooks".into(),
                    ..CompanyInfo::default()
                };
                let id = state
                    .companies
                    .create(&info)
                    .map_err(|e| anyhow::anyhow!(e.message))?;
                eprintln!("created company {id}");
                id
            };
            let books = books_of(&state, id)?;
            if save || company.is_none() {
                let mut to_save = settings.clone();
                if let Some(key) = transport.given_key()? {
                    let name = server_import::bridge_key_name(id);
                    state.ops.secrets.set(&name, &key)?;
                    to_save.bridge.key_ref = Some(format!("secret:{name}"));
                }
                if let Some(d) = delay {
                    to_save.bridge.delay_ms = d.as_millis() as u64;
                }
                server_import::save_settings(&books, &to_save)
                    .map_err(|e| anyhow::anyhow!(e.message))?;
                eprintln!("saved the import settings on company {id}");
            }
            let options = SyncOptions {
                full,
                sweep,
                txn_limit: limit,
                txn_from: from,
                txn_to: to,
                ..SyncOptions::default()
            };
            let started = Instant::now();
            let last = Mutex::new((String::new(), Instant::now()));
            let progress = |p: &SyncProgress| {
                let mut l = last
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let label = format!("{:?} {:?}", p.entity, p.phase);
                // A line per entity and phase, then at most one every 5 seconds.
                if l.0 != label || l.1.elapsed() > Duration::from_secs(5) || p.done == p.total {
                    eprintln!("  {label}: {}/{}", p.done, p.total);
                    *l = (label, Instant::now());
                }
            };
            let report = rt.block_on(lb_qbd::sync(&books, &source, &options, &progress))?;
            let dir = data_dir.join(server_import::REPORTS_DIR);
            std::fs::create_dir_all(&dir)?;
            let stamp = jiff::Timestamp::now()
                .strftime("%Y%m%dT%H%M%SZ")
                .to_string();
            let path = dir.join(format!("{id}-{stamp}.json"));
            std::fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
            println!(
                "synced in {:.0?} over the {} transport",
                started.elapsed(),
                settings.transport.as_str()
            );
            for e in &report.entities {
                if e.listed + e.created + e.updated + e.failed > 0 {
                    println!(
                        "  {:<22} listed {:>6}  created {:>6}  updated {:>5}  unchanged {:>6}  failed {:>4}",
                        format!("{:?}", e.entity),
                        e.listed,
                        e.created,
                        e.updated,
                        e.unchanged,
                        e.failed
                    );
                }
            }
            println!(
                "  {} issues, {} mismatches, {} gaps; full report: {}",
                report.issues.len(),
                report.mismatches.len(),
                report.gaps.len(),
                path.display()
            );
            if let Some(why) = &report.aborted {
                println!("  stopped early: {why} (run again to carry on)");
            }
            if parity {
                print_parity(&rt, &books, &source, None)?;
            }
        }
        ImportCommand::Parity {
            transport,
            company,
            as_of,
        } => {
            let settings = transport.settings(saved_settings(&state, Some(company))?);
            let source = open_source(&state, Some(company), &settings, &transport, None)?;
            let books = books_of(&state, company)?;
            print_parity(&rt, &books, &source, as_of)?;
        }
        ImportCommand::Qwc { company, url, out } => {
            let books = books_of(&state, company)?;
            let mut settings = server_import::settings_of(&books, company)
                .map_err(|e| anyhow::anyhow!(e.message))?;
            if let Some(u) = url {
                settings.qbwc.app_url = Some(u);
            }
            let info = books.read(lb_store::company::info)?;
            let file = server_import::qwc_file(&state, company, &info.name, &settings, None)?;
            let xml = file.to_xml();
            match out {
                Some(path) => {
                    std::fs::write(&path, xml)?;
                    eprintln!(
                        "wrote {}; add it in the QuickBooks Web Connector on the QuickBooks machine (user {}); the password comes from `import qbwc-password`",
                        path.display(),
                        settings.qbwc.user_name
                    );
                }
                None => {
                    std::io::stdout().write_all(xml.as_bytes())?;
                }
            }
        }
        ImportCommand::QbwcPassword { company, generate } => {
            let _ = books_of(&state, company)?;
            let password = if generate {
                server_import::new_password()
            } else {
                if std::io::stdin().is_terminal() {
                    eprintln!(
                        "enter the Web Connector password, then Ctrl+Z/Ctrl+D (or pipe it in):"
                    );
                }
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                let s = s.trim().to_owned();
                if s.len() < 8 {
                    bail!("the Web Connector password needs at least 8 characters");
                }
                s
            };
            state
                .ops
                .secrets
                .set(&server_import::qbwc_password_name(company), &password)?;
            if generate {
                println!("{password}");
                eprintln!("(shown this once; type it into the Web Connector's password box)");
            } else {
                eprintln!("saved the Web Connector password for company {company}");
            }
        }
    }
    Ok(())
}

fn print_parity(
    rt: &tokio::runtime::Runtime,
    books: &lb_store::Books,
    source: &Source,
    as_of: Option<jiff::civil::Date>,
) -> Result<()> {
    let report = rt.block_on(lb_qbd::parity(
        books,
        source,
        &ParityOptions {
            as_of,
            trial_balance: true,
        },
    ))?;
    let s = &report.summary;
    println!(
        "parity: {}/{} accounts match QuickBooks; total difference {}; trial balance rows matching {}/{}",
        s.accounts_matching,
        s.accounts_compared,
        s.total_abs_difference,
        s.trial_balance_matching,
        s.trial_balance_rows
    );
    for row in report.accounts.iter().filter(|r| !r.difference.is_zero()) {
        println!(
            "  {:<48} QBD {:>14}  ours {:>14}  off {:>12}",
            row.full_name, row.qbd, row.ours, row.difference
        );
    }
    for n in &s.notes {
        println!("  note: {n}");
    }
    Ok(())
}
