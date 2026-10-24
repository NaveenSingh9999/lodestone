//! Dependency-closure resolution over a Termux Packages index.
//!
//! Policy (per the Lodestone spec, provisioning §4.2):
//! - the JDK package's transitive `Depends` closure is installed;
//! - `ca-certificates-java` is always pulled in (bootstraps a truststore);
//! - tooling-only / audio-only packages are skipped, with names recorded in
//!   the runtime metadata for auditability;
//! - OR dependencies pick the first alternative present in the index;
//! - version constraints are verified strictly — a pinned dependency like
//!   `ca-certificates (= 1:2026.10.20-1)` must resolve against the index.

use std::collections::BTreeMap;

use super::repo::{DepAtom, PackageEntry};

const SKIP_PACKAGES: &[(&str, &str)] = &[
    ("alsa-plugins", "servers never start audio sessions"),
    ("alsa-lib", "audio-only dependency of alsa-plugins"),
    ("pulseaudio", "audio-only dependency of alsa-plugins"),
    ("resolv-conf", "Android resolves DNS natively"),
];

fn skip_reason(name: &str) -> Option<&'static str> {
    SKIP_PACKAGES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, reason)| *reason)
}

#[derive(Debug)]
pub struct ResolveError {
    pub message: String,
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug)]
pub struct ResolveResult {
    /// Packages to install, dependencies first.
    pub plan: Vec<PackageEntry>,
    /// Skipped packages with human reasons (goes into runtime metadata).
    pub skipped: Vec<(String, String)>,
    /// chosen package name -> chosen version.
    pub chosen: BTreeMap<String, String>,
}

/// Computes the dependency closure of `roots` over `index`.
pub fn resolve(
    index: &BTreeMap<String, PackageEntry>,
    roots: &[&str],
) -> Result<ResolveResult, ResolveError> {
    let mut visited: BTreeMap<String, String> = BTreeMap::new();
    let mut plan = Vec::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut chosen: BTreeMap<String, String> = BTreeMap::new();

    for name in roots {
        if skip_reason(name).is_some() {
            skipped.push((
                (*name).to_string(),
                skip_reason(name).unwrap_or("").to_string(),
            ));
            continue;
        }
        visit(
            index,
            name,
            &mut visited,
            &mut plan,
            &mut skipped,
            &mut chosen,
        )?;
    }

    Ok(ResolveResult {
        plan,
        skipped,
        chosen,
    })
}

fn visit(
    index: &BTreeMap<String, PackageEntry>,
    name: &str,
    visited: &mut BTreeMap<String, String>,
    plan: &mut Vec<PackageEntry>,
    skipped: &mut Vec<(String, String)>,
    chosen: &mut BTreeMap<String, String>,
) -> Result<(), ResolveError> {
    if let Some(v) = visited.get(name) {
        let _ = v;
        return Ok(());
    }
    if let Some(reason) = skip_reason(name) {
        visited.insert(name.to_string(), "(skipped)".to_string());
        skipped.push((name.to_string(), reason.to_string()));
        return Ok(());
    }

    let entry = index.get(name).ok_or_else(|| ResolveError {
        message: format!("package {name} is not present in the repository index"),
    })?;

    // Absent on cycle handling: apt has no cycles here; mark visited before
    // recursing so accidental repeats don't loop forever.
    visited.insert(name.to_string(), entry.version.clone());
    chosen.insert(name.to_string(), entry.version.clone());

    for group in &entry.depends {
        let atom = pick(index, group).ok_or_else(|| ResolveError {
            message: format!(
                "no alternative of dependency of {name} is in the index: {}",
                group.iter().map(dep_label).collect::<Vec<_>>().join(" | ")
            ),
        })?;

        // consistency: must agree with an already-chosen version
        if let Some(prev) = chosen.get(&atom.name)
            && !atom.is_satisfied(prev)
        {
            return Err(ResolveError {
                message: format!(
                    "version conflict while resolving {name}: {atom_name} is pinned to {} but {prev} was chosen",
                    atom.constraint
                        .as_ref()
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default(),
                    atom_name = atom.name
                ),
            });
        }
        if let Some(reason) = skip_reason(&atom.name) {
            if !visited.contains_key(&atom.name) {
                visited.insert(atom.name.clone(), "(skipped)".to_string());
            }
            skipped.push((atom.name.clone(), reason.to_string()));
            continue;
        }
        visit(index, &atom.name, visited, plan, skipped, chosen)?;
    }

    plan.push(entry.clone());
    Ok(())
}

