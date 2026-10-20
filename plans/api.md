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
| `POST /companies/import` | admin: multipart `file` (a `.lbk` from `GET /c/:company/backups/export` or the desktop app), `name?` → 201 `{ id, name, from: PortableManifest }`: a **new** company; nothing existing is replaced. 400 without a file; 422 `damaged` for a file that isn't a sound `.lbk`. Streams to disk; up to 8 GiB. |
| `GET /events?company=:id` | Server-Sent Events: `event: change`, `data: {kind, id, action}` |
| `GET /server` / `PUT /server` | admin: server settings (see "Server administration") |
| `POST /server/update/check`, `POST /server/update/install` | admin |
| `GET /server/tunnel` | admin: cloudflared status |

### Server administration (admin only)

Settings live in `server.json` in the data folder; credentials never do (they
are references: `secret:<name>`, `env:NAME`, `file:/path`). Details and reasons:
`plans/server-ops.md`.

| Method & path | Body → Response |
| --- | --- |
| `GET /server` | → `{ version, data_dir, settings: ServerSettings, secrets: [{name, updated_at}], update: UpdateView, tunnel: TunnelView, backups: [{company_id, company_name, status: SchedulerStatus}], restart_required: string[], running }` |
| `PUT /server` | `ServerSettings` (whole) → same as `GET`. 400 `invalid` with `detail.problems: string[]` when something must be fixed first (a missing secret, a bad URL…). |
| `GET /server/secrets` | → `[{ name, updated_at }]`. Never values. |
| `PUT /server/secrets/:name` | `{ value }` → 204. Write-only. Names `[A-Za-z0-9._-]{1,128}`. |
| `DELETE /server/secrets/:name` | → 204; 404 if none; 409 `in_use` while the settings refer to it |
| `GET /server/update` | → `UpdateView { config, status: UpdateStatus \| null, unavailable: string \| null }` (`unavailable` says why, e.g. this build has no release signing key) |
| `POST /server/update/check` | → `UpdateView`. 409 `unavailable` without an updater; 502 `network` / `untrusted` |
| `POST /server/update/install` | `{ restart?: true }` → `{ installed: Installed, restarting }`. 409 `up_to_date`. With `restart`, the server shuts down gracefully and starts the new version (or exits 75 for its supervisor). |
| `GET /server/tunnel` | → `TunnelView { enabled, public_hostname, public_url, token_ref, token_set, tunnel_id, cloudflared_path, auto_download, cloudflared, preparing_since, status: TunnelStatus, problems, restart_required }` (`status.state.state` is `stopped` \| `starting` \| `connected` \| `reconnecting` \| `failed`) |
| `PUT /server/tunnel` | any of `{ enabled, public_hostname, token, cloudflared_path, auto_download }` (`""` clears a text field) → `TunnelView`. `token` is checked, saved as the secret `tunnel-token`, and never returned. |
| `GET /server/backups/status` | → `{ enabled, running, companies: [{company_id, company_name, status}] }` |

`ServerSettings`: `{ public_urls: string[], backups: BackupConfig, updates:
UpdateConfig, tunnel: TunnelConfig, access: {team_domain, audience: string[]} |
null, feeds: { auto_sync, every } }` (durations ISO 8601, e.g. `"PT6H"`). Changing
`public_urls` or the tunnel hostname needs a restart for passkeys
(`restart_required` says so).

**Cloudflare Access**: with `access` set, a request that came through Cloudflare
(it has `cf-ray`) without a valid `Cf-Access-Jwt-Assertion` gets 403 with
`detail.reason = "cloudflare_access"`. Direct (LAN, localhost) requests are not
affected. `/qbwc` (below) is outside this check.

### The QuickBooks Web Connector endpoint: `/qbwc` (outside `/api/v1`)

