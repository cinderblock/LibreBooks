# LibreBooks web UI (phase A: foundation)

Living plan for the SPA under `web/`. Master plan: `plans/librebooks.md`
(decisions 7 and 13). HTTP contract: `plans/api.md`. Auth library:
`plans/auth.md`.

## Goal

The UI's foundation, built before the server existed: the app shell, the input
system (keyboard + input components) and the data layer, plus a **fixture mode**
(an in-memory fake API seeded with a synthetic company) so every screen renders
and can be exercised with `bun run dev`.

User requirements this meets: mobile first class from day one; QuickBooks-quality
input (QBD keyboard shortcuts, QuickFill, date keys, the amount calculator);
reports and views. The Tauri desktop app loads this same SPA, so the keymap
differs between the desktop app (real QBD keys) and a browser (where
Ctrl+W/T/N/Tab can't be captured).

## Environment / context

- Built on worktree branch `worktree-agent-ab818e840111ec792` (merged by the
  parent session). `master` was merged in once, to pick up the generated
  `CompanyInfo`/`Preferences`/lb-auth types.
- Vite 8, React 19 (StrictMode), Mantine 9.7 (has `Menubar`), React Router 8.5
  (v7-style API from `react-router`), TanStack Query 5, TanStack Virtual 3,
  TypeScript 7 (`tsc`), Bun 1.4 (package manager + unit tests), Playwright 1.64,
  `@simplewebauthn/browser` 14.
- Commands, from `web/` (wrap heavy ones in the CPU broker:
  `node ~/.claude/bin/cpu-slots.mjs run --slots N --label "librebooks web" -- …`;
  1 slot for check/tests, 2 for `vite build` and for Playwright's 2 workers):
  - `bun run dev` → http://localhost:5740. Add `?fixtures=1` (or
    `?fixtures=stress`, `&latency=0`, `&keymap=desktop|browser`).
  - `bun run check` = `tsc --noEmit && bun test src/` (157 unit tests).
  - `bun run test:e2e` (Playwright; starts the dev server with `VITE_FIXTURES=1`;
    one-time `bunx playwright install chromium`). 14 smoke tests, desktop and 375px.
  - `bun run build`.
- Generated API types live in `web/src/api/generated/` (ts-rs). Never edit them.

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

## Architecture

```
web/src/
  main.tsx, App.tsx      boot: create the API client, providers, router
  lib/                   pure logic, unit-tested
    money, decimal, calc     cents, exact decimals, QBD calculator
    dates                    ISO dates, period.rs port (date keys, macros), typed-date parser
    quickfill, tree          ranked hierarchical matching; Parent:Child helpers
    accountTypes, txnTypes   mirrors of lb-core facts (labels, classes, register columns)
    platform, prefs, useMobile
  api/                   the data layer
    client.ts                ApiClient interface (the one thing screens talk to)
    http.ts                  fetch client: cookies, X-LibreBooks, bearer token, SSE w/ retry
    errors.ts                ApiError, posting-error text, error → form field mapping
    json.ts                  edit_seq revived to BigInt (see Findings)
    types.ts, blank.ts       re-exports of generated types + local stand-ins; blank records
    keys.ts, hooks.ts        TanStack Query keys, hooks per resource, invalidation per event
    context.tsx              ApiProvider, CompanyProvider, SSE subscription, change bus
    select.ts                fixture vs HTTP; desktop injection; page singleton
    fixtures/                FIXTURE ONLY: seed company, posting mirror, fake server, reports
  keyboard/              commands, platform keymaps, scoped registry (pure), React provider
  windows/               open-window list (reducer), URL ↔ window, provider, catalog
  shell/                 desktop + mobile shells, menus, palette, overlay, user menu
  inputs/                DateField, MoneyField, QuickFill (+ pickers, QuickCreate), LineGrid, RegisterGrid
  forms/                 Write Checks + its pure models (purchase lines, check draft, register entry)
  reports/               Report types (local stand-in), ReportViewer, outline + CSV logic
  screens/               Home, Chart of Accounts, centres, Register, Reports, Find, Lists, Company, Banking, placeholders
  auth/                  SignIn (/signin), Enrol (/enroll#token=…)
  components/KeyList     keyboard-driven virtualized list used by list windows
```

Data flow: screens → hooks (`api/hooks.ts`) → `ApiClient`. Writes invalidate what
they touch; server change events (`/events?company=`) invalidate by `kind` and
are re-published on an in-app bus so an open form can react to its own record
changing. Unknown event kinds invalidate the whole company (correct, heavier).

## Component inventory

| Component | What it does |
| --- | --- |
| `DateField` | Typed dates (`10/19`, `10-19-26`, `101926`, `19`, `t`); QBD keys `+ - T W K M H Y R [ ]` (`+`/`-` act as keys only on a whole date, so `10-19-26` still types); Alt+↓ calendar; non-blocking past/future warning (`useDateWarning`). |
| `MoneyField` | Cents in/out; `1,234.56`, `$5`, `(12.00)`; calculator `12*3+4` then `=`/Enter/Tab with a live tape; `+`/`-`/`*`/`/` on a filled field adjusts it; Esc undoes. |
| `QuickFill` | Type-ahead: inline completion on prefix matches, any-colon-level/word/key matching, indented tree with Type/Balance columns, `<Add New>` + not-in-list quick add, inactive filtering, ↓↑ PgUp/PgDn Enter Tab Esc Alt+↓ Ctrl+L, virtualized. |
| `AccountQuickFill`, `NameQuickFill`, `ItemQuickFill`, `ClassQuickFill` | QuickFill bound to the lists (memoized options). |
| `QuickCreateProvider` | The modal behind `<Add New>`: account, name (incl. job of), item. |
| `LineGrid` | Form line grid: Tab/arrows/Enter navigation, Ctrl+Insert/Delete, one trailing blank line, per-cell errors, footer totals; card-per-line on phones. |
| `RegisterGrid` | QBD two-line register, virtualized (tested at 100k lines), keyboard selection, cleared toggle (optimistic), inline new-transaction line (check/deposit/charge/credit/journal); tappable list on phones. |
| `ReportViewer` | Any `Report`: sticky header and label column, outline collapse showing subtotals, QuickZoom drill (txn / report / register), keyboard, virtualized; CSV export. |
| `KeyList` | List windows: virtualized, ↑↓ PgUp PgDn Home End Enter, type-to-jump. |
| `WriteChecks` | The template form (see below). |
| Shell | Desktop: menu bar with shortcut hints, icon rail, window tabs with unsaved dots, palette button, live indicator, fixture badge, theme, user menu. Phone: header with Back/search, stacked windows, bottom tabs with "+" create sheet and More sheet. |
| `CommandPalette` | Ctrl+K: forms, reports, accounts, names, items, open windows; QuickFill ranking; grouped. |
| `ShortcutOverlay` | `?`/F1: every key on this platform by scope, the ones live in this window highlighted, plus field keys; keymap switch. |
| `SignIn`, `Enrol` | Passkey sign-in (button + conditional UI autofill); enrolment from `#token=` (stripped from the URL), name asked before the passkey prompt, same-credential retry if the server wants a name. |

Write Checks (the template for the other forms): bank account + ending balance,
payee (recalls the payee's last check, else the vendor's default expense
account), number (next number per account; "Print later"), date with warning,
`$ Amount` (typed amount fills or balances the lines, QBD-style), amount in
words, address, memo; Expenses/Items tabs with totals (item → description/cost;
qty × cost exact; editing amount back-solves cost to 5 places); Save & New (Alt+S),
Save & Close (Alt+A), Revert/Clear, Delete (Ctrl+D), Journal (Ctrl+Y, server
preview), History (Ctrl+H), Print (Ctrl+P), Previous/Next (PgUp/PgDn). Client
validation, then server errors (posting/closed period/invalid/duplicate) mapped
onto header fields and grid cells. Stale `edit_seq` → "Someone else changed this"
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
| Edit | window | Ctrl+E | Ctrl+E | Ctrl+E | ⌘E |
| Delete | window | Ctrl+D | Ctrl+D | Ctrl+D | ⌘D |
| Transaction history | window | Ctrl+H | Ctrl+H | Ctrl+H | ⌥⇧H |
| QuickReport | window | Ctrl+Q | Ctrl+Q | Alt+Shift+Q | ⌥⇧Q |
| Memorize transaction | window | Ctrl+M | Ctrl+M | Ctrl+M | ⌥⇧M |
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

Rules: the browser keymap is derived from QBD's by dropping what browsers keep
(Ctrl+W/T/N and their Shift forms, Ctrl+Tab, Ctrl+PgUp/PgDn; Ctrl+Q off Windows)
and substituting the alternate; macOS also reserves ⌘Q/⌘H/⌘M, in the desktop app
too. A test asserts no browser keymap binds a reserved combo. The desktop keymap
is chosen when Tauri globals or `window.__LIBREBOOKS__.mode === "desktop"` are
present (override: `?keymap=…`, or Company → My preferences, or the overlay).
Field-handled keys (date keys, calculator, QuickFill, grid and register
navigation) are listed in the overlay from `FIELD_KEYS`.

## Mobile patterns

- Below 48em (768px) the phone shell replaces the desktop one (no flash: read
  synchronously). Bottom tabs: Home, Customers, **+**, Banking, More.
- Windows become a stack: the header's Back closes the active window (asking if
  unsaved) and returns to the previous one; a window with a sub-view (a centre's
  detail) can claim Back to return to its list (`useWindow().setBackHandler`).
