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

## SDK transport doesn't ask QuickBooks for read-only access

Seen 2026-10-24 reviewing `crates/lb-qbd/src/sdk.rs` (around line 405): the
session calls `OpenConnection2` then `BeginSession` without first setting
`AuthPreferences.PutIsReadOnly(true)` on the request processor, so the
authorisation dialog QuickBooks shows offers full access. Our own
`check_read_only` choke point (`source.rs:102`) still refuses every non-query,
but the Web Connector path has QuickBooks enforce read-only as a second lock
(`.qwc` `IsReadOnly`) and the SDK path should too, before anyone authorises
it against a real company. Also untested: whether COM from a non-interactive
(SSH, session 0) process can reach a QuickBooks running on the desktop.
