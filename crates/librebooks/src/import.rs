//! `librebooks import qbd`: read a company out of QuickBooks Desktop through the
//! QB Bridge, read-only, into a company file; and `librebooks import parity` to
//! compare the two.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use lb_core::{CompanyId, CompanyInfo};
use lb_qbd::{BridgeClient, BridgeConfig, ParityOptions, SyncOptions, SyncProgress};

#[derive(Subcommand, Debug)]
pub enum ImportCommand {
    /// Copy a company out of QuickBooks Desktop (read-only), or bring an
    /// imported one up to date. Run it again any time: only what changed in
    /// QuickBooks is fetched.
    Qbd {
        #[command(flatten)]
        bridge: BridgeArgs,
        /// The company to import into (an id from `company list`). Without it, a
        /// new company is created.
        #[arg(long)]
        company: Option<CompanyId>,
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
        /// Milliseconds to wait between requests, to go easy on QuickBooks.
        #[arg(long, default_value_t = 100)]
        delay_ms: u64,
        /// Compare balances with QuickBooks afterwards.
        #[arg(long)]
        parity: bool,
    },
    /// Compare an imported company's balances with QuickBooks' own.
    Parity {
        #[command(flatten)]
        bridge: BridgeArgs,
        #[arg(long)]
        company: CompanyId,
        /// The trial balance's date (default today).
        #[arg(long)]
        as_of: Option<jiff::civil::Date>,
    },
}

#[derive(Args, Debug)]
pub struct BridgeArgs {
    /// The QB Bridge, e.g. http://10.255.0.18:8743 (an IPv4 literal: the bridge
    /// refuses public IPv6).
    #[arg(long, env = "QB_API_URL")]
    bridge_url: String,
    /// The bridge's API key. Better: leave it out and use --key-file.
    #[arg(long, env = "QB_API_KEY", hide_env_values = true)]
    key: Option<String>,
    /// A .env file holding QB_API_KEY=… (read without printing it).
    #[arg(long)]
    key_file: Option<PathBuf>,
}

impl BridgeArgs {
    fn client(&self, delay: Duration) -> Result<BridgeClient> {
        let key = match (&self.key, &self.key_file) {
            (Some(k), _) => k.clone(),
            (None, Some(f)) => lb_qbd::client::read_env_value(f, "QB_API_KEY")
                .with_context(|| format!("couldn't read {}", f.display()))?
                .with_context(|| format!("{} has no QB_API_KEY", f.display()))?,
            (None, None) => bail!("give the bridge's key with --key-file or QB_API_KEY"),
        };
        let mut config = BridgeConfig::new(&self.bridge_url, key)?;
        config.politeness_delay = delay;
        Ok(BridgeClient::new(config)?)
    }
}

pub fn run(data_dir: &Path, command: ImportCommand) -> Result<()> {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let companies = lb_server::Companies::new(data_dir.join("companies"), events)?;
    let rt = tokio::runtime::Runtime::new()?;
    match command {
        ImportCommand::Qbd {
            bridge,
            company,
            full,
            sweep,
            limit,
            from,
            to,
            delay_ms,
            parity,
        } => {
            let client = bridge.client(Duration::from_millis(delay_ms))?;
            let id = if let Some(id) = company {
                id
            } else {
                let info = CompanyInfo {
                    name: "Imported from QuickBooks".into(),
                    ..CompanyInfo::default()
                };
                let id = companies
                    .create(&info)
                    .map_err(|e| anyhow::anyhow!(e.message))?;
                eprintln!("created company {id}");
                id
            };
            let books = companies.get(id).map_err(|e| anyhow::anyhow!(e.message))?;
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
            let report = rt.block_on(lb_qbd::sync(&books, &client, &options, &progress))?;
            let dir = data_dir.join("imports");
            std::fs::create_dir_all(&dir)?;
            let stamp = jiff::Timestamp::now()
                .strftime("%Y%m%dT%H%M%SZ")
                .to_string();
            let path = dir.join(format!("{id}-{stamp}.json"));
            std::fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
            println!("synced in {:.0?}", started.elapsed());
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
                print_parity(&rt, &books, &client, None)?;
            }
        }
        ImportCommand::Parity {
            bridge,
            company,
            as_of,
        } => {
            let client = bridge.client(Duration::from_millis(100))?;
            let books = companies
                .get(company)
                .map_err(|e| anyhow::anyhow!(e.message))?;
            print_parity(&rt, &books, &client, as_of)?;
        }
    }
    Ok(())
}

fn print_parity(
    rt: &tokio::runtime::Runtime,
    books: &lb_store::Books,
    client: &BridgeClient,
    as_of: Option<jiff::civil::Date>,
) -> Result<()> {
    let report = rt.block_on(lb_qbd::parity(
        books,
        client,
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
