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

/// The per-category numbers both the tables and the JSON snapshot are built from.
struct Figures<'a> {
    n: usize,
    /// Months with complete data: all but the current one.
    full: usize,
    current: &'a str,
    /// Expense categories with any activity, biggest average first: per month (budgeted, spent).
    expense: Vec<(String, Vec<(i64, i64)>)>,
    /// Income categories with any activity: received per month.
    income: Vec<(String, Vec<i64>)>,
}

impl<'a> Figures<'a> {
    fn new(input: &'a Input) -> Self {
        let n = input.months.len();
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
        let mut figures = Figures {
            n,
            full: n.saturating_sub(1),
            current: &input.months[n - 1],
            expense: expense
                .into_iter()
                .filter(|(_, row)| row.iter().any(|(b, s)| *b != 0 || *s != 0))
                .collect(),
            income: income.into_iter().filter(|(_, r)| r.iter().any(|v| *v != 0)).collect(),
        };
        let mut expense = std::mem::take(&mut figures.expense);
        expense.sort_by_key(|(_, row)| -figures.avg_spent(row));
        figures.expense = expense;
        figures
    }

    /// Average over the full months of a per-month series.
    fn avg(&self, values: impl Iterator<Item = i64>) -> i64 {
        if self.full == 0 {
            0
        } else {
            values.take(self.full).sum::<i64>() / self.full as i64
        }
    }

    fn avg_spent(&self, row: &[(i64, i64)]) -> i64 {
        self.avg(row.iter().map(|(_, s)| -s))
    }

    /// Spending per month over all expense categories, positive.
    fn spent_totals(&self) -> Vec<i64> {
        let mut totals = vec![0i64; self.n];
        for (_, row) in &self.expense {
            for (i, (_, s)) in row.iter().enumerate() {
                totals[i] += -s;
            }
        }
        totals
    }

    fn income_totals(&self) -> Vec<i64> {
        let mut totals = vec![0i64; self.n];
        for (_, row) in &self.income {
            for (i, v) in row.iter().enumerate() {
                totals[i] += v;
            }
        }
        totals
    }

    fn budget_total(&self) -> i64 {
        self.expense.iter().map(|(_, row)| row[self.n - 1].0).sum()
    }
}

