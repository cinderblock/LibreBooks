# LibreBooks feeds (`crates/lb-feeds`)

## Goal

Get bank, credit card and other account activity into the books "as
automatically as possible", the way QuickBooks Desktop's **Bank Feeds** do:
download (or import a file), land every transaction in a review queue, rename and
categorise it with rules, **match** it to what is already entered, or **add** it
as a new Check / Deposit / Credit Card Charge / Credit Card Credit with one key.
Master plan decision 12 (`plans/librebooks.md`) fixes the order of sources:
files → SimpleFIN → OFX Direct Connect → Plaid/Teller (BYO keys) → Amazon →
Stripe/PayPal/Gusto.

## Environment / context

- Crate: `crates/lb-feeds`. Depends on `lb-core` (money, ids, `Txn`) and
  `lb-store` (`Books`, `Writer`, `txns::save`, `balances::set_cleared`).
- Tables (lb-store migration 2, already shipped): `feed_connection`,
  `feed_txn` (`UNIQUE(account_id, fitid)`), `feed_rule`. No schema change was
  needed for this crate.
- Fixtures: `crates/lb-feeds/testdata/` (marked `-text` in `.gitattributes`, so
  CRLF, BOMs and Windows-1252 bytes survive checkout exactly). All synthetic.
- TypeScript types: ts-rs, exported to `web/src/api/generated/` by
  `cargo test -p lb-feeds export_bindings`.
- Reference only (do not modify): `C:\Users\camer\git\Personal Projects\amazon-transaction-downloader`
  (Puppeteer scrape of `amazon.com/cpe/yourpayments/transactions`).

## Decisions (don't re-ask)

1. **One `Statement` shape for every source.** OFX, CSV, SimpleFIN and Direct
   Connect all produce `Statement { account, balances, start, end, transactions }`.
   Everything downstream (dedupe, rules, matching, add) only sees that.
2. **Sign convention: `+` is money into the account**, from the account
   holder's side. A deposit is `+`, a check `-`, a card purchase `-`, a card
   payment or refund `+`. This is OFX's and SimpleFIN's convention, and it is
   exactly the debit-positive posting a matching register line has in that
   account (bank: deposit debits the asset; card: payment debits the liability),
   so matching compares the feed amount to the posting sum directly.
3. **Dates are the bank's calendar date.** `20261001000000[0:GMT]` is
   2026-10-01, not 2026-09-30 in Pacific time. The time and zone are kept in
   `raw` but never shift the date: banks stamp postings with midnight in
   whatever zone their mainframe uses, and converting moves transactions across
   days (and months, at statement boundaries).
4. **Character sets:** strip a UTF-8 BOM; honour UTF-16 BOMs; then if the bytes
   are valid UTF-8 use them as UTF-8 regardless of what the header claims (banks
   routinely declare `CHARSET:1252` and send UTF-8), otherwise decode as
   Windows-1252 (a superset of Latin-1 and of US-ASCII), via `encoding_rs`.
5. **Account numbers are never stored whole.** `account_id_masked` keeps the
   last four characters; `account_key` is a SHA-256 fingerprint of
   `bank_id|account_id` used to remember which LibreBooks account a file belongs
   to.
6. **Synthesised FITIDs** (CSV, and any OFX/SimpleFIN record without an id):
   `lb1:` + first 16 hex chars of SHA-256 over `date|amount|normalised payee` +
   `:` + the occurrence number of that same triple within the statement (in
   file order). Re-importing the same file, or an overlapping later file, gives
   the same ids, so dedupe on `(account_id, fitid)` works; two genuinely
   identical purchases on one day stay distinct (`:0`, `:1`). The prefix
   versions the scheme. Limitation: if a bank *edits* the description of an
   already-downloaded transaction the synthesised id changes and it shows up
   again (as a likely match for the first copy, so it is easy to ignore).
