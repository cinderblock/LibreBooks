# LibreBooks: master plan

## Goal

An MIT-licensed, open-source, complete drop-in replacement for QuickBooks Desktop
(QBD): the same double-entry books, the same forms, registers, lists and reports,
the same keyboard-first data entry, without a subscription and without Intuit.

It runs three ways from one codebase:

1. **Desktop app** (Tauri) holding the books locally. It can also serve them to
   the network (its own web server) for phones and other computers.
2. **Headless server** (one binary, or a container), first class, not an
   afterthought: everything the desktop app does, through the web UI.
3. **Thin client**: the desktop app pointed at a remote server, holding nothing
   locally.

The web UI works on phones from day one, not as a later port.

The first real user is the author's own company (TWILL Technology, Inc.), whose
books live in QBD on `cad-3.tsl`. LibreBooks dogfoods those books by
**reading** them, never writing, until it is close to feature parity, so that
QBD operations are not disturbed.

## Environment / context

- Dev machine: Windows 11, Bun 1.4.2, Rust 1.99 (stable), Node 24, tauri-cli 2.10.1,
  cargo-dist installed. Repo at `C:\Users\camer\git\Personal Projects\LibreBooks`.
- Primary branch `master` (system `init.defaultBranch`). Checkout CRLF;
  `.gitattributes` pins LF only for files Linux executes.
- Sibling projects whose conventions this follows:
  - `open-task` (Rust + minisign self-updater `ot-update`, tag-driven release
    workflow, `plans/`, `ISSUES.md`, CRLF `rustfmt.toml`).
  - `time-tracker` (Bun + React + Mantine + SimpleWebAuthn; passkey-only auth
    with one-time enrolment links; talks to QBD through the QB Bridge and the
    QuickBooks Web Connector).
  - `amazon-transaction-downloader` (scrapes Amazon's transaction history to match
    card charges to orders): reference for the Amazon connector.
- **QB Bridge** (`github.com/cinderblock/quickbooks-desktop-sdk-bridge`), running
  on `cad-3.tsl`, base URL `http://10.255.0.18:8743` (use the IPv4 literal; the
  hostname can resolve to public IPv6 and get a 403 `FORBIDDEN_IP`). Header
  `X-API-Key`, key in `Receipts/.env` as `QB_API_KEY`. Envelope
  `{ok, data, meta}`. See "Findings" for its shape.

## Decisions already made (don't re-ask)

User's requirements (from the first message, verbatim intent):

- Greenfield, open source, **MIT**.
- Complete drop-in replacement for QBD. Native QBW support would be neat;
  importing into our own format is acceptable (from a QBW file or via a
  bridge/API).
- Desktop app, "probably Tauri". Headless server mode is equally first class.
  The desktop app can enable its web server. The desktop app can be a thin
  client to a remote backend.
- Web views are mobile first class from day one.
- Built-in backups.
- API connections to load bank / Amazon / other accounts as automatically as
  possible.
- Self-update, plus opt-in automatic update.
- CI builds and releases on new tags.
- All remote auth is passkeys (or something equally secure that isn't a
  password), plus a simple enrolment token that expires after a short window or
  its first use.
- High-quality input: QuickBooks-like keyboard shortcuts, reports, views.
- Opt-in wrapping of `cloudflared`, so publishing through a Cloudflare zero-trust
  tunnel needs only the tunnel token (maybe the hostname too).
- Windows and Linux; macOS eventually.
- Dogfood with real company data without disturbing QBD until near parity.
- "Boil the ocean": full scope, best way over fastest.

Architecture decisions taken in this plan (reasons inline; revisit only with the
user):

1. **Rust backend, TypeScript/React frontend.** One Rust library holds the
   accounting engine, storage, auth and HTTP API. The headless server is that
   library in a small static binary; the Tauri app links the same library
   in-process (no sidecar, no second runtime). Reasons: Tauri is Rust already; a
   ~20 MB single binary for servers instead of a ~100 MB Bun executable; the type
   system enforces ledger invariants; binary-format work (QBW, OFX) is natural in
   Rust; sibling project `open-task` already proves the release/update pipeline
   in Rust.
