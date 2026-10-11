//! Drives the real binary against mock actual-http-api and Jev servers.

use std::collections::HashMap;
use std::process::Command;

use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const SYNC: &str = "b1";

fn jev_response(choice: &str, confidence: f64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "model": "jev-1.13.0",
        "answers": { "category": {
            "type": "choice", "choice": choice, "confidence": confidence,
            "probabilities": { choice: confidence, "Unsure": 1.0 - confidence }
        }},
        "usage": { "input_tokens": 300, "output_tokens": 0 }
    }))
}

async fn mock_budget(actual: &MockServer, transactions: Value) {
    let base = format!("/v1/budgets/{SYNC}");
    Mock::given(method("GET"))
        .and(path(format!("{base}/accounts")))
        .and(header("x-api-key", "k"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [
            { "id": "a1", "name": "Checking", "offbudget": false, "closed": false },
            { "id": "a2", "name": "Pension", "offbudget": true, "closed": false }
        ]})))
        .mount(actual)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{base}/categorygroups")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [
            { "id": "g1", "name": "Everyday", "is_income": false, "hidden": false, "categories": [
                { "id": "c1", "name": "Groceries", "group_id": "g1", "is_income": false, "hidden": false },
                { "id": "c2", "name": "Restaurants", "group_id": "g1", "is_income": false, "hidden": false }
            ]}
        ]})))
        .mount(actual)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{base}/payees")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [
            { "id": "p1", "name": "ICA" }, { "id": "p2", "name": "Espresso House" }
        ]})))
        .mount(actual)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{base}/accounts/a1/transactions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": transactions })))
        .mount(actual)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{base}/rules")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .with_priority(10)
        .mount(actual)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{base}/accounts/a2/transactions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(actual)
        .await;
}

fn write_config(
    dir: &std::path::Path,
    actual: &MockServer,
    jev: &MockServer,
) -> std::path::PathBuf {
    let path = dir.join("jevcat.toml");
    std::fs::write(
        &path,
        format!(
            r#"
[actual]
base_url = "{}"
budget_sync_id = "{SYNC}"
since_days = 30

[jev]
base_url = "{}"
concurrency = 2
max_retries = 1

[jev.descriptions]
Groceries = "Supermarkets"

[policy]
apply_threshold = 0.85
learn_after = 1

[import]
account = "Checking"
delimiter = ";"
date_column = "Datum"
amount_column = "Belopp"
payee_column = "Text"
"#,
            actual.uri(),
            jev.uri()
        ),
    )
    .unwrap();
    path
}

