<!-- BEGIN swamp managed section - DO NOT EDIT -->
# Project

This repository is managed with [swamp](https://github.com/swamp-club/swamp).

## Rules

1. **Search before you build.** When automating AWS, APIs, or any external service: (a) search community extensions with `swamp extension search <query>` — prefer `@swamp/*` official extensions first, (b) search local/installed types with `swamp behavior type search <query>`, (c) if a community extension exists, install it with `swamp extension pull <package>` instead of building from scratch, (d) extend an existing type if it covers the domain but lacks the method you need, (e) only create a custom extension behavior in `extensions/models/` as a last resort. Use the `swamp` skill for guidance. The `command/shell` behavior is ONLY for ad-hoc one-off shell commands, NEVER for wrapping CLI tools or building integrations.
2. **Extend, don't be clever.** When a behavior covers the domain but lacks the method you need, extend it with `export const extension` — don't bypass it with shell scripts, CLI tools, or multi-step hacks. One method, one purpose. Use `swamp behavior type describe <type> --json` to check available methods.
3. **Use the data model.** Once data exists in a behavior (via `lookup`, `start`, `sync`, etc.), reference it with CEL expressions. Don't re-fetch data that's already available.
4. **CEL expressions everywhere.** Wire behaviors together with CEL expressions. Always prefer `data.latest("<name>", "<dataName>").attributes.<field>` over the deprecated `model.<name>.resource.<spec>.<instance>.attributes.<field>` pattern.
5. **Verify before destructive operations.** Always `swamp behavior get <name> --json` and verify resource IDs before running delete/stop/destroy methods.
6. **Prefer fan-out methods over loops.** When operating on multiple targets, use a single method that handles all targets internally (factory pattern) rather than looping N separate `swamp behavior run` calls against the same behavior. Multiple parallel calls against the same behavior contend on the per-behavior lock, causing timeouts. A single fan-out method acquires the lock once and produces all outputs in one execution. Check `swamp behavior type describe` for methods that accept filters or produce multiple outputs.
7. **Extension npm deps are bundled; pin them.** Swamp's bundler inlines all npm packages (except zod) into extension bundles at bundle time. When the extension has a `deno.json`, the `deno.lock` beside it decides which versions are bundled; otherwise the registry's current release does. Always pin explicit versions in `npm:` and `jsr:` import specifiers (e.g., `npm:lodash-es@4.17.21`) — `swamp extension push` warns on unpinned and range specifiers.
8. **Reports for reusable data pipelines.** When the task involves building a repeatable pipeline to transform, aggregate, or analyze behavior output (security reports, cost analysis, compliance checks, summaries), create a report extension. Use the `swamp` skill for guidance.
9. **"Workflow" means a swamp workflow.** In this repository the word "workflow" (and "create/run/execute/validate/debug workflow", "automate", "orchestrate", "automated/nightly job") refers to a swamp workflow — a declarative YAML DAG of behavior-method steps authored via `swamp workflow create`. Load and follow the `swamp` skill for these requests. Do NOT interpret these as a request to build an agent task list, spin up worktrees, or schedule a cron/remote agent. Only use those orchestration mechanisms when the user explicitly names one (e.g. "task list", "subagent", "worktree", "cron", "remote agent") or explicitly asks you to do the work yourself step by step rather than author a swamp workflow.
10. **Use swamp, don't bypass it.** Always work through swamp commands — don't go around them with raw shell tools. Use `swamp data query` to find data, not `grep`/`find` on `.swamp/` files. Use behavior methods to interact with resources, not `curl`/`aws`/`gcloud`/`kubectl` when a behavior type already wraps that API — check with `swamp behavior type search`. Use `swamp help` for CLI discovery, not guesswork. Composing with swamp output is fine (e.g. piping `--json` through `jq`) — the anti-pattern is bypassing swamp entirely.
11. **Inspect reports after failures.** When a behavior method or workflow run fails, inspect its generated reports before retrying or changing definitions. Reports run even on failure and capture structured diagnostics — error messages, execution status, arguments, and data output pointers. Use `swamp report get @swamp/method-summary --model <model> --json` for method failures or `swamp report get @swamp/workflow-summary --workflow <workflow> --json` for workflow failures. Run `swamp help report get` to confirm current retrieval syntax.

## Skills

**IMPORTANT:** Always load swamp skills, even when in plan mode. The skills provide
essential context for working with this repository.

- `swamp` - Swamp CLI — behaviors, workflows, data, vaults, extensions, publishing, repos, reports, issues, and troubleshooting
- `swamp-getting-started` - Interactive onboarding for new swamp users

## Getting Started

If you were asked to read a task brief under `tasks/` (a file whose first
line is `<!-- swamp start brief v1 -->`), read that brief, then invoke the
`swamp-getting-started` skill and follow it from its brief entry point.

If the user is new to swamp or asks to get started, or a swamp command you
run shows this repository has no behaviors yet, invoke the
`swamp-getting-started` skill. It checks for existing behaviors itself and walks
new users through an interactive onboarding tutorial.

Otherwise, use the `swamp` skill to work with swamp behaviors.

## Commands

Use `swamp --help` to see available commands. For a machine-readable JSON
schema of the CLI (commands, options, arguments) intended for agent
consumption, run `swamp help [<command>...]` — e.g. `swamp help` returns
the full tree, and `swamp help behavior run` scopes to a subtree.

Behaviors (formerly models) are managed with `swamp behavior` — e.g.
`swamp behavior run <name> <method>` (or
`swamp behavior run @<type> <method> <name>` to run against a type
directly). Every `swamp model …` spelling remains supported.
<!-- END swamp managed section -->

# furulid-finance

Vilberg's household finance setup. The household is Vilberg, Melanie, their kids (Ylva
among them) and some goats, in Kungsäter outside Varberg. One bank account, a Nordea
Personkonto, is tracked in a self-hosted [Actual Budget](https://actualbudget.org/).
This repo holds `jevcat`, the Rust tool that categorizes those transactions, plus the Fly
deploy files and Claude skills for working with the budget.

## The pieces

| Piece | Where | Notes |
|---|---|---|
| Actual server | Fly.io, app name in `.env` | Budget data lives on its volume. Bank sync (Nordea via Enable Banking, GoCardless before 2026-10-11) runs inside it and covers 2026-07-15 onwards. |
| actual-http-api | Fly.io, URL in `jevcat.toml` | REST sidecar over `@actual-app/api`. Everything here talks to it; the key is `ACTUAL_HTTP_API_KEY` in `.env`. Same version as the server, always. |
| jevcat | `src/`, installed as `jevcat` | Imports CSVs and categorizes with TypeSafe Jev. `README.md` explains the commands. |
| Rubric | `rubric.toml` `[descriptions]` | One line per category telling Jev what belongs there. This is the main accuracy lever. Tracked in git; redeploy to the Pi after editing. |
| Skills | `.claude/skills/` | See below. |

`.env` and `jevcat.toml` are gitignored and must stay that way. Never print the API key.
`cargo test` for unit tests; `cargo test preflight -- --ignored --nocapture` checks the
live wiring.

## Skills

- `actual-api`: query or patch the budget with `jevcat api`. Documents the endpoints and
  their quirks.
- `fill-gap`: `jevcat import` for a period bank sync missed, without duplicates.
- `jevcat report --months 6` is where any "how are things" question starts: spending and
  income per month, this month against budget, and recurring charges with stopped,
  doubled and look-alike payees flagged. Budgets come from `#template <kr>` notes on the
  categories (`jevcat budget --list`); `sync` fills the current and next month from them.
- `review-categories`: `jevcat open` to list uncategorized rows with full bank context,
  `jevcat --since-days N run` for a wider Jev pass, `jevcat set` to apply categories with a
  `#reviewed` note, and how to identify payees.
- `update-actual`: bump the server and API images on Fly in lockstep.
- `invest`: investment suggestions sized to the household. Combines `jevcat report` with
  the stock analyst in `../trader` (verdicts with `amount_sek`, and `trader plan`, which
  says how much to invest now and per month, and where). The trader reads the budget with
  `jevcat report --months 6 --json`, run in this directory.
  The Swamp workflow `invest` (`workflows/workflow-invest.yaml`) does the whole round on
  the desktop: runs `sync`, hands `jevcat report --json` to `trader household --file -`,
  makes a plan with `trader plan --json` and asserts its buys fit the money it says is
  there (`--input syncBank=false` skips the bank). The plan's JSON is in the run's data.

`jevcat sync` (bank pull, payee cleanup, categorize, fill budget from templates) runs every Monday 07:30 as a systemd user timer on
a small home server; the same job also exists as the Swamp workflow `sync`
(`workflows/workflow-sync.yaml`: `swamp workflow run sync`, `--input dryRun=true` to write
nothing), which reads the secrets from Swamp's `default` vault (per machine, filled from
`.env` with `swamp vault put default KEY`), asserts the uncategorized share stays under
`maxOpenShare` percent and keeps a one-month report as the run's record. Step timeouts in
`command/shell` are milliseconds. The Pi's service runs that workflow and falls back to plain
`jevcat sync` when swamp cannot start (its API key lives only in `~/.config/swamp/api-key` on
the Pi; the journal says which path ran); its SSH address is `JEVCAT_SYNC_HOST` in `.env` and must not be
written anywhere tracked. Deploy with `deploy/pi/install.sh`, read the log with
`journalctl --user -u jevcat-sync` there. The server carries its own copy of `jevcat.toml`
and `.env`; re-run the install script after changing them. The bank link moved from
GoCardless to Enable Banking on 2026-10-11; the same `banksync` endpoint drives it.

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
- `jevcat payees` (run by `sync`) merges the per-day `Kortköp 260817 DISNEY PLUS` payees
  bank sync creates into one `Disney Plus`, and points reference-number rows at a payee
  taken from notes. Done for the whole history on 2026-10-11. The bank text stays in
  `imported_payee` and `notes`, so rules on those keep working.
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

- Start with `jevcat report --months 6`; it has budgeted, spent and balance per category
  per month, the income side and the recurring charges. Go to `jevcat open --payee X --all`
  or the API only for detail the report does not show. Compare at least three months; a
  single month is noise (Euroblast, Iceland, Christmas).
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
- For anything about investing (what to buy, how much, amortize or not), use the `invest`
  skill: the trader's verdicts and plan are already sized to this budget.
- Do not moralize about hobbies, travel or the kids. Point out patterns (a subscription
  nobody uses, a category that doubled) and leave the judgment to him.
