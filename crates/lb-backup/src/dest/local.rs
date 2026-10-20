//! A folder as a destination: another disk, a NAS mount, a synced folder.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::dest::SMALL_OBJECT_LIMIT;
use crate::{BackupError, Result};

/// Objects are files under `root`; `/` in a name is a subfolder.
#[derive(Debug, Clone)]
pub struct LocalStore {
    root: PathBuf,
}

impl LocalStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        LocalStore { root: root.into() }
    }

    fn path(&self, name: &str) -> PathBuf {
        name.split('/')
            .fold(self.root.clone(), |p, seg| p.join(seg))
    }

    pub(crate) async fn put_file(&self, name: &str, src: &Path) -> Result<()> {
        let target = self.path(name);
        let src = src.to_owned();
        blocking(move || {
            write_atomically(&target, |out| {
                let mut f = std::fs::File::open(&src)?;
                std::io::copy(&mut f, out).map(|_| ())
            })
        })
        .await
    }

    pub(crate) async fn put_bytes(&self, name: &str, bytes: Vec<u8>) -> Result<()> {
        let target = self.path(name);
        blocking(move || write_atomically(&target, |out| out.write_all(&bytes))).await
    }

    pub(crate) async fn get_file(&self, name: &str, dest: &Path) -> Result<()> {
        let src = self.path(name);
        tokio::fs::copy(&src, dest).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BackupError::NotFound(src.display().to_string())
            } else {
                BackupError::Io {
                    what: format!("copy {}", src.display()),
                    source: e,
                }
            }
        })?;
        Ok(())
    }

    pub(crate) async fn get_bytes(&self, name: &str) -> Result<Vec<u8>> {
        let path = self.path(name);
        let meta = tokio::fs::metadata(&path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BackupError::NotFound(path.display().to_string())
            } else {
                BackupError::Io {
                    what: format!("read {}", path.display()),
                    source: e,
                }
            }
        })?;
        if meta.len() > SMALL_OBJECT_LIMIT {
            return Err(BackupError::Manifest(format!(
                "{} is unexpectedly large",
                path.display()
            )));
        }
        tokio::fs::read(&path)
            .await
            .map_err(BackupError::io(format!("read {}", path.display())))
    }

    pub(crate) async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let root = self.root.clone();
        let prefix = prefix.to_owned();
        blocking(move || {
            let mut out = Vec::new();
            walk(&root, "", &mut out)?;
            out.retain(|n| n.starts_with(&prefix));
            Ok(out)
        })
        .await
    }

    pub(crate) async fn delete(&self, name: &str) -> Result<()> {
        let path = self.path(name);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(BackupError::Io {
                what: format!("delete {}", path.display()),
                source: e,
            }),
        }
    }
}

/// Names of every file under `dir`, `/`-separated relative to the root. Our own
/// partial files are skipped.
fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && rel.is_empty() => return Ok(()),
        Err(e) => {
            return Err(BackupError::Io {
                what: format!("list {}", dir.display()),
                source: e,
            });
        }
    };
    for entry in entries {
        let entry = entry.map_err(BackupError::io(format!("list {}", dir.display())))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let rel_name = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let kind = entry
            .file_type()
            .map_err(BackupError::io(format!("list {}", dir.display())))?;
        if kind.is_dir() {
            walk(&entry.path(), &rel_name, out)?;
        } else if !name.ends_with(PARTIAL) {
            out.push(rel_name);
        }
    }
    Ok(())
}

const PARTIAL: &str = ".partial";

/// Write `target` through a temporary file beside it, flushed to disk before it
/// takes the final name, so a crash leaves either nothing or the whole file.
fn write_atomically(
    target: &Path,
    fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> Result<()> {
    let dir = target.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(BackupError::io(format!("create {}", dir.display())))?;
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = tempfile::Builder::new()
        .prefix(&format!(".{file_name}."))
        .suffix(PARTIAL)
        .tempfile_in(dir)
        .map_err(BackupError::io(format!(
            "create a file in {}",
            dir.display()
        )))?;
    let (mut file, tmp_path) = tmp.into_parts();
    let what = format!("write {}", target.display());
    fill(&mut file).map_err(BackupError::io(what.clone()))?;
    file.sync_all().map_err(BackupError::io(what.clone()))?;
    drop(file);
    tmp_path.persist(target).map_err(|e| BackupError::Io {
        what,
        source: e.error,
    })?;
    Ok(())
}

pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| BackupError::Task(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_folder_stores_lists_reads_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalStore::new(dir.path().join("not-yet"));
        assert!(
            store.list("").await.unwrap().is_empty(),
            "a missing root is empty"
        );
        store.put_bytes("a/one.json", b"1".to_vec()).await.unwrap();
        store.put_bytes("a/two.json", b"2".to_vec()).await.unwrap();
        store
            .put_bytes("b/three.json", b"3".to_vec())
            .await
            .unwrap();
        let mut names = store.list("a/").await.unwrap();
        names.sort();
        assert_eq!(names, ["a/one.json", "a/two.json"]);
        assert_eq!(store.get_bytes("a/two.json").await.unwrap(), b"2");
        store.delete("a/one.json").await.unwrap();
        store.delete("a/one.json").await.unwrap();
        assert_eq!(store.list("a/").await.unwrap(), ["a/two.json"]);
        assert!(matches!(
            store.get_bytes("a/one.json").await,
            Err(BackupError::NotFound(_))
        ));
        // Overwriting replaces atomically and leaves no partial files behind.
        store.put_bytes("a/two.json", b"22".to_vec()).await.unwrap();
        assert_eq!(store.get_bytes("a/two.json").await.unwrap(), b"22");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("not-yet/a"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(PARTIAL))
            .collect();
        assert!(leftovers.is_empty());
    }
}
