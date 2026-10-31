//! `jevcat payees`: give synced rows the same payee a person would type.
//!
//! Bank sync does not clean payee names, so every card purchase arrives as its own payee
//! (`Kortköp 260817 DISNEY PLUS`), and Bankgiro or Autogiro rows arrive under a nine-digit
//! reference number with the real description in notes. Two fixes:
//!
//! - A raw payee that cleans to one merchant is merged into the payee of that name,
//!   or simply renamed when no such payee exists. Rules move with the merge.
//! - A reference-number payee covers many merchants, so each of its rows is pointed at
//!   a payee derived from the row's notes instead. The reference payee itself is left.

use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;

use crate::actual::{ActualStore, Payee, Transaction, TransactionPatch};
use crate::text::{clean_payee, kr, strip_all_tags};

#[derive(Debug, Clone, PartialEq)]
pub struct Merge {
    pub from_id: String,
    pub from_name: String,
    pub to_name: String,
    /// `None` means rename `from_id` to `to_name`; `Some` means merge into that payee.
    pub to_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reassign {
    pub tx_id: String,
    pub date: String,
    pub amount: i64,
    pub from_name: String,
    pub to_name: String,
    /// `None` means the payee has to be created first.
    pub to_id: Option<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub merges: Vec<Merge>,
    pub reassigns: Vec<Reassign>,
}

fn reference() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\d{9}$").expect("static regex"))
}

fn leading_reference() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\d{9}\s+").expect("static regex"))
}

fn reservation() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^Reservation (Kortköp )?").expect("static regex"))
}

pub fn is_reference(name: &str) -> bool {
    reference().is_match(name.trim())
}

