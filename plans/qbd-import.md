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

1. Bridge probes (done, findings below) and this plan.
2. Crate skeleton: entities table, `BridgeClient`, `QbdSource` trait. (done)
3. qbXML helpers (`Record`: one-or-many, string scalars, refs, leftovers → `extra`). (done)
4. Normaliser for every exposed entity. (done)
5. Sync engine + `SyncReport`. (done)
6. Parity check + `ParityReport` (ts-rs). (done)
7. qbXML XML → JSON converter; QBWC sketch below. (done; SOAP endpoint not built)
8. Tests: fixtures, mock bridge (axum), end-to-end sync, incremental, parity. (done)
9. Light live smoke test (`tests/qbd/live.rs`, ignored by default). (done; see
   progress log)
10. ▶ Hand over: the parent session runs the first full import with the user
    (`sync` with default options into a real company file, then `parity`).

## How to run it

```rust
let key = lb_qbd::client::read_env_value(path_to_env, "QB_API_KEY")?.unwrap();
let client = lb_qbd::BridgeClient::new(lb_qbd::BridgeConfig::new("http://10.255.0.18:8743", key)?)?;
let report = lb_qbd::sync(&books, &client, &lb_qbd::SyncOptions::default(), &|p| { /* progress */ }).await?;
let parity = lb_qbd::parity(&books, &client, &lb_qbd::ParityOptions::default()).await?;
```

`SyncOptions { sweep: true, .. }` finds deletions (lists every id again);
`full: true` ignores watermarks; `reimport: true` re-imports unchanged records
(after the importer improves). `txn_limit` / `txn_from` / `txn_to` make a
partial run that leaves watermarks alone. The UI types (`SyncReport`,
`ParityReport`…) export with ts-rs: add `-p lb-qbd` to `gen:types` in the root
`package.json`.

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
- **`ListID`s are not unique across lists.** In the user's file five Terms and
  five Payment Methods share ids (`80000003-<same timestamp>`…): each list numbers
  from its own counter and lists created together share the timestamp. Every
  lookup must name the list (`extref::get` with a kind); lb-store's
  `get_any_kind` doc claimed otherwise and was corrected. `TxnID`s do come from
  one company-wide counter.
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

## Field mapping (QBD → LibreBooks)

Everything a transaction carries that isn't listed here goes into `Txn.extra` /
`TxnLine.extra` verbatim (`TxnNumber`, `TemplateRef`, `FOB`, `Other`,
`LinkedTxn`…); list fields with no home are counted by name in
`SyncReport.unmapped_fields`. Each imported line keeps its `TxnLineID` in
`extra`, which re-imports use to keep LibreBooks line ids stable.

