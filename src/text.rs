//! Reading a transaction the way a person would: the bank's own text, the second
//! remittance line Nordea hides in notes, the original currency, and Jev's guess.

use std::sync::OnceLock;

use regex::Regex;

use crate::actual::Transaction;
use crate::tags;

/// Prefixes Nordea puts in front of the payee. Same list as `jevcat.example.toml`.
pub const DEFAULT_CLEANUP: [&str; 4] = [
    r"^Kortköp \d{6} ",
    r"^Swish (in)?betalning ",
    r"^Autogiro ",
    r"^(Betalning|Open Banking) BG [\d-]+ ",
];

fn cleanup() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        DEFAULT_CLEANUP
            .iter()
            .map(|p| Regex::new(p).expect("static regex"))
            .collect()
    })
}

/// `Kortköp 261008 ICA SUPERMARKET` → `ICA SUPERMARKET`.
pub fn clean_payee(text: &str) -> String {
    let mut out = text.to_string();
    for pattern in cleanup() {
        out = pattern.replace_all(&out, "").into_owned();
    }
    out.trim().to_string()
}

/// Notes without jevcat tags, the `[jev: …]` guess, and `#reviewed`.
pub fn strip_all_tags(notes: &str) -> String {
    tags::strip(notes, "#jev")
        .split_whitespace()
        .filter(|token| *token != "#reviewed")
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the bank wrote. Notes carry the fuller text (the second remittance line for
/// Bankgiro rows, the Swish message), so they win over `imported_payee`.
pub fn bank_text(tx: &Transaction) -> String {
    let notes = strip_all_tags(tx.notes.as_deref().unwrap_or(""));
    if !notes.is_empty() {
        return notes;
    }
    tx.imported_payee.clone().unwrap_or_default()
}

/// Bank transaction type and the original foreign amount, from bank sync's raw payload.
pub fn synced_extra(tx: &Transaction) -> (String, String) {
    let Some(raw) = tx.raw_synced_data.as_deref() else {
        return (String::new(), String::new());
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (String::new(), String::new());
    };
    let kind = value["bank_transaction_code"]["description"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let instructed = &value["exchange_rate"]["instructed_amount"];
    let fx = match instructed["currency"].as_str() {
        Some(currency) if currency != "SEK" => format!(
            "{currency} {}",
            instructed["amount"].as_str().unwrap_or("")
        ),
        _ => String::new(),
    };
    (kind, fx)
}

/// `Groceries 0.71` from a flag tag, `unsure` from a miss tag, else empty.
pub fn jev_guess(tx: &Transaction) -> String {
    static GUESS: OnceLock<Regex> = OnceLock::new();
    let notes = tx.notes.as_deref().unwrap_or("");
    let re = GUESS.get_or_init(|| Regex::new(r"\[jev: ([^\]]+)\]").expect("static regex"));
    if let Some(m) = re.captures(notes) {
        return m[1].to_string();
    }
    if tags::has_tag(notes, "#jev-miss") {
        return "unsure".to_string();
    }
    String::new()
}

pub fn kr(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.abs();
    let whole = abs / 100;
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{sign}{grouped}.{:02}", abs % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(notes: &str, imported_payee: Option<&str>, raw: Option<&str>) -> Transaction {
        serde_json::from_value(serde_json::json!({
            "id": "t1", "account": "a", "date": "2026-10-01", "amount": -100,
            "notes": notes, "imported_payee": imported_payee, "raw_synced_data": raw
        }))
        .unwrap()
    }

    #[test]
    fn bank_text_prefers_notes_without_tags() {
        let t = tx("000029407 Autogiro K*VR Resa #jev-miss", Some("000029407"), None);
        assert_eq!(bank_text(&t), "000029407 Autogiro K*VR Resa");
        let t = tx("", Some("Kortköp 260926 GG Varberg"), None);
        assert_eq!(bank_text(&t), "Kortköp 260926 GG Varberg");
        assert_eq!(strip_all_tags("Hay (goats) #reviewed"), "Hay (goats)");
    }

    #[test]
    fn synced_extra_reads_type_and_currency() {
        let raw = r#"{"bank_transaction_code":{"description":"Kortköp"},
            "exchange_rate":{"instructed_amount":{"currency":"EUR","amount":"-40.00"}}}"#;
        assert_eq!(
            synced_extra(&tx("", None, Some(raw))),
            ("Kortköp".into(), "EUR -40.00".into())
        );
        let sek = r#"{"bank_transaction_code":{"description":"BankGiro"},
            "exchange_rate":{"instructed_amount":{"currency":"SEK","amount":"-1.00"}}}"#;
        assert_eq!(
            synced_extra(&tx("", None, Some(sek))),
            ("BankGiro".into(), String::new())
        );
    }

    #[test]
    fn guesses_and_money() {
        assert_eq!(jev_guess(&tx("x #jev-flag [jev: Fuel 0.40]", None, None)), "Fuel 0.40");
        assert_eq!(jev_guess(&tx("x #jev-miss", None, None)), "unsure");
        assert_eq!(jev_guess(&tx("x #jev", None, None)), "");
        assert_eq!(kr(-195990), "-1,959.90");
        assert_eq!(kr(500), "5.00");
        assert_eq!(kr(3687902), "36,879.02");
        assert_eq!(clean_payee("Kortköp 260926 GG Varberg"), "GG Varberg");
    }
}
