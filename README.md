# jevcat

Categorizes [Actual Budget](https://actualbudget.org/) transactions with
[TypeSafe Jev](https://typesafe.ai), a decision model that answers a multiple-choice
question with a calibrated confidence instead of generating text. Every category is an
option, `Unsure` is always one of them, and the confidence decides whether a category is
written, flagged for review, or left alone.

Run it by hand. There is no daemon and no schedule.

```
jevcat import statement.csv      import a bank CSV into Actual, then categorize what it added
jevcat run                       categorize what is already uncategorized (dry run)
jevcat run --apply               ...and write it
jevcat calibrate                 measure accuracy per confidence on your categorized history
jevcat learn --apply             turn payees Jev has placed consistently into Actual rules
jevcat list                      show what a run would look at, without calling Jev
jevcat open                      table of uncategorized transactions with the bank text and Jev's guess
jevcat set plan.json             write categories from a plan, tagging each row #reviewed
jevcat api accounts              call actual-http-api directly
```

`--since-days N` before any command widens the look-back for that call.

## How it reaches Actual

Actual's server is a sync log, not a REST API, so the binary talks to
[actual-http-api](https://github.com/jhonderson/actual-http-api), a small container that
wraps the official `@actual-app/api` and exposes it over HTTP. Run it next to your Actual
server:

```sh
docker run -d --name actual-http-api \
  -p 5007:5007 \
  -e ACTUAL_SERVER_URL="http://actual-server:5006/" \
  -e ACTUAL_SERVER_PASSWORD="…" \
  -e API_KEY="pick-a-long-random-string" \
  jhonderson/actual-http-api:26.10.0
```

Hosting Actual on Fly.io? `deploy/actual-server/` and `deploy/actual-api/` have a `fly.toml`
each, and `deploy/actual-api/README.md` has the steps to run both there.

Everything that touches Actual goes through the `ActualStore` trait in `src/actual`, so a
native Rust sync client can replace the sidecar later without changing anything else.

## Setup

```sh
cargo install --path .
cp jevcat.example.toml jevcat.toml      # fill in base_url, budget_sync_id, [import] columns
cp .env.example .env                    # fill in the API keys; exported env vars also work
jevcat list
```

Check that everything is wired up before the first import:

```sh
cargo test preflight -- --ignored --nocapture
```

It reads `jevcat.toml` and `.env`, loads the budget (read-only), checks that the import
account and every `[jev.descriptions]` key exist, and sends five sample bank rows through the
import parser and the real Jev client. It is ignored by a plain `cargo test`.

Then calibrate before trusting it:

```sh
jevcat calibrate --limit 300
```

It replays your most recent categorized transactions through Jev and prints accuracy per
confidence bucket plus a suggested `apply_threshold` for 95% precision. Put that number in
the config. If no threshold reaches the target, improve `[jev.descriptions]` and rerun;
the rubric text is what Jev reads.

## Importing a CSV

```sh
jevcat import ~/Downloads/statement.csv --account Checking
```

1. The CSV is parsed with the column names, delimiter and date format from `[import]`.
   Amounts like `-1 234,56`, `-1.234,56` and `1,234.56` are all read correctly.
2. Rows already in the account are skipped: same date, amount, and the bank text found in
   the existing row's notes or imported payee. That catches rows that arrived through
   bank sync or by hand, which Actual's own `imported_id` check cannot see.
3. The rest go through Actual's import in chunks of 100 (`--chunk`), oldest first, which
   runs your rules. Actual also fuzzy-matches a new row to an older one with the same
   amount within seven days; the import re-reads the account afterwards, checks that every
   CSV row is present exactly once, and adds back the rows that were merged away.
4. Only the rows that were added are classified, and the result is written.

`--dry-run` prints what would be skipped and imported, and how Jev would classify the new
rows, without writing anything. `--no-categorize` imports only.

## Reviewing what Jev could not place

```sh
jevcat open                          # uncategorized rows, last six months
jevcat open --around 2026-09-26      # everything within three days of a date
jevcat open --payee "Sot & Ven"      # every row for a payee, with its category
jevcat set plan.json --dry-run       # then without --dry-run
```

`open` prints a markdown table with the payee, the bank's full text (including the second
remittance line Nordea puts in notes and Swish messages), the transaction type and original
currency from bank sync, and Jev's guess. `set` takes a JSON list of
`{"id", "category", "note"?, "replace_notes"?, "rule"?}` and writes the category with a
`#reviewed` tag, optionally creating a payee rule.

## What a run writes

| Jev says | Confidence | Action | Note tag |
|---|---|---|---|
| a category | at or above threshold | category set | `#jev` |
| a category | below threshold | left uncategorized | `#jev-flag [jev: Groceries 0.71]` |
| `Unsure` | any | left uncategorized, not retried | `#jev-miss` |

Search `#jev-flag` in Actual to review the guesses. Tags are the only state the tool
keeps; `--rerun-missed` retries the `#jev-miss` ones.

## Rules, so Jev is asked less over time

Once a payee has been applied with confidence `policy.learn_after` times (default 2) and
every applied transaction of that payee agrees on the category, a run creates an Actual
rule `payee is X → set category Y`. From then on Actual categorizes that payee itself on
import or bank sync and it never reaches Jev. A rule is not created when a transaction
you categorized by hand disagrees, or when a rule for the payee already exists.
`jevcat learn` does the same pass on demand without calling Jev.

Actual's own category learning (the toggle at the bottom of the Payees page) does the
same for transactions you categorize by hand. Keep it on; the two complement each other.

Skipped and never touched: transfers, split parents, starting balances, off-budget
accounts, rows with no payee, and anything that already has a category.

## Layout

| Module | Role |
|---|---|
| `actual` | `ActualStore` trait, budget types, `Snapshot` |
| `actual::http` | the actual-http-api backend |
| `classify` | `Classifier` trait, `Taxonomy` built from category groups, `Subject` sent to the model |
| `classify::jev` | one `POST /v1/systemone` with retries |
| `import` | CSV parsing to Actual's import shape |
| `gap` | matching CSV rows against the account, verification after an import |
| `review` | `open` and `set` |
| `text` | reading bank text, currency and Jev tags off a transaction |
| `filter` | eligibility rules |
| `policy` | decision to apply / flag / skip |
| `calibrate` | accuracy per confidence bucket, threshold suggestion |
| `run` | the pass over the budget |
| `tags` | note tag handling |

`cargo test` runs the unit tests; `tests/` holds end-to-end tests against mock HTTP servers.
