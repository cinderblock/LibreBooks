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
