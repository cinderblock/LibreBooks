# Desktop: several companies open at once, each in its own window

## Goal

The user keeps two companies (TWILL and the older QuickBooks 14 company) and
wants both open at the same time, side by side. The server already serves every
company in the folder at once (`lb-server` `Companies` keeps a map of open
books; the SPA routes are `/c/:companyId/*`), so two browser tabs work today.
The desktop app is the gap: it has one window (`main`), and `on_new_window`
sends every popup to the system browser.

## Environment / context

- `desktop/src-tauri/src/window.rs`: `open_main` builds the `main` window
  (init script with the token, navigation guard, WebView2 key lockdown, dropped
  file paths). `lib.rs`: `activate` rebuilds `main` when the backend restarts;
  closing `main` quits (or hides it while serving the network).
- SPA: `web/src/shell/CompanyPicker.tsx` (the list), `UserMenu.tsx` ("Open
  another company").

## Decisions already made (don't re-ask)

- Several windows, one backend (one server process): every window shares the
  same backend and token. (The user suggested windows.)

## Plan / steps

1. [x] **Bug first** (its own commit): the forms' remembered choices in
       localStorage (`librebooks.deposit.account`, `payBills.account`,
       `payment.account`, `purchase.<type>.account`, `sales.taxItem`) aren't per
       company, so one company's account id is offered in another. Key them by
       company id. Done (`912470a`).
2. [x] Desktop: company windows. Any window showing the SPA is a "company
       window" (`main`, plus `company-<n>`), built by one function with the same
       init script, guards and WebView2 setup. A same-origin `window.open` (or
       Shift+click on a link) opens a new company window instead of the system
       browser. (No separate shell call: `window.open` is the same in a browser,
       where it opens a tab.)
       Closing a window closes only it; the app quits when the last one closes
       (or hides it while serving). A backend restart closes them all and reopens
       one. The window title follows the page's title.
3. [x] SPA: "Open in new window" on the company picker and in the user menu
       (desktop and browser: in a browser it's a new tab), plus Shift/Ctrl/middle
       click on a company. Fixture mode is carried in the URL (it is per tab).
       The page title already named the company.
4. [ ] **Current:** Tests (e2e for the picker action and per-company memories; desktop unit
       tests for the URL rules), README, deploy to cad-3.

## Findings / gotchas

- Building a webview from inside WebView2's NewWindowRequested event can
  deadlock on Windows; the new window is built from a spawned task.
- A new window starts from `about:blank` like the main one, so the key lockdown
  and the dropped-file handler are in place before the SPA loads.
- `pick_company_file` parents its dialog on the window that asked, not `main`.
- Using a relative `CARGO_TARGET_DIR` from `desktop/src-tauri` makes a stray
  `desktop/src-tauri/target`; run the desktop build from the repo root.

## Things not to do

- Don't reopen every window after a backend restart: the origin and token
  change, and a switch to a remote server may not have the same companies.
