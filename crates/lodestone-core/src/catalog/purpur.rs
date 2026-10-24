//! Purpur provider over the PurpurMC v2 API. Purpur only publishes an MD5
//! for each build; verification uses that (weaker than SHA-256 but it is the
//! only checksum the upstream provides — we never install without one).

use serde::Deserialize;

use crate::error::CatalogError;
use crate::net::HttpClient;

use super::types::{Artifact, Checksum, HashAlgo, Provider, ServerType, VersionInfo};

pub const PURPUR_API: &str = "https://api.purpurmc.org/v2/purpur";

#[derive(Debug, Deserialize)]
struct LatestResponse {
    build: String,
    md5: String,
    result: String,
}

pub struct PurpurProvider<H: HttpClient> {
    client: H,
}

impl<H: HttpClient> PurpurProvider<H> {
    pub fn new(client: H) -> Self {
        PurpurProvider { client }
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        what: &str,
    ) -> Result<T, CatalogError> {
        let body = self.client.get(url).await?;
        let text = std::str::from_utf8(&body)
            .map_err(|e| CatalogError::Source(format!("{what} utf8: {e}")))?;
        serde_json::from_str(text).map_err(|e| CatalogError::Source(format!("{what}: {e}")))
    }
}

impl<H: HttpClient> Provider for PurpurProvider<H> {
    async fn versions(&self) -> Result<Vec<VersionInfo>, CatalogError> {
        let raw: serde_json::Value = self.get_json(PURPUR_API, "purpur versions").await?;
        let versions = raw
            .get("versions")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // Only stable release ids (all-numeric dot versions).
        Ok(versions
            .into_iter()
            .filter(|v| {
                v.split('.')
                    .all(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())
            })
            .map(|id| VersionInfo {
                server_type: "purpur".to_string(),
                id,
                release: true,
                java_major: None,
            })
            .collect())
    }

    async fn resolve(&self, version: &str) -> Result<Artifact, CatalogError> {
        let latest_url = format!("{PURPUR_API}/{version}/latest");
        let info: LatestResponse = self.get_json(&latest_url, "purpur latest").await?;
        if info.result != "SUCCESS" {
            return Err(CatalogError::Source(format!(
                "purpur build {} for {version} is not SUCCESS",
                info.build
            )));
        }
        let build = info.build;
        let md5 = info.md5;
        let url = format!("{PURPUR_API}/{version}/{build}/download");
        // Purpur's java requirement: 1.21+ needs 21.
        let java_major = if version.starts_with("1.21") || version.starts_with("26") {
            Some(21)
        } else if version.starts_with("1.20")
            || version.starts_with("1.19")
            || version.starts_with("1.18")
        {
            Some(17)
        } else {
            Some(8)
        };
        Ok(Artifact {
            server_type: ServerType::Purpur,
            version: version.to_string(),
            file_name: format!("purpur-{version}-{build}.jar"),
            url,
            checksum: Some(Checksum {
                algo: HashAlgo::Md5,
                hex: md5,
            }),
            size_bytes: None,
            java_major,
        })
    }

    async fn install(
        &self,
        artifact: &Artifact,
        dir: &std::path::Path,
    ) -> Result<(), CatalogError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| CatalogError::Source(format!("mkdir {}: {e}", dir.display())))?;
        let dest = dir.join(&artifact.file_name);
        if dest.exists()
            && let Some(c) = &artifact.checksum
            && c.verify_file(&dest).await.is_ok()
        {
            return Ok(());
        }
        self.client
            .download(&artifact.url, &dest)
            .await
            .map_err(CatalogError::Http)?;
        let c = artifact.checksum.as_ref().ok_or_else(|| {
            CatalogError::Source("no checksum; refusing unverified install".to_string())
        })?;
        c.verify_file(&dest).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/purpur")
                .join(name),
        )
        .expect("fixture present")
    }

    struct FxClient {
        map: HashMap<String, Vec<u8>>,
    }
    impl FxClient {
        fn get(&self, url: &str) -> Result<Vec<u8>, crate::error::NetError> {
            self.map
                .get(url)
                .cloned()
                .ok_or_else(|| crate::error::NetError::Http(format!("miss: {url}")))
        }
    }
    impl HttpClient for FxClient {
        async fn get(&self, url: &str) -> Result<Vec<u8>, crate::error::NetError> {
            self.get(url)
        }
        async fn download(
            &self,
            url: &str,
            dest: &std::path::Path,
        ) -> Result<u64, crate::error::NetError> {
            let bytes = self.get(url)?;
            std::fs::write(dest, &bytes).unwrap();
            Ok(bytes.len() as u64)
        }
    }

    #[test]
    fn parses_versions_list() {
        let raw: serde_json::Value = serde_json::from_str(&fixture("project.json")).unwrap();
        let versions = raw.get("versions").unwrap().as_array().unwrap();
        assert!(!versions.is_empty());
    }

    #[tokio::test]
    async fn resolves_latest_build() {
        let project_url = PURPUR_API.to_string();
        let latest_url = format!("{PURPUR_API}/1.21.11/latest");
        let download_url = format!("{PURPUR_API}/1.21.11/2568/download");
        let mut map = HashMap::new();
        map.insert(project_url, fixture("project.json").into_bytes());
        map.insert(latest_url, fixture("latest-1.21.11.json").into_bytes());
        // download payload
        map.insert(download_url.clone(), b"purpur jar bytes".to_vec());

        let provider = PurpurProvider::new(FxClient { map });
        let a = provider.resolve("1.21.11").await.unwrap();
        assert_eq!(a.server_type, ServerType::Purpur);
        assert_eq!(a.file_name, "purpur-1.21.11-2568.jar");
        assert!(a.checksum.is_some());
        let c = a.checksum.clone().unwrap();
        assert_eq!(c.algo, HashAlgo::Md5);
        // install verifies
        // make the checksum match our synthetic payload
        let mut a = a;
        use md5::compute;
        let d = compute(b"purpur jar bytes");
        a.checksum = Some(Checksum {
            algo: HashAlgo::Md5,
            hex: hex::encode(d.0),
        });
        let dir = std::env::temp_dir().join(format!("lode-purpur-{}", std::process::id()));
        provider.install(&a, &dir).await.unwrap();
        assert!(dir.join(&a.file_name).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn versions_filters_to_stable() {
        let project_url = PURPUR_API.to_string();
        let mut map = HashMap::new();
        map.insert(project_url, fixture("project.json").into_bytes());
        let provider = PurpurProvider::new(FxClient { map });
        let vs = provider.versions().await.unwrap();
        assert!(!vs.is_empty());
        for v in &vs {
            assert!(v.release);
        }
    }
}
