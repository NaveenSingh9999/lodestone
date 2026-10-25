//! Fabric provider over the FabricMC Meta v2 API. The meta server-jar
//! bundle carries no published checksum, so the verifiable path is the
//! fabric-installer artifact from maven.fabricmc.net (sha1 sidecar):
//! download + verify the installer, then the engine runs it headless
//! (`java -jar fabric-installer server -dir <instance> -downloadMinecraft`).

use serde::Deserialize;

use crate::error::CatalogError;
use crate::net::HttpClient;

use super::types::{Artifact, Checksum, HashAlgo, Provider, ServerType, VersionInfo};

pub const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
pub const GAME_ENDPOINT: &str = "https://meta.fabricmc.net/v2/versions/game";
pub const INSTALLER_ENDPOINT: &str = "https://meta.fabricmc.net/v2/versions/installer";

#[derive(Debug, Deserialize)]
struct GameEntry {
    version: String,
    stable: bool,
}

#[derive(Debug, Deserialize)]
struct LoaderEntry {
    #[serde(rename = "launcherMeta")]
    launcher_meta: LauncherMeta,
}

#[derive(Debug, Deserialize)]
struct LauncherMeta {
    min_java_version: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct InstallerEntry {
    version: String,
    url: String,
    stable: bool,
}

pub struct FabricProvider<H: HttpClient> {
    client: H,
    game: String,
}

impl<H: HttpClient> FabricProvider<H> {
    pub fn new(client: H, game: String) -> Self {
        FabricProvider { client, game }
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        what: &str,
    ) -> Result<T, CatalogError> {
        let body = self.client.get(url).await?;
        serde_json::from_slice(&body).map_err(|e| CatalogError::Source(format!("{what}: {e}")))
    }

    async fn sha1_sidecar(&self, url: &str) -> Result<Checksum, CatalogError> {
        let body = self
            .client
            .get(&format!("{url}.sha1"))
            .await
            .map_err(|e| CatalogError::Source(format!("installer sha1: {e}")))?;
        let hex = String::from_utf8_lossy(&body).trim().to_string();
        if hex.len() != 40 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CatalogError::Source(format!(
                "installer sha1 sidecar malformed: {hex:?}"
            )));
        }
        Ok(Checksum {
            algo: HashAlgo::Sha1,
            hex,
        })
    }
}

impl<H: HttpClient> Provider for FabricProvider<H> {
    async fn versions(&self) -> Result<Vec<VersionInfo>, CatalogError> {
        let games: Vec<GameEntry> = self.get_json(GAME_ENDPOINT, "fabric game list").await?;
        Ok(games
            .into_iter()
            .filter(|g| g.stable)
            .map(|g| VersionInfo {
                server_type: ServerType::Fabric.id().to_string(),
                id: g.version,
                release: true,
                java_major: None,
            })
            .collect())
    }