7. **Matching marks the register line Cleared** with
   `lb_store::balances::set_cleared(…, ClearedStatus::Cleared)` (the brief for
   this crate asks for that). QBD itself shows a separate "downloaded"
   lightning bolt and leaves the reconcile check mark alone; if that distinction
   is wanted later it needs a column on `posting`, i.e. a new migration.
8. **Providers are a dyn-compatible trait** returning a boxed future
   (`FeedProvider::fetch(&self, since) -> BoxFuture<Result<Vec<Statement>>>`)
   so the server can hold `Box<dyn FeedProvider>` per connection. Native
   `async fn` in traits is not dyn-compatible yet.
9. **Secrets never live in the company file.** Connection settings hold a
   `SecretRef` (an opaque key); the server resolves it through the `Secrets`
   trait it implements (OS keychain, server secret store…). `Secret` has a
   redacting `Debug` and no `Display`, URLs with credentials are split into a
   bare URL plus basic-auth before any request, and reqwest errors are stripped
   of their URL before they are turned into messages.
10. **Rules: first match wins**, in priority order (QBD's renaming rules work
    the same way), so a person can predict what happens. When no rule matches,
    the payee's history (how the last feed transaction with the same cleaned-up
    payee was added) supplies the suggestion.
11. **Order of automation on ingest:** import (dedupe) → rules → match →
    auto-add. A transaction that matches something already entered is never
    auto-added, even when a rule says auto-add.

## Plan / steps

- [x] Read master plan, API plan, lb-core, lb-store (schema migration 2, txns, balances).
- [x] `Statement` model, OFX parser (1.x SGML + 2.x XML, QFX, QBO), fixtures.
- [x] CSV mappings, parser, layout auto-detection (Chase, BofA, Amex, Apple Card,
      Capital One, Wells Fargo, Discover, generic).
- [x] Feed store: connections CRUD, import with dedupe, statuses, queries.
- [x] Rules: conditions, actions, CRUD, application, history suggestions.
- [x] Matching: candidates, scoring, auto-match, accept/unmatch.
- [x] Add: Check / Deposit / CC Charge / CC Credit, batch add.
- [x] Providers: trait, secrets, SimpleFIN (claim + fetch, mock server test),
      OFX Direct Connect (request builder snapshot, mock server test).
- [x] Amazon order-history CSV parser + charge matcher + annotate.
- [x] ts-rs export, clippy pedantic clean, fmt, commit.
- [ ] ▶ Next (other crates/threads): `/feeds` routes in lb-server (proposal
      below), the Bank Feeds UI, a scheduler that calls `providers::sync`.

## Module map (`crates/lb-feeds/src`)

| Module | What it holds |
| --- | --- |
| `statement` | `Statement`, `StatementAccount`, `StatementTxn`, `AccountKind`, `TrnType`, `Direction`; fitid synthesis, duplicate-id settling, fingerprints |
| `text` | byte decoding, `parse_amount`, `parse_ofx_date`, `parse_date` + `DateOrder`, `clean_payee`, `similarity`, `mask_account` |
| `ofx` | tolerant SGML/XML tree reader, `parse` (→ `OfxDocument`), `parse_statements`, `OfxStatus` |
| `csvfile` | `CsvMapping`, `Column`, `AmountColumns`, `parse`, `detect` (→ `CsvDetection`, `CsvLayout`) |
| `queue` | `FeedTxn`, `FeedStatus`, `Suggestion`, `import_statement`, `query`/`FeedFilter`, `summary`, `ignore`, `undo`, `set_memo` |
| `rules` | `FeedRule`, `Condition`, `RuleActions`, CRUD, `reorder`, `RuleSet`, `History`, `apply_to`/`apply_all` |
| `matching` | `score`/`ScoreInput`, `candidates`, `accept`, `auto_match`, `STRONG` |
| `add` | `plan` (preview), `add`, `add_all` (savepoint per row), `txn_type_for` |
| `ingest` | `ingest` (import → rules → match → auto-add), `ingest_for_connection` |
| `connections` | `FeedConnection`, `ProviderSettings`, CRUD, `record_sync`, `by_account_key` |
| `secrets` | `SecretRef`, `Secret` (redacted), `Secrets` trait, `MemorySecrets` |
| `providers` | `FeedProvider` trait, `http_client`, `for_connection`, `since_for`, `sync`; `simplefin`, `ofx_direct` |
| `amazon` | `parse_order_history`, `match_charges`, `annotate` |
| `lib` | `write`/`read` helpers that keep `FeedError` through `Books::write`, `sniff` |

