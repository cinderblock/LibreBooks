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
