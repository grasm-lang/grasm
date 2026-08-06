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
    /// Filesystem path, relative to the depending package's manifest. `None`
    /// means the package comes from the registry, which requires a `version`.
    pub path: Option<String>,
    /// What the depending package accepts. `None` accepts anything, which is
    /// what a manifest that names only a path is saying.
    pub req: Option<VersionReq>,
}

/// `MAJOR.MINOR.PATCH`, ordered the obvious way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub fn parse(v: &str) -> Result<Version, String> {
        let parts: Vec<&str> = v.split('.').collect();
        let nums: Option<Vec<u64>> = if parts.len() == 3 {
            parts.iter().map(|p| p.parse::<u64>().ok()).collect()
        } else {
            None
        };
        match nums {
            Some(n) => Ok(Version {
                major: n[0],
                minor: n[1],
                patch: n[2],
            }),
            None => Err(format!(
                "version `{v}` is not `MAJOR.MINOR.PATCH` (e.g. `0.1.0`)"
            )),
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A version requirement written in a manifest.
///
/// Two forms, because a third would need justifying. `=1.2.3` is that version
/// and no other. `^1.2.3` — also written bare as `1.2.3` — is that version or
/// any later one that does not change the leading non-zero number, which is
/// the same compatibility rule the stability policy states for the language
/// itself: before 1.0, the minor number carries the weight of a major.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionReq {
    Exact(Version),
    Compatible(Version),
}

impl VersionReq {
    /// The lowest version this requirement admits. Selection starts here and
    /// only ever moves up, which is what makes the result independent of the
    /// order the graph happens to be walked in.
    pub fn floor(&self) -> Version {
        match self {
            VersionReq::Exact(v) | VersionReq::Compatible(v) => *v,
        }
    }

    pub fn parse(text: &str) -> Result<VersionReq, String> {
        let t = text.trim();
        if let Some(rest) = t.strip_prefix('=') {
            return Ok(VersionReq::Exact(Version::parse(rest.trim())?));
        }
        let rest = t.strip_prefix('^').unwrap_or(t);
        Ok(VersionReq::Compatible(Version::parse(rest.trim())?))
    }

    /// The upper bound is exclusive and sits at the next change of the leading
    /// non-zero number: `^1.2.3` stops at 2.0.0, `^0.2.3` at 0.3.0, and
    /// `^0.0.3` accepts only 0.0.3, because in a 0.0.x line every release is
    /// allowed to break.
    pub fn matches(&self, v: &Version) -> bool {
        match self {
            VersionReq::Exact(want) => v == want,
            VersionReq::Compatible(low) => {
                if v < low {
                    return false;
                }
                if low.major > 0 {
                    v.major == low.major
                } else if low.minor > 0 {
                    v.major == 0 && v.minor == low.minor
                } else {
                    v.major == 0 && v.minor == 0 && v.patch == low.patch
                }
            }
        }
    }
}

