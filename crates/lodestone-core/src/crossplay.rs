//! Bedrock crossplay (spec 4.11 optional switch).
//!
//! Installs Geyser (and optionally Floodgate, ViaVersion, ViaBackwards) for a
//! given server type and Minecraft version using the Modrinth v2 API. Geyser
//! bridges Bedrock clients to the Java server; Floodgate lets Bedrock players
//! join without a Java account. ViaVersion/ViaBackwards let older clients
//! connect to newer servers and vice versa.
//!
//! The content folder is determined by the server type:
//! - Paper-family (paper, purpur, folia): `plugins/`
//! - Fabric/Quilt/Forge/NeoForge: `mods/`

use crate::content::ModrinthClient;
use crate::error::CatalogError;
use crate::net::HttpClient;

/// Which Bedrock crossplay components to install.
#[derive(Debug, Clone, Copy, Default)]
pub struct CrossplayOptions {
    pub geyser: bool,
    pub floodgate: bool,
    pub via_version: bool,
    pub via_backwards: bool,
}

impl CrossplayOptions {
    pub fn full() -> Self {
        Self {
            geyser: true,
            floodgate: true,
            via_version: true,
            via_backwards: true,
        }
    }
}

/// Result of a crossplay installation.
#[derive(Debug, Clone, Default)]
pub struct CrossplayResult {
    pub installed: Vec<CrossplayComponent>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CrossplayComponent {
    pub name: String,
    pub file: String,
    pub version: String,
}

/// Errors during crossplay installation.
#[derive(Debug, thiserror::Error)]
pub enum CrossplayError {
    #[error("unsupported server type for crossplay: {0}")]
    UnsupportedServerType(String),
    #[error("no compatible version found for {0} (mc={1}, loader={2})")]
    NoVersion(String, String, String),
    #[error("catalog: {0}")]
    Catalog(#[from] CatalogError),
}

/// Maps a server type to the Modrinth content loader string.
fn loader_for_server_type(server_type: &str) -> Result<&'static str, CrossplayError> {
    match server_type {
        "paper" | "purpur" | "folia" => Ok("paper"),
        "fabric" => Ok("fabric"),
        "quilt" => Ok("quilt"),
        "forge" => Ok("forge"),
        "neoforge" => Ok("neoforge"),
        other => Err(CrossplayError::UnsupportedServerType(other.to_string())),
    }
}

/// Returns true if the server type uses a `plugins/` folder.
fn uses_plugins(server_type: &str) -> bool {
    matches!(
        server_type,
        "paper" | "purpur" | "folia" | "spigot" | "bukkit"
    )
}

/// Returns the content folder name for a server type.
pub fn content_folder(server_type: &str) -> &'static str {
    if uses_plugins(server_type) {
        "plugins"
    } else {
        "mods"
    }
}

/// Project slugs on Modrinth for each component, keyed by server type.
fn geyser_slug(server_type: &str) -> &'static str {
    match server_type {
        "fabric" => "geyser-fabric",
        "forge" => "geyser-forge",
        "neoforge" => "geyser-neoforge",
        _ => "geyser",
    }
}

fn floodgate_slug(server_type: &str) -> &'static str {
    match server_type {
        "fabric" => "floodgate-fabric",
        "forge" => "floodgate-forge",
        "neoforge" => "floodgate-neoforge",
        _ => "floodgate",
    }
}

/// ViaVersion project slug (same for all platforms).
const VIA_VERSION_SLUG: &str = "viaversion";
/// ViaBackwards project slug.
const VIA_BACKWARDS_SLUG: &str = "viabackwards";

/// Install Bedrock crossplay components into `instance_dir`.
///
/// For each enabled component in `options`, searches Modrinth, resolves a
/// version compatible with `mc_version` and the server's loader, and installs
/// the primary file into `<instance_dir>/<content_folder>/`.
pub async fn install_crossplay<H: HttpClient>(
    client: &ModrinthClient<H>,
    server_type: &str,
    mc_version: &str,
    instance_dir: &std::path::Path,
    options: CrossplayOptions,
) -> Result<CrossplayResult, CrossplayError> {
    let loader = loader_for_server_type(server_type)?;
    let folder = content_folder(server_type);
    let dest_dir = instance_dir.join(folder);
    std::fs::create_dir_all(&dest_dir).ok();

    let mut result = CrossplayResult::default();

    // Geyser
    if options.geyser {
        let slug = geyser_slug(server_type);
        match install_component(client, slug, mc_version, loader, &dest_dir).await {
            Ok(comp) => result.installed.push(comp),
            Err(e) => result.skipped.push(format!("geyser: {e}")),
        }
    }

    // Floodgate
    if options.floodgate {
        let slug = floodgate_slug(server_type);
        match install_component(client, slug, mc_version, loader, &dest_dir).await {
            Ok(comp) => result.installed.push(comp),
            Err(e) => result.skipped.push(format!("floodgate: {e}")),
        }
    }

    // ViaVersion
    if options.via_version {
        match install_component(client, VIA_VERSION_SLUG, mc_version, loader, &dest_dir).await {
            Ok(comp) => result.installed.push(comp),
            Err(e) => result.skipped.push(format!("viaversion: {e}")),
        }
    }

    // ViaBackwards
    if options.via_backwards {
        match install_component(client, VIA_BACKWARDS_SLUG, mc_version, loader, &dest_dir).await {
            Ok(comp) => result.installed.push(comp),
            Err(e) => result.skipped.push(format!("viabackwards: {e}")),
        }
    }

    Ok(result)
}