fn jevcat(config: &std::path::Path, args: &[&str]) -> (bool, String) {
    // Run from the config's temp dir so a real .env in the repo is never loaded.
    let output = Command::new(env!("CARGO_BIN_EXE_jevcat"))
        .current_dir(config.parent().unwrap())
        .arg("--config")
        .arg(config)
        .args(args)
        .env("ACTUAL_HTTP_API_KEY", "k")
        .env("TYPESAFE_API_KEY", "t")
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

fn transactions() -> Value {
    json!([
        { "id": "t1", "account": "a1", "date": "2026-10-08", "amount": -84750, "payee": "p1",
          "imported_payee": "ICA KVANTUM", "category": null, "notes": null },
        { "id": "t2", "account": "a1", "date": "2026-10-07", "amount": -4500, "payee": "p2",
          "category": null, "notes": "fika" },
        { "id": "t3", "account": "a1", "date": "2026-10-06", "amount": -100, "payee": "p1",
          "category": "c1", "notes": null },
        { "id": "t4", "account": "a1", "date": "2026-10-05", "amount": -100, "payee": "p1",
          "category": null, "transfer_id": "x", "notes": null }
    ])
}

#[tokio::test]
async fn run_applies_flags_and_misses_by_confidence() {
    let actual = MockServer::start().await;
    let jev = MockServer::start().await;
    mock_budget(&actual, transactions()).await;

    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", "Bearer t"))
        .and(body_partial_json(json!({ "state": { "payee": "ICA" } })))
        .respond_with(jev_response("Groceries", 0.93))
        .mount(&jev)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(body_partial_json(
            json!({ "state": { "payee": "Espresso House" } }),
        ))
        .respond_with(jev_response("Restaurants", 0.62))
        .mount(&jev)
        .await;

    let patches = Mock::given(method("PATCH"))
        .and(path(format!("/v1/budgets/{SYNC}/transactions/t1")))
        .and(body_partial_json(
            json!({ "transaction": { "category": "c1", "notes": "#jev" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "message": "ok" })))
        .expect(1)
        .named("apply t1");
    patches.mount(&actual).await;
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/budgets/{SYNC}/transactions/t2")))
        .and(body_partial_json(
            json!({ "transaction": { "notes": "fika #jev-flag [jev: Restaurants 0.62]" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "message": "ok" })))
        .expect(1)
        .named("flag t2")
        .mount(&actual)
        .await;

    Mock::given(method("POST"))
        .and(path(format!("/v1/budgets/{SYNC}/rules")))
        .and(body_partial_json(json!({ "rule": {
            "stage": null,
            "conditionsOp": "and",
            "conditions": [{ "field": "payee", "op": "is", "value": "p1", "type": "id" }],
            "actions": [{ "field": "category", "op": "set", "value": "c1", "type": "id" }]
        }})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "r1" } })))
        .expect(1)
        .named("rule for ICA")
        .mount(&actual)
        .await;

    let dir = tempdir();
    let config = write_config(&dir, &actual, &jev);

    let (ok, out) = jevcat(&config, &["run"]);
    assert!(ok, "{out}");
    assert!(out.contains("Dry run"), "{out}");
    assert!(out.contains("APPLY"), "{out}");
    assert!(out.contains("FLAG"), "{out}");
    assert!(out.contains("RULE   ICA"), "{out}");
    assert!(
        out.contains("would write: 1 applied, 1 flagged, 0 missed, 0 failed, 1 rules"),
        "{out}"
    );
    assert_eq!(
        actual
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "PATCH")
            .count(),
        0,
        "a dry run must not write"
    );
    assert_eq!(
        actual
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        0,
        "a dry run must not create rules"
    );

    let (ok, out) = jevcat(&config, &["run", "--apply"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("written: 1 applied, 1 flagged, 0 missed, 0 failed, 1 rules"),
        "{out}"
    );
    assert!(out.contains("skipped     1  transfer"), "{out}");
    assert!(out.contains("skipped     1  already categorized"), "{out}");
    actual.verify().await;

    let criteria: Vec<Request> = jev.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&criteria[0].body).unwrap();
    let options = body["questions"]["category"]["criteria"]
        .as_object()
        .unwrap();
    assert_eq!(options.len(), 3);
    assert_eq!(options["Groceries"], "Supermarkets");
    assert_eq!(options["Restaurants"], "Category group: Everyday");
    assert!(options.contains_key("Unsure"));
}

