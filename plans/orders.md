# Order fulfilment: purchase orders, sales orders, estimates

## Goal

The server side of QBD's non-posting orders: how much of a purchase order has
been received or billed, how much of a sales order has been invoiced, how much
of an estimate has been progress-invoiced, which orders are still open, and how
an order (or one line of it) is closed by hand. The web UI builds "Receive
Items against a PO", "Create Invoice from Sales Order / Estimate" and the open
order lists on this.

## Environment / context

- Worktree branch `worktree-agent-a3138d1d8fa081ddd` off `master` at `f2c40f9`.
- Crates touched: `lb-core` (model), `lb-store` (validation, `orders` module,
  migration 4), `lb-server` (routes). Nothing under `web/` or `desktop/` except
  the ts-rs regenerated files in `web/src/api/generated/`.
- Every cargo command through the compute broker:
  `node ~/.claude/bin/cpu-slots.mjs run --slots 4 --label <label> -- cargo …`.

## Decisions already made (don't re-ask)

- **Links live on lines**: `TxnLine.linked_txn_id` / `linked_line_id` (already
  there; deposits use them for payments). No new link type.
- **Legal links** (validated by the store on save):
  - ItemReceipt, Bill, Check, CreditCardCharge line → PurchaseOrder line of the
    same vendor (QBD lets a check or card charge receive against a PO too).
  - Bill line → ItemReceipt line of the same vendor ("Enter Bill for Received
    Items"; it stays a separate transaction here because a transaction can't
    change type). It does **not** count toward the PO: the receipt already did.
  - Invoice, SalesReceipt line → SalesOrder line of the same customer (a job
    rolls up to its customer).
  - Invoice line → Estimate line (progress invoicing; progress is by amount).
- **Same item**: an item line must link to a line with the same item. An
  expense (Account) line links by amount and needs no item match.
- **No quantity cap**: over-receipt is allowed (QBD allows it); remaining
  quantities and amounts floor at zero.
- **Closed** means: `Txn.is_manually_closed`, or every fulfillable line is
  closed (`TxnLine.is_closed` by hand, or fully fulfilled). Text lines and
  zero-amount lines without a quantity are not fulfillable and never block
  closing.
- **`open_balance` of an order = remaining amount**, 0 when closed, so
  `TxnFilter { open_only: true }` lists open orders with no new column. Void
  orders keep `NULL` (as other non-posting types do).
- **New table `txn_link`** (migration 4) mirrors `deposit_link`: one row per
  linked line, indexed by target, so fulfilment is one indexed query rather
  than a JSON scan of every transaction. Rows are written only for non-void
  transactions, so a voided receipt reopens its PO. Deleting an order that has
  links is refused (like deleting a paid invoice); voiding it is allowed.
- **New TS fields are optional in TypeScript** (`is_closed?: boolean`,
  `is_manually_closed?: boolean`, via `#[ts(optional = nullable)]`) so the web
  UI's literal `TxnLine`/`Txn` objects (`web/src/api/blank.ts`) keep compiling
  without touching `web/`. The server always sends them.
- **Edits keep their existing links**: re-saving a receipt whose line already
  links to a since-closed PO line is allowed; only *new* links to a closed line
  or a manually closed order are refused.
- **Links with no `linked_line_id`** (an import that knows only the TxnID)
  attach to the first order line with the same item.

## Plan / steps

1. [x] Read the plans, `txn.rs`, `posting.rs`, `txns.rs`, `schema.rs`, routes.
2. [x] `lb-core`: `TxnLine.is_closed`, `Txn.is_manually_closed`
   (serde-defaulted), `TxnType::is_order`, `TxnType::link_targets`, docs on
   legal links; unit tests (`4a5fc49`).
3. [x] `lb-store`: migration 4 (`txn_link` + backfill, baseline `open_balance`
   on orders); `orders` module (`fulfilment`, `remaining`, link checks, link
   rows, `refresh_all`); `save`/`delete`/`refresh_open_balance` wiring;
   integration tests.
4. [x] `lb-server`: `GET /txns/:id/fulfilment`, `/open?kind=purchase_orders|
   sales_orders|estimates`; API test; `plans/api.md`.
5. [x] fmt, clippy (`--all-targets -D warnings`), `cargo test -p lb-core -p
   lb-store -p lb-server`; commit per step; update `plans/librebooks.md`.

Done on the worktree branch; left for the merge and for other threads:

- Merge the branch; `web/src/api/blank.ts` may then set `is_closed: false` /
  `is_manually_closed: false` explicitly (the fields are optional in TS, so
  nothing breaks without it).
- Web UI: Receive Items against a PO, Create Invoice from a sales order or
  estimate, the open-order lists and the "Clsd"/"Closed" marks (consumes
  `Fulfilment`, `/open?kind=…`).
- [x] Import: QBD's closed marks mapped (an order's `IsManuallyClosed` → the
  stamp, a line's → `is_closed`, a fully received/invoiced line or order →
  closed lines; an inactive estimate → closed). Sales orders aren't imported
  at all yet (`ISSUES.md`).

## Findings / gotchas

- `balances::name_balances` joins `open_balance` to the A/R-A/P account and
  falls back on the form type for `Invoice`/`Bill`/…; orders hit its
  `_ => continue`, so an order's open balance never leaks into a name's A/R or
  A/P balance. `apply.rs` filters by type. Aging reports use postings.
- ts-rs 12: `#[ts(optional)]` needs an `Option<T>`; `#[ts(optional = nullable)]`
  keeps the field's type and adds `?`, so it works on a `bool`.
- `repair_ar_ap_links` only runs at import time, so the order backfill can't
  piggyback on it. Migration 4 sets a baseline in SQL and `Books::open`
  recomputes orders precisely (`orders::refresh_all`) right after upgrading a
  file from a schema below 4.
- The QBD import can't link the receipts and invoices QBD counted against an
  order, so a line QBD reports fully received or invoiced is closed on import
  (`is_closed`), with QBD's `ReceivedQuantity`/`Invoiced` kept in the line's
  `extra`; the fulfilment shown for imported orders is therefore closed-or-open,
  not a running count, until LibreBooks receives against them itself.
- [x] Plan written.
- [x] lb-core model + tests (`4a5fc49`).
- [x] lb-store migration, orders module, save wiring, 5 integration tests
      (PO received in parts / billed from the receipt / closed by hand /
      over-receipt / void reopens / delete refused; SO invoiced by the job's
      customer and closed by its line's "Clsd"; estimate progress-invoiced by
      amount; refusals; older-file upgrade).
- [x] lb-server routes, API test, api.md.
- [x] Checks green (workspace clippy clean, fmt clean, lb-core 72+18,
      lb-store 21+5+32, lb-server 11 api + the rest), commits made, master
      plan updated.

## Open questions for the user

None yet.

## Things not to do

- Don't cap received/invoiced quantities at the ordered quantity.
- Don't hand-edit `web/src/api/generated/`; `cargo test` regenerates it.
- Don't touch anything else under `web/` or `desktop/` from this worktree.
