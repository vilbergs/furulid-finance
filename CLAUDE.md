# furulid-finance

Vilberg's household finance setup. The household is Vilberg, Melanie, their kids (Ylva
among them) and some goats, in Kungsäter outside Varberg. One bank account, a Nordea
Personkonto, is tracked in a self-hosted [Actual Budget](https://actualbudget.org/).
This repo holds `jevcat`, the Rust tool that categorizes those transactions, plus the Fly
deploy files and Claude skills for working with the budget.

## The pieces

| Piece | Where | Notes |
|---|---|---|
| Actual server | Fly.io, app name in `.env` | Budget data lives on its volume. Bank sync (Nordea via GoCardless) runs inside it and covers 2026-07-15 onwards. |
| actual-http-api | Fly.io, URL in `jevcat.toml` | REST sidecar over `@actual-app/api`. Everything here talks to it; the key is `ACTUAL_HTTP_API_KEY` in `.env`. Same version as the server, always. |
| jevcat | `src/`, installed as `jevcat` | Imports CSVs and categorizes with TypeSafe Jev. `README.md` explains the commands. |
| Rubric | `jevcat.toml` `[jev.descriptions]` | One line per category telling Jev what belongs there. This is the main accuracy lever and it is gitignored, so edits only live on this machine. |
| Skills | `.claude/skills/` | See below. |

`.env` and `jevcat.toml` are gitignored and must stay that way. Never print the API key.
`cargo test` for unit tests; `cargo test preflight -- --ignored --nocapture` checks the
live wiring.

## Skills

- `actual-api`: query or patch the budget with `jevcat api`. Documents the endpoints and
  their quirks.
- `fill-gap`: `jevcat import` for a period bank sync missed, without duplicates.
- `review-categories`: `jevcat open` to list uncategorized rows with full bank context,
  `jevcat --since-days N run` for a wider Jev pass, `jevcat set` to apply categories with a
  `#reviewed` note, and how to identify payees.
- `update-actual`: bump the server and API images on Fly in lockstep.

Everything is a `jevcat` subcommand; prefer them over ad hoc curl or scripts. They encode
what went wrong before, and the whole run of `open` is under a second.

## How the data looks

- Amounts are integer öre. Dates are `YYYY-MM-DD`.
- A transaction's bank text is in `imported_payee` and `notes`. For Bankgiro and Autogiro
  rows the bank sends a reference number as payee and the real description on a second
  line, which ends up in `notes`. Always read notes, not only the payee.
- Three generations of rows: before 2025-11-22 hand-imported (no `imported_id`), 2025-11-22
  to 2026-07-15 from jevcat CSV import (`jevcat-` ids), after that bank sync (timestamp ids,
  `raw_synced_data` with transaction type and original currency).
- Bank sync does not clean payee names, so synced card rows get a payee per day
  (`Kortköp 260817 DISNEY PLUS`). Rules keyed on payee will not fire for them; key rules on
  `imported_payee contains` instead. Merging those payees in Actual is an open chore.
- Tags in notes: `#jev` applied by Jev, `#jev-flag [jev: X 0.71]` below threshold,
  `#jev-miss` Jev was unsure, `#reviewed` set by hand or by Claude with Vilberg's input.

## Categorization conventions

Decided with Vilberg; apply them without re-asking.

- Food at home, including takeaway and work lunches Swished to colleagues (Twedmark,
  Alex Wang, Josefine Österlund): Restaurants/Cafes. Food abroad or in transit: Food & Drink.
- Expenses the employer (Teachiq / Exam.net) pays back through salary: Income, so they net.
- Apple.com/Bill is mixed; the four subscriptions have amount rules, everything else needs
  the receipt. Nintendo, Apple Arcade, Steam, EVE: Hobbies/Leisure. Medley Pingvinen,
  Astrid Lindgrens Värld, Leo's Lekland, Kungsäters IF: Kids Leisure.
- Hay and feed for the goats: Animals. Second-hand tools: Maintenance. Bigger building
  projects: Renovation.
- Euroblast festival in Köln every late September (Mostar-Promotion GmbH, SumUp vendors):
  Entertainment. Concert tickets bought via a friend: Entertainment.
- Never create rules on numeric reference-number payees.
- Rows under a few hundred kronor that nobody can identify may stay uncategorized; say what
  share of spending is open and let Vilberg decide.

## Giving financial advice

When Vilberg asks how things look or what to change, work from the budget, not from
general rules:

- Pull `months/<YYYY-MM>/categories` for budgeted, spent and balance per category, and
  `accounts/<id>/transactions` for the detail. Compare at least three months; a single
  month is noise (Euroblast, Iceland, Christmas).
- Report the uncategorized share first. Advice on a budget with more than a few percent
  uncategorized is advice on a guess.
- Fixed commitments to keep in mind: three Nordea mortgage loans (Omsättning lån, ~9,335 kr
  each per quarter plus interest), a Ziklo car loan, Avanza savings for Ylva (730 kr/month),
  Fello, Telia, Verisure, Varberg El and Vatten, insurance with If, LF, Arctic and ICA.
- Income is salary from Teachiq AB, child benefit (Barnbidrag), parental benefit
  (Föräldrapenning, FKASSA) and occasional Utlandsinsättning from abroad.
- Be concrete: name the category, the monthly amount, and what the number would be after
  the change. Kronor, no percentages without the amount next to them. Vilberg prefers a
  short table over prose.
- Do not moralize about hobbies, travel or the kids. Point out patterns (a subscription
  nobody uses, a category that doubled) and leave the judgment to him.