| QBD | LibreBooks | Notes |
| --- | --- | --- |
| `AccountRet.Name`, `ParentRef`, `Sublevel` | `Account.name`, `parent_id` (full name and level recomputed by the store) | parents imported first (sort by `Sublevel`) |
| `AccountType` | `AccountType::from_qbxml` | unknown type → skipped with reason |
| `AccountNumber`, `Desc`, `BankNumber` | `number`, `description`, `bank_account_number` | |
| `TaxLineInfoRet.TaxLineName` | `tax_line` | `TaxLineID` dropped (same thing as a code) |
| `CashFlowClassification` | `cash_flow_class` (`None` → unset) | |
| `SpecialAccountType` | `role`: AccountsReceivable, AccountsPayable, UndepositedFunds, OpeningBalanceEquity, RetainedEarnings, SalesTaxPayable, InventoryAssets→InventoryAsset, CostOfGoodsSold, PayrollLiabilities, PayrollExpenses, ReconciliationDifferences→ReconciliationDiscrepancies, Estimates, PurchaseOrders, SalesOrders | others noted; the company's own role accounts are adopted, not duplicated |
| `Balance`, `TotalBalance` | (parity only) | |
| Customer/Vendor/Employee/OtherName | `Entity` of that kind | jobs: `ParentRef`; vendors have no `FullName` (use `Name`) |
| `CompanyName`, `Salutation`, `First/Middle/LastName`, `JobTitle`, `Phone`, `AltPhone`, `Mobile`, `Fax`, `Email`, `Cc`, `Contact`, `AltContact`, `Notes`, `AccountNumber`, `CreditLimit` | same-named `Entity` fields (`Cc`→`cc_email`) | |
| `BillAddress` / `VendorAddress` / `EmployeeAddress` / `OtherNameAddress`, `ShipAddress` | `bill_address`, `ship_address` (`Addr1–5`→`lines`) | `…AddressBlock` ignored (derived) |
| `AdditionalContactRef` | Mobile/Main Phone/Alt. Phone/Fax/Main Email/CC Email/Website → fields; others → `custom_fields["Contact: …"]` | |
| `DataExtRet` (custom fields) | `custom_fields[DataExtName]` | |
| `TermsRef`, `SalesRepRef`, `SalesTaxCodeRef` | `terms_id`, `sales_rep_id`, `sales_tax_code_id` | tax codes created on first use |
| `CustomerTypeRef`, `VendorTypeRef`, `JobTypeRef` | `type_id`, `job_type_id` | type lists created on first use (hierarchical) |
| `ResaleNumber` / `VendorTaxIdent` | `tax_id` | |
| `NameOnCheck` (vendor), `PrintAs` (employee) | `print_as` | |
| `IsVendorEligibleFor1099` | `is_1099` | |
| `PrefillAccountRef` (first) | `default_expense_account_id` | |
| `JobStatus`, `JobStartDate`, `JobProjectedEndDate`, `JobEndDate`, `JobDesc` | job fields | |
| `HiredDate`, `ReleasedDate` | `hired_date`, `released_date` | SSN, birth date, gender, ethnicity, citizenship, veteran/disability status, emergency contacts, payroll info deliberately **not** imported |
| Items (generic `/items` + typed lists) | `Item` with `ItemType` | see the classifier in Findings |
| `SalesOrPurchase{Desc, Price/PricePercent, AccountRef}` | `sales_description`, `sales_price`/`is_percent`, `income_account_id` | one-sided item: its one account |
| `SalesAndPurchase{…}` | plus `has_purchase_info`, `purchase_*`, `expense_account_id`, `preferred_vendor_id` | |
| Inventory `COGSAccountRef`, `AssetAccountRef`, `ReorderPoint` | `expense_account_id`, `asset_account_id`, `reorder_point` | on hand / average cost come from LibreBooks' ledger |
| Discount `DiscountRate`/`DiscountRatePercent`, `AccountRef` | `sales_price` (+`is_percent`), `income_account_id` | |
| Group `ItemGroupLine`, `IsPrintItemsInGroup` | `components`, `print_components` | member-less groups skipped |
| Sales tax `TaxRate`, `TaxVendorRef`; group `ItemSalesTaxRef[]` | `tax_rate`, `tax_agency_id`; `components` | not listed by the bridge: created from `ItemSalesTaxRef` + `SalesTaxPercentage` on sales forms, owed to a placeholder agency vendor |
| `StdDueDays`/`StdDiscountDays`/`DiscountPct` | `TermsKind::Standard` | |
| `DayOfMonthDue`/`DueNextMonthDays`/`DiscountDayOfMonth` | `TermsKind::DateDriven` | |
| `PaymentMethodType` | `PaymentMethodKind` | |
| `SalesRep.Initial`, `SalesRepEntityRef` | `initials`, `entity_id` | |
| **Transactions** (all) `TxnDate`, `RefNumber`, `Memo`, `TimeCreated` | `date`, `number`, `memo`, `created_at` | |
| `ClassRef`, `TermsRef`, `DueDate`, `ShipDate`, `PONumber`, `SalesRepRef`, `IsToBePrinted`, `IsToBeEmailed`, `IsPending` | same-named `Txn` fields | |
| Check `AccountRef`, `PayeeEntityRef`, `Address`, `Amount` | `account_id`, `entity_id`, `bill_address`; `Amount` compared | |
| `ExpenseLineRet{AccountRef, Amount, Memo, CustomerRef, ClassRef, SalesTaxCodeRef, BillableStatus}` | `LineKind::Account` | |
| `ItemLineRet{ItemRef, Desc, Quantity, UnitOfMeasure, Cost, Amount, CustomerRef, ClassRef, BillableStatus}` | `LineKind::Item` (`Cost`→`rate`) | no item and no amount → `LineKind::Text` |
| `ItemGroupLineRet` / `…LineGroupRet` | group's line (amount 0, `TotalAmount` in extra) + members | unresolvable group → text line |
| Bill `VendorRef`, `APAccountRef`, `AmountDue`, `OpenAmount` | `entity_id`, `ar_ap_account_id`; compared | |
| Invoice/CreditMemo/SalesReceipt/Estimate `CustomerRef`, `ARAccountRef`, `CustomerMsgRef`, `ItemSalesTaxRef`, `CustomerSalesTaxCodeRef` | `entity_id`, `ar_ap_account_id`, `message`, `sales_tax_item_id`, `customer_tax_code_id` | `Subtotal`+`SalesTaxTotal` (or `TotalAmount`) and `SalesTaxTotal` compared with what LibreBooks computes |
| `…LineRet{Rate|RatePercent, ServiceDate, Other1, Other2}` | `rate` (+`rate_is_percent`), `service_date`, `other1/2` | |
| `BalanceRemaining` / `CreditRemaining` / `OpenAmount` / `UnusedPayment` | compared with the store's `open_balance` at the end of the sync | |
| SalesReceipt `DepositToAccountRef`, `PaymentMethodRef` | `account_id` (absent = Undeposited Funds), `payment_method_id` | |
| PurchaseOrder `VendorRef`, `VendorMsg`, `PurchaseOrderLineRet{Rate}` | `entity_id`, `message`, item lines | non-posting; `TotalAmount` kept as `amount` |
| JournalEntry `JournalDebitLine` / `JournalCreditLine{AccountRef, Amount, Memo, EntityRef, ClassRef, BillableStatus}`, `IsAdjustment` | `LineKind::Journal`, debit +, credit −, in `TxnLineID` order; `is_adjustment` | |
| ReceivePayment `CustomerRef`, `ARAccountRef`, `TotalAmount`, `PaymentMethodRef`, `DepositToAccountRef` | `entity_id`, `ar_ap_account_id`, `amount`, `payment_method_id`, `account_id` | `CreditCardTxnInfo` not imported |
| `AppliedToTxnRet{TxnID, TxnType, Amount, DiscountAmount, DiscountAccountRef, DiscountClassRef}` | `Application{target, amount, discount, discount_account_id, discount_class_id}` | target not importable → reported, payment imported unapplied |
| `AppliedToTxnRet.LinkedTxn` of type CreditMemo (VendorCredit for bill payments) | credit-only `Application{credit_txn_id, credit_amount}` | de-duplicated across payments; other types ignored |
| BillPaymentCheck `PayeeEntityRef`, `APAccountRef`, `BankAccountRef`, `Amount`, `Address` | `entity_id`, `ar_ap_account_id`, `account_id`, `amount`, `bill_address` | |
| Deposit `DepositToAccountRef`, `DepositTotal` | `account_id`; compared | |
| `DepositLineRet{TxnID, TxnType, AccountRef, EntityRef, Memo, CheckNumber, PaymentMethodRef, ClassRef, Amount}` | `LineKind::DepositLine` (`TxnID`→`linked_txn_id`) | unresolvable `TxnID` kept in the line's extra |
| `CashBackInfoRet{AccountRef, Memo, Amount}` | `LineKind::CashBack` | |
| TimeTracking `EntityRef`, `CustomerRef`, `ItemServiceRef`, `Duration`, `Notes`, `BillableStatus`, `PayrollItemWageRef` | `entity_id`; one line: `entity_id`, `item_id`, `quantity` (hours), `description`, `billable`; wage item + raw duration in extra | |
| Memo `VOID:` with zero amounts | `is_void` | QBD has no void flag |
| `CompanyRet` | `CompanyInfo` (name, legal name, addresses, phone, email, website, EIN, fiscal/tax year start, tax form) | |
| `PreferencesRet` | `Preferences` (account numbers, class tracking, JE numbering, estimates, progress invoicing, multi-currency, inventory, auto-apply, report basis, sales tax on/cash) | |

