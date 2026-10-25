//! Forge provider over the Forge promotions API + maven shim.
//! The installer jar is verified against the sha1 sidecar served by maven.

use serde::Deserialize;

use crate::error::CatalogError;
use crate::net::HttpClient;

use super::types::{Artifact, Checksum, HashAlgo, Provider, ServerType, VersionInfo};

pub const PROMOS: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";

#[derive(Debug, Deserialize)]
struct Promos {
    promos: std::collections::HashMap<String, String>,
}

pub struct ForgeProvider<H: HttpClient> {
    client: H,
}

impl<H: HttpClient> ForgeProvider<H> {
    pub fn new(client: H) -> Self {
        ForgeProvider { client }
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
            .map_err(|e| CatalogError::Source(format!("forge sha1: {e}")))?;
        let hex = String::from_utf8_lossy(&body).trim().to_string();
        if hex.len() != 40 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CatalogError::Source(format!(
                "forge sha1 sidecar malformed: {hex:?}"
            )));
        }
        Ok(Checksum {
            algo: HashAlgo::Sha1,
            hex,
        })
    }

    fn mc_version_key(mc: &str) -> String {
        format!("{mc}-latest")
    }
}

impl<H: HttpClient> Provider for ForgeProvider<H> {
    async fn versions(&self) -> Result<Vec<VersionInfo>, CatalogError> {
        let data: Promos = self.get_json(PROMOS, "forge promotions").await?;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for key in data.promos.keys() {
            let mc = if let Some(stripped) = key.strip_suffix("-latest") {
                stripped.to_string()
            } else if let Some(stripped) = key.strip_suffix("-recommended") {
                stripped.to_string()
            } else {
                continue;
            };
            if seen.insert(mc.clone()) {
                let recommended = data.promos.contains_key(&format!("{mc}-recommended"));
                out.push(VersionInfo {
                    server_type: ServerType::Forge.id().to_string(),
                    id: mc,
                    release: recommended,
                    java_major: None,
                });
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    async fn resolve(&self, mc: &str) -> Result<Artifact, CatalogError> {
        let data: Promos = self.get_json(PROMOS, "forge promotions").await?;
        let key = Self::mc_version_key(mc);
        let build = data
            .promos
            .get(&key)
            .ok_or_else(|| CatalogError::VersionNotFound {
                server_type: ServerType::Forge.id().to_string(),
                version: mc.to_string(),
            })?
            .clone();
        let file_name = format!("forge-{mc}-{build}-installer.jar");
        let url = format!(
            "https://maven.minecraftforge.net/net/minecraftforge/forge/{mc}-{build}/forge-{mc}-{build}-installer.jar"
        );
        let checksum = self.sha1_sidecar(&url).await?;
        Ok(Artifact {
            server_type: ServerType::Forge,
            version: mc.to_string(),
            file_name,
            url,
            checksum: Some(checksum),
            size_bytes: None,
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
            .ok_or_else(|| CatalogError::Source("forge installer has no checksum".to_string()))?;
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
            "{}/tests/fixtures/forge/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture present")
    }

    #[tokio::test]
    async fn resolve_picks_latest_promo_build() {
        let mut map = HashMap::new();
        map.insert(PROMOS.to_string(), fixture("promotions-slim.json"));
        let sha_fixture = std::fs::read(format!(
            "{}/tests/fixtures/forge/installer-sha1.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        map.insert(
            "https://maven.minecraftforge.net/net/minecraftforge/forge/1.21.4-54.1.22/forge-1.21.4-54.1.22-installer.jar.sha1".to_string(),
            sha_fixture.clone(),
        );
        map.insert(
            "https://maven.minecraftforge.net/net/minecraftforge/forge/1.21.4-51.0.33/forge-1.21.4-51.0.33-installer.jar.sha1".to_string(),
            sha_fixture,
        );
        let p = ForgeProvider {
            client: FxClient { map },
        };
        let art = p.resolve("1.21.4").await.unwrap();
        assert_eq!(art.server_type, ServerType::Forge);
        assert_eq!(art.file_name, "forge-1.21.4-54.1.22-installer.jar");
        let sum = art.checksum.as_ref().unwrap();
        assert_eq!(sum.algo, HashAlgo::Sha1);
        let sha = std::fs::read(format!(
            "{}/tests/fixtures/forge/installer-sha1.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let want = String::from_utf8_lossy(&sha).trim().to_string();
        assert_eq!(&sum.hex, &want[..40]);
    }
}