- "+" opens a bottom sheet of quick-create actions; More holds the rest, the open
  window list, shortcuts, theme and Sign out.
- Line grids become one card per line with labelled fields and "Add line";
  registers become tappable cards (newest first) with a large cleared toggle;
  reports scroll sideways inside the grid with the label column sticky; nothing
  scrolls the page sideways (asserted at 375px).
- Inputs are 16px on touch (no iOS zoom), use `inputMode`/`enterKeyHint`, QuickFill
  opens its list on tap, keyboard hints are hidden on touch. Safe-area insets
  respected.

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
and duplicate-name errors, emits change events, and builds P&L, Balance Sheet
(it balances; tested), Trial Balance, Account QuickReport, Transaction List by
Date and Customer Balance Summary with drill-downs. `?fixtures=stress` adds 5,000
vendors and a 100,000-line "Stress Test Checking" register. Console handle:
`__librebooks.simulateRemoteEdit(txnId)`, `__librebooks.find('account', 'Checking')`.

## Findings / gotchas

- **ts-rs `edit_seq` is `bigint`, JSON is a number.** `api/json.ts` revives it to
  BigInt on read and writes it back as a number, so the generated types are
  truthful at runtime. A cleaner fix on the Rust side: `#[ts(type = "number")]`.
- **Local stand-ins** to swap when generated: `Report`/`ReportRequest`
  (`reports/types.ts`), `SaveOutcome`/`PreviewOutcome`/`RegisterResponse`/
  `OpenTxn`/`Session` (`api/types.ts`).