#[tokio::test]
async fn jev_retries_on_429_then_succeeds() {
    let actual = MockServer::start().await;
    let jev = MockServer::start().await;
    mock_budget(
        &actual,
        json!([{ "id": "t1", "account": "a1", "date": "2026-10-08", "amount": -100,
                 "payee": "p1", "category": null }]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after-ms", "10"))
        .up_to_n_times(1)
        .mount(&jev)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(jev_response("Groceries", 0.9))
        .mount(&jev)
        .await;

    let dir = tempdir();
    let config = write_config(&dir, &actual, &jev);
    let (ok, out) = jevcat(&config, &["run"]);
    assert!(ok, "{out}");
    assert!(out.contains("would write: 1 applied"), "{out}");
    assert_eq!(jev.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn import_sends_rows_then_categorizes_only_the_added_ones() {
    let actual = MockServer::start().await;
    let jev = MockServer::start().await;
    mock_budget(&actual, json!([])).await;

    // Before the import the account holds one old row without bank text, so the CSV's
    // ESPRESSO HOUSE row is not recognised as present and is sent. Actual then
    // fuzzy-matches it to old1 and reports it as updated.
    let before_import = json!([
        { "id": "old1", "account": "a1", "date": "2026-10-01", "amount": -500, "payee": "p2",
          "category": null, "notes": null }
    ]);
    let after_import = json!([
        { "id": "new1", "account": "a1", "date": "2026-10-09", "amount": -12900, "payee": "p1",
          "imported_payee": "ICA NÄRA", "category": null, "notes": null },
        { "id": "old1", "account": "a1", "date": "2026-10-01", "amount": -500, "payee": "p2",
          "imported_payee": "ESPRESSO HOUSE", "imported_id": "jevcat-bafa8e15ed756c85",
          "category": null, "notes": null }
    ]);
    Mock::given(method("POST"))
        .and(path(format!("/v1/budgets/{SYNC}/accounts/a1/transactions/import")))
        .and(body_partial_json(json!({
            "dryRun": false,
            "payeeNameNormalization": "title-case",
            "transactions": [
                { "date": "2026-10-01", "amount": -500, "payee_name": "ESPRESSO HOUSE" },
                { "date": "2026-10-09", "amount": -12900, "payee_name": "ICA NÄRA", "imported_payee": "ICA NÄRA" }
            ]
        })))
        .respond_with(move |_: &Request| {
            ResponseTemplate::new(201).set_body_json(json!({ "data": { "added": ["new1"], "updated": ["old1"] } }))
        })
        .expect(1)
        .mount(&actual)
        .await;
    // The snapshot load and the pre-import look-back see the old state; every later
    // read (verification, categorization) sees the imported rows. The run is limited to
    // the ids the import added, so old1 must stay untouched even though it is uncategorized.
    Mock::given(method("GET"))
        .and(path(format!("/v1/budgets/{SYNC}/accounts/a1/transactions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": before_import })))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&actual)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/budgets/{SYNC}/accounts/a1/transactions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": after_import })))
        .with_priority(2)
        .mount(&actual)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(jev_response("Groceries", 0.97))
        .expect(1)
        .mount(&jev)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/budgets/{SYNC}/rules")))
        .and(body_partial_json(json!({ "rule": {
            "conditions": [{ "field": "payee", "op": "is", "value": "p1" }],
            "actions": [{ "field": "category", "op": "set", "value": "c1" }]
        }})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "r1" } })))
        .expect(1)
        .mount(&actual)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/budgets/{SYNC}/transactions/new1")))
        .and(body_partial_json(
            json!({ "transaction": { "category": "c1", "notes": "#jev" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "message": "ok" })))
        .expect(1)
        .mount(&actual)
        .await;

    let dir = tempdir();
    let config = write_config(&dir, &actual, &jev);
    let csv = dir.join("statement.csv");
    std::fs::write(
        &csv,
        "Datum;Belopp;Text\n2026-10-09;-129,00;ICA NÄRA\n2026-10-01;-5,00;ESPRESSO HOUSE\n",
    )
    .unwrap();

    let (ok, out) = jevcat(&config, &["import", csv.to_str().unwrap()]);
    assert!(ok, "{out}");
    assert!(out.contains("2 rows in"), "{out}");
    assert!(out.contains("2 rows to import"), "{out}");
    assert!(
        out.contains("Import: 1 added, 1 matched existing, 0 restored; all 2 CSV rows present exactly once"),
        "{out}"
    );
    assert!(
        out.contains("written: 1 applied, 0 flagged, 0 missed, 0 failed, 1 rules"),
        "{out}"
    );
    actual.verify().await;
    jev.verify().await;
}

