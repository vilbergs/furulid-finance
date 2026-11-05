//! `ActualStore` backed by actual-http-api, the Node sidecar that wraps `@actual-app/api`.

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::{MonthCategory, 
    Account, ActualStore, AddTransaction, CategoryGroup, ImportOptions, ImportResult,
    NewTransaction, Payee, Rule, Transaction, TransactionPatch,
};

pub struct HttpStore {
    client: reqwest::Client,
    base_url: String,
    budget_sync_id: String,
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

impl HttpStore {
    pub fn new(
        base_url: &str,
        budget_sync_id: &str,
        api_key: &str,
        encryption_password: Option<&str>,
    ) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_str(api_key)?);
        if let Some(password) = encryption_password {
            headers.insert(
                "budget-encryption-password",
                HeaderValue::from_str(password)?,
            );
        }
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .context("building HTTP client for actual-http-api")?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            budget_sync_id: budget_sync_id.to_string(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/v1/budgets/{}/{}",
            self.base_url, self.budget_sync_id, path
        )
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let url = self.url(path);
        let response = self
            .client
            .get(&url)
            .query(query)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .with_context(|| format!("reading {url}"))?;
        anyhow::ensure!(status.is_success(), "GET {url} returned {status}: {body}");
        let envelope: Envelope<T> =
            serde_json::from_str(&body).with_context(|| format!("decoding {url}: {body}"))?;
        Ok(envelope.data)
    }

    /// Any request against the budget, for `jevcat api`. Returns the `data` field when
    /// the response has one, otherwise the whole body.
    pub async fn raw(
        &self,
        method: &str,
        path: &str,
        query: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let mut url = self.url(path.trim_start_matches('/'));
        if let Some(query) = query {
            url.push('?');
            url.push_str(query);
        }
        let method = reqwest::Method::from_bytes(method.to_uppercase().as_bytes())
            .with_context(|| format!("bad method {method:?}"))?;
        let mut request = self.client.request(method.clone(), &url);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("{method} {url}"))?;
        let status = response.status();
        let text = response.text().await.with_context(|| format!("reading {url}"))?;
        anyhow::ensure!(status.is_success(), "{method} {url} returned {status}: {text}");
        let value: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|_| serde_json::Value::String(text.clone()));
        Ok(match value {
            serde_json::Value::Object(mut map) if map.contains_key("data") => {
                map.remove("data").unwrap_or_default()
            }
            other => other,
        })
    }

    async fn post_json(&self, path: &str, body: &serde_json::Value) -> Result<()> {
        let url = self.url(path);
        let response = self
            .client
            .post(&url)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("POST {url} returned {status}: {body}");
        }
        Ok(())
    }
}

#[async_trait]
impl ActualStore for HttpStore {
    async fn accounts(&self) -> Result<Vec<Account>> {
        self.get("accounts", &[]).await
    }

    async fn account_balance(&self, account_id: &str) -> Result<i64> {
        self.get(&format!("accounts/{account_id}/balance"), &[]).await
    }

    async fn category_groups(&self) -> Result<Vec<CategoryGroup>> {
        self.get("categorygroups", &[]).await
    }

    async fn payees(&self) -> Result<Vec<Payee>> {
        self.get("payees", &[]).await
    }

    async fn transactions(&self, account_id: &str, since: &str) -> Result<Vec<Transaction>> {
        let rows: Vec<Transaction> = self
            .get(
                &format!("accounts/{account_id}/transactions"),
                &[("since_date", since)],
            )
            .await?;
        // Deleted rows come back with tombstone set; nothing here should see them.
        Ok(rows.into_iter().filter(|tx| !tx.tombstone).collect())
    }