pub fn render(input: &Input) -> String {
    let mut out = String::new();
    let n = input.months.len();
    if n == 0 {
        return "no months to report\n".to_string();
    }
    let f = Figures::new(input);
    let (full, current) = (f.full, f.current);

    // 1. Spending
    let _ = writeln!(out, "Spending per month (kr), average over the {full} full months, budget for {current}\n");
    let _ = write!(out, "{:<24}", "category");
    for m in input.months {
        let _ = write!(out, "{:>9}", short(m));
    }
    let _ = writeln!(out, "{:>9}{:>9}", "avg", "budget");
    for (name, row) in &f.expense {
        let _ = write!(out, "{:<24}", trunc(name, 24));
        // Money into an expense category (a refund, a reserve moved back) shows negative.
        for (_, spent) in row {
            let _ = write!(out, "{:>9}", if *spent == 0 { "·".into() } else { signed(-*spent) });
        }
        let budget = row[n - 1].0;
        let _ = writeln!(out, "{:>9}{:>9}", signed(f.avg_spent(row)), if budget == 0 { "·".into() } else { kr(budget) });
    }
    let totals = f.spent_totals();
    let total_budget = f.budget_total();
    let _ = write!(out, "{:<24}", "total");
    for t in &totals {
        let _ = write!(out, "{:>9}", kr(*t));
    }
    let total_avg = f.avg(totals.iter().copied());
    let _ = writeln!(out, "{:>9}{:>9}\n", kr(total_avg), kr(total_budget));

    // 2. Income
    let _ = writeln!(out, "Income per month (kr)\n");
    let _ = write!(out, "{:<24}", "category");
    for m in input.months {
        let _ = write!(out, "{:>9}", short(m));
    }
    let _ = writeln!(out, "{:>9}", "avg");
    for (name, row) in &f.income {
        let _ = write!(out, "{:<24}", trunc(name, 24));
        for v in row {
            let _ = write!(out, "{:>9}", if *v == 0 { "·".into() } else { kr(*v) });
        }
        let _ = writeln!(out, "{:>9}", kr(f.avg(row.iter().copied())));
    }
    let in_totals = f.income_totals();
    let _ = write!(out, "{:<24}", "total");
    for t in &in_totals {
        let _ = write!(out, "{:>9}", kr(*t));
    }
    let in_avg = f.avg(in_totals.iter().copied());
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
    for (name, row) in &f.expense {
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
    let recurring = recurring(input);
    let _ = writeln!(out, "{:<32}{:>8}{:>9}  {:<6} {}", "payee", "months", "kr/mo", "last", "notes");
    for r in &recurring {
        let _ = writeln!(
            out,
            "{:<32}{:>4}/{:<3}{:>9}  {:<6} {}",
            trunc(&r.name, 32),
            r.months_seen,
            n,
            kr(r.per_month),
            short(&r.last),
            r.flags.join("; ")
        );
    }
    let recurring_total: i64 = recurring.iter().map(|r| r.per_month).sum();
    let _ = writeln!(out, "\n{} recurring payees, about {} kr/month together", recurring.len(), kr(recurring_total));
    out
}

struct Recurring {
    name: String,
    months_seen: usize,
    /// Average per month it was seen, positive.
    per_month: i64,
    last: String,
    flags: Vec<String>,
}

/// Payees charged in 3+ of the months, biggest first, with the things worth a second
/// look flagged.
fn recurring(input: &Input) -> Vec<Recurring> {
    let n = input.months.len();
    let full = n.saturating_sub(1);
    let mut stems: BTreeMap<String, BTreeMap<String, Vec<i64>>> = BTreeMap::new();
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
            .or_default()
            .entry(month.to_string())
            .or_default()
            .push(-tx.amount);
    }
    let last_full = if full > 0 { Some(&input.months[full - 1]) } else { None };
    let recurring: Vec<(&String, &BTreeMap<String, Vec<i64>>)> = stems
        .iter()
        .filter(|(_, by_month)| by_month.len() >= 3)
        .collect();
    let first_words: Vec<(String, &String)> = recurring
        .iter()
        .filter_map(|(name, _)| {
            let w = name.split(|c: char| !c.is_alphanumeric()).next()?.to_lowercase();
            (w.len() >= 4).then_some((w, *name))
        })
        .collect();
    let mut out: Vec<Recurring> = Vec::new();
    for (name, by_month) in &recurring {
        let months_seen = by_month.len();
        let total: i64 = by_month.values().flatten().sum();
        let last = by_month.keys().last().unwrap();
        let mut flags: Vec<String> = Vec::new();
        if let Some(lf) = last_full {
            if last.as_str() < lf.as_str() {
                flags.push(format!("stopped after {}", short(last)));
            }
        }
        // Twice in one month at an amount that also recurs in other months: a
        // subscription billed on two lines, not two fuel stops.
        let doubled = by_month.iter().any(|(month, v)| {
            v.iter().any(|a| {
                v.iter().filter(|b| *b == a).count() >= 2
                    && by_month
                        .iter()
                        .filter(|(m, w)| *m != month && w.contains(a))
                        .count()
                        >= 2
            })
        });
        if doubled {
            flags.push("same amount twice in a month".into());
        }
        if by_month.keys().next().map(String::as_str) >= Some(input.months[n.saturating_sub(2)].as_str()) {
            flags.push("new".into());
        }
        let my_word = name.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("").to_lowercase();
        for (w, other) in &first_words {
            if *w == my_word && *other != *name {
                flags.push(format!("see also {other}"));
            }
        }
        out.push(Recurring {
            name: (*name).clone(),
            months_seen,
            per_month: total / months_seen as i64,
            last: last.clone(),
            flags,
        });
    }
    out.sort_by_key(|r| -r.per_month);
    out
}

/// Whole kronor, rounded, for the JSON snapshot.
fn whole(minor: i64) -> i64 {
    (minor + if minor < 0 { -50 } else { 50 }) / 100
}

