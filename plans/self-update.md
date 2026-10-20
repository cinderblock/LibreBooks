# Self-update (`crates/lb-update`) and the release signing key

> Parent plan: `plans/librebooks.md`, decision 9 and Phase 6. API:
> `POST /server/update/check`, `POST /server/update/install`, `GET/PUT /server`
> (`plans/api.md`). Design borrowed from open-task's `ot-update` and its
> `plans/self-update.md`.

## Goal

User requirements: **the app updates itself; automatic installing is opt-in; CI
builds and releases on tags.** This crate is the **headless server's** updater.
The desktop app uses Tauri's updater plugin, signed with the same key (below).

## Environment / context

- Releases: GitHub releases of `cinderblock/LibreBooks`, tags `vX.Y.Z` (semver,
  pre-releases `vX.Y.Z-rc.N`).
- Feed: `GET https://api.github.com/repos/cinderblock/LibreBooks/releases?per_page=30`
  (owner, repo and API base are configurable: tests run against a local server).
- Trust: `minisign-verify` (by minisign's author, no dependencies).
- Archives: `zip` (Windows), `flate2` + `tar` (everything else), pure Rust.
- `build.rs` records the target triple (`LB_UPDATE_TARGET`) so the updater asks
  for exactly its own asset.

## Decisions already made (don't re-ask)

1. **Trust only the signed sums.** The API says which releases exist and where
   their files are; a release is believed only when `SHA256SUMS` verifies with the
   compiled-in minisign key *and* the signed trusted comment is exactly
   `librebooks <tag>` for the tag the API named (stops replaying an old, genuinely
   signed release under a new tag). Each download is checked against its line.
2. **Channels**: `stable` (default) takes releases not flagged pre-release and
   without a semver pre-release part; `prerelease` takes any non-draft release.
   Tags that are not `v<semver>` (`nightly`) are ignored.
3. **Version ordering** is open-task's: semver precedence, and a development
   build `0.2.1-19-g892159c[-dirty]` is newer than `0.2.1` and older than
   anything above it (offered `0.2.2`, never `0.2.1`).
4. **Checking is on and only reports; auto-install is opt-in**
   (`UpdateConfig { check: true, auto_install: false, channel: stable, interval: 24h }`).
   The policy (`policy::next_step`) is a pure, table-tested function. A version
   whose install failed is not retried automatically.
5. **Only plain installs replace themselves.** Containers (pull the image),
   development builds, package-manager copies (`/usr/bin`, Nix, Snap, Homebrew),
   unwritable folders and `LIBREBOOKS_NO_SELF_UPDATE` get advice instead and are
   never touched. Containers are detected by `LIBREBOOKS_CONTAINER` (the
   Dockerfile should set it), `container=`, `/.dockerenv`, `/run/.containerenv`.
6. **Install keeps the previous binary** as `<name>.old` beside it
   (`install::rollback` puts it back). The running executable is replaced with
   `self-replace`; any other path by copy-beside-then-rename.
7. **Restart is the caller's job**, guided by `Restart`:
   - under a supervisor (systemd sets `INVOCATION_ID`; a Windows service or any
     other supervisor should set `LIBREBOOKS_SUPERVISED=1`): exit with code **75**
     (`RESTART_EXIT_CODE`, EX_TEMPFAIL) and let the supervisor start the new binary;
   - otherwise: `Restart::spawn()` starts the new binary with the same arguments,
     then the caller exits 0.

## Asset naming contract (the release workflow must follow it)

```text
librebooks-<tag>-<rust target triple>.zip      Windows targets
librebooks-<tag>-<rust target triple>.tar.gz   all other targets
SHA256SUMS                                     sha256sum output for every asset
SHA256SUMS.minisig                             minisign -S … -t "librebooks <tag>"
```

e.g. `librebooks-v0.3.0-x86_64-unknown-linux-musl.tar.gz`. The archive holds the
binary `librebooks` / `librebooks.exe` at the top level or inside one folder (other
files are ignored). Planned headless targets: `x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl`, `x86_64-pc-windows-msvc`, later
`aarch64-apple-darwin` / `x86_64-apple-darwin`.

## Release signing key (to do once, needs the user)

There is **no production key yet**. `trust::PRODUCTION_PUBLIC_KEY` is `None`, a
clearly marked placeholder; until it is set, `TrustedKey::built_in()` fails with
"this build has no release signing key" and the server reports that instead of
trusting anything.

1. Generate (on the dev machine; minisign is installed via winget
   `jedisct1.minisign`):

   ```powershell
   minisign -G -W -p minisign.pub -s $env:USERPROFILE\.minisign\librebooks.key
   ```

   `-W` = no password (CI has no one to type it; the secret lives only in the GitHub
   secret and this file). `rsign generate -W` makes the same format.
2. Commit `minisign.pub` at the repository root and paste its text into
   `PRODUCTION_PUBLIC_KEY` as `Some("untrusted comment: …\nRW…")` (or switch the
   constant to `include_str!("../../../minisign.pub")`, as open-task does).
3. Store the secret key file's contents as the repository secret
   **`MINISIGN_SECRET_KEY`** (`gh secret set MINISIGN_SECRET_KEY < …\librebooks.key`)
   — a third-party account setting: stage it and wait for the user's explicit yes.
4. The release workflow signs with
   `minisign -S -s key -m SHA256SUMS -x SHA256SUMS.minisig -t "librebooks $TAG"`
   (strip `\r` from the secret first; the key may have been made on Windows), then
   checks the result with `minisign -V -p minisign.pub`, and fails the release if the
   secret is missing rather than publishing something the updater cannot verify.
   open-task's `.github/workflows/release.yml` "Sign" step is the template.

For testing a whole update against a throwaway key, build with
`LB_UPDATE_PUBLIC_KEY="<key text>"`; it only affects that binary.

### The desktop app (Tauri updater), same key

Tauri's updater verifies minisign signatures too: `plugins.updater.pubkey` in
`tauri.conf.json` is the base64 of the whole `minisign.pub` text, and the
workflow signs each updater bundle with `TAURI_SIGNING_PRIVATE_KEY` = the base64 of
the secret key file. Using one key means one secret to guard and one public key in
the repo. **Unverified:** whether `tauri signer sign` accepts a password-less (`-W`)
minisign key. Check it with the first desktop release; if it refuses, generate the
key with `tauri signer generate` instead (also minisign format), use it for both,
and give CI its password as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (minisign reads
the password from stdin for the `SHA256SUMS` signature).

## Supervisor expectations (document in the README's server section)

- **systemd**: `Restart=always` (or `Restart=on-failure` plus
  `RestartForceExitStatus=75`). systemd sets `INVOCATION_ID`, so the updater
  chooses "exit 75". The binary's folder must be writable by the service user for
  self-update (e.g. `/opt/librebooks/`, not `/usr/bin`); otherwise it reports
  "can't write to …" and does nothing.
- **Windows service**: set `LIBREBOOKS_SUPERVISED=1` in the service's environment
  and configure recovery to restart on failure, including non-crash failures:
  `sc failure librebooks reset= 0 actions= restart/5000` and
  `sc failureflag librebooks 1`. The service's folder must be writable by its
  account.
- **Containers**: never self-update; pull `ghcr.io/cinderblock/librebooks` and
  recreate. The Dockerfile should set `LIBREBOOKS_CONTAINER=1`.

## Plan / steps

1. ~~Implement crate, fixtures, tests; clippy/fmt clean; commit.~~
2. ▶ Release workflow (`.github/workflows/release.yml`) following the asset contract
   (another thread / Phase 6).
3. Key generation and the GitHub secret (needs the user's yes).
4. Server wiring: `Updater` in server state, a timer calling `tick()`, the two
   routes, `UpdateStatus` in `GET /server`, exit/respawn per `Restart`.

## Findings / gotchas

- minisign on Windows writes `.minisig` and `.pub` files with CRLF; CI on Linux
  writes LF. Both verify (tested).
- The GitHub API needs a `User-Agent`; the updater sends `LibreBooks/<version>
  (self-update)` plus `Accept: application/vnd.github+json` and
  `X-GitHub-Api-Version: 2022-11-28`. Unauthenticated calls are limited to 60 an
  hour per IP, ample for a daily check.

## Test fixtures

`crates/lb-update/testdata/` (bytes are the test; `-text` in `.gitattributes`):
`test.pub` (test-only key; its secret was never committed and is discarded),
`SHA256SUMS` + `SHA256SUMS.minisig` (`librebooks v0.3.0`), `other-key.minisig`,
`wrong-comment.minisig` (`librebooks 0.3.0`), `replayed-tag.minisig`
(`librebooks v0.2.9`), fake archives for `x86_64-pc-windows-msvc` (zip, binary at
top level) and `x86_64-unknown-linux-gnu` (tar.gz, binary in a folder),
`fake-binary.bin` (what both contain), `releases.json` (API shape: nightly,
draft, rc, stable, older).

## Gaps / later

- Before replacing, the new binary must answer `--version` (30 s, clean exit,
  naming the release's version): `install::smoke_test`, on by default
  (`UpdaterOptions::smoke_test`), off in tests whose binary is a stand-in.
- Download progress is not reported (the archive is ~10 MB).
- Only the first page (30 releases) of the API is read.

## Progress log

- [x] lb-update implemented: version, feed, trust, install, policy, updater.
  32 tests (unit + end-to-end against a local GitHub stand-in, both archive kinds).
- [x] Release workflow (`4058946`), unexercised until the first tag.
- [x] Production key + secret: `minisign.pub` at the repo root, private key only
      in GitHub secrets (`42946a7`).
- [x] Server wiring: `/update` routes, policy in housekeeping, CLI `update`.

## Things not to do

- Never trust a release whose sums are unsigned, signed by another key, or signed
  for another tag — no "unverified" fallback.
- Never replace a binary inside a container or one a package manager owns.
- Don't run `gh secret set` without the user's explicit yes.