2. **SQLite, one file per company** (like a `.QBW`), WAL mode, via `rusqlite`
   (bundled SQLite, so every platform gets the same version). A server holds any
   number of companies. Money is `i64` minor units (cents); quantities and rates
   are `rust_decimal` stored as text. Never floating point.
3. **Ledger model**: every transaction has a header, "form lines" (what the user
   typed: items, expenses, quantities) and **postings** (the double-entry
   result). Postings are derived from the form by type-specific rules and must
   sum to zero; the database refuses a transaction that does not balance.
   Reports read postings; forms read lines.
4. **Every change is audited.** Transactions carry an edit sequence (optimistic
   concurrency, like QBD's `EditSequence`), and every save/void/delete writes a
   full before/after revision. Closing date per company, overridable only by an
   admin with an explicit acknowledgement.
5. **QBD import is read-only by construction.** The bridge client exposes GET
   only; there is no code path that can write to QBD. Every imported record keeps
   its QBD id (`ListID`/`TxnID`) and `EditSequence`, so re-syncs are incremental
   and idempotent. A parity page compares LibreBooks' balances with QBD's own
   (account balances, trial balance) every sync.
6. **Two QBD transports, one normaliser.** The bridge speaks qbXML-shaped JSON;
   the QuickBooks Web Connector (free, ships with QBD) speaks qbXML over SOAP.
   Both feed one normaliser, so anyone (not just users of the bridge) can import
   from QBD. Native `.QBW` reading is a research track (see Phase 9).
7. **Frontend**: React 19 + Vite SPA, Mantine (as in time-tracker), React Router
   in data mode, TanStack Query for server state. Served by the Rust server from
   assets embedded in the binary. No SSR (the desktop app and the thin client load
   the same SPA).
8. **Auth**:
   - Browsers: passkeys (WebAuthn, `webauthn-rs`).
   - Enrolment: an admin (or the CLI on the server) mints a token: 128-bit random,
     single use, expiring in 15 minutes by default, scoped to a role and company.
     It is shown as a link + QR code. Redeeming it registers a passkey. First run
     prints a bootstrap enrolment link for the first admin.
   - Desktop thin clients and CLI/API clients: a **device key** (Ed25519, private
     half in the OS keychain) registered with an enrolment token, signing a
     server challenge. Covers WebKitGTK (Linux webview) which lacks WebAuthn.
   - Local desktop mode needs no login: the app talks to its in-process server
     over a per-launch secret on loopback.
   - Sessions: server-side, HttpOnly + Secure + SameSite=Strict cookies, idle and
     absolute timeouts, revocable per device.
   - Optional: verify Cloudflare Access JWTs (`Cf-Access-Jwt-Assertion`) as a
     second layer when behind a CF Access application.
9. **Self-update**: signed releases (minisign over `SHA256SUMS`, the same scheme
   as open-task). Desktop uses the Tauri updater; the headless server replaces its
   own binary and restarts (systemd / Windows service / container users just pull).
   Checking is on by default and only notifies; automatic install is opt-in.
10. **cloudflared**: opt-in. LibreBooks downloads the official binary (checksum
    verified), runs `cloudflared tunnel run --token <token>` under supervision,
    and shows its status. The public hostname is configured in the Cloudflare
    dashboard; LibreBooks takes it too so passkeys get the right relying-party id.
11. **Backups**: SQLite online-backup API snapshot → zstd → age encryption
    (optional) → destinations (local folder, any S3-compatible bucket, more
    later). Scheduled, retention by grandfather-father-son, each backup verified by
    opening it and running `integrity_check`. Restore from the UI or CLI. Plus a
    portable company file (`.lbk`: zip of the database and attachments).
12. **Bank / account feeds**, in order: file import (OFX/QFX/QBO Web Connect, CSV
    with saved mappings) → SimpleFIN Bridge (cheap, user-owned, read-only) → OFX
    Direct Connect (what QBD itself uses, free where banks still allow it) →
    Plaid and Teller with the user's own keys → Amazon (order history scrape or
    data export, as amazon-transaction-downloader does) → Stripe/PayPal/Gusto
    APIs. Downloaded transactions land in a "Bank Feeds" queue with rules,
    matching and one-key accept, as QBD's bank feeds do.