    async fn update_transaction(&self, id: &str, patch: &TransactionPatch) -> Result<()> {
        let url = self.url(&format!("transactions/{id}"));
        let response = self
            .client
            .patch(&url)
            .json(&serde_json::json!({ "transaction": patch }))
            .send()
            .await
            .with_context(|| format!("PATCH {url}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("PATCH {url} returned {status}: {body}");
        }
        Ok(())
    }

    async fn import_transactions(
        &self,
        account_id: &str,
        transactions: &[NewTransaction],
        options: &ImportOptions,
    ) -> Result<ImportResult> {
        let url = self.url(&format!("accounts/{account_id}/transactions/import"));
        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "transactions": transactions,
                "dryRun": options.dry_run,
                "payeeNameNormalization": options.payee_normalization,
            }))
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .with_context(|| format!("reading {url}"))?;
        anyhow::ensure!(status.is_success(), "POST {url} returned {status}: {body}");
        let envelope: Envelope<ImportResult> =
            serde_json::from_str(&body).with_context(|| format!("decoding {url}: {body}"))?;
        Ok(envelope.data)
    }

    async fn rules(&self) -> Result<Vec<Rule>> {
        self.get("rules", &[]).await
    }

    async fn create_rule(&self, rule: &Rule) -> Result<()> {
        self.post_json("rules", &serde_json::json!({ "rule": rule }))
            .await
    }

    async fn add_transaction(&self, account_id: &str, transaction: &AddTransaction) -> Result<()> {
        // One transaction per call: the endpoint rejects a `transactions` array.
        self.post_json(
            &format!("accounts/{account_id}/transactions"),
            &serde_json::json!({
                "transaction": transaction,
                "learnCategories": false,
                "runTransfers": false,
            }),
        )
        .await
    }

    async fn create_payee(&self, name: &str) -> Result<String> {
        let value = self
            .raw(
                "POST",
                "payees",
                None,
                Some(serde_json::json!({ "payee": { "name": name } })),
            )
            .await?;
        value
            .as_str()
            .map(str::to_string)
            .with_context(|| format!("creating payee {name:?}: unexpected response {value}"))
    }

    async fn rename_payee(&self, id: &str, name: &str) -> Result<()> {
        let url = self.url(&format!("payees/{id}"));
        let response = self
            .client
            .patch(&url)
            .json(&serde_json::json!({ "payee": { "name": name } }))
            .send()
            .await
            .with_context(|| format!("PATCH {url}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("PATCH {url} returned {status}: {body}");
        }
        Ok(())
    }

    async fn merge_payees(&self, target_id: &str, merge_ids: &[String]) -> Result<()> {
        self.post_json(
            "payees/merge",
            &serde_json::json!({ "targetId": target_id, "mergeIds": merge_ids }),
        )
        .await
    }

    async fn month_categories(&self, month: &str) -> Result<Vec<MonthCategory>> {
        self.get(&format!("months/{month}/categories"), &[]).await
    }

    async fn set_budgeted(&self, month: &str, category_id: &str, amount: i64) -> Result<()> {
        let url = self.url(&format!("months/{month}/categories/{category_id}"));
        let response = self
            .client
            .patch(&url)
            .json(&serde_json::json!({ "category": { "budgeted": amount } }))
            .send()
            .await
            .with_context(|| format!("PATCH {url}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("PATCH {url} returned {status}: {body}");
        }
        Ok(())
    }

    async fn category_note(&self, category_id: &str) -> Result<String> {
        let value = self
            .raw("GET", &format!("notes/category/{category_id}"), None, None)
            .await?;
        Ok(value.as_str().unwrap_or("").to_string())
    }

    async fn set_category_note(&self, category_id: &str, note: &str) -> Result<()> {
        let url = self.url(&format!("notes/category/{category_id}"));
        let response = self
            .client
            .put(&url)
            .json(&serde_json::json!({ "data": note }))
            .send()
            .await
            .with_context(|| format!("PUT {url}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("PUT {url} returned {status}: {body}");
        }
        Ok(())
    }

    async fn bank_sync(&self, account_id: &str) -> Result<()> {
        self.post_json(
            &format!("accounts/{account_id}/banksync"),
            &serde_json::json!({}),
        )
        .await
    }
}
