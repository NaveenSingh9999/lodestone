//! Orchestration of Termux runtime provisioning (spec §4.2): verify repo →
//! fetch index → resolve closure → fetch/verify/extract every package →
//! place the truststore → report launch facts.
//!
//! Everything is hash-verified against repository data that is itself
//! signature-verified against embedded Termux keys before use.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::RuntimeError;
use crate::net::HttpClient;

use super::deb;
use super::keys::embedded_public_keys;
use super::repo::{ReleaseMeta, parse_sha256_section};
use super::resolve;
use super::verify::verify_inrelease;

/// Base URL of the Termux main apt repository. packages.termux.org redirects
/// to packages.termux.dev; both hosts are in the spec allowlist, so either is
/// usable. The redirect chain is validated by the client.
pub const DEFAULT_TERMUX_BASE: &str = "https://packages.termux.org/apt/termux-main";
pub const SUITE_LABEL: &str = "stable";
pub const COMPONENT: &str = "main";

/// Persistent record of a provisioned runtime.
#[derive(Debug, Serialize, Deserialize)]
pub struct RuntimeMeta {
    pub name: String,
    pub java_major: u32,
    pub base_url: String,
    pub suite: String,
    pub component: String,
    pub arch: String,
    pub created_unix: u64,
    pub inrelease_verified_by: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<PackageRecord>,
    pub java_home_rel: String,
    pub truststore_path_rel: String,
    pub ld_library_path_rel: Vec<String>,
    /// Page-size contract the runtime tree was installed under.
    pub page_size_requires: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SkippedEntry {
    pub package: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageRecord {
    pub package: String,
    pub version: String,
    pub sha256: String,
    pub size: u64,
}

/// Facts the launcher (spec §4.3) needs to start a Java process on Android.
#[derive(Debug, Clone)]
pub struct Bootstrap {
    pub java_home: PathBuf,
    pub truststore: PathBuf,
    pub ld_library_path: Vec<PathBuf>,
    pub extra_java_props: Vec<(String, String)>,
}

pub enum ProvisionState {
    AlreadyInstalled(RuntimeMeta),
    Installed(RuntimeMeta),
}

pub fn runtime_dir_name(java_major: u32) -> String {
    format!("openjdk-{java_major}")
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

fn file_sha256(path: &Path) -> Result<String, RuntimeError> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path)
        .map_err(|e| RuntimeError::Provision(format!("open {}: {e}", path.display())))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 1 << 16];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf)
            .map_err(|e| RuntimeError::Provision(format!("hash read: {e}")))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

pub struct TermuxProvisioner {
    pub base_url: String,
    pub arch: String,
}

impl TermuxProvisioner {
    pub fn standard(arch: impl Into<String>) -> Self {
        TermuxProvisioner {
            base_url: DEFAULT_TERMUX_BASE.to_string(),
            arch: arch.into(),
        }
    }

    pub async fn ensure_runtime<H: HttpClient>(
        &self,
        client: &H,
        runtimes_root: &Path,
        java_major: u32,
    ) -> Result<ProvisionState, RuntimeError> {
        let runtime_root = runtimes_root.join(runtime_dir_name(java_major));
        let meta = read_meta(&runtime_root.join("meta.toml"))?;
        if let Some(m) = meta {
            let java_bin = runtime_root.join(&m.java_home_rel).join("bin/java");
            if m.java_major == java_major && m.arch == self.arch && java_bin.exists() {
                return Ok(ProvisionState::AlreadyInstalled(m));
            }
        }
        let meta = self.provision(client, runtimes_root, java_major).await?;
        Ok(ProvisionState::Installed(meta))
    }

