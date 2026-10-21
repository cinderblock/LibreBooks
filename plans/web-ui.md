# LibreBooks web UI (phases A, B and C)

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

User requirements this meets: mobile first class from day one; QuickBooks-quality
input (QBD keyboard shortcuts, QuickFill, date keys, the amount calculator);
reports and views. The Tauri desktop app loads this same SPA, so the keymap
differs between the desktop app (real QBD keys) and a browser (where
Ctrl+W/T/N/Tab can't be captured).

## Environment / context

- Phase A was built on worktree branch `worktree-agent-ab818e840111ec792`,
  phase B on `worktree-agent-af93b28647e2ebef7` (both merged by the parent
  session), phase C on `worktree-agent-a9571472f4dda8acb` and its import
  follow-up on `worktree-agent-ab9ab2d43381969c5`. The parent merges `master`
  in; the worktree never does.
- Vite 8, React 19 (StrictMode), Mantine 9.7 (has `Menubar`), React Router 8.5
  (v7-style API from `react-router`), TanStack Query 5, TanStack Virtual 3,
  TypeScript 7 (`tsc`), Bun 1.4 (package manager + unit tests), Playwright 1.64,
  `@simplewebauthn/browser` 14, `qrcode.react` 4.
- Commands, from `web/` (wrap heavy ones in the CPU broker:
  `node ~/.claude/bin/cpu-slots.mjs run --slots N --label "librebooks web" -- …`;
  1 slot for check/tests, 2 for `vite build` and for Playwright's 2 workers):
  - `bun run dev` → http://localhost:5740. Add `?fixtures=1` (or
    `?fixtures=stress`, `&latency=0`, `&keymap=desktop|browser`).
  - `bun run check` = `tsc --noEmit && bun test src/` (235 unit tests, 21 files).
  - `bun run test:e2e` (Playwright; starts the dev server with `VITE_FIXTURES=1`;
    one-time `bunx playwright install chromium`). 41 smoke tests: 27 desktop
    (1366×860) and 14 at 375px.
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
  forms/                 the form engine (useTxnForm) and every form with its pure draft model
  reports/               request ↔ URL params, row outline + CSV, ReportViewer
  screens/               Home, Chart of Accounts, centres, Register, Reconcile, Reports, Find, Lists,
                         Company, Banking, feeds/, Memorized, Attachments, Merge, MyAccount, admin/
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
| `ReportViewer` | Any `Report`: sticky header and label column, outline collapse showing subtotals, QuickZoom drill (txn / report / register), keyboard, virtualized; CSV export. |
| `ReportCenter`, `ReportWindow` | `GET /reports` catalog by QBD category with memorized reports; a report window with the options bar (date macro / from–to, basis, columns, comparisons, show zero), filters modal, Export CSV (`POST /reports/export?format=csv`), Memorize (`POST /saved/memorized_report`), drill-down from `Drill`. |
| `KeyList` | List windows: virtualized, ↑↓ PgUp PgDn Home End Enter, type-to-jump. |
| `Center` | Customer, Vendor and Employee Centers: the names with jobs indented, **Balance** (the name's own open items) and **Balance Total** (with its jobs) from `GET /balances/names`, the detail with contact, bill-to, balance card and transactions, edit, New transaction menu, Ctrl+Q QuickReport. |
| `useTxnForm` / `TxnFormActions` | The form engine (see below) and the standard action bar: Delete, Void, Journal, Memorize, paperclip (Attachments for this transaction), Revert/Clear, Save & New, Save & Close. |
| `PurchaseForm` | Write Checks, Enter Bills, Vendor Credits, Credit Card Charges/Credits, Item Receipts, Purchase Orders (non-posting, says so): Expenses/Items tabs, terms → due/discount dates, recall of the payee's last transaction. |
| `SalesForm` | Invoices, Sales Receipts, Credit Memos, Estimates (non-posting): item lines with qty × rate, sales tax item and customer tax code, live server-confirmed totals (`PostingsPanel`). |
| `PaymentForm`, `PayBills` | Receive Payments (open invoices from `GET /open`, auto-apply, credits, Undeposited Funds or a bank account) and Pay Bills (open bills from `GET /txns?types=Bill&open_only`, discounts/credits, check or card). |
| `DepositForm`, `TransferForm`, `JournalForm` | Make Deposits (payments waiting in Undeposited Funds from `GET /undeposited`, cash back), Transfer Funds, General Journal (debit/credit columns, must balance, auto-numbered). |
| `MemorizeModal` | Ctrl+M: name, Never / Remind / Automatic, interval, next date, days in advance, remaining, group → `POST /saved/memorized_txn`. |
| `Reconcile` | Begin (statement date, ending balance, service charge, interest) then the two-column window: Mark All, running difference, Enter Adjustment, Discard, Undo Last; one column at a time on phones. |
| `BankFeeds` | Accounts summary, the download queue with Match (scored candidates) / Add (preview) / Ignore / Undo, rules (conditions → actions, reorder, apply), connections (file, SimpleFIN claim, Direct Connect, credentials write-only), file import with OFX detection and CSV mapping/preview, Amazon order matching. |
| `MemorizedList` | Ctrl+T: every memorized transaction with its schedule, due ones first; Enter Now (`GET /memorized/:id/use` → hand-off to its form), edit schedule, delete. |
| `Attachments` | Inbox and Attached tabs (the latter from `GET /attachments?all=true`, showing what each document is attached to with a way into the transaction) and one record's files; Add files, Take a photo (`<input capture="environment">`), image thumbnails, attach to a transaction (search by payee/memo/number), detach, delete. Reached from the paperclip on any saved form. |
| `MergeWindow` | Names / accounts / items: pick what disappears and what it folds into; confirm; `POST /merge`. |
| `Company` | Company information with an edit dialog (`PUT /info`, incl. the main preferences), Set Closing Date (`PUT /closing-date`), the administration grid (admins), this device's preferences. |
| `MyAccount` | Passkeys (rename, remove), device keys (revoke), where I'm signed in (sign another device out), Add another device (an `add_device` link with QR), Add a passkey here (mints a link and follows it). |
| `UsersWindow` | Users with role and status; a detail drawer: rename, server role, disable/enable, access to this company (grant/revoke with a role), passkeys and devices, add-device link, delete. |
| `EnrollmentsWindow` | Outstanding links (purpose, role, who for, expiry, revoke) and `MintLinkModal`: new user (role, suggested name, company access, TTL) or add device (whose); the minted link shown once as text + `QRCodeSVG`. |
| `ServerWindow` | General (public URLs, feed auto-sync, Cloudflare Access), Updates (status, check, install with/without restart, policy), Tunnel (state, logs, token write-only, hostname, cloudflared), Backups (schedule, destinations folder/S3, encryption passphrase/age/none, retention, per-company status), Secrets (list, add, replace, delete; `in_use` refused). `PUT /server` sends the document whole; 400 `invalid` lists `detail.problems`. |
| `BackupsWindow` | This company's backups in every destination, Back up now, Verify (checksum; contents with the passphrase/identities), Restore as a new company, Export company file. |
| `ImportWindow` | QuickBooks import on one generated `ImportStatus` document (`GET /import/qbd`; while a job runs, `…/status?wait=` is long-polled and answers on every progress step, so the bar is live; the job's `import` change events refetch it and, on a sync's `finished`/`failed`, the books, with a notification when a job ends): Sync (now / sweep deletions / full; the running job with progress, or "waiting for the Web Connector"), the last `SyncReport` (per-entity counts, issues, mismatches, gaps, unmapped fields, created lists); Connection (the generated `ImportSettings`: bridge URL + write-only key + delay, Web Connector user name / schedule / app URL / password (set or generated once) / .qwc download, SDK file mode, company file), `400 invalid` listing `detail.problems`; Parity (`GET …/parity`: the report, QBD beside ours, differences first; a running comparison polls). |
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
Estimates, Deposits, Transfers, Journal Entries, Reconcile, Bank Feeds, Report
Center, Vendor Center) exist as commands with empty keymaps, so menus, the
palette and a future keymap editor reach them.

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
  nothing scrolls the page sideways (asserted at 375px).
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
  `UpdateView`, `TunnelView`, backups), `MintedEnrolmentLink` (the generated
  `MintedEnrolment` lacks `url`) and `UserDetail`. Everything lb-qbd exports
  (`ImportSettings`, `QbdTransport`, `BridgeSettings`, `QbwcSettings`,
  `SdkSettings`, `FileMode`), `NameBalance`, `StatementCharge` and
  `RegisterLine.downloaded` are the generated ones since phase C, and the
  server's import envelopes (`ImportStatus`, `RunningJob`, `JobKind`,
  `SyncBody`, `ParityBody`, `ParityStatus`) since its follow-up. The two
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
  + a `popstate` event). Two workers; the suite takes ~1.6 min.

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

