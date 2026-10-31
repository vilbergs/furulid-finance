//! `jevcat budget`: fill a month's budget from `#template` notes on the categories.
//!
//! Actual reads `#template 7000` in a category's note and fills the month when you click
//! "Apply budget template". This does the same over the API so the weekly sync can keep
//! the current and next month filled. Only categories still at zero are touched unless
//! `force` is set, so an amount edited by hand in Actual survives.

use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex::Regex;

use crate::actual::{ActualStore, CategoryGroup};

fn template() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?mi)^\s*#template\s+(-?[\d\s]+(?:[.,]\d{1,2})?)\s*$").expect("static regex"))
}

/// The simple `#template <amount>` line, in minor units. Other template forms are left
/// to Actual's own applier and return `None` here.
pub fn parse_template(note: &str) -> Option<i64> {
    let caps = template().captures(note)?;
    let raw: String = caps[1].chars().filter(|c| !c.is_whitespace()).collect();
    let raw = raw.replace(',', ".");
    let value: f64 = raw.parse().ok()?;
    Some((value * 100.0).round() as i64)
}

/// Replaces or adds the `#template` line in a note, keeping the rest.
pub fn with_template(note: &str, minor: i64) -> String {
    let line = format!("#template {}", minor / 100);
    let kept: Vec<&str> = note
        .lines()
        .filter(|l| !l.trim_start().to_lowercase().starts_with("#template"))
        .collect();
    let mut out = kept.join("\n").trim_end().to_string();
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&line);
    out
}

pub struct Template {
    pub category_id: String,
    pub name: String,
    pub amount: i64,
}

/// Reads every visible expense category's note and keeps the ones with a template.
pub async fn templates(store: &dyn ActualStore, groups: &[CategoryGroup]) -> Result<Vec<Template>> {
    let mut out = Vec::new();
    for group in groups.iter().filter(|g| !g.hidden && !g.is_income) {
        for cat in group.categories.iter().filter(|c| !c.hidden) {
            let note = store.category_note(&cat.id).await?;
            if let Some(amount) = parse_template(&note) {
                out.push(Template {
                    category_id: cat.id.clone(),
                    name: cat.name.clone(),
                    amount,
                });
            }
        }
    }
    Ok(out)
}

/// Fills `month` from the templates. Returns (set, left alone).
pub async fn apply(
    store: &dyn ActualStore,
    templates: &[Template],
    month: &str,
    force: bool,
    dry_run: bool,
) -> Result<(usize, usize)> {
    let current: HashMap<String, i64> = store
        .month_categories(month)
        .await?
        .into_iter()
        .map(|c| (c.id, c.budgeted))
        .collect();
    let mode = if dry_run { "dry-run" } else { "apply" };
    let (mut set, mut kept) = (0, 0);
    for t in templates {
        let now = current.get(&t.category_id).copied().unwrap_or(0);
        if now == t.amount || (now != 0 && !force) {
            kept += 1;
            continue;
        }
        println!(
            "{mode}  {month}  {:<24} {:>8} -> {:>8}",
            t.name,
            now / 100,
            t.amount / 100
        );
        if !dry_run {
            store.set_budgeted(month, &t.category_id, t.amount).await?;
        }
        set += 1;
    }
    Ok((set, kept))
}

/// Writes `#template` lines from `Name=kr` pairs.
pub async fn set_templates(
    store: &dyn ActualStore,
    groups: &[CategoryGroup],
    pairs: &[String],
    dry_run: bool,
) -> Result<usize> {
    let mut count = 0;
    for pair in pairs {
        let (name, amount) = pair
            .split_once('=')
            .with_context(|| format!("expected Name=kr, got {pair:?}"))?;
        let kr: f64 = amount.trim().replace(' ', "").parse().with_context(|| format!("bad amount in {pair:?}"))?;
        let cat = groups
            .iter()
            .flat_map(|g| g.categories.iter())
            .find(|c| c.name.eq_ignore_ascii_case(name.trim()))
            .with_context(|| format!("no category {name:?}"))?;
        let note = store.category_note(&cat.id).await?;
        let new_note = with_template(&note, (kr * 100.0).round() as i64);
        println!("{}  {:<24} {}", if dry_run { "dry-run" } else { "apply" }, cat.name, new_note.replace('\n', " | "));
        if !dry_run {
            store.set_category_note(&cat.id, &new_note).await?;
        }
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actual_style_templates() {
        assert_eq!(parse_template("#template 7000"), Some(700000));
        assert_eq!(parse_template("#template 9 500"), Some(950000));
        assert_eq!(parse_template("note\n#template 730.50\n"), Some(73050));
        assert_eq!(parse_template("#Template 100"), Some(10000));
        assert_eq!(parse_template("#template 500 repeat every 3 months"), None);
        assert_eq!(parse_template("nothing"), None);
    }

    #[test]
    fn replaces_template_line_only() {
        assert_eq!(with_template("", 700000), "#template 7000");
        assert_eq!(with_template("Three loans\n#template 6000", 700000), "Three loans\n#template 7000");
        assert_eq!(with_template("Three loans", 700000), "Three loans\n#template 7000");
    }
}