#[tokio::test]
async fn import_skips_rows_already_in_the_account_and_restores_merged_ones() {
    let actual = MockServer::start().await;
    let jev = MockServer::start().await;
    mock_budget(&actual, json!([])).await;

    // ICA NÄRA arrived through bank sync (different id scheme) and must be skipped.
    // A Swish to Alex from 2026-09-28 has no imported_id and the same amount as the
    // CSV's 2026-10-01 Swish, so Actual merges the new row into it.
    let before = json!([
        { "id": "sync1", "account": "a1", "date": "2026-10-09", "amount": -12900, "payee": "p1",
          "imported_payee": "ICA NÄRA", "imported_id": "2026-10-09-20.03.00.1", "category": "c1", "notes": "ICA NÄRA #jev" },
        { "id": "old1", "account": "a1", "date": "2026-09-28", "amount": -500, "payee": "p2",
          "imported_payee": "Swish betalning ALEX", "category": "c2", "notes": "Swish betalning ALEX" }
    ]);
    let merged = json!([
        before[0],
        { "id": "old1", "account": "a1", "date": "2026-09-28", "amount": -500, "payee": "p2",
          "imported_payee": "Swish betalning ALEX", "imported_id": "jevcat-a6f1df4b2d80b0d7",
          "category": "c2", "notes": "Swish betalning ALEX" }
    ]);
    let restored = json!([
        before[0], before[1],
        { "id": "new1", "account": "a1", "date": "2026-10-01", "amount": -500, "payee": "p2",
          "imported_payee": "Swish betalning ALEX", "imported_id": "jevcat-a6f1df4b2d80b0d7",
          "category": "c2", "notes": "Swish betalning ALEX" }
    ]);
    for (state, n, priority) in [(before, 2, 1), (merged, 1, 2), (restored, 1, 3)] {
        Mock::given(method("GET"))
            .and(path(format!("/v1/budgets/{SYNC}/accounts/a1/transactions")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": state })))
            .up_to_n_times(n)
            .with_priority(priority)
            .mount(&actual)
            .await;
    }
    Mock::given(method("POST"))
        .and(path(format!("/v1/budgets/{SYNC}/accounts/a1/transactions/import")))
        .and(body_partial_json(json!({ "transactions": [
            { "date": "2026-10-01", "amount": -500, "payee_name": "Swish betalning ALEX" }
        ]})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "data": { "added": [], "updated": ["old1"] } })))
        .expect(1)
        .mount(&actual)
        .await;
    // The repair: clear the stolen id on old1, add the row back with old1's payee and category.
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/budgets/{SYNC}/transactions/old1")))
        .and(body_partial_json(json!({ "transaction": { "imported_id": null } })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "message": "ok" })))
        .expect(1)
        .mount(&actual)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/budgets/{SYNC}/accounts/a1/transactions")))
        .and(body_partial_json(json!({ "transaction": {
            "date": "2026-10-01", "amount": -500, "payee": "p2", "category": "c2",
            "imported_payee": "Swish betalning ALEX", "imported_id": "jevcat-a6f1df4b2d80b0d7", "cleared": true
        }, "learnCategories": false })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "message": "ok" })))
        .expect(1)
        .mount(&actual)
        .await;

    let dir = tempdir();
    let config = write_config(&dir, &actual, &jev);
    let csv = dir.join("statement.csv");
    std::fs::write(
        &csv,
        "Datum;Belopp;Text\n2026-10-09;-129,00;ICA NÄRA\n2026-10-01;-5,00;Swish betalning ALEX\n",
    )
    .unwrap();

    let (ok, out) = jevcat(&config, &["import", csv.to_str().unwrap(), "--no-categorize"]);
    assert!(ok, "{out}");
    assert!(out.contains("1 rows already in the account, skipped:"), "{out}");
    assert!(out.contains("1 rows Actual may merge into an older transaction"), "{out}");
    assert!(out.contains("restored 2026-10-01      -5.00 Swish betalning ALEX (payee and category copied from 2026-09-28)"), "{out}");
    assert!(
        out.contains("Import: 0 added, 1 matched existing, 1 restored; all 2 CSV rows present exactly once"),
        "{out}"
    );
    actual.verify().await;
    assert!(jev.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn import_dry_run_writes_nothing() {
    let actual = MockServer::start().await;
    let jev = MockServer::start().await;
    mock_budget(&actual, json!([])).await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/budgets/{SYNC}/accounts/a1/transactions/import"
        )))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&actual)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(jev_response("Groceries", 0.97))
        .expect(1)
        .mount(&jev)
        .await;

    let dir = tempdir();
    let config = write_config(&dir, &actual, &jev);
    let csv = dir.join("statement.csv");
    std::fs::write(&csv, "Datum;Belopp;Text\n2026-10-09;-129,00;ICA\n").unwrap();

    let (ok, out) = jevcat(
        &config,
        &[
            "import",
            csv.to_str().unwrap(),
            "--dry-run",
            "--account",
            "checking",
        ],
    );
    assert!(ok, "{out}");
    assert!(out.contains("1 rows to import"), "{out}");
    assert!(
        out.contains("APPLY") && out.contains("-> Groceries (0.97)"),
        "{out}"
    );
    assert!(out.contains("would write: 1 applied"), "{out}");
    jev.verify().await;
    actual.verify().await;
    assert_eq!(
        actual
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "PATCH")
            .count(),
        0
    );
}

