---
name: fill-gap
description: Import a Nordea CSV export into Actual to fill a period the bank sync missed, without creating duplicates. Use when asked to import a bank statement, fill a gap in the transaction history, backfill months, or re-import a CSV safely. `jevcat import` skips rows already present, imports in chunks, verifies every row landed exactly once and restores rows Actual's fuzzy matching swallowed.
---

# Fill a gap from a Nordea CSV

`jevcat import` does the whole job. It matches each CSV row against the account on date,
amount and bank text, so rows that arrived through bank sync or by hand are recognised
even though their `imported_id` differs. It sends the rest in chunks of 100 (the endpoint
rejects large bodies), then re-reads the account and checks that every CSV row is there
exactly once. Rows Actual merged into an older transaction by fuzzy matching are added
back with the older row's payee and category.

The CSV must be the Nordea "Personkonto" export (`Bokföringsdag;Belopp;...;Rubrik;...`),
which `[import]` in `jevcat.toml` describes. Keep the file outside the repo; `*.csv` is
gitignored anyway.

## 1. Dry run

```sh
jevcat import ~/Downloads/statement.csv --dry-run --no-categorize
```

Prints the rows already in the account (skipped), the rows to import, and the rows
Actual may merge into an older transaction. If the skipped list holds anything other
than the boundary days, stop and look: the sync may reach further than assumed. Drop
`--no-categorize` to also see how Jev would classify the new rows (calls Jev, writes
nothing).

## 2. Import

```sh
jevcat import ~/Downloads/statement.csv
```

Watch for:

- `chunk N: ... X added, Y matched existing`. `Y > 0` means fuzzy matching; the
  verification below handles it.
- `restored ...` lines: rows added back after a merge.
- `EXTRA` lines: rows in the account inside the CSV's date range that the CSV does not
  explain. Usually fine (a bank-sync row from the boundary day) but read them.
- The final `Import: A added, B matched existing, C restored; all N CSV rows present
  exactly once`. If that line is missing the command failed; the error says which chunk.

The added rows then go through Jev with the usual apply / flag / miss policy, and payee
rules are learned. `--no-categorize` imports only; `--chunk 50` for a stubborn 413.

## 3. Report

Say how many rows the CSV had, how many were skipped as already present, how many were
added and restored, and what Jev applied / flagged / missed (the `written:` line). Then
suggest `review-categories` for the flagged and missed rows.