async fn install_component<H: HttpClient>(
    client: &ModrinthClient<H>,
    slug: &str,
    mc_version: &str,
    loader: &str,
    dest_dir: &std::path::Path,
) -> Result<CrossplayComponent, CrossplayError> {
    let versions = client
        .versions(slug)
        .await
        .map_err(CrossplayError::Catalog)?;

    // Find a version compatible with mc_version and loader.
    let compatible = versions
        .iter()
        .find(|v| {
            v.game_versions.iter().any(|gv| gv == mc_version)
                && (v.loaders.is_empty() || v.loaders.iter().any(|l| l == loader))
        })
        .or_else(|| versions.first());

    let version = compatible.ok_or_else(|| {
        CrossplayError::NoVersion(slug.to_string(), mc_version.to_string(), loader.to_string())
    })?;

    // Pick the primary file (or first file).
    let file = version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
        .ok_or_else(|| {
            CrossplayError::NoVersion(slug.to_string(), mc_version.to_string(), loader.to_string())
        })?;

    let path = client
        .install_file(file, dest_dir)
        .await
        .map_err(CrossplayError::Catalog)?;

    Ok(CrossplayComponent {
        name: slug.to_string(),
        file: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        version: version.version_number.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::ModrinthClient;
    use std::collections::HashMap;

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
    fn loader_for_server_types() {
        assert_eq!(loader_for_server_type("paper").unwrap(), "paper");
        assert_eq!(loader_for_server_type("purpur").unwrap(), "paper");
        assert_eq!(loader_for_server_type("fabric").unwrap(), "fabric");
        assert_eq!(loader_for_server_type("quilt").unwrap(), "quilt");
        assert_eq!(loader_for_server_type("forge").unwrap(), "forge");
        assert_eq!(loader_for_server_type("neoforge").unwrap(), "neoforge");
        assert!(loader_for_server_type("vanilla").is_err());
    }

    #[test]
    fn content_folder_mapping() {
        assert_eq!(content_folder("paper"), "plugins");
        assert_eq!(content_folder("purpur"), "plugins");
        assert_eq!(content_folder("fabric"), "mods");
        assert_eq!(content_folder("forge"), "mods");
        assert_eq!(content_folder("quilt"), "mods");
        assert_eq!(content_folder("neoforge"), "mods");
    }

    #[test]
    fn geyser_slug_mapping() {
        assert_eq!(geyser_slug("paper"), "geyser");
        assert_eq!(geyser_slug("fabric"), "geyser-fabric");
        assert_eq!(geyser_slug("forge"), "geyser-forge");
        assert_eq!(geyser_slug("neoforge"), "geyser-neoforge");
    }

    #[test]
    fn floodgate_slug_mapping() {
        assert_eq!(floodgate_slug("paper"), "floodgate");
        assert_eq!(floodgate_slug("fabric"), "floodgate-fabric");
        assert_eq!(floodgate_slug("forge"), "floodgate-forge");
        assert_eq!(floodgate_slug("neoforge"), "floodgate-neoforge");
    }

    #[tokio::test]
    async fn install_crossplay_with_mock() {
        use sha1::Digest as _;
        // Build a fake version response for geyser
        let ver = serde_json::json!([{
            "id": "geyser-1.21",
            "name": "Geyser 2.4.0",
            "version_number": "2.4.0",
            "game_versions": ["1.21.1"],
            "loaders": ["paper"],
            "files": [{
                "url": "fixture://geyser.jar",
                "filename": "Geyser-Spigot.jar",
                "size": 16,
                "hashes": {"sha1": "placeholder"},
                "primary": true
            }]
        }]);
        let payload = b"geyser jar bytes".to_vec();
        let mut h = sha1::Sha1::new();
        h.update(&payload);
        let sha = hex::encode(h.finalize());
        let mut ver = ver;
        ver[0]["files"][0]["hashes"]["sha1"] = serde_json::Value::String(sha.clone());

        let versions_url = "https://api.modrinth.com/v2/project/geyser/version".to_string();
        let mut map = HashMap::new();
        map.insert(versions_url, ver.to_string().into_bytes());
        map.insert("fixture://geyser.jar".to_string(), payload);

        let client = ModrinthClient::new(FxClient { map });
        let dir = std::env::temp_dir().join(format!("lode-xplay-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        let opts = CrossplayOptions {
            geyser: true,
            floodgate: false,
            via_version: false,
            via_backwards: false,
        };
        let result = install_crossplay(&client, "paper", "1.21.1", &dir, opts)
            .await
            .unwrap();
        assert_eq!(result.installed.len(), 1);
        assert_eq!(result.installed[0].name, "geyser");
        assert!(dir.join("plugins").join("Geyser-Spigot.jar").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn crossplay_unsupported_server_type() {
        let client = ModrinthClient::new(FxClient {
            map: HashMap::new(),
        });
        let dir = std::env::temp_dir().join(format!("lode-xplay2-{}", std::process::id()));
        let opts = CrossplayOptions::default();
        let err = install_crossplay(&client, "vanilla", "1.21.1", &dir, opts)
            .await
            .unwrap_err();
        assert!(matches!(err, CrossplayError::UnsupportedServerType(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
