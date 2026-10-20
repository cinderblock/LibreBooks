//! Destinations: where backup objects are stored.
//!
//! A destination is a flat namespace of objects named with `/`-separated paths
//! (`acme/acme-20261019T023000.000Z.manifest.json`), the lowest common
//! denominator of a folder and a bucket. Writes are atomic: a local file appears
//! under its final name only once complete (temp name, fsync, rename); an S3 PUT is
//! atomic by nature.

use std::path::Path;

use crate::config::Destination;
use crate::secret::SecretResolver;
use crate::{BackupError, Result};

pub mod local;
pub mod s3;

pub use local::LocalStore;
pub use s3::S3Store;

/// An opened destination.
#[derive(Debug)]
pub enum Store {
    Local(LocalStore),
    S3(S3Store),
}

/// Manifests and error bodies are small; anything bigger is not what we wrote.
pub(crate) const SMALL_OBJECT_LIMIT: u64 = 4 << 20;

impl Store {
    /// Open `destination`, resolving its secrets now.
    ///
    /// # Errors
    /// [`BackupError::Secret`] or [`BackupError::Config`] when it cannot be used.
    pub fn open(destination: &Destination, secrets: &dyn SecretResolver) -> Result<Store> {
        Ok(match destination {
            Destination::Local { dir } => Store::Local(LocalStore::new(dir)),
            Destination::S3 { .. } => Store::S3(S3Store::new(destination, secrets)?),
        })
    }

    /// Store the file at `path` as `name`. `sha256` (hex) and `size` describe it.
    pub async fn put_file(&self, name: &str, path: &Path, sha256: &str, size: u64) -> Result<()> {
        check_name(name)?;
        match self {
            Store::Local(s) => s.put_file(name, path).await,
            Store::S3(s) => s.put_file(name, path, sha256, size).await,
        }
    }

    /// Store `bytes` as `name`.
    pub async fn put_bytes(&self, name: &str, bytes: Vec<u8>) -> Result<()> {
        check_name(name)?;
        match self {
            Store::Local(s) => s.put_bytes(name, bytes).await,
            Store::S3(s) => s.put_bytes(name, bytes).await,
        }
    }

    /// Fetch `name` into a new file at `path`.
    pub async fn get_file(&self, name: &str, path: &Path) -> Result<()> {
        check_name(name)?;
        match self {
            Store::Local(s) => s.get_file(name, path).await,
            Store::S3(s) => s.get_file(name, path).await,
        }
    }

    /// Fetch a small object (at most 4 MiB).
    pub async fn get_bytes(&self, name: &str) -> Result<Vec<u8>> {
        check_name(name)?;
        match self {
            Store::Local(s) => s.get_bytes(name).await,
            Store::S3(s) => s.get_bytes(name).await,
        }
    }

    /// Every object whose name starts with `prefix`, sorted.
    pub async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let mut names = match self {
            Store::Local(s) => s.list(prefix).await?,
            Store::S3(s) => s.list(prefix).await?,
        };
        names.sort();
        Ok(names)
    }

    /// Delete `name`. Deleting what is not there is not an error.
    pub async fn delete(&self, name: &str) -> Result<()> {
        check_name(name)?;
        match self {
            Store::Local(s) => s.delete(name).await,
            Store::S3(s) => s.delete(name).await,
        }
    }

    /// Write, read back, list and delete a small probe object: the "Test" button
    /// next to a destination in settings.
    pub async fn check(&self) -> Result<()> {
        let name = format!(
            ".librebooks-probe/{}",
            jiff::Timestamp::now().as_millisecond()
        );
        let body = b"LibreBooks can write here.".to_vec();
        self.put_bytes(&name, body.clone()).await?;
        let back = self.get_bytes(&name).await?;
        let listed = self.list(".librebooks-probe/").await?;
        self.delete(&name).await?;
        if back != body {
            return Err(BackupError::Config(
                "the destination returned something other than what was written".into(),
            ));
        }
        if !listed.contains(&name) {
            return Err(BackupError::Config(
                "the destination does not list what was written to it".into(),
            ));
        }
        Ok(())
    }
}

/// Object names are relative, `/`-separated, and never climb out of the root.
fn check_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name.contains('\0')
        || name
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..");
    if bad {
        Err(BackupError::Config(format!(
            "{name:?} is not a valid object name"
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_names_stay_inside_the_root() {
        for good in ["a", "acme/acme-1.manifest.json", ".librebooks-probe/1"] {
            assert!(check_name(good).is_ok(), "{good}");
        }
        for bad in ["", "/a", "a//b", "../a", "a/../b", "a\\b", "a/./b", "a/"] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }
}
