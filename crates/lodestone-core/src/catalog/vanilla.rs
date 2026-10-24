use serde::Deserialize;

use crate::error::CatalogError;
use crate::net::HttpClient;

use super::types::{Artifact, Checksum, HashAlgo, Provider, ServerType, VersionInfo};

pub const VANILLA_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, Deserialize)]
pub struct RawManifest {
    pub latest: RawLatest,
    pub versions: Vec<RawVersion>,
}

#[derive(Debug, Deserialize)]
pub struct RawLatest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Deserialize)]
pub struct RawVersion {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RawVersionJson {
    #[serde(rename = "javaVersion", default)]
    pub java_version: Option<RawJavaVersion>,
    #[serde(default)]
    pub downloads: Option<RawDownloads>,
}

#[derive(Debug, Deserialize)]
pub struct RawJavaVersion {
    #[serde(default)]
    pub component: Option<String>,
    #[serde(rename = "majorVersion")]
    pub major_version: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct RawDownloads {
    pub server: Option<RawDownload>,
}

#[derive(Debug, Deserialize)]
pub struct RawDownload {
    pub url: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub sha1: Option<String>,
}

pub fn parse_manifest(text: &str) -> Result<RawManifest, CatalogError> {
    serde_json::from_str(text).map_err(|e| CatalogError::Source(format!("manifest: {e}")))
}

pub fn parse_version_json(text: &str) -> Result<RawVersionJson, CatalogError> {
    serde_json::from_str(text).map_err(|e| CatalogError::Source(format!("version json: {e}")))
}

pub struct VanillaProvider<H: HttpClient> {
    client: H,
}

impl<H: HttpClient> VanillaProvider<H> {
    pub fn new(client: H) -> Self {
        VanillaProvider { client }
    }

    async fn fetch_manifest(&self) -> Result<RawManifest, CatalogError> {
        let body = self.client.get(VANILLA_MANIFEST_URL).await?;
        let text = std::str::from_utf8(&body)
            .map_err(|e| CatalogError::Source(format!("manifest utf8: {e}")))?;
        parse_manifest(text)
    }
}

impl<H: HttpClient> Provider for VanillaProvider<H> {
    async fn versions(&self) -> Result<Vec<VersionInfo>, CatalogError> {
        let raw = self.fetch_manifest().await?;
        Ok(raw
            .versions
            .into_iter()
            .filter(|v| v.kind == "release" || v.kind == "snapshot")
            .map(|v| VersionInfo {
                server_type: ServerType::Vanilla.id().to_string(),
                id: v.id,
                release: v.kind == "release",
                java_major: None,
            })
            .collect())
    }

    async fn resolve(&self, version: &str) -> Result<Artifact, CatalogError> {
        let raw = self.fetch_manifest().await?;
        let manifest_entry = raw
            .versions
            .iter()
            .find(|v| v.id == version)
            .ok_or_else(|| CatalogError::VersionNotFound {
                server_type: ServerType::Vanilla.id().to_string(),
                version: version.to_string(),
            })?;

        let body = self.client.get(&manifest_entry.url).await?;
        let text = std::str::from_utf8(&body)
            .map_err(|e| CatalogError::Source(format!("version json utf8: {e}")))?;
        let vj = parse_version_json(text)?;

        let download = vj
            .downloads
            .and_then(|d| d.server)
            .ok_or_else(|| CatalogError::Source(format!("no server download for {version}")))?;
        let checksum = download.sha1.ok_or_else(|| {
            CatalogError::Source(format!("no SHA1 for the {version} server download"))
        })?;
        let java_major = vj
            .java_version
            .and_then(|j| j.major_version)
            .ok_or_else(|| CatalogError::Source(format!("no javaVersion for {version}")))?;

        Ok(Artifact {
            server_type: ServerType::Vanilla,
            version: version.to_string(),
            file_name: format!("vanilla-{version}-server.jar"),
            url: download.url,
            checksum: Some(Checksum {
                algo: HashAlgo::Sha1,
                hex: checksum,
            }),
            size_bytes: download.size,
            java_major: Some(java_major),
        })
    }

    async fn install(
        &self,
        artifact: &Artifact,
        dir: &std::path::Path,
    ) -> Result<(), CatalogError> {
        // Refuse an unverified artifact BEFORE spending bytes on it.
        artifact.checksum.as_ref().ok_or_else(|| {
            CatalogError::Source(
                "artifact has no checksum; refusing unverified install".to_string(),
            )
        })?;
        std::fs::create_dir_all(dir)
            .map_err(|e| CatalogError::Source(format!("mkdir {}: {e}", dir.display())))?;
        let dest = dir.join(&artifact.file_name);

        if dest.exists() {
            if let Some(checksum) = &artifact.checksum {
                if checksum.verify_file(&dest).await.is_ok() {
                    return Ok(());
                }
            }
        }

        self.client.download(&artifact.url, &dest).await?;

        let checksum = artifact.checksum.as_ref().unwrap();
        checksum.verify_file(&dest).await?;

        if let Some(expected_size) = artifact.size_bytes {
            let actual = tokio::fs::metadata(&dest)
                .await
                .map_err(|e| CatalogError::Source(format!("stat {}: {e}", dest.display())))?
                .len();
            if actual != expected_size {
                return Err(CatalogError::Source(format!(
                    "size mismatch for {}: expected {expected_size}, got {actual}",
                    dest.display()
                )));
            }
        }
        Ok(())
    }
}
