//! grasm_pkg — manifests, dependency resolution, lockfiles, and the
//! capability ceiling that makes a dependency's reach checkable.
//!
//! The security idea (MASTERPLAN §9.1): a package declares the effects it is
//! allowed to perform. Because effects are inferred from source, a package
//! that quietly starts touching the filesystem or network exceeds its declared
//! ceiling and **fails to build in every consumer** — a compile error, not a
//! runtime surprise, and not something a reviewer has to notice.

pub mod hash;
pub mod toml;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const MANIFEST_NAME: &str = "grasm.toml";
pub const LOCKFILE_NAME: &str = "grasm.lock";

/// Effects the runtime provides; a package may always declare these.
pub const BUILTIN_CAPABILITIES: &[&str] = &["io", "async"];

#[derive(Debug, Clone, PartialEq)]
pub struct Dependency {
    pub name: String,
    /// Filesystem path, relative to the depending package's manifest.
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    /// The effect ceiling. `None` means "unrestricted" (with a lint);
    /// `Some(set)` is enforced against what the code actually does.
    pub capabilities: Option<BTreeSet<String>>,
    pub dependencies: Vec<Dependency>,
    /// Entry source file, relative to the manifest. Defaults to `src/main.gr`.
    pub entry: String,
    pub dir: PathBuf,
}

impl Manifest {
    pub fn parse(text: &str, dir: &Path) -> Result<Manifest, String> {
        let doc = toml::parse(text)?;
        let pkg = doc
            .table("package")
            .ok_or("manifest is missing a [package] table")?;

        let name = pkg
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("[package] is missing `name`")?
            .to_string();
        if name.is_empty() {
            return Err("[package] name must not be empty".into());
        }
        let version = pkg
            .get("version")
            .and_then(|v| v.as_str())
            .ok_or("[package] is missing `version`")?
            .to_string();
        validate_version(&version)?;

        let capabilities = match pkg.get("capabilities") {
            None => None,
            Some(v) => {
                let list = v
                    .as_str_array()
                    .ok_or("`capabilities` must be an array of strings")?;
                Some(list.into_iter().map(|s| s.to_string()).collect())
            }
        };

        let entry = pkg
            .get("entry")
            .and_then(|v| v.as_str())
            .unwrap_or("src/main.gr")
            .to_string();

        let mut dependencies = Vec::new();
        for (dep_name, table) in doc.subtables("dependencies") {
            let path = table
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    format!("dependency `{dep_name}` needs a `path` (registries are not implemented yet)")
                })?
                .to_string();
            dependencies.push(Dependency {
                name: dep_name,
                path,
            });
        }
        dependencies.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Manifest {
            name,
            version,
            capabilities,
            dependencies,
            entry,
            dir: dir.to_path_buf(),
        })
    }

    pub fn load(dir: &Path) -> Result<Manifest, String> {
        let path = dir.join(MANIFEST_NAME);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Manifest::parse(&text, dir)
    }

    pub fn entry_path(&self) -> PathBuf {
        self.dir.join(&self.entry)
    }
}

/// `MAJOR.MINOR.PATCH`, each a number.
fn validate_version(v: &str) -> Result<(), String> {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || p.parse::<u64>().is_err()) {
        return Err(format!(
            "version `{v}` is not `MAJOR.MINOR.PATCH` (e.g. `0.1.0`)"
        ));
    }
    Ok(())
}

/// A resolved dependency graph in topological order (dependencies first).
#[derive(Debug)]
pub struct Resolution {
    pub packages: Vec<Manifest>,
}

impl Resolution {
    pub fn root(&self) -> &Manifest {
        self.packages.last().expect("resolution is never empty")
    }
}

