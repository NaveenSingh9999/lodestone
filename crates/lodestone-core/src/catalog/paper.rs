//! PaperMC provider over the Fill v3 API (`fill.papermc.io/v3/...`) per the
//! Lodestone spec. Every artifact is hash-verified (sha256 declared by the
//! signed-build API payload on `fill-data.papermc.io`).

use serde::Deserialize;

use crate::error::CatalogError;
use crate::net::HttpClient;

use super::types::{Artifact, Checksum, HashAlgo, Provider, ServerType, VersionInfo};

pub const FILL_V3_BASE: &str = "https://fill.papermc.io/v3";

pub fn project_endpoint(project: &str) -> String {
    format!("{FILL_V3_BASE}/projects/{project}")
}

pub fn versions_endpoint(project: &str) -> String {
    format!(
        "{project_path}/versions",
        project_path = project_endpoint(project)
    )
}

pub fn version_endpoint(project: &str, version: &str) -> String {
    format!(
        "{project_path}/versions/{version}",
        project_path = project_endpoint(project)
    )
}

pub fn latest_build_endpoint(project: &str, version: &str) -> String {
    format!(
        "{project_path}/versions/{version}/builds/latest",
        project_path = project_endpoint(project)
    )
}

#[derive(Debug, Deserialize)]
pub struct RawProject {
    pub versions: std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RawVersionsList {
    pub versions: Vec<RawVersionsEntry>,
}

#[derive(Debug, Deserialize)]
pub struct RawVersionsEntry {
    pub version: RawVersionMeta,
}

#[derive(Debug, Deserialize)]
pub struct RawVersionMeta {
    pub id: String,
    #[serde(default)]
    pub support: Option<RawSupport>,
    #[serde(default)]
    pub java: Option<RawJava>,
}

#[derive(Debug, Deserialize)]
pub struct RawSupport {
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RawJava {
    #[serde(default)]
    pub version: Option<RawJavaVersion>,
    #[serde(default)]
    pub flags: Option<RawJavaFlags>,
}

#[derive(Debug, Deserialize)]
pub struct RawJavaVersion {
    #[serde(rename = "minimum", default)]
    pub minimum: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct RawJavaFlags {
    #[serde(rename = "recommended", default)]
    pub recommended: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct RawVersionDetail {
    pub version: RawVersionMeta,
    pub builds: Vec<u64>,
}

/// One Fill v3 build object, restricted to what we consume.
#[derive(Debug, Deserialize)]
pub struct RawBuild {
    pub id: u64,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub downloads: Option<std::collections::BTreeMap<String, RawDownload>>,
}

#[derive(Debug, Deserialize)]
pub struct RawDownload {
    pub name: String,
    #[serde(default)]
    pub checksums: Option<RawChecksums>,
    #[serde(default)]
    pub size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct RawChecksums {
    #[serde(rename = "sha256", default)]
    pub sha256: Option<String>,
}

fn parse_json<T: for<'de> Deserialize<'de>>(text: &str, what: &str) -> Result<T, CatalogError> {
    serde_json::from_str(text).map_err(|e| CatalogError::Source(format!("{what}: {e}")))
}

/// True for plain releases (e.g. `1.21.11`), false for rc/pre/snapshot ids.
pub fn is_release_version(id: &str) -> bool {
    id.split('.')
        .all(|part| part.chars().all(|c| c.is_ascii_digit()) && !part.is_empty())
        && !id.is_empty()
}

pub struct PaperProvider<H: HttpClient> {
    client: H,
    /// Fill v3 project id ("paper", "folia" share the same API shape).
    project: &'static str,
}

impl<H: HttpClient> PaperProvider<H> {
    pub fn new(client: H) -> Self {
        PaperProvider {
            client,
            project: "paper",
        }
    }

    pub fn with_project(project: &'static str, client: H) -> Self {
        PaperProvider { client, project }
    }

    async fn get_json<T>(&self, url: &str, what: &str) -> Result<T, CatalogError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let body = self.client.get(url).await?;
        let text = std::str::from_utf8(&body)
            .map_err(|e| CatalogError::Source(format!("{what} utf8: {e}")))?;
        parse_json(text, what)
    }
}

impl<H: HttpClient> Provider for PaperProvider<H> {
    async fn versions(&self) -> Result<Vec<VersionInfo>, CatalogError> {
        let raw = self
            .get_json::<RawVersionsList>(&versions_endpoint(self.project), "versions list")
            .await?;
        Ok(raw
            .versions
            .into_iter()
            .filter_map(|entry| {
                let java_major = entry
                    .version
                    .java
                    .as_ref()
                    .and_then(|j| j.version.as_ref())
                    .and_then(|v| v.minimum);
                let id = entry.version.id;
                if !is_release_version(&id) {
                    return None;
                }
                Some(VersionInfo {
                    server_type: self.project.to_string(),
                    id,
                    release: true,
                    java_major,
                })
            })
            .collect())
    }

    async fn resolve(&self, version: &str) -> Result<Artifact, CatalogError> {
        if !is_release_version(version) {
            return Err(CatalogError::VersionNotFound {
                server_type: self.project.to_string(),
                version: version.to_string(),
            });
        }
        let url = version_endpoint(self.project, version);
        let detail = self
            .get_json::<RawVersionDetail>(&url, "version detail")
            .await?;
        let java_major = detail
            .version
            .java
            .as_ref()
            .and_then(|j| j.version.as_ref())
            .and_then(|jv| jv.minimum);
        if detail.builds.is_empty() {
            return Err(CatalogError::VersionNotFound {
                server_type: self.project.to_string(),
                version: version.to_string(),
            });
        }
        let latest = detail.builds.iter().max().copied().unwrap_or(0);
        let latest_url = latest_build_endpoint(self.project, version);
        let build: RawBuild = self
            .get_json::<RawBuild>(&latest_url, "latest build")
            .await?;
        if build.id != latest {
            return Err(CatalogError::Source(format!(
                "API drifted: latest build moved from {latest} to {} mid-resolve; refusing",
                build.id
            )));
        }
        // Non-stable channels require explicit acknowledgment (spec: users are
        // warned before EXPERIMENTAL dev builds).
        let channel = build.channel.unwrap_or_default();
        if channel != "STABLE" {
            return Err(CatalogError::Source(format!(
                "latest {}/{} build {} has channel {channel}; stable policy requires it to be STABLE",
                self.project, version, build.id
            )));
        }
        let downloads = build.downloads.unwrap_or_default();
        let dl = downloads
            .get("server:default")
            .ok_or_else(|| CatalogError::Source("download 'server:default' missing".to_string()))?;
        let checksum_hex = dl
            .checksums
            .as_ref()
            .and_then(|c| c.sha256.clone())
            .ok_or_else(|| {
                CatalogError::Source(format!(
                    "no sha256 checksum for {} build {}; refusing unverified download",
                    self.project, build.id
                ))
            })?;
        Ok(Artifact {
            server_type: ServerType::Paper,
            version: version.to_string(),
            file_name: dl.name.clone(),
            url: dl.url.clone(),
            checksum: Some(Checksum {
                algo: HashAlgo::Sha256,
                hex: checksum_hex,
            }),
            size_bytes: dl.size,
            java_major,
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
        if dest.exists()
            && let Some(checksum) = &artifact.checksum
            && checksum.verify_file(&dest).await.is_ok()
        {
            return Ok(());
        }
        self.client.download(&artifact.url, &dest).await?;
        artifact
            .checksum
            .as_ref()
            .unwrap()
            .verify_file(&dest)
            .await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::NetError;
    use std::collections::HashMap;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/paper")
                .join(name),
        )
        .expect("fixture present")
    }

    struct FxClient {
        map: HashMap<String, Vec<u8>>,
    }

    impl FxClient {
        fn get(&self, url: &str) -> Result<Vec<u8>, NetError> {
            self.map
                .get(url)
                .cloned()
                .ok_or_else(|| NetError::Http(format!("fixture miss: {url}")))
        }
    }

    impl HttpClient for FxClient {
        async fn get(&self, url: &str) -> Result<Vec<u8>, NetError> {
            self.get(url)
        }
        async fn download(&self, url: &str, dest: &std::path::Path) -> Result<u64, NetError> {
            let bytes = self.get(url)?;
            std::fs::write(dest, &bytes).unwrap();
            Ok(bytes.len() as u64)
        }
    }

    fn provider_with_fixture() -> PaperProvider<FxClient> {
        let detail = fixture("version-1.21.11.json");
        let build = fixture("build-1.21.11-132.json");
        let mut map = HashMap::new();
        map.insert(version_endpoint("paper", "1.21.11"), detail.into_bytes());
        map.insert(
            latest_build_endpoint("paper", "1.21.11"),
            build.into_bytes(),
        );
        PaperProvider {
            client: FxClient { map },
            project: "paper",
        }
    }

    #[test]
    fn parses_project_listing() {
        let proj: RawProject = parse_json(&fixture("project.json"), "project").unwrap();
        assert!(proj.versions.contains_key("1.21"));
        assert!(proj.versions["1.16"].contains(&"1.16.5".to_string()));
    }

    #[test]
    fn parses_versions_list() {
        let list: RawVersionsList = parse_json(&fixture("versions-slim.json"), "versions").unwrap();
        assert_eq!(list.versions.len(), 2);
        let java = list.versions[0]
            .version
            .java
            .as_ref()
            .expect("java metadata")
            .version
            .as_ref()
            .expect("version")
            .minimum;
        assert!(java.is_some());
    }

    #[test]
    fn release_filter() {
        assert!(is_release_version("1.21.11"));
        assert!(is_release_version("26.3"));
        assert!(!is_release_version("1.21.11-rc3"));
        assert!(!is_release_version(""));
    }

    #[tokio::test]
    async fn resolves_latest_stable_build() {
        let p = provider_with_fixture();
        let a = p.resolve("1.21.11").await.unwrap();
        assert_eq!(a.file_name, "paper-1.21.11-132.jar");
        assert_eq!(
            a.checksum.unwrap().hex,
            "5ffef465eeeb5f2a3c23a24419d97c51afd7dbb4923ff42df9a3f58bba1ccfba"
        );
        assert_eq!(a.size_bytes, Some(54846016));
        assert_eq!(a.java_major, Some(21));
        assert_eq!(a.server_type, ServerType::Paper);
    }

    #[tokio::test]
    async fn rejects_snapshot_versions() {
        let p = provider_with_fixture();
        assert!(p.resolve("1.21.11-rc3").await.is_err());
    }

    #[tokio::test]
    async fn channels_must_be_stable() {
        let mut fx = fixture("build-1.21.11-132.json");
        fx = fx.replace("STABLE", "EXPERIMENTAL");
        let mut map = HashMap::new();
        map.insert(
            version_endpoint("paper", "1.21.11"),
            fixture("version-1.21.11.json").into_bytes(),
        );
        map.insert(latest_build_endpoint("paper", "1.21.11"), fx.into_bytes());
        let p = PaperProvider {
            client: FxClient { map },
            project: "paper",
        };
        let err = p.resolve("1.21.11").await.unwrap_err();
        assert!(err.to_string().contains("EXPERIMENTAL"), "{err}");
    }

    #[tokio::test]
    async fn refuse_to_install_without_checksum() {
        let art = Artifact {
            server_type: ServerType::Paper,
            version: "x".into(),
            file_name: "x.jar".into(),
            url: "fixture://missing".into(),
            checksum: None,
            size_bytes: None,
            java_major: None,
        };
        let p = PaperProvider {
            client: FxClient {
                map: HashMap::new(),
            },
            project: "paper",
        };
        let dir = std::env::temp_dir().join(format!("lode-paper-{}", std::process::id()));
        let err = p.install(&art, &dir).await.unwrap_err();
        assert!(err.to_string().contains("no checksum"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn install_verifies_sha256() {
        let payload = b"paper jar pretend bytes".to_vec();
        let sha = {
            use sha2::Digest;
            let mut h = sha2::Sha256::new();
            h.update(&payload);
            hex::encode(h.finalize())
        };
        let art = Artifact {
            server_type: ServerType::Paper,
            version: "1.21.11".into(),
            file_name: "paper-test.jar".into(),
            url: "fixture://jar".into(),
            checksum: Some(Checksum {
                algo: HashAlgo::Sha256,
                hex: sha.clone(),
            }),
            size_bytes: Some(payload.len() as u64),
            java_major: None,
        };
        let mut map = HashMap::new();
        map.insert("fixture://jar".to_string(), payload);
        let p = PaperProvider {
            client: FxClient { map },
            project: "paper",
        };
        let dir = std::env::temp_dir().join(format!("lode-paper-ok-{}", std::process::id()));
        p.install(&art, &dir).await.unwrap();
        assert!(
            dir.join("paper-test.jar").exists(),
            "expected installed jar"
        );
        // wrong checksum must fail
        let mut bad = art.clone();
        bad.checksum = Some(Checksum {
            algo: HashAlgo::Sha256,
            hex: "ff".repeat(32),
        });
        assert!(p.install(&bad, &dir).await.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
