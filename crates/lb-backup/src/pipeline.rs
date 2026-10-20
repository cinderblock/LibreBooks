//! The blocking work: copy the company file, check the copy, and turn it into
//! stored bytes and back. Callers run these on a blocking thread.
//!
//! Packing is one streaming pass: database → SHA-256 → zstd → age → SHA-256 →
//! file, so neither the database nor the backup is ever held in memory.

use std::io::{self, BufReader, Read, Write};
use std::path::Path;

use age::secrecy::SecretString;
use lb_store::Books;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::manifest::EncryptionKind;
use crate::{BackupError, Result};

/// The encryption to apply, with secrets resolved.
pub(crate) enum Encrypt {
    None,
    Recipients(Vec<age::x25519::Recipient>),
    Passphrase {
        passphrase: SecretString,
        work_factor: Option<u8>,
    },
}

impl Encrypt {
    pub(crate) fn kind(&self) -> EncryptionKind {
        match self {
            Encrypt::None => EncryptionKind::None,
            Encrypt::Recipients(_) => EncryptionKind::X25519,
            Encrypt::Passphrase { .. } => EncryptionKind::Passphrase,
        }
    }
}

/// What decrypting needs, resolved.
pub(crate) enum Decrypt<'a> {
    None,
    Identities(&'a [age::x25519::Identity]),
    Passphrase(&'a SecretString),
}

/// Sizes and hashes of a packed backup.
#[derive(Debug, Clone)]
pub(crate) struct Packed {
    pub database_size: u64,
    pub database_sha256: String,
    pub stored_size: u64,
    pub stored_sha256: String,
}

/// Copy the open company into `dest` with SQLite's online backup, check the copy,
/// and leave it as one self-contained file. Returns the schema version.
pub(crate) fn snapshot(books: &Books, dest: &Path) -> Result<i64> {
    books.backup_to(dest)?;
    let schema = check_database(dest)?;
    // The copy carries the live file's WAL flag (page 1 is copied verbatim). As a
    // rollback-journal file it opens read-only in any SQLite tool without needing
    // -wal/-shm files beside it; LibreBooks switches it back to WAL when it opens it.
    let conn = Connection::open(dest)?;
    let _: String = conn.query_row("PRAGMA journal_mode = DELETE", [], |r| r.get(0))?;
    conn.close().map_err(|(_, e)| e)?;
    Ok(schema)
}

/// Open the file at `path` as a company and run the store's integrity checks.
/// Returns its schema version (`user_version`).
pub(crate) fn check_database(path: &Path) -> Result<i64> {
    let books = Books::open(path)?;
    let problems = books.check_integrity()?;
    let schema = books.read(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))?;
    drop(books);
    if problems.is_empty() {
        Ok(schema)
    } else {
        Err(BackupError::Integrity(problems))
    }
}

/// Compress (and encrypt) the database at `db` into a new file at `out`.
pub(crate) fn pack(db: &Path, out: &Path, level: i32, encrypt: &Encrypt) -> Result<Packed> {
    let input = std::fs::File::open(db).map_err(BackupError::io("read the copy"))?;
    let mut input = Hashing::new(BufReader::new(input));
    let file = std::fs::File::create(out).map_err(BackupError::io("create the backup file"))?;
    let sink = Hashing::new(io::BufWriter::new(file));
    let write_err = BackupError::io("write the backup file");

    let sink = match encrypt {
        Encrypt::None => {
            let mut z = zstd::Encoder::new(sink, level).map_err(BackupError::io("start zstd"))?;
            io::copy(&mut input, &mut z).map_err(BackupError::io("compress the copy"))?;
            z.finish().map_err(BackupError::io("compress the copy"))?
        }
        Encrypt::Recipients(_) | Encrypt::Passphrase { .. } => {
            let encryptor = encryptor(encrypt)?;
            let age = encryptor
                .wrap_output(sink)
                .map_err(|e| BackupError::Encrypt(e.to_string()))?;
            let mut z = zstd::Encoder::new(age, level).map_err(BackupError::io("start zstd"))?;
            io::copy(&mut input, &mut z).map_err(BackupError::io("compress the copy"))?;
            let age = z.finish().map_err(BackupError::io("compress the copy"))?;
            age.finish()
                .map_err(|e| BackupError::Encrypt(e.to_string()))?
        }
    };
    let (stored_size, stored_sha256, writer) = sink.finish();
    let file = writer
        .into_inner()
        .map_err(|e| BackupError::io("write the backup file")(e.into_error()))?;
    file.sync_all().map_err(write_err)?;
    let (database_size, database_sha256, _) = input.finish();
    Ok(Packed {
        database_size,
        database_sha256,
        stored_size,
        stored_sha256,
    })
}

