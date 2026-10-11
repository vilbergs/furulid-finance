//! Matching CSV rows against what the account already holds, so an import never
//! duplicates a bank-synced or hand-entered row, and checking afterwards that every row
//! landed exactly once.
//!
//! Actual's own de-duplication only sees `imported_id`, which bank sync and manual
//! entries set differently or not at all. A row is "the same" here when the date and
//! amount agree and the bank text appears in the existing row's notes or imported payee.
//! Matching consumes the existing row, so two identical card purchases on one day need
//! two existing rows.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;

use crate::actual::{NewTransaction, Transaction};
use crate::text::strip_all_tags;

fn norm(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn same(row: &NewTransaction, tx: &Transaction) -> bool {
    if tx.date != row.date || tx.amount != row.amount {
        return false;
    }
    let text = norm(&row.imported_payee);
    norm(&strip_all_tags(tx.notes.as_deref().unwrap_or(""))).contains(&text)
        || norm(tx.imported_payee.as_deref().unwrap_or("")) == text
}

pub struct Split {
    pub keep: Vec<NewTransaction>,
    pub present: Vec<NewTransaction>,
}

/// Rows that are already in `existing` go to `present`; the rest to `keep`.
pub fn split_existing(rows: &[NewTransaction], existing: &[Transaction]) -> Split {
    let mut used: HashSet<&str> = HashSet::new();
    let mut split = Split {
        keep: Vec::new(),
        present: Vec::new(),
    };
    for row in rows {
        let hit = existing
            .iter()
            .find(|tx| !used.contains(tx.id.as_str()) && same(row, tx));
        match hit {
            Some(tx) => {
                used.insert(&tx.id);
                split.present.push(row.clone());
            }
            None => split.keep.push(row.clone()),
        }
    }
    split
}

fn days_between(a: &str, b: &str) -> Option<i64> {
    let a = NaiveDate::parse_from_str(a, "%Y-%m-%d").ok()?;
    let b = NaiveDate::parse_from_str(b, "%Y-%m-%d").ok()?;
    Some((a - b).num_days().abs())
}

/// Rows Actual's import may merge into an older row instead of adding: same amount
/// within seven days of an existing row that has no `imported_id`.
pub fn fuzzy_risks<'a>(
    keep: &'a [NewTransaction],
    existing: &'a [Transaction],
) -> Vec<(&'a NewTransaction, &'a Transaction)> {
    let mut risks = Vec::new();
    for row in keep {
        for tx in existing {
            if tx.amount == row.amount
                && tx.imported_id.is_none()
                && days_between(&tx.date, &row.date).is_some_and(|d| d <= 7)
            {
                risks.push((row, tx));
            }
        }
    }
    risks
}

pub struct Verify {
    /// Imported rows that are not in the account.
    pub missing: Vec<NewTransaction>,
    /// Older rows that took an imported row's `imported_id` through fuzzy matching.
    pub merged: Vec<Transaction>,
    /// Rows in the imported date window that no CSV row explains.
    pub extra: Vec<Transaction>,
}

