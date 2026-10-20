# Backups (`crates/lb-backup`)

> Parent plan: `plans/librebooks.md`, decision 11 and Phase 6. API routes:
> `plans/api.md` (`/backups/…`, `GET/PUT /server`).

## Goal

User requirement: **built-in backups**. Every company file is snapshotted on a
schedule while it stays open, verified, compressed, optionally encrypted, written
to one or more destinations (a local folder, any S3-compatible bucket), pruned by
a grandfather-father-son policy, and restorable from the UI or CLI. Plus a
portable company file (`.lbk`) for moving books between machines.

## Environment / context

- Crate `crates/lb-backup`; depends on `lb-store` (`Books::backup_to`,
  `Books::open`, `Books::check_integrity`).
- Pure-Rust TLS only (reqwest with rustls); no OpenSSL anywhere
  (`cargo tree -i openssl-sys` must find nothing).
- S3 request signing (SigV4) is implemented in-crate with `sha2` + `hmac`, tested
  against AWS's published vectors (aws-c-auth signing test suite and the S3
  documentation examples).

## Decisions already made (don't re-ask)

1. **A backup is two objects**: the artifact `<slug>/<slug>-<UTC stamp>.lbdb.zst[.age]`
   and its manifest `<slug>/<slug>-<UTC stamp>.manifest.json`, under the
   destination's folder or prefix. The artifact is written first and the manifest
   last, so a listed backup is always complete; pruning deletes the manifest first.
2. **Pipeline**: SQLite online backup to a work file next to the company file (same
   volume; `/tmp` is often a small tmpfs) → open the copy with `Books::open` and run
   `check_integrity` (a copy that fails is not stored, and nothing is pruned) →
   switch the copy to `journal_mode=DELETE` so it is one self-contained file → zstd
   (level 9 default) → optional age encryption → SHA-256 of both the database and
   the stored bytes in the manifest.
3. **Encryption** uses age: X25519 recipients (the server holds only public keys;
   restoring needs the identity) or a passphrase (scrypt). The passphrase is a
   secret reference, so the server can verify its own passphrase backups.
4. **Secrets are references, not values** in config: `env:NAME`, `file:/path`, or
   any scheme a caller-supplied `SecretResolver` understands (the server's secret
   store once it exists). Same idea in `lb-tunnel`; to be unified into one shared
   type when the server's settings store lands.
5. **Retention follows borg/restic semantics**: `keep_last`, then the newest backup
   in each of the last N days / ISO weeks / months / years *that have backups*.
   Time passing alone never deletes anything (if backups stop, history stays).
   Periods are computed in the configured time zone. An all-zero policy keeps
   everything. Backups dated after `now` are never touched. Default: last 10, 7
   daily, 4 weekly, 12 monthly, 7 yearly (accounting records are commonly kept
   seven years).
6. **Restore never overwrites**: it writes to a new path (atomic no-clobber
   rename) after checking the stored hash, decrypting, decompressing, checking the
   database hash, opening it and running the integrity check. The caller swaps
   files.
7. **One destination failing does not fail the backup**; the record lists each
   destination's outcome. All failing is an error.
8. **`.lbk`** is a zip (deflate, for portability with OS zip tools) holding
   `company.lbdb` and `manifest.json`. `attachments/` is reserved for when the store
   grows attachments.

## Public API (summary)

- `BackupConfig { schedule, destinations, encryption, retention, compression_level, time_zone, work_dir }`
- `backup_now(&Books, company_name, &BackupConfig, &dyn SecretResolver) -> BackupRecord`
- `list_backups(&Destination, &dyn SecretResolver, company_slug: Option<&str>)`
- `verify(&BackupEntry, &dyn SecretResolver, &Unlock) -> VerifyReport`
- `restore(&BackupEntry, dest_path, &dyn SecretResolver, &Unlock) -> Manifest`
- `plan_retention(&[Dated], &Retention, now: &Zoned) -> Vec<String>`
- `next_run(&Schedule, last, now)`; `Scheduler::start/status/run_now/update/shutdown`
- `export_lbk(&Books, company_name, path)`, `import_lbk(path, dest_path)`

## Plan / steps

1. ~~Implement crate, tests, clippy/fmt clean, commit.~~ Done.
2. ▶ Server wiring (owned by the lb-server thread): config in `PUT /server`, routes
   under `/backups/…`, `Scheduler` per open company.

## Findings / gotchas

- The mock S3 server in `tests/common` re-derives every request's SigV4
  signature from what it actually received, so a client that signs one encoding
  and sends another fails the tests the way it would fail against AWS. Keys with
  `$`, `+`, `&`, `=` and spaces round-trip.
- S3 refuses chunked uploads; a streamed file body needs an explicit
  `Content-Length`, which `S3Store` sets.

- `Books::backup_to` produces a WAL-flagged file (page 1 is copied verbatim). The
  pipeline flips the copy to `journal_mode=DELETE` before compressing so the
  artifact opens read-only in any SQLite tool without creating `-wal`/`-shm`.
- `Books::open` migrates. Restoring a backup from an older schema therefore
  upgrades the restored copy during verification; one from a newer LibreBooks fails
  with the store's "file is from a newer LibreBooks" error, which is right.

## Gaps / later

- S3 uploads are a single PUT (fine to 5 GB); multipart is not implemented.
- `.lbk` does not carry attachments yet (the store has none).
- No SFTP/WebDAV/Google Drive destinations yet.

## Progress log

- [x] lb-backup implemented and committed: config, secrets, manifest, SigV4
  (aws-c-auth vectors + S3 docs examples pass), local and S3 destinations,
  pack/unpack pipeline, backup/list/verify/restore, retention, schedule,
  Scheduler, `.lbk`. 57 tests (unit + local round trips + mock-S3 flow).
- [x] Server wiring: `/backups` routes, scheduler in housekeeping, CLI
  `backup` commands (merged `ef1d3e4`, `5427aba`).

## Things not to do

- Never prune after a failed backup or on a destination that just failed.
- Never write a restore over an existing file.
