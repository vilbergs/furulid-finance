---
name: invest
description: Investment suggestions grounded in the household's own budget - how much can be invested now and per month, and in what - by combining the Actual budget (jevcat) with the stock analyst in ../trader (holdings, verdicts, buy candidates, the portfolio plan). Use when asked what to invest in, whether to buy or sell a stock, how much to put into the market or savings, whether to amortize instead, or how the portfolio fits the household's economy.
---

# Investment suggestions from the budget and the trader

Two systems, one answer:

- **Budget** (this repo): `jevcat report --months 6` is the household's cash flow;
  `jevcat report --months 6 --json` is the same as one JSON document in whole kronor
  (income, spending, net to savings, surplus, uncategorized share, recurring charges, the
  Nordea account's balance in Actual).
- **Trader** (`../trader`, a Rust CLI and the dashboard at trade.vilberg.dev on this
  desktop): Avanza market data, Claude verdicts per stock (buy/add/hold/trim/sell, scored
  against OMXS30), and a **plan** that sizes the investing to the household. It reads the
  budget by running `jevcat report --months 6 --json` in this directory
  (`TRADER_HOUSEHOLD_DIR` in `~/.config/trader.env`), daily, before each plan, and each
  verdict carries `amount_sek` and `sizing_note` from it.

The trader needs its env for the CLI: run it as

```sh
(set -a; . ~/.config/trader.env; set +a; trader <command>)
```

## Steps

1. **Budget first.** `jevcat report --months 6`. Note the uncategorized share (say it
   first, as for any advice), the average surplus, and which income moved (Föräldrapenning
   ending, a salary month missing). A negative surplus means the answer starts there, not
   with stocks.
2. **What the trader sees.** `trader household` refreshes the snapshot and prints the
   block the verdicts get: cash, buffer target (three months of spending), investable now
   and per month. Cash is only the Nordea account in Actual plus `reserve_outside_budget`.
   If Vilberg has savings elsewhere (a savings account, the reserve abroad, Avanza cash),
   ask for the amount and set it with `trader household --cached --reserve <kr>`; without
   it the plan treats the household as having no buffer.
3. **Holdings.** `trader hold ls`. Holdings are entered by hand (`trader hold set <id>
   <shares> <avg price>`); if what he owns on Avanza is missing, the plan cannot size
   trims or weights. Ask, don't guess.
4. **The plan.** `trader plan --last` shows the latest (remade Mondays after the evening
   verdicts); `trader plan` makes a new one (one Opus call, about a minute). It returns
   invest now, invest per month, the buffer status, actions per stock with kronor,
   better uses of the money (`instead`), risks, and when to revisit.
5. **A specific stock.** `trader show` lists the latest verdicts; `trader analyze <id>`
   makes a fresh one with sizing. `trader score` shows how good the analyst has been:
   few scored verdicts means its buys are unproven, and the amounts should be small.

## Answering

- Lead with what the budget allows, then what to do with it. One short table: action,
  stock or alternative, kronor now, kronor per month, why.
- Weigh alternatives the plan names: rebuilding the buffer, amortizing the Nordea loans
  (compare the loan rate with the policy rate in the plan's inputs), a broad index fund
  for a monthly amount. Ylva's Avanza savings (730 kr/month) are hers; leave them out of
  what is investable.
- The trader is advisory and places no orders. Say what to do; Vilberg places it.
- Kronor, not bare percentages. No moralizing about spending; point out a pattern only
  when it changes what can be invested, and name the category and amount.
