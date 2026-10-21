# Issues

Side-issues found while working on something else, each just enough to seed a
fresh thread. Claim one by marking it `in progress — <thread/branch>` and commit
that first; remove it (or mark it fixed with the commit) when done.

## QBD import doesn't carry a purchase order's closed marks

**Seen:** `crates/lb-qbd/src/qbxml/txns.rs` `purchase_order` / `sales_lines`
leave QBD's `IsManuallyClosed` (order and line), `IsFullyReceived` and
`ReceivedQuantity` in `extra`; nothing sets `Txn.is_manually_closed` or
`TxnLine.is_closed`, and imported receipts carry no `linked_txn_id` to the PO's
lines (only deposit lines are linked, `txns.rs:632`).

**Why it matters:** since order fulfilment (`plans/orders.md`) an order's
`open_balance` is what remains on it, so every imported PO (and sales order /
estimate) lists as fully open under `open_only` / `GET /open?kind=
purchase_orders`, including ones QBD shows closed or fully received. Mapping
`IsManuallyClosed` → `is_manually_closed`, line `IsManuallyClosed` →
`is_closed`, and treating `IsFullyReceived` / `IsFullyInvoiced` as closed on
import (we can't recompute them without the links) would make the lists match
QBD's.
