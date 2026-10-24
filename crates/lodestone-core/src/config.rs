//! Server configuration files (`server.properties` and Paper-family config) and
//! the tuning profiles the spec applies per device class (§ "Server config
//! defaults by class"). Everything is real file I/O with atomic writes; no
//! templating of secrets, no invented settings for server types that do not
//! read them.

use std::collections::BTreeMap;

use crate::error::InstanceError;
use crate::storage::{FsStorage, Storage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuningProfile {
    Low,
    Mid,
    High,
}

impl TuningProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            TuningProfile::Low => "low",
            TuningProfile::Mid => "mid",
            TuningProfile::High => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerProperties {
    pub entries: BTreeMap<String, String>,
}

impl ServerProperties {
    pub fn new() -> Self {
        ServerProperties {
            entries: BTreeMap::new(),
        }
    }

    pub fn parse(text: &str) -> Self {
        let mut entries = BTreeMap::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                entries.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        ServerProperties { entries }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.entries.insert(key.into(), value.into());
    }

    pub fn remove(&mut self, key: &str) {
        self.entries.remove(key);
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# Minecraft server properties (managed by Lodestone)\n");
        for (k, v) in &self.entries {
            out.push_str(k);
            out.push('=');
            out.push_str(v);
            out.push('\n');
        }
        out
    }

    /// Apply the device-class baseline for `server_type`. Only writes keys the
    /// server type actually reads: Vanilla gets `server.properties` only;
    /// Paper-family gets the same baseline (Paper reads these too).
    pub fn apply_profile(&mut self, _server_type: &str, profile: TuningProfile) {
        let (view, sim) = match profile {
            TuningProfile::Low => (4u32, 3u32),
            TuningProfile::Mid => (6, 4),
            TuningProfile::High => (8, 6),
        };
        self.set("view-distance", view.to_string());
        self.set("simulation-distance", sim.to_string());
        self.set("network-compression-threshold", "256".to_string());
        self.set(
            "max-players",
            match profile {
                TuningProfile::Low => "8",
                TuningProfile::Mid => "12",
                TuningProfile::High => "20",
            },
        );
    }
}

impl Default for ServerProperties {
    fn default() -> Self {
        Self::new()
    }
}

/// Paper-family servers get an optimized baseline for entity and chunk
/// settings, written to `config/paper-global.yml`. Vanilla does not read
/// this file and must not receive it.
pub fn paper_global_config(profile: TuningProfile) -> String {
    let (entity, despawn) = match profile {
        TuningProfile::Low => (16, 30),
        TuningProfile::Mid => (32, 45),
        TuningProfile::High => (48, 60),
    };
    format!(
        "# Paper global config (managed by Lodestone)\n\
         _version: 29\n\
         chunk-loading:\n  autoconfig-send-distance: true\n\
         entity-activation-range:\n  animals: {entity}\n  monsters: {entity}\n  raiders: {entity}\n  misc: {entity}\n  water: {entity}\n  villagers: {entity}\n  flying-monsters: {entity}\n\
         despawn-ranges:\n  soft: {despawn}\n  hard: {hard}\n",
        hard = despawn + 16
    )
}

pub fn is_paper_family(server_type: &str) -> bool {
    matches!(server_type, "paper" | "folia" | "purpur")
}

/// Write the tuning profile to an instance directory. Creates the files
/// atomically. Returns the list of relative paths written.
pub async fn apply_to_instance_dir(
    storage: &FsStorage,
    server_type: &str,
    profile: TuningProfile,
) -> Result<Vec<String>, InstanceError> {
    let mut props = ServerProperties::new();
    // Preserve any existing settings the user may have set manually.
    if let Ok(existing) = storage.read_file("server.properties").await
        && let Ok(text) = std::str::from_utf8(&existing)
    {
        props = ServerProperties::parse(text);
    }
    props.apply_profile(server_type, profile);
    storage
        .write_file("server.properties", props.render().as_bytes())
        .await
        .map_err(|e| InstanceError::Config(e.to_string()))?;

    let mut written = vec!["server.properties".to_string()];
    if is_paper_family(server_type) {
        // Ensure config/ exists by writing through storage (atomic write
        // creates parents).
        storage
            .write_file(
                "config/paper-global.yml",
                paper_global_config(profile).as_bytes(),
            )
            .await
            .map_err(|e| InstanceError::Config(e.to_string()))?;
        written.push("config/paper-global.yml".to_string());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_root(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("lode-cfg-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn parse_and_render_roundtrip() {
        let text = "view-distance=6\n# comment\nsimulation-distance=4\n";
        let p = ServerProperties::parse(text);
        assert_eq!(p.get("view-distance"), Some("6"));
        assert_eq!(p.get("simulation-distance"), Some("4"));
        let rendered = p.render();
        assert!(rendered.contains("view-distance=6"));
        // re-parse is stable
        let p2 = ServerProperties::parse(&rendered);
        assert_eq!(p, p2);
    }

    #[test]
    fn profile_values_match_spec() {
        for (profile, view, sim) in [
            (TuningProfile::Low, "4", "3"),
            (TuningProfile::Mid, "6", "4"),
            (TuningProfile::High, "8", "6"),
        ] {
            let mut p = ServerProperties::new();
            p.apply_profile("paper", profile);
            assert_eq!(p.get("view-distance"), Some(view), "{profile:?}");
            assert_eq!(p.get("simulation-distance"), Some(sim), "{profile:?}");
            assert_eq!(p.get("network-compression-threshold"), Some("256"));
        }
    }

    #[tokio::test]
    async fn vanilla_does_not_get_paper_config() {
        let root = tmp_root("vanilla");
        let storage = FsStorage::open(&root).unwrap();
        let written = apply_to_instance_dir(&storage, "vanilla", TuningProfile::Mid)
            .await
            .unwrap();
        assert_eq!(written, vec!["server.properties".to_string()]);
        assert!(root.join("server.properties").exists());
        assert!(!root.join("config/paper-global.yml").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn paper_family_gets_paper_config() {
        let root = tmp_root("paper");
        let storage = FsStorage::open(&root).unwrap();
        let written = apply_to_instance_dir(&storage, "paper", TuningProfile::High)
            .await
            .unwrap();
        assert!(written.contains(&"config/paper-global.yml".to_string()));
        let yaml = std::fs::read_to_string(root.join("config/paper-global.yml")).unwrap();
        assert!(yaml.contains("entity-activation-range"));
        assert!(yaml.contains("despawn-ranges"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn preserves_existing_manual_settings() {
        let root = tmp_root("preserve");
        let storage = FsStorage::open(&root).unwrap();
        storage
            .write_file("server.properties", b"motd=My Server\nview-distance=10\n")
            .await
            .unwrap();
        apply_to_instance_dir(&storage, "paper", TuningProfile::Low)
            .await
            .unwrap();
        let text = std::fs::read_to_string(root.join("server.properties")).unwrap();
        assert!(text.contains("motd=My Server"), "{text}");
        assert!(text.contains("view-distance=4"), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