fn encryptor(encrypt: &Encrypt) -> Result<age::Encryptor> {
    match encrypt {
        Encrypt::None => unreachable!("only called to encrypt"),
        Encrypt::Recipients(rs) => {
            age::Encryptor::with_recipients(rs.iter().map(|r| r as &dyn age::Recipient))
                .map_err(|e| BackupError::Encrypt(e.to_string()))
        }
        Encrypt::Passphrase {
            passphrase,
            work_factor,
        } => {
            let mut r = age::scrypt::Recipient::new(passphrase.clone());
            if let Some(n) = work_factor {
                r.set_work_factor(*n);
            }
            age::Encryptor::with_recipients(std::iter::once(&r as &dyn age::Recipient))
                .map_err(|e| BackupError::Encrypt(e.to_string()))
        }
    }
}

/// Decrypt (if needed) and decompress the stored bytes at `stored` into a new
/// database file at `out`. Returns the database's size and SHA-256.
pub(crate) fn unpack(
    stored: &Path,
    out: &Path,
    kind: EncryptionKind,
    decrypt: &Decrypt<'_>,
) -> Result<(u64, String)> {
    let file = std::fs::File::open(stored).map_err(BackupError::io("read the backup file"))?;
    let file = BufReader::new(file);
    let reader: Box<dyn Read> = match kind {
        EncryptionKind::None => Box::new(file),
        EncryptionKind::X25519 | EncryptionKind::Passphrase => {
            let d = age::Decryptor::new_buffered(file).map_err(decrypt_error)?;
            let r = match (decrypt, d.is_scrypt()) {
                (Decrypt::Passphrase(p), true) => {
                    let id = age::scrypt::Identity::new((*p).clone());
                    d.decrypt(std::iter::once(&id as &dyn age::Identity))
                }
                (Decrypt::Identities(ids), false) => {
                    d.decrypt(ids.iter().map(|i| i as &dyn age::Identity))
                }
                (_, true) => {
                    return Err(BackupError::Decrypt(
                        "it is encrypted with a passphrase; enter the passphrase".into(),
                    ));
                }
                (_, false) => {
                    return Err(BackupError::Decrypt(
                        "it is encrypted to a key; provide the matching age identity (AGE-SECRET-KEY-1…)".into(),
                    ));
                }
            }
            .map_err(decrypt_error)?;
            Box::new(r)
        }
    };
    let mut z = zstd::Decoder::new(reader).map_err(BackupError::io("start zstd"))?;
    let file = std::fs::File::create(out).map_err(BackupError::io("create the database file"))?;
    let mut sink = Hashing::new(io::BufWriter::new(file));
    io::copy(&mut z, &mut sink).map_err(|e| match e.kind() {
        // age reports a failed authentication tag as InvalidData.
        io::ErrorKind::InvalidData => BackupError::Checksum {
            what: "the backup's contents",
        },
        _ => BackupError::Io {
            what: "decompress the backup".into(),
            source: e,
        },
    })?;
    let (size, sha, writer) = sink.finish();
    let file = writer
        .into_inner()
        .map_err(|e| BackupError::io("write the database file")(e.into_error()))?;
    file.sync_all()
        .map_err(BackupError::io("write the database file"))?;
    Ok((size, sha))
}

fn decrypt_error(e: age::DecryptError) -> BackupError {
    match e {
        age::DecryptError::NoMatchingKeys => BackupError::Decrypt(
            "none of the given keys can open it (wrong identity or passphrase)".into(),
        ),
        age::DecryptError::DecryptionFailed => {
            BackupError::Decrypt("wrong passphrase, or the file is damaged".into())
        }
        other => BackupError::Decrypt(other.to_string()),
    }
}

/// Size and SHA-256 (hex) of a file.
pub(crate) fn hash_file(path: &Path) -> Result<(u64, String)> {
    let f =
        std::fs::File::open(path).map_err(BackupError::io(format!("read {}", path.display())))?;
    let mut h = Hashing::new(BufReader::new(f));
    io::copy(&mut h, &mut io::sink())
        .map_err(BackupError::io(format!("read {}", path.display())))?;
    let (size, sha, _) = h.finish();
    Ok((size, sha))
}

/// Counts and hashes what passes through, reading or writing.
pub(crate) struct Hashing<T> {
    inner: T,
    hasher: Sha256,
    count: u64,
}

impl<T> Hashing<T> {
    pub(crate) fn new(inner: T) -> Self {
        Hashing {
            inner,
            hasher: Sha256::new(),
            count: 0,
        }
    }

    pub(crate) fn finish(self) -> (u64, String, T) {
        (self.count, hex::encode(self.hasher.finalize()), self.inner)
    }
}

impl<R: Read> Read for Hashing<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.count += n as u64;
        Ok(n)
    }
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.count += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