13. **Keyboard map**: QBD's shortcuts are the default. Where a browser reserves a
    key (Ctrl+W/T/N), the browser keymap uses an alternate (shown in menus and the
    `?` overlay); the desktop app captures the real ones.
14. **Repo layout**: Cargo workspace under `crates/`, the SPA under `web/`, the
    Tauri app under `desktop/`. Bun workspace for JS. Crates are prefixed `lb-`.

## Architecture

```
crates/
  lb-core        domain types, money, account types, posting rules, validation (pure, no IO)
  lb-store       SQLite schema + migrations, repositories, audit, concurrency
  lb-reports     report engine (P&L, BS, TB, GL, aging, job costing, …), basis, date macros
  lb-auth        passkeys, enrolment tokens, device keys, sessions, roles
  lb-qbd         QBD import: qbXML/bridge normaliser, read-only bridge client, QBWC endpoint, IIF
  lb-feeds       OFX/QFX/QBO/CSV parsers, SimpleFIN/Plaid/Teller clients, matching rules
  lb-backup      snapshots, compression, encryption, destinations, retention, restore
  lb-update      signed-release self-updater
  lb-tunnel      cloudflared download + supervision
  lb-server      axum HTTP API, SSE live updates, embedded SPA, server config
  librebooks     the `librebooks` binary: serve, enrol, backup, restore, import, update
desktop/         Tauri 2 app (local / local+web / thin-client modes, tray, updater)
web/             React SPA (mobile-first, keyboard-first)
```

API: JSON over HTTPS at `/api/v1`, OpenAPI generated from the Rust types
(`utoipa`), TypeScript types generated from the spec for the SPA. Live updates
over SSE (`/api/v1/events`) so a second user's change appears in open registers.

## Plan / steps

Phases are ordered; each ends usable. **Current step is marked ▶.**

- Phase 0 — Foundation
  - Plan, repo, license, README, workspace skeleton, `.gitattributes`. Done
    (`858b1be`). CI still to write.
- **Wave 1 (2026-10-19), parallel worktrees, each with `plans/<area>.md`:**
  merged so far: lb-auth (`ff76a50`), lb-feeds (`11134c1`), lb-reports
  (`51d6943`), lb-backup/lb-update/lb-tunnel (`bc44451`). Still running: lb-qbd,
  web UI phase A (fixture mode), desktop (Tauri), and a server-ops integration
  agent (secret store, settings, backups/update/tunnel/feeds routes and CLI).
  Merged since: lb-qbd (`a0a7559`), web UI, desktop (`5951bd2`), server ops
  (`ef1d3e4`: secret store, settings, backups/update/tunnel/feeds routes).
  Server-ops CLI merged too (`5427aba`). **2026-10-20 decisions from the user:**
  repo published (github.com/cinderblock/LibreBooks, pushed); release signing
  keys generated and kept only as GitHub secrets (`42946a7`); feeds use a
  separate "downloaded" mark (`b084b6f`); auto-add stays strict; importing must
  not require the QB Bridge → merged QuickBooks transports (Web Connector SOAP
  endpoint, SDK over COM, import routes/settings/CLI; 91 lb-qbd tests). Whole
  workspace: 623 tests; CI green at `5a2aa32`. ▶ Running: web phase B agent
  (resumed 2026-10-20 09:20 PT after a session-limit cut-off). Next with the user: point
  QuickBooks' Web Connector on cad-3 at this server over HTTPS (steps in
  `plans/qbd-import.md`), run a sync, and see whether the 14 bridge gaps close.
