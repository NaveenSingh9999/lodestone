//! Backup engine (spec: create_backup / list_backups / restore_backup /
//! export_backup). Archives are real `tar.gz` files produced with the `tar`
//! and `flate2` crates; every backup carries a SHA-256 of its bytes in a
//! sidecar `.sha256` file, and `verify` re-hashes the archive to detect
//! corruption. Restore is refused if the archive fails verification.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct BackupInfo {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
}

pub struct BackupEngine {
    /// Root under which backups are stored: `<data>/backups/<instance>/`.
    pub backups_root: PathBuf,
}

impl BackupEngine {
    pub fn new(backups_root: impl Into<PathBuf>) -> Self {
        BackupEngine {
            backups_root: backups_root.into(),
        }
    }

    fn instance_dir(&self, instance_id: &str) -> PathBuf {
        self.backups_root.join(instance_id)
    }

    /// Create a `.tar.gz` archive of `source_dir` and store it under the
    /// instance's backup directory. Returns the backup info including the
    /// computed SHA-256.
    pub fn create(
        &self,
        instance_id: &str,
        source_dir: &Path,
        label: &str,
    ) -> Result<BackupInfo, String> {
        if !source_dir.is_dir() {
            return Err(format!(
                "source dir does not exist: {}",
                source_dir.display()
            ));
        }
        let dir = self.instance_dir(instance_id);
        fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let safe_label: String = label
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let name = format!("{stamp}-{safe_label}.tar.gz");
        let archive_path = dir.join(&name);

        // Build the archive in memory first so a failure never leaves a
        // partial file behind.
        let mut buf: Vec<u8> = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut tar = tar::Builder::new(gz);
            tar.append_dir_all(".", source_dir)
                .map_err(|e| format!("tar append: {e}"))?;
            tar.finish().map_err(|e| format!("tar finish: {e}"))?;
            // drop encoder to flush
            let gz = tar
                .into_inner()
                .map_err(|e| format!("tar into_inner: {e}"))?;
            gz.finish().map_err(|e| format!("gz finish: {e}"))?;
        }

        let mut f = fs::File::create(&archive_path).map_err(|e| format!("create archive: {e}"))?;
        f.write_all(&buf)
            .map_err(|e| format!("write archive: {e}"))?;
        f.flush().map_err(|e| format!("flush: {e}"))?;
        f.sync_all().map_err(|e| format!("sync: {e}"))?;

        let sha = sha256_bytes(&buf);
        let sidecar = sidecar_path(&archive_path);
        fs::write(&sidecar, format!("{sha}  {name}\n"))
            .map_err(|e| format!("write sidecar: {e}"))?;

        let size = buf.len() as u64;
        Ok(BackupInfo {
            name,
            path: archive_path,
            size,
            sha256: sha,
        })
    }

    /// List backups for an instance, newest first.
    pub fn list(&self, instance_id: &str) -> Result<Vec<BackupInfo>, String> {
        let dir = self.instance_dir(instance_id);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| format!("read_dir: {e}"))? {
            let entry = entry.map_err(|e| format!("entry: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("gz") {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let sha = fs::read_to_string(sidecar_path(&path))
                .ok()
                .and_then(|t| t.split_whitespace().next().map(String::from))
                .unwrap_or_default();
            out.push(BackupInfo {
                name,
                path,
                size,
                sha256: sha,
            });
        }
        out.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(out)
    }

    /// Verify an archive against its sidecar (or a supplied hash).
    pub fn verify(&self, archive: &Path, expected_sha: Option<&str>) -> Result<(), String> {
        let bytes = fs::read(archive).map_err(|e| format!("read archive: {e}"))?;
        let got = sha256_bytes(&bytes);
        let expected = match expected_sha {
            Some(h) => h.to_string(),
            None => {
                let sidecar = sidecar_path(archive);
                let text =
                    fs::read_to_string(&sidecar).map_err(|e| format!("read sidecar: {e}"))?;
                text.split_whitespace()
                    .next()
                    .ok_or_else(|| "empty sidecar".to_string())?
                    .to_string()
            }
        };
        if !got.eq_ignore_ascii_case(&expected) {
            return Err(format!("backup corrupt: expected {expected}, got {got}"));
        }
        Ok(())
    }

    /// Restore an archive into `dest_dir`. The archive is verified first; on
    /// failure nothing is written. Existing files are overwritten.
    pub fn restore(
        &self,
        archive: &Path,
        dest_dir: &Path,
        expected_sha: Option<&str>,
    ) -> Result<u64, String> {
        self.verify(archive, expected_sha)?;
        fs::create_dir_all(dest_dir).map_err(|e| format!("mkdir: {e}"))?;
        let bytes = fs::read(archive).map_err(|e| format!("read: {e}"))?;
        let gz = flate2::read::GzDecoder::new(bytes.as_slice());
        let mut archive = tar::Archive::new(gz);
        let mut count = 0u64;
        for entry in archive.entries().map_err(|e| format!("entries: {e}"))? {
            let mut entry = entry.map_err(|e| format!("entry: {e}"))?;
            entry
                .unpack_in(dest_dir)
                .map_err(|e| format!("unpack: {e}"))?;
            count += 1;
        }
        Ok(count)
    }

    /// Export (copy) a backup to `target` path, verifying first.
    pub fn export(
        &self,
        archive: &Path,
        target: &Path,
        expected_sha: Option<&str>,
    ) -> Result<(), String> {
        self.verify(archive, expected_sha)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        fs::copy(archive, target).map_err(|e| format!("copy: {e}"))?;
        Ok(())
    }

    /// Delete a backup (archive + sidecar).
    pub fn delete(&self, archive: &Path) -> Result<(), String> {
        let sidecar = sidecar_path(archive);
        let _ = fs::remove_file(&sidecar);
        fs::remove_file(archive).map_err(|e| format!("remove: {e}"))
    }
}

