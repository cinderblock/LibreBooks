# LibreBooks

Open-source double-entry accounting, built to be a complete drop-in replacement
for QuickBooks Desktop: the same books, forms, registers, lists, reports and
keyboard-first data entry, with no subscription. MIT licensed.

> **Status: early, under heavy construction.** The accounting core and the
> company file are working and tested; the server, web UI, desktop app,
> QuickBooks import and everything else in [the plan](plans/librebooks.md) are
> being built now. Not ready for real books yet.

## What it is meant to be

- **The books QuickBooks Desktop keeps**, one for one: the same account types,
  the same transaction types (checks, deposits, transfers, bills, bill payments,
  invoices, payments, sales receipts, credit memos, journal entries, credit card
  charges, inventory adjustments, estimates and orders…), the same name lists
  (customers and jobs, vendors, employees, other names), items, classes, terms
  and sales tax. Every transaction keeps both what was typed on the form and the
  double-entry postings it made; the file refuses anything that doesn't balance.
- **Three ways to run it**, one codebase:
  - a **desktop app** (Windows and Linux; macOS later) holding the books locally,
    which can also serve them to phones and other computers on your network;
  - a **headless server** (one binary or a container), first class;
  - the desktop app as a **thin client** to a remote server.
- **Phones are first class.** The web UI is built for a phone as much as for a
  desk: snap a receipt, check a balance, enter a bill.
- **Keyboard first.** QuickBooks' shortcuts (Ctrl+W write checks, Ctrl+I
  invoice, Ctrl+R register, Ctrl+J customer centre, `+`/`-`/`T`/`M`/`H` in
  date fields, QuickFill everywhere, Tab and Enter flow).
- **Passwordless.** Remote access uses passkeys; new devices join with a
  short-lived, single-use enrolment link. Desktop thin clients use a device key
  kept in the OS keychain.
- **Bring your QuickBooks data.** A read-only importer pulls a company out of
  QuickBooks Desktop (through the QuickBooks Web Connector, or the
  [QB Bridge](https://github.com/cinderblock/quickbooks-desktop-sdk-bridge)),
  incrementally, and compares LibreBooks' balances with QuickBooks' own so you
  can run both side by side until you trust it. It never writes to QuickBooks.
- **Feeds.** Bank and card transactions from OFX/QFX/QBO/CSV downloads,
  SimpleFIN, OFX Direct Connect and others, with matching rules; Amazon orders.
- **Built-in backups**: scheduled, verified, compressed, optionally encrypted,
  to a folder or any S3-compatible bucket.
- **Updates itself** from signed releases; automatic installs are opt-in.
- **One switch to publish** through a Cloudflare tunnel: paste the tunnel token.

## Repository layout

| Path | What |
| --- | --- |
| `crates/lb-core` | Accounting core: money, accounts, names, items, transactions, posting rules. Pure, no I/O. |
| `crates/lb-store` | The company file: SQLite, migrations, saving transactions, open balances, registers, audit trail. |
| `crates/lb-reports` | Reports. |
| `crates/lb-auth` | Passkeys, enrolment tokens, device keys, sessions. |
| `crates/lb-qbd` | QuickBooks Desktop import (read-only). |
| `crates/lb-feeds` | Bank and card feeds. |
| `crates/lb-backup` | Backups. |
| `crates/lb-update` | Self-update. |
| `crates/lb-tunnel` | Cloudflare tunnel wrapper. |
| `crates/lb-server` | HTTP API and embedded web UI. |
| `crates/librebooks` | The `librebooks` command. |
| `web/` | The web UI (React). |
| `desktop/` | The desktop app (Tauri). |
| `plans/` | Plans and decisions; start with [`plans/librebooks.md`](plans/librebooks.md). |

## Building

Needs Rust (stable, see `rust-toolchain.toml`) and [Bun](https://bun.sh).

```sh
cargo test                 # the Rust workspace
```

## Running a server

```sh
librebooks serve --bind 127.0.0.1:8740       # prints a first-admin link on first start
librebooks backup now | list | verify <id> | restore <id | file.lbk>
librebooks update check | install
librebooks tunnel setup --hostname books.example.com   # token on stdin
librebooks secret set <name>                 # value on stdin; refer to it as secret:<name>
```

Settings are in `server.json` in the data folder (also editable from the web UI);
credentials live in the encrypted `secrets.age`, never in the settings. Backups
are on by default (daily, into `<data folder>/backups`; add a second destination).
Under systemd, `Restart=on-failure` plus `RestartForceExitStatus=75` lets a
self-update restart the server; a full example unit, the Windows service setup,
and the reasons behind all of it are in
[`plans/server-ops.md`](plans/server-ops.md).

## Money

Amounts are whole cents (`i64` in Rust, integer `number` in TypeScript), never
floating point. Quantities and rates are decimals. Rounding to cents is half away
from zero, as QuickBooks does.

## License

[MIT](LICENSE).
