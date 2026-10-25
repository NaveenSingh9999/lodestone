//! Quilt provider over meta.quiltmc.org/v3. The installer entry carries
//! a sha1 hash inline in the JSON response — no sidecar fetch needed.

use serde::Deserialize;

use crate::error::CatalogError;
use crate::net::HttpClient;

use super::types::{Artifact, Checksum, HashAlgo, Provider, ServerType, VersionInfo};

pub const QUILT_META: &str = "https://meta.quiltmc.org/v3";

#[derive(Debug, Deserialize)]
struct GameEntry {
    version: String,
    stable: bool,
}

#[derive(Debug, Deserialize)]
struct InstallerEntry {
    version: String,
    url: String,
    file_size: u64,
    hashes: InstallerHashes,
}

#[derive(Debug, Deserialize)]
struct InstallerHashes {
    sha1: String,
}

pub struct QuiltProvider<H: HttpClient> {
    client: H,
}

impl<H: HttpClient> QuiltProvider<H> {
    pub fn new(client: H) -> Self {
        QuiltProvider { client }
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        what: &str,
    ) -> Result<T, CatalogError> {
        let body = self.client.get(url).await?;
        serde_json::from_slice(&body).map_err(|e| CatalogError::Source(format!("{what}: {e}")))
    }
}

impl<H: HttpClient> Provider for QuiltProvider<H> {
    async fn versions(&self) -> Result<Vec<VersionInfo>, CatalogError> {
        let games: Vec<GameEntry> = self
            .get_json(&format!("{QUILT_META}/versions/game"), "quilt game list")
            .await?;
        Ok(games
            .into_iter()
            .filter(|g| g.stable)
            .map(|g| VersionInfo {
                server_type: ServerType::Quilt.id().to_string(),
                id: g.version,
                release: true,
                java_major: None,
            })
            .collect())
    }

    async fn resolve(&self, version: &str) -> Result<Artifact, CatalogError> {
        let installers: Vec<InstallerEntry> = self
            .get_json(
                &format!("{QUILT_META}/versions/installer"),
                "quilt installers",
            )
            .await?;
        let installer = installers
            .iter()
            .find(|i| i.hashes.sha1.len() == 40)
            .or_else(|| installers.first())
            .ok_or_else(|| CatalogError::Source("quilt installer list empty".into()))?;
        Ok(Artifact {
            server_type: ServerType::Quilt,
            version: version.to_string(),
            file_name: format!("quilt-installer-{}.jar", installer.version),
            url: installer.url.clone(),
            checksum: Some(Checksum {
                algo: HashAlgo::Sha1,
                hex: installer.hashes.sha1.clone(),
            }),
            size_bytes: Some(installer.file_size),
            java_major: None,
        })
    }

    async fn install(
        &self,
        artifact: &Artifact,
        dir: &std::path::Path,
    ) -> Result<(), CatalogError> {
        let checksum = artifact
            .checksum
            .as_ref()
            .ok_or_else(|| CatalogError::Source("quilt installer has no checksum".to_string()))?;
        std::fs::create_dir_all(dir)
            .map_err(|e| CatalogError::Source(format!("mkdir {}: {e}", dir.display())))?;
        let dest = dir.join(&artifact.file_name);
        if dest.exists() && checksum.verify_file(&dest).await.is_ok() {
            return Ok(());
        }
        self.client.download(&artifact.url, &dest).await?;
        checksum.verify_file(&dest).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::NetError;
    use std::collections::HashMap;

    struct FxClient {
        map: HashMap<String, Vec<u8>>,
    }

    impl HttpClient for FxClient {
        async fn get(&self, url: &str) -> Result<Vec<u8>, NetError> {
            self.map
                .get(url)
                .cloned()
                .ok_or_else(|| NetError::Http(format!("fixture miss: {url}")))
        }
        async fn download(&self, url: &str, dest: &std::path::Path) -> Result<u64, NetError> {
            let bytes = self.get(url).await?;
            std::fs::write(dest, &bytes).unwrap();
            Ok(bytes.len() as u64)
        }
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "{}/tests/fixtures/quilt/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture present")
    }

    #[tokio::test]
    async fn resolve_returns_sha1_gated_installer() {
        let mut map = HashMap::new();
        map.insert(
            format!("{QUILT_META}/versions/installer"),
            fixture("installer-slim.json"),
        );
        let p = QuiltProvider {
            client: FxClient { map },
        };
        let art = p.resolve("1.21.11").await.unwrap();
        assert_eq!(art.server_type, ServerType::Quilt);
        assert_eq!(art.file_name, "quilt-installer-0.15.1.jar");
        let sum = art.checksum.as_ref().unwrap();
        assert_eq!(sum.algo, HashAlgo::Sha1);
        assert_eq!(&sum.hex[..16], "a6ea7a9e08e6f5ca");
    }

    #[tokio::test]
    async fn install_verifies_installer_sha1() {
        use sha1::{Digest, Sha1};
        let jar_bytes = b"quilt installer pretend bytes".to_vec();
        let mut h = Sha1::new();
        h.update(&jar_bytes);
        let jar_sha = hex::encode(h.finalize());
        let mut map = HashMap::new();
        map.insert(
            format!("{QUILT_META}/versions/installer"),
            fixture("installer-slim.json"),
        );
        let p = QuiltProvider {
            client: FxClient { map },
        };
        let art = p.resolve("1.21.11").await.unwrap();
        // overwrite checksum so download + verify both use the same value
        let mut art2 = art.clone();
        art2.checksum = Some(Checksum {
            algo: HashAlgo::Sha1,
            hex: jar_sha,
        });
        let dir = std::env::temp_dir().join(format!("lode-quilt-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let mut map2 = HashMap::new();
        map2.insert(
            "https://maven.quiltmc.org/repository/release/org/quiltmc/quilt-installer/0.15.1/quilt-installer-0.15.1.jar"
                .to_string(),
            jar_bytes,
        );
        let p2 = QuiltProvider {
            client: FxClient { map: map2 },
        };
        p2.install(&art2, &dir).await.unwrap();
        assert!(dir.join(&art.file_name).exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