/// Walks the dependency graph depth-first, detecting cycles and missing
/// packages, and returns packages with dependencies before dependents.
pub fn resolve(root_dir: &Path) -> Result<Resolution, String> {
    let mut ordered: Vec<Manifest> = Vec::new();
    let mut visiting: Vec<String> = Vec::new();
    let mut done: BTreeMap<String, ()> = BTreeMap::new();
    visit(root_dir, &mut ordered, &mut visiting, &mut done)?;
    Ok(Resolution { packages: ordered })
}

fn visit(
    dir: &Path,
    ordered: &mut Vec<Manifest>,
    visiting: &mut Vec<String>,
    done: &mut BTreeMap<String, ()>,
) -> Result<(), String> {
    let manifest = Manifest::load(dir)?;

    if done.contains_key(&manifest.name) {
        return Ok(());
    }
    if visiting.contains(&manifest.name) {
        let mut cycle = visiting.clone();
        cycle.push(manifest.name.clone());
        return Err(format!(
            "dependency cycle: {}",
            cycle.join(" -> ")
        ));
    }
    visiting.push(manifest.name.clone());

    for dep in &manifest.dependencies {
        let dep_dir = manifest.dir.join(&dep.path);
        if !dep_dir.join(MANIFEST_NAME).exists() {
            return Err(format!(
                "dependency `{}` of `{}`: no {MANIFEST_NAME} at {}",
                dep.name,
                manifest.name,
                dep_dir.display()
            ));
        }
        let dep_manifest = Manifest::load(&dep_dir)?;
        if dep_manifest.name != dep.name {
            return Err(format!(
                "dependency `{}` of `{}` resolves to a package named `{}`",
                dep.name, manifest.name, dep_manifest.name
            ));
        }
        visit(&dep_dir, ordered, visiting, done)?;
    }

    visiting.pop();
    done.insert(manifest.name.clone(), ());
    ordered.push(manifest);
    Ok(())
}

// ---------------------------------------------------------------------------
// Capability enforcement
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityViolation {
    pub package: String,
    /// Effects the code performs that its manifest does not permit.
    pub exceeded: Vec<String>,
    pub declared: Vec<String>,
}

impl std::fmt::Display for CapabilityViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let exceeded = self
            .exceeded
            .iter()
            .map(|e| format!("`{e}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let declared = if self.declared.is_empty() {
            "nothing".to_string()
        } else {
            self.declared
                .iter()
                .map(|e| format!("`{e}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        write!(
            f,
            "package `{}` performs {exceeded} but its manifest only permits {declared}",
            self.package
        )
    }
}

/// Checks one package's actual effects against its declared ceiling.
///
/// `performed` is the set of effects inferred from the package's source.
pub fn check_capabilities(
    manifest: &Manifest,
    performed: &BTreeSet<String>,
) -> Option<CapabilityViolation> {
    let declared = manifest.capabilities.as_ref()?;
    let exceeded: Vec<String> = performed
        .iter()
        .filter(|e| !declared.contains(*e))
        .cloned()
        .collect();
    if exceeded.is_empty() {
        return None;
    }
    Some(CapabilityViolation {
        package: manifest.name.clone(),
        exceeded,
        declared: declared.iter().cloned().collect(),
    })
}

/// A capability a manifest declares that names no real effect.
#[derive(Debug, Clone, PartialEq)]
pub struct UnknownCapability {
    pub package: String,
    pub name: String,
    /// The closest real effect name, when one is close enough to be a typo.
    pub suggestion: Option<String>,
}

impl std::fmt::Display for UnknownCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "package `{}` declares capability `{}`, which is not an effect \
             this program defines",
            self.package, self.name
        )?;
        if let Some(s) = &self.suggestion {
            write!(f, " — did you mean `{s}`?")?;
        }
        Ok(())
    }
}