## Proposed `/api/v1/c/:company/feeds` routes (for lb-server)

Not built here (lb-server is another crate). Every body/response type below
is exported to `web/src/api/generated/`.

| Method & path | Body → Response | lb-feeds call |
| --- | --- | --- |
| `GET /feeds` | → `FeedAccountSummary[]` | `queue::summary` |
| `GET /feeds/txns?…` | `FeedFilter` as query → `FeedTxn[]` | `queue::query` |
| `POST /feeds/detect` | file (multipart) → `{ kind: FileKind, csv?: CsvDetection, statements?: Statement[] }` | `sniff`, `csvfile::detect` or `ofx::parse_statements` |
| `POST /feeds/import` | file + `account_id` + `connection_id?` + `mapping?` → `IngestReport` | parse, then `ingest::ingest` / `ingest_for_connection` |
| `GET /feeds/txns/:id/candidates` | → `MatchCandidate[]` | `matching::candidates` |
| `POST /feeds/txns/:id/match` | `{ txn_id }` → 204 | `matching::accept` |
| `POST /feeds/txns/:id/preview` | `AddRequest` → `Txn` + postings (via `/txns/preview`) | `add::plan` |
| `POST /feeds/txns/:id/add` | `AddRequest` → `AddOutcome` | `add::add` |
| `POST /feeds/txns/add-all` | `{ ids, request? }` → `BatchOutcome` | `add::add_all` |
| `POST /feeds/txns/ignore` | `{ ids }` → `{ ignored }` | `queue::ignore` |
| `POST /feeds/txns/:id/undo` | → `FeedTxn` | `queue::undo` |
| `GET/POST/PUT/DELETE /feeds/connections[/:id]` | `FeedConnection` | `connections::*` |
| `POST /feeds/connections/:id/sync` | → `IngestReport` | `providers::for_connection` + `providers::sync` |
| `POST /feeds/simplefin/claim` | `{ setup_token }` → accounts to connect | `simplefin::claim`, then `SimpleFin::accounts`; the server stores the access URL as a secret |
| `POST /feeds/connections/:id/remote-accounts` | → `StatementAccount[]` | `OfxDirect::accounts` |
| `GET/POST/PUT/DELETE /feeds/rules[/:id]`, `POST /feeds/rules/reorder`, `POST /feeds/rules/apply` | `FeedRule` | `rules::*` |
| `POST /feeds/amazon/orders` | file → `AmazonMatch[]` (preview) | `amazon::parse_order_history` + `match_charges` |
| `POST /feeds/amazon/apply` | `AmazonMatch[]` → `{ annotated }` | `amazon::annotate` |