fn dep_label(a: &DepAtom) -> String {
    match &a.constraint {
        Some((op, v)) => format!("{a_name} ({op} {v})", a_name = a.name),
        None => a.name.clone(),
    }
}

/// Choose, among a comma-item's OR alternatives, the first that exists in the
/// index and satisfies its own version constraint (when any).
fn pick(index: &BTreeMap<String, PackageEntry>, group: &[DepAtom]) -> Option<DepAtom> {
    let mut first_missing: Option<&DepAtom> = None;
    for atom in group {
        match index.get(&atom.name) {
            None => {
                if first_missing.is_none() {
                    first_missing = Some(atom);
                }
            }
            Some(entry) => {
                if atom.is_satisfied(&entry.version) {
                    return Some(atom.clone());
                }
            }
        }
    }
    // no satisfiable alternative: report the first missing atom's name so
    // callers can speak of a concrete package name in errors
    first_missing.cloned()
}

#[cfg(test)]
mod tests {
    use super::super::repo::PackageIndex;
    use super::*;

    fn index_from(text: &str) -> BTreeMap<String, PackageEntry> {
        PackageIndex::parse(text, "main").by_name
    }

    #[test]
    fn resolves_flat_chain() {
        let idx = index_from(
            "Package: a\nVersion: 1\nDepends: b\n\nPackage: b\nVersion: 1\nDepends: c\n\nPackage: c\nVersion: 1\n",
        );
        let r = resolve(&idx, &["a"]).unwrap();
        let names: Vec<&str> = r.plan.iter().map(|p| p.package.as_str()).collect();
        assert_eq!(names, vec!["c", "b", "a"]);
    }

    #[test]
    fn skips_audio_stack() {
        let idx = index_from(
            "Package: a\nVersion: 1\nDepends: alsa-plugins, libx\n\nPackage: alsa-plugins\nVersion: 1\n\nPackage: libx\nVersion: 1\n",
        );
        let r = resolve(&idx, &["a"]).unwrap();
        let names: Vec<&str> = r.plan.iter().map(|p| p.package.as_str()).collect();
        assert_eq!(names, vec!["libx", "a"]);
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.skipped[0].0, "alsa-plugins");
    }

    #[test]
    fn missing_package_is_error() {
        let idx = index_from("Package: a\nVersion: 1\nDepends: nothere\n\n");
        let e = resolve(&idx, &["a"]).unwrap_err();
        assert!(e.message.contains("nothere"), "{e}");
    }

    #[test]
    fn or_dependency_picks_first_present() {
        let idx = index_from("Package: a\nVersion: 1\nDepends: x | y\n\nPackage: y\nVersion: 1\n");
        let r = resolve(&idx, &["a"]).unwrap();
        let names: Vec<&str> = r.plan.iter().map(|p| p.package.as_str()).collect();
        assert_eq!(names, vec!["y", "a"]);
    }

    #[test]
    fn pinned_constraint_passes_and_fails() {
        let ok =
            index_from("Package: a\nVersion: 1\nDepends: b (= 1.0)\n\nPackage: b\nVersion: 1.0\n");
        resolve(&ok, &["a"]).unwrap();

        let bad =
            index_from("Package: a\nVersion: 1\nDepends: b (= 2.0)\n\nPackage: b\nVersion: 1.0\n");
        let e = resolve(&bad, &["a"]).unwrap_err();
        assert!(e.message.contains("2.0"), "{e}");
    }
}