/// Checks that every declared capability names a real effect.
///
/// A ceiling is only meaningful if its names mean something. `capabilities =
/// ["Netwrok"]` grants nothing and denies nothing; it silently reads to a
/// human reviewer as network access while the compiler sees an unknown word.
/// The manifest is a claim other people rely on, so it has to be checkable.
pub fn check_capability_names(
    manifest: &Manifest,
    vocabulary: &BTreeSet<String>,
) -> Vec<UnknownCapability> {
    let Some(declared) = manifest.capabilities.as_ref() else {
        return Vec::new();
    };
    declared
        .iter()
        .filter(|c| !vocabulary.contains(*c))
        .map(|c| UnknownCapability {
            package: manifest.name.clone(),
            name: c.clone(),
            suggestion: nearest(c, vocabulary),
        })
        .collect()
}

/// The closest vocabulary entry within a small edit distance, for a typo hint.
fn nearest(word: &str, vocabulary: &BTreeSet<String>) -> Option<String> {
    vocabulary
        .iter()
        .map(|v| (edit_distance(word, v), v))
        .filter(|(d, v)| *d <= 2.max(v.len() / 3))
        .min_by_key(|(d, _)| *d)
        .map(|(_, v)| v.clone())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Effects performed by a package's own source files (not its dependencies').
pub fn effects_of_package(manifest: &Manifest) -> Result<BTreeSet<String>, String> {
    let entry = manifest.entry_path();
    if !entry.exists() {
        return Err(format!(
            "package `{}`: entry file {} does not exist",
            manifest.name,
            entry.display()
        ));
    }
    let program = grasm_sema::loader::load_from_disk(&entry)?;
    Ok(grasm_sema::effects::effects_of_program(&program))
}

/// The effect names a package's sources make available, for name checking.
pub fn effect_vocabulary_of_package(manifest: &Manifest) -> Result<BTreeSet<String>, String> {
    let entry = manifest.entry_path();
    if !entry.exists() {
        return Err(format!(
            "package `{}`: entry file {} does not exist",
            manifest.name,
            entry.display()
        ));
    }
    let program = grasm_sema::loader::load_from_disk(&entry)?;
    Ok(grasm_sema::effects::effect_vocabulary(&program))
}

// ---------------------------------------------------------------------------
// Lockfile
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct LockEntry {
    pub name: String,
    pub version: String,
    /// SHA-256 over the package's sorted source files.
    pub checksum: String,
    pub capabilities: Vec<String>,
}

/// Hashes every `.gr` file in a package, in sorted order, so the digest is
/// stable across filesystems.
pub fn package_checksum(manifest: &Manifest) -> Result<String, String> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_sources(&manifest.dir, &mut files)?;
    files.sort();
    let mut buf: Vec<u8> = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(&manifest.dir).unwrap_or(f);
        buf.extend(rel.to_string_lossy().as_bytes());
        buf.push(0);
        buf.extend(
            std::fs::read(f).map_err(|e| format!("cannot read {}: {e}", f.display()))?,
        );
        buf.push(0);
    }
    Ok(hash::sha256_hex(&buf))
}

fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read directory {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if path.is_dir() {
            collect_sources(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "gr") {
            out.push(path);
        }
    }
    Ok(())
}

pub fn build_lock(resolution: &Resolution) -> Result<Vec<LockEntry>, String> {
    let mut entries = Vec::new();
    for m in &resolution.packages {
        entries.push(LockEntry {
            name: m.name.clone(),
            version: m.version.clone(),
            checksum: package_checksum(m)?,
            capabilities: m
                .capabilities
                .as_ref()
                .map(|c| c.iter().cloned().collect())
                .unwrap_or_default(),
        });
    }
    Ok(entries)
}

pub fn render_lock(entries: &[LockEntry]) -> String {
    let mut out = String::from(
        "# Generated by `grasm build`. Commit this file.\n\
         # Checksums are SHA-256 over each package's sorted .gr sources.\n",
    );
    for e in entries {
        out.push_str("\n[[package]]\n");
        out.push_str(&format!("name = {}\n", toml::quote(&e.name)));
        out.push_str(&format!("version = {}\n", toml::quote(&e.version)));
        out.push_str(&format!("checksum = {}\n", toml::quote(&e.checksum)));
        let caps: Vec<String> = e.capabilities.iter().map(|c| toml::quote(c)).collect();
        out.push_str(&format!("capabilities = [{}]\n", caps.join(", ")));
    }
    out
}

