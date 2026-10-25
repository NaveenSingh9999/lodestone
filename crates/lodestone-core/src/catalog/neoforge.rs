//! NeoForge provider. Installer jars are verified against the sha1 sidecar
//! from maven.neoforged.net.

use serde::Deserialize;

use crate::error::CatalogError;
use crate::net::HttpClient;

use super::types::{Artifact, Checksum, HashAlgo, Provider, ServerType, VersionInfo};

pub const VERSIONS_URL: &str =
    "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";

#[derive(Debug, Deserialize)]
struct VersionList {
    #[serde(default)]
    versions: Vec<String>,
}

pub struct NeoForgeProvider<H: HttpClient> {
    client: H,
}

impl<H: HttpClient> NeoForgeProvider<H> {
    pub fn new(client: H) -> Self {
        NeoForgeProvider { client }
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
            .map_err(|e| CatalogError::Source(format!("neoforge sha1: {e}")))?;
        let hex = String::from_utf8_lossy(&body).trim().to_string();
        if hex.len() != 40 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CatalogError::Source(format!(
                "neoforge sha1 sidecar malformed: {hex:?}"
            )));
        }
        Ok(Checksum {
            algo: HashAlgo::Sha1,
            hex,
        })
    }

    fn game_version(neo: &str) -> Option<String> {
        let comps: Vec<&str> = neo.split('.').collect();
        if comps.len() < 2 {
            return None;
        }
        Some(format!("1.{}.{}", comps[0], comps[1]))
    }
}

impl<H: HttpClient> Provider for NeoForgeProvider<H> {
    async fn versions(&self) -> Result<Vec<VersionInfo>, CatalogError> {
        let data: VersionList = self.get_json(VERSIONS_URL, "neoforge version list").await?;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for v in data.versions.iter().filter(|v| !v.contains("beta")) {
            if let Some(mc) = Self::game_version(v)
                && seen.insert(mc.clone())
            {
                out.push(VersionInfo {
                    server_type: ServerType::NeoForge.id().to_string(),
                    id: mc,
                    release: true,
                    java_major: None,
                });
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    async fn resolve(&self, mc: &str) -> Result<Artifact, CatalogError> {
        let data: VersionList = self.get_json(VERSIONS_URL, "neoforge version list").await?;
        let mc_prefix = mc
            .strip_prefix("1.")
            .ok_or_else(|| CatalogError::VersionNotFound {
                server_type: ServerType::NeoForge.id().to_string(),
                version: mc.to_string(),
            })?;
        let mc_parts: Vec<&str> = mc_prefix.split('.').collect();
        let mc_maj = mc_parts
            .first()
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(0);
        let mc_min = mc_parts
            .get(1)
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(0);
        let mut best: Option<String> = None;
        for v in data.versions.iter().filter(|v| !v.contains("beta")) {
            let comps: Vec<u32> = v.split('.').map(|p| p.parse().unwrap_or(0)).collect();
            if comps.len() < 2 {
                continue;
            }
            if comps[0] != mc_maj || comps[1] != mc_min {
                continue;
            }
            let cmp = match best.as_ref() {
                None => true,
                Some(b) => {
                    let bc: Vec<u32> = b.split('.').map(|p| p.parse().unwrap_or(0)).collect();
                    let bv_maj = bc.first().copied().unwrap_or(0);
                    let bv_min = bc.get(1).copied().unwrap_or(0);
                    let bv_patch = bc.get(2).copied().unwrap_or(0);
                    let pv_patch = comps.get(2).copied().unwrap_or(0);
                    (comps[0], comps[1], pv_patch) > (bv_maj, bv_min, bv_patch)
                }
            };
            if cmp {
                best = Some(v.clone());
            }
        }
        let chosen = best.ok_or_else(|| CatalogError::VersionNotFound {
            server_type: ServerType::NeoForge.id().to_string(),
            version: mc.to_string(),
        })?;
        let file_name = format!("neoforge-{chosen}-installer.jar");
        let url = format!(
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/{chosen}/neoforge-{chosen}-installer.jar"
        );
        let checksum = self.sha1_sidecar(&url).await?;
        Ok(Artifact {
            server_type: ServerType::NeoForge,
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
        let checksum = artifact.checksum.as_ref().ok_or_else(|| {
            CatalogError::Source("neoforge installer has no checksum".to_string())
        })?;
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
            "{}/tests/fixtures/neoforge/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture present")
    }

    #[tokio::test]
    async fn resolve_returns_sha1_gated_installer() {
        let mut map = HashMap::new();
        map.insert(VERSIONS_URL.to_string(), fixture("versions-slim.json"));
        let sha = std::fs::read(format!(
            "{}/tests/fixtures/neoforge/installer-sha1.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        map.insert(
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.3/neoforge-21.1.3-installer.jar.sha1"
                .to_string(),
            sha.clone(),
        );
        let p = NeoForgeProvider {
            client: FxClient { map },
        };
        let art = p.resolve("1.21.1").await.unwrap();
        assert_eq!(art.server_type, ServerType::NeoForge);
        assert!(art.file_name.starts_with("neoforge-21.1."));
        assert!(art.file_name.ends_with("-installer.jar"));
        let sum = art.checksum.as_ref().unwrap();
        assert_eq!(sum.algo, HashAlgo::Sha1);
        let want = String::from_utf8_lossy(&sha).trim().to_string();
        assert_eq!(&sum.hex, &want);
    }
}