/// Text the bank shouts entirely in capitals gets Actual's title case (`Ica Supermarket`,
/// `Netflix.Com`); anything with lowercase in it is left alone, so `Teachiq AB` survives.
/// Tokens of one or two letters stay capitals (`AB`, `LF`, `K*`).
pub fn title_case(text: &str) -> String {
    let letters = text.chars().filter(|c| c.is_alphabetic());
    let shouted = letters.clone().count() >= 2 && letters.clone().all(|c| c.is_uppercase());
    if !shouted {
        return text.trim().to_string();
    }
    text.split_whitespace()
        .map(|word| {
            if word.chars().filter(|c| c.is_alphabetic()).count() <= 2 {
                return word.to_string();
            }
            let mut out = String::new();
            let mut start = true;
            for c in word.chars() {
                if c.is_alphabetic() {
                    if start {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    start = false;
                } else {
                    out.push(c);
                    start = true;
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn klarna() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^K\*\s+").expect("static regex"))
}

fn trailing_number() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s+\d{10,}\s*$").expect("static regex"))
}

/// The name a person would give this bank text.
pub fn canonical(text: &str) -> String {
    let text = leading_reference().replace(text.trim(), "");
    let text = reservation().replace(&text, "");
    let text = clean_payee(&text);
    // Klarna writes `K* Klarna` one month and `K*Klarna` the next.
    let text = klarna().replace(&text, "K*");
    let text = trailing_number().replace(&text, "");
    let mut words: Vec<&str> = Vec::new();
    for word in text.split_whitespace() {
        if words.last().map_or(true, |last| !last.eq_ignore_ascii_case(word)) {
            words.push(word);
        }
    }
    title_case(&words.join(" "))
}

/// Works out what to merge, rename and reassign. Pure, so it can be shown as a dry run.
pub fn plan(payees: &[Payee], transactions: &[Transaction]) -> Plan {
    let by_id: HashMap<&str, &Payee> = payees.iter().map(|p| (p.id.as_str(), p)).collect();
    // Lowercased name -> id, updated as renames are planned so later rows find them.
    let mut by_name: HashMap<String, String> = payees
        .iter()
        .map(|p| (p.name.trim().to_lowercase(), p.id.clone()))
        .collect();
    let mut plan = Plan::default();

    // Raw merchant payees, once each, in a stable order.
    let mut seen_payees: Vec<&str> = Vec::new();
    for tx in transactions {
        if let Some(id) = tx.payee.as_deref() {
            if !seen_payees.contains(&id) {
                seen_payees.push(id);
            }
        }
    }
    for id in &seen_payees {
        let Some(payee) = by_id.get(id) else { continue };
        let name = payee.name.trim();
        if name.is_empty() || is_reference(name) {
            continue;
        }
        let target = canonical(name);
        if target.is_empty() || target == name {
            continue;
        }
        let to_id = by_name.get(&target.to_lowercase()).cloned();
        if to_id.as_deref() == Some(id) {
            continue;
        }
        if to_id.is_none() {
            by_name.insert(target.to_lowercase(), id.to_string());
        }
        plan.merges.push(Merge {
            from_id: id.to_string(),
            from_name: name.to_string(),
            to_name: target,
            to_id,
        });
    }

    // Rows under a reference number: one payee per row, from the notes.
    for tx in transactions {
        let Some(id) = tx.payee.as_deref() else { continue };
        let Some(payee) = by_id.get(id) else { continue };
        if !is_reference(&payee.name) {
            continue;
        }
        let text = strip_all_tags(tx.notes.as_deref().unwrap_or(""));
        let target = canonical(&text);
        if target.is_empty() || is_reference(&target) {
            continue;
        }
        let to_id = by_name.get(&target.to_lowercase()).cloned();
        if to_id.is_none() {
            // Created on apply; later rows in this plan reuse it.
            by_name.insert(target.to_lowercase(), String::new());
        }
        plan.reassigns.push(Reassign {
            tx_id: tx.id.clone(),
            date: tx.date.clone(),
            amount: tx.amount,
            from_name: payee.name.clone(),
            to_name: target,
            to_id: to_id.filter(|id| !id.is_empty()),
        });
    }
    plan
}

/// Prints the plan and, unless `dry_run`, writes it. Returns (merges, reassigns) done.
pub async fn apply(store: &dyn ActualStore, plan: &Plan, dry_run: bool) -> Result<(usize, usize)> {
    let mode = if dry_run { "dry-run" } else { "apply" };
    for m in &plan.merges {
        let verb = if m.to_id.is_some() { "MERGE " } else { "RENAME" };
        println!("{mode}  {verb}  {:<36} -> {}", trunc(&m.from_name, 36), m.to_name);
        if dry_run {
            continue;
        }
        match &m.to_id {
            Some(target) => store.merge_payees(target, &[m.from_id.clone()]).await?,
            None => store.rename_payee(&m.from_id, &m.to_name).await?,
        }
    }
    let mut created: HashMap<String, String> = HashMap::new();
    for r in &plan.reassigns {
        println!(
            "{mode}  SET     {} {:>10}  {:<12} -> {}",
            r.date,
            kr(r.amount),
            r.from_name,
            r.to_name
        );
        if dry_run {
            continue;
        }
        let key = r.to_name.to_lowercase();
        let to_id = match r.to_id.clone().or_else(|| created.get(&key).cloned()) {
            Some(id) => id,
            None => {
                let id = store.create_payee(&r.to_name).await?;
                created.insert(key, id.clone());
                id
            }
        };
        store
            .update_transaction(
                &r.tx_id,
                &TransactionPatch {
                    payee: Some(to_id),
                    ..Default::default()
                },
            )
            .await?;
    }
    Ok((plan.merges.len(), plan.reassigns.len()))
}

fn trunc(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payee(id: &str, name: &str) -> Payee {
        Payee {
            id: id.into(),
            name: name.into(),
        }
    }

    fn tx(id: &str, payee: &str, notes: &str) -> Transaction {
        Transaction {
            id: id.into(),
            account: "a".into(),
            date: "2026-10-01".into(),
            amount: -1000,
            payee: Some(payee.into()),
            imported_payee: None,
            category: None,
            notes: Some(notes.into()),
            transfer_id: None,
            is_parent: false,
            is_child: false,
            starting_balance_flag: false,
            tombstone: false,
            raw_synced_data: None,
            imported_id: None,
        }
    }

    #[test]
    fn canonical_names() {
        assert_eq!(canonical("Kortköp 260817 DISNEY PLUS"), "Disney Plus");
        assert_eq!(canonical("000029407 Autogiro K* Klarna"), "K*Klarna");
        assert_eq!(canonical("000949892 BARNBDR BARNBDR 1994090395351383"), "Barnbdr");
        assert_eq!(canonical("Betalning PG 834704-9 Telia Sverige AB"), "Telia Sverige AB");
        assert_eq!(canonical("Betalning Pg 834704-9 Telia Sverige"), "Telia Sverige");
        assert_eq!(canonical("Autogiro K*KLARNA"), "K*Klarna");
        assert_eq!(canonical("Swish betalning ALEX WANG"), "Alex Wang");
        assert_eq!(canonical("Reservation Kortköp TYPESAFE AI, INC."), "Typesafe AI, Inc.");
        assert_eq!(canonical("Teachiq AB"), "Teachiq AB");
        assert_eq!(canonical("Omsättning lån 3992 50 55334"), "Omsättning lån 3992 50 55334");
        assert_eq!(title_case("ICA SUPERMARKET DERO"), "Ica Supermarket Dero");
        assert_eq!(title_case("MCDVARBERG/LASSABACKA"), "Mcdvarberg/Lassabacka");
    }

    #[test]
    fn raw_payees_merge_into_existing_or_rename() {
        let payees = vec![
            payee("p1", "Kortköp 260817 DISNEY PLUS"),
            payee("p2", "Disney Plus"),
            payee("p3", "Kortköp 260901 NETFLIX.COM"),
            payee("p4", "Kortköp 260902 NETFLIX.COM"),
            payee("p5", "Teachiq AB"),
        ];
        let txs = vec![
            tx("t1", "p1", ""),
            tx("t2", "p3", ""),
            tx("t3", "p4", ""),
            tx("t4", "p5", ""),
        ];
        let plan = plan(&payees, &txs);
        assert_eq!(plan.reassigns, vec![]);
        assert_eq!(plan.merges.len(), 3);
        assert_eq!(plan.merges[0].to_id.as_deref(), Some("p2"));
        // First Netflix payee is renamed, the second merged into it.
        assert_eq!(plan.merges[1].to_id, None);
        assert_eq!(plan.merges[1].to_name, "Netflix.Com");
        assert_eq!(plan.merges[2].to_id.as_deref(), Some("p3"));
    }

    #[test]
    fn reference_rows_get_a_payee_from_notes() {
        let payees = vec![payee("r1", "000029407"), payee("p1", "Avanza Bank")];
        let txs = vec![
            tx("t1", "r1", "000029407 Autogiro AVANZA BANK #jev"),
            tx("t2", "r1", "000029407 Autogiro K*Klarna"),
            tx("t3", "r1", "000029407 Autogiro K*Klarna"),
            tx("t4", "r1", "000029407"),
        ];
        let plan = plan(&payees, &txs);
        assert!(plan.merges.is_empty());
        assert_eq!(plan.reassigns.len(), 3);
        assert_eq!(plan.reassigns[0].to_id.as_deref(), Some("p1"));
        assert_eq!(plan.reassigns[1].to_name, "K*Klarna");
        assert_eq!(plan.reassigns[1].to_id, None);
        assert_eq!(plan.reassigns[2].to_id, None);
    }
}