    async fn provision<H: HttpClient>(
        &self,
        client: &H,
        runtimes_root: &Path,
        java_major: u32,
    ) -> Result<RuntimeMeta, RuntimeError> {
        let runtime_root = runtimes_root.join(runtime_dir_name(java_major));

        // Fresh extraction is mandatory: a tree without (or with stale) meta
        // must not be upgraded in place.
        if runtime_root.exists() {
            std::fs::remove_dir_all(&runtime_root).map_err(|e| {
                RuntimeError::Provision(format!("wipe {}: {e}", runtime_root.display()))
            })?;
        }
        std::fs::create_dir_all(&runtime_root).map_err(|e| {
            RuntimeError::Provision(format!("mkdir {}: {e}", runtime_root.display()))
        })?;

        // 1. InRelease (signature-verified) --------------------------------
        let inrelease_url = format!(
            "{}/dists/{}/InRelease",
            self.base_url.trim_end_matches('/'),
            SUITE_LABEL
        );
        let armored = client
            .get(&inrelease_url)
            .await
            .map_err(|e| RuntimeError::Provision(format!("fetch InRelease: {e}")))?;
        let armored_text = String::from_utf8(armored)
            .map_err(|e| RuntimeError::Provision(format!("InRelease utf8: {e}")))?;
        let keys =
            embedded_public_keys().map_err(|e| RuntimeError::Provision(format!("keyring: {e}")))?;
        verify_inrelease(&armored_text, &keys, SystemTime::now())
            .map_err(|e| RuntimeError::Provision(format!("InRelease signature: {e}")))?;

        let hdr = ReleaseMeta::parse(&armored_text).map_err(RuntimeError::Provision)?;
        if !hdr.components.iter().any(|c| c == COMPONENT) {
            return Err(RuntimeError::Provision(format!(
                "InRelease lists Components {Components:?}; missing '{COMPONENT}'",
                Components = hdr.components
            )));
        }

        // 2. Packages index (hash-verified against the signed Release) -----
        let digests = parse_sha256_section(&armored_text).map_err(RuntimeError::Provision)?;
        let want_gz = format!("{COMPONENT}/binary-{}/Packages.gz", self.arch);
        let want_plain = format!("{COMPONENT}/binary-{}/Packages", self.arch);
        let pick = digests
            .iter()
            .find(|d| d.path == want_gz)
            .or_else(|| digests.iter().find(|d| d.path == want_plain))
            .ok_or_else(|| {
                RuntimeError::Provision(format!(
                    "{want_gz} (or plain Packages) not listed in the InRelease SHA256 section"
                ))
            })?;
        let index_url = format!(
            "{}/dists/{}/{}",
            self.base_url.trim_end_matches('/'),
            SUITE_LABEL,
            pick.path
        );
        let payload = client
            .get(&index_url)
            .await
            .map_err(|e| RuntimeError::Provision(format!("fetch Packages: {e}")))?;
        let raw = if pick.path.ends_with(".gz") {
            let mut dec = Vec::new();
            use std::io::Read;
            flate2::read::MultiGzDecoder::new(payload.as_slice())
                .read_to_end(&mut dec)
                .map_err(|e| RuntimeError::Provision(format!("gunzip Packages: {e}")))?;
            dec
        } else {
            payload
        };
        let got = sha256_hex(&raw);
        if got != pick.sha256 {
            return Err(RuntimeError::Provision(format!(
                "Packages sha256 mismatch: expected {}, got {}",
                pick.sha256, got
            )));
        }
        let index_text = String::from_utf8(raw)
            .map_err(|e| RuntimeError::Provision(format!("Packages utf8: {e}")))?;
        let index = super::repo::PackageIndex::parse(&index_text, COMPONENT);

        // 3. Resolve the dependency closure ---------------------------------
        let roots: Vec<&str> = vec![
            Box::leak(format!("openjdk-{java_major}").into_boxed_str()),
            "ca-certificates-java",
        ];
        let plan = resolve::resolve(&index.by_name, &roots)
            .map_err(|e| RuntimeError::Provision(e.message))?;

        // 4. Download every package and hash-verify against the index -------
        let cache_dir = runtimes_root.join(".cache-deb");
        std::fs::create_dir_all(&cache_dir).ok();
        let mut records = Vec::new();
        for entry in plan.plan.iter() {
            // Verification-critical fields must exist for any installable plan
            let sha_expected = entry.sha256.clone().ok_or_else(|| {
                RuntimeError::Provision(format!(
                    "package {} has no SHA256 in the index; refusing an unverified download",
                    entry.package
                ))
            })?;
            let size = entry.size.ok_or_else(|| {
                RuntimeError::Provision(format!(
                    "package {} has no Size in the index",
                    entry.package
                ))
            })?;
            let url = entry
                .pool_url(&self.base_url)
                .map_err(RuntimeError::Provision)?;
            let fname = entry.filename.as_deref().ok_or_else(|| {
                RuntimeError::Provision(format!("package {} has no Filename", entry.package))
            })?;
            let deb_path = cache_dir.join(deb_file_name(fname));
            if !candidate_is_fresh(&sha_expected, size, &deb_path)? {
                client.download(&url, &deb_path).await.map_err(|e| {
                    RuntimeError::Provision(format!("fetch {}: {e}", entry.package))
                })?;
            }
            let got = file_sha256(&deb_path)?;
            if got != sha_expected {
                std::fs::remove_file(&deb_path).ok();
                return Err(RuntimeError::Provision(format!(
                    "{} sha256 mismatch: expected {}, got {}",
                    entry.package, sha_expected, got
                )));
            }
            records.push(PackageRecord {
                package: entry.package.clone(),
                version: entry.version.clone(),
                sha256: sha_expected,
                size,
            });
        }

        // 5. Extract (only after all downloads verified) --------------------
        for entry in plan.plan.iter() {
            let name = entry
                .filename
                .as_deref()
                .map(deb_file_name)
                .ok_or_else(|| {
                    RuntimeError::Provision(format!("package {} has no Filename", entry.package))
                })?;
            let deb_path = cache_dir.join(name);
            deb::extract_deb(&deb_path, &runtime_root)
                .map_err(|e| RuntimeError::Provision(format!("extract {}: {e}", entry.package)))?;
        }
        // The deb cache is disposable once extracted
        std::fs::remove_dir_all(&cache_dir).ok();

        // 6. Locate java home and the TLS truststore ------------------------
        let java_home = locate_java_home(&runtime_root, java_major).ok_or_else(|| {
            RuntimeError::Provision(format!(
                "lib/jvm/java-{java_major}-openjdk/bin/java not found after extraction"
            ))
        })?;
        let jsse = locate_jssecacerts(&runtime_root).ok_or_else(|| {
            RuntimeError::Provision("jssecacerts missing after extraction".to_string())
        })?;
        let trust_target = java_home.join("lib/security/jssecacerts");
        if trust_target != jsse {
            if let Some(p) = trust_target.parent() {
                std::fs::create_dir_all(p)
                    .map_err(|e| RuntimeError::Provision(format!("mkdir {}: {e}", p.display())))?;
            }
            std::fs::copy(&jsse, &trust_target)
                .map_err(|e| RuntimeError::Provision(format!("copy jssecacerts: {e}")))?;
        }

        // 7. Persist meta ---------------------------------------------------
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let java_home_rel = strip_prefix_string(&runtime_root, &java_home)
            .ok_or_else(|| RuntimeError::Provision("java home outside runtime".to_string()))?;
        let trust_rel = strip_prefix_string(&runtime_root, &trust_target)
            .ok_or_else(|| RuntimeError::Provision("truststore outside runtime".to_string()))?;
        let meta = RuntimeMeta {
            name: runtime_dir_name(java_major),
            java_major,
            base_url: self.base_url.clone(),
            suite: SUITE_LABEL.to_string(),
            component: COMPONENT.to_string(),
            arch: self.arch.clone(),
            created_unix: created,
            inrelease_verified_by: "embedded-termux-keyring".to_string(),
            skipped: plan
                .skipped
                .iter()
                .map(|(p, r)| SkippedEntry {
                    package: p.clone(),
                    reason: r.clone(),
                })
                .collect(),
            packages: records,
            java_home_rel,
            truststore_path_rel: trust_rel,
            ld_library_path_rel: vec!["lib".to_string()],
            page_size_requires: super::super::device_page_size().unwrap_or(16384),
        };
        write_meta_atomic(
            &runtime_root.join("meta.toml"),
            &toml::to_string(&meta)
                .map_err(|e| RuntimeError::Provision(format!("meta toml: {e}")))?,
        )?;
        Ok(meta)
    }
}

