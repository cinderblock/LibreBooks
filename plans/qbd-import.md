# QuickBooks Desktop import (`crates/lb-qbd`)

## Goal

Read the user's live QuickBooks Desktop (QBD) company into LibreBooks, **read-only**,
incrementally and idempotently, and prove the result with a parity check against
QBD's own balances. Phase 3 of `plans/librebooks.md`.

## Environment / context

- QB Bridge: `http://10.255.0.18:8743` (IPv4 literal), header `X-API-Key`, key in
  `Receipts/.env` as `QB_API_KEY` (read at runtime, never printed or committed).
  Envelope `{ok, data, meta}`; errors are FastAPI-wrapped:
  `{"detail":{"ok":false,"error":{"code","message"}}}` (unknown routes:
  `{"detail":"Not Found"}`).
- QBD is in daily use on `cad-3.tsl`; every bridge request hits the real
  QuickBooks. Keep live probing light, sequential, GET only.
- Probe scratch files live in `data/probe/` (gitignored). Never commit them.

## Decisions already made (don't re-ask)

- **GET only, by construction.** `BridgeClient` has one private request function
  and it is hard-wired to GET; a test scans the crate's sources for any other HTTP
  verb. (User rule: the books stay read-only until QBD is retired.)
- **Source id** is `qbd:<EIN digits>` (fallback: a slug of the legal name). It is
  bound to the company file on first sync (`qbd.source_id` setting); a later sync
  from a different QBD company is refused rather than merged.