## Web Connector (QBWC) transport sketch

The QuickBooks Web Connector ships free with QBD and polls a SOAP web service;
LibreBooks can be that service, which removes the dependency on the QB Bridge.
The converter (`qbxml::xml`) and normaliser are done; the endpoint is not.

- **`.qwc` file** (the user opens it in QBWC once): `AppName` "LibreBooks",
  `AppURL` `https://<host>/qbwc` (QBWC requires HTTPS except for `localhost`),
  `AppSupport` URL, `UserName`, `OwnerID` and `FileID` (GUIDs LibreBooks mints and
  keeps per company), `QBType` `QBFS`, `Scheduler` `RunEveryNMinutes`,
  `IsReadOnly` **true** (QBWC then asks QBD for read-only access, a second
  guarantee on top of LibreBooks only sending queries).
- **SOAP methods** (namespace `http://developer.intuit.com/`):
  - `serverVersion` → LibreBooks' version; `clientVersion(strVersion)` → `""`
    (or `W:`/`E:` to warn/refuse old QBWC).
  - `authenticate(strUserName, strPassword)` → `[ticket, ""]` (use the company
    file QBD has open) or `[ticket, "none"]` (nothing to do) / `"nvu"` (bad
    credentials). The password is a per-company token LibreBooks generates.
  - `sendRequestXML(ticket, strHCPResponse, strCompanyFileName, qbXMLCountry,
    qbXMLMajorVers, qbXMLMinorVers)` → the next qbXML request: the same queries
    the bridge client makes (`AccountQueryRq` with `ActiveStatus All`,
    `…QueryRq iterator="Start"` with `MaxReturned`, `FromModifiedDate` = the
    watermark, `IncludeLineItems true`, `IncludeLinkedTxns true`, `OwnerID 0` for
    custom fields). The first call's `strHCPResponse` carries `HostQueryRs`,
    `CompanyQueryRs` and `PreferencesQueryRs`.
  - `receiveResponseXML(ticket, response, hresult, message)` → percent done
    (0–100), or negative on error. Parse with `qbxml::xml::parse_response`, feed
    each `…Ret` to the same normaliser, continue the iterator with
    `iterator="Continue" iteratorID=…` while `iteratorRemainingCount > 0`.
  - `getLastError(ticket)` → message for QBWC's log; `connectionError(ticket,
    hresult, message)` → `"done"`; `closeConnection(ticket)` → summary text.
- The sync engine needs one change to use it: QBWC drives the conversation, so
  the engine becomes a state machine fed by `receiveResponseXML` rather than a
  loop calling `QbdSource`. The `QbdSource` trait stays for the bridge.
- What QBWC gains over the bridge: `IncludeLineItems` on queries (one request
  per page instead of one per transaction), every transaction type
  (CreditCardCharge, Transfer, VendorCredit…), `TxnDeletedQuery`, the item
  element names, sales tax items and codes.
- **Never** send `…AddRq`/`…ModRq`/`…DelRq`: the request builder should only be
  able to build `…QueryRq`, with a test like `read_only.rs`.

## Known gaps and limitations

- **Sales tax can't be overridden.** LibreBooks computes sales tax from the tax
  item's rate; QBD stores the amount. If a company's rate changed over time, or
  a code's taxability was guessed wrong (codes come from transactions, taxable
  only if QBD's preferences or the name `Tax` say so), the invoice total differs
  and the sync reports a `SalesTax`/`Total` mismatch. Fix belongs in lb-core: an
  optional stored sales tax amount on `Txn`. (This company has sales tax off.)
- **Inventory COGS** uses LibreBooks' average cost at save time, and transactions
  import type by type, so sales can be costed before the purchases that set the
  cost. Needs an lb-store "repost inventory" pass. (This company has no
  inventory.) COGS per transaction isn't compared: the bridge doesn't give it.
- **Cash-basis parity** waits for lb-reports' cash-basis engine.
- **Per-customer/vendor balance parity** (the bridge lists `Balance` on names)
  isn't done yet; account and trial balance parity are.
- Open balances are compared only for transactions imported in that run.
- lb-store keeps customer, vendor and job types in one namespace; a type name used
  in two of those lists is linked only in the first (reported).
- A deliberately skipped record (QBD's member-less reimbursable group) is listed
  and skipped again on each sync at the watermark boundary (no extra request).
- `PayrollItemWage` isn't imported (no LibreBooks payroll item list).
- The Web Connector SOAP endpoint isn't built (converter and sketch are).

## Progress log

- [x] Probe the bridge (≈58 GETs).
- [x] Client, normaliser, sync, parity, XML converter, tests (60 tests: 34 unit,
      26 integration incl. a mock bridge over HTTP). Clippy pedantic clean.
- [x] Live smoke test, 2026-10-19 (`tests/qbd/live.rs`, 47 GETs, into an
      in-memory company, at most 3 of each transaction type): **0 errors**. Every
      list imported in full (160 accounts with the 5 role accounts adopted, 370
      vendors, 95 customers/jobs, 30 items with the reimbursable group skipped as
      designed, 7 terms, 9 payment methods incl. the ListIDs shared with terms).
      Sampled transactions imported with no total or sales-tax mismatch; the only
      warnings/mismatches were links to records outside the 3-per-type sample
      (payments of invoices not sampled, a deposit of an unsampled payment, open
      balances depending on them), which is what a full import resolves. Unmapped
      list fields seen: `Employee.EmployeeType`, `.KeyEmployee`,
      `.PartOrFullTime`. Parity on the sample is meaningless (most transactions
      not imported) but ran end to end against the live account list and trial
      balance.
- [ ] First full import with the user (parent session): expect ~8,100 detail
      GETs, ~45 min. Then `parity`; the two credit card accounts should be off by
      their unexposed charges/credits/payments.

## Open questions for the user

1. **Ask the bridge for `include_line_items` on lists?** (Recommendation: yes;
   it turns the full import from ~8,100 requests into ~20 and makes routine
   syncs much lighter on QuickBooks.) Same for the missing transaction types,
   which are what will keep parity off.
2. **Run the first full import during off-hours?** Each request touches the
   QuickBooks the user is working in; at the default 100 ms pause it's about 45
   minutes of light, steady load.

## Things not to do

- Never call anything but GET on the bridge.
- Never commit anything from `data/probe/` or any real names/amounts; fixtures are
  synthetic.
- Don't run a full sync of the live company from this crate's work; the parent
  session runs the first full import with the user.
