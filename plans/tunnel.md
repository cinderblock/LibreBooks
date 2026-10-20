# Cloudflare tunnel (`crates/lb-tunnel`)

> Parent plan: `plans/librebooks.md`, decisions 8 (Access JWT as a second layer)
> and 10, Phase 6. API: `GET /server/tunnel` (status), tunnel settings in
> `GET/PUT /server` (`plans/api.md`).

## Goal

User requirement: **opt-in wrapping of `cloudflared`, so publishing LibreBooks
through a Cloudflare zero-trust tunnel needs only the tunnel token (maybe the
hostname too).** LibreBooks finds or downloads cloudflared, runs it, keeps it
running, shows its state, and can verify Cloudflare Access's identity header.

## Environment / context

- cloudflared releases: `github.com/cloudflare/cloudflared/releases`, tags like
  `2026.10.1` (calendar versions, no `v`). Assets per platform: raw binaries for
  Windows/Linux (`cloudflared-windows-amd64.exe`, `cloudflared-linux-arm64`, …),
  `.tgz` for macOS (`cloudflared-darwin-arm64.tgz`).
- Dev machine has cloudflared 2026.10.0 at `C:\Program Files (x86)\cloudflared\`
  (used only for research; tests use a fake).
- Process control: `process-wrap` (watchexec's): job object with
  kill-on-job-close on Windows, process group on Unix, kill-on-drop; Linux adds
  `PR_SET_PDEATHSIG` via `libc` in a `pre_exec` (the crate's only `unsafe`, with
  a SAFETY comment).
- JWT: verified with the `rsa` crate (PKCS#1 v1.5, public-key operations only).

## Decisions already made (don't re-ask)

1. **Config** `TunnelConfig { enabled: false, token_ref, public_hostname,
   cloudflared_path, auto_download: true }`. The token is a reference
   (`env:NAME`, `file:/path`, or the caller's `TokenSource`), never stored in
   settings. `TokenInfo::parse` checks a pasted token (base64 JSON `{a,t,s}`,
   accepting the whole `cloudflared service install <token>` command) and yields
   the tunnel id for the UI; the secret is never kept.
2. **Where cloudflared comes from**, in order: configured path (missing = error,
   not a silent fallback), `PATH` / Cloudflare's MSI folder, the managed copy in
   `<data dir>/bin/`. Only the managed copy is downloaded or updated, at start,
   when the latest release is newer. A failed update keeps the current copy.
   `Sources::search_system = false` skips the system copy (tests; could be a
   setting).
3. **Run** `cloudflared tunnel --no-autoupdate run` with `TUNNEL_TOKEN` in the
   environment (argv is world-readable). `--no-autoupdate` because cloudflared's
   updater would replace a binary we verified or a package manager owns.
4. **Status** `TunnelStatus { state: stopped | starting | connected{connections}
   | reconnecting | failed{reason}, last_log_lines, version, restarts, tunnel_id,
   public_url }`, from log parsing (`logparse`). Any occurrence of the token in a
   log line is replaced with `[token]` before it is kept.
5. **Restart** with backoff 1 s doubling to 5 min; a run connected for ≥ 60 s
   resets it. A bad/revoked token is `failed` with an actionable reason and is
   retried at the slow end of the schedule (it may be fixed in the dashboard).
6. **Stop**: SIGTERM to the group (cloudflared drains gracefully), 10 s grace,
   then kill; on Windows, terminate the job. Dropping `Tunnel` kills at once.
7. **Access verifier**: `alg` must be exactly RS256; key by `kid` from
   `https://<team>.cloudflareaccess.com/cdn-cgi/access/certs`; checks `iss`,
   `aud` (string or list), `exp`/`nbf` with 60 s leeway. Keys cached, refreshed
   hourly or on an unknown `kid`, but never more than once per 30 s.

## Trust model for the download (honest version)

cloudflared releases carry no signature LibreBooks can check portably (Windows
binaries are Authenticode-signed, macOS notarised). What exists:

