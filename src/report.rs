//! `jevcat report`: the numbers an advice session starts from, straight from the budget.
//!
//! Four tables: spending per category per month, income per month, this month against
//! its budget, and recurring charges with the things worth a second look (a charge that
//! stopped, one that bills twice a month, two payees that look like the same service).

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use crate::actual::{MonthCategory, Transaction};
use crate::payees::canonical;

pub struct Input<'a> {
    /// Oldest first; the last one is the current, partial month.
    pub months: &'a [String],
    /// One entry per month, same order.
    pub per_month: &'a [Vec<MonthCategory>],
    pub transactions: &'a [Transaction],
    pub payee_names: &'a HashMap<String, String>,
}

/// Whole kronor with a thin-space thousands separator; negative shown without sign
/// because every table says which way the money went.
fn kr(minor: i64) -> String {
    let whole = (minor.abs() + 50) / 100;
    let digits = whole.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('\u{202f}');
        }
        out.push(c);
    }
    out
}

fn short(month: &str) -> &str {
    &month[2..]
}

pub fn render(input: &Input) -> String {
    let mut out = String::new();
    let n = input.months.len();
    if n == 0 {
        return "no months to report\n".to_string();
    }
    let full = n.saturating_sub(1); // months with complete data
    let current = &input.months[n - 1];

    // name -> per-month (budgeted, spent)
    let mut expense: BTreeMap<String, Vec<(i64, i64)>> = BTreeMap::new();
    let mut income: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (i, cats) in input.per_month.iter().enumerate() {
        for c in cats.iter().filter(|c| !c.hidden) {
            if c.is_income {
                income.entry(c.name.clone()).or_insert_with(|| vec![0; n])[i] = c.spent;
            } else {
                expense
                    .entry(c.name.clone())
                    .or_insert_with(|| vec![(0, 0); n])[i] = (c.budgeted, c.spent);
            }
        }
    }

    // 1. Spending
    let avg = |row: &[(i64, i64)]| -> i64 {
        if full == 0 {
            0
        } else {
            row[..full].iter().map(|(_, s)| -s).sum::<i64>() / full as i64
        }
    };
    let mut rows: Vec<(&String, &Vec<(i64, i64)>)> = expense
        .iter()
        .filter(|(_, row)| row.iter().any(|(b, s)| *b != 0 || *s != 0))
        .collect();
    rows.sort_by_key(|(_, row)| -avg(row));
    let _ = writeln!(out, "Spending per month (kr), average over the {full} full months, budget for {current}\n");
    let _ = write!(out, "{:<24}", "category");
    for m in input.months {
        let _ = write!(out, "{:>9}", short(m));
    }
    let _ = writeln!(out, "{:>9}{:>9}", "avg", "budget");
    let mut totals = vec![0i64; n];
    let mut total_budget = 0;
    for (name, row) in &rows {
        let _ = write!(out, "{:<24}", trunc(name, 24));
        for (i, (_, spent)) in row.iter().enumerate() {
            totals[i] += -spent;
            let _ = write!(out, "{:>9}", if *spent == 0 { "·".into() } else { kr(*spent) });
        }
        let budget = row[n - 1].0;
        total_budget += budget;
        let _ = writeln!(out, "{:>9}{:>9}", kr(avg(row)), if budget == 0 { "·".into() } else { kr(budget) });
    }
    let _ = write!(out, "{:<24}", "total");
    for t in &totals {
        let _ = write!(out, "{:>9}", kr(*t));
    }
    let total_avg = if full == 0 { 0 } else { totals[..full].iter().sum::<i64>() / full as i64 };
    let _ = writeln!(out, "{:>9}{:>9}\n", kr(total_avg), kr(total_budget));

    // 2. Income
    let _ = writeln!(out, "Income per month (kr)\n");
    let _ = write!(out, "{:<24}", "category");
    for m in input.months {
        let _ = write!(out, "{:>9}", short(m));
    }
    let _ = writeln!(out, "{:>9}", "avg");
    let mut in_totals = vec![0i64; n];
    for (name, row) in income.iter().filter(|(_, r)| r.iter().any(|v| *v != 0)) {
        let _ = write!(out, "{:<24}", trunc(name, 24));
        for (i, v) in row.iter().enumerate() {
            in_totals[i] += v;
            let _ = write!(out, "{:>9}", if *v == 0 { "·".into() } else { kr(*v) });
        }
        let a = if full == 0 { 0 } else { row[..full].iter().sum::<i64>() / full as i64 };
        let _ = writeln!(out, "{:>9}", kr(a));
    }
    let _ = write!(out, "{:<24}", "total");
    for t in &in_totals {
        let _ = write!(out, "{:>9}", kr(*t));
    }
    let in_avg = if full == 0 { 0 } else { in_totals[..full].iter().sum::<i64>() / full as i64 };
    let _ = writeln!(out, "{:>9}", kr(in_avg));
    let _ = writeln!(
        out,
        "\nAverage surplus over the full months: {} kr/month\n",
        signed(in_avg - total_avg)
    );

    // 3. This month against budget
    let _ = writeln!(out, "{current} against budget (kr)\n");
    let _ = writeln!(out, "{:<24}{:>9}{:>9}{:>9}", "category", "budget", "spent", "left");
    let mut over: Vec<(&String, i64, i64)> = Vec::new();
    for (name, row) in &rows {
        let (b, s) = row[n - 1];
        if b == 0 && s.abs() < 100 {
            continue;
        }
        over.push((name, b, -s));
    }
    over.sort_by_key(|(_, b, s)| b - s);
    for (name, b, s) in &over {
        let left = b - s;
        let mark = if left < 0 { "  over" } else { "" };
        let _ = writeln!(out, "{:<24}{:>9}{:>9}{:>9}{mark}", trunc(name, 24), kr(*b), kr(*s), signed(left));
    }
    let spent_now: i64 = over.iter().map(|(_, _, s)| s).sum();
    let _ = writeln!(out, "{:<24}{:>9}{:>9}{:>9}\n", "total", kr(total_budget), kr(spent_now), signed(total_budget - spent_now));

    // 4. Recurring charges
    let _ = writeln!(out, "Recurring charges (seen in 3+ months)\n");
    struct Stem {
        by_month: BTreeMap<String, Vec<i64>>,
    }
    let mut stems: BTreeMap<String, Stem> = BTreeMap::new();
    let first_month = &input.months[0];
    for tx in input.transactions.iter().filter(|t| t.amount < 0 && !t.is_parent) {
        let month = &tx.date[..7];
        if month < first_month.as_str() {
            continue;
        }
        let name = tx
            .payee
            .as_deref()
            .and_then(|id| input.payee_names.get(id))
            .map(String::as_str)
            .unwrap_or("");
        let mut stem = canonical(name);
        if stem.is_empty() || stem.chars().all(|c| c.is_ascii_digit()) {
            stem = canonical(tx.notes.as_deref().unwrap_or(""));
        }
        if stem.is_empty() {
            continue;
        }
        stems
            .entry(stem)
            .or_insert_with(|| Stem { by_month: BTreeMap::new() })
            .by_month
            .entry(month.to_string())
            .or_default()
            .push(-tx.amount);
    }
    let last_full = if full > 0 { Some(&input.months[full - 1]) } else { None };
    let recurring: Vec<(&String, &Stem)> = stems
        .iter()
        .filter(|(_, s)| s.by_month.len() >= 3)
        .collect();
    let first_words: Vec<(String, &String)> = recurring
        .iter()
        .filter_map(|(name, _)| {
            let w = name.split(|c: char| !c.is_alphanumeric()).next()?.to_lowercase();
            (w.len() >= 4).then_some((w, *name))
        })
        .collect();
    let mut lines: Vec<(i64, String)> = Vec::new();
    for (name, stem) in &recurring {
        let months_seen = stem.by_month.len();
        let total: i64 = stem.by_month.values().flatten().sum();
        let per_month = total / months_seen as i64;
        let last = stem.by_month.keys().last().unwrap();
        let mut flags: Vec<String> = Vec::new();
        if let Some(lf) = last_full {
            if last.as_str() < lf.as_str() {
                flags.push(format!("stopped after {}", short(last)));
            }
        }
        // Twice in one month at an amount that also recurs in other months: a
        // subscription billed on two lines, not two fuel stops.
        let doubled = stem.by_month.iter().any(|(month, v)| {
            v.iter().any(|a| {
                v.iter().filter(|b| *b == a).count() >= 2
                    && stem
                        .by_month
                        .iter()
                        .filter(|(m, w)| *m != month && w.contains(a))
                        .count()
                        >= 2
            })
        });
        if doubled {
            flags.push("same amount twice in a month".into());
        }
        if stem.by_month.keys().next().map(String::as_str) >= Some(input.months[n.saturating_sub(2)].as_str()) {
            flags.push("new".into());
        }
        let my_word = name.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("").to_lowercase();
        for (w, other) in &first_words {
            if *w == my_word && *other != *name {
                flags.push(format!("see also {other}"));
            }
        }
        lines.push((
            per_month,
            format!(
                "{:<32}{:>4}/{:<3}{:>9}  {:<6} {}",
                trunc(name, 32),
                months_seen,
                n,
                kr(per_month),
                short(last),
                flags.join("; ")
            ),
        ));
    }
    lines.sort_by_key(|(v, _)| -*v);
    let _ = writeln!(out, "{:<32}{:>8}{:>9}  {:<6} {}", "payee", "months", "kr/mo", "last", "notes");
    for (_, line) in &lines {
        let _ = writeln!(out, "{line}");
    }
    let recurring_total: i64 = lines.iter().map(|(v, _)| v).sum();
    let _ = writeln!(out, "\n{} recurring payees, about {} kr/month together", lines.len(), kr(recurring_total));
    out
}