fn deb_file_name(filename: &str) -> String {
    filename.rsplit('/').next().unwrap_or(filename).to_string()
}

/// A cached deb is reusable only if size AND hash match exactly.
fn candidate_is_fresh(
    sha_expected: &str,
    size: u64,
    deb_path: &Path,
) -> Result<bool, RuntimeError> {
    let Ok(md) = std::fs::metadata(deb_path) else {
        return Ok(false);
    };
    if md.len() != size {
        return Ok(false);
    }
    Ok(file_sha256(deb_path)
        .map(|h| h == sha_expected)
        .unwrap_or(false))
}

fn strip_prefix_string(root: &Path, target: &Path) -> Option<String> {
    let rel = target.strip_prefix(root).ok()?;
    let s = rel.to_path_buf().to_string_lossy().into_owned();
    if s.is_empty() { None } else { Some(s) }
}

fn locate_java_home(runtime_root: &Path, java_major: u32) -> Option<PathBuf> {
    let exact = runtime_root
        .join("lib/jvm")
        .join(format!("java-{java_major}-openjdk"));
    if exact.join("bin/java").exists() {
        return Some(exact);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(runtime_root.join("lib/jvm")).ok()? {
        let entry = entry.ok()?;
        candidates.push(entry.path());
    }
    candidates.sort();
    candidates.into_iter().find(|p| p.join("bin/java").exists())
}

/// Depth-bounded search for the first `jssecacerts` in the runtime tree.
fn locate_jssecacerts(runtime_root: &Path) -> Option<PathBuf> {
    const MAX_DEPTH: usize = 12;
    let mut stack = vec![(runtime_root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let read = std::fs::read_dir(&dir).ok()?;
        let mut entries: Vec<PathBuf> = Vec::new();
        for e in read.flatten() {
            entries.push(e.path());
        }
        entries.sort();
        for path in entries {
            let Ok(md) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if md.is_file()
                && path
                    .file_name()
                    .map(|n| n == "jssecacerts")
                    .unwrap_or(false)
            {
                return Some(path);
            }
            if md.is_dir() && depth + 1 < MAX_DEPTH {
                stack.push((path, depth + 1));
            }
        }
    }
    None
}

fn write_meta_atomic(path: &Path, data: &str) -> Result<(), RuntimeError> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)
            .map_err(|e| RuntimeError::Provision(format!("mkdir {}: {e}", p.display())))?;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = path.with_extension(format!("toml-{nanos}.tmp"));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| RuntimeError::Provision(format!("create meta: {e}")))?;
        f.write_all(data.as_bytes())
            .and_then(|_| f.flush())
            .and_then(|_| f.sync_all())
            .map_err(|e| RuntimeError::Provision(format!("write meta: {e}")))?;
    }
    std::fs::rename(&tmp, path)
        .map_err(|e| RuntimeError::Provision(format!("rename meta: {e}")))?;
    Ok(())
}

