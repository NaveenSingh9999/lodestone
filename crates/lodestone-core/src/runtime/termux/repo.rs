//! Parsing of apt repository metadata: the (clearsigned) InRelease file and
//! the Packages index.

use std::collections::BTreeMap;

use super::version::version_cmp;

/// Header of an apt `InRelease`/`Release` file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReleaseMeta {
    pub origin: String,
    pub label: String,
    pub suite: String,
    pub codename: String,
    pub date: String,
    pub architectures: Vec<String>,
    pub components: Vec<String>,
}

impl ReleaseMeta {
    /// Parse the header portion of InRelease (stops at empty line or the SHA
    /// section). Must only receive the *cleartext* portion of an InRelease.
    pub fn parse(cleartext: &str) -> Result<Self, String> {
        let mut m = ReleaseMeta::default();
        for line in cleartext.lines() {
            if line.is_empty() || line.starts_with("-----BEGIN") {
                break;
            }
            let Some((raw_key, raw_val)) = line.split_once(": ") else {
                continue;
            };
            match raw_key {
                "Origin" => m.origin = raw_val.trim().to_string(),
                "Label" => m.label = raw_val.trim().to_string(),
                "Suite" => m.suite = raw_val.trim().to_string(),
                "Codename" => m.codename = raw_val.trim().to_string(),
                "Date" => m.date = raw_val.trim().to_string(),
                "Architectures" => {
                    m.architectures = raw_val.split_whitespace().map(String::from).collect()
                }
                "Components" => {
                    m.components = raw_val.split_whitespace().map(String::from).collect()
                }
                _ => {}
            }
        }
        if m.components.is_empty() || m.architectures.is_empty() {
            return Err("InRelease header lacks Components or Architectures".to_string());
        }
        Ok(m)
    }
}

/// One `SHA256:` digest line of the Release file. `size` line semantics: the
/// size category precedes the path in apt Release files (`sha path size`).
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseDigest {
    pub size: u64,
    pub path: String,
    pub sha256: String,
}

/// All SHA256 digests declared by an apt Release.
pub fn parse_sha256_section(cleartext: &str) -> Result<Vec<ReleaseDigest>, String> {
    let mut out = Vec::new();
    let mut in_sha256 = false;
    for line in cleartext.lines() {
        if line.starts_with("SHA256:") {
            in_sha256 = true;
            continue;
        }
        if in_sha256 {
            if line.is_empty() {
                break;
            }
            if !line.starts_with(' ') {
                in_sha256 = false;
                continue;
            }
            let mut it = line.split_whitespace();
            let (size, path, sha) = (it.next(), it.next(), it.next());
            let (size, path, sha) = match (size, path, sha) {
                (Some(s), Some(p), Some(x)) if x.len() == 64 => (s, p, x),
                _ => return Err(format!("malformed SHA256 line: {line:?}")),
            };
            out.push(ReleaseDigest {
                size: size
                    .parse::<u64>()
                    .map_err(|_| format!("bad size in {line:?}"))?,
                path: path.to_string(),
                sha256: sha.to_lowercase(),
            });
        }
    }
    if out.is_empty() {
        return Err("no SHA256 digest lines found in Release".to_string());
    }
    Ok(out)
}

/// A dependency atom: `name`, optionally with a version constraint.
#[derive(Debug, Clone, PartialEq)]
pub struct DepAtom {
    pub name: String,
    pub constraint: Option<(String, String)>, // (op, version)
}

impl DepAtom {
    pub fn is_satisfied(&self, have: &str) -> bool {
        match &self.constraint {
            None => true,
            Some((op, want)) => match op.as_str() {
                "=" => version_cmp(have, want) == std::cmp::Ordering::Equal,
                "==" => version_cmp(have, want) == std::cmp::Ordering::Equal,
                ">=" => version_cmp(have, want) != std::cmp::Ordering::Less,
                "<=" => version_cmp(have, want) != std::cmp::Ordering::Greater,
                ">" => version_cmp(have, want) == std::cmp::Ordering::Greater,
                "<" => version_cmp(have, want) == std::cmp::Ordering::Less,
                _ => true, // unknown op: treat as satisfied
            },
        }
    }

