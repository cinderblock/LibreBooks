# Server operations: secrets, settings, backups, updates, tunnel, feeds

> Parent plan: `plans/librebooks.md` (Phase 6 and 7 wiring). Crate plans:
> `plans/backups.md`, `plans/self-update.md`, `plans/tunnel.md`, `plans/feeds.md`,
> `plans/auth.md`. API contract: `plans/api.md` ("Server administration",
> "Backups", "Bank feeds").

## Goal

Wire the already-built library crates (lb-backup, lb-update, lb-tunnel, lb-feeds)
into `lb-server` and the `librebooks` CLI, so a headless server keeps its own
backups, updates itself, publishes itself through a Cloudflare tunnel, downloads
bank feeds on a schedule, and keeps every credential those need in one encrypted
place that is never readable over the API.

## Environment / context

- Worktree branch `worktree-agent-a16c3059f0635380d`, branched from master at
  `bc44451`. The parent session merges it and is concurrently adding QBD import
  routes to lb-server, so this work keeps to new files plus small, contiguous hooks:
  - `state::Inner` gained one field, `ops: crate::ops::Ops`.
  - `lib.rs`: `mod ops; mod secrets;`, `state_with(config, OpsOptions)` (`state`
    calls it with defaults), the Access middleware layer in `router`, and `serve`
    starts/stops the background work.
  - `routes/mod.rs`: three `mod` lines, `.merge(feeds::routes()).merge(backups::routes())`
    on the company router and `.merge(ops::routes())` on the top one.
  - `auth.rs` `Client` extractor: falls back to `ops.forwarded_ip_header()`.
  - `librebooks/src/main.rs`: `mod ops;`, a global `--secret-key-file`, four
    subcommands, and `serve` merges settings' URLs then calls `ops::run_server`.
- New files: `lb-server/src/secrets.rs`, `lb-server/src/ops/{mod,settings,backups,update,tunnel,access,feeds}.rs`,
  `lb-server/src/routes/{ops,backups,feeds}.rs`, `lb-server/tests/{ops,feeds}.rs`,
  `lb-server/tests/support/mod.rs`, `librebooks/src/ops.rs`.
- Build/test through the CPU broker:
  `node ~/.claude/bin/cpu-slots.mjs run --slots 4 --label "librebooks server-ops" -- cargo test -p lb-server`.

## Data folder layout (headless server)

```text
<data dir>/
  server.json        settings (plain JSON, no secrets)
  secrets.age        the secret store (age-encrypted JSON)
  secret.key         its key (owner-only), unless --secret-key-file puts it elsewhere
  server.db          users, passkeys, sessions (lb-auth)
  companies/<id>.lbdb
  backups/<company id>/<slug>/…   default local backup destination
  bin/cloudflared[.exe]           managed cloudflared, when downloaded
  updates/                        update downloads
  .restore/                       restore work files (removed after each restore)
```

## Decisions (don't re-ask)

1. **Secret store = age.** `secrets.age` is a JSON map `{name: {value,
   updated_at}}` encrypted with age to an X25519 identity kept in `secret.key`
   (the identity line `AGE-SECRET-KEY-1…`). age is already a workspace dependency
   (lb-backup), so no new crypto. Inspect with the stock tool:
   `age -d -i secret.key secrets.age`. The key is created on the first `set`
   (`create_new`, so no race), mode 0600 on Unix (and re-tightened with a warning
   if found looser), and on Windows an ACL granting only the current user
   (`icacls <file> /inheritance:r /grant:r *<SID>:F`, SID from `whoami /user`,
   both run from `%SystemRoot%\System32`, no `unsafe`). A failure to set the ACL is
   logged, not fatal (the default data dir under `%APPDATA%` is user-private
   anyway). The key can live elsewhere (`LIBREBOOKS_SECRET_KEY_FILE` /
   `--secret-key-file`, e.g. a systemd credential), so a copy of the data dir alone
   reveals nothing. The store re-reads the file when its mtime/size changes, so
   `librebooks secret set` and a running server see each other's changes.