1. SHA-256 per asset in the release notes (`### SHA256 Checksums:` lines
   `name: hex`).
2. GitHub's own SHA-256 of each upload (the API's `digest`, `sha256:hex`).

A download must match the notes, and the digest when present. Both arrive in the
same GitHub API response over TLS, so this proves the bytes are what Cloudflare
published on GitHub (catches corruption and tampering on the download path) but
**not** a compromise of Cloudflare's GitHub account or of GitHub. A release whose
notes list no checksum for our asset is refused (`Unverifiable`). Users wanting
more can install from Cloudflare's signed apt/rpm repositories and set
`cloudflared_path`.

## Findings / gotchas

- **macOS `.tgz` checksums (2026.10.1)**: the release notes' hash for
  `cloudflared-darwin-*.tgz` is the hash of the `cloudflared` binary *inside* the
  archive; GitHub's `digest` is the archive's. Verified by downloading
  `cloudflared-darwin-amd64.tgz`: archive `3b0dbaf3…` (= digest), inner binary
  `8ccc6383…` (= notes). So: archive vs digest, unpack, binary vs notes. Raw
  binaries match both. Test fixture: the real 2026.10.1 release response, trimmed
  (`testdata/cloudflared-release-2026.10.1.json`).
- `cloudflared --version` prints `cloudflared version 2026.10.0 (built
  2026-10-05T08:39 UTC)`.
- With an invalid `TUNNEL_TOKEN`, cloudflared 2026.10.0 prints `Provided Tunnel
  token is not valid.` and `See 'cloudflared tunnel run --help'.` and exits 255,
  before any network activity.
- The fake cloudflared used by tests is a `[[bin]]` of this crate
  (`lb-tunnel-fake-cloudflared`, `tests/support/fake_cloudflared.rs`), so the
  supervision tests run a real child process identically on Windows and Unix.
- **Linux build.** Cross-checking from Windows (`cargo clippy --target
  x86_64-unknown-linux-gnu`) stops at `aws-lc-sys` (needs
  `x86_64-linux-gnu-gcc`); CI's Ubuntu job is the real check and passes
  (`5a2aa32`): the Unix-only code (`proc.rs` `pre_exec`/`prctl`, the SIGTERM
  stop, permissions in tests) builds and its tests pass there. macOS: untried.
- The CPU broker (`~/.claude/bin/cpu-slots.mjs`) sometimes fails with `EPERM`
  opening its `.mutex` while waiting for slots on Windows; rerunning works.

## Gaps / later

- macOS has no parent-death signal: a hard crash of LibreBooks can leave
  cloudflared running until the next start. Not tested end to end anywhere: the
  parent-death paths (job object close, PDEATHSIG) are OS behaviour; the tests
  cover drop/stop of the supervisor ending the child.
- State comes from logs; cloudflared's metrics endpoint (`/ready`) would be a
  sturdier second source.
- Windows on ARM uses the x64 build (no native asset).
- Server wiring (start/stop on settings change, `GET /server/tunnel`, SSE of
  status, Access verifier as middleware, passkey RP id from `public_hostname`) is
  the lb-server thread's.

## Plan / steps

1. ~~Implement crate and tests; clippy/fmt clean; commit.~~
2. ▶ Server wiring (lb-server thread).
3. Run the suite on Linux and macOS (CI): the Unix-only code has not been compiled yet.

## Progress log

- [x] lb-tunnel implemented and committed: config/token, log parsing, backoff,
  cloudflared locate/download/update, supervisor, Access verifier. 35 tests pass on
  Windows (unit; real child-process supervision with the fake; downloads against
  a local GitHub stand-in; JWTs against a local key server).
- [x] Server wiring: `/tunnel` routes, supervisor started by `serve`, CLI `tunnel`.

## Things not to do

- Never pass the token on the command line or keep it in status/logs.
- Never update a cloudflared that LibreBooks did not download.
- Never accept a JWT whose `alg` is not RS256.