fn signed(minor: i64) -> String {
    if minor < 0 {
        format!("-{}", kr(minor))
    } else {
        kr(minor)
    }
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

/// The last `n` months as `YYYY-MM`, oldest first, ending with the current month.
pub fn month_list(n: u32) -> Vec<String> {
    let today = chrono::Local::now().date_naive();
    let (mut y, mut m) = (today.format("%Y").to_string().parse::<i32>().unwrap(), today.format("%m").to_string().parse::<u32>().unwrap());
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(format!("{y:04}-{m:02}"));
        if m == 1 {
            m = 12;
            y -= 1;
        } else {
            m -= 1;
        }
    }
    out.reverse();
    out
}

pub fn next_month(month: &str) -> String {
    let y: i32 = month[..4].parse().unwrap_or(2000);
    let m: u32 = month[5..7].parse().unwrap_or(1);
    if m == 12 {
        format!("{:04}-01", y + 1)
    } else {
        format!("{y:04}-{:02}", m + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(name: &str, budgeted: i64, spent: i64, income: bool) -> MonthCategory {
        MonthCategory {
            id: name.to_lowercase(),
            name: name.into(),
            budgeted,
            spent,
            balance: budgeted + spent,
            is_income: income,
            hidden: false,
        }
    }

    fn tx(date: &str, amount: i64, payee: &str) -> Transaction {
        Transaction {
            id: format!("{date}{amount}{payee}"),
            account: "a".into(),
            date: date.into(),
            amount,
            payee: Some(payee.into()),
            imported_payee: None,
            category: None,
            notes: None,
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
    fn tables_add_up_and_flag_patterns() {
        let months = vec!["2026-06".to_string(), "2026-07".into(), "2026-08".into(), "2026-09".into(), "2026-10".into()];
        let per_month = vec![
            vec![cat("Groceries", 0, -1000000, false), cat("Income", 0, 4500000, true)],
            vec![cat("Groceries", 0, -900000, false), cat("Income", 0, 4500000, true)],
            vec![cat("Groceries", 0, -1100000, false), cat("Income", 0, 4500000, true)],
            vec![cat("Groceries", 0, -1000000, false), cat("Income", 0, 1000000, true)],
            vec![cat("Groceries", 950000, -200000, false), cat("Income", 0, 0, true)],
        ];
        let names: HashMap<String, String> = [
            ("n1", "Kortköp 260704 NETFLIX.COM"),
            ("f", "Fello"),
            ("v", "Verisure"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let txs = vec![
            tx("2026-07-04", -21900, "n1"),
            tx("2026-08-04", -21900, "n1"),
            tx("2026-09-04", -21900, "n1"),
            tx("2026-07-01", -18000, "f"),
            tx("2026-07-02", -18000, "f"),
            tx("2026-08-01", -18000, "f"),
            tx("2026-09-01", -18000, "f"),
            tx("2026-06-01", -66000, "v"),
            tx("2026-07-01", -66000, "v"),
            tx("2026-08-01", -66000, "v"),
        ];
        let text = render(&Input {
            months: &months,
            per_month: &per_month,
            transactions: &txs,
            payee_names: &names,
        });
        assert!(text.contains("Groceries"), "{text}");
        assert!(text.contains("10\u{202f}000"), "average groceries: {text}");
        assert!(text.contains("Average surplus over the full months: 26\u{202f}250"), "{text}");
        assert!(text.contains("same amount twice in a month"), "{text}");
        assert!(text.contains("stopped after 26-08"), "{text}");
        assert!(text.contains("Netflix.Com"), "{text}");
    }

    #[test]
    fn month_arithmetic() {
        assert_eq!(next_month("2026-12"), "2027-01");
        assert_eq!(next_month("2026-10"), "2026-11");
        let list = month_list(3);
        assert_eq!(list.len(), 3);
        assert_eq!(next_month(&list[1]), list[2]);
    }
}
