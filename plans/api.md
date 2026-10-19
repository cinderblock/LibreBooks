# LibreBooks HTTP API (v1 contract)

The JSON API the web UI, the desktop app and third parties use. Served by
`lb-server` under `/api/v1`. This document is the contract between the server
and the UI; change both together.

## Conventions

- **JSON** in and out. `Content-Type: application/json`.
- **Money** is an integer number of cents (`12345` = $123.45). Never a float,
  never a string.
- **Quantities, rates, percents** are decimal strings (`"1.5"`, `"220.00"`).
- **Dates** are `YYYY-MM-DD`. **Timestamps** are RFC 3339 UTC.
- **Ids** are UUID strings.
- **Types** are generated from Rust into `web/src/api/generated/` (ts-rs). Use
  them; don't hand-write shapes that exist there.
- **Optimistic concurrency.** Records carry `edit_seq`. Updates and deletes send
  the `edit_seq` they were read with; a stale one gets `409`.
- **Errors**: `{ "error": { "code": string, "message": string, "detail"?: any } }`

  | HTTP | code | When |
  | --- | --- | --- |
  | 400 | `invalid` | Bad input (message says what) |
  | 401 | `unauthenticated` | No or expired session |
  | 403 | `forbidden` | Role does not allow it |
  | 404 | `not_found` | |
  | 409 | `conflict` | Stale `edit_seq` (detail: `{expected, actual}`) or duplicate name (`duplicate`) |
  | 409 | `in_use` | Delete of something referenced |
  | 422 | `posting` | Posting rules refused it (detail: a `PostingError`) |
  | 423 | `closed_period` | Date on or before the closing date (detail: `{closing_date}`) |
  | 429 | `rate_limited` | |
  | 500 | `internal` | |

- **Auth**: a session cookie `lb_session` (HttpOnly, Secure except on loopback,
  SameSite=Strict). State-changing requests also need header
  `X-LibreBooks: 1` (CSRF guard; browsers can't send it cross-site without CORS).
  Desktop local mode uses a per-launch bearer token instead
  (`Authorization: Bearer <token>`).

## Server-level

| Method & path | Body → Response |
| --- | --- |
| `GET /health` | → `{ ok: true, version, mode: "server" \| "desktop" }` (no auth) |
| `GET /session` | → `{ user: {id, display_name, role}, companies: [{id, name, role}] }` or 401 |
| `POST /auth/login/start` | → `{ challenge_id, options }` (WebAuthn request options JSON for `@simplewebauthn/browser` `startAuthentication`) |
| `POST /auth/login/finish` | `{ challenge_id, credential }` → sets cookie, `{ user }` |
| `POST /auth/enroll/start` | `{ token }` → `{ challenge_id, options, purpose, display_name? }` (`startRegistration` options) |
| `POST /auth/enroll/finish` | `{ token, challenge_id, credential, display_name, passkey_name }` → sets cookie, `{ user }` |
| `POST /auth/device/challenge` | → `{ challenge_id, challenge }` (device-key login, thin clients) |
| `POST /auth/device/login` | `{ challenge_id, key_id, signature }` → sets cookie |
| `POST /auth/device/enroll` | `{ token, public_key, name }` → `{ key_id }` |
| `POST /auth/logout` | → 204 |
| `GET /auth/passkeys` | → my passkeys `[{id, name, created_at, last_used_at}]` |
| `DELETE /auth/passkeys/:id` | → 204 |
| `GET /auth/sessions` / `DELETE /auth/sessions/:id` | my signed-in devices |
| `POST /auth/enrollments` | admin: `{ purpose: "new_user" \| "add_device", role?, user_id?, ttl_minutes? }` → `{ token, url, expires_at }` |
| `GET /users`, `PATCH /users/:id`, `DELETE /users/:id` | admin user management |
| `GET /companies` | → `[{ id, name, role }]` |
| `POST /companies` | admin: `{ info: CompanyInfo }` → `{ id }` |
| `GET /events?company=:id` | Server-Sent Events: `event: change`, `data: {kind, id, action}` |
| `GET /server` / `PUT /server` | admin: server settings (bind address, public URL, tunnel, updates, backups) |
| `POST /server/update/check`, `POST /server/update/install` | admin |
| `GET /server/tunnel` | admin: cloudflared status |

## Company-scoped: `/api/v1/c/:company`

### Company

| | |
| --- | --- |
| `GET /info` / `PUT /info` | `CompanyInfo` |
| `GET /closing-date` / `PUT /closing-date` | `{ closing_date: string \| null }` (admin) |

### Lists

Each list has the same five routes. `:list` is one of `accounts`, `entities`,
`items`, `classes`, `terms`, `payment-methods`, `sales-tax-codes`,
`sales-reps`, `types`.

| | |
| --- | --- |
| `GET /:list?include_inactive=true` | → array (entities also take `kind=Customer\|Vendor\|Employee\|OtherName`; items `type=`) |
| `GET /:list/:id` | → record |
| `POST /:list` | record (`edit_seq: 0`; id may be supplied) → saved record |
| `PUT /:list/:id` | record (with current `edit_seq`) → saved record |
| `DELETE /:list/:id?edit_seq=N` | → 204, or 409 `in_use` |

### Transactions

| | |
| --- | --- |
| `GET /txns?…` | `TxnFilter` as query: `types=Check,Bill&from=&to=&entity_id=&include_jobs=true&account_id=&number=&text=&amount=&open_only=true&include_void=true&limit=&offset=&descending=true` → `TxnSummary[]` |
| `GET /txns/:id` | → `{ txn: Txn, postings: Posting[] }` |
| `POST /txns` | `Txn` (`edit_seq: 0`) → `SaveOutcome { txn, postings, total, sales_tax, warnings }` |
| `PUT /txns/:id` | `Txn` → `SaveOutcome` |
| `POST /txns/preview` | `Txn` → `{ postings, total, sales_tax, warnings }` without saving (live totals, the "show postings" view) |
| `POST /txns/:id/void` | `{ edit_seq }` → `SaveOutcome` |
| `DELETE /txns/:id?edit_seq=N` | → 204 |
| `GET /txns/next-number?type=Check&account_id=` | → `{ number: string \| null }` |
| `GET /txns/:id/history` | → audit entries |

### Registers and balances

| | |
| --- | --- |
| `GET /accounts/:id/register?from=&to=` | → `{ opening: Money, lines: RegisterLine[] }` |
| `PUT /accounts/:id/register/:txn_id/cleared` | `{ status: ClearedStatus }` |
| `GET /balances?from=&to=` | → `AccountBalance[]` (debit-positive) |
| `GET /open?entity_id=&kind=receivable\|payable` | open invoices/bills/credits for a name (Receive Payments / Pay Bills) |
| `GET /undeposited` | payments waiting in Undeposited Funds (Make Deposits) |

### Reports

| | |
| --- | --- |
| `GET /reports` | → catalog `[{ id, name, category, params }]` |
| `POST /reports/run` | `ReportRequest` → `Report` |
| `POST /reports/export?format=csv` | `ReportRequest` → file |
| `GET /saved/reports`, `POST /saved/reports`, … | memorized reports |

### Audit, attachments, import, feeds, backups

| | |
| --- | --- |
| `GET /audit?record_kind=&record_id=&before_id=&limit=` | audit trail |
| `POST /attachments` (multipart) / `GET /attachments/:id` | receipts, documents |
| `POST /import/qbd/sync` / `GET /import/qbd/status` / `GET /import/qbd/parity` | QuickBooks import (admin) |
| `/feeds/…` | bank feeds (see `lb-feeds`) |
| `/backups/…` | list, run now, restore (admin) |
