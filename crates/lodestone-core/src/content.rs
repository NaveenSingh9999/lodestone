//! Content (mods, resource packs, shaders, modpacks) via the Modrinth v2 API.
//! Every downloaded file is hash-verified against the sha1/sha512 the API
//! declares; nothing is installed without a checksum.

use serde::Deserialize;
use std::collections::BTreeMap;

use crate::error::CatalogError;
use crate::net::HttpClient;

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct SearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub downloads: u64,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    #[serde(default)]
    pub total_hits: u64,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Project {
    pub id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "project_type")]
    pub project_type: String,
    #[serde(default)]
    pub versions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Version {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version_number: String,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default)]
    pub files: Vec<VersionFile>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub primary: bool,
}

impl VersionFile {
    pub fn sha1(&self) -> Option<&str> {
        self.hashes.get("sha1").map(String::as_str)
    }
    pub fn sha512(&self) -> Option<&str> {
        self.hashes.get("sha512").map(String::as_str)
    }
}

pub struct ModrinthClient<H: HttpClient> {
    client: H,
}

impl<H: HttpClient> ModrinthClient<H> {
    pub fn new(client: H) -> Self {
        ModrinthClient { client }
    }

    pub async fn search(&self, query: &str, limit: u32) -> Result<SearchResponse, CatalogError> {
        let url = format!(
            "https://api.modrinth.com/v2/search?query={}&limit={}",
            urlencoding(query),
            limit
        );
        let body = self.client.get(&url).await?;
        let text = std::str::from_utf8(&body)
            .map_err(|e| CatalogError::Source(format!("search utf8: {e}")))?;
        serde_json::from_str(text).map_err(|e| CatalogError::Source(format!("search: {e}")))
    }

    pub async fn project(&self, id_or_slug: &str) -> Result<Project, CatalogError> {
        let url = format!("https://api.modrinth.com/v2/project/{id_or_slug}");
        let body = self.client.get(&url).await?;
        let text = std::str::from_utf8(&body)
            .map_err(|e| CatalogError::Source(format!("project utf8: {e}")))?;
        serde_json::from_str(text).map_err(|e| CatalogError::Source(format!("project: {e}")))
    }

    pub async fn versions(&self, project_id: &str) -> Result<Vec<Version>, CatalogError> {
        let url = format!("https://api.modrinth.com/v2/project/{project_id}/version");
        let body = self.client.get(&url).await?;
        let text = std::str::from_utf8(&body)
            .map_err(|e| CatalogError::Source(format!("versions utf8: {e}")))?;
        serde_json::from_str(text).map_err(|e| CatalogError::Source(format!("versions: {e}")))
    }

    /// Download `file` into `dest_dir`, verifying sha1 (preferred) or sha512.
    /// Refuses files with no hash.
    pub async fn install_file(
        &self,
        file: &VersionFile,
        dest_dir: &std::path::Path,
    ) -> Result<std::path::PathBuf, CatalogError> {
        use sha1::Digest as _;
        // Refuse before spending bytes.
        if file.sha1().is_none() && file.sha512().is_none() {
            return Err(CatalogError::Source(format!(
                "file {} has no sha1/sha512; refusing unverified download",
                file.filename
            )));
        }
        std::fs::create_dir_all(dest_dir)
            .map_err(|e| CatalogError::Source(format!("mkdir: {e}")))?;
        let dest = dest_dir.join(&file.filename);
        self.client
            .download(&file.url, &dest)
            .await
            .map_err(CatalogError::Http)?;
        let data = tokio::fs::read(&dest)
            .await
            .map_err(|e| CatalogError::Source(format!("read: {e}")))?;

        // verify
        if let Some(expected) = file.sha1() {
            let mut h = sha1::Sha1::new();
            h.update(&data);
            let got = hex::encode(h.finalize());
            if !got.eq_ignore_ascii_case(expected) {
                let _ = tokio::fs::remove_file(&dest).await;
                return Err(CatalogError::ChecksumMismatch {
                    expected: expected.to_string(),
                    actual: got,
                });
            }
        } else if let Some(expected) = file.sha512() {
            let mut h = sha2::Sha512::new();
            h.update(&data);
            let got = hex::encode(h.finalize());
            if !got.eq_ignore_ascii_case(expected) {
                let _ = tokio::fs::remove_file(&dest).await;
                return Err(CatalogError::ChecksumMismatch {
                    expected: expected.to_string(),
                    actual: got,
                });
            }
        }

        if let Some(size) = file.size
            && data.len() as u64 != size
        {
            let _ = tokio::fs::remove_file(&dest).await;
            return Err(CatalogError::Source(format!(
                "size mismatch for {}: expected {size}, got {}",
                file.filename,
                data.len()
            )));
        }
        Ok(dest)
    }
}