/// Checks `rows` (the full CSV, including rows that were already present) against the
/// account after an import.
pub fn verify(rows: &[NewTransaction], existing: &[Transaction]) -> Verify {
    let mut used: HashSet<&str> = HashSet::new();
    let mut missing = Vec::new();
    for row in rows {
        match existing
            .iter()
            .find(|tx| !used.contains(tx.id.as_str()) && same(row, tx))
        {
            Some(tx) => {
                used.insert(&tx.id);
            }
            None => missing.push(row.clone()),
        }
    }
    let ids: HashMap<&str, &NewTransaction> = rows
        .iter()
        .map(|row| (row.imported_id.as_str(), row))
        .collect();
    let merged = existing
        .iter()
        .filter(|tx| {
            tx.imported_id
                .as_deref()
                .and_then(|id| ids.get(id))
                .is_some_and(|row| !same(row, tx))
        })
        .cloned()
        .collect();
    let lo = rows.iter().map(|r| r.date.as_str()).min().unwrap_or("");
    let hi = rows.iter().map(|r| r.date.as_str()).max().unwrap_or("");
    let extra = existing
        .iter()
        .filter(|tx| {
            tx.date.as_str() >= lo && tx.date.as_str() <= hi && !used.contains(tx.id.as_str())
        })
        .cloned()
        .collect();
    Verify {
        missing,
        merged,
        extra,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(date: &str, amount: i64, text: &str, id: &str) -> NewTransaction {
        NewTransaction {
            date: date.into(),
            amount,
            payee_name: text.into(),
            imported_payee: text.into(),
            notes: Some(text.into()),
            imported_id: id.into(),
        }
    }

    fn tx(id: &str, date: &str, amount: i64, notes: &str, imported_id: Option<&str>) -> Transaction {
        serde_json::from_value(serde_json::json!({
            "id": id, "account": "a", "date": date, "amount": amount,
            "notes": notes, "imported_id": imported_id
        }))
        .unwrap()
    }

    #[test]
    fn matches_on_text_in_notes_regardless_of_id_scheme() {
        let rows = vec![
            row("2025-11-21", -21500, "Kortköp 251121 Phtoshp Lightrm Bndl", "jevcat-1"),
            row("2025-11-22", -25189, "Kortköp 251121 CLAUDE.AI SUBSCRIPTI", "jevcat-2"),
            row("2025-11-26", -7400, "Swish betalning VÄSTTRAFIK AB", "jevcat-3"),
            row("2025-11-26", -7400, "Swish betalning VÄSTTRAFIK AB", "jevcat-4"),
        ];
        let existing = vec![
            tx("e1", "2025-11-21", -21500, "Kortköp 251121 Phtoshp Lightrm Bndl", None),
            tx("e2", "2025-11-26", -7400, "Swish betalning VÄSTTRAFIK AB #jev", Some("bank-x")),
        ];
        let split = split_existing(&rows, &existing);
        assert_eq!(split.present.len(), 2);
        assert_eq!(split.keep.len(), 2);
        assert_eq!(split.keep[0].imported_id, "jevcat-2");
        assert_eq!(split.keep[1].imported_id, "jevcat-4");
    }

    #[test]
    fn fuzzy_risks_need_same_amount_within_a_week_and_no_id() {
        let keep = vec![row("2025-11-25", -10900, "Swish betalning ALEX WANG", "jevcat-1")];
        let existing = vec![
            tx("e1", "2025-11-20", -10900, "Swish betalning ALEX WANG", None),
            tx("e2", "2025-11-20", -10900, "x", Some("id")),
            tx("e3", "2025-11-10", -10900, "x", None),
        ];
        let risks = fuzzy_risks(&keep, &existing);
        assert_eq!(risks.len(), 1);
        assert_eq!(risks[0].1.id, "e1");
    }

    #[test]
    fn verify_finds_merged_and_missing_rows() {
        let rows = vec![
            row("2025-11-25", -10900, "Swish betalning ALEX WANG", "jevcat-1"),
            row("2025-11-26", -9386, "Kortköp 251125 APOTEK", "jevcat-2"),
        ];
        let existing = vec![
            // The older row that swallowed jevcat-1.
            tx("e1", "2025-11-20", -10900, "Swish betalning ALEX WANG", Some("jevcat-1")),
            tx("e2", "2025-11-26", -9386, "Kortköp 251125 APOTEK #jev", Some("jevcat-2")),
            tx("e3", "2025-11-25", -5000, "something else", None),
        ];
        let report = verify(&rows, &existing);
        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].imported_id, "jevcat-1");
        assert_eq!(report.merged.len(), 1);
        assert_eq!(report.merged[0].id, "e1");
        assert_eq!(report.extra.len(), 1);
        assert_eq!(report.extra[0].id, "e3");
    }
}