    async fn resolve(&self, version: &str) -> Result<Artifact, CatalogError> {
        let installers: Vec<InstallerEntry> = self
            .get_json(INSTALLER_ENDPOINT, "fabric installers")
            .await?;
        let pick = |list: &[InstallerEntry]| -> Result<InstallerEntry, CatalogError> {
            list.iter()
                .find(|i| i.stable)
                .or_else(|| list.first())
                .ok_or_else(|| CatalogError::Source("fabric installer list empty".into()))
                .map(|i| InstallerEntry {
                    version: i.version.clone(),
                    url: i.url.clone(),
                    stable: i.stable,
                })
        };
        let installer = pick(&installers)?;
        let checksum = self.sha1_sidecar(&installer.url).await?;
        let loaders: Vec<LoaderEntry> = self
            .get_json(
                &format!("{FABRIC_META}/versions/loader/{}", self.game),
                "fabric loader profile",
            )
            .await?;
        let java_major = loaders
            .first()
            .and_then(|l| l.launcher_meta.min_java_version);
        Ok(Artifact {
            server_type: ServerType::Fabric,
            version: version.to_string(),
            file_name: format!("fabric-installer-{}.jar", installer.version),
            url: installer.url.clone(),
            checksum: Some(checksum),
            size_bytes: None,
            java_major,
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
            .ok_or_else(|| CatalogError::Source("fabric installer has no checksum".to_string()))?;
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
            "{}/tests/fixtures/fabric/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture present")
    }

    fn fx() -> FabricProvider<FxClient> {
        let mut map = HashMap::new();
        map.insert(GAME_ENDPOINT.to_string(), fixture("game-slim.json"));
        map.insert(INSTALLER_ENDPOINT.into(), fixture("installer-slim.json"));
        map.insert(
            format!("{FABRIC_META}/versions/loader/1.21.11"),
            fixture("loader-1.21.11-slim.json"),
        );
        // sidecar for the first stable installer (1.1.2)
        map.insert(
            "https://maven.fabricmc.net/net/fabricmc/fabric-installer/1.1.2/fabric-installer-1.1.2.jar.sha1".to_string(),
            b"74685b5a0178c4dabd4be741a276d35c59b5ec96".to_vec(),
        );
        FabricProvider {
            client: FxClient { map },
            game: "1.21.11".into(),
        }
    }

    #[tokio::test]
    async fn versions_lists_stable_games() {
        let p = FabricProvider {
            client: FxClient {
                map: HashMap::from([(GAME_ENDPOINT.to_string(), fixture("game-slim.json"))]),
            },
            game: "1.21.11".into(),
        };
        let versions = p.versions().await.unwrap();
        assert!(versions.iter().all(|v| v.release));
        assert!(versions.iter().any(|v| v.id == "1.21.11"));
    }

    #[tokio::test]
    async fn resolve_returns_sha1_gated_installer() {
        let p = fx();
        let art = p.resolve("1.21.11").await.unwrap();
        assert_eq!(art.server_type, ServerType::Fabric);
        assert_eq!(art.version, "1.21.11");
        assert_eq!(art.file_name, "fabric-installer-1.1.2.jar");
        assert!(art.url.ends_with("fabric-installer-1.1.2.jar"));
        let sum = art.checksum.as_ref().unwrap();
        assert_eq!(sum.algo, HashAlgo::Sha1);
        assert_eq!(sum.hex, "74685b5a0178c4dabd4be741a276d35c59b5ec96");
        assert_eq!(art.java_major, Some(8));
    }

    #[tokio::test]
    async fn install_verifies_installer_sha1() {
        use sha1::{Digest, Sha1};
        let dir = std::env::temp_dir().join(format!("lode-fab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // jar bytes whose sha1 matches the (rewritten) sidecar
        let jar_bytes = b"fabric installer pretend bytes".to_vec();
        let mut h = Sha1::new();
        h.update(&jar_bytes);
        let sha = hex::encode(h.finalize());
        let mut map = HashMap::new();
        map.insert(GAME_ENDPOINT.to_string(), fixture("game-slim.json"));
        map.insert(INSTALLER_ENDPOINT.into(), fixture("installer-slim.json"));
        map.insert(
            format!("{FABRIC_META}/versions/loader/1.21.11"),
            fixture("loader-1.21.11-slim.json"),
        );
        map.insert(
            "https://maven.fabricmc.net/net/fabricmc/fabric-installer/1.1.2/fabric-installer-1.1.2.jar".to_string(),
            jar_bytes,
        );
        map.insert(
            "https://maven.fabricmc.net/net/fabricmc/fabric-installer/1.1.2/fabric-installer-1.1.2.jar.sha1".to_string(),
            sha.into_bytes(),
        );
        let p2 = FabricProvider {
            client: FxClient { map },
            game: "1.21.11".into(),
        };
        let art = p2.resolve("1.21.11").await.unwrap();
        p2.install(&art, &dir).await.unwrap();
        let dest = dir.join(&art.file_name);
        assert!(dest.exists());
        art.checksum.unwrap().verify_file(&dest).await.unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
