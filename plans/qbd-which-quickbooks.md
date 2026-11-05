# Which QuickBooks the SDK uses; the company file by drag or Browse

## Goal

cad-3 has two QuickBooks installed: Enterprise Solutions: Accountant Edition 14.0
(major version 24) and Pro 2021 (31). The user's older company was made in 14.0
and **must never be opened by 2021**: 2021 upgrades an older file in place, and
14.0 can't open it afterwards. Today the SDK transport will start QuickBooks
itself when the settings name a company file that isn't open, and with two
installed it may start 2021. So:

1. Each company chooses which installed QuickBooks it syncs from.
2. LibreBooks never lets the SDK start QuickBooks. The chosen QuickBooks must
   already be running, with no other version running beside it. An explicit
   "Open in QuickBooks 14.0" button starts *that* program with the file.
3. After connecting, `HostQuery` must report the chosen major version, or the
   session is closed before anything is read.
4. The company file path can be dropped onto the field (a `.QBW` from Explorer)
   or chosen with Browse…, in the desktop app.

## Environment / context

- cad-3, user `chtacklind`, signed-in session 1. QuickBooks registry:
  `HKLM\SOFTWARE\WOW6432Node\Intuit\QuickBooks\<major>.0\<edition>` with
  `Product` ("QuickBooks Pro 2021") and `Path` (…`\qbw32.exe`). Seen:
  `24.0\belacct` → `C:\Program Files (x86)\Intuit\QuickBooks Enterprise Solutions 14.0\qbw32.exe`;
  `31.0\pro` → `C:\Program Files (x86)\Intuit\QuickBooks 2021\qbw32.exe`.
  The 64-bit QuickBooks of 2022 and later should be under the 64-bit view
  (`HKLM\SOFTWARE\Intuit\QuickBooks\…`) with `QBW.EXE`; read both views.
- qbXML sent is 13.0 (`crates/lb-qbd/src/qbxml/request.rs`), which QuickBooks
  2014 (v24) supports.
- Detection lives in `lb-qbsdk` (it already has the `windows` crate); the guard
  is a pure function in `lb-qbd` so it is tested everywhere.

## Decisions already made (don't re-ask)

- The user approved the guard (2026-11-03): the chosen QuickBooks must already be
  running; a different running version refuses the sync. Applies to every
  company, so TWILL also needs 2021 open first.
- The user asked for a per-company choice of which QuickBooks to use.
- The user asked for drag-and-drop of the `.QBW` onto the path, plus Browse.
- Never open the old file with 2021 (memory: old-qbw-never-in-new-qbd).

## Plan / steps

1. [x] `lb-qbsdk::installs`: installed QuickBooks (registry, both views) and
       running ones in this session (process image path matched to an install).
2. [x] `lb-qbd::which` (the guard, pure, tested) and `sdk.rs`: the guard before
       connecting, the HostQuery version check after, `open_in_quickbooks`,
       `quickbooks_here`; `SdkSettings::quickbooks: Option<u32>`.
3. [x] `lb-server`: `quickbooks` in the import status; the choice passed to
       the SDK; `POST …/import/qbd/open-quickbooks`.
4. [x] CLI: the saved choice, `--quickbooks <major>`, `import quickbooks`.
5. [x] Web Import screen: the QuickBooks select (required with several), the
       "Open in …" button (uses saved settings), the chosen QuickBooks badge on
       the Sync tab, `CompanyFileField` (Browse…, drop).
6. [x] Desktop: `pick_company_file` (dialog, `allow-pick-company-file` on the
       main window) and `shell.pathOf(file)` through WebView2
       `postMessageWithAdditionalObjects`, local mode only, trusted origin only.
7. [x] Tests: guard unit tests, desktop clippy, e2e for Browse/drop (faked
       shell) and the browser fallback. README.
8. [x] Built (`9d37db8`). CLI deployed to cad-3 `bin\librebooks.exe`
       (0CBEDD92…, previous kept as `librebooks.prev.exe`); `import quickbooks`
       there lists `24 QuickBooks Enterprise Solutions: Accountant Edition 14.0`
       and `31 QuickBooks Pro 2021` with their `qbw32.exe` paths. (Over SSH
       neither shows running: a different Windows session.) Desktop app staged
       as `app\LibreBooks.new.exe` (67E3F55D…); a watcher swaps it in when the
       running app exits, keeping `LibreBooks.prev.exe`.
9. [ ] **Current:** live with the user: choose 14.0 for the old company, drop or
       browse its `.QBW`, Save, close 2021, "Open in …14.0", sync. Choose 2021 for
       TWILL.

## Findings / gotchas

- A browser never reveals a dropped file's full path. WebView2 does, to the host:
  JS `chrome.webview.postMessageWithAdditionalObjects(msg, [file])`, host
  `ICoreWebView2WebMessageReceivedEventArgs2::AdditionalObjects` →
  `ICoreWebView2File::Path`.

- Copying `librebooks.new.exe` and `LibreBooks.new.exe` to the same folder on
  Windows is one file (case-insensitive names): the second upload replaces the
  first. Stage the desktop app under a different name (`desktop-app.new.exe`).

## Things not to do

- Don't start QuickBooks through the SDK with a company file path.
- Don't open the old company file with anything but QuickBooks 14.0.