    /// Parse “name (>= 1.2)” into an atom.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        // Strip [arch list] qualifiers.
        let head = raw.split_whitespace().next().unwrap_or("");
        if head.is_empty() || head.starts_with('[') {
            return None;
        }
        if let Some((idx, _)) = raw.char_indices().find(|&(_, c)| c == '(') {
            let body = raw[idx + 1..].split(')').next().unwrap_or("").trim();
            let mut parts = body.splitn(2, char::is_whitespace);
            let op = parts.next().unwrap_or("").trim().to_string();
            let version = parts.next().unwrap_or("").trim().to_string();
            return Some(DepAtom {
                name: head.to_string(),
                constraint: Some((op, version)),
            });
        }
        Some(DepAtom {
            name: head.to_string(),
            constraint: None,
        })
    }
}

/// A single stanza from the Packages index.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageEntry {
    pub package: String,
    pub version: String,
    pub architecture: String,
    /// Relative filename inside the pool. Verification-critical; absent on
    /// minimal stanzas, present on real repository data.
    pub filename: Option<String>,
    pub size: Option<u64>,
    /// Lowercase SHA256. Verification-critical; absent on minimal stanzas.
    pub sha256: Option<String>,
    pub depends: Vec<Vec<DepAtom>>, // OR-alternatives grouped per comma item
    pub component: String,
}

impl PackageEntry {
    pub fn parse(block: &str, component: &str) -> Option<Self> {
        let mut fields: BTreeMap<String, String> = BTreeMap::new();
        for line in block.lines() {
            let Some((k, v)) = line.split_once(": ") else {
                continue;
            };
            fields.insert(k.trim().to_string(), v.trim().to_string());
        }
        let package = fields.get("Package")?.clone();
        let version = fields.get("Version")?.clone();
        let filename = fields.get("Filename").cloned();
        let sha256 = fields.get("SHA256").map(|s| s.to_lowercase());
        let size = fields.get("Size").and_then(|s| s.parse::<u64>().ok());
        let architecture = fields.get("Architecture").cloned().unwrap_or_default();
        let depends = fields
            .get("Depends")
            .map(|d| split_depends(d))
            .unwrap_or_default();
        Some(PackageEntry {
            package,
            version,
            architecture,
            filename,
            size,
            sha256,
            depends,
            component: component.to_string(),
        })
    }

    /// The URL of the package inside the pool as `${base}/${filename}`.
    pub fn pool_url(&self, base: &str) -> Result<String, String> {
        self.filename
            .as_deref()
            .map(|f| format!("{}/{}", base.trim_end_matches('/'), f))
            .ok_or_else(|| format!("package {} has no Filename field", self.package))
    }
}

/// Parse a `Depends:` value into comma groups; each comma item is an
/// OR-list of alternatives.
fn split_depends(value: &str) -> Vec<Vec<DepAtom>> {
    let mut out = Vec::new();
    for item in value.split(',') {
        let mut alts = Vec::new();
        for alt in item.split('|') {
            if let Some(atom) = DepAtom::parse(alt) {
                alts.push(atom);
            }
        }
        if !alts.is_empty() {
            out.push(alts);
        }
    }
    out
}

/// All parser-consumable packages of one Packages index document.
#[derive(Debug, Clone, Default)]
pub struct PackageIndex {
    pub by_name: BTreeMap<String, PackageEntry>,
}