2. **References, not values.** Settings hold `secret:<name>`, `env:NAME` or
   `file:/path`. `SecretStore` implements `lb_backup::SecretResolver` and
   `lb_tunnel::TokenSource`; `CompanySecrets` implements `lb_feeds::Secrets`.
   Names: `[A-Za-z0-9._-]{1,128}`.
3. **Write-only over the API.** `PUT/DELETE /server/secrets/:name`; the list
   gives names and update times. No route returns a value; `Debug` of the store and
   of every request body carrying a secret is redacted. A secret still referenced
   by the settings can't be deleted (409 `in_use`).
4. **Feed credentials are scoped per company.** Bookkeepers manage feed
   connections, so a connection may only reference `secret:feeds.<company-id>.*`
   (checked on create/update) and the feeds resolver refuses anything else (no
   `env:`/`file:`, no other company's, no server secrets). Otherwise a bookkeeper
   could point an OFX "password" at `file:/etc/shadow` and a bank URL they
   control. Credentials arrive only through `PUT …/credentials` and the SimpleFIN
   claim; replaced or deleted ones are removed unless another connection shares
   them (one SimpleFIN access URL serves several accounts).
5. **Settings file** `server.json`, written atomically (temp + rename), created
   with defaults on first start so what is shown is what runs. `PUT /server`
   replaces it whole after validation (`problems` → 400 with
   `detail.problems`), then applies the change to what is running.
6. **Backups are on by default** for a headless server, daily to
   `<data dir>/backups` (same disk: guards against mistakes and corruption, not
   disk loss; the UI should nudge towards a second destination). One server-wide
   `BackupConfig`; each company backs up into its own sub-folder per destination
   named by **company id** (`<dir>/<id>/…`, S3 `<prefix><id>/…`), because lb-backup's
   slug comes from the company *name*, and two companies with the same name would
   otherwise share (and prune!) one retention set.
7. **One `lb_backup::Scheduler` per company**, started and stopped by a reconcile
   loop (every minute, and immediately after a settings change, a restore or an
   import), which also restarts the scheduler of a renamed company and hands
   changed settings to the rest. Backups off (no destination, or
   `schedule.enabled = false`) → no schedulers. Manual "run now" calls
   `backup_now` directly and returns the record (one at a time per company).
8. **Restore never replaces**: restore into `.restore/<uuid>.lbdb`, adopt it as a
   new company (`Companies::adopt`, fresh id), rename it `"<name> (restored from
   <date>)"` unless a name is given, delete the work file. Needs company admin and
   server admin. Unlock: identities/passphrase in the request, else the settings'
   passphrase.
9. **Updater** exists only when the build has a release key (none yet; see
   `plans/self-update.md`); otherwise `GET /server/update` gives `unavailable`
   with the reason and check/install are 409 `unavailable`. A background loop
   calls `tick()` every 15 minutes (first after a minute); the policy decides
   whether a check is due and whether auto-install applies. After any install
   with `restart`, `Ops::request_restart` fires; `run_server` stops gracefully
   (listeners, schedulers, tunnel) and then `Restart::spawn()`s the new binary or
   exits with 75 for a supervisor.
10. **Tunnel** runs while `tunnel.enabled` and `serve` runs; any change to the
    tunnel settings restarts it. Preparing (finding/downloading cloudflared) runs
    as its own task with a generation counter, so a change or shutdown never waits
    on a download. The token is saved as the secret `tunnel-token` (`token_ref:
    "secret:tunnel-token"`); the view shows the tunnel id read from it, never the
    token. While the tunnel is enabled, `cf-connecting-ip` from loopback is the
    client address (unless `--client-ip-header` says otherwise).
11. **Passkey relying parties are fixed when the auth store opens** (`AuthStore`
    has no runtime reconfiguration), so `serve` builds them from `--public-url`,
    `server.json`'s `public_urls` and the tunnel hostname. Changing those at
    runtime shows up in `restart_required` (in `GET /server` and the tunnel view);
    a restart picks them up. Making lb-auth reconfigurable would remove this.
12. **Cloudflare Access check** (optional, `access: {team_domain, audience}`):
    middleware over the whole router. A request that came through Cloudflare (it
    has `cf-ray`, which the edge sets and a client cannot strip) must carry a
    valid `Cf-Access-Jwt-Assertion` (lb-tunnel's verifier: RS256 only, iss, aud,
    exp/nbf); direct requests (LAN, localhost) are not affected. 403 with
    `detail.reason = "cloudflare_access"`. Claims are left in request extensions.
13. **Feed sync loop**: every 5 minutes (first after 90 s), per company, per
    online connection with an account: due when the last success is older than
    `feeds.every` (default 6 h, minimum 1 h) and so is the last attempt (kept in
    memory). Refused credentials aren't retried until the connection is edited
    (`edit_seq` changes). Fetch is async; the read and the ingest write run in
    `spawn_blocking` (not `providers::sync`, which writes on the async thread).
    Background syncs act as a system actor that may **not** post into a closed
    period.
14. **`FeedError` → HTTP**: `Parse`/`Invalid` 400 `invalid`, `NotFound` 404,
    `Unauthorized` 409 `reconnect` (the bank refused; not the user's session, so
    not 401), `Provider` 502 `provider`, `Network` 502 `network`, `Unsupported`
    501 `unsupported`, `Store(..)` as store errors. `BackupError`: 404, 409
    exists, 400 config/secret/decrypt, 422 `damaged`, 502 `storage`.
    `UpdateError`: 502 `network`/`untrusted`, 409 `conflict`.
15. **Graceful shutdown**: Ctrl+C, SIGTERM (Unix) or a restart request stops
    every listener (in-flight requests finish), then `Background::shutdown`: the
    loops stop at their next pause (work in progress gets 30 s), every scheduler
    finishes its backup in progress, and cloudflared gets SIGTERM / job close with
    a 10 s grace.

## API and CLI (summary)

Routes: `plans/api.md`. CLI:

```text
librebooks serve [--bind] [--public-url] [--client-ip-header] [--no-auth]
librebooks backup now|list [--company <id|name>]
librebooks backup verify <backup-id> [--company] [--identity FILE] [--passphrase-env VAR]
librebooks backup restore <backup-id | file.lbk> [--company] [--name] [--identity] [--passphrase-env]
librebooks update check|install
librebooks tunnel status | setup --hostname H (token on stdin or LIBREBOOKS_TUNNEL_TOKEN) | disable
librebooks secret set NAME (value on stdin) | delete NAME | list
global: --data-dir, --secret-key-file
```

A headless, tunnel-only server is bootstrapped with `librebooks tunnel setup
--hostname books.example.com` before the first `serve`, so the first-admin link
already works at the public hostname (and passkeys are bound to it).

## Running under a supervisor

The server exits with **75** (`RESTART_EXIT_CODE`) after installing an update
when it detects a supervisor (systemd sets `INVOCATION_ID`; anything else sets
`LIBREBOOKS_SUPERVISED=1`); otherwise it starts the new binary itself and exits 0.

Example systemd unit (`/etc/systemd/system/librebooks.service`). The binary's
folder must be writable by the service user for self-update, so not `/usr/bin`:

```ini
[Unit]
Description=LibreBooks accounting server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=librebooks
Group=librebooks
ExecStart=/opt/librebooks/librebooks serve --bind 127.0.0.1:8740
Environment=LIBREBOOKS_DATA_DIR=/var/lib/librebooks
# Keep the secret store's key out of the data folder (and its backups):
LoadCredential=secret.key:/etc/librebooks/secret.key
Environment=LIBREBOOKS_SECRET_KEY_FILE=%d/secret.key
# Exit 75 after a self-update means "start me again".
Restart=on-failure
RestartForceExitStatus=75
RestartSec=2
# Graceful stop: listeners, backups in progress, cloudflared.
KillSignal=SIGTERM
TimeoutStopSec=60
# Hardening; the data dir and the binary's folder (self-update) stay writable.
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/librebooks /opt/librebooks
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

Caveat on `LoadCredential`: systemd's credential copy is read-only, so the key
must exist before the first start: `age-keygen -o /etc/librebooks/secret.key`
(LibreBooks reads the `AGE-SECRET-KEY-1…` line of any age identity file;
`%d` needs systemd 250+). Without the two credential lines the key is made in the
data folder the first time a secret is saved.

**Windows service**: set `LIBREBOOKS_SUPERVISED=1` in the service's environment
and make recovery restart on any failure, including a clean non-zero exit:
`sc failure librebooks reset= 0 actions= restart/5000` and
`sc failureflag librebooks 1`. The service account must be able to write the
program's folder (self-update) and the data folder. LibreBooks has no native
Windows-service entry point yet (see Gaps); run it under a wrapper (WinSW, NSSM,
or `sc create` with a wrapper) that forwards stop as Ctrl+C.

**Containers**: never self-update (pull the image); `LIBREBOOKS_CONTAINER=1`.

## Findings / gotchas

- `TunnelStatus` serialises its tagged `state` inside a field also called
  `state`: clients read `status.state.state` (and `status.state.reason`,
  `status.state.connections`). lb-tunnel's shape; left as is.
- axum's `Path` rejects a raw space in a URI before routing (tests must
  percent-encode `bad%20name`).
- The CPU broker and the shared cargo build dir queue builds behind other
  worktrees; a `cargo test` can sit several minutes on "Blocking waiting for file
  lock" before compiling anything. Run long ones in the background.
- `lb_feeds::providers::sync` holds a SQLite write on the async thread; the server
  reimplements its three steps (read, fetch, ingest+record) with the DB parts in
  `spawn_blocking`.
- The scheduler's own `last_success_at` does not see manual backups; a manual
  "run now" doesn't move the scheduled one (at worst one extra backup).

## Gaps / later

- **Passkey RPs and `--client-ip-header`** are fixed at start (decision 11);
  lb-auth would need `AuthStore::set_relying_parties` for runtime changes.
- **Live tunnel status from the CLI**: `librebooks tunnel status` shows settings,
  token validity and the cloudflared it would run; the live connection state is
  only in the running server's API.
- **No native Windows service** entry point (`windows-service` crate): needed for
  a clean SCM stop; until then a wrapper.
- **`.lbk` import** over the API: multipart `POST /companies/import` (streams
  the upload to `.restore/` under the data folder, then `import_lbk`); the CLI's
  `backup restore x.lbk` does the same from a path.
- **A renamed company's older backups** (under its old slug, inside its id
  folder) are pruned too: the server sets `RetentionScope::Destination` on
  each company's config, since the folder/prefix is that company's alone.
  The library default stays `CompanySlug` for shared destinations.
- **Secret-store races**: `set`/`delete` hold an exclusive lock on
  `secrets.lock` (std `File::lock`) across the read-modify-write, so the server
  and the CLI can't save over each other (test: 12 threads × 5 saves from
  separate stores; it loses saves without the lock). `server.json` is written
  by the server alone (the CLI changes settings through the API), so it only
  has the in-process `change` mutex.
- **Access check** is tested end to end: lb-tunnel's tests cover the verifier,
  and the server's `a_signed_access_assertion_gets_through` sends a token
  signed with lb-tunnel's test key through the middleware against a local key
  server (and refuses an unpublished key, another audience, an expired token).
- **Real cloudflared** is never started in these tests (a missing configured path
  exercises the failure path); lb-tunnel's own tests supervise its fake.
- `Build` for the updater is `CARGO_PKG_VERSION`; a development build that isn't
  in a `target/` folder looks like a release. Harmless until a release key exists;
  the release workflow should stamp the git-describe version.

## Progress log

- [x] `secrets` module: age store, owner-only key, references, company scoping (4 unit tests).
- [x] `ops` module: settings (3 unit tests), schedulers, updater, tunnel, Access, feed sync (1 unit test).
- [x] Routes: `/server…`, `/c/:company/backups…`, `/c/:company/feeds…`.
- [x] Access middleware; client IP from `cf-connecting-ip` while the tunnel is on.
- [x] API tests: `tests/ops.rs` (9), `tests/feeds.rs` (3); existing api/auth tests pass.
- [x] CLI: `backup`, `update`, `tunnel`, `secret`; `serve` with background work,
      SIGTERM, graceful shutdown and restart after update.
- [x] `plans/api.md` updated.

## Things not to do

- Never return, log or serialise a secret value; never put one in an error.
- Never let a feed connection resolve a secret outside its company's namespace.
- Never restore over an existing company file.
- Don't key backup folders by company name (collisions prune each other).