#[tokio::test]
async fn calibrate_reports_buckets_and_a_threshold() {
    let actual = MockServer::start().await;
    let jev = MockServer::start().await;
    mock_budget(
        &actual,
        json!([
            { "id": "t1", "account": "a1", "date": "2026-10-08", "amount": -100, "payee": "p1", "category": "c1" },
            { "id": "t2", "account": "a1", "date": "2026-10-07", "amount": -100, "payee": "p2", "category": "c2" },
            { "id": "t3", "account": "a1", "date": "2026-10-06", "amount": -100, "payee": "p1", "category": null }
        ]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(body_partial_json(json!({ "state": { "payee": "ICA" } })))
        .respond_with(jev_response("Groceries", 0.95))
        .mount(&jev)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(body_partial_json(
            json!({ "state": { "payee": "Espresso House" } }),
        ))
        .respond_with(jev_response("Groceries", 0.55))
        .mount(&jev)
        .await;

    let dir = tempdir();
    let config = write_config(&dir, &actual, &jev);
    let (ok, out) = jevcat(&config, &["calibrate", "--target", "0.9"]);
    assert!(ok, "{out}");
    assert!(out.contains("2 samples"), "{out}");
    assert!(
        out.contains("Suggested apply_threshold for 90% precision: 0.6"),
        "{out}"
    );
    assert!(out.contains("Restaurants -> Groceries"), "{out}");
    assert_eq!(
        jev.received_requests().await.unwrap().len(),
        2,
        "uncategorized rows are not replayed"
    );
}

#[tokio::test]
async fn list_needs_no_jev_and_secrets_are_required() {
    let actual = MockServer::start().await;
    let jev = MockServer::start().await;
    mock_budget(&actual, transactions()).await;
    let dir = tempdir();
    let config = write_config(&dir, &actual, &jev);

    let (ok, out) = jevcat(&config, &["list"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("2 eligible transactions, 2 categories"),
        "{out}"
    );
    assert!(jev.received_requests().await.unwrap().is_empty());

    // Run from the config's temp dir so a real .env in the repo is never loaded.
    let output = Command::new(env!("CARGO_BIN_EXE_jevcat"))
        .current_dir(config.parent().unwrap())
        .arg("--config")
        .arg(&config)
        .arg("list")
        .env_clear()
        .envs(HashMap::from([(
            "PATH",
            std::env::var("PATH").unwrap_or_default(),
        )]))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ACTUAL_HTTP_API_KEY"));
}

fn tempdir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "jevcat-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn learn_creates_rules_from_earlier_tags_and_skips_covered_payees() {
    let actual = MockServer::start().await;
    let jev = MockServer::start().await;
    mock_budget(
        &actual,
        json!([
            // ICA: applied by Jev twice, agrees -> rule
            { "id": "t1", "account": "a1", "date": "2026-10-08", "amount": -100, "payee": "p1", "category": "c1", "notes": "#jev" },
            { "id": "t2", "account": "a1", "date": "2026-10-07", "amount": -100, "payee": "p1", "category": "c1", "notes": "weekly #jev" },
            // Espresso House: applied twice but a person moved one -> no rule
            { "id": "t3", "account": "a1", "date": "2026-10-06", "amount": -100, "payee": "p2", "category": "c2", "notes": "#jev" },
            { "id": "t4", "account": "a1", "date": "2026-10-05", "amount": -100, "payee": "p2", "category": "c2", "notes": "#jev" },
            { "id": "t5", "account": "a1", "date": "2026-10-04", "amount": -100, "payee": "p2", "category": "c1", "notes": null }
        ]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/budgets/{SYNC}/rules")))
        .and(body_partial_json(json!({ "rule": {
            "conditions": [{ "field": "payee", "op": "is", "value": "p1" }],
            "actions": [{ "field": "category", "op": "set", "value": "c1" }]
        }})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "r1" } })))
        .expect(1)
        .mount(&actual)
        .await;

    let dir = tempdir();
    let config = write_config(&dir, &actual, &jev);

    let (ok, out) = jevcat(&config, &["learn", "--after", "2"]);
    assert!(ok, "{out}");
    assert!(out.contains("RULE   ICA"), "{out}");
    assert!(!out.contains("Espresso"), "{out}");
    assert!(out.contains("would create 1 rules"), "{out}");

    let (ok, out) = jevcat(&config, &["learn", "--after", "2", "--apply"]);
    assert!(ok, "{out}");
    assert!(out.contains("created 1 rules"), "{out}");
    actual.verify().await;
    assert!(jev.received_requests().await.unwrap().is_empty());

    // Once a rule covers ICA, nothing is proposed.
    Mock::given(method("GET"))
        .and(path(format!("/v1/budgets/{SYNC}/rules")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [
            { "id": "r1", "stage": null, "conditionsOp": "and",
              "conditions": [{ "field": "payee", "op": "is", "value": "p1", "type": "id" }],
              "actions": [{ "field": "category", "op": "set", "value": "c1", "type": "id" }] }
        ]})))
        .with_priority(1)
        .mount(&actual)
        .await;
    let (ok, out) = jevcat(&config, &["learn", "--after", "2"]);
    assert!(ok, "{out}");
    assert!(out.contains("would create 0 rules"), "{out}");
}
