# Native QuickBooks company files (`.QBW`, `.QBB`, `.QBM`): research

Phase 9 of `plans/librebooks.md`. Research only; no code. Written 2026-10-20 from
public sources (every factual claim carries a link; where a claim is my
inference it says so). Nothing here is legal advice.

## Goal

Decide whether LibreBooks should read a QuickBooks Desktop (QBD) company file
*directly*, without QuickBooks running or installed, so a user could import or
open a `.QBW` (or a `.QBB` backup, or a `.QBM` portable file) natively; and if
not, what the sanctioned paths can and cannot deliver, so the import we already
have (see `plans/qbd-import.md`) can be finished with its eyes open.

## Environment / context

- Three import transports exist today and share one normaliser: the QB Bridge
  (REST, GET only), the QuickBooks Web Connector (qbXML over SOAP, `/qbwc`),
  and the SDK over COM (`QBXMLRP2`, Windows, same machine as QuickBooks). All
  three need a QuickBooks installation somewhere; all three are read-only by
  construction. Details and the live-run checklist: `plans/qbd-import.md`.
- Intuit stopped selling new US subscriptions to Pro Plus, Premier Plus, Mac
  Plus and Enhanced Payroll after 2024-09-30 (Enterprise unaffected; existing
  subscribers may renew) — the business reason people will arrive with a
  `.QBW` and no working QuickBooks.
  <https://quickbooks.intuit.com/r/whats-new/quickbooks-desktop-stop-sell/>
- The user's own books are in QBD on `cad-3.tsl`; nothing in this research
  touched them, and nothing here should until the user says so.

## Decisions already made (don't re-ask)

From `plans/librebooks.md`: MIT licence; "native QBW support would be neat;
importing into our own format is acceptable"; QBD stays read-only until the
user retires it; importing must not require the QB Bridge (hence the Web
Connector and SDK transports). Nothing in this document reopens those.

## Findings

### 1. The on-disk format

