use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

use crate::error::StorageError;

pub trait Storage: Send + Sync {
    fn read_file(
        &self,
        path: &str,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, StorageError>> + Send;
    fn write_file(
        &self,
        path: &str,
        data: &[u8],
    ) -> impl std::future::Future<Output = Result<(), StorageError>> + Send;
    fn list_dir(
        &self,
        dir: &str,
    ) -> impl std::future::Future<Output = Result<Vec<String>, StorageError>> + Send;
    fn remove(
        &self,
        path: &str,
    ) -> impl std::future::Future<Output = Result<(), StorageError>> + Send;
}

pub fn canonicalize(rel: &str) -> Result<String, StorageError> {
    let mut parts: Vec<&str> = Vec::new();
    for seg in rel.split('/') {
        match seg {
            "" | "." => continue,
            ".." => {
                if parts.pop().is_none() {
                    return Err(StorageError::PathEscape(rel.to_string()));
                }
            }
            s => parts.push(s),
        }
    }
    Ok(parts.join("/"))
}

pub struct FsStorage {
    root: PathBuf,
}

impl FsStorage {
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(FsStorage { root })
    }

    fn resolve(&self, rel: &str) -> Result<PathBuf, StorageError> {
        let canonical = canonicalize(rel)?;
        let full = self.root.join(&canonical);
        if !full.starts_with(&self.root) {
            return Err(StorageError::PathEscape(rel.to_string()));
        }
        Ok(full)
    }

    async fn atomic_write(&self, target: &Path, data: &[u8]) -> Result<(), StorageError> {
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = target.with_extension(format!("tmp-{nanos}-{}", std::process::id()));
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(data).await?;
        file.flush().await?;
        file.sync_all().await?;
        tokio::fs::rename(&tmp, target).await?;
        Ok(())
    }
}

impl Storage for FsStorage {
    async fn read_file(&self, path: &str) -> Result<Vec<u8>, StorageError> {
        let full = self.resolve(path)?;
        tokio::fs::read(&full).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StorageError::NotFound(full.display().to_string()),
            _ => StorageError::Io(e),
        })
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        let full = self.resolve(path)?;
        self.atomic_write(&full, data).await
    }

    async fn list_dir(&self, dir: &str) -> Result<Vec<String>, StorageError> {
        let full = self.resolve(dir)?;
        let canonical_parent = canonicalize(dir)?;
        let mut entries = tokio::fs::read_dir(&full)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => StorageError::NotFound(full.display().to_string()),
                _ => StorageError::Io(e),
            })?;
        let mut out = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(StorageError::Io)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            out.push(if canonical_parent.is_empty() {
                name
            } else {
                format!("{canonical_parent}/{name}")
            });
        }
        out.sort();
        Ok(out)
    }

    async fn remove(&self, path: &str) -> Result<(), StorageError> {
        let full = self.resolve(path)?;
        match tokio::fs::symlink_metadata(&full).await {
            Ok(meta) => {
                if meta.is_dir() {
                    tokio::fs::remove_dir_all(&full).await?;
                } else {
                    tokio::fs::remove_file(&full).await?;
                }
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(StorageError::NotFound(full.display().to_string()))
            }
            Err(e) => Err(StorageError::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("lode-fs-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[tokio::test]
    async fn roundtrip_list_remove() {
        let root = temp_root("roundtrip");
        let s = FsStorage::open(&root).unwrap();
        s.write_file("instances/a/config.toml", b"alpha")
            .await
            .unwrap();
        s.write_file("instances/b/config.toml", b"beta")
            .await
            .unwrap();
        s.write_file("runtimes/jdk21/libjli.so", b"elf")
            .await
            .unwrap();

        assert_eq!(
            s.read_file("instances/a/config.toml").await.unwrap(),
            b"alpha".to_vec()
        );

        let top = s.list_dir("").await.unwrap();
        assert_eq!(top, vec!["instances", "runtimes"]);

        let inst = s.list_dir("instances").await.unwrap();
        assert_eq!(inst, vec!["instances/a", "instances/b"]);

        let files_in_a = s.list_dir("instances/a").await.unwrap();
        assert_eq!(files_in_a, vec!["instances/a/config.toml"]);

        s.remove("instances/b").await.unwrap();
        assert!(matches!(
            s.read_file("instances/b/config.toml").await,
            Err(StorageError::NotFound(_))
        ));

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn write_is_atomic_no_tmp_leftover() {
        let root = temp_root("atomic");
        let s = FsStorage::open(&root).unwrap();
        s.write_file("deep/nested/file.bin", b"payload")
            .await
            .unwrap();
        let parent = s.list_dir("deep/nested").await.unwrap();
        assert_eq!(parent, vec!["deep/nested/file.bin"]);
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn blocks_escape() {
        let root = temp_root("escape");
        let s = FsStorage::open(&root).unwrap();
        s.write_file("a/b.txt", b"x").await.unwrap();
        assert!(matches!(
            s.read_file("a/../..//b.txt").await,
            Err(StorageError::PathEscape(_))
        ));
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