- **`/session` shape** assumed from the coordinator: `{ user: {id, display_name,
  role, kind}, companies, sign_in }`, with `sign_in` meaning "this server uses
  sign-in" (Sign out shown only then). master's `system::session` doesn't send
  `kind`/`sign_in` yet; both are optional in the type.
- **Desktop injection**: `window.__LIBREBOOKS__ = { mode: "desktop", token,
  apiBase }`; `apiBase` may be the server root or the `/api/v1` base (both
  handled). EventSource can't send headers, so the token goes as `?token=` on
  `/events` (the server accepts it there only).
- React StrictMode runs state initializers twice: anything with identity (the API
  client, the enrolment token read from the URL) is a module-level singleton.
- A window opened in the background has no size; virtualized lists that scroll
  on mount must wait for a size (RegisterGrid does, via ResizeObserver).
- Mantine Spotlight's open animation delayed focus enough to drop keys typed
  right after Ctrl+K; the palette opens with no transition.
- The phone window host must be a flex child, or every list renders at 0 height
  (found by Playwright; desktop was fine).
- Mantine `Select` is a `combobox` role (Playwright locators).
- Per-name balances: the API has no endpoint; the centres sum `open_balance` from
  `GET /txns?types=…&open_only=true`. A `GET /balances/names` would be cheaper.
- The worktree guard here rejects shell loops and `$(…)`/`sh -c` around git; run
  plain commands. The CPU broker's mutex can race (`EPERM … .mutex`) while
  waiting; retry.

## Progress log

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
- [x] This document.

## What's left (next phases)

1. **Other forms** on the Write Checks template: Enter Bills and Credit Card
   Charges first (they reuse `purchaseLines`/most of `checkDraft`), then Invoice
   / Sales Receipt / Credit Memo (sale-side lines, sales tax), Receive Payments
   and Pay Bills (`GET /open`), Make Deposits (`GET /undeposited`), Transfer,
   General Journal (debit/credit columns), Estimates/POs. Placeholders exist for
   each menu entry.
2. **Registers**: editing an existing line in place (QBD allows it); Pending (`*`)
   from reconcile; reconcile screen (server has `/reconciliations`).
3. **Reports UI**: swap to generated `Report` types; memorized reports, column
   options (by month/class/job), comparisons, PDF/XLSX export, print styles
   beyond the basics, the real catalog's params.
4. **Auth/admin screens**: mint enrolment links (QR via `qrcode.react`), users and
   roles, my passkeys and sessions, device keys; company creation; company
   preferences editing (read-only now).
5. **Lists**: item edit form, other list editors; memorized transactions.
6. **Polish**: code-split screens (main chunk ~300 KB gzip), Alt+letter menu
   mnemonics (conflicts with browser menus need care), ESLint (the code carries
   `react-hooks` disable comments in anticipation), cash basis in fixture reports.

## Open questions for the user

1. **Enter in grids**: default is "next field" (fast keyboard entry); QBD's own
   default is "save". Recommendation: keep "next field", the preference is one
   click in Company → My preferences.
2. **Browser alternates** use Alt+Shift+letter (Write Checks = Alt+Shift+W).
   Recommendation: keep; they're shown in menus, tooltips and the `?` overlay.

## Things not to do

- Don't hand-edit `web/src/api/generated/`.
- Don't do float arithmetic on money, rates or quantities.
- Don't bind browser-reserved keys in the browser keymap; they never reach the page.
- Don't import `api/fixtures/` outside `api/select.ts` (it's a lazy chunk and a fake).
- Don't put the enrolment token in a query string you log or link; it lives in the
  URL fragment and is stripped on read.
