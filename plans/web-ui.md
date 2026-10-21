# LibreBooks web UI (phases A, B, C, D and E)

Living plan for the SPA under `web/`. Master plan: `plans/librebooks.md`
(decisions 7 and 13). HTTP contract: `plans/api.md`. Auth library:
`plans/auth.md`. Server operations: `plans/server-ops.md`.

## Goal

Phase A built the foundation before the server existed: the app shell, the
input system (keyboard + input components) and the data layer, plus a
**fixture mode** (an in-memory fake API seeded with a synthetic company) so
every screen renders and can be exercised with `bun run dev`.

Phase B built the rest of QuickBooks Desktop's everyday surface on top of it:
the generated report types and the Report Center, every transaction form on
the Write Checks template, register editing in place, Reconcile, Bank Feeds,
and the administrative and account screens (users, enrolment links, My
Account, server settings, backups, QuickBooks import, closing date, memorized
transactions, attachments, merge).

Phase C (2026-10-20) took up what the server pass of the same day enabled: the
generated QuickBooks import and `NameBalance` types replace the UI's local
copies, the centres read `GET /balances/names`, Attachments gained the Attached
tab on `GET /attachments?all=true`, and the fixture answers 423 for the closing
date as the server does. A follow-up the same day took the server's generated
import envelopes (`ImportStatus`, `RunningJob`, `JobKind`, `SyncBody`,
`ParityBody`, `ParityStatus`) and made the Import window follow a job's
`import` change events, with the status long-poll answering on every progress
step.

Phase D (2026-10-20) took up four of the "What's left" items: full editors
for every list (accounts, customers and jobs, vendors, employees, other names,
every item type, classes, terms, payment methods, sales tax codes) as windows
on a record-form engine beside the transaction one; attachments on names,
accounts and items; New company and Open a .lbk file from the company picker;
and the Import window's Options disclosures for the rest of `SyncBody` and
`ParityBody`.

A report export follow-up (2026-10-20) took up the server's XLSX and PDF
export (`plans/reports.md`, "Export: CSV, XLSX, PDF"): the report window's
Export menu offers Excel, PDF (Letter or A4) and CSV, Ctrl+E repeats the last
pick, Print offers Save as PDF, and a phone reaches it all from one overflow
menu.

Phase E (2026-10-20) took up "What's left" item 3 bar payroll: the forms that
draw on one another. Statement Charges (a customer's A/R register with a
new-charge line), Sales Orders (non-posting, invoiced whole or in part),
progress invoicing from an estimate (100%, a percentage, or per-line amounts)
and Receive Items / Enter Bills against a purchase order (Select PO, partial
receipts). Every drawn line carries `linked_txn_id` / `linked_line_id` back to
the order line it fulfils, and the invoiced / received / remaining figures,
the closing of an order and the open-order lists are all derived from those
links on the client (`lib/orders.ts`), because the server keeps nothing else
(see "For the server", phase E).