fn sidecar_path(archive: &Path) -> PathBuf {
    let mut p = archive.as_os_str().to_os_string();
    p.push(".sha256");
    PathBuf::from(p)
}

fn sha256_bytes(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_root(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("lode-bak-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn make_source(root: &Path) {
        fs::create_dir_all(root.join("world")).unwrap();
        fs::write(root.join("world/level.dat"), b"level data").unwrap();
        fs::write(root.join("server.properties"), b"motd=hi\n").unwrap();
        let mut f = fs::File::create(root.join("big.bin")).unwrap();
        f.write_all(&vec![7u8; 100_000]).unwrap();
    }

    #[test]
    fn create_verify_list_restore() {
        let root = tmp_root("flow");
        let src = root.join("instance");
        make_source(&src);
        let engine = BackupEngine::new(root.join("backups"));
        let info = engine.create("inst-1", &src, "manual").unwrap();
        assert!(info.path.exists());
        assert_eq!(info.size, fs::metadata(&info.path).unwrap().len());

        // verify passes
        engine.verify(&info.path, None).unwrap();

        // list shows it
        let list = engine.list("inst-1").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].sha256, info.sha256);

        // restore into a fresh dir reproduces the content
        let dest = root.join("restored");
        let count = engine.restore(&info.path, &dest, None).unwrap();
        assert!(count > 0);
        assert_eq!(
            fs::read_to_string(dest.join("server.properties")).unwrap(),
            "motd=hi\n"
        );
        assert_eq!(
            fs::read(dest.join("world/level.dat")).unwrap(),
            b"level data"
        );
        assert_eq!(fs::metadata(dest.join("big.bin")).unwrap().len(), 100_000);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_backup_is_rejected() {
        let root = tmp_root("corrupt");
        let src = root.join("instance");
        make_source(&src);
        let engine = BackupEngine::new(root.join("backups"));
        let info = engine.create("inst-1", &src, "manual").unwrap();

        // tamper
        let mut bytes = fs::read(&info.path).unwrap();
        bytes[100] ^= 0xFF;
        fs::write(&info.path, &bytes).unwrap();

        let err = engine.verify(&info.path, None).unwrap_err();
        assert!(err.contains("corrupt"), "{err}");

        // restore refused
        let dest = root.join("restored");
        assert!(engine.restore(&info.path, &dest, None).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn export_copies_verified_archive() {
        let root = tmp_root("export");
        let src = root.join("instance");
        make_source(&src);
        let engine = BackupEngine::new(root.join("backups"));
        let info = engine.create("inst-1", &src, "manual").unwrap();
        let target = root.join("exported.tar.gz");
        engine
            .export(&info.path, &target, Some(&info.sha256))
            .unwrap();
        assert!(target.exists());
        engine.verify(&target, Some(&info.sha256)).unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn delete_removes_sidecar() {
        let root = tmp_root("delete");
        let src = root.join("instance");
        make_source(&src);
        let engine = BackupEngine::new(root.join("backups"));
        let info = engine.create("inst-1", &src, "manual").unwrap();
        let sidecar = sidecar_path(&info.path);
        assert!(sidecar.exists());
        engine.delete(&info.path).unwrap();
        assert!(!info.path.exists());
        assert!(!sidecar.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_source_is_error() {
        let root = tmp_root("missing");
        let engine = BackupEngine::new(root.join("backups"));
        assert!(engine.create("inst-1", &root.join("nope"), "x").is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