Phase D (worktree `worktree-agent-ab9aa5ed8179d8f95`, 2026-10-20; "What's
left" items 2, 5, 6 and 8), in progress:

- [ ] Step 1: `POST /companies` and `POST /companies/import` in the client,
      HTTP and fixture (extra companies share the demo books), with tests.
- [ ] Step 2: the record-form engine (`forms/useRecordForm.tsx`, sharing the
      transaction engine's dialogs, action bar and error model) and the list
      editors as windows (`/lists/:list/new`, `/lists/:list/:id`): accounts,
      names, items, classes, terms, payment methods, sales tax codes; the list
      windows open them (Ctrl+N / Ctrl+E / Enter / Ctrl+D).
- [ ] Step 3: Attachments on names, accounts and items; the paperclip on the
      editors; the Attach-to dialog targets any record kind.
- [ ] Step 4: the company picker's New company and Open a .lbk file (admins),
      reachable from the user menu when one company auto-opens.
- [ ] Step 5: Import window Options disclosures (`SyncBody`, `ParityBody`).
- [ ] Step 6: Playwright coverage (customer and item editors, new company,
      import options; a 375px editor pass); this document.

## What's left (next phases)

1. **Stand-ins without a generated twin** (see Findings) stay until the server
   exports them; `MintedEnrolment` would need `url` to replace
   `MintedEnrolmentLink`.
2. **Lists**: item edit form and the other list editors (terms, classes, payment
   methods, sales tax codes) — the list windows are read-only beyond quick add.
3. **Forms not built**: Pay Employees (payroll; out of scope for now), Statement
   Charges, Sales Orders (`sales_orders_enabled`), progress invoicing from an
   estimate, Receive Items against a PO (the form exists; linking lines to the PO
   does not).
4. **Reports**: PDF/XLSX export and print styles beyond the basics; cash basis
   in the fixture engine; the real catalog's per-report params beyond what the
   options bar covers.
5. **Attachments on list records** (names, accounts, items): the API takes any
   `record_kind`; the window only offers transactions as targets.
6. **Company creation** (`POST /companies`) and switching from the picker; the
   restored-company flow relies on the picker refreshing.
7. **Polish**: code-split screens (main chunk ~300 KB gzip), Alt+letter menu
   mnemonics (conflicts with browser menus need care), ESLint (the code carries
   `react-hooks` disable comments in anticipation), the Home page's admin
   shortcuts, a filter box on the Attached tab once a company has many
   documents.
8. **Import options the UI doesn't offer yet**: `SyncBody` has `reimport`,
   `txn_limit`, `txn_from`/`txn_to`, `apply_credits`, `entities`; `ParityBody`
   has `as_of` and `trial_balance`. The client sends them; the window only has
   Sync / sweep / full and Compare now.

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
- `PATCH /auth/passkeys/:id` and the revoke routes are assumed to answer 204
  (the UI refetches rather than reading a body).
- `PUT /import/qbd` and `POST …/qbwc/password` emit no change event (only
  jobs do), so another tab's Connection panel stays stale until it refetches.
  The fixture matches; an `import` event with `id: "settings"` would let the
  UI refresh it.
  Closed since (`a3c075a`): both emit `import` / `settings` / `updated`.
- Restoring a backup creates a company; `GET /session` must include it for the
  picker to show it (the UI invalidates the session query).

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
