# Alpha software notice

## Goal

LibreBooks is alpha software. Say so where people will see it, and make every
person accept a notice before their first change to any books: a modal that has
to be accepted (or the change is not made).

## Decisions already made (don't re-ask)

- **Acceptance is remembered per user, on the server** (Cameron, 2026-11-05).
  Per-device browser storage was rejected because the desktop app serves its UI
  from a random loopback port each launch, so `localStorage` is a new origin
  (empty) every time.
  - Signed-in people (passkey, device key): two columns on the auth DB's `user`
    row (`alpha_notice_version`, `alpha_notice_at`), auth migration 2.
  - The desktop app's local token ("This computer") and open mode have no user
    row: their acceptance is `notices.json` in the server's data dir.
- **The notice has a version** owned by the web UI (`ALPHA_NOTICE_VERSION`).
  Bumping it (when the wording changes materially) asks everyone again. The
  server keeps the highest version accepted and when.
- **The gate is in the API client**, not in each form: a proxy around the
  `ApiClient` holds every state-changing call until the notice is accepted. An
  exhaustive, type-checked table classifies every `ApiClient` method as `read`,
  `write` or `exempt`, so a new method can't be added without deciding. Exempt:
  sign-in, sign-out, enrolment, backups/verify (protective, not edits), and the
  acceptance call itself.
- **Declining** cancels the held call with an `ApiError` (`alpha_notice`), so
  the form stays open with its data and says it wasn't saved. Accepting lets the
  held call carry on: nothing typed is lost.
- **Not enforced by the server.** API clients, the CLI and the QuickBooks
  bridge are not people at a screen.