**The engine.** QuickBooks 2006 (Enterprise 6.0) was "the first version with
locked SQL Sybase database format"; 2005 was "the last version with original
b-tree database format" (Qtools' release table,
<https://www.qtools.com/qbupdates.htm>). Contemporary press said QuickBooks
2006 "uses the iAnywhere Sybase database" (PCMag, quoted at
<https://forum.servoy.com/t/quickbooks-now-runs-on-ianywhere-sybase/5311>);
a 2007 column notes the Pro/Premier engine is the same as Enterprise's,
"tuned" to five concurrent users
(<https://www.cpapracticeadvisor.com/2007/09/01/tuning-performance-of-quickbooks-on-a-network/3825/>);
Insightful Accountant (2014): "Since 2006, Intuit has used Sybase as the
'database' for QuickBooks", with a personal server for stand-alone installs
and a network server for multi-user
(<https://insightfulaccountant.com/accounting-tech/general-ledger/the-data-detective---quickbooks-database-quiz>).
The engine is SAP (formerly Sybase, originally Watcom) SQL Anywhere; Wikipedia
lists it as used in Intuit QuickBooks and gives the version dates: 9 (2003),
10 (2006), 11 (2008), 12 (2010), 16 (2013-04-18), 17 (2015-07-15)
(<https://en.wikipedia.org/wiki/SQL_Anywhere>). Intuit's only public word is
indirect: a QuickBooks Team reply (2022-08-18) that "the SQL server is not
supported" for QBD 2021
(<https://quickbooks.intuit.com/learn-support/en-us/other-questions/sql-server/00/1129755>);
the "proprietary database technology based on Sybase" phrase often quoted
comes from a community member in that thread, not Intuit.

**Which SQL Anywhere for which QuickBooks year.** Sparse, and mostly
third-party:

| QuickBooks | SQL Anywhere | Evidence |
| --- | --- | --- |
| 2006–2013 | unknown (SA 9–12 by date; **inference**) | no source found |
| 2014 (Enterprise 14) | **11** | Linux DB server binary at `/opt/qbes14/SYBSsa11/bin/QBDBMgrN_24` (<https://steakwiki.com/index.php/Quickbooks_Database_Server_Manager>) |
| 2018 | **17.0.4.2182** | a QB 2018 user's Database Server Manager "DB Engine Version" (Intuit community post surfaced by search; the thread page I could fetch did not show the number — treat as unconfirmed) |
| 2018 → Enterprise 24 | **17.0.4 build 2182** | page 0 of 112/112 sample files carries `"2182 SAP SE, Copyright (c)2015 17.0.4."` (OpenQBW spec, below) |

So for every QuickBooks year LibreBooks is likely to meet (2018–2024, the
subscription era), the container is one SA 17.0.4 build. The service name
`QuickBooksDBXX` and the user `QBDataServiceUserXX` carry the QuickBooks
internal version (24 = 2014; 33 = 2024 per one unofficial source);
`QBDBMgrN.exe` is the network server, `QBDBMgr.exe` the personal one
(<https://www.file.net/process/qbdbmgrn.exe.html>,
<https://www.file.net/process/qbdbmgr.exe.html>). Task Manager shows it as
"SQL Anywhere Network Server"
(<https://notes.ponderworthy.com/quickbooks-now-using-sybase-sql-at-its-heart>).

**The file header (QBW, 2018–2024).** From the OpenQBW draft specification
(reverse-engineered from a 112-file, 1.87 GiB corpus; self-described as
"incomplete, in-progress reverse engineering";
<https://raw.githubusercontent.com/Sigilweaver/OpenQBW/main/SPECIFICATION.md>):

- 4096-byte pages; every file is an exact multiple of 4096 bytes.
- Superblock: `u32_LE` at `0x14` = `0xDA7ABA5E` (bytes `5E BA 7A DA`); a
  "format triple" 3 / 201 / 12 at `0x10`/`0x18`/`0x1A` (an engine constant,
  not a version signal); a page-count hint at `0x1C` (usually
  `total_pages − 128`); an unexplained `flags_06` byte.
- Every page ends in a CRC-32 (zlib) of its first 4092 bytes — 100 % match
  across 456,521 pages; a page-type letter at `0xFF2` (`E` data, `A`
  allocation, `C` catalog, `I` index, `H`, `M`, `@`, `G`).
- Page 0 names the collation (`1252LATIN1`, `windows-1252`, `UCA`).
- The SA system catalog (`SYSTABLE`, `SYSCOLUMN`, `SYSINDEX`, `SYSOBJECT`) is
  readable; QuickBooks' own schema sits on top as ordinary user tables.

A recovery vendor documents a different QBW signature, `MAUI` at offset 0x60
with 1024-byte blocks (<https://www.file-recovery.com/qbw-signature-format.htm>);
that is almost certainly the pre-2006 b-tree format (**inference** from the
block size and the 2006 switch). Any native reader must sniff both.

**"Encryption".** SQL Anywhere offers `-ea none | simple | AES | AES256 |
AES_FIPS | AES256_FIPS`; "Simple encryption is equivalent to obfuscation and
is intended only to keep data hidden" and needs no key; strong encryption
needs a key and "If you lose the encryption key there is no way to access the
data, even with the assistance of technical support"
(<https://infocenter.sybase.com/help/topic/com.sybase.help.sqlanywhere.12.0.1/dbadmin/dbinit.html>).
What OpenQBW found in QuickBooks files is a per-sector additive arithmetic
progression mod 256 — `stored[i] = (base + i·step + plaintext[i]) mod 256`
with `(base, step)` a function of `(page, sector)` only: "No secret key is
required at any point", it is "file-independent" (about 14 % of pages are
byte-identical between unrelated companies), and there is "no keyed
encryption layer" (SPECIFICATION.md, above). Whether that *is* SA's `simple`
mode or an Intuit layer is not stated by either Intuit or SAP; OpenQBW's
landing page calls it "Intuit's encryption layer", its README "Intuit's
additive-progression obfuscation". **Inference:** it is keyless either way,
so the bytes are readable without any secret; the barrier is the undocumented
schema, not cryptography. Consistent with that: Passware's QuickBooks Key
removes QuickBooks user/admin passwords from `.QBW`/`.QBA` "instantly,
regardless of passwords used" for QuickBooks 5.0 through 2024 (€39;
<https://www.passware.com/quickbooks/>) — the password gates the application,
not the bytes; and a 2016 advisory describes QuickBooks running the Watcom
SQL embedded in company files with DBA privileges without validation
(<https://www.exploit-db.com/raw/39804>).

**Can a stock SQL Anywhere engine open it?** `QBDBMgrN` *is* an SA network
server, so an engine opens it every day; the question is whether a *client*
can log in. Nobody in the sources has documented doing so: a 2015 hashcat
thread records a failed direct connection ("can't establish a connection to
the database") with no resolution
(<https://hashcat.net/forum/thread-4150.html>); the Linux QBES notes guess
that Sybase Central could discover the server but admit not knowing the
credentials (the server log shows a login as `41646d696e` = "Admin")
(<https://steakwiki.com/index.php/Quickbooks_Database_Server_Manager>); a
read-only sync project states the file "is encrypted, Intuit provides no
credentials for it, and direct access risks corrupting the file"
(<https://github.com/riccanet/qbsync>). SAP's own forum, on a QuickBooks POS
database: "specific recovery of the Intuit database must be done by Intuit
personnel"
(<https://sqlanywhere-forum.sap.com/questions/8807/need-to-repair-a-quickbooks-pos-database-sql-anywhere-9-db>).
Even if a login were found, it would mean running SAP's engine (licensing,
below) and Intuit's embedded SQL with DBA rights on the user's file — the
exact risk `qbsync` names.

**`.QBB` (backup).** Intuit: "When you back up your company file, QuickBooks
creates a backup file with a .qbb extension"
(<https://quickbooks.intuit.com/learn-support/en-ca/getting-started/file-types-and-extensions-used-by-quickbooks-desktop/00/261705>).
PRONOM registers the format (fmt/1354) with signature `45 86 00 00 06 00` at
offset 0 and no further structure
(<https://www.nationalarchives.gov.uk/pronom/fmt/1354>). Vendors say it also
carries templates, logos, letters and images; one claims WinZip "sometimes"
extracts it (unverified marketing;
<https://cleverence.com/articles/quickbooks-documentation/opening-qbb-files-without-quickbooks-8234>).
Passware requires a `.QBB` to be restored to `.QBW` in QuickBooks first. No
public specification and no open-source reader exist; OpenQBW has no `.QBB`
samples.

**`.QBM` (portable company file).** Intuit: "a compact version of your company
file" that "doesn't include your message templates, logos, images, or
templates", nor ".tlg or .log files, Loan Manager, Statement Writer, or Fixed
Asset Manager files"; "isn't a replacement for a full backup"; QuickBooks
"automatically reindexes your data" on open
(<https://quickbooks.intuit.com/learn-support/en-us/back-up-data/create-or-restore-a-quickbooks-portable-company-file/00/203400>).
The 2016 advisory says `.QBX` and `.QBM` "are compressed SQL dumps" — a
zlib-compressed `reload.sql` inside a compound file
(<https://www.exploit-db.com/raw/39804>); Intuit's own troubleshooting article
"Fix Error in reload.sql" exists precisely for `.QBM`/`.QBX` restores
(<https://quickbooks.intuit.com/learn-support/en-us/help-article/data-utilities/fix-error-reload-sql/L4avVlq5T_US_en_US>),
which corroborates that a portable file is an SQL Anywhere *unload* (schema
plus data as SQL text), not a page image. **Inference:** of the three
containers, `.QBM` is the most tractable to parse (decompress, then read SQL)
— but the table and column names are still Intuit's undocumented schema. A
third-party claim that `.QBM` drops the audit trail is unverified
(<https://cleverence.com/articles/quickbooks-documentation/portable-company-file-quickbooks-intuit-7836>).

**`.TLG` (transaction log).** Intuit: "QuickBooks keeps track of changes to
your company file since your last backup"; it resets at the next backup
(file-types article above). Auto Data Recovery pairs the `.TLG` with
`.QBW.adr` copies to replay recent transactions; it exists in Pro, Premier and
Enterprise, not Accountant editions, and only for files under 1.5 GB
(<https://quickbooks.intuit.com/learn-support/en-us/help-article/update-products/recover-lost-data-quickbooks-auto-data-recovery/L6vZ97286_US_en_US>).
**Inference:** it is SQL Anywhere's transaction log under another name; SAP's
`dbtran` "translates a transaction log into a SQL script file" directly from
the log file and needs a key only for strongly encrypted databases
(<https://infocenter.sybase.com/help/topic/com.sybase.help.sqlanywhere.12.0.1/dbadmin/dbtran.html>).
Whether a `.TLG` shares the `.QBW` superblock is an open item in OpenQBW's
spec.

**`.ND` (network descriptor).** Intuit: created in multi-user mode, "helps
QuickBooks manage your user access settings" (file-types article). It is a
small text configuration file (server host/port for the Database Server
Manager) with no financial data, regenerated by the server if deleted
(<https://www.cleverence.com/articles/quickbooks-documentation/.nd-files-in-quickbooks-desktop-7932/>;
the Linux notes list `.ND`, `.DSN`, `.TLD` as regenerable path files).
Irrelevant to import.

### 2. Prior art

| Tool | What it actually does | Needs QuickBooks / Intuit's DB manager? |
| --- | --- | --- |
| **OpenQBW** (Sigilweaver; Rust + Python; Apache-2.0; <https://github.com/Sigilweaver/OpenQBW>) | Reads the SA17 page store directly, removes the obfuscation in memory, walks `SYSTABLE`/`SYSCOLUMN`/`SYSINDEX`, extracts "invoice line items and transaction headers at row granularity", exports catalog + line items to SQLite. On the Rock Castle sample: "13,375 / 13,375 invoices and a grand total of $399,914,792.78", matching QuickBooks. Non-goals: shipping Intuit code, "Breaking passwords or DRM", writing. 101 commits, 11 stars at the time of reading. | **No.** Pure file parsing. |
| **OpenSQLAnywhere** (`opensqlany`; same author; Apache-2.0; <https://github.com/Sigilweaver/OpenSQLAnywhere>, <https://sigilweaver.app/opensqlanywhere/docs/>) | Clean-room SA17 (build 2182) page-store reader: superblock, per-page CRC, slotted-page row directories, `ApModel` de-obfuscation. "Derived from observation of the on-disk bytes of SQL Anywhere files and from SAP's own public documentation"; "ships with no SAP code or binaries". Column-level decoding of arbitrary user rows is "the v0.2 milestone"; upstream spec still lacks a reliable byte offset for money/quantity/rate fields and a rule for multi-page `LONG VARBIT` values. | **No.** |
| **Balaxxe/openqbw** fork (<https://github.com/Balaxxe/openqbw>) | Adds an "accounting path": chart of accounts + a normalised posting ledger → accrual trial balance, GL, P&L, balance sheet (CSV/JSON/SQLite), for **QuickBooks Enterprise 24 R21 only**, matched against a schema manifest; "other QB versions, unknown schemas/layouts/families" are rejected fail-closed. Private acceptance "reconciled … to native QuickBooks reports at three as-of dates … zero-cent variance". Cash basis not supported. | **No.** |
| **QODBC** (and Intuit's Enterprise ODBC driver, which is QODBC) | SQL → qbXML → the SDK. "QODBC or QRemote cannot connect directly to the QuickBooks company file"; "QODBC cannot push or pull data from QuickBooks if your application is not running"; auth is QuickBooks' integrated-application certificate (<https://qodbc.com/?p=72>). | **Yes**, QuickBooks running (QRemote lets it run on another host). |
| **qbsync** (riccanet; <https://github.com/riccanet/qbsync>) | Read-only incremental sync to Postgres/n8n via `QBXMLRP2` COM, `*Query` only. Explicitly refuses direct DB access (quoted above). | **Yes**, QuickBooks installed on the workstation. |
| **Stellar Repair for QuickBooks** and similar repair vendors | Scan a damaged `.QBW` and "preview recoverable data"; versions 2007–2024 claimed (<https://www.stellarinfo.com/help/stellar-repair-for-quickbooks-software-12-windows-technician-en-about-stellar-repair-for-quickbooks-software.html>). Closed source; method undocumented; marketing claims. | Unclear; the product is a parser, the restore typically needs QuickBooks. |
| **Passware QuickBooks Key** | Removes QuickBooks user passwords from `.QBW`/`.QBA` instantly (above). | **No** for the removal; QuickBooks to open the result. |
| **Grideon "QuickBooks Forensics"** (~$99) | A forensics forum reports it "was able to immediately crack the encrypted QBW file" (QuickBooks 2016); the vendor page was not reachable to me (<https://www.forensicfocus.com/forums/general/decryption-of-quickbooks-2016-qbw-file-solved/>, 403 on fetch; anecdote via search). | Reportedly **no**. |
| **Intuit: Verify / Rebuild Data** | Integrity check and repair inside QuickBooks (File › Utilities); back up first because a rebuild "can in rare cases delete transactions"; persistent errors (503, C=44) are not fixed by it (<https://quickbooks.intuit.com/learn-support/en-ca/help-articles/verify-and-rebuild-data-in-quickbooks-desktop/00/389186>, <https://quickbooks.intuit.com/learn-support/en-us/help-article/damaged-data/resolve-potential-data-issues-quickbooks-desktop/L0VSmlulL_US_en_US>). | **Yes** (it is QuickBooks). |
| **Intuit: File Doctor** (Tool Hub) | Fixes company-file and network errors (-6xxx, H2xx); download fresh each time. Sources do not state whether it runs without QuickBooks installed; it operates on `.QBW` files in place. | Treat as **yes** (same install family). |
| **Intuit: Auto Data Recovery** | Rebuilds from `.QBW.adr` + `.TLG` (above). | **Yes**. |
| **IIF export / Find → CSV** | Lists export cleanly; transactions via Edit › Find › Export, which QuickBooks 2015 "will silently cap" at 32,768 lines; starting balances, bill/invoice metadata lost (<https://lwn.net/Articles/729087/>, 2017-08-07, with the GPL `qb-escape` scripts). Intuit staff say IIF can import but not export transactions (<https://quickbooks.intuit.com/learn-support/forums/replypage/board-id/other-questions/message-id/142090>) — the two statements conflict; the Find-export route is the one that works. No payroll export by IIF. | **Yes**. |
| **Intuit Data Services** (paid repair/conversion) | Intuit staff work on the file. | Intuit. |

Net: the only things that read a `.QBW` without QuickBooks are closed
forensic/repair products with undocumented methods, and one young Apache-2.0
clean-room project that can so far decode the catalog and invoice line items
upstream, and one QuickBooks build's full ledger in a fork.

### 3. Legal and licensing

**Intuit's EULA.** The US "QuickBooks Desktop and Payroll End User License
Agreement" (2020, 2021, 2022 editions) and the Canadian Desktop agreements
(2021–2024) include, among the things the licensee agrees not to do:
"Reproduce, duplicate, modify, copy, deconstruct, reverse-engineer, sell,
trade or resell the Software" and attempts at "unauthorized access to any
other Intuit systems" (<https://www.intuit.com/legal/terms/en-us/quickbooks/desktop-payroll-2022>,
<https://www.intuit.com/legal/terms/en-ca/quickbooks/desktop-2024>; intuit.com
rate-limited my fetches, so the quotes are from search excerpts of those
pages — re-read the full text before relying on them). The Point of Sale
agreement additionally forbids decompiling and disassembling
(<https://www.intuit.com/legal/terms/en-us/quickbooks/desktop-pos>). I found
no clause that names the company file or its database as such.
**Inference:** the EULA binds the QuickBooks licensee, not LibreBooks; the
ledger data in a company file is the licensee's; but (i) decompiling
`QBW32.EXE`/`QBDBMgrN.exe` to learn the format is squarely
"reverse-engineer the Software", and (ii) whether reading the *file* counts as
"deconstructing the Software" is arguable, since the file carries Intuit's
schema and embedded SQL. OpenQBW's posture — observe bytes of files you
lawfully own, never touch Intuit binaries, never ship Intuit code — is the
defensible shape.

**DMCA §1201.** The reverse-engineering exception, §1201(f), permits
circumventing a measure that controls access to "a computer program" to
identify the elements "necessary to achieve interoperability of an
independently created computer program", and (f)(2) permits building the
tools (<https://www.project-disco.org/?p=8314>, Jonathan Band, 2017-07-06,
summarising the Copyright Office's 2017 §1201 report, which also widened who
may use such tools). Critics have long noted it "applies to executables, but
not data", and the DeCSS court (Universal v. Reimerdes) held it does not
authorise circumventing measures that control access to *other* works
(<https://cacm.acm.org/magazines/2015/7/188738-anti-circumvention-rules-limit-reverse-engineering/abstract>,
Samuelson, CACM July 2015; comments at
<https://www.copyright.gov/1201/comments/224.pdf>). The open questions a
lawyer would have to answer: is a keyless additive progression a
"technological measure that effectively controls access" (courts have
accepted weak measures), and whose copyrighted *work* does it protect — the
user's ledger (theirs) or Intuit's schema and embedded SQL (Intuit's)? The
provision that bites a *published* tool is the trafficking ban
(§1201(a)(2), (b)), not the act of a user reading their own file. Nobody has
sued OpenQBW; that proves nothing either way.

**SAP SQL Anywhere.** Proprietary. The free Runtime Engine "is no longer
offered"; redistribution needs an OEM agreement "and there is a cost to this"
(SAP forum, 2016; <https://sqlanywhere-forum.sap.com/questions/26713/runtime-engine>).
An SAP representative told OEMs "You are not allowed to sell/distribute (SQL
Anywhere) licenses after 31.12.25", later softened to "re-evaluating"; SA 17's
end of mainstream maintenance is listed as 2028-12-31
(<https://sqlanywhere-forum.sap.com/questions/38344>; forum-sourced, the
pages returned 403 to my fetches). The Developer Edition is free but "cannot
be used in production"
(<https://sqlanywhere-forum.sap.com/questions/11452/convert-developer-edition-to-chip-license>).
**Conclusion:** shipping or depending on any SAP binary (`dbeng17`,
`dbsrv17`, `dblib`) is a non-starter for an MIT project, and so is depending
on Intuit's `QBDBMgrN`. A clean-room page-store reader involves no SAP code;
OpenSQLAnywhere's docs are CC-BY-SA 4.0 and its code Apache-2.0, both usable
as a dependency of an MIT project (Apache-2.0 is MIT-compatible for inclusion
as a crate; LibreBooks' own code stays MIT).

**What LibreBooks does today is clean.** The Web Connector and `QBXMLRP2`
ship with QuickBooks and are used on the licensee's machine under Intuit's
published SDK; LibreBooks ships none of them.

### 4. The sanctioned alternatives and their ceilings

**qbXML via SDK / Web Connector.** Intuit's SDK 14.0 release notes
(2020-12-10) give the version table: QuickBooks 2018 (R16+) through 2021
speak qbXML 14.0 and everything down to 1.0; 2014 → 13.0; 2013 → 12.0; …;
2006 → 2.1; 2002 → 1.0 (<https://static.developer.intuit.com/resources/ReleaseNotes_QBXMLSDK_14_0.pdf>).
SDK 16.0 pairs with QuickBooks 2023 and qbXML 17.0 with QuickBooks 2024 (R18+)
and 2023 (R16+) (<https://developer.intuit.com/app/developer/qbdesktop/docs/get-started/sdk-compatibility-with-quickbooks-releases>,
<https://cdn.cdata.com/help/RQN/cis/RSBQuickBooks_p_QBXMLVersion.htm>). The
request processor "is installed automatically by QuickBooks" — so QuickBooks
must be installed; "unattended" access needs the admin's one-time grant
"Yes, allow access even if QuickBooks is not running", and even then the
QuickBooks UI opens in the background
(<https://developer.intuit.com/app/developer/qbdesktop/docs/develop/connections-sessions-and-authorizations>,
<https://developer.intuit.com/app/developer/qbdesktop/docs/additional-reference/qbfc-iqbauthpreferences-reference>).
SDK applications are unavailable while QuickBooks runs in multi-instance mode
(release notes above). A trial install does not support the SDK
(<https://github.com/Donny-GUI/quickbooks-desktop-python>, third-party).

Not reachable through qbXML (from Intuit's object table and report list — the
pages are JavaScript-rendered and would not fetch, so this is my reading of
the OSR as used in `lb-qbd`'s capability table; verify each against
<https://developer.intuit.com/app/developer/qbdesktop/docs/additional-reference/quickbooks-objects-and-operations-accessible-with-the-sdk>
and <https://developer.intuit.com/app/developer/qbdesktop/docs/additional-reference/reports-that-can-be-requested-with-the-sdk>):

- **Payroll detail.** Paychecks surface only as `TransactionQuery` headers;
  item breakdowns come only from the payroll *report* queries, and those work
  only "if your application is accessing a company file that is currently
  signed up for a subscription to a payroll service" (release notes above).
  `PayrollItemWage`/`NonWage` lists are queryable.
- **Audit trail.** Not a report category the SDK offers; it is a UI report
  (Reports › Accountant & Taxes › Audit Trail) exportable to Excel by hand.
- **Attachments (Doc Center).** No qbXML object. But Intuit staff confirm the
  files live on disk in an `Attach` folder beside the company file, with
  `Txn` and `Inbox` subfolders
  (<https://quickbooks.intuit.com/learn-support/en-us/other-questions/attachments-no-longer-supported/00/726646>,
  <https://www.coraltreetech.com/qbox-blog/quickbooks-attachments>). The
  bytes are importable without any parsing; how the folder names map to
  `TxnID`s needs checking on a real file.
- **Memorized transactions, memorized reports, letters, form templates'
  layout, users and roles, bank-feed connections and downloaded-but-unmatched
  items, reconciliation history** (only each transaction's `ClearedStatus`),
  budgets (reports only, no list object), most preferences (a read-only
  subset via `PreferencesQuery`).
- `TxnDeletedQuery`/`ListDeletedQuery` exist (deletions are reachable, as the
  Web Connector plan already uses).

**IIF / CSV / report exports.** Lists complete; transactions only via the
Find → export route (capped) or report exports (no `TxnID`, so totals only,
as `plans/qbd-import.md` found for the bridge's GL report); no payroll, no
links between payments and charges, price levels per item not carried
(<https://kb.accountedge.com/en/articles/14784871-iif-import-tool-technical-notes>).

**Portable company file.** Complete *financial* data, minus templates, logos,
images, attachments, `.TLG`, Loan Manager, Statement Writer and Fixed Asset
Manager (Intuit, above). Only QuickBooks can open it today.

**QuickBooks Online migration.** Intuit's own converter moves lists, balances
and transactions but not "payroll item breakdowns and employee year-to-date
numbers", bank feeds, custom templates, users, company settings or the
Payments account; large files (over 1.2 M targets in Canada, 500 k in the UK)
get lists-and-balances only; the window is 60 days after QBO sign-up
(<https://quickbooks.intuit.com/learn-support/en-ca/help-article/import-export-files/migrating-quickbooks-desktop-quickbooks-online/L47H2Uugp_CA_en_CA>).
Custom reports and reconciliation reports do not convert
(<https://www.dummies.com/article/an-overview-of-what-wont-import-into-quickbooks-online-282790>).
Useful only as Intuit's own admission of what a migration can skip; it
targets QBO, not us.

## Options

**(a) Stay with qbXML (Web Connector / SDK) as the import path; close its gaps.**
Pros: sanctioned, documented, already built and read-only by construction;
covers every posting transaction type (the 14 bridge gaps are qbXML-reachable);
deletions and incremental sync solved; zero licensing or §1201 exposure;
works for every QuickBooks year since 2002. Cons: needs a QuickBooks install
somewhere (a lapsed subscriber may not have one that opens); payroll detail
needs an active payroll subscription; audit trail, attachments metadata,
memorized items need side channels.

**(b) A one-time "export everything" run on the QuickBooks machine** — the SDK
transport in the desktop app or `librebooks import qbd --transport sdk`, plus
the side channels: copy the `Attach` folder; export the Audit Trail and
reconciliation reports to CSV from the UI and import them as read-only
history; IIF for memorized-transaction *lists* if wanted. Pros: everything in
(a), plus the long tail captured once, before the subscription lapses. Cons:
a few manual UI steps the user must be walked through; still needs a working
QuickBooks on the day.

**(c) Native `.QBW` parsing.** What it would take: a Rust `lb-qbw` crate built
on `opensqlany` (page store, CRC, de-obfuscation, catalog) plus a QuickBooks
schema layer: table discovery, row decoding (upstream still lacks money-field
offsets and multi-page value rules), the posting model (the fork needed five
"posting families" and a schema manifest *per QuickBooks build*), then
`.QBM` (zlib + SQL unload, probably easier) and `.QBB` (no spec at all). A
corpus of real files per QuickBooks year is needed to validate — which is
exactly the privacy hazard OpenQBW hit (public repos with real SSNs). Pros:
the only path for someone with a file and no QuickBooks; a strong story for
the project. Cons: months of reverse engineering per release; fail-closed per
schema version or risk silently wrong books; legal review before publishing
(EULA "reverse-engineer", §1201 trafficking); never ship SAP or Intuit code;
read-only forever (writing a `.QBW` is out of the question).

**(d) Hybrid: drive QuickBooks headlessly to produce exports.** The SDK's
unattended mode already does this for qbXML (the UI opens in the background;
this is the existing SDK transport). Pros: nothing new to build. Cons: it is
(a)/(b) by another name; a QuickBooks trial cannot be scripted via the SDK;
and UI automation for the few non-SDK reports is brittle and not worth
owning.

## Recommendation

Do **(a) + (b)** now, and run **(c) as a watch-and-contribute track**, not as
LibreBooks code yet.

Reasons: the import that exists already covers every ledger-affecting record
through sanctioned channels; the live Web Connector run (`plans/qbd-import.md`
step 12) is the cheapest way to prove parity to the cent. The genuine gaps
(attachments, audit trail, payroll detail, memorized items) are either on disk
already (`Attach`), exportable from the UI, or not worth a reverse-engineering
programme. Native parsing is *technically* feasible — the container is one SA
17.0.4 build for 2018–2024, keyless, with a readable catalog, and two
Apache-2.0 projects have proved line items and (for one build) a zero-cent
trial balance — but it is version-fragile, unvalidated beyond one or two
builds, and carries the only real legal exposure in the whole import story.
Contributing fixtures, bug reports or decoding work upstream to OpenQBW gets
LibreBooks the capability later without owning the risk now; if upstream
reaches reliable row decoding across the 2018–2024 schemas, add an optional
`lb-qbw` crate behind a feature flag, read-only, fail-closed on unknown
schemas, with a lawyer's read first.

Concretely, for the parent session:

1. Finish the live Web Connector sync and parity (`plans/qbd-import.md`).
2. Add the side channels to the import story: import the `Attach` folder
   (map folder → `TxnID` on the user's real file first); a CSV import for the
   UI-exported Audit Trail and reconciliation reports into read-only history.
3. Reword Phase 9 in `plans/librebooks.md`: drop "Ghidra is available" (that is
   decompiling Intuit's binaries, the one approach that is clearly
   "reverse-engineer the Software" under the EULA) in favour of the
   clean-room, on-disk approach and the upstream-contribution track. (Not
   edited here: this task touches only this file.)
4. If the user wants a cheap experiment: characterise one `.QBM` of their own
   company (decompress, look at the SQL unload) on `cad-3`, locally, never
   committed. That would tell us in an afternoon whether the portable file is
   the shortcut it looks like.

## Open questions for the user

1. **Appetite for legal review** before any native-format code is published
   under the LibreBooks name? My recommendation: yes, a short opinion on the
   EULA clause and §1201 trafficking, before (c) starts, not after.
2. **Contribute upstream or stay independent?** Recommendation: upstream to
   OpenQBW/OpenSQLAnywhere (Apache-2.0); forking the reverse engineering
   would split a tiny community.
3. **Phase 9 wording** — may the parent session drop the Ghidra line as above?
4. **The `.QBM` experiment** — do you want it, and on which machine? It needs
   a portable file made from your company (File › Create Copy), opened only by
   our tooling, never left on a shared disk.
5. **Gap-closer priority** — attachments first (concrete, on disk), or audit
   trail (CSV import), or neither until parity is proven?

## Things not to do

- Do not decompile `QBW32.EXE`, `QBDBMgrN.exe` or any SAP binary, in Ghidra or
  otherwise. It is the one path that is plainly "reverse-engineer the
  Software" under Intuit's EULA, and the clean-room on-disk route makes it
  unnecessary.
- Do not ship, bundle, download at runtime, or link SAP SQL Anywhere
  (`dbeng*`, `dbsrv*`, `dblib*`) or Intuit's `QBDBMgrN`; an MIT project cannot
  carry their licences, and SAP's OEM channel is closing.
- Do not try to log in to a running `QBDBMgrN` as a database client, even
  read-only: no documented credentials, DBA-level embedded SQL, and a real
  chance of corrupting the user's live file.
- Do not collect other people's `.QBW` files from public repositories as a
  corpus; OpenQBW found real names and SSNs in them. Synthetic fixtures or the
  user's own file, locally, only.
- Do not write to a `.QBW`, `.QBB` or `.QBM`, ever. Native support, if it
  comes, is read-only.
- Do not commit any real company data (`data/probe/` stays gitignored; the
  text extracted from Intuit's SDK release notes lives there too).

## Sources relied on most

- OpenQBW draft specification — <https://raw.githubusercontent.com/Sigilweaver/OpenQBW/main/SPECIFICATION.md>
- OpenQBW README — <https://github.com/Sigilweaver/OpenQBW>; landing page <https://sigilweaver.app/openqbw/>
- OpenSQLAnywhere — <https://github.com/Sigilweaver/OpenSQLAnywhere>; docs <https://sigilweaver.app/opensqlanywhere/docs/>
- Balaxxe/openqbw fork (Enterprise 24 R21 accounting path) — <https://github.com/Balaxxe/openqbw>
- SAP/Sybase `dbinit` (encryption modes) — <https://infocenter.sybase.com/help/topic/com.sybase.help.sqlanywhere.12.0.1/dbadmin/dbinit.html>
- SAP/Sybase `dbtran` (transaction log) — <https://infocenter.sybase.com/help/topic/com.sybase.help.sqlanywhere.12.0.1/dbadmin/dbtran.html>
- Exploit-DB 39804, QuickBooks embedded SQL / `.QBM` as SQL dump (2016) — <https://www.exploit-db.com/raw/39804>
- Intuit, file types and extensions — <https://quickbooks.intuit.com/learn-support/en-ca/getting-started/file-types-and-extensions-used-by-quickbooks-desktop/00/261705>
- Intuit, portable company file — <https://quickbooks.intuit.com/learn-support/en-us/back-up-data/create-or-restore-a-quickbooks-portable-company-file/00/203400>
- Intuit, "Fix Error in reload.sql" — <https://quickbooks.intuit.com/learn-support/en-us/help-article/data-utilities/fix-error-reload-sql/L4avVlq5T_US_en_US>
- Intuit, Auto Data Recovery — <https://quickbooks.intuit.com/learn-support/en-us/help-article/update-products/recover-lost-data-quickbooks-auto-data-recovery/L6vZ97286_US_en_US>
- Intuit, Desktop stop-sell — <https://quickbooks.intuit.com/r/whats-new/quickbooks-desktop-stop-sell/>
- Intuit, SDK 14.0 release notes (version table, payroll reports, QBXMLRP2) — <https://static.developer.intuit.com/resources/ReleaseNotes_QBXMLSDK_14_0.pdf>
- Intuit, SDK compatibility — <https://developer.intuit.com/app/developer/qbdesktop/docs/get-started/sdk-compatibility-with-quickbooks-releases>
- Intuit, QBD → QBO migration — <https://quickbooks.intuit.com/learn-support/en-ca/help-article/import-export-files/migrating-quickbooks-desktop-quickbooks-online/L47H2Uugp_CA_en_CA>
- Intuit EULAs — <https://www.intuit.com/legal/terms/en-us/quickbooks/desktop-payroll-2022>, <https://www.intuit.com/legal/terms/en-ca/quickbooks/desktop-2024>
- QODBC FAQ — <https://qodbc.com/?p=72>
- qbsync — <https://github.com/riccanet/qbsync>
- Qtools release table (2006 format switch) — <https://www.qtools.com/qbupdates.htm>
- QBES on Linux notes (`SYBSsa11`, `QBDBMgrN_24`) — <https://steakwiki.com/index.php/Quickbooks_Database_Server_Manager>
- Wikipedia, SQL Anywhere — <https://en.wikipedia.org/wiki/SQL_Anywhere>
- SAP forum: runtime engine / OEM — <https://sqlanywhere-forum.sap.com/questions/26713/runtime-engine>; OEM after 2025 — <https://sqlanywhere-forum.sap.com/questions/38344>; developer edition — <https://sqlanywhere-forum.sap.com/questions/11452/convert-developer-edition-to-chip-license>
- Passware QuickBooks Key — <https://www.passware.com/quickbooks/>
- hashcat forum, QBW encoding (2015) — <https://hashcat.net/forum/thread-4150.html>
- LWN, "Escape from QuickBooks (with data in hand)" (2017) — <https://lwn.net/Articles/729087/>
- PRONOM fmt/1354 (`.QBB` signature) — <https://www.nationalarchives.gov.uk/pronom/fmt/1354>
- file-recovery.com QBW signature (`MAUI`) — <https://www.file-recovery.com/qbw-signature-format.htm>
- DisCo on the 2017 §1201 report — <https://www.project-disco.org/?p=8314>; Samuelson, CACM 2015 — <https://cacm.acm.org/magazines/2015/7/188738-anti-circumvention-rules-limit-reverse-engineering/abstract>