`POST /qbwc` is the SOAP 1.1 service the QuickBooks Web Connector talks to
(`text/xml`; methods `serverVersion`, `clientVersion`, `authenticate`,
`sendRequestXML`, `receiveResponseXML`, `getLastError`, `connectionError`,
`closeConnection`, namespace `http://developer.intuit.com/`). It has no
session cookie and no CSRF header: the Web Connector signs in with the user
name from the company's `.qwc` and the password set through
`POST /c/:company/import/qbd/qbwc/password`, compared in constant time.
`authenticate` answers `nvu` (bad credentials), `none` (nothing queued),
`busy` (another Web Connector run is in this job's session) or the company
file to open (empty = the one QuickBooks has open). `GET /qbwc` is a plain-text
page for checking the URL in a browser. Put the path on a Cloudflare Access
bypass policy if the hostname is behind Access.

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
| `GET /balances/names` | → `NameBalance[]`: every name with an open A/R or A/P item (`receivable`, `payable`, both positive = owed; credits and unapplied payments count against), with `*_total` rolling jobs up to their parent (the Customer/Vendor Centers' Balance and Balance Total) |
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
| `POST /attachments` (multipart) / `GET /attachments/:id` | receipts, documents. `GET /attachments` lists the inbox; `?record_kind=&record_id=` one record's; `?all=true` everything. `GET /attachments/:id` takes the session cookie, a bearer token, or `?token=` (for `<img>`), like every route |
| `/import/qbd/…` | QuickBooks Desktop import (admin; below) |
| `/feeds/…` | bank feeds (below) |
| `/backups/…` | list, run now, verify, restore, export (below) |

### QuickBooks Desktop import (admin on the company)

Three transports feed one importer (`plans/qbd-import.md`): the QB Bridge, the
QuickBooks Web Connector (QuickBooks polls this server) and the QuickBooks SDK
(LibreBooks on the QuickBooks machine). One job (sync or parity) runs per
company at a time, in the background; status and reports are read back, with
`?wait=N` (seconds, at most 120) to long-poll until the running job finishes.
Types `ImportSettings`, `QbdTransport`, `SyncReport`, `ParityReport`, `ImportStatus`, `RunningJob`, `JobKind`, `SyncBody`, `ParityBody`, `ParityStatus`,
`SyncProgress` are lb-qbd's (`web/src/api/generated/`).

| | |
| --- | --- |
| `GET /import/qbd` | → `ImportStatus { settings: ImportSettings, bridge_key_set, qbwc_password_set, sdk_available, running: RunningJob \| null, last_sync: SyncReport \| null, last_parity: ParityReport \| null, last_error }`. `RunningJob { kind: "sync" \| "parity", transport, started_at, progress: SyncProgress \| null, percent, waiting_for_web_connector, web_connector_connected }` |
| `PUT /import/qbd` | `ImportSettings` plus write-only `bridge_key?` (saved as the secret `qbd.<company>.bridge-key`; `bridge.key_ref` then points at it) → `ImportStatus`. 400 `invalid` with `detail.problems` (empty bridge URL, no key, bad app URL…); 409 `duplicate` when another company uses the Web Connector user name. `qbwc.owner_id` / `file_id` are minted once and can't be changed. |
| `POST /import/qbd/sync` | `{ full?, sweep?, reimport?, txn_limit?, txn_from?, txn_to?, apply_credits?, entities? }` (all optional) → 202 `ImportStatus`. 409 `running` while a job runs; 400 when the settings have problems; 501 `unsupported` for the SDK on a build without it; 502 `network` when the bridge can't be reached. Over the Web Connector the job waits for its next poll. |
| `GET /import/qbd/status?wait=N` | → `ImportStatus`; with `wait`, returns as soon as the job finishes **or reports progress** (else after N s, max 120) |
| `POST /import/qbd/parity` | `{ as_of?, trial_balance? }` → 202 `ImportStatus` (same 409/400 rules) |
| `GET /import/qbd/parity?wait=N` | → `ParityStatus { running, job: RunningJob \| null, report: ParityReport \| null, error }` |
| events | A job emits `change` events with `kind: "import"`, `id: "sync" \| "parity"`, `action: started \| progress \| finished \| failed` on `GET /events`; saving the settings or the Web Connector password emits `id: "settings", action: updated`; the imported records themselves arrive as their own change events while it runs. |
| `GET /import/qbd/qbwc.qwc` | → `application/xml` download `librebooks-<slug>.qwc` for the QuickBooks Web Connector (`AppURL` = the company's `qbwc.app_url`, else the server's first public URL, else the tunnel hostname, else the request's origin, + `/qbwc`; `IsReadOnly` true). 400 unless the URL is `https://` or `http://localhost`. |
| `POST /import/qbd/qbwc/password` | `{ value }` (≥ 8 characters) → 204, or no body → `{ password }`: a generated one, shown this once. Saved as the secret `qbd.<company>.qbwc-password`. |

### Backups (admin on the company)

Each company backs up into its own folder (named by company id) in every
destination of the server's backup settings.

| | |
| --- | --- |
| `GET /backups` | → `{ backups: [{ destination, destination_name, manifest: Manifest }], errors: string[] }` newest first |
| `GET /backups/status` | → `{ enabled, destinations: string[], scheduler: SchedulerStatus \| null }` |
| `POST /backups/run` | → `BackupRecord` (one manual backup per company at a time) |
| `POST /backups/:id/verify?destination=` | `{ identities?: string[], passphrase? }` (optional) → `VerifyReport` |
| `POST /backups/:id/restore?destination=` | `{ name?, identities?, passphrase? }` → 201 `{ id, name, from: Manifest }`: a **new** company (also needs server admin); nothing existing is replaced. Default name "<name> (restored from <date>)". |
| `GET /backups/export` | → `application/zip` download, `<slug>.lbk` |

Errors: 404 unknown backup, 400 bad unlock/config, 422 `damaged` (checksum or
integrity), 502 `storage` (destination unreachable).

### Bank feeds

Types are lb-feeds' (`web/src/api/generated/`). Reading needs viewer, changes
bookkeeper. Multipart uploads use a `file` part plus text parts.

| | |
| --- | --- |
| `GET /feeds` | → `FeedAccountSummary[]` |
| `GET /feeds/txns?account_id=&connection_id=&statuses=new,matched&from=&to=&text=&amount=&limit=&offset=&descending=` | → `FeedTxn[]` |
| `GET /feeds/txns/:id` | → `FeedTxn` |
| `POST /feeds/detect` | multipart `file` → `{ kind: "Ofx" \| "Csv", csv?: CsvDetection, statements?: Statement[] }` |
| `POST /feeds/import` | multipart `file`, `account_id?`, `connection_id?`, `mapping?` (CsvMapping JSON), `statement?` (index) → `IngestReport`. A file whose account was imported before goes to that connection; a CSV without a confident mapping is 400 (detect first). |
| `GET /feeds/txns/:id/candidates?days=` | → `MatchCandidate[]` |
| `POST /feeds/txns/:id/match` | `{ txn_id }` → 204 |
| `POST /feeds/txns/:id/preview` | `AddRequest` → `{ txn, postings, total, sales_tax }` (nothing saved) |
| `POST /feeds/txns/:id/add` | `AddRequest` → 201 `AddOutcome` |
| `POST /feeds/txns/add-all` | `{ ids, request? }` → `BatchOutcome` |
| `POST /feeds/txns/ignore` | `{ ids }` → `{ ignored }` |
| `POST /feeds/txns/:id/undo` | → `FeedTxn` |
| `GET/POST /feeds/connections`, `GET/PUT /feeds/connections/:id`, `DELETE /feeds/connections/:id?edit_seq=` | `FeedConnection`. Credential references must be this company's own (`secret:feeds.<company>.…`), else 400; deleting removes credentials nothing else uses. |
| `PUT /feeds/connections/:id/credentials` | `{ value }` (password, PIN, access URL or token) → `FeedConnection`. Saved in the secret store; never returned. |
| `POST /feeds/connections/:id/sync` | → `IngestReport` |
| `POST /feeds/connections/:id/remote-accounts` | → `StatementAccount[]` (Direct Connect, SimpleFIN) |
| `POST /feeds/simplefin/claim` | `{ setup_token }` → `{ access: "secret:feeds.<company>.simplefin-…", accounts: StatementAccount[], errors }`; then create one connection per account with `access` and `remote_account_id` |
| `GET/POST /feeds/rules`, `GET/PUT/DELETE /feeds/rules/:id` | `FeedRule` |
| `POST /feeds/rules/reorder` | `{ ids }` → `FeedRule[]` |
| `POST /feeds/rules/apply` | `{ ids? , account_id? }` → `{ applied }` |
| `POST /feeds/amazon/orders` | multipart `file` (Amazon order-history CSV), `account_id?` → `{ matches: AmazonMatch[], orders: AmazonOrder[] }` (preview) |
| `POST /feeds/amazon/apply` | `{ matches, orders }` → `{ annotated }` |

Feed errors: 400 `invalid` (parse/bad input), 404, **409 `reconnect`** (the bank
or bridge refused the saved credentials; not the person's session), 502
`provider` / `network`, 501 `unsupported`. Online connections also download by
themselves (`feeds.every`, default 6 h).