/// The report as one JSON document in whole kronor, for tools that size decisions to the
/// household's cash flow (the trader's verdicts and plan). `balance` is the account's
/// balance as Actual has it, in öre.
pub fn snapshot(input: &Input, account: &str, balance: Option<i64>) -> serde_json::Value {
    use serde_json::json;
    let n = input.months.len();
    if n == 0 {
        return json!({ "error": "no months to report" });
    }
    let f = Figures::new(input);
    let spent = f.spent_totals();
    let received = f.income_totals();
    let spent_avg = f.avg(spent.iter().copied());
    let income_avg = f.avg(received.iter().copied());
    // Money moved to savings is not consumption: what the household could put aside
    // is the surplus plus what already goes to the savings categories.
    let is_saving = |name: &str| name.to_lowercase().contains("saving");
    let mut savings = vec![0i64; n];
    for (_, row) in f.expense.iter().filter(|(name, _)| is_saving(name)) {
        for (i, (_, s)) in row.iter().enumerate() {
            savings[i] += -s;
        }
    }
    let savings_avg = f.avg(savings.iter().copied());

    // Uncategorized spending over the full months, against all spending in them.
    let first = &input.months[0];
    let current = f.current;
    let open: i64 = input
        .transactions
        .iter()
        .filter(|t| {
            t.amount < 0
                && t.category.is_none()
                && t.transfer_id.is_none()
                && !t.is_parent
                && !t.starting_balance_flag
                && t.date.as_str() >= first.as_str()
                && &t.date[..7] < current
        })
        .map(|t| -t.amount)
        .sum();
    let spent_full: i64 = spent[..f.full].iter().sum();
    let open_share = if open + spent_full > 0 {
        (open as f64 / (open + spent_full) as f64 * 1000.0).round() / 10.0
    } else {
        0.0
    };

    let months = |values: &[i64]| -> serde_json::Map<String, serde_json::Value> {
        input.months.iter().zip(values).map(|(m, v)| (m.clone(), json!(whole(*v)))).collect()
    };
    let recurring = recurring(input);
    json!({
        "as_of": chrono::Local::now().format("%Y-%m-%d").to_string(),
        "account": account,
        "currency": "SEK",
        "months": input.months,
        "full_months": f.full,
        "note": "Whole kronor. Averages are over the full months; the last month is partial.",
        "income": {
            "avg_per_month": whole(income_avg),
            "per_month": months(&received),
            "by_category_avg": f.income.iter()
                .map(|(name, row)| (name.clone(), json!(whole(f.avg(row.iter().copied())))))
                .collect::<serde_json::Map<_, _>>(),
        },
        "spending": {
            "avg_per_month": whole(spent_avg),
            "per_month": months(&spent),
            "budget_this_month": whole(f.budget_total()),
            "by_category": f.expense.iter().map(|(name, row)| json!({
                "category": name,
                "avg": whole(f.avg_spent(row)),
                "budget": whole(row[n - 1].0),
            })).collect::<Vec<_>>(),
        },
        "net_to_savings": {
            "note": "Booked to the savings categories; negative means money came back out of savings into the account.",
            "avg_per_month": whole(savings_avg),
            "per_month": months(&savings),
            "categories": f.expense.iter().filter(|(name, _)| is_saving(name)).map(|(name, _)| name).collect::<Vec<_>>(),
        },
        "surplus_avg_per_month": whole(income_avg - spent_avg),
        "surplus_before_savings_avg_per_month": whole(income_avg - spent_avg + savings_avg),
        "uncategorized": {
            "kr": whole(open),
            "share_of_spending_pct": open_share,
        },
        "recurring": {
            "kr_per_month": whole(recurring.iter().map(|r| r.per_month).sum()),
            "payees": recurring.iter().map(|r| json!({
                "payee": r.name,
                "kr_per_month": whole(r.per_month),
                "months_seen": r.months_seen,
                "last": r.last,
                "flags": r.flags,
            })).collect::<Vec<_>>(),
        },
        "account_balance": balance.map(whole),
    })
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
    fn snapshot_counts_savings_and_open_rows() {
        let months = vec!["2026-08".to_string(), "2026-09".into(), "2026-10".into()];
        let per_month = vec![
            vec![cat("Groceries", 0, -1000000, false), cat("Savings", 0, -500000, false), cat("Income", 0, 4000000, true)],
            vec![cat("Groceries", 0, -1200000, false), cat("Savings", 0, -500000, false), cat("Income", 0, 3000000, true)],
            vec![cat("Groceries", 900000, -100000, false), cat("Savings", 500000, 0, false), cat("Income", 0, 0, true)],
        ];
        let mut open = tx("2026-09-12", -300000, "x");
        open.category = None;
        let mut transfer = tx("2026-09-13", -900000, "x");
        transfer.transfer_id = Some("t".into());
        let mut current = tx("2026-10-02", -50000, "x");
        current.category = None;
        let names = HashMap::new();
        let v = snapshot(
            &Input { months: &months, per_month: &per_month, transactions: &[open, transfer, current], payee_names: &names },
            "Nordea",
            Some(1905050),
        );
        assert_eq!(v["income"]["avg_per_month"], 35000);
        assert_eq!(v["spending"]["avg_per_month"], 16000);
        assert_eq!(v["net_to_savings"]["avg_per_month"], 5000);
        assert_eq!(v["surplus_avg_per_month"], 19000);
        assert_eq!(v["surplus_before_savings_avg_per_month"], 24000);
        assert_eq!(v["spending"]["budget_this_month"], 14000);
        // 3 000 open against 32 000 categorized over the two full months
        assert_eq!(v["uncategorized"]["kr"], 3000);
        assert_eq!(v["uncategorized"]["share_of_spending_pct"], 8.6);
        assert_eq!(v["account_balance"], 19051);
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
