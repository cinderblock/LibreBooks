# Issues

Side-issues found while working on something else, each just enough to seed a
fresh thread. Claim one by marking it `in progress — <thread/branch>` and commit
that first; remove it (or mark it fixed with the commit) when done.

## QBD import has no SalesOrder entity

**Seen:** `crates/lb-qbd/src/entity.rs` lists Estimate and PurchaseOrder but no
SalesOrder; `qbxml/txns.rs` has no `SalesOrderLineRet` handling, and the bridge
probe (`plans/qbd-import.md`) never listed sales orders.

**Why it matters:** companies with sales orders enabled lose them on import,
and order fulfilment (`plans/orders.md`) can't show invoiced-against-order for
them. Needs the entity, a `SalesOrderQueryRq`, and the same line mapping as
purchase orders (`Invoiced`, `IsManuallyClosed`, `IsFullyInvoiced`), which the
normaliser's `item_line` already reads.
