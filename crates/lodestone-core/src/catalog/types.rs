use sha2::Digest;

use crate::error::CatalogError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerType {
    Vanilla,
    Paper,
    Folia,
    Purpur,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl ServerType {
    pub fn id(&self) -> &'static str {
        match self {
            ServerType::Vanilla => "vanilla",
            ServerType::Paper => "paper",
            ServerType::Folia => "folia",
            ServerType::Purpur => "purpur",
            ServerType::Fabric => "fabric",
            ServerType::Quilt => "quilt",
            ServerType::Forge => "forge",
            ServerType::NeoForge => "neoforge",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgo {
    Sha1,
    Sha256,
}

#[derive(Debug, Clone)]
pub struct Checksum {
    pub algo: HashAlgo,
    pub hex: String,
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn digest_hex(algo: HashAlgo, data: &[u8]) -> String {
    match algo {
        HashAlgo::Sha1 => {
            let mut h = sha1::Sha1::new();
            h.update(data);
            lower_hex(&h.finalize())
        }
        HashAlgo::Sha256 => {
            let mut h = sha2::Sha256::new();
            h.update(data);
            lower_hex(&h.finalize())
        }
    }
}

impl Checksum {
    pub fn verify(&self, data: &[u8]) -> Result<(), CatalogError> {
        let actual = digest_hex(self.algo, data);
        if actual.eq_ignore_ascii_case(&self.hex) {
            Ok(())
        } else {
            Err(CatalogError::ChecksumMismatch {
                expected: self.hex.clone(),
                actual,
            })
        }
    }

    pub async fn verify_file(&self, path: &std::path::Path) -> Result<(), CatalogError> {
        let data = tokio::fs::read(path)
            .await
            .map_err(|e| CatalogError::Source(format!("read {}: {e}", path.display())))?;
        self.verify(&data)
    }
}

#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub server_type: String,
    pub id: String,
    pub release: bool,
    pub java_major: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Artifact {
    pub server_type: ServerType,
    pub version: String,
    pub file_name: String,
    pub url: String,
    pub checksum: Option<Checksum>,
    pub size_bytes: Option<u64>,
    pub java_major: Option<u32>,
}

pub trait Provider: Send + Sync {
    fn versions(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<VersionInfo>, CatalogError>> + Send;
    fn resolve(
        &self,
        version: &str,
    ) -> impl std::future::Future<Output = Result<Artifact, CatalogError>> + Send;
    fn install(
        &self,
        artifact: &Artifact,
        dir: &std::path::Path,
    ) -> impl std::future::Future<Output = Result<(), CatalogError>> + Send;
}