/// How a lockfile on disk differs from what the source tree now hashes to.
#[derive(Debug, Clone, PartialEq)]
pub enum LockChange {
    /// A package's sources changed since the lockfile was written. This is the
    /// case a lockfile exists to catch: a dependency that is not what it was.
    Changed { name: String, was: String, now: String },
    /// A package the lockfile does not mention.
    Added(String),
    /// A package the lockfile mentions that is no longer in the graph.
    Removed(String),
    /// A package's declared capability ceiling changed — the security-relevant
    /// change, reported separately from a content change.
    CapabilitiesChanged {
        name: String,
        was: Vec<String>,
        now: Vec<String>,
    },
    Unreadable,
}

impl std::fmt::Display for LockChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockChange::Changed { name, was, now } => write!(
                f,
                "`{name}` contents changed since the lockfile ({}… → {}…)",
                &was[..was.len().min(12)],
                &now[..now.len().min(12)]
            ),
            LockChange::Added(name) => write!(f, "`{name}` is not in the lockfile"),
            LockChange::Removed(name) => {
                write!(f, "`{name}` is in the lockfile but no longer a dependency")
            }
            LockChange::CapabilitiesChanged { name, was, now } => write!(
                f,
                "`{name}` now declares [{}] where the lockfile recorded [{}]",
                now.join(", "),
                was.join(", ")
            ),
            LockChange::Unreadable => {
                write!(f, "the existing lockfile is unreadable and will be rewritten")
            }
        }
    }
}

/// Parses a lockfile into its recorded entries.
pub fn parse_lock(text: &str) -> Result<Vec<LockEntry>, String> {
    let doc = toml::parse(text)?;
    let mut out = Vec::new();
    for t in doc.array("package") {
        let get = |k: &str| t.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let (Some(name), Some(version), Some(checksum)) =
            (get("name"), get("version"), get("checksum"))
        else {
            return Err("a [[package]] entry is missing name, version, or checksum".into());
        };
        let capabilities = t
            .get("capabilities")
            .and_then(|v| v.as_str_array())
            .map(|c| c.into_iter().map(str::to_string).collect())
            .unwrap_or_default();
        out.push(LockEntry {
            name,
            version,
            checksum,
            capabilities,
        });
    }
    Ok(out)
}

/// Compares a fresh lock against one on disk, reporting packages whose
pub fn diff_lock(previous: &str, current: &[LockEntry]) -> Vec<LockChange> {
    let Ok(recorded) = parse_lock(previous) else {
        return vec![LockChange::Unreadable];
    };
    let by_name: BTreeMap<&str, &LockEntry> =
        recorded.iter().map(|e| (e.name.as_str(), e)).collect();

    let mut changes = Vec::new();
    for e in current {
        match by_name.get(e.name.as_str()) {
            None => changes.push(LockChange::Added(e.name.clone())),
            Some(old) => {
                if old.checksum != e.checksum {
                    changes.push(LockChange::Changed {
                        name: e.name.clone(),
                        was: old.checksum.clone(),
                        now: e.checksum.clone(),
                    });
                }
                if old.capabilities != e.capabilities {
                    changes.push(LockChange::CapabilitiesChanged {
                        name: e.name.clone(),
                        was: old.capabilities.clone(),
                        now: e.capabilities.clone(),
                    });
                }
            }
        }
    }
    let present: BTreeMap<&str, ()> = current.iter().map(|e| (e.name.as_str(), ())).collect();
    for e in &recorded {
        if !present.contains_key(e.name.as_str()) {
            changes.push(LockChange::Removed(e.name.clone()));
        }
    }
    changes
}
