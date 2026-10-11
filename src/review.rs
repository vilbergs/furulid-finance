//! `jevcat open`: what is still uncategorized, with everything needed to decide.
//! `jevcat set`: write categories from a plan file, tagging each row `#reviewed`.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::actual::{ActualStore, Rule, RuleClause, Transaction, TransactionPatch};
use crate::text::{bank_text, clean_payee, jev_guess, kr, strip_all_tags, synced_extra};

pub const REVIEWED_TAG: &str = "#reviewed";

pub struct OpenOptions {
    pub until: Option<String>,
    /// Show every row, categorized or not.
    pub all: bool,
    /// Substring on payee name or bank text, case-insensitive.
    pub payee: Option<String>,
}

pub struct OpenReport {
    pub table: String,
    pub json: serde_json::Value,
}

/// Renders the rows to look at as a markdown table plus a short summary.
pub fn open(
    transactions: &[Transaction],
    payees: &HashMap<String, String>,
    categories: &HashMap<String, String>,
    options: &OpenOptions,
) -> OpenReport {
    let mut rows: Vec<&Transaction> = transactions
        .iter()
        .filter(|tx| !tx.is_parent)
        .filter(|tx| options.until.as_deref().map_or(true, |u| tx.date.as_str() <= u))
        .filter(|tx| options.all || tx.category.is_none())
        .filter(|tx| match &options.payee {
            Some(q) => {
                let q = q.to_lowercase();
                payees
                    .get(tx.payee.as_deref().unwrap_or(""))
                    .is_some_and(|name| name.to_lowercase().contains(&q))
                    || bank_text(tx).to_lowercase().contains(&q)
            }
            None => true,
        })
        .collect();
    rows.sort_by(|a, b| a.date.cmp(&b.date));

    let payee_of = |tx: &Transaction| -> String {
        tx.payee
            .as_deref()
            .and_then(|id| payees.get(id))
            .map(|name| clean_payee(name))
            .unwrap_or_else(|| "-".to_string())
    };
    let last_header = if options.all { "category" } else { "jev guess" };
    let mut table = String::new();
    let _ = writeln!(
        table,
        "| id | date | amount | payee | bank text | type / fx | {last_header} |"
    );
    let _ = writeln!(table, "|---|---|---|---|---|---|---|");
    let mut json = Vec::new();
    for tx in &rows {
        let (kind, fx) = synced_extra(tx);
        let last = if options.all {
            tx.category
                .as_deref()
                .and_then(|id| categories.get(id))
                .cloned()
                .unwrap_or_default()
        } else {
            jev_guess(tx)
        };
        let text = bank_text(tx);
        let _ = writeln!(
            table,
            "| {} | {} | {} | {} | {} | {} | {} |",
            &tx.id[..8],
            tx.date,
            kr(tx.amount),
            payee_of(tx),
            text,
            format!("{kind} {fx}").trim(),
            last
        );
        json.push(serde_json::json!({
            "id": tx.id, "date": tx.date, "amount": tx.amount, "payee": payee_of(tx),
            "text": text, "type": kind, "fx": fx, "last": last,
        }));
    }
    let total: i64 = rows.iter().map(|tx| tx.amount).sum();
    let _ = writeln!(table, "\n{} rows, {} kr", rows.len(), kr(total));

    if !options.all {
        let spend: i64 = transactions.iter().filter(|t| t.amount < 0).map(|t| t.amount).sum();
        if spend != 0 {
            let _ = writeln!(
                table,
                "uncategorized share of spending in the window: {:.1}%",
                100.0 * total as f64 / spend as f64
            );
        }
        let mut by_payee: HashMap<String, usize> = HashMap::new();
        for tx in &rows {
            *by_payee.entry(payee_of(tx)).or_default() += 1;
        }
        let mut repeats: Vec<_> = by_payee.into_iter().filter(|(_, n)| *n > 1).collect();
        repeats.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        if !repeats.is_empty() {
            let _ = writeln!(
                table,
                "repeat payees: {}",
                repeats
                    .iter()
                    .map(|(p, n)| format!("{p} x{n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let mut incoming: Vec<&Transaction> = transactions
            .iter()
            .filter(|t| t.amount > 0 && t.amount < 2_000_000)
            .filter(|t| !bank_text(t).to_lowercase().contains("lön"))
            .collect();
        incoming.sort_by(|a, b| a.date.cmp(&b.date));
        if !incoming.is_empty() {
            let _ = writeln!(table, "\nincoming money in the window (reimbursements?):");
            for t in incoming {
                let _ = writeln!(table, "   {} {:>10} {}", t.date, kr(t.amount), bank_text(t));
            }
        }
    }
    OpenReport {
        table,
        json: serde_json::Value::Array(json),
    }
}

/// `--around DATE --days N` → (since, until).
pub fn window_around(date: &str, days: i64) -> Result<(String, String)> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").context("--around wants YYYY-MM-DD")?;
    Ok((
        (d - chrono::Duration::days(days)).format("%Y-%m-%d").to_string(),
        (d + chrono::Duration::days(days)).format("%Y-%m-%d").to_string(),
    ))
}

/// One line of a `jevcat set` plan.
#[derive(Debug, Clone, Deserialize)]
pub struct PlanItem {
    /// Full transaction id or its first eight characters.
    pub id: String,
    pub category: String,
    /// Appended in parentheses before the tag. The bank text stays.
    #[serde(default)]
    pub note: Option<String>,
    /// Replaces the notes entirely (plus the tag).
    #[serde(default)]
    pub replace_notes: Option<String>,
    /// Also create `payee is X → set category` unless a rule for the payee exists.
    #[serde(default)]
    pub rule: bool,
}

/// The notes a plan item produces for a transaction.
pub fn reviewed_notes(tx: &Transaction, item: &PlanItem) -> String {
    let mut notes = match &item.replace_notes {
        Some(text) => text.trim().to_string(),
        None => {
            let mut base = strip_all_tags(tx.notes.as_deref().unwrap_or(""));
            if let Some(note) = item.note.as_deref().filter(|n| !n.trim().is_empty()) {
                if !base.is_empty() {
                    base.push(' ');
                }
                base.push_str(&format!("({})", note.trim()));
            }
            base
        }
    };
    if !notes.is_empty() {
        notes.push(' ');
    }
    notes.push_str(REVIEWED_TAG);
    notes
}

/// Applies a plan. Returns how many transactions and rules were written.
pub async fn set(
    store: &dyn ActualStore,
    transactions: &[Transaction],
    payees: &HashMap<String, String>,
    categories: &HashMap<String, String>,
    plan: &[PlanItem],
    dry_run: bool,
) -> Result<(usize, usize)> {
    let category_id = |name: &str| -> Result<String> {
        categories
            .iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(name))
            .map(|(id, _)| id.clone())
            .with_context(|| {
                let mut names: Vec<&String> = categories.values().collect();
                names.sort();
                format!("no category {name:?}; categories are {names:?}")
            })
    };
    let mut ruled: HashSet<String> = store
        .rules()
        .await?
        .iter()
        .flat_map(|r| r.conditions.iter())
        .filter(|c| c.field == "payee")
        .filter_map(|c| c.value.as_str().map(str::to_string))
        .collect();

    let (mut written, mut rules) = (0, 0);
    for item in plan {
        let Some(tx) = transactions
            .iter()
            .find(|t| t.id == item.id || t.id.starts_with(&item.id))
        else {
            println!("SKIP {}: no such transaction", item.id);
            continue;
        };
        let cid = category_id(&item.category)?;
        let notes = reviewed_notes(tx, item);
        let payee_name = tx
            .payee
            .as_deref()
            .and_then(|id| payees.get(id))
            .map(String::as_str)
            .unwrap_or("-");
        println!(
            "{} {:>10} {:<28} -> {:<22} | {}",
            tx.date,
            kr(tx.amount),
            payee_name.chars().take(28).collect::<String>(),
            item.category,
            notes
        );
        if !dry_run {
            store
                .update_transaction(
                    &tx.id,
                    &TransactionPatch {
                        category: Some(cid.clone()),
                        notes: Some(notes),
                        ..Default::default()
                    },
                )
                .await?;
            written += 1;
        }
        if item.rule {
            match tx.payee.as_deref() {
                None => println!("   no payee on this row, rule skipped"),
                Some(pid) if ruled.contains(pid) => {
                    println!("   rule for {payee_name} already exists")
                }
                Some(pid) => {
                    if dry_run {
                        println!("   would create rule: {payee_name} -> {}", item.category);
                    } else {
                        store
                            .create_rule(&Rule {
                                id: None,
                                stage: None,
                                conditions_op: "and".into(),
                                conditions: vec![RuleClause {
                                    field: "payee".into(),
                                    op: "is".into(),
                                    value: pid.into(),
                                    kind: Some("id".into()),
                                }],
                                actions: vec![RuleClause {
                                    field: "category".into(),
                                    op: "set".into(),
                                    value: cid.clone().into(),
                                    kind: Some("id".into()),
                                }],
                            })
                            .await?;
                        ruled.insert(pid.to_string());
                        rules += 1;
                        println!("   rule created: {payee_name} -> {}", item.category);
                    }
                }
            }
        }
    }
    Ok((written, rules))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(id: &str, date: &str, amount: i64, notes: &str, category: Option<&str>) -> Transaction {
        serde_json::from_value(serde_json::json!({
            "id": id, "account": "a", "date": date, "amount": amount,
            "notes": notes, "category": category, "payee": "p1",
            "imported_payee": notes
        }))
        .unwrap()
    }

    #[test]
    fn reviewed_notes_strip_tags_and_add_reason() {
        let t = tx("t1", "2026-06-13", -60000, "Swish betalning CAROLINE #jev-miss", None);
        let item = PlanItem {
            id: "t1".into(),
            category: "Maintenance".into(),
            note: Some("second-hand tools".into()),
            replace_notes: None,
            rule: false,
        };
        assert_eq!(
            reviewed_notes(&t, &item),
            "Swish betalning CAROLINE (second-hand tools) #reviewed"
        );
        let replace = PlanItem {
            replace_notes: Some("Second hand tools from Alf".into()),
            ..item
        };
        assert_eq!(reviewed_notes(&t, &replace), "Second hand tools from Alf #reviewed");
    }

    #[test]
    fn open_lists_uncategorized_with_summary() {
        let transactions = vec![
            tx("aaaaaaaa-1", "2026-06-13", -60000, "Swish betalning CAROLINE #jev-miss", None),
            tx("bbbbbbbb-1", "2026-06-14", -10000, "Kortköp 260613 ICA #jev", Some("c1")),
            tx("cccccccc-1", "2026-06-15", 79600, "ELSTÖD", Some("c2")),
        ];
        let payees = HashMap::from([("p1".to_string(), "Caroline Kindal".to_string())]);
        let categories = HashMap::from([("c1".to_string(), "Groceries".to_string())]);
        let report = open(
            &transactions,
            &payees,
            &categories,
            &OpenOptions {
                until: None,
                all: false,
                payee: None,
            },
        );
        assert_eq!(report.table.matches("| 2026-").count(), 1);
        assert!(report.table.contains("| aaaaaaaa | 2026-06-13 | -600.00 | Caroline Kindal | Swish betalning CAROLINE |  | unsure |"));
        assert!(report.table.contains("1 rows, -600.00 kr"));
        assert!(report.table.contains("share of spending in the window: 85.7%"));
        assert!(report.table.contains("796.00 ELSTÖD"));
    }
}
