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
///
/// `unsafe` is Tier 3: it is not something the runtime does for you, it is
/// permission to leave the safe tiers. It belongs here because it must be
/// declarable without a matching `effect` declaration in the source.
pub const BUILTIN_CAPABILITIES: &[&str] = &["io", "async", "net", "unsafe"];

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
    /// System libraries this package's `extern fn`s are resolved from.
    ///
    /// An `extern fn` is resolved by the system linker, so a package calling
    /// one needs its library on the link line. Before this existed the
    /// requirement was undeclared: `packages/sdl` built only if the caller
    /// remembered `--link SDL2`, and forgot it meant a linker error naming
    /// fourteen symbols rather than the one missing library. A package that
    /// says what it needs is the whole point of the manifest.
    ///
    /// Sorted and deduplicated, because the link line is assembled from every
    /// package in the graph and two of them asking for the same library must
    /// not put it there twice.
    pub links: BTreeSet<String>,
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

        let links: BTreeSet<String> = match pkg.get("links") {
            None => BTreeSet::new(),
            Some(v) => {
                let list = v
                    .as_str_array()
                    .ok_or("`links` must be an array of strings")?;
                for name in &list {
                    // This goes on a linker command line as `-l<name>`, so a
                    // name carrying a flag, a path or a shell character would
                    // be the manifest choosing linker arguments. A library is
                    // an identifier.
                    // `-` and `.` are allowed inside a name -- `SDL2-2.0`
                    // is a real library -- but not as the first character,
                    // where `-` would make the entry a linker flag and `.`
                    // the start of a relative path. The first character
                    // being checked separately is the whole difference
                    // between rejecting `-lSDL2` and accepting it.
                    let shape_ok = !name.is_empty()
                        && name
                            .chars()
                            .all(|c| {
                                c.is_ascii_alphanumeric()
                                    || c == '_'
                                    || c == '-'
                                    || c == '.'
                                    || c == '+'
                            })
                        && name.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
                    if !shape_ok {
                        return Err(format!(
                            "`links` entry `{name}` is not a library name — \
                             letters, digits, `_`, `-`, `.` and `+`, and it \
                             must start with a letter, a digit or `_`"
                        ));
                    }
                }
                list.into_iter().map(|s| s.to_string()).collect()
            }
        };

        let entry = pkg
            .get("entry")
            .and_then(|v| v.as_str())
            .unwrap_or("src/main.gr")
            .to_string();
        // A package's sources are the `.gr` files under its own directory:
        // that is what the lockfile checksums, what the capability scan reads,
        // and what `publish` copies. An `entry` pointing outside it — `../x.gr`
        // or an absolute path — built and ran, while the lockfile recorded a
        // checksum over files the program never used and the ceiling was
        // computed from a tree the entry had left. A ceiling nobody can rely
        // on is worse than none.
        {
            let p = Path::new(&entry);
            if p.is_absolute() {
                return Err(format!(
                    "`entry` must be a path inside the package: `{entry}` is absolute"
                ));
            }
            if p.components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(format!(
                    "`entry` must be a path inside the package: `{entry}` leaves it with \
`..` — the lockfile checksums, the capability ceiling and `grasm publish` all cover this \
package's own directory, and a file outside it is in none of them"
                ));
            }
            // And it has to be a Grasm source file. The lockfile's checksum
            // and `grasm publish` both collect `.gr` files, so an `entry`
            // named anything else left the package's *executed source* outside
            // the digest — `--locked` reported "up to date" after it changed —
            // and outside the published set, which shipped a package that
            // could not build into a slot that cannot be republished.
            if !is_grasm_source(p) {
                return Err(format!(
                    "`entry` must name a `.gr` file: `{entry}` is not one, and the \
lockfile's checksum and `grasm publish` both cover this package's `.gr` sources — a file \
outside that set is executed but recorded nowhere"
                ));
            }
        }

        let mut dependencies = Vec::new();
        for (dep_name, table) in doc.subtables("dependencies") {
            let path = table.get("path").and_then(|v| v.as_str()).map(String::from);
            let req = match table.get("version") {
                None => None,
                Some(v) => {
                    let text = v.as_str().ok_or_else(|| {
                        format!("dependency `{dep_name}`: `version` must be a string")
                    })?;
                    Some(
                        VersionReq::parse(text)
                            .map_err(|e| format!("dependency `{dep_name}`: {e}"))?,
                    )
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

        // An unknown key is a typo, and one typo costs the whole guarantee:
        // `capabilitys = []` left the package *unrestricted*, which is what
        // omitting the key means, and only `grasm deps`'s `(unrestricted)`
        // column said so. `entrypoint` for `entry` silently kept the default.
        // A key in no table at all — written above `[package]` — was ignored
        // the same way.
        const PACKAGE_KEYS: &[&str] = &["name", "version", "capabilities", "entry", "links"];
        if let Some(pkg) = doc.table("package") {
            if let Some(k) = pkg.keys().find(|k| !PACKAGE_KEYS.contains(&k.as_str())) {
                return Err(format!(
                    "`[package]` has no key `{k}` — it takes {}",
                    PACKAGE_KEYS
                        .iter()
                        .map(|k| format!("`{k}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        if let Some(root) = doc.table("") {
            if let Some(k) = root.keys().next() {
                return Err(format!(
                    "`{k}` is written above the first table header, so it belongs to no \
table — put it under `[package]`"
                ));
            }
        }
        // A table nobody recognises is the same typo one level up, and it
        // spent the same thing: `[capabilities]` instead of
        // `capabilities = []` left the package *unrestricted*, and unlike the
        // key spelling it left no trace in the lockfile at all.
        for name in doc.tables.keys() {
            if name.is_empty() || name == "package" || name == "dependencies" {
                continue;
            }
            if name
                .strip_prefix("dependencies.")
                .is_some_and(|d| !d.is_empty())
            {
                continue;
            }
            return Err(format!(
                "`[{name}]` is not a table this manifest has — it takes `[package]` and \
`[dependencies.<name>]`"
            ));
        }
        const DEP_KEYS: &[&str] = &["path", "version"];
        for (dep_name, table) in doc.subtables("dependencies") {
            if let Some(k) = table.keys().find(|k| !DEP_KEYS.contains(&k.as_str())) {
                return Err(format!(
                    "dependency `{dep_name}` has no key `{k}` — it takes `path` or `version`"
                ));
            }
        }

        // A package's name is how a consumer writes `import <name>`, so a name
        // that is not an identifier names a package nobody can depend on. It
        // was accepted, listed in the graph and written to the lockfile, and
        // the failure surfaced only in the consumer's parser.
        if !is_identifier(&name) {
            return Err(format!(
                "`{name}` is not a usable package name: a consumer writes `import {name}`, \
so it has to be a Grasm identifier — letters, digits and `_`, not starting with a digit, \
and not a keyword"
            ));
        }

        Ok(Manifest {
            name,
            version,
            capabilities,
            links,
            dependencies,
            entry,
            dir: dir.to_path_buf(),
        })
    }

    pub fn load(dir: &Path) -> Result<Manifest, String> {
        let path = dir.join(MANIFEST_NAME);
        let text = read_manifest_text(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        // Named here, not in the parser: `line 1: expected `key = value``
        // said which line of *something*, and a graph has one manifest per
        // package. Which file is the first thing a reader needs.
        Manifest::parse(&text, dir).map_err(|e| format!("{}: {e}", grasm_sema::loader::tidy(&path)))
    }

    pub fn entry_path(&self) -> PathBuf {
        self.dir.join(&self.entry)
    }
}

/// Whether `name` is something a consumer could write after `import`.
fn is_identifier(name: &str) -> bool {
    // The keyword list the grammar reserves. A package named for one of them
    // parses as that keyword in the consumer's `import` line.
    const KEYWORDS: &[&str] = &[
        "and",
        "as",
        "bench",
        "break",
        "const",
        "continue",
        "effect",
        "else",
        "extern",
        "fail",
        "false",
        "fn",
        "for",
        "handled_by",
        "if",
        "import",
        "in",
        "inout",
        "let",
        "match",
        "not",
        "or",
        "own",
        "parallel",
        "pub",
        "ref",
        "region",
        "return",
        "spawn",
        "test",
        "true",
        "try",
        "type",
        "unsafe",
        "var",
        "while",
        "with",
    ];
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    !KEYWORDS.contains(&name)
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
        // Absolute first, because the walk is by `parent()` and a relative
        // path runs out of parents immediately: `.` has none, and `user` has
        // one empty one. So the registry was found only when the working
        // directory was *exactly* the one holding `.grasm/`, and the same
        // package built or did not build depending on where the shell happened
        // to be — while the error told the reader to put `.grasm/registry`
        // beside the package, which is where it already was.
        let start = from
            .canonicalize()
            .or_else(|_| std::env::current_dir().map(|c| c.join(from)))
            .unwrap_or_else(|_| from.to_path_buf());
        let mut here = Some(start);
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

/// What `publish` did, so the caller can report it without re-deriving it.
pub struct Published {
    pub name: String,
    pub version: Version,
    pub dir: PathBuf,
    pub files: usize,
}

/// Copies a package into a registry at its declared version.
///
/// **A published version is immutable.** Republishing a version that already
/// exists is refused rather than overwritten, and that refusal is the whole
/// point: `grasm.lock` records a checksum per package, so content that changes
/// under a fixed version number is the one supply-chain move the lockfile is
/// there to catch. Refusing it at the source is better than catching it at
/// every consumer.
///
/// Only the manifest and the source tree are copied. Build output, lockfiles,
/// and version-control directories are a publisher's local state and have no
/// business in a package other people resolve against.
pub fn publish(pkg_dir: &Path, registry: &Registry) -> Result<Published, String> {
    let manifest = Manifest::load(pkg_dir)?;
    let version = Version::parse(&manifest.version)?;

    let entry = manifest.entry_path();
    if !entry.exists() {
        return Err(format!(
            "package `{}`: entry file {} does not exist",
            manifest.name,
            entry.display()
        ));
    }

    // Omitting `capabilities` means *unrestricted*, not empty
    // ([T-CAPABILITY]), which is the right default for a directory of files
    // you are editing and the wrong one for something other people resolve
    // against: a consumer cannot see what they are agreeing to, and nothing
    // would be checked for them. So the key is optional everywhere except
    // here. `capabilities = []` is the answer for a package that performs
    // nothing, and says so.
    if manifest.capabilities.is_none() {
        return Err(format!(
            "package `{}` has no `capabilities` key, so nothing about what it \
may reach is checked for the people who depend on it\n  add `capabilities = []` \
if it performs nothing, or list what it performs",
            manifest.name
        ));
    }

    // A package that already exceeds its own ceiling should not enter a
    // registry at all: every consumer would fail, and the failure would look
    // like their problem.
    let performed = effects_of_package(&manifest)?;
    if let Some(v) = check_capabilities(&manifest, &performed) {
        return Err(format!(
            "{v}\n  refusing to publish a package that exceeds its own ceiling"
        ));
    }

    // A `path` dependency names a directory beside the package, and only the
    // package travels into the registry. Published anyway, the release named a
    // dependency that resolved to `<registry>/<name>/../util` — outside the
    // version directory — so every consumer failed on a package that cannot be
    // republished. Refused for the same reason an `entry` may not leave the
    // package.
    if let Some(dep) = manifest.dependencies.iter().find(|d| d.path.is_some()) {
        return Err(format!(
            "`{}` depends on `{}` by path, which cannot travel into a registry: \
only this package's own directory is published, so the release would name a dependency \
no consumer can resolve\n  give `{}` a `version` and publish it first",
            manifest.name, dep.name, dep.name
        ));
    }

    let dest = registry.dir(&manifest.name, &version);
    if dest.join(MANIFEST_NAME).exists() {
        return Err(format!(
            "`{}` {} is already in the registry at {}\n  \
             a published version is immutable — publish {}.{}.{} instead\n  \
             (`grasm.lock` records a checksum per package, so content that\n  \
             changes under a fixed version is exactly what it exists to catch)",
            manifest.name,
            version,
            dest.display(),
            version.major,
            version.minor,
            version.patch + 1,
        ));
    }

    std::fs::create_dir_all(&dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut files = 0;
    copy_into(&pkg_dir.join(MANIFEST_NAME), &dest.join(MANIFEST_NAME))?;
    files += 1;
    // From the package root, which is what the checksum covers. Walking from
    // the *entry's* directory dropped every `.gr` file beside `src/`.
    files += copy_sources(pkg_dir, &dest)?;

    Ok(Published {
        name: manifest.name,
        version,
        dir: dest,
        files,
    })
}

/// Adds a registry dependency to a manifest, in the text the author wrote.
///
/// Appends rather than reformats. A manifest is a file someone maintains by
/// hand — comments, ordering, and spacing are theirs — and a tool that
/// rewrites it to its own taste every time it touches it is a tool people stop
/// using.
pub fn add_dependency(pkg_dir: &Path, name: &str, req: &VersionReq) -> Result<String, String> {
    // The name is written into the manifest verbatim, so it has to be one a
    // manifest can hold — and one a consumer can write after `import`, which
    // is the same rule a package's own name follows. Unchecked, `grasm add
    // ../evil 1.0.0` wrote `[dependencies.../evil]` and resolved it outside
    // the registry root; a name containing a newline and a `[` appended a
    // second `[package]` table and corrupted the manifest while `add` reported
    // success.
    if !is_identifier(name) {
        return Err(format!(
            "`{name}` is not a usable dependency name: a consumer writes `import {name}`, \
so it has to be a Grasm identifier — letters, digits and `_`, not starting with a digit, \
and not a keyword"
        ));
    }
    let path = pkg_dir.join(MANIFEST_NAME);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let manifest = Manifest::parse(&text, pkg_dir)?;
    if let Some(existing) = manifest.dependencies.iter().find(|d| d.name == name) {
        return Err(match &existing.req {
            Some(r) => format!("`{name}` is already a dependency, at {r}"),
            None => format!("`{name}` is already a dependency, by path"),
        });
    }
    let mut out = text;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("\n[dependencies.{name}]\nversion = \"{req}\"\n"));
    std::fs::write(&path, &out).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(manifest.name)
}

fn copy_into(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("cannot copy {} to {}: {e}", from.display(), to.display()))
}

/// Every `.gr` file under `from`, preserving the tree so imports still resolve.
/// Copies the package's sources into the registry, from the same enumeration
/// the checksum uses.
///
/// Two walks meant two answers about one released version: a `.gr` file beside
/// `src/` was hashed by the author and dropped here, so the registry copy and
/// the published package had different checksums and there was no way to check
/// one against the other.
fn copy_sources(from: &Path, to: &Path) -> Result<usize, String> {
    let mut n = 0;
    for path in source_files(from)? {
        let rel = path.strip_prefix(from).unwrap_or(&path);
        let dest = to.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        copy_into(&path, &dest)?;
        n += 1;
    }
    Ok(n)
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
    thread_local! {
        static CACHE: std::cell::RefCell<std::collections::HashMap<PathBuf, PathBuf>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    }
    // Resolution canonicalizes the same directories at every hop, and
    // `realpath` was the single largest cost of `grasm check` on a
    // sixty-four-package chain. A directory that moves out from under a
    // running command is not a case this has to serve.
    if let Some(hit) = CACHE.with(|c| c.borrow().get(p).cloned()) {
        return hit;
    }
    let real = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    CACHE.with(|c| c.borrow_mut().insert(p.to_path_buf(), real.clone()));
    real
}

/// A manifest's text, read once per (path, modification time, length).
///
/// Resolution asks for the same manifests over and over — every dependency is
/// loaded once to check its name, once to check its version, and once more
/// when the walk reaches it — so a chain of N packages read O(N²) files. The
/// stat keeps the cache honest: a manifest edited between two loads in the
/// same process is re-read.
fn read_manifest_text(path: &Path) -> std::io::Result<String> {
    type Stamp = (Option<std::time::SystemTime>, u64);
    thread_local! {
        static CACHE: std::cell::RefCell<std::collections::HashMap<PathBuf, (Stamp, String)>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    }
    let Some(stamp) = std::fs::metadata(path)
        .ok()
        .map(|m| (m.modified().ok(), m.len()))
    else {
        return std::fs::read_to_string(path);
    };
    let hit = CACHE.with(|c| {
        c.borrow()
            .get(path)
            .filter(|(s, _)| *s == stamp)
            .map(|(_, t)| t.clone())
    });
    if let Some(text) = hit {
        return Ok(text);
    }
    let text = std::fs::read_to_string(path)?;
    CACHE.with(|c| {
        c.borrow_mut()
            .insert(path.to_path_buf(), (stamp, text.clone()))
    });
    Ok(text)
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
                let v = self
                    .versions
                    .get(&dep.name)
                    .ok_or_else(|| format!("internal: no version selected for `{}`", dep.name))?;
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
                // A dependency is reached from the depending package's
                // directory, so the path came out as `md/../gone` — correct,
                // and not what anyone would type or could `cd` to and look at.
                grasm_sema::loader::tidy(&dep_dir)
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
/// Where a package performs each effect, for the ceiling message.
///
/// One representative site per effect. Effects a package only inherits from a
/// call it makes have no site, and are reported by name alone as before.
/// The package's own dependency graph, as name → directory.
///
/// A registry dependency has no path in the manifest, so a module that
/// imports one cannot be read without this — and a *sibling* module that did
/// made the ceiling check refuse to build against the package "whose reach is
/// unknown", naming nothing.
fn own_resolved(manifest: &Manifest) -> std::collections::HashMap<String, PathBuf> {
    resolve(&manifest.dir)
        .map(|r| {
            r.packages
                .iter()
                .map(|p| (p.name.clone(), p.dir.clone()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn effect_sites_of_package(
    manifest: &Manifest,
) -> Result<std::collections::HashMap<String, String>, String> {
    let entry = manifest.entry_path();
    if !entry.exists() {
        return Ok(std::collections::HashMap::new());
    }
    let resolved = own_resolved(manifest);
    let program = grasm_sema::loader::load_from_disk_with(&entry, &resolved)?;
    let mut out = grasm_sema::effects::effect_sites(&program);
    // The same whole-package scan the ceiling uses, so a breach in an
    // unimported sibling module still says where it is.
    for extra in package_modules(manifest, &entry) {
        if let Ok(p) = grasm_sema::loader::load_from_disk_with(&extra, &resolved) {
            for (k, v) in grasm_sema::effects::effect_sites(&p) {
                out.entry(k).or_insert(v);
            }
        }
    }
    Ok(out)
}

pub fn effects_of_package(manifest: &Manifest) -> Result<BTreeSet<String>, String> {
    let entry = manifest.entry_path();
    if !entry.exists() {
        return Err(format!(
            "package `{}`: entry file {} does not exist",
            manifest.name,
            entry.display()
        ));
    }
    let resolved = own_resolved(manifest);
    // Every failure this returns names the package, so a caller listing them
    // does not have to add the name back — and adding it anyway printed
    // ``dep`: package `dep`: …`.
    let program = grasm_sema::loader::load_from_disk_with(&entry, &resolved)
        .map_err(|e| format!("package `{}`: {e}", manifest.name))?;
    // The entry file gets the same treatment as a sibling: a package whose
    // source cannot be read cannot be held to its ceiling, and affirming it
    // anyway reported `performs []` and wrote a lockfile over code that had
    // never been analysed.
    if let Some(failure) = first_failure(&program, &entry) {
        return Err(format!("package `{}`: {failure}", manifest.name));
    }
    let mut out = grasm_sema::effects::effects_of_program(&program);
    // Every module in the package, not only the ones its entry imports.
    //
    // A consumer reaches a `pub` function with `import dep.module`, whether or
    // not the dependency's own entry mentions that module — so a `capabilities
    // = []` package could keep a `pub fn read_secrets` in an unimported
    // sibling file and be reported as `performs []`. The effects then landed
    // on the *consumer*, at a source location inside the dependency that the
    // consumer cannot fix, and if the consumer already declared `io` for its
    // own reasons, nothing was reported at all.
    for extra in package_modules(manifest, &entry) {
        let p = grasm_sema::loader::load_from_disk_with(&extra, &resolved)
            .map_err(|e| format!("package `{}`: {}", manifest.name, e))?;
        // A file that does not parse cannot be read for what it performs, and
        // affirming the package anyway is the wrong answer in the one
        // direction that matters: `grasm deps` reported `performs []` and
        // wrote a lockfile over sources it had never analysed.
        if let Some(failure) = first_failure(&p, &extra) {
            return Err(format!("package `{}`: {failure}", manifest.name));
        }
        out.extend(grasm_sema::effects::effects_of_program(&p));
    }
    Ok(out)
}

/// Why a package could not be analysed, naming the file the failure is in.
///
/// A loaded program's `diagnostics` is one flat list across every file the
/// load pulled in — including the source of every dependency — so the first
/// entry is often not in the entry file at all. Naming the entry anyway sent a
/// reader to a file with nothing wrong in it: a syntax error in `dep` was
/// reported as `cannot read ./src/main.gr` against the consumer, at a path
/// spelled `./../dep/src/main.gr` when it named the right file at all.
fn first_failure(program: &grasm_sema::loader::Program, entry: &Path) -> Option<String> {
    // Parse errors live on the program; type errors come from the check, and
    // only the first half was consulted. A package that parsed but did not
    // typecheck was reported `performs []` and written into a lockfile — and
    // "did not typecheck" is exactly where an effect the ceiling would have
    // caught can hide, since a refused declaration performs nothing the
    // inference can see. Affirming a package nobody could compile is the one
    // direction that must never be silent.
    let checked;
    let d = match program.diagnostics.first() {
        Some(d) => d,
        None => {
            checked = grasm_sema::check::check_program(program).diagnostics;
            checked.first()?
        }
    };
    let (path, borrowed) = match program.files.get(program.file_of(d.span)) {
        Some(f) => (
            grasm_sema::loader::tidy(Path::new(&f.path)),
            f.path != grasm_sema::loader::tidy(entry) && !f.name.is_empty() && f.base != 0,
        ),
        None => (grasm_sema::loader::tidy(entry), false),
    };
    // A failure inside a dependency is not the consumer's to fix, and saying
    // so is the difference between one useful line and two identical ones.
    let whose = if borrowed {
        " (reached through a dependency)"
    } else {
        ""
    };
    Some(format!("`{path}`{whose} — {}", d.message))
}

/// Every `.gr` file in a package's source tree apart from its entry.
fn package_modules(manifest: &Manifest, entry: &std::path::Path) -> Vec<std::path::PathBuf> {
    // From the *package* directory, which is what the checksum and
    // `grasm publish` cover. Walking from the entry's directory instead meant
    // a `.gr` file elsewhere in the package — `tests/t.gr` beside `src/` — was
    // checksummed and shipped and never held to the ceiling, nor even parsed:
    // a `capabilities = []` package could keep a filesystem reader there and
    // be reported as performing nothing, which is the failure this whole-
    // package scan exists to prevent.
    source_files(&manifest.dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p != entry)
        .collect()
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
    let program = grasm_sema::loader::load_from_disk_with(&entry, &own_resolved(manifest))?;
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
    /// The declared ceiling, or `None` for a manifest that declares none.
    ///
    /// `[]` and "unrestricted" are opposite things — nothing permitted versus
    /// everything — and recording both as `[]` made a later run report that a
    /// package "now declares [] where the lockfile recorded [io]" about a
    /// manifest that had not changed.
    pub capabilities: Option<Vec<String>>,
}

/// Hashes every `.gr` file in a package, in sorted order, so the digest is
/// stable across filesystems.
pub fn package_checksum(manifest: &Manifest) -> Result<String, String> {
    let files = source_files(&manifest.dir)?;
    let mut buf: Vec<u8> = Vec::new();
    // The manifest is part of what a consumer depends on, and it was not part
    // of the checksum: three releases with identical sources and different
    // `capabilities` lines hashed the same. The ceiling is the security-
    // relevant field, so a checksum that cannot tell two of them apart is a
    // record with a hole in exactly the place it matters.
    if let Ok(text) = std::fs::read(manifest.dir.join(MANIFEST_NAME)) {
        buf.extend(MANIFEST_NAME.as_bytes());
        buf.push(0);
        buf.extend(text);
        buf.push(0);
    }
    for f in &files {
        let rel = f.strip_prefix(&manifest.dir).unwrap_or(f);
        buf.extend(rel.to_string_lossy().as_bytes());
        buf.push(0);
        buf.extend(std::fs::read(f).map_err(|e| format!("cannot read {}: {e}", f.display()))?);
        buf.push(0);
    }
    Ok(hash::sha256_hex(&buf))
}

/// Every Grasm source file in a package, sorted.
///
/// The lockfile's checksum, `grasm publish` and the capability scan are three
/// claims about "this package's sources", and each used to enumerate them
/// differently — which is one hole per disagreement:
///
///   * the checksum skipped `target/` and every dot-directory, and both are
///     legal import path segments, so a module under one was executed and
///     `--locked` still reported "up to date" after it changed;
///   * the extension was matched case-sensitively, so `HELPER.GR` — which
///     `import helper` resolves to on a case-insensitive filesystem — was
///     invisible to all three, including the ceiling;
///   * `publish` walked from the *entry's* directory and the checksum from the
///     *package's*, so a `.gr` file beside `src/` was hashed by the author and
///     dropped by `publish`: one released version, two checksums.
///
/// One walk answers all three. Symlinked directories are followed — a source
/// tree may legitimately be assembled that way — but each real directory is
/// visited once: a package containing two self-links took `deps` and `publish`
/// from "34 copies into an immutable slot" to "never returns".
pub fn source_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_sources(root, &mut out, &mut seen)?;
    out.sort();
    out.dedup();
    Ok(out)
}

fn collect_sources(
    dir: &Path,
    out: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
) -> Result<(), String> {
    let real = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if !seen.insert(real) {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read directory {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sources(&path, out, seen)?;
        } else if is_grasm_source(&path) {
            out.push(path);
        }
    }
    Ok(())
}

/// Whether a path names a Grasm source file.
///
/// Case-insensitively: `import helper` resolves to `HELPER.GR` on a
/// case-insensitive filesystem, so a file spelled that way is executable code
/// and has to be counted as such.
fn is_grasm_source(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("gr"))
}

pub fn build_lock(resolution: &Resolution) -> Result<Vec<LockEntry>, String> {
    let mut entries = Vec::new();
    for m in &resolution.packages {
        entries.push(LockEntry {
            name: m.name.clone(),
            version: m.version.clone(),
            checksum: package_checksum(m)?,
            capabilities: m.capabilities.as_ref().map(|c| c.iter().cloned().collect()),
        });
    }
    Ok(entries)
}

pub fn render_lock(entries: &[LockEntry]) -> String {
    let mut out = String::from(
        "# Generated by `grasm deps`. Commit this file.\n\
         # Checksums are SHA-256 over each package's grasm.toml and its sorted .gr sources.\n",
    );
    for e in entries {
        out.push_str("\n[[package]]\n");
        out.push_str(&format!("name = {}\n", toml::quote(&e.name)));
        out.push_str(&format!("version = {}\n", toml::quote(&e.version)));
        out.push_str(&format!("checksum = {}\n", toml::quote(&e.checksum)));
        // Omitted entirely when the manifest declares no ceiling, so the
        // lockfile says the same thing the manifest does.
        if let Some(caps) = &e.capabilities {
            let caps: Vec<String> = caps.iter().map(|c| toml::quote(c)).collect();
            out.push_str(&format!("capabilities = [{}]\n", caps.join(", ")));
        }
    }
    out
}

/// How a lockfile on disk differs from what the source tree now hashes to.
#[derive(Debug, Clone, PartialEq)]
pub enum LockChange {
    /// A package's sources changed since the lockfile was written. This is the
    /// case a lockfile exists to catch: a dependency that is not what it was.
    Changed {
        name: String,
        was: String,
        now: String,
    },
    /// A package resolved to a different *version* than the lockfile records.
    ///
    /// Reported separately from a content change because it can happen without
    /// one: a metadata-only release has the same sources, so the checksums
    /// match and only the version moved. Comparing checksums alone let that
    /// through `--locked`, and the lockfile then went on recording a version
    /// that was no longer in the build — a factually wrong record passing CI.
    VersionChanged {
        name: String,
        was: String,
        now: String,
    },
    /// A package the lockfile does not mention.
    Added(String),
    /// A package the lockfile mentions that is no longer in the graph.
    Removed(String),
    /// A package's declared capability ceiling changed — the security-relevant
    /// change, reported separately from a content change.
    CapabilitiesChanged {
        name: String,
        was: Option<Vec<String>>,
        now: Option<Vec<String>>,
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
            LockChange::VersionChanged { name, was, now } => write!(
                f,
                "`{name}` is version {now} but the lockfile records {was}"
            ),
            LockChange::Added(name) => write!(f, "`{name}` is not in the lockfile"),
            LockChange::Removed(name) => {
                write!(f, "`{name}` is in the lockfile but no longer a dependency")
            }
            LockChange::CapabilitiesChanged { name, was, now } => {
                // "declares nothing" and "declares no ceiling" are opposite
                // things, and saying `[]` for both is how the same manifest
                // appeared to have changed.
                let show = |c: &Option<Vec<String>>| match c {
                    Some(v) if v.is_empty() => "[] (nothing permitted)".to_string(),
                    Some(v) => format!("[{}]", v.join(", ")),
                    None => "no ceiling".to_string(),
                };
                write!(
                    f,
                    "`{name}` now declares {} where the lockfile recorded {}",
                    show(now),
                    show(was)
                )
            }
            LockChange::Unreadable => {
                write!(
                    f,
                    "the existing lockfile is unreadable and will be rewritten"
                )
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
        let capabilities: Option<Vec<String>> = t
            .get("capabilities")
            .and_then(|v| v.as_str_array())
            .map(|c| c.into_iter().map(str::to_string).collect());
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
                if old.version != e.version {
                    changes.push(LockChange::VersionChanged {
                        name: e.name.clone(),
                        was: old.version.clone(),
                        now: e.version.clone(),
                    });
                }
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

#[cfg(test)]
mod links_tests {
    use super::*;

    fn parse(body: &str) -> Result<Manifest, String> {
        Manifest::parse(body, Path::new("/tmp/x"))
    }

    const HEAD: &str = "[package]\nname = \"p\"\nversion = \"1.0.0\"\n";

    #[test]
    fn a_package_declares_the_libraries_it_links() {
        let m = parse(&format!("{HEAD}links = [\"SDL2\", \"m\"]\n")).unwrap();
        assert_eq!(
            m.links.iter().cloned().collect::<Vec<_>>(),
            vec!["SDL2".to_string(), "m".to_string()]
        );
    }

    #[test]
    fn no_links_is_an_empty_set_not_an_error() {
        // The overwhelming majority of packages call no C at all.
        assert!(parse(HEAD).unwrap().links.is_empty());
    }

    #[test]
    fn a_library_named_twice_is_held_once() {
        // The link line is assembled from every package in the graph, so two
        // of them wanting the same library must not pass it twice.
        let m = parse(&format!("{HEAD}links = [\"SDL2\", \"SDL2\"]\n")).unwrap();
        assert_eq!(m.links.len(), 1);
    }

    #[test]
    fn a_links_entry_is_a_library_name_and_not_a_linker_flag() {
        // This becomes `-l<name>` on a command line. Anything that could be
        // read as a flag, a path or a shell word would be the manifest
        // choosing linker arguments, which is not what it is for.
        for bad in [
            "-lSDL2",
            "/usr/lib/libSDL2.dylib",
            "SDL2 -rpath /tmp",
            "a;b",
            "$(whoami)",
            "`id`",
            "",
            "a\nb",
        ] {
            let body = format!("{HEAD}links = [{:?}]\n", bad);
            assert!(
                parse(&body).is_err(),
                "`links = [{bad:?}]` was accepted and should not be"
            );
        }
    }

    #[test]
    fn links_must_be_an_array_of_strings() {
        assert!(parse(&format!("{HEAD}links = \"SDL2\"\n")).is_err());
        assert!(parse(&format!("{HEAD}links = 3\n")).is_err());
    }

    #[test]
    fn a_misspelled_key_is_refused_rather_than_ignored() {
        // `link` instead of `links` would silently leave the library off the
        // line, which is the failure this whole field exists to remove.
        assert!(parse(&format!("{HEAD}link = [\"SDL2\"]\n")).is_err());
    }
}

#[cfg(test)]
mod links_shape_tests {
    use super::*;

    const HEAD: &str = "[package]\nname = \"p\"\nversion = \"1.0.0\"\n";

    #[test]
    fn a_name_may_carry_a_dash_or_dot_but_not_lead_with_one() {
        // Real libraries have both in them, and `-lSDL2` has a dash too --
        // which is why only the first character decides.
        for good in ["SDL2-2.0", "stdc++abi", "z", "a.b", "_x", "SDL2_ttf"] {
            let body = format!("{HEAD}links = [{good:?}]\n");
            assert!(
                Manifest::parse(&body, Path::new("/tmp/x")).is_ok(),
                "`{good}` is a library name and was refused"
            );
        }
        for bad in ["-lSDL2", "-L/usr/lib", ".hidden", "./lib", "-"] {
            let body = format!("{HEAD}links = [{bad:?}]\n");
            assert!(
                Manifest::parse(&body, Path::new("/tmp/x")).is_err(),
                "`{bad}` is a flag or a path and was accepted"
            );
        }
    }
}
