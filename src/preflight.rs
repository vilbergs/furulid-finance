//! Sanity check against the real services, to run once before importing:
//!
//! ```sh
//! cargo test preflight -- --ignored --nocapture
//! ```
//!
//! Reads `jevcat.toml` and `.env` like the binary does, loads the budget from
//! actual-http-api (read-only), and sends a few transactions shaped like a Nordea export
//! through the import parser and the real Jev client. Nothing is written to Actual. Costs
//! one Jev call per sample row.

use std::path::Path;

use crate::actual::Snapshot;
use crate::classify::{Subject, Taxonomy};
use crate::config::{Config, Secrets};
use crate::{classifier, import, since_date, HttpStore};

/// Rows a model that is wired up correctly should not be unsure about.
const SAMPLE: &str = "\
Bokföringsdag;Belopp;Avsändare;Mottagare;Namn;Rubrik;Saldo;Valuta;
2026/10/08;-123,95;123456-7890;;;Kortköp 261007 ICA SUPERMARKET;1000,00;SEK;
2026/10/08;-111,00;123456-7890;;;Kortköp 261007 SUSHI BAR;1123,95;SEK;
2026/10/07;-76,00;123456-7890;;;Swish betalning VÄSTTRAFIK AB;1234,95;SEK;
2026/10/08;-514,00;123456-7890;;;Betalning BG 5014-1100 Telia Sverig;1111,00;SEK;
2026/09/14;-219,00;123456-7890;;;Kortköp 260914 NETFLIX.COM;2000,00;SEK;
";

#[tokio::test]
#[ignore = "calls the real actual-http-api and Jev; run before importing"]
async fn preflight() {
    dotenvy::dotenv().ok();
    let config = Config::load(Path::new("jevcat.toml")).expect("jevcat.toml");
    let secrets = Secrets::from_env().expect("secrets in .env or the environment");

    let store = HttpStore::new(
        &config.actual.base_url,
        &config.actual.budget_sync_id,
        &secrets.actual_api_key,
        secrets.actual_encryption_password.as_deref(),
    )
    .unwrap();
    let snapshot = Snapshot::load(&store, &since_date(config.actual.since_days))
        .await
        .expect("loading the budget from actual-http-api");
    let taxonomy = Taxonomy::from_groups(
        &snapshot.groups,
        &config.jev.descriptions,
        &config.jev.unsure_label,
    );
    let categories = taxonomy.options.len() - 1;
    assert!(categories > 0, "the budget has no visible categories");
    assert!(categories < 255, "Jev accepts at most 255 options");
    for label in config.jev.descriptions.keys() {
        assert!(
            taxonomy.option(label).is_some(),
            "[jev.descriptions] names {label:?}, which is not a visible category"
        );
    }

    let import_config = config.import.as_ref().expect("[import] section");
    if let Some(account) = &import_config.account {
        assert!(
            snapshot.account(account).is_some(),
            "import.account {account:?} is not an account in the budget"
        );
    }
    let rows = import::parse_csv(SAMPLE, import_config).expect("parsing the sample rows");

    let classifier = classifier(&config, &secrets).unwrap();
    let mut confident = 0;
    println!(
        "\n{categories} categories, apply_threshold {}",
        config.policy.apply_threshold
    );
    for row in &rows {
        let subject = Subject {
            payee: row.payee_name.clone(),
            imported_payee: row.imported_payee.clone(),
            amount: row.amount as f64 / 100.0,
            direction: if row.amount >= 0 { "inflow" } else { "outflow" },
            date: row.date.clone(),
            account: import_config.account.clone().unwrap_or_default(),
            notes: String::new(),
        };
        let decision = classifier
            .classify(&subject, &taxonomy)
            .await
            .expect("calling Jev");
        assert!(
            taxonomy.option(&decision.label).is_some(),
            "Jev answered {:?}, which is not one of the options",
            decision.label
        );
        assert!((0.0..=1.0).contains(&decision.confidence));
        if decision.label != config.jev.unsure_label {
            confident += 1;
        }
        println!(
            "  {:<22} → {:<28} {:.2}",
            subject.payee, decision.label, decision.confidence
        );
    }
    assert!(
        confident * 2 > rows.len(),
        "Jev was unsure about most obvious transactions; check [jev.descriptions] and the categories"
    );
}