- Phase 1 — Ledger core
  - Domain types, chart of accounts (all QBD account types), names (customers +
    jobs, vendors, employees, other names), items (all QBD item types), classes,
    terms, payment methods, sales tax codes.
  - Transactions: every QBD posting type with its posting rules, linked
    transactions (payments ↔ invoices, bill payments ↔ bills, credits, deposits
    of undeposited funds, PO → bill), voids, audit revisions.
  - SQLite store with migrations; balance invariant enforced; closing date.
- Phase 2 — Server + auth
  - axum API for lists, transactions, registers, reports; SSE; embedded SPA.
  - Passkeys, enrolment tokens (CLI + UI, QR), device keys, sessions, roles.
- Phase 3 — QBD dogfood import (read-only)
  - Bridge client (GET-only), normaliser, full + incremental sync, id mapping,
    parity page (per-account balance diff vs QBD, trial balance diff).
  - Gaps in the bridge (types it does not expose) logged in ISSUES.md; filled
    meanwhile by the Web Connector transport.
- Phase 4 — Web UI
  - Shell: desktop sidebar/menus + mobile bottom nav; command palette; `?`
    shortcut overlay; company switcher.
  - Chart of accounts, registers (inline-editable, QuickFill), centres
    (customer/vendor/employee), lists.
  - Forms: write check, enter bills, pay bills, invoice, receive payment, sales
    receipt, deposit, transfer, credit card charge/credit, journal entry,
    credit memo, vendor credit, estimate, PO, item receipt.
  - Input: date field (QBD keys + / - t w k m h y r), amount field with
    calculator, QuickFill comboboxes with `Parent:Child` names, Tab/Enter flow.
  - Reports: P&L, BS, TB, GL, transaction detail, AR/AP aging, open invoices,
    unpaid bills, customer/vendor balance, sales by customer/item, job costing
    (P&L by job), cash basis, comparisons, memorised reports, export CSV/XLSX/PDF,
    QuickZoom drill-down.
  - Reconcile, find, transaction history, attachments (photo a receipt on a phone).
- Phase 5 — Desktop (Tauri)
  - Local mode with in-process server, "serve to network" toggle, thin-client
    mode with device-key auth, tray, real QBD keymap, updater.
- Phase 6 — Ops features
  - Backups (schedule, destinations, verify, restore, portable file).
  - Self-update (server + desktop), opt-in auto install.
  - cloudflared wrapper, CF Access JWT check.
  - Release workflow on tags: Windows (MSI/NSIS), Linux (AppImage/deb/rpm),
    headless binaries (x86_64/aarch64 Linux musl, Windows), GHCR image, signed
    sums, updater manifest.
- Phase 7 — Feeds
  - OFX/QFX/QBO/CSV import, Bank Feeds queue, rules, matching, SimpleFIN,
    OFX Direct Connect, Plaid/Teller (BYO keys), Amazon, Stripe/PayPal.
- Phase 8 — Parity tail
  - Inventory (average cost, assemblies, adjustments), sales tax, payroll records
    (not calculation, initially), 1099s, budgets, multi-currency, statements,
    printing checks/forms, email, memorised transactions, recurring, IIF
    import/export, multi-user record locking UX.
- Phase 9 — Native QBW (research)
  - `.QBW` is a Sybase/SAP SQL Anywhere database with Intuit's encryption. Study
    the format (Ghidra is available via the `ghidra` MCP) to read a QBW directly
    for a one-shot migration without QBD installed.

## Findings / gotchas

- **Bridge entity coverage (2026-10-19):** Account, Customer, Vendor, Employee,
  Item (+ Service/Inventory/NonInventory), Invoice, Bill, ReceivePayment,
  JournalEntry, Estimate, SalesReceipt, CreditMemo, PurchaseOrder, Class, Terms,
  SalesRep, PaymentMethod, Check, Deposit, OtherName, PayrollItemWage,
  TimeTracking, BillPaymentCheck. **Missing**: CreditCardCharge, CreditCardCredit,
  Transfer, VendorCredit, ItemReceipt, BillPaymentCreditCard,
  SalesTaxPaymentCheck, InventoryAdjustment, Charge, SalesOrder, Paycheck,
  LiabilityCheck, BuildAssembly, other item types (Group, Discount, Payment,
  SalesTax, OtherCharge, Subtotal, Assembly, FixedAsset), SalesTaxCode,
  ItemSalesTax. Until the bridge grows these, the parity check will show the gap
  per account.
