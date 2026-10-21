//! The portable company file, `.lbk`: a company packed for moving to another
//! machine or sending to an accountant.
//!
//! It is an ordinary zip (deflate, so the operating system's own zip tools open
//! it) holding:
//!
//! ```text
//! manifest.json   PortableManifest: who, when, schema, size and SHA-256
//! company.lbdb    the company file, as a self-contained SQLite database
//! attachments/    reserved for receipts and documents, once the store has them
//! ```
//!
//! Export takes the same verified snapshot a backup does. Import checks the size
//! and hash, opens the result and runs the integrity checks, and only then puts it
//! at the destination path, which it never overwrites.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use lb_store::Books;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use zip::write::SimpleFileOptions;

use crate::pipeline::{self, Hashing};
use crate::{APP_VERSION, BackupError, Result};

/// The `.lbk` format this build writes and the newest it reads.
pub const LBK_FORMAT: u32 = 1;
/// The manifest's name in the zip.
pub const LBK_MANIFEST: &str = "manifest.json";
/// The database's name in the zip.
pub const LBK_DATABASE: &str = "company.lbdb";
/// What `kind` says, so that a zip that only looks similar is refused.
const KIND: &str = "librebooks-company";

/// `manifest.json` in a `.lbk` (and `from` on `POST /companies/import`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortableManifest {
    pub kind: String,
    pub format: u32,
    pub app_version: String,
    pub company_name: String,
    #[ts(type = "number")]
    pub schema_version: i64,
    pub created_at: Timestamp,
    #[ts(type = "number")]
    pub database_size: u64,
    pub database_sha256: String,
}

/// Write `books` as a `.lbk` at `dest`, which must not exist. Blocking.
///
/// # Errors
/// [`BackupError::Exists`], [`BackupError::Integrity`] if the snapshot fails its
/// checks, or an I/O error.
pub fn export_lbk(books: &Books, company_name: &str, dest: &Path) -> Result<PortableManifest> {
    if dest.exists() {
        return Err(BackupError::Exists(dest.to_owned()));
    }
    let parent = parent_dir(dest);
    let work = tempfile::Builder::new()
        .prefix(".lb-export-")
        .tempdir_in(&parent)
        .map_err(BackupError::io(format!(
            "create a working folder in {}",
            parent.display()
        )))?;
    let db = work.path().join(LBK_DATABASE);
    let created_at = Timestamp::now();
    let schema_version = pipeline::snapshot(books, &db)?;
    let (database_size, database_sha256) = pipeline::hash_file(&db)?;
    let manifest = PortableManifest {
        kind: KIND.to_owned(),
        format: LBK_FORMAT,
        app_version: APP_VERSION.to_owned(),
        company_name: company_name.to_owned(),
        schema_version,
        created_at,
        database_size,
        database_sha256,
    };

    let out = tempfile::Builder::new()
        .prefix(".lb-export-")
        .suffix(".lbk")
        .tempfile_in(&parent)
        .map_err(BackupError::io(format!(
            "create a file in {}",
            parent.display()
        )))?;
    let (file, out_path) = out.into_parts();
    let write = |what: &str| BackupError::io(format!("write {what} into {}", dest.display()));
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(database_size >= u64::from(u32::MAX));
    zip.start_file(LBK_MANIFEST, options)
        .map_err(|e| zip_error(dest, &e))?;
    let manifest_json = serde_json::to_vec_pretty(&manifest).expect("a manifest always serializes");
    zip.write_all(&manifest_json)
        .map_err(write("the manifest"))?;
    zip.start_file(LBK_DATABASE, options)
        .map_err(|e| zip_error(dest, &e))?;
    let mut src = std::fs::File::open(&db).map_err(BackupError::io("read the snapshot"))?;
    std::io::copy(&mut src, &mut zip).map_err(write("the company file"))?;
    let buf = zip.finish().map_err(|e| zip_error(dest, &e))?;
    let file = buf
        .into_inner()
        .map_err(|e| write("the company file")(e.into_error()))?;
    file.sync_all().map_err(write("the company file"))?;
    drop(file);
    persist(out_path, dest)?;
    drop(work);
    Ok(manifest)
}