impl PackageIndex {
    /// Parse the whole document. Later blocks for the same package are ignored
    /// in favor of the first one (mirroring apt's behavior in a sense: versions
    /// come earlier in apt ordering, keeping it deterministic).
    pub fn parse(text: &str, component: &str) -> Self {
        let mut idx = PackageIndex::default();
        for block in text.split("\n\n") {
            if let Some(entry) = PackageEntry::parse(block, component) {
                idx.by_name.entry(entry.package.clone()).or_insert(entry);
            }
        }
        idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Package: openjdk-21
Version: 21.0.13
Architecture: aarch64
Depends: libandroid-shmem, libandroid-spawn, libiconv (>= 1.17), alsa-plugins | sndio
Filename: pool/main/o/openjdk-21/openjdk-21_21.0.13_aarch64.deb
Size: 106105996
SHA256: 5E5BFF2252236B30339A68739834EC98BAF858F19905F9FAE87198BF5B117CB2

Package: ca-certificates-java
Version: 1:2026.10.20-1
Architecture: all
Depends: ca-certificates (= 1:2026.10.20-1)
Filename: pool/main/c/ca-certificates-java/ca-certificates-java_1:2026.10.20-1_all.deb
Size: 95020
SHA256: 3CE94C49C9D77FBB4DB097E434B4117B267AD48EE5EDE512E69EEADAF47AC6F4
";

    #[test]
    fn parses_stanzas() {
        let idx = PackageIndex::parse(SAMPLE, "main");
        assert_eq!(idx.by_name.len(), 2);
        let jdk = &idx.by_name["openjdk-21"];
        assert_eq!(jdk.version, "21.0.13");
        assert_eq!(jdk.size, Some(106105996));
        assert_eq!(
            jdk.sha256,
            Some("5e5bff2252236b30339a68739834ec98baf858f19905f9fae87198bf5b117cb2".to_string())
        );
        assert_eq!(jdk.depends.len(), 4);
        // OR alternatives survive parsing
        assert_eq!(jdk.depends[3].len(), 2);
        assert_eq!(jdk.depends[3][0].name, "alsa-plugins");
        assert_eq!(jdk.depends[3][1].name, "sndio");
        let ca = &idx.by_name["ca-certificates-java"];
        assert_eq!(ca.depends[0][0].name, "ca-certificates");
        assert_eq!(
            ca.depends[0][0].constraint,
            Some(("=".to_string(), "1:2026.10.20-1".to_string()))
        );
    }

    #[test]
    fn dep_atom_satisfaction() {
        let a = DepAtom {
            name: "libiconv".into(),
            constraint: Some((">=".into(), "1.17".into())),
        };
        assert!(a.is_satisfied("1.17"));
        assert!(a.is_satisfied("1.18"));
        assert!(!a.is_satisfied("1.16"));
        let pinned = DepAtom {
            name: "ca-certificates".into(),
            constraint: Some(("=".into(), "1:2026.10.20-1".into())),
        };
        assert!(pinned.is_satisfied("1:2026.10.20-1"));
        assert!(!pinned.is_satisfied("1:2026.10.21-1"));
    }

    #[test]
    fn release_meta_and_digests() {
        let header = "Origin: termux-main stable\nSuite: stable\nComponents: main\nArchitectures: aarch64 arm\n\nSHA256:\n 1871832 main/binary-aarch64/Packages 1a2b3c4d5e6f77889900aabbccddeeff112233445566778899aabbccddeeff11\n  729678 main/binary-aarch64/Packages.gz 2a2b3c4d5e6f77889900aabbccddeeff112233445566778899aabbccddeeff22\n";
        let meta = ReleaseMeta::parse(header).unwrap();
        assert_eq!(meta.components, vec!["main"]);
        assert!(meta.architectures.contains(&"aarch64".to_string()));
        let digs = parse_sha256_section(header).unwrap();
        assert_eq!(digs.len(), 2);
        assert_eq!(digs[0].size, 1871832);
        assert!(digs[0].path.ends_with("main/binary-aarch64/Packages"));
        assert!(digs[1].sha256.starts_with("2a2b3c"));
    }

    #[test]
    fn pool_url() {
        let idx = PackageIndex::parse(SAMPLE, "main");
        let e = &idx.by_name["ca-certificates-java"];
        assert_eq!(
            e.pool_url("https://packages.termux.dev/apt/termux-main")
                .unwrap(),
            "https://packages.termux.dev/apt/termux-main/pool/main/c/ca-certificates-java/ca-certificates-java_1:2026.10.20-1_all.deb"
        );
        let bare = PackageIndex::parse("Package: a\nVersion: 1\n", "main")
            .by_name
            .remove("a")
            .unwrap();
        assert!(bare.pool_url("https://x.y").is_err());
    }
}
