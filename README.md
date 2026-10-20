# LibreBooks

Open-source double-entry accounting, built to be a complete drop-in replacement
for QuickBooks Desktop: the same books, forms, registers, lists, reports and
keyboard-first data entry, with no subscription. MIT licensed.

> **Status: early, under heavy construction.** The accounting core, company
> file, server, passkey sign-in, reports, bank feeds, backups, the read-only
> QuickBooks Desktop import and the web UI's foundation are working and tested;
> most forms, the desktop app and the rest of [the plan](plans/librebooks.md)
> are being built now. Not ready for real books yet.

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
  QuickBooks Desktop three ways: through the QuickBooks Web Connector (ships
  free with QuickBooks; nothing else to install), the QuickBooks SDK when
  LibreBooks runs on the QuickBooks machine, or the
  [QB Bridge](https://github.com/cinderblock/quickbooks-desktop-sdk-bridge).
  Incremental, and it compares LibreBooks' balances with QuickBooks' own so you
  can run both side by side until you trust it. It never writes to QuickBooks:
  every transport can only send queries, by construction.
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
cargo test                       # the Rust workspace
bun install && bun run --cwd web check  # the web UI: typecheck and unit tests
bun run --cwd web dev            # the web UI on http://localhost:5740/?fixtures=1
                                 # (fixture mode: a synthetic company, no server)
bun run --cwd web build          # web/dist, which the server embeds
cargo run -p librebooks -- serve # the server on http://127.0.0.1:8740
```

The first `serve` prints a one-hour link to create the first admin with a
passkey. `librebooks --help` lists the rest: companies, enrolment links, the
QuickBooks Desktop import (`import qbd --transport bridge|sdk`, read-only), its
parity check, and the Web Connector set-up (`import qwc`, `import
qbwc-password`; the sync itself is started from the server, which QuickBooks'
Web Connector polls). Set-up steps per transport: `plans/qbd-import.md`.

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
The reasons behind all of it are in [`plans/server-ops.md`](plans/server-ops.md).

### As a service

The same server, started with the machine. `service install` takes `serve`'s
options and bakes them, with the data folder, into the service's command line,
so the service runs exactly what `serve` would; `--print` shows what would be
registered without registering anything.

```sh
# Windows (from an administrator prompt): a native service, "LibreBooks" in services.msc
librebooks service install --data-dir "D:\LibreBooks" --bind 0.0.0.0:8740 --public-url https://books.example.com
librebooks service start | stop | status | uninstall

# Linux: a systemd unit for this user (~/.config/systemd/user/librebooks.service) ...
librebooks service install --data-dir ~/books --bind 127.0.0.1:8740
# ... or system-wide (/etc/systemd/system), as a dedicated account that owns the data folder
sudo librebooks service install --system --run-as librebooks --data-dir /var/lib/librebooks
librebooks service start | stop | status | uninstall [--system]
```

On Windows a Stop from the service manager is the same graceful shutdown as
Ctrl+C (requests finish, a backup in progress completes, cloudflared stops),
the log is `logs/server.<date>.log` in the data folder (a service has no
console), and the recovery actions registered at install restart the service
after a failure and after the exit code 75 a self-update leaves with.
`--account` runs it as something other than LocalSystem; the account must be
able to write the data folder and the program's folder.

On Linux the unit has `Restart=on-failure`, `RestartForceExitStatus=75`,
`KillSignal=SIGTERM`, `TimeoutStopSec=90` and sandboxing (`ProtectSystem=strict`
with only the data folder and the program's folder writable, `PrivateTmp`,
`NoNewPrivileges`). `start`, `stop` and `status` are `systemctl`. A user unit
stops at logout unless `loginctl enable-linger` is set.

## Money

Amounts are whole cents (`i64` in Rust, integer `number` in TypeScript), never
floating point. Quantities and rates are decimals. Rounding to cents is half away
from zero, as QuickBooks does.

## License

[MIT](LICENSE).