/// Unpack the `.lbk` at `lbk` into a company file at `dest`, which must not exist.
/// Blocking.
///
/// # Errors
/// [`BackupError::Manifest`] for something that is not a `.lbk`,
/// [`BackupError::TooNew`], [`BackupError::Checksum`], [`BackupError::Integrity`],
/// [`BackupError::Exists`].
pub fn import_lbk(lbk: &Path, dest: &Path) -> Result<PortableManifest> {
    if dest.exists() {
        return Err(BackupError::Exists(dest.to_owned()));
    }
    let file =
        std::fs::File::open(lbk).map_err(BackupError::io(format!("open {}", lbk.display())))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|_| {
        BackupError::Manifest(format!(
            "{} is not a LibreBooks company file",
            lbk.display()
        ))
    })?;

    let manifest: PortableManifest = {
        let entry = zip.by_name(LBK_MANIFEST).map_err(|_| {
            BackupError::Manifest(format!("{} has no {LBK_MANIFEST}", lbk.display()))
        })?;
        let mut bytes = Vec::new();
        entry
            .take(1 << 20)
            .read_to_end(&mut bytes)
            .map_err(BackupError::io("read the manifest"))?;
        #[derive(Deserialize)]
        struct Probe {
            kind: String,
            format: u32,
        }
        let probe: Probe =
            serde_json::from_slice(&bytes).map_err(|e| BackupError::Manifest(e.to_string()))?;
        if probe.kind != KIND {
            return Err(BackupError::Manifest(format!(
                "{} is not a LibreBooks company file",
                lbk.display()
            )));
        }
        if probe.format > LBK_FORMAT {
            return Err(BackupError::TooNew {
                found: probe.format,
                known: LBK_FORMAT,
            });
        }
        serde_json::from_slice(&bytes).map_err(|e| BackupError::Manifest(e.to_string()))?
    };

    let parent = parent_dir(dest);
    std::fs::create_dir_all(&parent)
        .map_err(BackupError::io(format!("create {}", parent.display())))?;
    let out = tempfile::Builder::new()
        .prefix(".lb-import-")
        .suffix(".lbdb")
        .tempfile_in(&parent)
        .map_err(BackupError::io(format!(
            "create a file in {}",
            parent.display()
        )))?;
    let (file, out_path) = out.into_parts();
    {
        let entry = zip.by_name(LBK_DATABASE).map_err(|_| {
            BackupError::Manifest(format!("{} has no {LBK_DATABASE}", lbk.display()))
        })?;
        // Never more than the manifest promised (plus one byte, to notice).
        let mut sink = Hashing::new(std::io::BufWriter::new(file));
        std::io::copy(&mut entry.take(manifest.database_size + 1), &mut sink)
            .map_err(BackupError::io("unpack the company file"))?;
        let (size, sha, writer) = sink.finish();
        let file = writer
            .into_inner()
            .map_err(|e| BackupError::io("write the company file")(e.into_error()))?;
        file.sync_all()
            .map_err(BackupError::io("write the company file"))?;
        if size != manifest.database_size || sha != manifest.database_sha256 {
            return Err(BackupError::Checksum {
                what: "the company file inside",
            });
        }
    }
    pipeline::check_database(&out_path)?;
    persist(out_path, dest)?;
    Ok(manifest)
}

fn persist(tmp: tempfile::TempPath, dest: &Path) -> Result<()> {
    tmp.persist_noclobber(dest).map_err(|e| {
        if e.error.kind() == std::io::ErrorKind::AlreadyExists {
            BackupError::Exists(dest.to_owned())
        } else {
            BackupError::Io {
                what: format!("move the file to {}", dest.display()),
                source: e.error,
            }
        }
    })
}

fn zip_error(dest: &Path, e: &zip::result::ZipError) -> BackupError {
    BackupError::Io {
        what: format!("write {}", dest.display()),
        source: std::io::Error::other(e.to_string()),
    }
}

fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_owned(),
        _ => PathBuf::from("."),
    }
}