Server responsibilities: implement `Secrets` (OS keychain / encrypted file),
run `providers::sync` on a schedule (`spawn_blocking` is not needed for the
fetch, which is async; the ingest is a short write), and map `FeedError`:
`Parse`/`Invalid` → 400, `NotFound` → 404, `Unauthorized` → 401-ish
`reconnect` state on the connection (not the user's session), `Provider` /
`Network` → 502, `Unsupported` → 501, `Store(..)` → as the store's errors.

## Provider comparison

| Provider | Cost | Coverage | Who holds the credentials | Status |
| --- | --- | --- | --- | --- |
| File import (OFX/QFX/QBO/CSV) | Free | Any bank with a download button | Nobody (person downloads) | Implemented |
| SimpleFIN Bridge | ~$15/yr per user (bridge fee), free protocol | US banks via MX behind the bridge; thousands of institutions | The **user** holds an access URL with basic-auth creds; the bridge holds bank logins | Implemented |
| OFX Direct Connect | Free at most banks that still offer it; some charge ~$3–10/mo | Shrinking: many big banks moved to Intuit's proprietary Web Connect / Express Web Connect; still common at credit unions, Amex, some brokerages | The **user's bank username/password** (or a Direct Connect PIN) stored by us | Implemented (client) |
| Plaid | Free dev tier; production needs approval and per-item fees | Best US/CA/EU coverage | Plaid holds bank tokens; we hold `client_id`/`secret` + per-item `access_token` (user's own Plaid account) | Stub / notes |
| Teller | Free dev tier (100 enrolments); paid production | US major banks via direct APIs, very reliable where supported | Teller holds bank sessions; we hold a client certificate + per-enrolment access token | Stub / notes |
| Stripe | Free API | Payouts, charges, fees | Restricted API key (read-only) | Notes |
| PayPal | Free API | Transactions (Reporting API, 3-year window) | REST client id/secret | Notes |
| Amazon | Free | Orders (not money movement) | Session cookie (scrape) or none (data export file) | Order-history CSV parser + matcher |

### Notes per stub

- **Plaid**: `/link/token/create` → Link in the browser → `public_token` →
  `/item/public_token/exchange` → `access_token`; then `/transactions/sync`
  with a cursor (store the cursor in connection settings). Amounts are
  **positive for money out** (opposite of ours): negate. `transaction_id` is the
  fitid; `pending_transaction_id` links a posted txn to its pending one. BYO
  keys: the user creates a Plaid account and pastes client id + secret; the
  server needs the Link flow page.
- **Teller**: Teller Connect in the browser yields an access token; requests use
  mutual TLS with the developer's certificate (rustls supports client certs).
  `GET /accounts/{id}/transactions`; amounts are signed from the holder's side
  (negative = debit) — same as ours. `id` is the fitid.
- **Stripe**: `GET /v1/balance_transactions` (charges, refunds, fees, payouts):
  a "Stripe" clearing account (Other Current Asset) gets gross charges as
  deposits and fees as checks; payouts become transfers to the bank, which then
  match the bank feed's deposit.
- **PayPal**: Transaction Search API (`/v1/reporting/transactions`, 31-day
  windows, 3 years back). Same clearing-account pattern as Stripe.

## Amazon design

Amazon is not a money account; it explains card charges. Two sources:

1. **"Request your data" → Your Orders** (`Retail.OrderHistory.1.csv`, one row
   per item, with `Order ID`, `Order Date`, `Ship Date`, `Total Owed`,
   `Shipment Item Subtotal`, `Shipment Item Subtotal Tax`, `Product Name`,
   `Payment Instrument Type`, `Carrier Name & Tracking Number`). Free,
   no scraping, but manual and can lag days.
2. **Transaction history scrape** (what `amazon-transaction-downloader` does):
   `amazon.com/cpe/yourpayments/transactions` lists each *card charge* with its
   date, amount, payment method (`Visa ****1234`) and order number. This is the
   best signal because Amazon charges per **shipment**, not per order, and this
   page lists exactly the charges. Needs a logged-in browser session (Puppeteer
   in that project); in LibreBooks it would be an optional desktop-only helper.

Matching charges to orders (`amazon::match_charges`, implemented as a pure
function): for each card feed transaction whose payee looks like Amazon
(`AMAZON`, `AMZN`, `Amazon.com`, `AMZN Mktp`), find orders/shipments whose
amount equals the charge (orders first, then per-shipment groups: same order,
same ship date), dated from 2 days before to 10 days after the order date (the
downloader used ±6 days on transaction dates); take the closest by date; an
order is used once. Refunds (positive card amounts) match negative
"refund" rows only from the transaction scrape (the order export has no refund
amounts), so they are left for the person.

Attaching: on match, the feed transaction's memo becomes
`Amazon 111-2223334-4445556: Product A; Product B` and `raw.amazon` keeps the
order (items, quantities, link `https://www.amazon.com/gp/your-account/order-details?orderID=…`).
When the person adds it, that memo lands on the Credit Card Charge, so the
register explains every Amazon line. Later: split the charge across items'
expense accounts by rules on product names, and attach the printable order
summary PDF as an attachment (the downloader's `createPDF.ts` shows how).

## Matching algorithm

Candidates for a feed transaction `f` in account `A`:

- register transactions with postings in `A` whose sum in `A` equals
  `f.amount` exactly (sign-aware: deposits match deposits, never checks);
- dated within `±match_days` (default 10) of `f.posted`;
- not void, not already Cleared or Pending in `A`, and not already linked to
  another feed transaction (matched or added).

Score (0–100, clamped):

| Signal | Points |
| --- | --- |
| Exact amount (required) | 40 |
| Date distance `d` days of `N` | `30 × (1 − d/(N+1))` |
| Both have check numbers and they are equal | +30 |
| Both have check numbers and they differ | −40 |
| Payee similarity (token Dice coefficient of cleaned feed payee vs. the register line's name, memo) | up to +20 |

A candidate is **strong** at ≥ 65. Auto-match happens only when exactly one
candidate is strong, and that register line is not also the single strong
candidate of another feed transaction in the same batch (then both wait for a
person). Accepting a match (auto or by hand) sets the feed transaction to
`matched` with `matched_txn_id` and marks the register line Cleared.

## Findings / gotchas

- `feed_txn.matched_txn_id REFERENCES txn(id)` with no cascade: deleting a
  transaction that a feed line points to fails with a foreign-key error, so
  `undo` on an added line clears the link before deleting the transaction.
- `feed_txn.connection_id REFERENCES feed_connection(id)`: deleting a connection
  first detaches its feed transactions (history stays).
- `lb_store::txns::Query` is `pub(crate)`; this crate builds its own SQL.
- `Books::write`'s closure must return `lb_store::Result`, which would flatten
  every `FeedError` into `StoreError::Invalid(text)`. `lb_feeds::write`/`read`
  capture the real error outside the closure and return it after the rollback.
- `add_all` uses raw `SAVEPOINT`/`ROLLBACK TO` inside the store's write
  transaction (a `Writer` can't be built over a rusqlite `Savepoint`). A failed
  row leaves no rows behind; its change notification may still be in the
  `Writer`'s list, so listeners can hear about a transaction that was rolled
  back (harmless: they re-read).
- Amex's CSV wraps its reference in apostrophes on both sides
  (`'320262850123456781'`) so Excel keeps it as text; both are stripped.
- Bank of America's checking CSV has a five-row summary and a
  "Beginning balance" row with no amount above its transactions; detection
  skips the summary by finding the header row, and rows with a date but no
  amount are skipped quietly (they're balance lines, never transactions).
- Line breaks inside quoted CSV cells arrive as the file's line ending; they
  are normalised to `\n`.
- `ts-rs` export tests write into `web/src/api/generated/` whenever
  `cargo test -p lb-feeds` runs (the workspace sets `TS_RS_EXPORT_DIR`), and
  re-export lb-core types they import with LF endings. Git normalises those
  back (no content change), so only new files show up in a commit.
- The root `package.json` `gen:types` script lists `-p lb-core -p lb-store
  -p lb-reports`; it should also list `-p lb-feeds` (not changed here: outside
  this crate; flagged for the parent session).
- No `node_modules` in the worktree: the generated types were checked with a
  throwaway `bunx -p typescript@5 tsc --noEmit --strict` over the generated
  folder (passes).

## Progress log

- [x] Statement + OFX — 7 fixture files (SGML/CRLF/1252, XML/BOM/QFX, one-line
      QBO with missing/duplicate FITIDs, multi-account with an error status and
      an investment block, sign-on error, lower-case/tabs/decimal comma).
- [x] CSV + detection — 12 fixtures (Chase ×2, BofA ×2, Amex with multi-line
      cells, Apple Card, Capital One ×2, Wells Fargo without a header, Discover,
      semicolon/day-first/decimal-comma/1252, tab-separated).
- [x] Feed store — import dedupe (bank ids and synthesised), start date,
      reversed signs, query/summary/ignore/undo.
- [x] Rules — first match wins, regex/range/direction, validation, reorder,
      history suggestions.
- [x] Matching — scoring table test; end-to-end auto-match of a check (by
      number), a deposit and a named payee; ambiguity held back both ways.
- [x] Add — Check, Deposit, CC Charge, CC Credit; balances verified with
      `check_integrity` and account sums; Add All with partial failure; undo.
- [x] Providers — SimpleFIN claim/fetch/sync against a loopback bridge; Direct
      Connect request snapshots + mock OFX server.
- [x] Amazon — order-history parser, order/shipment matching, annotate → memo.
- [x] Types exported, clippy pedantic clean, fmt.

Commits on `worktree-agent-a4a0c4227d387787c`: `de88d57` (crate), `6370cb9`
(provider tests), `69528b8` (TypeScript types), then this plan.

## Gaps (known, not done)

- Cross-source dedupe: the same transaction imported once from a QFX file and
  again from SimpleFIN has two different fitids and lands twice. Once the
  first copy is matched or added, its register line is *excluded* from the
  second copy's candidates (it's linked), so the second copy looks new and a
  person has to ignore it. Fix later: flag a new row whose date, amount and
  cleaned payee equal a linked row from another source as "probably a
  duplicate of …".
- OFX `CORRECTACTION` (REPLACE/DELETE) is reported as a warning, not applied.
- OFX Direct Connect multi-factor (`MFACHALLENGERQ`) and `USERKEY` sessions are
  not implemented; banks that require them fail with the bank's message.
- Investment statements (`INVSTMTMSGSRSV1`) are skipped with a warning.
- Currency: a statement's `CURDEF`/currency is recorded but not checked; `add`
  creates home-currency transactions.
- Adding into liability registers other than credit cards (loans, lines of
  credit) is refused as unsupported; those downloads can be matched to
  transactions entered by hand.
- Splits: `add` makes a one-line transaction. A split add (several expense
  lines, as QBD's "Add More Details") is a UI that builds a `Txn` and then
  `matching::accept`s it.
- Transfers: a payment from checking to a card added on both sides makes two
  transactions; the better flow is to add it once (as a CC Credit from
  Checking, or a Transfer) and *match* the other side. Matching already finds
  it; a "this is a transfer" shortcut in the UI would help.
- SimpleFIN's `posted` is read as a UTC date; a bridge that stamps local
  midnight in a zone west of UTC would shift dates by one (none seen yet).
- Plaid and Teller: settings variants exist; `for_connection` returns
  `Unsupported`. Stripe, PayPal: notes only.
- Amazon: refunds aren't matched (the order export has no refund amounts); the
  transaction-history scrape (desktop-only helper) is design only.

## Open questions for the user

1. **Cleared vs "downloaded".** Decided 2026-10-20 (user: "I trust your rec"):
   matching or adding a download sets a separate `downloaded` mark on the
   register line (lb-store migration 3, `balances::set_downloaded`,
   `RegisterLine.downloaded`), shown as QBD's lightning bolt. Cleared keeps
   meaning reconciled. Candidates for matching exclude lines already matched to
   a download, not cleared ones.
2. **Auto-add default.** Rules only add by themselves when the rule says so
   and nothing in the register resembles the download. Keep it that strict?
   Recommendation: yes.

## Things not to do

- Don't log or put in error messages: SimpleFIN access URLs, OFX passwords,
  Plaid/Teller tokens.
- Don't convert OFX timestamps to local time (decision 3).
- Don't hit the real internet in tests: SimpleFIN and Direct Connect tests run
  against local mock servers.
- Don't commit real statements; fixtures are synthetic.