- **Order of import** (dependency order, not the bridge's order): accounts
  (parents first) → classes → terms → payment methods → vendors → employees →
  other names → sales reps (they point at a vendor/employee/other name) →
  customers and jobs (they point at sales reps; parents first) → items (parents
  first) → non-payment transactions → payments and bill payments → deposits.
- **Skip unchanged records without fetching their lines**: the list row's
  `EditSequence` is compared with the stored one before the per-record GET.
- **Watermarks** are QBD's own `TimeModified` strings, passed back verbatim as
  `modified_after` (inclusive, so the boundary record is re-listed and skipped by
  `EditSequence`). A watermark never moves past a record that failed or was
  imported with a missing link, so it is retried on the next sync.
- **Payments whose target isn't importable** (e.g. a statement charge, which the
  bridge doesn't expose) are imported without that application rather than
  failing: the ledger is identical either way (applications move no money); the
  gap is reported.

## Plan / steps

1. ▶ Bridge probes (done, findings below) and this plan.
2. Crate skeleton: entities table, `BridgeClient`, `QbdSource` trait.
3. qbXML helpers (`Record`: one-or-many, string scalars, refs, leftovers → `extra`).
4. Normaliser for every exposed entity.
5. Sync engine + `SyncReport`.
6. Parity check + `ParityReport` (ts-rs).
7. qbXML XML → JSON converter; QBWC sketch below.
8. Tests: fixtures, mock bridge (axum), end-to-end sync, incremental, parity.
9. Light live smoke test (a handful of GETs), then hand over for the first full
   import with the user.

## Findings / gotchas (from live probes, 2026-10-19)

- **`active` defaults to `ActiveOnly`** on list endpoints. The importer must send
  `active=All` or inactive accounts/names/items silently vanish.
- **`max_returned` defaults to 100**; without an iterator a list is silently
  truncated. Max is 5000.
- **Iterators are not supported** for Account, Terms, Employee, OtherName,
  TimeTracking (and, presumably, Class, PaymentMethod, SalesRep, PayrollItemWage):
  the bridge answers 400 `ITERATOR_NOT_SUPPORTED`. The client falls back to one
  `max_returned=5000` call; for transaction types it bisects the date range if a
  call comes back full.
- Iterator protocol: `iterator_id=Start`, then the returned `meta.iterator_id`
  (constant across pages) until `meta.remaining == 0`. Re-sending the same
  filters with the iterator id works.
- **`modified_after`** is inclusive (`>=`) and accepts both naive local time and
  QBD's `-08:00` form (same results).
- Not found: 404 `{"detail":{"ok":false,"error":{"code":"NOT_FOUND",...}}}`.
- **Company file has no stable id** in `/company` (no GUID). EIN is the most stable
  field.
- `/company` took 19 s on a cold connection; everything else 0.1–0.4 s. Timeouts
  must be generous (default 120 s).
- **Report basis**: this company's `SummaryReportBasis` preference is **Cash**, so
  reports must ask for `basis=Accrual` explicitly.
- **Account `Balance` semantics**: balance-sheet accounts: the balance as of now,
  natural sign (credit cards and liabilities positive when owed). Income-statement
  accounts: the **all-time cumulative** balance since the company began, natural
  sign. Proven: Σ P&L `Balance` (debit-positive) = trial balance Retained Earnings
  row + Σ P&L trial balance rows, to the cent.
  `Retained Earnings` `Balance` is only its direct postings (0 here).
  `TotalBalance` = own + subaccounts.
- **Trial balance report** is "as of" `to_date` only (`from_date` changes nothing):
  balance-sheet accounts as of the date, P&L accounts fiscal-year-to-date, Retained
  Earnings = prior years' P&L + its own postings. Shape: `{title, subtitle, basis,
  columns:[{id,type,title}], rows:[{type:"DataRow"|"TotalRow", label, values:{"1":
  name, "2": debit, "3": credit}}]}`; rows are only accounts with a balance; labels
  are account full names.
- Bridge GL report rows have no TxnID (from the master plan): totals only.
- **Generic `/items` drops the item type**: records carry no `ItemServiceRet` etc.
  Classify with the typed lists (`/items/service`, `/items/non-inventory`,
  `/items/inventory`) plus shape: `SalesOrPurchase`/`SalesAndPurchase` in neither
  typed list ⇒ Other Charge; `DiscountRate`/`DiscountRatePercent` ⇒ Discount;
  `IsPrintItemsInGroup` ⇒ Group; `TaxRate` ⇒ Sales Tax; `ItemSalesTaxRef` only ⇒
  Sales Tax Group; `DepositToAccountRef`/`PaymentMethodRef` ⇒ Payment; otherwise ⇒
  Subtotal. Subtotal/payment items have no `FullName`.
- QBD's hidden `ReimbursableExpenseGroup` item is a Group with no members
  (LibreBooks refuses memberless groups): skipped and reported; invoice group lines
  that use it become text lines.
- **Line order**: `JournalDebitLine` and `JournalCreditLine` (and item vs group
  lines) arrive in separate keys. `TxnLineID` is `<hex counter>-<timestamp>`; the
  hex counter is company-wide and increasing, so sorting by it restores entry order
  (lines added in a later edit sort last).
- `EditSequence` is a Unix timestamp of the last edit.
- **`AppliedToTxnRet.LinkedTxn` lists *other* transactions linked to the target**,
  not only the credits this payment used: a $500 payment's applied-to invoice
  listed a *different* $1,500 `ReceivePayment` (`Amount: "-1500.00"`). Credits
  (`TxnType` CreditMemo / VendorCredit) found there are therefore de-duplicated:
  the first payment imported that lists a (credit, invoice) pair claims it.
- **No `LinkedTxn` on credit memos or invoices** (the bridge doesn't ask QBD for
  `IncludeLinkedTxns`). A credit memo applied straight to an invoice, without a
  payment, is invisible; the open-balance comparison flags it.
- Voided transactions have no flag: amounts are zero and the memo starts `VOID:`.
- Payroll: `PayrollItemWage` has no LibreBooks list yet (not imported). Employee
  personal fields (birth date, gender, ethnicity, SSN) are deliberately not
  imported; `IsPersonalDataAccessAllowed` is false anyway.
- Volumes (2026-10-19): 160 accounts, 95 customers/jobs, ~370 vendors, 2
  employees, 30 items, 9 payment methods, 0 classes, 0 sales reps;
  checks 6163, deposits 602, invoices 616, payments 498, journal entries 186,
  credit memos 22; **0 bills, 0 bill payments**, 0 estimates, 0 sales receipts,
  0 purchase orders. Sales tax is off (no `SalesTaxPreferences`).
- **First full import cost**: ~8,100 transactions × one detail GET each (lists omit
  lines) ≈ 8,100 requests, ~45 min at the default 100 ms politeness delay.

## Bridge gaps (what the parity check will show)

Not exposed: CreditCardCharge, CreditCardCredit, Transfer, VendorCredit,
ItemReceipt, BillPaymentCreditCard, SalesTaxPaymentCheck, InventoryAdjustment,
Charge (statement charges), SalesOrder, Paycheck, LiabilityCheck, BuildAssembly,
ItemSalesTax/ItemSalesTaxGroup lists, SalesTaxCode list, customer/vendor/job type
lists, price levels, TxnDeleted. This company has two credit card accounts, so
their balances will be off by exactly the missing charges/credits/payments.

## Asks for the bridge (or the user)

1. `include_line_items=true` (and `include_linked_txns=true`) on list endpoints:
   would cut the first full import from ~8,100 requests to ~20.
2. Expose CreditCardCharge, CreditCardCredit, Transfer, VendorCredit,
   BillPaymentCreditCard, SalesTaxPaymentCheck, Paycheck, LiabilityCheck,
   InventoryAdjustment, ItemReceipt, Charge.
3. Expose `TxnDeletedQuery` / `ListDeletedQuery` so deletions are found without a
   full sweep.
4. A stable company-file id in `/company` (e.g. the file path or a GUID the bridge
   stores) so the source binding doesn't hang on EIN.
5. Report the qbXML element name (`ItemServiceRet`…) on generic `/items` rows.

## Progress log

- [x] Probe the bridge (≈55 GETs total so far).
- [ ] Client, normaliser, sync, parity, XML converter, tests.
- [ ] Live smoke test.

## Open questions for the user

(none yet)

## Things not to do

- Never call anything but GET on the bridge.
- Never commit anything from `data/probe/` or any real names/amounts; fixtures are
  synthetic.
- Don't run a full sync of the live company from this crate's work; the parent
  session runs the first full import with the user.
