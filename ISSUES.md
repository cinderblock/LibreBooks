# Issues

Side-issues found while working on something else, each just enough to seed a
fresh thread. Claim one by marking it `in progress — <thread/branch>` and commit
that first; remove it (or mark it fixed with the commit) when done.

## Playwright: the Report Center scenario failed once under the full suite

Seen 2026-10-22 on `worktree-agent-adcfa6cee1449001f` (web phase F): one full
`bun run test:e2e` run reported `[desktop] › e2e/desktop.spec.ts:346:1 › the
Report Center lists QBD categories; memorizing a report keeps it` failed, with
the other 57 passing; the same suite passed 58/58 immediately before and after
(the phase F changes touch nothing it exercises). Likely a timing-sensitive
assertion under two workers. Worth a look at the test's waits before it costs
someone a rerun.

## Self-update replaces only librebooks.exe, not the 32-bit SDK helper

Seen 2026-10-30 adding `librebooks-qbsdk.exe`: `lb-update::install::extract_binary`
takes just the main binary from the release zip, so a self-updated server keeps
its old helper, or has none if it first installed before the helper shipped.
The hello carries a protocol version, so a stale helper refuses with a clear
message instead of misbehaving, but QuickBooks 2021 users would then be stuck
until they copy it by hand. The desktop app's installer isn't affected. The
updater should replace companion files from the same verified archive.

## release.yml never passes tauri.release.conf.json

Seen 2026-10-30: `createUpdaterArtifacts` is only on in
`desktop/src-tauri/tauri.release.conf.json`, and `plans/desktop.md:236` says to
build with `--config` on it, but nothing in `.github/workflows/` does, so a
tagged release may publish no updater artifacts despite `includeUpdaterJson`.
Not checked against a real tagged run; the first release should confirm it.

## Credits matched oldest-first though qbXML says which invoice each settled

Seen 2026-10-31 on the first full SDK import: 48 applications were made by
`apply_credits_to` (`lb-store/src/apply.rs`), dated by LibreBooks. That guess
exists because the bridge never said which credit memo settled which invoice,
but over qbXML `CreditMemoQueryRq` sends `LinkedTxn` (`IncludeLinkedTxns`), so
the real pairing is available. Balances and open amounts agree either way;
only which credit paid which invoice, and its date, can differ from QuickBooks.

## On Windows the desktop and CLI builds overwrite each other in target/release

The CLI's binary is `librebooks.exe` and the desktop's `mainBinaryName` is
`LibreBooks`. Both land in the shared `target/release/`, and Windows file names
are case-insensitive, so a `cargo tauri build` replaces the CLI and the other
way round. Seen 2026-11-02 about 5:08 PM PST: a script that meant to start the
CLI headless started the desktop app instead. The workaround for now is
`CARGO_TARGET_DIR=target/desktop` for desktop builds. CI and the release
workflow should be checked for the same collision; renaming one binary, or
keeping a separate target dir everywhere, would fix it.

## Desktop app: per-device preferences reset on every launch

Seen 2026-11-05 adding the alpha notice: the desktop app serves the SPA from
`127.0.0.1:<random port>` (`desktop/src-tauri/src/backend.rs`, unless
`LIBREBOOKS_DESKTOP_PORT` is set), so each launch is a new origin with empty
`localStorage`. Everything in `web/src/lib/prefs.ts` (Enter behaviour, report
export format and paper) and the other `localStorage` keys are forgotten at
every start. The
alpha notice avoided this by keeping its answer on the server. Either a stable
port/origin for the desktop webview, or prefs stored server-side, would fix it.
