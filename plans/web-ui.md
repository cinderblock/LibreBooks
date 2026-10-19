# LibreBooks web UI (phase A: foundation)

Living plan for the SPA under `web/`. Master plan: `plans/librebooks.md`
(decisions 7 and 13). HTTP contract: `plans/api.md`.

## Goal

The UI's foundation, built before the server exists: the app shell, the input
system (keyboard + input components) and the data layer, plus a **fixture mode**
(an in-memory fake API seeded with a synthetic company) so every screen renders
and can be exercised with `bun run dev`.

User requirements this must meet: mobile first class from day one;
QuickBooks-quality input (QBD keyboard shortcuts, QuickFill, date keys, the
amount calculator); reports and views. The Tauri desktop app loads this same SPA,
so the keymap differs between the desktop app (real QBD keys) and a browser
(where Ctrl+W/T/N/Tab can't be captured).

## Environment / context

- Worktree branch `worktree-agent-ab818e840111ec792`; parent session merges it.
- Vite 8, React 19, Mantine 9.7 (has `Menubar`), React Router 8.5 (v7-style API,
  `createBrowserRouter`/`RouterProvider` from `react-router`), TanStack Query 5,
  TanStack Virtual 3, TypeScript 7 (`tsc`), Bun 1.4 (package manager + unit tests),
  Playwright 1.64.
- Commands (wrap heavy ones in the CPU broker,
  `node ~/.claude/bin/cpu-slots.mjs run --slots N --label "librebooks web" -- …`):
  - `bun run dev` (from `web/`) → http://localhost:5740 ; add `?fixtures=1`.
  - `bun run typecheck`, `bun test src/`, `bun run test:e2e`.
- Generated API types live in `web/src/api/generated/` (ts-rs from Rust). Never
  edit them.

## Decisions already made (don't re-ask)

- Only `web/` and this file are touched in phase A.
- Money is integer cents (`number`). Rates/quantities are decimal strings; exact
  decimal math is BigInt-scaled integers (`lib/decimal.ts`). No floats anywhere
  near money.
- Fixture mode: `?fixtures=1` (sticky for the tab via `sessionStorage`) or
  `VITE_FIXTURES=1`. In-memory only; reload resets it.

## Plan / steps

1. Pure logic + tests: money, decimal, calculator, dates (port of `period.rs`),
   QuickFill matching.
2. Data layer: API client interface, HTTP client, fixture client, query hooks, SSE.
3. Keyboard: combos, command catalogue, platform keymaps, scoped registry.
4. Window manager (QBD's open-window list as tabs).
5. Shell: desktop (menu bar, nav, tabs), mobile (bottom bar, stack, quick create),
   command palette, shortcut overlay, theme.
6. Inputs: DateField, MoneyField, QuickFill, LineGrid, RegisterGrid.
7. Screens: Home, Chart of Accounts, centres, register, Write Checks,
   ReportViewer, Find, lists.
8. Playwright smoke tests (desktop + 375px mobile), screenshots.
9. Final pass on this document.

## Findings / gotchas

- ts-rs maps Rust `i64`/`u64` `edit_seq` to TS `bigint`, but JSON carries a
  plain number. The client revives `edit_seq` into `BigInt` on read and writes
  bigints back as numbers, so the generated types stay truthful at runtime.
  Better long-term fix (Rust side, not this phase): `#[ts(type = "number")]` on
  `edit_seq`.
- `CompanyInfo`, `SaveOutcome`, `Report`/`ReportRequest` and the session shapes
  are not generated yet; `web/src/api/types.ts` and `web/src/reports/types.ts`
  hold local stand-ins to be swapped for generated ones.

## Progress log

- [ ] Step 1
- [ ] Step 2
- [ ] Step 3
- [ ] Step 4
- [ ] Step 5
- [ ] Step 6
- [ ] Step 7
- [ ] Step 8
- [ ] Step 9

## Open questions for the user

(none yet)

## Things not to do

- Don't hand-edit `web/src/api/generated/`.
- Don't do float arithmetic on money, rates or quantities.
- Don't bind browser-reserved keys (Ctrl+W/T/N, Ctrl+Tab, Ctrl+Shift+W/T/N) in
  the browser keymap; they never reach the page.
