# Other QuickBooks replacements: the landscape

## Goal

Know what else exists, so LibreBooks borrows good ideas (never code from
incompatible licenses), knows where it is ahead, and knows what users of the
alternatives will expect. Surveyed 2026-11-04 (stars, licenses and dates from
the GitHub API that day). The gaps this survey suggested are folded into Phase 8
of [`librebooks.md`](librebooks.md).

## Findings

**Nothing else in the open aims at QuickBooks Desktop parity**: the same data
model, the same forms and keyboard, and full-history import through the SDK or
Web Connector. Every project below either rebuilds a QBD-like feature set its own
way, or targets QuickBooks Online / Xero-style web accounting.

### SlowBooks Pro 2026 (the closest)

<https://github.com/VonHoltenCodes/SlowBooks-Pro-2026>, by Trent Von Holten.

- Created 2026-04-11; ~2,450 commits; v2.31.6 on 2026-11-04, releasing several
  times a week. 846 stars, 156 forks. Two main authors (VonHoltenCodes 1,830
  commits, ContractorKeith 412); 143 commits by `claude`.
- **License: source-available, not open source** ("SlowBooks Pro 2026 — Source
  Available License, version 2.0"). Free to use, modify and redistribute,
  including for running a for-profit business; may not be sold, rented or
  offered as a paid service. A program that only interoperates through its HTTP
  API, import/export formats or company-file format is explicitly *not* a
  derivative work. **Consequence for us: never copy its code into MIT
  LibreBooks; studying its feature list and interoperating are fine.**
- Story: the author ran QuickBooks 2003 Pro for 14 years; Intuit's activation
  servers went away, a disk died, no reinstall. Early versions carried invented
  "decompiled from QBW32.EXE" comments, removed in v2.14; the README now states a
  from-scratch reimplementation.
- Stack: Python + FastAPI, PostgreSQL (SQLite for desktop company files and
  tests), SQLAlchemy/Alembic, vanilla HTML/JS with no build step, WeasyPrint PDFs,
  pywebview + PyInstaller desktop. Signed Windows installer, notarized Apple
  Silicon DMG; Docker for Linux/Intel Mac.
- Features: AR/AP (invoices, estimates, credit memos, POs, bills, vendor credits,
  recurring, batch), journals with closing date and audit log, registers, check
  printing, OFX/QFX/CSV, SimpleFIN and Mercury feeds, rules, reconciliation;
  P&L/BS/TB/GL/cash flow/agings/sales tax/Schedule C and a PDF pack; **US
  payroll with W-2/W-3/940/941**, deductions, garnishments, PTO, an employee
  portal; perpetual inventory (weighted average); job costing; receipt OCR;
  Stripe/PayPal/Square; fixed assets; nonprofit and construction modes; 90-day
  cash forecast; bring-your-own-key AI; multi-user Server Edition with roles;
  company-file encryption with a recovery key; WCAG 2.1 AA target.
- **QuickBooks Desktop import is IIF only.** QBD's IIF export covers lists, and
  transactions only through workarounds (see `qbw-research.md`, "IIF export /
  Find → CSV"). It also imports QuickBooks Online's export zip, Xero, MYOB, Sage
  50, Wave, Zoho Books and GnuCash.
- No keyboard-shortcut parity is claimed; no passkeys, tunnel, self-update or S3
  backups mentioned.

### New in 2026

| Project | What | License | State (2026-11-04) |
| --- | --- | --- | --- |
| [TitusBooks](https://github.com/ChrisTitusTech/titusbooks) (Chris Titus Tech) | Desktop bookkeeping; C#/.NET 10, Avalonia, ASP.NET Core, PostgreSQL. Accounts, registers, manual posting, reports, CSV import. No QuickBooks import. | **None** (no LICENSE: not legally open source) | 42 stars; created 2026-06-16, last push 2026-07-14. The author's YouTube audience could make it widely known overnight. |
| [dubbl](https://github.com/dubbl-org/dubbl) | Web, team-oriented "alternative to Xero and QuickBooks"; Next.js/Postgres. Very broad: payroll, inventory (BOM, lots), CRM, budgets, multi-currency, REST API, MCP server. | Apache-2.0 | 62 stars; created 2026-03-02, last push 2026-10-11. |
| [Slashbooks](https://github.com/giltotherescue/slashbooks) | Not an app: an AI agent (Claude Code, Codex) keeps books in a local SQLite ledger from QuickBooks exports and bank feeds. | Apache-2.0 | 37 stars; active. |
| [openbook](https://github.com/TronMach/openbook), [nonprofit-ledger](https://github.com/NLSM-JeremiahOBrien/nonprofit-ledger), [FastBooks](https://github.com/junitygroup-bit/FastBooks) | Fresh starts, 0 stars. | AGPL / AGPL / ? | Watch only. |

### Established, not QBD-shaped

| Project | Notes | License | Stars |
| --- | --- | --- | --- |
| [GnuCash](https://github.com/Gnucash/gnucash) | The classic free desktop double-entry; clumsy small-business AR/AP; no real QB import. | GPL | 4.4k |
| [Frappe Books](https://github.com/frappe/books) | Electron small-business desktop accounting; closest in spirit. | AGPL-3.0 | 5.0k |
| [Bigcapital](https://github.com/bigcapitalhq/bigcapital) | Web, QBO/Xero-shaped; no payroll. | AGPL-3.0 | 4.1k |
| [Akaunting](https://github.com/akaunting/akaunting) | Web; free core, paid apps (payroll, double-entry, reports). License not detected by GitHub. | ? | 10.3k |
| [LedgerSMB](https://github.com/ledgersmb/LedgerSMB) | SQL-Ledger lineage, Perl, still active. | GPL | 573 |
| [ERPNext](https://github.com/frappe/erpnext), [Odoo](https://github.com/odoo/odoo), [Dolibarr](https://github.com/Dolibarr/dolibarr) | Full ERPs; far heavier than bookkeeping. | GPL / LGPL+proprietary / GPL | 41k / 55k / 7.8k |
| [Beancount](https://github.com/beancount/beancount), [hledger](https://github.com/hledgerorg/hledger) | Plain-text accounting; a good export target. | GPL | 6.1k / 4.8k |

Invoice Ninja is source-available (Elastic); Crater has had no push since
2024-08.

### QBD data tools (relevant to the importer)

Already in `qbw-research.md`: OpenQBW, OpenSQLAnywhere, qbsync, QODBC. New:
[quickbooks-desktop-extractor](https://github.com/josephsprei-lang/quickbooks-desktop-extractor)
(13 stars, Python, SDK → JSON for customers, items, vendors, invoices with lines,
payments, AR): a much smaller cousin of our SDK transport.

## What this suggests (folded into Phase 8)

- A document pipeline (form templates → PDF → print/email) is table stakes.
- Payroll is in every serious competitor (SlowBooks, dubbl, ERPNext).
- Migration from QBO, GnuCash, Xero and IIF widens the audience beyond QBD.
- Company-file encryption at rest, an accountant role, receipt OCR, payment
  links, and an agent-facing API/MCP server are now common.

## Things not to do

- Don't copy code, templates or assets from SlowBooks (source-available license)
  or from AGPL/GPL projects into MIT LibreBooks. Read for ideas only.
- Don't treat TitusBooks as reusable: it has no license at all.
