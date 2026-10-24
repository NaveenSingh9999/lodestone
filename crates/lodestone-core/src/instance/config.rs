use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::InstanceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeapProfile {
    #[default]
    Auto,
    Low,
    Mid,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceConfig {
    pub id: String,
    pub name: String,
    pub server_type: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    pub java_major: u32,
    #[serde(default)]
    pub heap_profile: HeapProfile,
    pub port: u16,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
    pub created_ms: u64,
    #[serde(default)]
    pub eula_accepted: bool,
}

impl InstanceConfig {
    pub fn to_toml(&self) -> Result<String, InstanceError> {
        toml::to_string(self).map_err(|e| InstanceError::Config(e.to_string()))
    }

    pub fn from_toml(text: &str) -> Result<Self, InstanceError> {
        let cfg: InstanceConfig =
            toml::from_str(text).map_err(|e| InstanceError::Config(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), InstanceError> {
        if self.id.is_empty() {
            return Err(InstanceError::Config("id must not be empty".into()));
        }
        if self.name.trim().is_empty() {
            return Err(InstanceError::Config("name must not be blank".into()));
        }
        if self.version.is_empty() {
            return Err(InstanceError::Config("version must not be empty".into()));
        }
        if self.port == 0 {
            return Err(InstanceError::Config("port must not be 0".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> InstanceConfig {
        InstanceConfig {
            id: "i-001".into(),
            name: "Home SMP".into(),
            server_type: "paper".into(),
            version: "1.21.9".into(),
            build: Some("22".into()),
            java_major: 21,
            heap_profile: HeapProfile::Auto,
            port: 25565,
            options: BTreeMap::new(),
            created_ms: 1,
            eula_accepted: false,
        }
    }

    #[test]
    fn roundtrip() {
        let a = sample();
        let text = a.to_toml().unwrap();
        let b = InstanceConfig::from_toml(&text).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn rejects_blank_name_and_zero_port() {
        let invalid = InstanceConfig {
            name: "  ".into(),
            ..sample()
        };
        assert!(invalid.to_toml().is_ok());
        let text = invalid.to_toml().unwrap();
        assert!(InstanceConfig::from_toml(&text).is_err());

        let invalid = InstanceConfig {
            port: 0,
            ..sample()
        };
        assert!(InstanceConfig::from_toml(invalid.to_toml().unwrap().as_str()).is_err());
    }

    #[test]
    fn minimal_file_parses_with_defaults() {
        let text = r#"
id = "i-002"
name = "Van"
server_type = "vanilla"
version = "1.21.1"
java_major = 21
port = 25566
created_ms = 7
"#;
        let cfg = InstanceConfig::from_toml(text).unwrap();
        assert_eq!(cfg.heap_profile, HeapProfile::Auto);
        assert!(!cfg.eula_accepted);
    }
}