fn urlencoding(s: &str) -> String {
    // Minimal: space -> %20, keep alphanumerics and a few safe chars.
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            ' ' => out.push_str("%20"),
            _ => {
                for b in c.to_string().as_bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/modrinth")
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
    fn parses_search() {
        let r: SearchResponse = serde_json::from_str(&fixture("search.json")).unwrap();
        assert!(!r.hits.is_empty());
        assert!(r.total_hits > 0);
        assert!(!r.hits[0].slug.is_empty());
    }

    #[test]
    fn parses_project() {
        let p: Project = serde_json::from_str(&fixture("project.json")).unwrap();
        assert_eq!(p.slug, "lithium");
        assert!(!p.versions.is_empty());
    }

    #[test]
    fn parses_versions() {
        let v: Vec<Version> = serde_json::from_str(&fixture("versions.json")).unwrap();
        assert!(!v.is_empty());
        assert!(!v[0].files.is_empty());
        assert!(v[0].files[0].sha1().is_some() || v[0].files[0].sha512().is_some());
    }

    #[tokio::test]
    async fn search_and_versions_flow() {
        let search_url = "https://api.modrinth.com/v2/search?query=lithium&limit=3".to_string();
        let project_url = "https://api.modrinth.com/v2/project/lithium".to_string();
        let mut map = HashMap::new();
        map.insert(search_url, fixture("search.json").into_bytes());
        map.insert(project_url.clone(), fixture("project.json").into_bytes());
        // versions url uses the real project id from the project fixture
        let project: Project = serde_json::from_str(&fixture("project.json")).unwrap();
        let versions_url = format!("https://api.modrinth.com/v2/project/{}/version", project.id);
        map.insert(versions_url, fixture("versions.json").into_bytes());
        let client = ModrinthClient::new(FxClient { map });
        let r = client.search("lithium", 3).await.unwrap();
        assert!(!r.hits.is_empty());
        let p = client.project("lithium").await.unwrap();
        assert_eq!(p.slug, "lithium");
        let vs = client.versions(&p.id).await.unwrap();
        assert!(!vs.is_empty());
    }

    #[tokio::test]
    async fn install_file_verifies_hash() {
        let versions: Vec<Version> = serde_json::from_str(&fixture("versions.json")).unwrap();
        let file = &versions[0].files[0];
        let payload = b"fake mod bytes".to_vec();
        // compute sha1 of payload and override file hash to match
        use sha1::Digest;
        let mut h = sha1::Sha1::new();
        h.update(&payload);
        let sha = hex::encode(h.finalize());
        let mut f = file.clone();
        f.hashes.insert("sha1".to_string(), sha);
        f.url = "fixture://mod".to_string();
        f.size = Some(payload.len() as u64);

        let mut map = HashMap::new();
        map.insert("fixture://mod".to_string(), payload.clone());
        let client = ModrinthClient::new(FxClient { map });
        let dir = std::env::temp_dir().join(format!("lode-mod-{}", std::process::id()));
        let path = client.install_file(&f, &dir).await.unwrap();
        assert!(path.exists());

        // wrong hash fails
        let mut bad = f.clone();
        bad.hashes.insert("sha1".to_string(), "0".repeat(40));
        let err = client.install_file(&bad, &dir).await.unwrap_err();
        assert!(
            err.to_string().contains("checksum") || err.to_string().contains("Mismatch"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn refuses_file_without_hash() {
        let versions: Vec<Version> = serde_json::from_str(&fixture("versions.json")).unwrap();
        let mut f = versions[0].files[0].clone();
        f.hashes.clear();
        f.url = "fixture://nohash".to_string();
        let client = ModrinthClient::new(FxClient {
            map: HashMap::new(),
        });
        let dir = std::env::temp_dir().join(format!("lode-mod2-{}", std::process::id()));
        let err = client.install_file(&f, &dir).await.unwrap_err();
        assert!(err.to_string().contains("no sha1"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