fn read_meta(path: &Path) -> Result<Option<RuntimeMeta>, RuntimeError> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let meta: RuntimeMeta = toml::from_slice(&bytes)
                .map_err(|e| RuntimeError::Provision(format!("meta parse: {e}")))?;
            Ok(Some(meta))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(RuntimeError::Provision(format!("meta read: {e}"))),
    }
}

/// Launch facts derived from a provisioned runtime, for the §4.3 harness.
pub fn bootstrap_from_meta(
    runtime_root: &Path,
    meta: &RuntimeMeta,
) -> Result<Bootstrap, RuntimeError> {
    let java_home = runtime_root.join(&meta.java_home_rel);
    let truststore = runtime_root.join(&meta.truststore_path_rel);
    if !java_home.join("bin/java").exists() {
        return Err(RuntimeError::Provision(format!(
            "java_home {} missing bin/java",
            java_home.display()
        )));
    }
    let mut ld: Vec<PathBuf> = meta
        .ld_library_path_rel
        .iter()
        .map(|r| runtime_root.join(r))
        .collect();
    let jli = java_home.join("lib/jli");
    if jli.exists() {
        ld.push(jli);
    }
    let truststore_str = truststore.to_string_lossy().into_owned();
    Ok(Bootstrap {
        java_home,
        truststore,
        ld_library_path: ld,
        extra_java_props: vec![
            ("javax.net.ssl.trustStore".to_string(), truststore_str),
            (
                "javax.net.ssl.trustStoreType".to_string(),
                "JKS".to_string(),
            ),
        ],
    })
}

impl ProvisionState {
    pub fn into_meta(self) -> RuntimeMeta {
        match self {
            ProvisionState::AlreadyInstalled(m) | ProvisionState::Installed(m) => m,
        }
    }
}
