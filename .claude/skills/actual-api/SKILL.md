---
name: actual-api
description: Read and change data in the Actual Budget through actual-http-api (accounts, transactions, categories, payees, rules, monthly budgets, balances) using `jevcat api`. Use when asked to look something up in the budget, check whether transactions exist, inspect categories or rules, or make a one-off fix to a transaction. Also the reference for the endpoint quirks (payload limits, fuzzy matching) and the shape of the data.
---

# Query the Actual budget over actual-http-api

Actual's own server is a sync log with no REST API. Everything goes through
actual-http-api, the sidecar on Fly that wraps `@actual-app/api`. The URL and budget sync
id are in `jevcat.toml` (`[actual]`), the key is `ACTUAL_HTTP_API_KEY` in `.env`. Both are
gitignored; never print the key or commit either file.

`jevcat api` handles the key, the base URL and the `data` envelope:

```sh
jevcat api accounts
jevcat api accounts/<account id>/transactions 'since_date=2026-06-01&until_date=2026-06-30'
jevcat api -X PATCH transactions/<id> '{"transaction":{"notes":"fixed"}}'
jevcat api -X POST rules '{"rule":{...}}'
```

It prints the `data` field as pretty JSON, or the whole body when there is none, and
fails with the status and body on a non-2xx response. For anything beyond a lookup, the
higher-level commands are faster and safer: `jevcat open` (what is uncategorized, with
context), `jevcat set` (write categories from a plan), `jevcat import` (CSV without
duplicates). See the `review-categories` and `fill-gap` skills.

## Facts about this budget

- One account, `Nordea Account`. `jevcat api accounts` gives the id; `open`, `set` and
  `import` pick it automatically.
- Amounts are integer minor units (öre): `-25000` is `-250,00 kr`.
- Dates are `YYYY-MM-DD` strings. Months are `YYYY-MM`.
- Category and payee fields on a transaction are ids. `jevcat open` resolves them.
- Transactions arrive in three shapes. Rows from the bank sync (2026-07-15 onwards) carry
  an `imported_id` like `2026-07-16-16.03.26.982942`, the bank text in `imported_payee`
  and `notes`, and `raw_synced_data` with the transaction type and original currency.
  Rows from a jevcat CSV import carry `imported_id` `jevcat-<16 hex>` and the raw Nordea
  text (`Kortköp 251121 ICA ...`) in `notes` and `imported_payee`. Older hand-entered rows
  (before 2025-11-22) have no `imported_id` and the raw text in `notes`.
- For Bankgiro and Autogiro rows the bank sends a reference number as payee and the real
  description as a second line, which lands in `notes` (`000029407 Autogiro K*VR Resa`).
  Swish rows carry the sender's message there too.
- Deleted rows come back with `tombstone` set; jevcat drops them. `is_child` rows are
  parts of their `parent_id` split.

## Endpoints that exist on the deployed version (26.10.0)

| Path | Returns |
|---|---|
| `accounts` | list of accounts with `closed`, `offbudget` |
| `accounts/<id>/balance` | integer balance in öre |
| `accounts/<id>/transactions?since_date=&until_date=` | transactions; `until_date` optional |
| `transactions/<id>` (PATCH, body `{"transaction": {...}}`) | update fields; `"imported_id": null` clears it |
| `accounts/<id>/transactions` (POST, body `{"transaction": {...}, "learnCategories": false, "runTransfers": false}`) | add one transaction, no matching against existing rows |
| `accounts/<id>/transactions/import` (POST, body `{"transactions": [...], "dryRun": bool}`) | Actual's import with rules and de-duplication |
| `categories`, `categorygroups` | categories (`is_income`, `hidden`, `group_id`) |
| `months` | list of month strings the budget covers |
| `months/<YYYY-MM>` | totals: `totalBudgeted`, `totalIncome`, `toBudget`, ... |
| `months/<YYYY-MM>/categories` | per category `budgeted`, `spent`, `balance` for the month |
| `payees` | payees with `transfer_acct`; POST `{"payee":{"name":...}}` creates, DELETE `payees/<id>` removes |
| `rules` | rules with `conditions` and `actions`; PATCH `rules/<id>` updates |

The add endpoint takes one transaction per call under `transaction`; a `transactions`
array returns `transaction information is required`.

## Import quirks worth knowing before writing to the budget

- The import endpoint returns `413 Payload Too Large` somewhere above a few hundred rows.
  Nothing is written when that happens. `jevcat import` sends chunks of 100.
- Besides exact `imported_id` matching, Actual fuzzy-matches an incoming row to an
  existing one with the same amount within 7 days, preferring rows that have no
  `imported_id`. A matched row is reported as `updated`, not `added`, and the incoming row
  is dropped. `jevcat import` verifies every row afterwards and restores the lost ones.
- `dryRun` on the import endpoint was ignored by older actual-http-api versions; jevcat
  never sends it and does its dry run locally.
- The first request after the Fly machine has been idle can take several seconds while
  the API loads the budget. Retry once before concluding the API is down.