impl std::fmt::Display for VersionReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionReq::Exact(v) => write!(f, "={v}"),
            VersionReq::Compatible(v) => write!(f, "^{v}"),
        }
    }
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
            let path = table.get("path").and_then(|v| v.as_str()).map(String::from);
            let req = match table.get("version") {
                None => None,
                Some(v) => {
                    let text = v
                        .as_str()
                        .ok_or_else(|| format!("dependency `{dep_name}`: `version` must be a string"))?;
                    Some(VersionReq::parse(text).map_err(|e| {
                        format!("dependency `{dep_name}`: {e}")
                    })?)
                }
            };
            if path.is_none() && req.is_none() {
                return Err(format!(
                    "dependency `{dep_name}` needs a `path` or a `version`: \
                     without either there is nothing to resolve it to"
                ));
            }
            dependencies.push(Dependency {
                name: dep_name,
                path,
                req,
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
    Version::parse(v).map(|_| ())
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

// ---------------------------------------------------------------------------
// Registry and version selection
// ---------------------------------------------------------------------------

/// A directory of packages, laid out as `<root>/<name>/<version>/grasm.toml`.
///
/// Deliberately a directory rather than a service. Fetching packages over a
/// network is a separate problem from deciding which ones to use, and only the
/// second one is the language's business. Point `GRASM_REGISTRY` at a mirror,
/// a vendored tree, or a checkout, and resolution is identical.
#[derive(Debug, Clone)]
pub struct Registry {
    pub root: PathBuf,
}

impl Registry {
    pub fn new(root: PathBuf) -> Registry {
        Registry { root }
    }

    /// `GRASM_REGISTRY` if set, otherwise the nearest `.grasm/registry`
    /// walking up from `from`. Absent, registry dependencies are an error
    /// naming both, rather than a silent failure to find anything.
    pub fn discover(from: &Path) -> Option<Registry> {
        if let Some(dir) = std::env::var_os("GRASM_REGISTRY") {
            return Some(Registry::new(PathBuf::from(dir)));
        }
        let mut here = Some(from.to_path_buf());
        while let Some(d) = here {
            let candidate = d.join(".grasm").join("registry");
            if candidate.is_dir() {
                return Some(Registry::new(candidate));
            }
            here = d.parent().map(Path::to_path_buf);
        }
        None
    }

    /// Every version of `name` the registry holds, ascending. Directories that
    /// are not versions are ignored rather than rejected, so a `README` beside
    /// the version directories does not break resolution.
    pub fn versions(&self, name: &str) -> Vec<Version> {
        let mut out: Vec<Version> = Vec::new();
        let Ok(entries) = std::fs::read_dir(self.root.join(name)) else {
            return out;
        };
        for e in entries.flatten() {
            let file = e.file_name();
            let Some(text) = file.to_str() else { continue };
            if let Ok(v) = Version::parse(text) {
                if e.path().join(MANIFEST_NAME).is_file() {
                    out.push(v);
                }
            }
        }
        out.sort();
        out
    }

    pub fn dir(&self, name: &str, v: &Version) -> PathBuf {
        self.root.join(name).join(v.to_string())
    }
}

/// One package's stated requirement, kept so a conflict can name who asked.
#[derive(Debug, Clone)]
struct Requirement {
    requirer: String,
    on: String,
    req: VersionReq,
}

/// Chooses a version for every registry dependency in the graph.
///
/// **Minimal version selection.** Each requirement names a lowest acceptable
/// version. For each package, take the highest such floor anyone stated, then
/// select the lowest version the registry actually holds at or above it —
/// and then *check* that the selection satisfies every requirement, including
/// the upper bound a `^` implies.
///
/// There is no search and no backtracking. Two consequences are worth stating
/// because they are the reason for choosing this over solving:
///
/// - The answer does not depend on the order the graph was walked, so it is
///   the same on every machine without a lockfile having to pin it.
/// - Adding a dependency cannot silently upgrade one you already had. Nothing
///   moves unless some requirement says it must.
///
/// When requirements genuinely disagree, that is an error naming both sides
/// rather than a version chosen on the program's behalf.
fn select_versions(
    root: &Manifest,
    registry: Option<&Registry>,
) -> Result<BTreeMap<String, Version>, String> {
    let mut floors: BTreeMap<String, Version> = BTreeMap::new();
    let mut selected: BTreeMap<String, Version> = BTreeMap::new();

    // Floors only rise and the registry holds finitely many versions, so this
    // settles. The bound is a backstop against a registry that changes under
    // us, not an expected limit.
    for _ in 0..1000 {
        let stated = collect_requirements(root, registry, &selected)?;
        let mut raised = false;
        for r in &stated {
            let floor = r.req.floor();
            let entry = floors.entry(r.on.clone()).or_insert(floor);
            if floor > *entry {
                *entry = floor;
            }
            if floors[&r.on] > selected.get(&r.on).copied().unwrap_or(floor) {
                raised = true;
            }
        }
        let mut next: BTreeMap<String, Version> = BTreeMap::new();
        for (name, floor) in &floors {
            let Some(reg) = registry else {
                return Err(missing_registry(name, &stated));
            };
            let available = reg.versions(name);
            let Some(pick) = available.iter().copied().filter(|v| v >= floor).min() else {
                return Err(no_such_version(name, floor, &available, reg));
            };
            next.insert(name.clone(), pick);
        }
        if next == selected && !raised {
            return verify(stated, selected);
        }
        selected = next;
    }
    Err("version selection did not settle; is the registry changing?".into())
}

/// Every requirement stated anywhere in the graph, given the versions selected
/// so far. Path dependencies are followed on disk; registry dependencies are
/// followed only once a version has been picked for them.
fn collect_requirements(
    root: &Manifest,
    registry: Option<&Registry>,
    selected: &BTreeMap<String, Version>,
) -> Result<Vec<Requirement>, String> {
    let mut out: Vec<Requirement> = Vec::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: Vec<Manifest> = vec![root.clone()];

    while let Some(m) = queue.pop() {
        if !seen.insert(canonical(&m.dir)) {
            continue;
        }
        for dep in &m.dependencies {
            match &dep.path {
                Some(p) => {
                    let dir = m.dir.join(p);
                    if dir.join(MANIFEST_NAME).is_file() {
                        queue.push(Manifest::load(&dir)?);
                    }
                }
                None => {
                    // A registry dependency always states a version: the
                    // manifest parser rejects one that states neither.
                    let req = dep.req.expect("registry dependency without a version");
                    out.push(Requirement {
                        requirer: m.name.clone(),
                        on: dep.name.clone(),
                        req,
                    });
                    if let (Some(reg), Some(v)) = (registry, selected.get(&dep.name)) {
                        let dir = reg.dir(&dep.name, v);
                        if dir.join(MANIFEST_NAME).is_file() {
                            queue.push(Manifest::load(&dir)?);
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Confirms the selection satisfies everyone, and says who disagrees if not.
fn verify(
    stated: Vec<Requirement>,
    selected: BTreeMap<String, Version>,
) -> Result<BTreeMap<String, Version>, String> {
    for r in &stated {
        let Some(got) = selected.get(&r.on) else {
            continue;
        };
        if !r.req.matches(got) {
            let mut who: Vec<String> = stated
                .iter()
                .filter(|o| o.on == r.on)
                .map(|o| format!("  `{}` requires {}", o.requirer, o.req))
                .collect();
            who.sort();
            who.dedup();
            return Err(format!(
                "no version of `{}` satisfies every requirement:\n{}\n  \
                 selected {got}, the lowest available at or above the highest floor\n  \
                 nothing here can be satisfied at once — one of these has to change",
                r.on,
                who.join("\n")
            ));
        }
    }
    Ok(selected)
}

fn missing_registry(name: &str, stated: &[Requirement]) -> String {
    let who: Vec<&str> = stated
        .iter()
        .filter(|r| r.on == name)
        .map(|r| r.requirer.as_str())
        .collect();
    format!(
        "`{}` requires `{name}` from the registry, but no registry was found.\n  \
         set GRASM_REGISTRY, or create .grasm/registry beside the package",
        who.first().copied().unwrap_or("this package")
    )
}

fn no_such_version(name: &str, floor: &Version, available: &[Version], reg: &Registry) -> String {
    if available.is_empty() {
        return format!(
            "the registry has no package named `{name}`\n  looked in {}",
            reg.root.join(name).display()
        );
    }
    let have: Vec<String> = available.iter().map(Version::to_string).collect();
    format!(
        "the registry has no `{name}` at or above {floor}\n  available: {}",
        have.join(", ")
    )
}

/// A package the walk has already settled on.
struct Resolved {
    /// Canonical directory, so two routes to one package compare equal.
    dir: PathBuf,
    version: Version,
    /// Who pulled it in, for the message when somebody else disagrees.
    required_by: String,
}

/// Where a path leads with `..` and symlinks removed, so that `../text` from
/// two different packages is recognised as the same package.
fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Walks the dependency graph depth-first, detecting cycles and missing
/// packages, and returns packages with dependencies before dependents.
///
/// A name resolves to exactly one package. Reaching the same name through two
/// different directories is an error rather than a coin flip: the build would
/// otherwise link whichever copy the walk happened to see first, and give the
/// other dependent something it never asked for.
pub fn resolve(root_dir: &Path) -> Result<Resolution, String> {
    let registry = Registry::discover(root_dir);
    resolve_in(root_dir, registry.as_ref())
}

/// `resolve` against an explicit registry, which is what the tests use so they
/// do not depend on what happens to be installed.
pub fn resolve_in(root_dir: &Path, registry: Option<&Registry>) -> Result<Resolution, String> {
    let root = Manifest::load(root_dir)?;
    let selected = select_versions(&root, registry)?;
    let mut ordered: Vec<Manifest> = Vec::new();
    let mut visiting: Vec<String> = Vec::new();
    let mut done: BTreeMap<String, Resolved> = BTreeMap::new();
    let ctx = Selection {
        registry: registry.cloned(),
        versions: selected,
    };
    visit(root_dir, None, &ctx, &mut ordered, &mut visiting, &mut done)?;
    Ok(Resolution { packages: ordered })
}

/// The registry and what was selected from it, threaded through the walk.
struct Selection {
    registry: Option<Registry>,
    versions: BTreeMap<String, Version>,
}

impl Selection {
    /// Where a dependency's sources are: beside the manifest for a path
    /// dependency, in the registry at the selected version otherwise.
    fn dir_of(&self, from: &Manifest, dep: &Dependency) -> Result<PathBuf, String> {
        match &dep.path {
            Some(p) => Ok(from.dir.join(p)),
            None => {
                let reg = self
                    .registry
                    .as_ref()
                    .ok_or_else(|| missing_registry(&dep.name, &[]))?;
                let v = self.versions.get(&dep.name).ok_or_else(|| {
                    format!("internal: no version selected for `{}`", dep.name)
                })?;
                Ok(reg.dir(&dep.name, v))
            }
        }
    }
}

fn visit(
    dir: &Path,
    required_by: Option<&str>,
    sel: &Selection,
    ordered: &mut Vec<Manifest>,
    visiting: &mut Vec<String>,
    done: &mut BTreeMap<String, Resolved>,
) -> Result<(), String> {
    let manifest = Manifest::load(dir)?;
    let here = canonical(dir);

    if let Some(prev) = done.get(&manifest.name) {
        if prev.dir != here {
            return Err(format!(
                "package `{}` resolves to two different directories:\n  \
                 {} is {} (required by `{}`)\n  {} is {} (required by `{}`)\n\
                 a name resolves to one package, so these have to be the same directory",
                manifest.name,
                prev.dir.display(),
                prev.version,
                prev.required_by,
                here.display(),
                manifest.version,
                required_by.unwrap_or("the root package"),
            ));
        }
        return Ok(());
    }
    if visiting.contains(&manifest.name) {
        let mut cycle = visiting.clone();
        cycle.push(manifest.name.clone());
        return Err(format!("dependency cycle: {}", cycle.join(" -> ")));
    }
    visiting.push(manifest.name.clone());

    for dep in &manifest.dependencies {
        let dep_dir = sel.dir_of(&manifest, dep)?;
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
        if let Some(req) = &dep.req {
            let found = Version::parse(&dep_manifest.version)?;
            if !req.matches(&found) {
                return Err(format!(
                    "dependency `{}` of `{}`: requires {req}, but {} is {found}",
                    dep.name,
                    manifest.name,
                    dep_dir.display()
                ));
            }
        }
        visit(&dep_dir, Some(&manifest.name), sel, ordered, visiting, done)?;
    }

    visiting.pop();
    done.insert(
        manifest.name.clone(),
        Resolved {
            dir: here,
            version: Version::parse(&manifest.version)?,
            required_by: required_by.unwrap_or("the root package").to_string(),
        },
    );
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