- **Visible badge**: an "Alpha" badge in both shells' headers, the company
  picker and sign-in. It is a button that opens the notice (no hover-only
  tooltips: they don't exist on phones).
- Fixture (demo) mode gates too, so the flow is testable; its acceptance is
  kept in `localStorage` (`librebooks.fixtures.alphaNotice`), which Playwright
  pre-seeds through `storageState` so existing tests are untouched.

## API

- `GET /session` → `user.alpha_notice: { version, accepted_at } | null`.
- `POST /alpha-notice` `{ version }` → `{ version, accepted_at }` (the stored
  acceptance; never lowers the version).

## Progress

- [x] Plan
- [x] lb-auth: migration 2, `NoticeAcceptance`, `User.alpha_notice`, `accept_alpha_notice`, test
- [x] lb-server: `notices.rs` (`notices.json` for local principals), `/session` field, `POST /alpha-notice`, tests
- [x] Regenerate TS types (`bun run gen:types`)
- [x] Web: `api/access.ts` (table + `gateWrites`), `alpha/notice.ts` (state + prompt), `alpha/AlphaNotice.tsx` (modal, badge, `AlphaTitle`), fixture support, unit tests
- [x] e2e: `storageState` seed in `playwright.config.ts`; `desktop.spec.ts` "the alpha notice"
- [x] README + plans/api.md
- [x] Checks pass: lb-auth, lb-server, clippy, web `bun run check` (282), e2e (64/64)
- [x] Deployed to cad-3 2026-11-05 2:42 AM PST (10:42Z): `app\LibreBooks.exe`
      (FFD6D314…) and `bin\librebooks.exe` (763CF33C…), both previous kept as
      `*.prev.exe`. The CLI went too: the auth DB migration (2) would leave an
      older CLI refusing a newer auth database. Not yet opened by anyone.

## First prerelease: v0.1.0-alpha.1

Cameron said go 2026-11-05. Until then `release.yml` had never run (no tags).

- [x] Workspace version 0.1.0 → `0.1.0-alpha.1` (`Cargo.toml`, `Cargo.lock`,
      `web/package.json`, `bun.lock`): the draft job requires tag == version.
- [x] `release.yml`: pass `tauri.release.conf.json` (updater bundles; was an
      ISSUES entry), and a generated `tauri.msi.conf.json` so the MSI gets a
      numeric version (`0.1.0-alpha.1` → `0.1.0.1`).
- [x] Local MSI/NSIS build with the prerelease version: both built
      (`LibreBooks_0.1.0-alpha.1_x64_en-US.msi`, `…_x64-setup.exe`).
- [x] Pushed, tagged `v0.1.0-alpha.1` (12:41 PM PST, 20:41Z). Run
      42105661610 **failed**, with two bugs:
  - The draft job read the new draft's id back from the release list right
    after creating it and got an empty answer (the list lags), so Windows
    built and signed both installers and then logged "No releaseId or tagName
    provided, skipping all uploads".
  - Linux: `invalid version "0.1.0-alpha.1": contains invalid character` —
    RPM versions can't contain `-`.
  - The server binaries and the container image built fine; the draft
    (id 435708590) kept the four server archives. Nothing was published.
- [x] Fix `4e45caa`: the id comes from the create call; a prerelease's
      Linux desktop build is `--bundles appimage,deb`.
- [x] Re-ran by hand for the same tag (run 42109872107, 1:08 PM PST, 21:08Z):
      every job passed. Published as a prerelease at 1:33 PM PST (21:33Z):
      https://github.com/cinderblock/LibreBooks/releases/tag/v0.1.0-alpha.1
- [x] Checked: Windows NSIS + MSI, Linux AppImage + deb, each with a `.sig`;
      `latest.json` (all six platforms); the four server archives;
      `SHA256SUMS.minisig` verifies against `minisign.pub` (trusted comment
      `librebooks v0.1.0-alpha.1`); image `ghcr.io/cinderblock/librebooks`
      tagged `v0.1.0-alpha.1` and `prerelease`.
- [ ] Not tried: installing from these installers on a real machine.

Notes:

- Prerelease versions sort *below* their release: `0.1.0-alpha.1 < 0.1.0`.
  The hand-built dogfood copy on cad-3 says `0.1.0`, so it would never see
  this as an update. It doesn't matter yet: the Tauri updater reads
  `releases/latest/download/latest.json`, and GitHub's "latest" skips
  prereleases, so no prerelease is offered as an update to anyone.
- Signing keys: both secrets are password-less (commit `42946a7`); the Tauri
  updater key (`C9F5…F5B1`) is separate from `minisign.pub` (`7D23…0917`).

## Where things are

- Version: `web/src/alpha/version.ts` (`ALPHA_NOTICE_VERSION`). Bump it to ask everyone again.
- Classification of every API call: `web/src/api/access.ts` (`API_ACCESS`).
- Wording: `web/src/alpha/AlphaNotice.tsx`.
- Badge spots: both shells' headers, `AlphaTitle` on sign-in, enrolment and the company picker.

## Findings / gotchas

- Desktop local mode: `Principal::local("This computer")`, `user_id: None`, no
  auth store at all (`sessions: None`).
- Desktop `localStorage` resets every launch (random port → new origin). This
  also affects every existing per-device pref in `web/src/lib/prefs.ts`; logged
  in `ISSUES.md`.
- The auth DB keeps milliseconds: `accept_alpha_notice` truncates `now` first,
  or its answer (µs) differs from what `/session` reads back later (a test
  caught this).
- Global shortcuts are already suppressed while any `aria-modal` is open
  (`KeyboardProvider.modalOpen`), so a second Alt+A can't double-save under
  the notice.
- Desktop build: `CARGO_TARGET_DIR` must be **absolute**
  (`$PWD/target/desktop`). A relative one is taken from `desktop/src-tauri`,
  so the build lands in `desktop/src-tauri/target/desktop` and the old exe in
  `target/desktop` looks current (same hash as the last deploy: caught that way).
- Over SSH, `powershell -Command -` with a multi-line script silently did
  nothing; `scp` the script and run it with `-File`.
- Playwright's `storageState` seeds the fixture acceptance; one
  `test.use({ storageState: { cookies: [], origins: [] } })` opts out.