- **Bridge list responses omit line items**; `GET /{entity}/{id}` includes them
  (`ExpenseLineRet`, `ItemLineRet`, `InvoiceLineRet`, `JournalDebitLine`, …).
  A full import is one list call plus one GET per transaction.
- **qbXML → JSON quirk**: a repeated element is an array when there are several
  and a bare object when there is one (`JournalCreditLine` above was an object,
  `JournalDebitLine` an array). The normaliser must accept both everywhere.
- All scalars arrive as strings (`"true"`, `"27505.49"`). Times carry the QBD
  machine's offset (`-08:00` even in October, i.e. QBD reports standard time).
- List params: `modified_after` (ISO datetime), `from_date`/`to_date`,
  `max_returned`, `iterator_id` (`Start`, then `meta.iterator_id`).
- Account list carries `Balance`/`TotalBalance`: a ready-made parity oracle.
- GL report rows have no TxnID, so they cannot fill missing transaction types;
  useful only for totals.
- `/api/v1/status` reported `qb_desktop_running:false` while queries still
  succeeded; don't gate on that field.

- **Compute budget:** the machine is shared (6 cores). Every cargo/test run
  goes through `node ~/.claude/bin/cpu-slots.mjs run --slots 4 --label … -- …`.
  Cargo's build dir is shared machine-wide (`~/.cargo/build`), so parallel
  worktree builds take turns on its lock.
- `ts-rs` exports TypeScript into `web/src/api/generated/` whenever
  `cargo test` runs (`.cargo/config.toml` sets `TS_RS_EXPORT_DIR`); commit them.
- `MemoryBooks` helper methods are `add_*`: inherent methods named like the
  `PostingContext` trait's (`account`, `item`) lose to the trait's `&self`
  methods at autoref, which gave baffling arity errors.
- **Two crates can export the same TypeScript type name** and ts-rs silently
  overwrites one with the other (lb-feeds' and lb-reports' `Column` collided at
  merge, 2026-10-19). Rename on the TS side with `#[ts(export, rename = "…")]`.
- **cpu-slots broker** sometimes fails with a transient `EPERM` on its mutex
  file under contention (Windows). Rerun.
- **Passkeys and addresses:** a passkey is bound to a hostname; browsers refuse
  IP addresses as RP IDs and need HTTPS or `http://localhost`. LAN-IP clients
  use device keys. People enrol once per hostname they use (details in
  `plans/auth.md`).

