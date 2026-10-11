mod actual;
mod calibrate;
mod classify;
mod config;
mod filter;
mod gap;
mod import;
mod learn;
mod policy;
#[cfg(test)]
mod preflight;
mod review;
mod run;
mod tags;
mod text;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::actual::http::HttpStore;
use crate::actual::{
    Account, ActualStore, AddTransaction, ImportOptions, Snapshot, TransactionPatch,
};
use crate::classify::jev::JevClient;
use crate::classify::{Classifier, Subject, Taxonomy};
use crate::config::{Config, Secrets};
use crate::filter::Filter;
use crate::text::kr;

/// Categorize Actual Budget transactions with TypeSafe Jev.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Path to the TOML config file.
    #[arg(short, long, default_value = "jevcat.toml")]
    config: PathBuf,
    /// Look back this many days instead of `actual.since_days` from the config.
    #[arg(long, global = true)]
    since_days: Option<u32>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the transactions a run would classify, without calling Jev.
    List {
        /// Include transactions tagged as missed on an earlier run.
        #[arg(long)]
        rerun_missed: bool,
    },
    /// Replay categorized history through Jev and report accuracy per confidence.
    Calibrate {
        /// How many of the most recent categorized transactions to replay.
        #[arg(long, default_value_t = 200)]
        limit: usize,
        /// Precision the suggested threshold must reach.
        #[arg(long, default_value_t = 0.95)]
        target: f64,
    },
    /// Classify uncategorized transactions already in Actual. Dry run unless --apply is given.
    Run {
        /// Write categories and tags back to Actual.
        #[arg(long)]
        apply: bool,
        /// Retry transactions tagged as missed on an earlier run.
        #[arg(long)]
        rerun_missed: bool,
    },
    /// Create Actual rules for payees Jev has placed consistently, without calling Jev.
    Learn {
        /// Write the rules to Actual.
        #[arg(long)]
        apply: bool,
        /// How many agreeing applies a payee needs. Defaults to policy.learn_after.
        #[arg(long)]
        after: Option<usize>,
    },
    /// Import a bank CSV into an account, then categorize what the import added.
    ///
    /// Rows already in the account (same date, amount and bank text, whatever put them
    /// there) are skipped. The rest go in chunks, oldest first, and every row is checked
    /// afterwards; rows Actual merged into an older transaction are restored.
    Import {
        /// The CSV file the bank exported.
        csv: PathBuf,
        /// Actual account name or id. Overrides `import.account` in the config.
        #[arg(long)]
        account: Option<String>,
        /// Show what would be imported and how Jev would classify it, but write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Import only; leave the new rows uncategorized.
        #[arg(long)]
        no_categorize: bool,
        /// Rows per request to the import endpoint, which rejects large bodies.
        #[arg(long, default_value_t = 100)]
        chunk: usize,
    },
    /// Show uncategorized transactions with the bank text, currency and Jev's guess.
    Open {
        /// First date to include (YYYY-MM-DD). Defaults to the config's look-back.
        #[arg(long)]
        since: Option<String>,
        /// Last date to include.
        #[arg(long)]
        until: Option<String>,
        /// Show every transaction within --days of this date, categorized or not.
        #[arg(long)]
        around: Option<String>,
        #[arg(long, default_value_t = 3)]
        days: i64,
        /// Only rows whose payee or bank text contains this, across the whole history.
        #[arg(long)]
        payee: Option<String>,
        /// Include categorized rows.
        #[arg(long)]
        all: bool,
        /// Also write the rows as JSON to this file.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Actual account name or id. Defaults to `import.account` or the only account.
        #[arg(long)]
        account: Option<String>,
    },
    /// Set categories from a JSON plan, tagging each row #reviewed.
    ///
    /// The plan is a list of {"id", "category", "note"?, "replace_notes"?, "rule"?}.
    Set {
        plan: PathBuf,
        /// Print what would change without writing.
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        account: Option<String>,
    },
    /// Call actual-http-api directly: `jevcat api accounts`, `jevcat api -X PATCH
    /// transactions/<id> '{"transaction":{"notes":"x"}}'`.
    Api {
        /// HTTP method.
        #[arg(short = 'X', long, default_value = "GET")]
        method: String,
        /// Path under /v1/budgets/<sync id>/.
        path: String,
        /// Query string for GET (`since_date=2026-01-01`), JSON body otherwise.
        arg: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Secrets may sit in a gitignored .env next to jevcat.toml; real env vars win.
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let mut config = Config::load(&cli.config)?;
    if let Some(days) = cli.since_days {
        config.actual.since_days = days;
    }
    let secrets = Secrets::from_env()?;

    let store = HttpStore::new(
        &config.actual.base_url,
        &config.actual.budget_sync_id,
        &secrets.actual_api_key,
        secrets.actual_encryption_password.as_deref(),
    )?;

    // `api` needs no budget load; everything else starts from a snapshot.
    if let Command::Api { method, path, arg } = &cli.command {
        let (query, body) = if method.eq_ignore_ascii_case("GET") {
            (arg.as_deref(), None)
        } else {
            let body = arg
                .as_deref()
                .map(serde_json::from_str::<serde_json::Value>)
                .transpose()
                .context("the body must be JSON")?;
            (None, body)
        };
        let value = store.raw(method, path, query, body).await?;
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    let since = since_date(config.actual.since_days);
    let snapshot = Snapshot::load(&store, &since).await?;
    let taxonomy = Taxonomy::from_groups(
        &snapshot.groups,
        &config.jev.descriptions,
        &config.jev.unsure_label,
    );
    anyhow::ensure!(
        taxonomy.options.len() <= 255,
        "Jev accepts at most 255 options; this budget has {} visible categories",
        taxonomy.options.len() - 1
    );

    match cli.command {
        Command::Api { .. } => unreachable!("handled above"),
        Command::List { rerun_missed } => {
            let filter = Filter::new(&snapshot, &config.tags.missed, rerun_missed);
            let mut count = 0;
            for tx in snapshot
                .transactions
                .iter()
                .filter(|tx| filter.check(tx).is_ok())
            {
                let subject = Subject::from_transaction(tx, &snapshot);
                println!(
                    "{}  {:>10.2}  {:<24}  {}",
                    subject.date, subject.amount, subject.account, subject.payee
                );
                count += 1;
            }
            println!(
                "\n{count} eligible transactions, {} categories",
                taxonomy.options.len() - 1
            );
        }
        Command::Calibrate { limit, target } => {
            let classifier = classifier(&config, &secrets)?;
            let report = calibrate::calibrate(
                &snapshot,
                &taxonomy,
                classifier,
                &config.tags.missed,
                limit,
                config.jev.concurrency,
            )
            .await?;
            print!("{}", report.render(target));
        }
        Command::Run {
            apply,
            rerun_missed,
        } => {
            let classifier = classifier(&config, &secrets)?;
            let options = run::RunOptions {
                apply,
                rerun_missed,
                apply_threshold: config.policy.apply_threshold,
                concurrency: config.jev.concurrency,
                only_ids: None,
                learn_after: config.policy.learn_after,
            };
            if !apply {
                println!("Dry run: nothing is written. Pass --apply to write.\n");
            }
            let summary = run::run(
                &store as &dyn ActualStore,
                &snapshot,
                &taxonomy,
                classifier,
                &config.tags,
                &options,
            )
            .await?;
            print!("{}", summary.render(apply));
        }
        Command::Learn { apply, after } => {
            let learn_after = after.unwrap_or(config.policy.learn_after).max(1);
            if !apply {
                println!("Dry run: nothing is written. Pass --apply to write.\n");
            }
            let created = run::learn_rules(
                &store as &dyn ActualStore,
                &snapshot,
                &taxonomy,
                &[],
                &config.tags.applied,
                learn_after,
                apply,
            )
            .await?;
            let verb = if apply { "created" } else { "would create" };
            println!("\n{verb} {created} rules");
        }
        Command::Import {
            csv,
            account,
            dry_run,
            no_categorize,
            chunk,
        } => {
            import_csv(
                &store,
                &snapshot,
                &taxonomy,
                &config,
                &secrets,
                &since,
                &csv,
                account.as_deref(),
                dry_run,
                no_categorize,
                chunk.max(1),
            )
            .await?;
        }
        Command::Open {
            since: open_since,
            until,
            around,
            days,
            payee,
            all,
            json,
            account,
        } => {
            let account = pick_account(&snapshot, &config, account.as_deref())?;
            let (mut from, mut until, mut all) = (
                open_since.unwrap_or_else(|| since_date(config.actual.since_days.max(183))),
                until,
                all,
            );
            if let Some(date) = &around {
                let (lo, hi) = review::window_around(date, days)?;
                from = lo;
                until = Some(hi);
                all = true;
            }
            if payee.is_some() {
                from = "1970-01-01".to_string();
                all = true;
            }
            let transactions = store.transactions(&account.id, &from).await?;
            let report = review::open(
                &transactions,
                &snapshot.payee_names(),
                &snapshot.category_names(),
                &review::OpenOptions { until, all, payee },
            );
            print!("{}", report.table);
            if let Some(path) = json {
                std::fs::write(&path, serde_json::to_string_pretty(&report.json)?)?;
                println!("wrote {}", path.display());
            }
        }
        Command::Set {
            plan,
            dry_run,
            account,
        } => {
            let account = pick_account(&snapshot, &config, account.as_deref())?;
            let text = std::fs::read_to_string(&plan)
                .with_context(|| format!("reading {}", plan.display()))?;
            let items: Vec<review::PlanItem> =
                serde_json::from_str(&text).with_context(|| format!("parsing {}", plan.display()))?;
            if dry_run {
                println!("Dry run: nothing is written.\n");
            }
            let transactions = store.transactions(&account.id, "1970-01-01").await?;
            let (written, rules) = review::set(
                &store as &dyn ActualStore,
                &transactions,
                &snapshot.payee_names(),
                &snapshot.category_names(),
                &items,
                dry_run,
            )
            .await?;
            let verb = if dry_run { "would update" } else { "updated" };
            println!(
                "\n{verb} {} transactions, {rules} rules created",
                if dry_run { items.len() } else { written }
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn import_csv(
    store: &HttpStore,
    snapshot: &Snapshot,
    taxonomy: &Taxonomy,
    config: &Config,
    secrets: &Secrets,
    since: &str,
    csv: &std::path::Path,
    account: Option<&str>,
    dry_run: bool,
    no_categorize: bool,
    chunk: usize,
) -> Result<()> {
    let import_config = config
        .import
        .as_ref()
        .context("add an [import] section to the config to use `jevcat import`")?;
    let account_name = account
        .or(import_config.account.as_deref())
        .context("pass --account or set import.account in the config")?;
    let account = snapshot.account(account_name).with_context(|| {
        let names: Vec<&str> = snapshot.accounts.iter().map(|a| a.name.as_str()).collect();
        format!("no account {account_name:?}; accounts are {names:?}")
    })?;

    let rows = import::read_csv(csv, import_config)?;
    anyhow::ensure!(!rows.is_empty(), "{} holds no transactions", csv.display());
    let earliest = rows
        .iter()
        .map(|row| row.date.as_str())
        .min()
        .unwrap_or(since)
        .to_string();
    let latest = rows.iter().map(|row| row.date.as_str()).max().unwrap_or("");
    println!(
        "{} rows in {} for account {} ({earliest} to {latest})",
        rows.len(),
        csv.display(),
        account.name,
    );

    // Existing rows from a little before the file starts, so boundary overlaps and
    // fuzzy-match candidates are both visible.
    let lookback = shift_date(&earliest, -10);
    let existing = store.transactions(&account.id, &lookback).await?;
    let split = gap::split_existing(&rows, &existing);
    if !split.present.is_empty() {
        println!("\n{} rows already in the account, skipped:", split.present.len());
        for row in &split.present {
            println!("   {} {:>10} {}", row.date, kr(row.amount), row.imported_payee);
        }
    }
    let risks = gap::fuzzy_risks(&split.keep, &existing);
    if !risks.is_empty() {
        println!(
            "\n{} rows Actual may merge into an older transaction (checked and restored after the import):",
            risks.len()
        );
        for (row, tx) in &risks {
            println!(
                "   {} {:>10} {}  ~  {} {}",
                row.date,
                kr(row.amount),
                row.imported_payee,
                tx.date,
                text::bank_text(tx)
            );
        }
    }
    println!("\n{} rows to import", split.keep.len());
    if split.keep.is_empty() {
        return Ok(());
    }

    if dry_run {
        println!("Dry run: nothing is written.\n");
        if no_categorize {
            return Ok(());
        }
        let summary = run::preview(
            &split.keep,
            &account.name,
            taxonomy,
            classifier(config, secrets)?,
            config.policy.apply_threshold,
            config.jev.concurrency,
        )
        .await;
        print!("{}", summary.render(false));
        return Ok(());
    }

    let mut keep = split.keep.clone();
    keep.sort_by(|a, b| a.date.cmp(&b.date));
    let options = ImportOptions {
        dry_run: false,
        payee_normalization: import_config.payee_normalization.clone(),
    };
    let mut added: HashSet<String> = HashSet::new();
    let mut matched = 0;
    for (n, part) in keep.chunks(chunk).enumerate() {
        let result = store
            .import_transactions(&account.id, part, &options)
            .await
            .with_context(|| format!("chunk {} ({} rows from {})", n + 1, part.len(), part[0].date))?;
        println!(
            "chunk {}: {} rows ({} to {}): {} added, {} matched existing",
            n + 1,
            part.len(),
            part[0].date,
            part[part.len() - 1].date,
            result.added.len(),
            result.updated.len()
        );
        added.extend(result.added);
        matched += result.updated.len();
    }

    // Verify every CSV row is in the account exactly once; restore merged rows.
    let mut fresh = store.transactions(&account.id, &lookback).await?;
    let report = gap::verify(&rows, &fresh);
    for tx in &report.extra {
        println!(
            "   EXTRA {} {:>10} {} (in the account, not in the CSV)",
            tx.date,
            kr(tx.amount),
            text::bank_text(tx)
        );
    }
    if !report.missing.is_empty() {
        println!(
            "\n{} rows were merged into older transactions; restoring them:",
            report.missing.len()
        );
        let mut restored = 0;
        for row in &report.missing {
            let donor = report
                .merged
                .iter()
                .find(|tx| tx.imported_id.as_deref() == Some(row.imported_id.as_str()));
            if let Some(donor) = donor {
                store
                    .update_transaction(
                        &donor.id,
                        &TransactionPatch {
                            imported_id: Some(None),
                            ..Default::default()
                        },
                    )
                    .await?;
            }
            store
                .add_transaction(
                    &account.id,
                    &AddTransaction {
                        date: row.date.clone(),
                        amount: row.amount,
                        payee: donor.and_then(|d| d.payee.clone()),
                        category: donor.and_then(|d| d.category.clone()),
                        notes: row.notes.clone(),
                        imported_payee: row.imported_payee.clone(),
                        imported_id: row.imported_id.clone(),
                        cleared: true,
                    },
                )
                .await?;
            restored += 1;
            println!(
                "   restored {} {:>10} {}{}",
                row.date,
                kr(row.amount),
                row.imported_payee,
                donor
                    .map(|d| format!(" (payee and category copied from {})", d.date))
                    .unwrap_or_default()
            );
        }
        fresh = store.transactions(&account.id, &lookback).await?;
        let again = gap::verify(&rows, &fresh);
        anyhow::ensure!(
            again.missing.is_empty(),
            "{} rows are still missing after restoring {restored}",
            again.missing.len()
        );
        let restored_ids = fresh
            .iter()
            .filter(|tx| report.missing.iter().any(|row| tx.imported_id.as_deref() == Some(row.imported_id.as_str())))
            .filter(|tx| tx.category.is_none())
            .map(|tx| tx.id.clone());
        added.extend(restored_ids);
    }
    println!(
        "\nImport: {} added, {} matched existing, {} restored; all {} CSV rows present exactly once",
        added.len(),
        matched,
        report.missing.len(),
        rows.len()
    );
    if no_categorize || added.is_empty() {
        return Ok(());
    }

    info!(account = %account.name, since = %earliest, "categorizing imported transactions");
    let snapshot = Snapshot::new(
        snapshot.accounts.clone(),
        snapshot.groups.clone(),
        snapshot.payees.clone(),
        fresh,
    );
    let classifier = classifier(config, secrets)?;
    let options = run::RunOptions {
        apply: true,
        rerun_missed: false,
        apply_threshold: config.policy.apply_threshold,
        concurrency: config.jev.concurrency,
        only_ids: Some(added),
        learn_after: config.policy.learn_after,
    };
    let summary = run::run(
        store as &dyn ActualStore,
        &snapshot,
        taxonomy,
        classifier,
        &config.tags,
        &options,
    )
    .await?;
    print!("{}", summary.render(true));
    Ok(())
}

fn pick_account<'a>(
    snapshot: &'a Snapshot,
    config: &Config,
    name: Option<&str>,
) -> Result<&'a Account> {
    let name = name.or(config
        .import
        .as_ref()
        .and_then(|import| import.account.as_deref()));
    match name {
        Some(name) => snapshot.account(name).with_context(|| {
            let names: Vec<&str> = snapshot.accounts.iter().map(|a| a.name.as_str()).collect();
            format!("no account {name:?}; accounts are {names:?}")
        }),
        None => {
            let open: Vec<&Account> = snapshot.accounts.iter().filter(|a| !a.closed).collect();
            anyhow::ensure!(
                open.len() == 1,
                "pass --account; accounts are {:?}",
                open.iter().map(|a| a.name.as_str()).collect::<Vec<_>>()
            );
            Ok(open[0])
        }
    }
}

fn classifier(config: &Config, secrets: &Secrets) -> Result<Arc<dyn Classifier>> {
    Ok(Arc::new(JevClient::new(
        &config.jev.base_url,
        &secrets.typesafe_api_key,
        &config.jev.model,
        &config.jev.instructions,
        Duration::from_secs(config.jev.timeout_secs),
        config.jev.max_retries,
    )?))
}

fn since_date(days: u32) -> String {
    (chrono::Local::now().date_naive() - chrono::Duration::days(i64::from(days)))
        .format("%Y-%m-%d")
        .to_string()
}

fn shift_date(date: &str, days: i64) -> String {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| (d + chrono::Duration::days(days)).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| date.to_string())
}