User requirements this meets: mobile first class from day one; QuickBooks-quality
input (QBD keyboard shortcuts, QuickFill, date keys, the amount calculator);
reports and views. The Tauri desktop app loads this same SPA, so the keymap
differs between the desktop app (real QBD keys) and a browser (where
Ctrl+W/T/N/Tab can't be captured).

## Environment / context

- Phase A was built on worktree branch `worktree-agent-ab818e840111ec792`,
  phase B on `worktree-agent-af93b28647e2ebef7` (both merged by the parent
  session), phase C on `worktree-agent-a9571472f4dda8acb`, its import
  follow-up on `worktree-agent-ab9ab2d43381969c5`, phase D on
  `worktree-agent-ab9aa5ed8179d8f95`, the report export follow-up on
  `worktree-agent-a3a9fec5729e41528`, and phase E on
  `worktree-agent-a19a3c5986ee129af`. The parent merges `master` in; the
  worktree never does.
- Vite 8, React 19 (StrictMode), Mantine 9.7 (has `Menubar`), React Router 8.5
  (v7-style API from `react-router`), TanStack Query 5, TanStack Virtual 3,
  TypeScript 7 (`tsc`), Bun 1.4 (package manager + unit tests), Playwright 1.64,
  `@simplewebauthn/browser` 14, `qrcode.react` 4.
- Commands, from `web/` (wrap heavy ones in the CPU broker:
  `node ~/.claude/bin/cpu-slots.mjs run --slots N --label "librebooks web" -- …`;
  1 slot for check/tests, 2 for `vite build` and for Playwright's 2 workers):
  - `bun run dev` → http://localhost:5740. Add `?fixtures=1` (or
    `?fixtures=stress`, `&latency=0`, `&keymap=desktop|browser`).
  - `bun run check` = `tsc --noEmit && bun test src/` (269 unit tests, 25 files).
  - `bun run test:e2e` (Playwright; starts the dev server with `VITE_FIXTURES=1`;
    one-time `bunx playwright install chromium`). 55 smoke tests: 38 desktop
    (1366×860) and 17 at 375px; the suite takes ~2.5 min.
  - `bun run build`.
- Generated API types live in `web/src/api/generated/` (ts-rs). Never edit them.
- Windows: the Write tool produces LF; before a commit, normalise changed files
  to CRLF (a `node -e` one-liner replacing `\r?\n` with `\r\n`). The worktree
  guard rejects shell loops, heredocs and `$(…)` around `git`; use plain
  commands and several `-m` for commit messages.

## Decisions already made (don't re-ask)

- Only `web/` and this file are touched by this work. (The root README's
  "Building" section could gain the web commands above; left to the parent.)
- Money is integer cents (`number`). Rates/quantities are decimal strings; exact
  decimal math is BigInt-scaled integers (`lib/decimal.ts`); the calculator uses
  BigInt rationals. Rounding half away from zero, once, as Rust/QBD do.
- Fixture mode: `?fixtures=1` (sticky for the tab via `sessionStorage`; `=0`
  turns it off) or `VITE_FIXTURES=1`. In memory only; reload resets. Loaded by
  dynamic import, so production pages don't download it.
- Windows are kept mounted while hidden (switching never loses unsaved edits);
  the URL names the active window.
- Report windows are keyed by subject (report + account/name); changing dates or
  basis modifies the open report in place, as QBD does.
- Enter in line grids defaults to "next field" (QBD's "Enter moves between
  fields" preference); a per-device preference switches to next line or save.
- Ctrl+A and Ctrl+Y keep their text-field meanings (select all, redo) inside a
  field; elsewhere they are QBD's Chart of Accounts / Transaction Journal.
- Totals on sales forms are the server's: the form shows a live
  `POST /txns/preview` (debounced 400 ms) and never computes sales tax itself.
- Estimates and purchase orders are non-posting and the forms say so.
- "Cleared" means reconciled only. A bank-feed match or add sets the
  `downloaded` mark (QBD's lightning bolt) instead.
- Prefilled forms (Enter Now on a memorized transaction, "Add more details" on
  a bank-feed line) hand a `Txn` over through `windows/handoff.ts`: a one-shot
  token in `?draft=` that is part of the window key. A reload shows an empty
  form rather than re-entering something that may already be saved.
- Secrets are write-only everywhere (server secrets, the tunnel token, feed
  credentials, the bridge key, the Web Connector password): a field to set or
  replace, a "set" badge, never a value back.
- Restoring a backup makes a **new** company; nothing is replaced in place.
- Admin-only menu items are hidden from non-admins (`MenuItem.admin`) and the
  windows carry an `AdminGate`; the server refuses anyway.
- List record editors are **windows** (`/lists/:list/new`, `/lists/:list/:id`),
  not modals, though QBD's are dialogs: a half-edited customer survives a
  switch to the register, the URL names it, and the same keys (Alt+S / Alt+A,
  Ctrl+D, Esc) work as on a transaction form. QuickCreate stays the quick add
  behind QuickFill's `<Add New>` and the phone's "+" sheet.
- An account's opening balance is a general journal entry against the
  `OpeningBalanceEquity` role account, posted after the account saves (what
  QBD does under the hood; the API's `Account` has no balance field).
- A new company from the picker (`POST /companies`) or a `.lbk`
  (`POST /companies/import`) is always a NEW company, like a restore; the
  picker refetches the session and opens it. In fixture mode such companies
  share the demo books and differ only in their information.
- **Orders are derived, never stored** (phase E). What has been invoiced
  from a sales order or estimate, or received against a purchase order, is
  read back from the lines of the documents that link to it
  (`TxnLine.linked_txn_id` / `linked_line_id`); remaining = ordered − Σ linked,
  per line, in exact decimals (`lib/orders.ts`). An order is closed when every
  line is fully drawn (sales and purchase orders by quantity, estimates by
  amount) or when closed by hand, which is kept in `Txn.extra` under QBD's own
  tag `IsManuallyClosed` (the server stores `extra` verbatim; an import that
  carried the tag would land in the same place). Nothing else is invented on
  the client: the UI reads no `open_balance` for an order, so it works the
  same against the real server as against the fixture.
- Statement charges are entered in a customer's A/R register (QBD's Enter
  Statement Charges), one item line per charge, no sales tax; a saved charge
  opens in the sales form as a `StatementCharge` for editing, voiding and
  deleting. Sales orders are a sales form too (non-posting, like estimates).
- Drawing on an order makes a NEW, unsaved document handed to its form
  through `windows/handoff.ts` (as memorized transactions are): the person
  sees the invoice, receipt or bill before anything is saved, and the number,
  terms and dates are the form's.

## Architecture

```
web/src/
  main.tsx, App.tsx      boot: create the API client, providers, router
  lib/                   pure logic, unit-tested
    money, decimal, calc     cents, exact decimals, QBD calculator
    dates                    ISO dates, period.rs port (date keys, macros), typed-date parser, relative times
    format                   byte sizes, ISO 8601 durations (PT6H ↔ minutes)
    quickfill, tree          ranked hierarchical matching; Parent:Child helpers
    accountTypes, txnTypes   mirrors of lb-core facts (labels, classes, register columns)
    orders                   what an order's linked documents have drawn: remaining quantities and amounts, closed, IsManuallyClosed
    platform, prefs, useMobile
  api/                   the data layer
    client.ts                ApiClient interface (the one thing screens talk to)
    http.ts                  fetch client: cookies, X-LibreBooks, bearer token, multipart, downloads, SSE w/ retry
    errors.ts                ApiError, posting-error text, error → form field mapping
    json.ts                  JSON parse/stringify helpers (no BigInt: see Findings)
    types.ts, blank.ts       re-exports of generated types + local stand-ins; blank records
    keys.ts, hooks.ts        TanStack Query keys (qk per company, sk server-level), hooks per resource, invalidation per event
    context.tsx              ApiProvider, CompanyProvider, SSE subscription, change bus
    select.ts                fixture vs HTTP; desktop injection; page singleton
    fixtures/                FIXTURE ONLY: seed company, posting mirror, fake server, reports, feeds, admin
  keyboard/              commands, platform keymaps, scoped registry (pure), React provider
  windows/               open-window list (reducer), URL ↔ window, provider, catalog, hand-off stash
  shell/                 desktop + mobile shells, menus (with admin gating), palette, overlay, user menu
  inputs/                DateField, MoneyField, QuickFill (+ pickers, QuickCreate), LineGrid, RegisterGrid
  forms/                 the form engine (useTxnForm) and every form with its pure draft model;
                         the record engine (useRecordForm) and records/ (the list editors, AddressFields);
                         linked.ts (drawing on an order), CreateFromOrder (the dialog), OrderProgress (the panel),
                         statementCharge.ts (the register's new-charge line)
  reports/               request ↔ URL params, row outline + CSV, ReportViewer
  screens/               Home, Chart of Accounts, centres, Register, StatementCharges, Reconcile, Reports, Find,
                         Lists, RecordWindow, Company (+ CompanyInfoFields), Banking, feeds/, Memorized,
                         Attachments, Merge, MyAccount, admin/
  auth/                  SignIn (/signin), Enrol (/enroll#token=…)
  components/KeyList     keyboard-driven virtualized list used by list windows
```

Data flow: screens → hooks (`api/hooks.ts`) → `ApiClient`. Writes invalidate what
they touch; server change events (`/events?company=`) invalidate by `kind` and
are re-published on an in-app bus so an open form can react to its own record
changing. Known kinds: `txn`, the list kinds, `company`/`info`, `feed*`,
`saved`/`memorized`, `attachment`, `backup`, `import` (a job's `started` /
`progress` / `finished` / `failed` with `id` sync|parity: the import queries
are invalidated without cancelling a long-poll in flight, and a sync's
`finished` or `failed` also invalidates everything derived from transactions),
`user`/`enrolment`, `server`. Unknown kinds invalidate the whole company
(correct, heavier).

## Component inventory

| Component | What it does |
| --- | --- |
| `DateField` | Typed dates (`10/19`, `10-19-26`, `101926`, `19`, `t`); QBD keys `+ - T W K M H Y R [ ]`; Alt+↓ calendar; non-blocking past/future warning (`useDateWarning`). |
| `MoneyField` | Cents in/out; `1,234.56`, `$5`, `(12.00)`; calculator `12*3+4` then `=`/Enter/Tab with a live tape; `+`/`-`/`*`/`/` on a filled field adjusts it; Esc undoes. |
| `QuickFill` | Type-ahead: inline completion on prefix matches, any-colon-level/word/key matching, indented tree with Type/Balance columns, `<Add New>` + not-in-list quick add, inactive filtering, ↓↑ PgUp/PgDn Enter Tab Esc Alt+↓ Ctrl+L, virtualized. |
| `AccountQuickFill`, `NameQuickFill`, `ItemQuickFill`, `ClassQuickFill` | QuickFill bound to the lists (memoized options). |
| `QuickCreateProvider` | The modal behind `<Add New>`: account, name (incl. job of), item. |
| `LineGrid` | Form line grid: Tab/arrows/Enter navigation, Ctrl+Insert/Delete, one trailing blank line, per-cell errors, footer totals; card-per-line on phones. |
| `RegisterGrid` | QBD two-line register, virtualized (tested at 100k lines), keyboard selection, cleared toggle (optimistic), the ⚡ downloaded mark, inline new-transaction line, **edit in place** (Ctrl+E / double-click; Esc cancels; stale `edit_seq` → conflict dialog), 1-Line mode; tappable list on phones. |
| `ReportViewer` | Any `Report`: sticky header and label column, outline collapse showing subtotals, QuickZoom drill (txn / report / register), keyboard, virtualized. |
| `ReportCenter`, `ReportWindow` | `GET /reports` catalog by QBD category with memorized reports; a report window with the options bar (date macro / from–to, basis, columns, comparisons, show zero), filters modal, an **Export menu** (Excel, PDF on Letter or A4 with the paper remembered, CSV: `POST /reports/export?format=xlsx\|pdf\|csv&paper=`; Ctrl+E repeats the last pick; the server's file name from `Content-Disposition`; the on-screen CSV as the fallback for CSV only), Print as a split button with Save as PDF, one "More actions" overflow menu at 375px, Memorize (`POST /saved/memorized_report`), drill-down from `Drill`; `aria-busy` while a changed request loads behind the kept report. |
| `KeyList` | List windows: virtualized, ↑↓ PgUp PgDn Home End Enter, type-to-jump. |
| `Center` | Customer, Vendor and Employee Centers: the names with jobs indented, **Balance** (the name's own open items) and **Balance Total** (with its jobs) from `GET /balances/names`, the detail with contact, bill-to, balance card and transactions (statement charges among them, with their open amount), edit, New transaction menu (Customers: Invoice, Receive Payment, Sales Receipt, Estimate, Sales Order, Statement Charge for this name), Ctrl+Q QuickReport. |
| `useTxnForm` / `TxnFormActions` | The form engine (see below) and the standard action bar: Delete, Void, Journal, Memorize, paperclip (Attachments for this transaction), Revert/Clear, Save & New, Save & Close. |
| `useRecordForm` / `RecordFormActions` / `RecordLoader` | The list-record engine, the same shape as the transaction one and sharing its pieces (`ConflictModal`, `RemoteChangeBanner`, `FormActionBar`, `FormErrors`): a pure `RecordFormSpec` (draft ↔ record, validate, fingerprint, server-error mapping, next draft), Save & New / Save & Close, Revert/Clear, Delete (Ctrl+D) with the server's `in_use` refusal on the form, stale `edit_seq` → the conflict dialog, a change pushed while editing → banner or silent reload, Ctrl+N a fresh editor, Enter moving between fields (`enterProps`), the paperclip for `entity`/`account`/`item`. |
| `RecordWindow` | `/lists/:list/new` (`?kind=`, `?type=`, `?parent=` presets) and `/lists/:list/:id`: loads the record and renders its editor. Titles "New Customer", "Edit Item: Walnut". |
| `AccountEditor` | Type (fixed for role accounts), name, number, Subaccount of (same type, never inside itself), description, bank account number, tax line, cash-flow class; a new balance-sheet account's opening balance as of a date (a journal entry against Opening Balance Equity after the save); inactive. |
| `NameEditor` | Customers and jobs (Job of), vendors, employees, other names on QBD's tabs: Address Info (names, contact details, bill-to and ship-to with Copy), Payment Settings (account no., terms, credit limit, price level, sales rep, tax code; vendors: print-as, tax id, 1099, account prefill), Additional Info (type, notes, custom fields), Job Info (status, type, dates, description), Employment (hire/release). Add Job from the Customer Center presets the parent. |
| `ItemEditor` | Every type the generated `Item` carries: service / non-inventory / other charge (two-sided on request: purchase and sale sides), inventory (COGS, income, asset, reorder point, on hand), subtotal, group and sales tax group (members in a `LineGrid`), discount and other charge as amount or percent, payment (Undeposited Funds or an account), sales tax item (rate, agency). Type fixed once saved; subitem of the same type. |
| `ClassEditor`, `TermsEditor`, `PaymentMethodEditor`, `SalesTaxCodeEditor` | Subclass of; standard (net days, discount % within days) or date-driven (day of month, cutoff, discount by day) terms; payment type; a ≤3-character code, taxable or not. |
| `ListWindow` | Items, classes, terms, payment methods, sales tax codes, other names: Find, Include inactive, New / Edit / Delete, Ctrl+N / Ctrl+E (or Enter) / Ctrl+D (the server's `in_use` shown), Ctrl+Q where there's a QuickReport. The Chart of Accounts and the Centers open the same editors (Edit, New, Add job). |
| `CompanyPicker` | `/` (one company opens by itself) and `/companies` (always the list; the user menu's "Open another company", the phone's More → Companies). Admins: New company (`CompanyInfoFields` → `POST /companies`) and Open a .lbk file (`POST /companies/import`, multipart file + name), each a NEW company opened at once after the session refetches. |
| `PurchaseForm` | Write Checks, Enter Bills, Vendor Credits, Credit Card Charges/Credits, Item Receipts, Purchase Orders (non-posting, says so): Expenses/Items tabs, terms → due/discount dates, recall of the payee's last transaction. **Receive Items and Enter Bills against a PO** (phase E): a vendor with open purchase orders gets a notice and a **Select PO** button (an `OrderPicker` of the open ones with what remains on each); the chosen orders' remaining lines land on the Items tab linked to their PO lines, quantities editable for a partial receipt. A saved PO shows the `OrderProgress` panel (ordered / received / remaining per line, the receipts and bills drawing on it, Open / Closed, Mark as closed / Reopen, Create Item Receipt / Enter Bill). |
| `SalesForm` | Invoices, Sales Receipts, Credit Memos, Estimates and **Sales Orders** (both non-posting), **Statement Charges** (a saved charge's editor): item lines with qty × rate, sales tax item and customer tax code, live server-confirmed totals (`PostingsPanel`) on the posting kinds. Phase E: a saved sales order or estimate shows `OrderProgress` (invoiced / remaining per line, the invoices drawing on it, progress to date in percent for an estimate) with **Create Invoice** → `CreateFromOrder`; a new invoice for a customer with open sales orders offers **Invoice from a sales order…**; an invoice made from an order says "From Sales Order N" with a link to it, and its lines carry `linked_txn_id` / `linked_line_id`. |
| `CreateFromOrder` | The dialog behind Create Invoice / Create Item Receipt / Enter Bill on an order. Sales and purchase orders: everything that remains, or selected items and quantities (Ordered, Invoiced or Received, Remaining, To invoice / To receive per line). Estimates, QBD's three: the entire estimate (100%), a percentage of the whole, or selected items with an amount or a percent per line (Est Amt, Prior Amt, Prior %, Amount, Curr %), the total and its percent of the estimate at the foot. Makes a new unsaved document (`forms/linked.ts: documentFromOrder`) and hands it to the form. |
| `OrderProgress` | Under a saved order: status (Open, Closed: fully invoiced / received, Closed by hand), per-line Ordered / Invoiced or Received / Remaining (amounts and percent for an estimate), the linked documents with a way into each, Mark as closed / Reopen (sales and purchase orders; `IsManuallyClosed` in `extra`), and the Create… button. Disabled while the form has unsaved edits (save first). |
| `OrderPicker` | A name's open orders of a kind (Select PO; Available Sales Orders): number, date, memo, amount and what remains; pick one or several. |
| `StatementCharges` | Customers → Enter Statement Charges: a customer:job and its A/R register (invoices, charges, credits, payments; Amt Chrg / Amt Paid / Open / running Balance; ↑↓, Enter or a click QuickZooms into the transaction) with the new-charge line at the foot (Date, Number, Item, Qty, Rate, Amount, Class, Due date from the customer's terms, Description) that records a `StatementCharge` with one item line and no sales tax. Reached from the menu, the palette, the Customer Center's New transaction menu (preset to the name) and a phone's More. Stacked fields and cards at 375px. |
| `PaymentForm`, `PayBills` | Receive Payments (open invoices from `GET /open`, auto-apply, credits, Undeposited Funds or a bank account) and Pay Bills (open bills from `GET /txns?types=Bill&open_only`, discounts/credits, check or card). |
| `DepositForm`, `TransferForm`, `JournalForm` | Make Deposits (payments waiting in Undeposited Funds from `GET /undeposited`, cash back), Transfer Funds, General Journal (debit/credit columns, must balance, auto-numbered). |
| `MemorizeModal` | Ctrl+M: name, Never / Remind / Automatic, interval, next date, days in advance, remaining, group → `POST /saved/memorized_txn`. |
| `Reconcile` | Begin (statement date, ending balance, service charge, interest) then the two-column window: Mark All, running difference, Enter Adjustment, Discard, Undo Last; one column at a time on phones. |
| `BankFeeds` | Accounts summary, the download queue with Match (scored candidates) / Add (preview) / Ignore / Undo, rules (conditions → actions, reorder, apply), connections (file, SimpleFIN claim, Direct Connect, credentials write-only), file import with OFX detection and CSV mapping/preview, Amazon order matching. |
| `MemorizedList` | Ctrl+T: every memorized transaction with its schedule, due ones first; Enter Now (`GET /memorized/:id/use` → hand-off to its form), edit schedule, delete. |
| `Attachments` | Inbox and Attached tabs (the latter from `GET /attachments?all=true`, showing what each document is attached to with a way into the transaction or the name's / account's / item's editor) and one record's files (any `record_kind`: the title and copy name the record); Add files, Take a photo (`<input capture="environment">`), image thumbnails, attach to a transaction (search by payee/memo/number) or to a name, account or item (QuickFill), detach, delete. Reached from the paperclip on any saved form or editor, and from the Centers' detail. |
| `MergeWindow` | Names / accounts / items: pick what disappears and what it folds into; confirm; `POST /merge`. |
| `Company` | Company information with an edit dialog (`PUT /info`, incl. the main preferences), Set Closing Date (`PUT /closing-date`), the administration grid (admins), this device's preferences. |
| `MyAccount` | Passkeys (rename, remove), device keys (revoke), where I'm signed in (sign another device out), Add another device (an `add_device` link with QR), Add a passkey here (mints a link and follows it). |
| `UsersWindow` | Users with role and status; a detail drawer: rename, server role, disable/enable, access to this company (grant/revoke with a role), passkeys and devices, add-device link, delete. |
| `EnrollmentsWindow` | Outstanding links (purpose, role, who for, expiry, revoke) and `MintLinkModal`: new user (role, suggested name, company access, TTL) or add device (whose); the minted link shown once as text + `QRCodeSVG`. |
| `ServerWindow` | General (public URLs, feed auto-sync, Cloudflare Access), Updates (status, check, install with/without restart, policy), Tunnel (state, logs, token write-only, hostname, cloudflared), Backups (schedule, destinations folder/S3, encryption passphrase/age/none, retention, per-company status), Secrets (list, add, replace, delete; `in_use` refused). `PUT /server` sends the document whole; 400 `invalid` lists `detail.problems`. |
| `BackupsWindow` | This company's backups in every destination, Back up now, Verify (checksum; contents with the passphrase/identities), Restore as a new company, Export company file. |
| `ImportWindow` | QuickBooks import on one generated `ImportStatus` document (`GET /import/qbd`; while a job runs, `…/status?wait=` is long-polled and answers on every progress step, so the bar is live; the job's `import` change events refetch it and, on a sync's `finished`/`failed`, the books, with a notification when a job ends): Sync (now / sweep deletions / full; the running job with progress, or "waiting for the Web Connector"), the last `SyncReport` (per-entity counts, issues, mismatches, gaps, unmapped fields, created lists); Connection (the generated `ImportSettings`: bridge URL + write-only key + delay, Web Connector user name / schedule / app URL / password (set or generated once) / .qwc download, SDK file mode, company file), `400 invalid` listing `detail.problems`; Parity (`GET …/parity`: the report, QBD beside ours, differences first; a running comparison polls). **Options** disclosures (phase D) hold the rest of `SyncBody` (transaction limit, date range, only some kinds of record, apply credits as server default / yes / no, re-import, full, sweep) behind their own "Sync with these options" button, and `ParityBody` (balances as of, trial balance) for Compare now; only what was set goes over the wire. |
| Shell | Desktop: menu bar with shortcut hints, icon rail, window tabs with unsaved dots, palette button, live indicator, fixture badge, theme, user menu (My Account, Sign out). Phone: header with Back/search, stacked windows, bottom tabs with "+" create sheet and More sheet (incl. My Account). |
| `CommandPalette` | Ctrl+K: forms, reports, accounts, names, items, open windows; QuickFill ranking; grouped; admin items only for admins. |
| `ShortcutOverlay` | `?`/F1: every key on this platform by scope, the ones live in this window highlighted, plus field keys; keymap switch. |
| `SignIn`, `Enrol` | Passkey sign-in (button + conditional UI autofill); enrolment from `#token=` (stripped from the URL), name asked before the passkey prompt, same-credential retry if the server wants a name. |

The form engine (`forms/useTxnForm.tsx`, Write Checks was the template): a
form supplies a pure `TxnFormSpec<D>` (new draft, `fromTxn`/`toTxn`, validate,
fingerprint for dirtiness, server-error mapping, the next draft after Save &
New, title, siblings for Previous/Next) and renders its fields; the engine does
the rest. Client validation, then server errors (posting / closed period /
invalid / duplicate) mapped onto header fields and grid cells. Save & New
(Alt+S), Save & Close (Alt+A), Revert/Clear, Delete (Ctrl+D), Void, Journal
(Ctrl+Y, server preview), History (Ctrl+H), Memorize (Ctrl+M), Print (Ctrl+P),
Previous/Next (PgUp/PgDn). Stale `edit_seq` → "Someone else changed this"
(Reload theirs / Save mine anyway / Keep editing); a change pushed while editing
shows a banner, and reloads silently when there are no local edits.

## Keymap (generated from `keyboard/keymaps.ts`)

| Command | Scope | Desktop app (QBD) | Browser (Windows) | Browser (Linux) | Browser (macOS) |
| --- | --- | --- | --- | --- | --- |
| Search / go to anything | global | Ctrl+K | Ctrl+K | Ctrl+K | ⌘K |
| Keyboard shortcuts | global | F1, ? | F1, ? | F1, ? | F1, ? |
| Write Checks | global | Ctrl+W | Alt+Shift+W | Alt+Shift+W | ⌥⇧W |
| Create Invoices | global | Ctrl+I | Ctrl+I | Ctrl+I | ⌘I |
| Customer Center | global | Ctrl+J | Ctrl+J | Ctrl+J | ⌘J |
| Use Register | global | Ctrl+R | Ctrl+R | Ctrl+R | ⌘R |
| Chart of Accounts | global | Ctrl+A | Ctrl+A | Ctrl+A | ⌘A |
| Find | global | Ctrl+F | Ctrl+F | Ctrl+F | ⌘F |
| Memorized Transaction List | global | Ctrl+T | Alt+Shift+T | Alt+Shift+T | ⌥⇧T |
| Next window | global | Ctrl+Tab, Ctrl+F6 | Alt+Shift+→, Ctrl+F6 | Alt+Shift+→, Ctrl+F6 | ⌥⇧→, ⌃F6 |
| Previous window | global | Ctrl+Shift+Tab, Ctrl+Shift+F6 | Alt+Shift+←, Ctrl+Shift+F6 | Alt+Shift+←, Ctrl+Shift+F6 | ⌥⇧←, ⌃⇧F6 |
| Close window (asks if unsaved) | window | Esc, Ctrl+F4 | Esc, Ctrl+F4 | Esc, Ctrl+F4 | Esc, ⌃F4 |
| New | window | Ctrl+N | Alt+Shift+N | Alt+Shift+N | ⌥⇧N |
| Edit (register: edit the line in place) | window | Ctrl+E | Ctrl+E | Ctrl+E | ⌘E |
| Export report (the format picked last) | window | Ctrl+E | Ctrl+E | Ctrl+E | ⌘E |
| Delete | window | Ctrl+D | Ctrl+D | Ctrl+D | ⌘D |
| Transaction history | window | Ctrl+H | Ctrl+H | Ctrl+H | ⌥⇧H |
| QuickReport / QuickZoom | window | Ctrl+Q | Ctrl+Q | Alt+Shift+Q | ⌥⇧Q |
| Memorize transaction / report | window | Ctrl+M | Ctrl+M | Ctrl+M | ⌥⇧M |
| Transaction journal | window | Ctrl+Y | Ctrl+Y | Ctrl+Y | ⌘Y |
| Go to the other register | window | Ctrl+G | Ctrl+G | Ctrl+G | ⌘G |
| Print | window | Ctrl+P | Ctrl+P | Ctrl+P | ⌘P |
| Show the list for this field | window | Ctrl+L | Ctrl+L | Ctrl+L | ⌘L |
| Save & New | form | Alt+S | Alt+S | Alt+S | ⌥S |
| Save & Close | form | Alt+A | Alt+A | Alt+A | ⌥A |
| Previous transaction | form | Page Up | Page Up | Page Up | Page Up |
| Next transaction | form | Page Down | Page Down | Page Down | Page Down |
| Insert line | grid | Ctrl+Insert, Ctrl+Shift+Enter | Ctrl+Insert, Ctrl+Shift+Enter | Ctrl+Insert, Ctrl+Shift+Enter | ⌃Insert, ⇧⌘Enter |
| Delete line | grid | Ctrl+Delete, Ctrl+Shift+Backspace | Ctrl+Delete, Ctrl+Shift+Backspace | Ctrl+Delete, Ctrl+Shift+Backspace | ⌃Delete, ⇧⌘Backspace |

Commands with no key in QBD (Enter Bills, Pay Bills, Vendor Credits, Purchase
Orders, Credit Card Charges, Sales Receipts, Receive Payments, Credit Memos,
Estimates, Sales Orders, Statement Charges, Deposits, Transfers, Journal
Entries, Reconcile, Bank Feeds, Report Center, Vendor Center) exist as
commands with empty keymaps, so menus, the palette and a future keymap editor
reach them. Phase E added two of those (`form.salesOrder`,
`nav.statementCharges`) and no keys. In the Statement Charges window **New**
(Ctrl+N / Alt+Shift+N) puts the caret in the new-charge line, **QuickZoom**
(Ctrl+Q) and Enter open the selected register line, Enter in the new-charge
line records it. Menu items can depend on a company preference
(`MenuItem.pref`): Create Sales Orders shows only with `sales_orders_enabled`,
Create Estimates with `estimates_enabled`, Purchase Orders and Receive Items
with `inventory_enabled` off are still shown (QBD hides them; our fixture
company has inventory off and uses them).

Phase D added no commands; it widened where the existing ones apply. In every
list window (Chart of Accounts, the Centers, Item / Class / Terms / Payment
Method / Sales Tax Code / Other Names lists): **New** opens a fresh editor,
**Edit** (or Enter) the selected record's, **Delete** asks and shows the
server's "in use" refusal, **QuickReport** where the list has one. In an
editor: **Save & New**, **Save & Close**, **New** (another editor of the
kind), **Delete**, **Close window** (asks if unsaved), and Enter moves to the
next field as Tab does (fields that take Enter themselves, an open QuickFill
list, the calculator, a date field, a dropdown, keep it). Esc inside an open
Select or MultiSelect dropdown, or an open Menu, closes the dropdown, not the
window (a capture-phase check in `KeyboardProvider`).

The report export follow-up added one command, **Export report** (Ctrl+E in a
report window): it sends the format picked last from the Export menu (CSV to
begin with) and shows its key on that item. Ctrl+E stays Edit in lists and
registers; a report has nothing to edit, so the two never meet in one window.

Rules: the browser keymap is derived from QBD's by dropping what browsers keep
(Ctrl+W/T/N and their Shift forms, Ctrl+Tab, Ctrl+PgUp/PgDn; Ctrl+Q off Windows)
and substituting the alternate; macOS also reserves ⌘Q/⌘H/⌘M, in the desktop app
too. A test asserts no browser keymap binds a reserved combo. The desktop keymap
is chosen when Tauri globals or `window.__LIBREBOOKS__.mode === "desktop"` are
present (override: `?keymap=…`, or Company → My preferences, or the overlay).
Field-handled keys (date keys, calculator, QuickFill, grid and register
navigation) are listed in the overlay from `FIELD_KEYS`. Escape closes the
window, so popovers, entry rows and dialogs that handle Escape themselves must
`preventDefault` + `stopPropagation`.

## Mobile patterns

- Below 48em (768px) the phone shell replaces the desktop one (no flash: read
  synchronously). Bottom tabs: Home, Customers, **+**, Banking, More.
- Windows become a stack: the header's Back closes the active window (asking if
  unsaved) and returns to the previous one; a window with a sub-view (a centre's
  detail) can claim Back to return to its list (`useWindow().setBackHandler`).
- "+" opens a bottom sheet of quick-create actions; More holds the rest, My
  Account, the open window list, shortcuts, theme and Sign out.
- Line grids become one card per line with labelled fields and "Add line";
  registers become tappable cards (newest first) with a large cleared toggle;
  reports scroll sideways inside the grid with the label column sticky;
  reconcile shows one column at a time with the difference pinned; bank-feed
  downloads and attachments are cards; the user detail is a full-screen drawer;
  the list editors stack their fields under the tabs with the Save bar fixed
  above the tab bar; nothing scrolls the page sideways (asserted at 375px).
- Inputs are 16px on touch (no iOS zoom), use `inputMode`/`enterKeyHint`, QuickFill
  opens its list on tap, keyboard hints are hidden on touch. Safe-area insets
  respected. The camera input uses `capture="environment"`.

## Fixture mode

`api/fixtures/`: "Juniper Hill Woodworks", a synthetic custom-furniture shop:
56 accounts (parent/sub, every BS/P&L type, roles, one inactive), 29 names (customers with
jobs, vendors with default expense accounts and a 1099 sub, employees), 18 two-sided
and other items, terms, classes, payment methods, tax codes; ~100 transactions over the last
four months dated relative to today (opening JE, checks with expense and item
lines, CC charges, bills and bill payments, invoices with sales tax, payments into
Undeposited Funds swept by deposits, transfers, loan payments, an estimate).
Deterministic ids (stable URLs). It posts with a simplified mirror of
`posting.rs`, enforces `edit_seq` (409), returns 422 `PostingError`s, `in_use`
and duplicate-name errors, honours a closing date (`closed_period`), emits
change events, and:

- **Reports** (`fixtures/reports.ts`): the 46-id catalog, every report built
  from the posted books with date macros, basis, column layouts (by month /
  quarter / class / name), comparisons and % columns, filters, and drills.
  Exports (`fixtures/exportFiles.ts`): the CSV as the server writes it; a PDF
  that is a real one-page document (title block and the first rows in
  Helvetica, on the paper asked for, a correct xref); the "workbook" is the
  CSV bytes under the XLSX MIME type, a placeholder Excel won't open. An
  unknown format or paper is a 400 `invalid`, as on the server.
- **Saved documents**: memorized transactions (with the due list and
  `/memorized/:id/use`) and memorized reports.
- **Reconciliation**: Pending while in progress, Cleared on finish, the
  adjustment journal entry, undo.
- **Bank feeds** (`fixtures/feeds.ts`): a seeded queue per bank/card account with
  rule and history suggestions, scored candidates, Match / Add / Ignore / Undo,
  a small OFX and CSV reader, connections, SimpleFIN claim, Amazon orders.
- **Administration** (`fixtures/admin.ts`): four users with company access,
  passkeys, a device key and three sessions; an outstanding enrolment link;
  the server settings document (validated whole, `problems` on 400), secrets
  (write-only, `in_use` on delete), an available update, the tunnel (connects
  when a token is set and it's turned on), five backups (run / verify / restore
  / export), two inbox attachments (real blobs; `attachmentUrl` is an object
  URL; `?all=true` lists everything), and the QuickBooks import as the server
  does it: one `ImportStatus`, settings on the generated `ImportSettings` (the
  `.qwc` ids can't change, the bridge key becomes a `secret:` reference,
  `problems()` mirrored as a 400), one job at a time (409 `running`), a sync
  that runs for ~2 s through the phases (over the Web Connector it first
  "waits for the poll"), the server's `import` change events (`id` sync|parity;
  `started`, `progress`, `finished`; settings writes emit nothing, as on the
  server), `?wait=` long-polls that answer on each progress step and when the
  job ends on the status route (`wait_for_change`) and only when it ends on the
  parity route (`wait_for_idle`), a plausible `SyncReport`, a parity job that
  lands a report built from the real fixture balances with two accounts off,
  the `.qwc` file and the Web Connector password (generated and returned once,
  or chosen and 204).
- **Name balances** (`GET /balances/names`): from open items exactly as
  `lb_store::balances::name_balances` does it — charges count what is still
  owed, credit memos / vendor credits / unapplied payments count against, jobs
  roll up into `*_total`, names with nothing open are absent.
- Merge really moves every reference (transactions, postings, parents, defaults)
  and removes the duplicate.
- The closing date refuses with 423 `closed_period` and `detail.closing_date`,
  as the server does.
- **Orders** (phase E): the seed has an open sales order (SO-1001 for Acme
  Corp:Conference Table), a purchase order (PO-2041 to Hardwood Supply Co) of
  which 50 of 80 walnut came in on an item receipt whose lines link back to
  the order's, and a statement charge for Nguyen, David; `sales_orders_enabled`
  and `progress_invoicing` are on. `TxnSummary.open_balance` for a sales
  order, purchase order or estimate is what no linked document has drawn yet
  (`lib/orders.ts: orderOpenAmount`), 0 once fully drawn or closed by hand
  (`extra.IsManuallyClosed`), so `open_only` honours closed orders: the
  behaviour the UI would want from the server, which keeps `open_balance`
  NULL for non-posting types (the UI derives the same figures itself and reads
  no `open_balance` for orders). Deleting an order something draws on is
  `in_use`. Orders post nothing whatever is drawn from them.
- **Companies** (phase D): `POST /companies` (name required, month 1–12) and
  `POST /companies/import` (any non-empty file passes; the name comes from
  the request, else the file name; the manifest carries the file's size and
  SHA-256) make extra companies the session and `GET /companies` list and
  `/c/:id/…` answer for; a restored backup joins them. They share the one set
  of demo books and differ only in their `CompanyInfo` (`PUT /info` is per
  company).
- The sync report follows the options: `full` lists everything again; a
  `txn_limit` or date range is a `partial_run` (watermarks untouched);
  `entities` narrows the report to those kinds; `reimport` counts the
  unchanged as updated. Parity's `as_of` and `trial_balance` show in the
  summary notes.

`?fixtures=stress` adds 5,000 vendors and a 100,000-line "Stress Test Checking"
register. Console handle: `__librebooks.simulateRemoteEdit(txnId)`,
`__librebooks.find('account', 'Checking')`, `__librebooks.txns(co, query)`.

## Findings / gotchas

- **ts-rs exports `u64`/`i64` as `bigint`, JSON carries numbers.** Fixed on the
  Rust side with `#[ts(type = "number")]` on every such field (`edit_seq`
  everywhere, `AttachmentMeta.size`, `BridgeSettings.delay_ms`,
  `FeedRule.priority`); `api/json.ts` no longer revives BigInt, no generated
  type says `bigint`, and the `asNumber()` guard is gone (phase C).
- **Local stand-ins** (in `api/types.ts`) are only shapes the server does not
  export: `Session`, `SaveOutcome`/`PreviewOutcome`/`RegisterResponse`,
  `AuditEntry`, the reconcile envelopes (`ReconcileOverview`, `StartReconcile`,
  `ReconciliationView`), the server-ops shapes (`ServerSettings`, `ServerView`,
  `UpdateView`, `TunnelView`, backups), `CompanySummary` (what `GET
  /companies` and `POST /companies` answer and `Session` lists),
  `ImportedCompany` and `UserDetail`. Everything lb-qbd exports
  (`ImportSettings`, `QbdTransport`, `BridgeSettings`, `QbwcSettings`,
  `SdkSettings`, `FileMode`), `NameBalance`, `StatementCharge` and
  `RegisterLine.downloaded` are the generated ones since phase C, the
  server's import envelopes (`ImportStatus`, `RunningJob`, `JobKind`,
  `SyncBody`, `ParityBody`, `ParityStatus`) since its follow-up, and
  `MintedEnrolmentLink` and `PortableManifest` since the phase D follow-up
  (the opening extras `AccountOpening`/`ItemOpening` are request shapes the
  server reads as extra fields, so they stay local). The two
  request bodies are `#[serde(default)]` on the server, so the client's
  `qbdSync`/`qbdParityRun` take a `Partial<SyncBody>`/`Partial<ParityBody>`
  (`txn_limit` is `number | null`; dates and `started_at` are ISO strings).
- **The generated `ImportSettings` is not what phase B guessed**: no
  `pause_ms`/`auto_sync`/`every` (the bridge has `delay_ms`; the Web Connector
  schedules itself through `qbwc.run_every_minutes` in the `.qwc`), the key is
  `bridge.key_ref` (a `secret:` reference, set through the write-only
  `bridge_key` beside the settings) and `bridge_key_set`/`qbwc_password_set`
  live on `ImportStatus`, not the settings. `GET /import/qbd` returns the whole
  `ImportStatus`; the UI reads one query for the three tabs.
- **`/session` shape** assumed from the coordinator: `{ user: {id, display_name,
  role, kind}, companies, sign_in }`, with `sign_in` meaning "this server uses
  sign-in" (Sign out shown only then). master's `system::session` doesn't send
  `kind`/`sign_in` yet; both are optional in the type.
- **Desktop injection**: `window.__LIBREBOOKS__ = { mode: "desktop", token,
  apiBase }`; `apiBase` may be the server root or the `/api/v1` base (both
  handled). EventSource can't send headers, so the token goes as `?token=` on
  `/events` (the server accepts it there only). `attachmentUrl` appends the
  same `?token=` for `<img>`/links in the desktop app (see "For the server").
- React StrictMode runs state initializers twice: anything with identity (the API
  client, the enrolment token read from the URL) is a module-level singleton.
- A window opened in the background has no size; virtualized lists that scroll
  on mount must wait for a size (RegisterGrid does, via ResizeObserver).
- Mantine Spotlight's open animation delayed focus enough to drop keys typed
  right after Ctrl+K; the palette opens with no transition.
- Mantine `Select` is a `combobox` role (Playwright locators); `SegmentedControl`
  radios are hidden (click the label); `Collapse` takes `expanded`, not `in`;
  a `<Title>` inside `Modal`'s `title` nests headings (React warns) — pass a
  string. Notifications render in their own corner, outside any window's test id.
- A report drilled from a row with monthly columns must drill the row, not the
  first cell's month (`accountTree` has a row-level drill).
- Per-name balances come from `GET /balances/names` (`NameBalance`: positive =
  owed; `receivable`/`payable` are the name's own, `*_total` include its jobs;
  names with nothing open are absent). Its query key sits under `balancesAll`,
  so every transaction change refreshes it with the account balances. The
  centres no longer sum open transactions client-side.
- Windows file-name casing: `payBills.ts` beside `PayBills.tsx` clashed on a
  case-insensitive checkout; the model is `billPayments.ts`.
- The worktree guard here rejects shell loops and `$(…)`/`sh -c` around git; run
  plain commands. The CPU broker's mutex can race (`EPERM … .mutex`) while
  waiting; retry.
- Playwright: `page.goto` resets the in-memory fixture books, so a test that
  needs its own earlier writes navigates in-app (menus, Ctrl+K, or `pushState`
  + a `popstate` event). Two workers; the suite takes ~2 min.
- **Esc reached the window from an open dropdown** (phase D): Mantine's
  Select/MultiSelect close their list on Esc without `preventDefault`, and by
  the time the window's bubble listener ran React had already re-rendered
  `aria-expanded="false"`, so the window-close shortcut fired. The provider
  now samples `aria-expanded` in a capture-phase listener first. A test that
  presses Escape with a dropdown open is the regression check.
- **Esc reached the window from an open Menu** (report export): with the focus
  inside a Mantine `Menu` dropdown (after arrow keys, or an item with
  `closeMenuOnClick={false}`) the event's target is the item, not the button
  carrying `aria-expanded`; and a `Tooltip` between `Menu.Target` and its
  button keeps `aria-expanded` off the button altogether (Tooltip forwards
  events, not attributes). The capture-phase check also excuses a target
  inside `[role="menu"]`, and menu targets take no Tooltip wrapper. The export
  test presses Escape inside the open menu.
- **A changed report request keeps the previous report on screen**
  (`useReport` uses `keepPreviousData`), so a test that changes an option and
  then acts on rows must wait for the new data or the rows it touched are
  replaced (and the collapse set reset) under it. The report window carries
  `aria-busy` while fetching and the subtitle names the period; the drill-down
  test waits on both. This was the "collapse Income" flake seen only in full
  runs on a busy machine.
- Playwright sees a blob-URL `a.download` click as a `download` event;
  `suggestedFilename()` is the `FileDownload.filename` and `download.path()`
  has the bytes (the export test reads the PDF's MediaBox to prove A4 was
  sent). Works in the mobile project too.
- Mantine `MultiSelect`'s input is a `combobox` role too (not a textbox);
  `NumberInput`'s is a `textbox`. `Collapse` takes `expanded`.
- The fixture status long-poll answers on every progress step, so a test that
  wants the finished report loops `while (st.running)` on `qbdStatus(co, 10)`.
- The record engine's Enter-moves-on handler runs on the editor's root in the
  bubble phase and only when nothing `preventDefault`ed: QuickFill's open list,
  the calculator and the date field consume their Enter; a closed Mantine
  Select opens on Enter with `preventDefault`, so it keeps it too. Checkboxes,
  buttons and textareas are skipped.
- The generated `Account` has no opening balance and `Item.quantity_on_hand`
  is "maintained by the store". Since the server's phase D pass, `POST
  /accounts` takes `opening_balance` + `as_of` and `POST /items` takes
  `opening_quantity` + `opening_value` + `as_of`, posting the entry in the
  same write: `ApiClient.createRecord` has an optional `opening`
  (`OpeningFor<L>`: `AccountOpening` | `ItemOpening` in `api/types.ts`), the
  HTTP client spreads it beside the record's fields, `useSaveRecord` takes
  `{ record, opening }` and refreshes the balances when one rode along, and a
  `RecordFormSpec` has an optional `opening(d)`. The record engine's two-step
  `afterSave` is gone with its "saved without its balance" failure mode. The
  item editor shows On hand, Total value and as of for a new inventory part
  and the store's on-hand quantity (and average cost) read-only afterwards;
  its saved toast says what the store now holds ("…saved; 12 on hand"). The
  fixture posts the journal entry / an `InventoryAdjustment` (new case in
  `fixtures/post.ts`: Item lines into the item's asset account, the header
  account the other side, `quantity` on the posting) inside `createRecord`,
  rolling the record back if the entry is refused (closed period…).
- `POST /companies` answers `CompanySummary`: the picker seeds the session
  query's company list from it before invalidating, so the shell knows the
  name without waiting on the refetch. `POST /companies/import` answers no
  role; the picker fills in `admin` (only a server admin can import).
- **A non-posting form's total is the lines' sum on the server** (`Txn.amount`
  comes back without tax for an estimate or sales order: `lines_total()`),
  while the form shows the client's estimated tax under the lines, so the
  saved toast says 185.00 where the form said 198.41. Pre-existing for
  estimates; sales orders inherit it. A server preview for non-posting kinds,
  or a decision that quotes carry no tax, would settle it (phase E).
- **QuickFill's list on a phone** (phase E): a field's list opens on tap and
  stays open while the field has the focus, and its 380px minimum width made
  a 375px page scroll sideways. The dropdown's minimum is now
  `min(380px, 100vw − 16px)`; the statement-charges mobile test measures with
  the list open.
- **Drawing a PO onto a bill** meets the vendor's prefilled expense line (its
  default account, no amount), which would fail validation ("Enter an
  amount"); Select PO drops such a line as the order's items arrive.
- A hand-off without a number (an invoice drawn from an order) now takes the
  next number: `salesDraftFromTxn` sets `numberIsAuto` for an unsaved
  transaction with no number, and the next-number effect also runs when
  `wantsAuto` flips.
- A statement charge for a customer without terms is due on its own date
  (QBD's due-on-receipt), so the due date is never blank.

## Progress log

Phase A:

- [x] Pure logic + tests (money, decimal, calculator, dates, QuickFill, tree).
- [x] Data layer: client interface, HTTP client, fixture client, hooks, SSE.
- [x] Keyboard: combos, commands, platform keymaps, scoped registry, provider.
- [x] Window manager (open windows as tabs) with URL sync and unsaved guard.
- [x] Shell: desktop and mobile, palette, overlay, theme, user menu.
- [x] Inputs: DateField, MoneyField, QuickFill (+ quick create), LineGrid, RegisterGrid.
- [x] Screens: Home, Chart of Accounts, centres, register, Write Checks, reports,
      Find, lists, Company/preferences, Banking, placeholders.
- [x] Auth: sign-in, enrolment, desktop bearer token, 401 → sign in, sign out.
- [x] Playwright smoke tests (desktop + 375px), screenshots reviewed, bugs fixed.

Phase B (commits on `worktree-agent-af93b28647e2ebef7`):

- [x] Reports on the generated lb-reports types; Report Center, options bar,
      export, memorize, drill-down; the fixture report engine.
- [x] Purchase forms: Enter Bills, Vendor Credits, CC Charges/Credits, Item
      Receipts, Purchase Orders (`forms/PurchaseForm.tsx`; Write Checks moved onto it).
- [x] Sales forms: Invoices, Sales Receipts, Credit Memos, Estimates with live
      server-confirmed totals.
- [x] Receive Payments, Pay Bills, Make Deposits, Transfer Funds, General Journal.
- [x] Registers: edit a line in place, 1-Line mode, QuickZoom, the downloaded mark.
- [x] Reconcile.
- [x] Bank Feeds (queue, rules, connections, file import, Amazon).
- [x] Admin and account screens: Users, Enrolment Links (QR), My Account, Server
      Settings, Backups, Import from QuickBooks, Company edit + Set Closing Date,
      Memorized Transaction List, Attachments, Merge (`1a50934`).
- [x] `bun run check` green (233 tests); Playwright 40/40 on desktop and 375px.
- [x] This document.

Phase C (commits on `worktree-agent-a9571472f4dda8acb`, 2026-10-20):

- [x] Generated types in place of the local stand-ins: lb-qbd's `ImportSettings`
      family, `NameBalance`, `StatementCharge`; `asNumber()` removed; the import
      client/fixture/window rebuilt on the server's `ImportStatus` envelope.
- [x] `GET /balances/names` in the client, HTTP, fixture (with tests: roll-up,
      unapplied payment as a credit, void), hooks, keys; the centres show Balance
      and Balance Total.
- [x] Attachments: the Attached tab on `GET /attachments?all=true`; fixture `all`.
- [x] Fixture closing-date refusal is 423 `closed_period` with `detail.closing_date`.
- [x] `bun run check` green (235 tests); Playwright 41/41 (new: the Customer
      Center's balance columns; the Attached tab in the attachments test).
- [x] This document.

Phase C follow-up (commits on `worktree-agent-ab9ab2d43381969c5`, 2026-10-20):

- [x] The server's generated import envelopes (`ImportStatus`, `RunningJob`,
      `JobKind`, `SyncBody`, `ParityBody`, `ParityStatus`) re-exported from
      `api/types.ts` in place of the local copies; `qbdSync`/`qbdParityRun`
      take `Partial<SyncBody>`/`Partial<ParityBody>`.
- [x] The Import window event-driven: `invalidateForEvent` refetches the import
      queries on every `import` event without cancelling a long-poll in flight,
      a sync's `finished`/`failed` invalidates the books, and the window shows a
      notification when a job ends. The fixture emits the server's vocabulary
      (`id` sync|parity; `started`/`progress`/`finished`) and its status
      long-poll answers on progress, as the server's does.
- [x] `bun run check` green (236 tests: `invalidateForEvent`'s import branch;
      the fixture test asserts the event sequence and the progress-wake
      long-poll); Playwright 41/41.
- [x] This document.

Phase D (commits on `worktree-agent-ab9aa5ed8179d8f95`, 2026-10-20; "What's
left" items 2, 5, 6 and 8):

- [x] `POST /companies` and `POST /companies/import` in the client, HTTP
      (JSON; multipart `file` + `name`) and fixture (extra companies share the
      demo books; a restored backup joins them), `blankCompanyInfo`, blank
      records for the small lists; tests on the fixture and the HTTP client.
- [x] The record-form engine (`forms/useRecordForm.tsx`) and the editors as
      windows: accounts (opening balance), names (customers, jobs, vendors,
      employees, other names), items (every type), classes, terms, payment
      methods, sales tax codes; the list windows, the Chart of Accounts and
      the Centers open them; Sales Tax Code List in the Lists menu.
- [x] Attachments on names, accounts and items: the window's title and copy,
      the Attach-to dialog's record kinds, the Attached tab's way into the
      editor, the paperclip on editors and in the Centers.
- [x] The company picker: New company (the shared `CompanyInfoFields`) and
      Open a .lbk file; `/companies` from the user menu and the phone's More.
- [x] Import window Options disclosures (`SyncBody`, `ParityBody`); the
      fixture's reports follow the options.
- [x] Esc in an open dropdown no longer closes the window (a side issue found
      by the Options test; its own commit).
- [x] `bun run check` green (241 tests); Playwright 47/47 (new: customer and
      item created and edited, an in-use delete refused and terms, attachments
      on a vendor, the picker's new company and .lbk, the Options disclosures;
      a vendor editor at 375px).
- [x] This document.

Phase D follow-up (the server's phase D pass taken up; commits on
`worktree-agent-a7de8ce120ad8b5d4`, 2026-10-20):

- [x] `createRecord(co, list, record, opening?)` in the client, HTTP (the
      extras spread beside the record) and fixture (the journal entry or an
      `InventoryAdjustment` posted inside the create, rolled back if refused;
      the item's on hand and average cost from it); `useSaveRecord` takes
      `{ record, opening }`; `RecordFormSpec.opening(d)`; `afterSave` removed.
- [x] Account editor sends `opening_balance` + `as_of` with the create (one
      request); item editor sends `opening_quantity` + `opening_value` +
      `as_of` for a new inventory part (On hand, Total value, as of) and the
      saved toast shows the store's on-hand quantity.
- [x] `POST /companies` → `CompanySummary`; the picker seeds the session's
      company list from it, then invalidates. `PortableManifest` and
      `MintedEnrolmentLink` are the generated types (stand-ins and
      `CreatedCompany` deleted).
- [x] Sales tax code: the fixture refuses a name outside 1–3 characters with
      422 `invalid` on `name` (the editor's `recordServerErrors` puts it on
      the field); the editor's own rule and hint say "one to three".
- [x] Tests: fixture (opening balance posts and shows in balances with the
      natural sign, today when undated, nothing for zero; a refused entry
      leaves no account; inventory opening → adjustment, on hand, average
      cost, asset balance, a non-inventory item refused; the tax code rule;
      the company summary), HTTP (the body carries the extras; the summary).
      Playwright: a bank account with an opening balance shows it in the
      chart from one request; an inventory part saves with its quantity.
- [x] This document.

Report export follow-up (the server's XLSX and PDF export taken up; commits on
`worktree-agent-a3a9fec5729e41528`, 2026-10-20):

- [x] `exportReport(co, request, format: ExportFormat, paper?: ExportPaper)`
      in the client, HTTP (`?format=` lower-cased from the generated enum,
      `&paper=` only when given, the header's file name with the request's
      title as the fallback) and fixture (400 `invalid` as the server; a real
      one-page PDF on the paper asked for and the CSV bytes under the workbook
      MIME type as placeholders). `reports/export.ts` holds the shared menu
      order, labels, extensions, MIME types, guards and file name.
- [x] The report window's Export menu (Excel, PDF, CSV; Letter / A4 ticked
      and remembered), `report.export` on Ctrl+E repeating the last pick
      (`lib/prefs.ts`: `librebooks.reportExportFormat`, `librebooks.reportPaper`),
      Print as a split button with Save as PDF, the 375px "More actions"
      overflow menu holding collapse, refresh, memorize and the exports.
- [x] Esc inside an open Menu closes the menu, not the window; the drill-down
      test waits for the new period (`aria-busy`, the subtitle) before it
      collapses a section.
- [x] Tests: HTTP (the query string with and without paper, `Accept: */*`,
      the header's name and the fallback), fixture (MIME types, bodies, the
      MediaBox per paper, the 400s), the pure helpers; Playwright desktop (the
      three formats, the key hint on the default, Esc in the menu, A4 then the
      PDF's MediaBox from the download, Ctrl+E repeating, the Print menu) and
      mobile (the overflow menu's formats, a PDF download at 375px).
      `bun run check` green (251 tests); Playwright 50/50.
- [x] This document; `plans/reports.md`'s open web item closed.

Phase E (commits on `worktree-agent-a19a3c5986ee129af`, 2026-10-20; "What's
left" item 3 bar payroll):

- [x] `lib/orders.ts` (progress per line and per order, closed, the
      `IsManuallyClosed` mark in `extra`) and `forms/linked.ts` (drawing the
      remainder, typed quantities, a percent or amounts per line; the drawn
      lines; the new document from an order), unit-tested with exact decimals.
- [x] Fixture: the seed's sales order, partly received PO (linked lines) and
      statement charge; `open_balance` for orders as what remains, `open_only`
      honouring closed orders; a test.
- [x] Hooks: `useLinkedDocs` / `useOrderProgress` (the documents linked to an
      order) and `useOpenOrders` (a name's open orders with their progress):
      summaries, then the transactions whole, since there is no `linked_to`
      filter; both under `txnsAll` so any transaction change refreshes them.
- [x] Sales Orders and Statement Charges as sales-form kinds (`hasDueDate`,
      `isNonPostingSale`); the forms' registry, menus (`MenuItem.pref`,
      `useMenuPrefs`), the two commands, GlobalCommands, the Customer Center's
      New transaction menu (Sales Order; Statement Charge preset to the name).
- [x] `OrderProgress` (panel, status, Mark as closed / Reopen, "From Sales
      Order N" note), `CreateFromOrder` (QBD's choices), `OrderPicker` (Select
      PO, Available Sales Orders); Create Invoice from a sales order and an
      estimate; Select PO on Receive Items and Enter Bills (the vendor's
      prefilled expense line gives way to the order's items); Invoice from a
      sales order on a new invoice.
- [x] The Statement Charges window (`screens/StatementCharges.tsx`) and its
      pure model (`forms/statementCharge.ts`: the charge's transaction, the
      register's Amt Chrg / Amt Paid / running balance); the
      `statement-charges` window kind and `paths.statementCharges(customer)`.
- [x] QuickFill's list capped at the phone's width (its 380px minimum pushed
      a 375px page sideways once a field opened it on tap; its own commit).
- [x] Tests: unit (orders, drawing, the statement charge model, menus by
      preference, the fixture's open orders), Playwright desktop (a statement
      charge recorded and QuickZoomed; a sales order invoiced for 4 of 10,
      remaining 6, closed by hand; E-118 at 50%, 425 + 1,350, progress 50%;
      a PO of 20 hinges received 12 then billed 8 through Select PO, closed)
      and 375px (statement charges with the QuickFill list open, a sales
      order). `bun run check` green (269 tests); Playwright 55/55.
- [x] This document.

## What's left (next phases)

1. **Stand-ins without a generated twin** (see Findings) stay until the server
   exports them (`Session`, `SaveOutcome`, the reconcile and server-ops
   envelopes, `UserDetail`).
2. **Lists, the rest**: sales reps and customer/vendor/job types have routes
   but no editor or list window (`RecordWindow` says so); inventory assemblies
   and fixed assets open in the item editor with their common fields only;
   QBD's "Add/Edit Multiple List Entries" grid; a price level list (the API
   has a percent per customer, no list).
3. **Forms not built**: Pay Employees (payroll; out of scope for now).
   Statement Charges, Sales Orders, progress invoicing and Receive Items
   against a PO landed in phase E; left from that family: Enter Bill for
   Received Items (a bill drawing on an item receipt, QBD's `linked` receipt
   lines), a sales order made from an estimate, per-line manual close
   (QBD's "Clsd" column; only the whole order closes here), and the
   Open Sales Orders / Open Purchase Orders reports (the fixture's `open_only`
   lists them; the server's would need the open amount below).
4. **Reports**: print styles beyond the basics (the browser's print of the
   on-screen grid; Save as PDF now covers paper output); cash basis in the
   fixture engine; the real catalog's per-report params beyond what the
   options bar covers. XLSX and PDF export landed in the export follow-up.
5. **Attachments**: done in phase D for every record kind; left: a filter box
   on the Attached tab once a company has many documents (also under 7).
6. **Companies**: creation and import are in the picker (phase D); left:
   renaming or deleting a company (no API), and switching companies from
   inside the shell without passing through the picker.
7. **Polish**: code-split screens (main chunk ~300 KB gzip), Alt+letter menu
   mnemonics (conflicts with browser menus need care), ESLint (the code carries
   `react-hooks` disable comments in anticipation), the Home page's admin
   shortcuts, a filter box on the Attached tab once a company has many
   documents.
8. **Import options**: every field of `SyncBody` and `ParityBody` is in the
   Options disclosures (phase D). Left: remembering the last options per
   company, and a Web Connector "run now" that the transport can't offer.

## For the server

Things the UI wanted that the API doesn't do (or does differently), recorded
rather than worked around twice:

Closed by the server pass of 2026-10-20 and taken up in phase C: `GET
/balances/names`, `GET /attachments?all=true`, `?token=` on `GET
/attachments/:id`, `?wait=` on `GET /import/qbd/parity`, `GET /open` with
`entity_id`/`kind`, `#[ts(type = "number")]` on the `u64`/`i64` fields,
`RegisterLine.downloaded` and the lb-qbd types in the generated files, 423
`closed_period` with `detail.closing_date`. Closed by the server's second pass
the same day and taken up in the phase C follow-up: `ImportStatus`,
`RunningJob`, `JobKind`, `SyncBody`, `ParityBody` and `ParityStatus` generated;
a job's `import` change events (`id` sync|parity, `action`
started|progress|finished|failed); `GET /import/qbd/status?wait=` answering on
progress as well as on finish.

Still open:

- `GET /open` returns `TxnSummary[]`; the UI reads `open_balance` from it and
  treats credits/unapplied payments as negative open items.
- `POST /auth/enrollments` returns `{id, token, purpose, expires_at, url}`; the
  generated `MintedEnrolment` lacks `url`. The UI's `MintedEnrolmentLink` adds it.
  Closed since (`c036536`): the server exports `MintedEnrolmentLink`; the UI
  uses it (phase D follow-up).
- `POST /companies/import` answers `{ id, name, from }` with no `role`; the
  picker assumes `admin` for the importer. A `CompanySummary` plus `from`
  would let it stop assuming.
- `PATCH /auth/passkeys/:id` and the revoke routes are assumed to answer 204
  (the UI refetches rather than reading a body).
- `PUT /import/qbd` and `POST …/qbwc/password` emit no change event (only
  jobs do), so another tab's Connection panel stays stale until it refetches.
  The fixture matches; an `import` event with `id: "settings"` would let the
  UI refresh it.
  Closed since (`a3c075a`): both emit `import` / `settings` / `updated`.
- Restoring a backup creates a company; `GET /session` must include it for the
  picker to show it (the UI invalidates the session query).

Found in phase D:

- Closed on the server (2026-10-20 evening): `POST /accounts` takes
  `opening_balance` + `as_of`, `POST /items` takes `opening_quantity` +
  `opening_value` + `as_of` (an inventory adjustment; the returned item has the
  on-hand quantity), `POST /companies` answers `{ id, name, role }`,
  `PortableManifest` is generated, a sales tax code is one to three characters
  server-side, and the parent rules (no cycles, subaccount type, jobs only
  under customers) were already enforced. Taken up in the phase D follow-up
  (same day): the editors send the opening extras with the create, the
  manifest and enrolment-link stand-ins are gone, the picker uses the
  summary, and the fixture mirrors the tax code rule. The notes as written:
- **An account's opening balance.** `Account` carries no balance; QBD's "Enter
  Opening Balance" is a journal entry against Opening Balance Equity, which
  the account editor posts itself after the save (`POST /txns`, a
  `JournalEntry` with two `Journal` lines dated as of the chosen day). Two
  requests where QBD's dialog is one; a failure between them leaves the
  account saved without its balance (the editor reopens with the error). An
  `opening_balance` + `as_of` on `POST /accounts` would make it atomic.
- **`Item.quantity_on_hand`** is "maintained by the store". The editor sends
  it on create (QBD's New Item takes an on-hand quantity and value as of a
  date, which posts an inventory adjustment) and shows it read-only after.
  Whether the server honours it on create, or needs an `InventoryAdjustment`
  transaction, is unverified.
- **`POST /companies` returns `{ id }` only**; the picker refetches
  `/session` to learn the name and role before it can list the company. A
  `CompanySummary` back would save the round-trip.
- **`PortableManifest`** (`from` on `POST /companies/import`) isn't among the
  generated types; `api/types.ts` has a stand-in (`kind`, `format`,
  `app_version`, `company_name`, `schema_version`, `created_at`,
  `database_size`, `database_sha256`).
- **No server-side rule for a sales tax code's length**; the editor enforces
  QBD's three characters. The server would accept a longer one from another
  client.
- **Parent validity for list records** (a record inside itself, a subaccount
  of another type, a job under a vendor) is enforced by the editors' pickers;
  whether the server refuses such a `parent_id` is unverified (the fixture
  only checks the parent exists).

Found in phase E (orders and what draws on them); none worked around twice,
all derived on the client from `linked_txn_id` / `linked_line_id`. **A server
pass is under way in parallel** (2026-10-21) adding order fulfilment on the
Rust side: `TxnLine.is_closed`, `Txn.is_manually_closed`,
`GET /txns/:id/fulfilment` and open orders in `GET /open`. When it lands and
the generated types carry the fields, the switch is small and in one place
each: `lib/orders.ts` reads `is_manually_closed` instead of
`extra.IsManuallyClosed` (keep reading the tag for imported books until a
migration moves it), `useOrderProgress` reads the fulfilment route instead of
fanning out over the name's documents, `useOpenOrders` reads `/open` with an
order kind, and `OrderProgress` gains the per-line close QBD's "Clsd" column
shows. The fixture then mirrors those routes rather than the open-amount
guess below.

- **No invoiced / received quantity, no "fully invoiced / received", no
  "manually closed" on a sales order, purchase order or estimate.** QBD has
  `IsFullyInvoiced` / `IsFullyReceived`, per-line `Invoiced` / `ReceivedQuantity`
  and `IsManuallyClosed` (order and line). The UI derives the first two from
  the linked documents (`lib/orders.ts`) and keeps the manual close in
  `Txn.extra.IsManuallyClosed` (stored verbatim). Server fields for these,
  maintained like `open_balance` is for A/R items, would make the figures
  authoritative and let the store refuse over-drawing a line.
- **`open_balance` is NULL for non-posting types**, so `GET /txns?open_only`
  cannot list open orders and `GET /open` knows nothing of them. The UI lists
  a name's open orders by fetching its orders and the documents that could
  draw on them, whole. The fixture answers what the server could: an order's
  `open_balance` = its remaining amount, 0 once closed.
- **No `linked_to=<txn id>` filter on `GET /txns`** (nor a lines-included
  summary), so the documents drawing on an order are found by listing the
  name's candidate types and reading each transaction whole
  (`useLinkedDocs`). One request would do with a filter.
- **A line's link is not validated**: the server stores any
  `linked_txn_id` / `linked_line_id` (only deposit lines are read). The forms
  send ids they just read; a check that the target exists, belongs to the
  same name and is of a kind the form may draw on would catch a stale pick.

## Open questions for the user

1. **Enter in grids**: default is "next field" (fast keyboard entry); QBD's own
   default is "save". Recommendation: keep "next field", the preference is one
   click in Company → My preferences.
2. **Browser alternates** use Alt+Shift+letter (Write Checks = Alt+Shift+W).
   Recommendation: keep; they're shown in menus, tooltips and the `?` overlay.
3. **Admin screens inside the company shell**: Users, Enrolment Links and
   Server Settings are server-wide but open as windows under `/c/:id/admin/…`
   so the window system and keyboard apply. Recommendation: keep; a company is
   always open when an admin works, and the paths are stable.

## Things not to do

- Don't hand-edit `web/src/api/generated/`.
- Don't do float arithmetic on money, rates or quantities.
- Don't bind browser-reserved keys in the browser keymap; they never reach the page.
- Don't import `api/fixtures/` outside `api/select.ts` (it's a lazy chunk and a fake).
- Don't put the enrolment token in a query string you log or link; it lives in the
  URL fragment and is stripped on read.
- Don't compute sales tax or form totals on the client; show the server's preview.
- Don't let a secret value come back from the API or linger in React state after
  a save (clear the field on success).
- Don't `page.goto` mid-test in Playwright when the test depends on earlier
  fixture writes.
- Don't build a list editor as a modal inside its list window; it's a `record`
  window on `useRecordForm` (see Decisions), so unsaved edits survive window
  switches and the keys are the transaction forms'.
- Don't post an opening balance or opening quantity as a separate transaction
  after the record saves; send `opening_balance`/`opening_quantity` +
  `opening_value` with the create in the natural sign the user typed. The
  server picks the debit side (a bank balance debits the account, a loan
  balance credits it, Opening Balance Equity takes the other side); the
  fixture mirrors that, nobody else computes it.
- Don't press Escape in a Playwright test expecting the window to stay open
  unless a dropdown is open (then Esc is the dropdown's).
- Don't read `open_balance`, or invent an `is_closed`, for a sales order,
  purchase order or estimate in the UI: derive it from the linked documents
  (`lib/orders.ts`), the one place that knows, so the UI behaves the same
  against the server (where `open_balance` is NULL for them) and the fixture.
- Don't link a drawn line by `linked_txn_id` alone: the quantity roll-up is
  per order line (`linked_line_id`); a line without it counts for nothing.