- **First full import of the real company (2026-10-19, 42 min, ~8,100 GETs):**
  12,900 transactions, 2 failures, 143/159 accounts match QBD to the cent.
  Every account that is off sits behind a bridge gap (card charges, transfers,
  statement charges) except two the import itself explained: payments QBD
  applied to a *check* in A/R (fixed: checks/deposits/card charges in A/R or
  A/P are open items, `9f68491`), and 55 invoices QBD settled with credit memos
  the bridge never links (fixed: targeted auto-apply during import). Report
  JSON under `%LOCALAPPDATA%\LibreBooks-dogfood\imports\`; never commit it.
- **Blanket auto-apply is wrong for an import:** a credit QBD left unapplied
  (because it paid a statement charge we can't see) got applied to an open
  invoice, creating a new mismatch. The import applies only to charges QBD
  shows more settled than ours, by the shortfall. The fixture test caught it.

- **Shared build dir, parallel worktrees:** `cargo clippy` in the main checkout
  reported `lb_store` had no `saved`/`merge`/`apply` modules while `cargo test`
  on the same sources passed. An agent worktree was building an older
  `lb-store` into the machine-wide `~/.cargo/build` at the same time and its
  check artifact won. `cargo clean -p lb-store` and rerunning fixed it. When
  clippy and test disagree, suspect this before the code.

## Progress log

- [x] Survey environment, sibling projects, QB bridge shape.
- [x] Phase 0: repo skeleton (`858b1be`); API contract `plans/api.md` (`091efac`).
- [x] CI workflow (`c74ff31`); release workflow on tags + Dockerfile (`4058946`).
      CI green on all three jobs (Rust Windows, Rust Linux, Web UI) at `5a2aa32`
      after fixing the first runs' three failures. Release workflow still
      unexercised until the first tag.
- [x] Phase 1 core: lb-core posting rules for all 25 types (18 rule tests),
      lb-store company file (15 integration tests); reconciliation, opening
      balances, attachments (`ac5475c`); inventory re-costing on back-dated purchases
      (`inventory.rs`); JEs, checks, card charges and deposits in A/R-A/P as
      open items (`9f68491`, `ff88706`).
- [x] Phase 2: server API (`84edc4c`), auth integration (`49063f0`), reports API
      (`c33ee6c`), server ops (`ef1d3e4`, `5427aba`), QBD import routes (`1f8fa94`).
- [x] Phase 3: QBD read-only import + parity (`a0a7559`, `b580484`); Web
      Connector and SDK transports + import routes merged. Full import
      of the real company done; after the open-item fixes (`9f68491`, `55e9c3a`,
      `ff88706`) the rerun has 0 failures, 0 open-balance mismatches, 145/159
      accounts exact. The 14 still off are all bridge gaps (card charges,
      transfers): see question 7.
- [~] Phase 4: web UI foundation merged (`web UI`, 157 tests, Playwright);
      other forms, reports catalog UI, admin screens, register editing next.
- [x] Phase 5: desktop merged (`5951bd2`): local + thin-client modes, NSIS built.
- [x] Phase 6: backups, self-update, tunnel: libraries, server routes, CLI (merged).
      Signing keys in GitHub secrets (`42946a7`); release workflow untested
      until the first tag.
- [x] Phase 7: feeds library and routes (merged); feeds UI pending (phase B).
- [ ] Phase 8: parity tail.
- [ ] Phase 9: native QBW research.

## Open questions for the user

1. ~~Publish where?~~ Answered 2026-10-20: public `github.com/cinderblock/LibreBooks`, pushed.
2. **Name.** "LibreBooks" is the working name; other unrelated projects have used
   it (e-book readers). Fine for now, worth a check before a 1.0.
3. **Bank aggregator.** Recommendation: SimpleFIN first (≈$15/yr, the user holds
   the access token, no developer approval). Plaid needs a developer account and
   production approval per deployment.
4. ~~Feeds and the cleared mark~~ Answered 2026-10-20 ("I trust your rec"): a
   separate downloaded flag (migration 3, `b084b6f`); Cleared still means reconciled.
5. ~~Auto-add strictness~~ Kept strict (explained to the user 2026-10-20; no
   objection).
6. ~~Release signing key~~ Answered 2026-10-20 ("Do it. Keep all the keys on gh
   secrets"): keys generated, private halves only in GitHub secrets (`42946a7`).
7. ~~Bridge gaps~~ Answered 2026-10-20: importing must not require the bridge,
   so all three transports are supported; the Web Connector/SDK carry full qbXML
   and should close the 14 gaps. Still to do with the user: the live Web
   Connector sync on cad-3 (steps in `plans/qbd-import.md`); any tunnel or proxy
   for it needs per-change authorisation.

## Things not to do

- Never call a non-GET endpoint on the QB Bridge, and never connect the Web
  Connector transport in a mode that sends add/mod requests, until the user
  decides QBD is retired.
- Never commit real company data. `data/` and `*.lbdb` are gitignored; test
  fixtures are synthetic.
- Don't use floating point for money anywhere (Rust or TS). TS uses integer cents
  in `number` (safe to ±$90 trillion) and formats at the edge.
- The repo is public now; pushing `master` is routine, but never force-push.
- Don't rename `master`.
