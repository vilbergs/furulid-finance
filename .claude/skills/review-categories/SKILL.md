---
name: review-categories
description: Review and fill in categories on Actual Budget transactions - list what is uncategorized with full bank context (`jevcat open`), run jevcat over a wider window, identify unknown payees, apply categories with a #reviewed note (`jevcat set`), add payee rules and extend the jevcat rubric. Use when asked to categorize transactions, check whether categorization is accurate, clear flagged or unsure rows, or figure out what a payment was for.
---

# Review and categorize transactions

Three sources of truth, in order: what Vilberg says, what the categorized history shows
for the same payee, and the rubric in `jevcat.toml`. Jev handles the routine; this skill
is for what it flagged or missed. See `.claude/skills/actual-api/SKILL.md` for the API.

## 1. Let Jev do its pass first

`jevcat run` only looks back `since_days` (90 in the config). Widen it per call:

```sh
jevcat --since-days 520 run --apply --rerun-missed
```

Rows with no payee are skipped by design; `open` shows them with payee `-`. Rows tagged
`#jev-miss` are only retried with `--rerun-missed`.

## 2. List what is left

```sh
jevcat open                                   # last 6 months
jevcat open --since 2025-01-01 --json /tmp/open.json
```

Read the whole row, not just the payee. The bank text column has the second remittance
line for Bankgiro/Autogiro rows (`000029407 Autogiro K*VR Resa` is a Klarna line for a VR
train ticket) and the Swish message (`Kanot`, `2 pers Kolabacken Discgolf`). The type/fx
column shows the original currency, which means travel. Incoming money in the window is
listed to spot reimbursements. The sum and its share of spending are printed; if the
open rows are a few percent, say so and let Vilberg decide how far to go.

## 3. Work out what each row is

Cheap checks first, in this order:

1. **History**: `jevcat open --payee "Sot & Ven"` shows every row whose payee or bank
   text contains that, with its category. Same payee, same category.
2. **Context**: `jevcat open --around 2026-09-26 --days 3` shows the surrounding days.
   Foreign rows in a cluster are one trip; Vilberg goes to Euroblast in Köln every late
   September, and has had trips to Iceland (Christmas), Italy, Linz, Stockholm, Vimmerby.
3. **Receipts**: Apple.com/Bill is mixed (Apple TV 119, HBO Max 189 = Entertainment;
   Apple Arcade 89 = Hobbies; Brilliant 270 = Education; audiobooks = Education; rentals =
   Entertainment). Rules exist for the four subscriptions; anything else needs the receipt
   screenshot, which Vilberg can paste.
4. **Web**: search the exact descriptor plus `allabolag`/`bolagsfakta`. Card descriptors
   are acquirer strings and often abbreviate the owning company, so a miss is common.
   Vilberg's own Google search sometimes finds what WebSearch does not.
5. **Calendar**: Google Calendar `list_events` around the date, and `fullText` on a name,
   tells whether a person is a colleague (Exam.net/Teachiq addresses).
6. **Nordea app / Swish history**: only Vilberg can look; the app shows merchant address
   and Nordea's own category, Swish shows the message. Ask for these before guessing on
   anything over about 500 kr.

Conventions already agreed:

- Swish to colleagues = work lunch, Restaurants/Cafes. Food abroad or on the road =
  Food & Drink. Second-hand tools from a person = Maintenance. Hay = Animals.
- Employer-reimbursed expenses (paid back inside salary) = Income, so they net out.
- Unidentified merchant with Nordea's "Leisure Other" label = Hobbies/Leisure.
- Nintendo and Apple Arcade = Hobbies/Leisure; Medley, Astrid Lindgrens Värld, Leo's =
  Kids Leisure. Festivals and concert tickets, also when Swished to a friend =
  Entertainment.

## 4. Apply

Write a plan and apply it:

```json
[
  {"id": "1d36c863", "category": "Maintenance", "replace_notes": "Second hand tools from Alf"},
  {"id": "ad9e4443", "category": "Restaurants/Cafes", "note": "work lunch", "rule": true}
]
```

```sh
jevcat set /tmp/plan.json --dry-run
jevcat set /tmp/plan.json
```

`id` is the eight-character prefix `open` prints or the full id. Every row gets
`#reviewed`; jevcat tags and guesses are stripped, the bank text stays unless
`replace_notes` is given. Use `"rule": true` only for payees that will recur with the
same category (colleagues, hay, a shop). Never create rules on numeric reference-number
payees; the same number carries different bills. For a payee whose category depends on
the amount, create a rule on `imported_payee contains` + `amount is` through `jevcat api
-X POST rules`, as done for Apple.

## 5. Teach Jev

When a payee was unknown to Jev, add it to the matching line in `[jev.descriptions]` in
`jevcat.toml` so the next one is categorized on import. One clause, naming the payee as
the bank writes it. `jevcat.toml` is gitignored; mention the edit in the report.

## 6. Report

Table of what was set and why, what is still open with the sum and its share of spending,
and which of the open rows are worth Vilberg's time (anything over about 500 kr).
