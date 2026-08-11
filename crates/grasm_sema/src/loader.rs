//! Multi-file program loader.
//!
//! `import utils` loads `utils.gr` next to the entry file; `import lib.math`
//! loads `lib/math.gr`. Each file's spans are shifted into one virtual span
//! space (per-file base offsets), so every later stage keeps a single flat
//! diagnostic list; the CLI maps spans back to files for rendering.

use std::collections::{HashMap, HashSet};

use grasm_syntax::ast::Module;
use grasm_syntax::diag::Diagnostic;
use grasm_syntax::span::Span;

pub struct LoadedFile {
    /// Full dotted module name; the entry file is always `"main"`.
    pub name: String,
    /// Short name used for qualified access (`math` for `lib.math`).
    pub alias: String,
    /// Display path for diagnostics.
    pub path: String,
    pub src: String,
    /// Base of this file's span range in the virtual span space.
    pub base: u32,
    pub module: Module,
    /// Aliases this module imported: alias → full module name.
    pub imports: HashMap<String, String>,
}

pub struct Program {
    /// files[0] is the entry module.
    pub files: Vec<LoadedFile>,
    pub diagnostics: Vec<Diagnostic>,
    /// Module names that are *dependency packages*, kept so anything that
    /// reloads this program — the doc-example harness does — can resolve
    /// imports the way the original load did. Dropping it re-created the
    /// bug the `deps` parameter exists to fix, one layer up.
    pub deps: HashSet<String>,
}

impl Program {
    /// Which file owns a span, for diagnostic rendering.
    pub fn file_of(&self, span: Span) -> usize {
        self.files
            .iter()
            .rposition(|f| span.start >= f.base)
            .unwrap_or(0)
    }
}

/// Loads from an arbitrary source provider (module name → source).
/// Used both by the disk path and by bundled executables.
pub fn load_with(
    entry_name: &str,
    entry_display_path: &str,
    provider: &dyn Fn(&str) -> Option<String>,
) -> Program {
    load_with_paths_in(entry_name, entry_display_path, provider, &|_| Vec::new(), &HashSet::new())
}

/// `load_with`, plus a way to say where a missing module was looked for.
///
/// A resolution failure has to name every place that was tried. Naming one of
/// them is worse than naming none: an import of a package resolves through the
/// package's entry file, and reporting only the sibling-file candidate sent a
/// reader hunting for a filename the toolchain never wanted.
pub fn load_with_paths(
    entry_name: &str,
    entry_display_path: &str,
    provider: &dyn Fn(&str) -> Option<String>,
    tried: &dyn Fn(&str) -> Vec<String>,
) -> Program {
    load_with_paths_in(entry_name, entry_display_path, provider, tried, &HashSet::new())
}

/// `load_with_paths`, told which module names are *dependencies*.
///
/// A module inside a dependency resolves its own imports inside that
/// dependency. Without this, `import sub.helper` in a library was looked up in
/// the *consumer's* source directory: a same-named file there silently
/// replaced the library's own private module, and the program ran and printed
/// the wrong answer with nothing reported.
pub fn load_with_paths_in(
    entry_name: &str,
    entry_display_path: &str,
    provider: &dyn Fn(&str) -> Option<String>,
    tried: &dyn Fn(&str) -> Vec<String>,
    deps: &HashSet<String>,
) -> Program {
    load_with_alias(entry_name, entry_display_path, provider, tried, deps, None)
}

/// `load_with_paths_in`, told which module name *is* the entry file.
///
/// A file may be both the entry and a module its own modules import — mutual
/// imports are legal, and `alpha.gr` importing `beta.gr` which imports `alpha`
/// is the ordinary shape of it. Loading it a second time under that name gave
/// two copies of every declaration in it, so a type declared once was reported
/// as a duplicate at its sole declaration, and `grasm test` ran each of its
/// tests twice. The import resolves to the entry instead.
pub(crate) fn load_with_alias(
    entry_name: &str,
    entry_display_path: &str,
    provider: &dyn Fn(&str) -> Option<String>,
    tried: &dyn Fn(&str) -> Vec<String>,
    deps: &HashSet<String>,
    entry_alias: Option<&str>,
) -> Program {
    let mut files: Vec<LoadedFile> = Vec::new();
    let mut diagnostics = Vec::new();
    let mut loaded: HashMap<String, ()> = HashMap::new();
    let mut queue: Vec<(String, String)> = vec![(entry_name.to_string(), entry_display_path.to_string())];
    let mut base = 0u32;

    while let Some((name, display)) = queue.pop() {
        if loaded.contains_key(&name) {
            continue;
        }
        loaded.insert(name.clone(), ());
        let Some(src) = provider(&name) else {
            let candidates = tried(&name);
            let where_ = if candidates.is_empty() {
                format!("looked for `{display}`")
            } else {
                format!("looked in:\n  {}", candidates.join("\n  "))
            };
            diagnostics.push(Diagnostic::error(
                Span::new(0, 1),
                format!("cannot load module `{name}` ({where_})"),
            ));
            continue;
        };
        let out = grasm_syntax::parser::parse_with_offset(&src, base);
        diagnostics.extend(out.diagnostics);
        // Which dependency, if any, this module lives inside.
        let owner: Option<String> = if deps.contains(&name) {
            Some(name.clone())
        } else {
            name.split_once('.')
                .map(|(root, _)| root.to_string())
                .filter(|root| deps.contains(root))
        };
        let mut imports = HashMap::new();
        for decl in &out.module.decls {
            if let grasm_syntax::ast::Decl::Import(imp) = decl {
                let written = imp.path.dotted();
                // Inside a dependency, a relative import stays inside it —
                // unless it names another dependency, which is absolute.
                let root = written.split('.').next().unwrap_or(&written);
                let full = match &owner {
                    Some(pkg) if !deps.contains(root) => format!("{pkg}.{written}"),
                    _ => written,
                };
                // An import that names the entry file resolves to it.
                let full = match entry_alias {
                    Some(a) if full == a => entry_name.to_string(),
                    _ => full,
                };
                let alias = imp
                    .path
                    .segments
                    .last()
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                if imports.contains_key(&alias) {
                    diagnostics.push(Diagnostic::error(
                        imp.span,
                        format!("duplicate import alias `{alias}` in module `{name}`"),
                    ));
                }
                imports.insert(alias, full.clone());
                let disp = format!("{}.gr", full.replace('.', "/"));
                queue.push((full, disp));
            }
        }
        let alias = name
            .rsplit('.')
            .next()
            .unwrap_or(&name)
            .to_string();
        let len = src.len() as u32;
        files.push(LoadedFile {
            name,
            alias,
            path: display,
            src,
            base,
            module: out.module,
            imports,
        });
        base += len + 1; // +1 keeps ranges disjoint
    }

    Program {
        files,
        diagnostics,
        deps: deps.clone(),
    }
}

/// Disk loader: resolves module names relative to the entry file's directory.
/// A path as a reader would write it.
///
/// A dependency is reached from the consumer's directory, so its files came
/// out as `taskflow/../textkit/src/main.gr` — correct, and not what anyone
/// would type. The collapse is lexical: these paths have already been opened,
/// so there is no symlink question to resolve, and touching the filesystem to
/// print a diagnostic would be the wrong trade.
fn tidy(path: &std::path::Path) -> String {
    use std::path::Component;
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for c in path.components() {
        match c {
            Component::ParentDir if parts.last().is_some_and(|p| p != "..") => {
                parts.pop();
            }
            Component::CurDir => {}
            other => parts.push(other.as_os_str().to_os_string()),
        }
    }
    let mut out = std::path::PathBuf::new();
    for p in parts {
        out.push(p);
    }
    out.display().to_string()
}

pub fn load_from_disk(entry_path: &std::path::Path) -> Result<Program, String> {
    load_from_disk_with(entry_path, &HashMap::new())
}

/// `load_from_disk`, plus package directories the caller resolved itself.
///
/// A path dependency is readable straight from the manifest, so the loader
/// handles those alone. A registry dependency is not: which version to use is
/// a decision about the whole graph, and the crate that makes it depends on
/// this one. So the toolchain resolves first and passes the answer down,
/// rather than this file growing a second, subtly different resolver.
pub fn load_from_disk_with(
    entry_path: &std::path::Path,
    resolved: &HashMap<String, std::path::PathBuf>,
) -> Result<Program, String> {
    load_from_disk_in(entry_path, resolved, None)
}

/// `load_from_disk_with`, with the entry file's text supplied by the caller.
///
/// An editor holds the buffer being typed into, which is not what is on disk.
/// Without this the language server could only analyse a single file with no
/// imports resolved at all, so every use of an imported name was reported as
/// unknown — twenty-two errors on a file `grasm check` passes, and the real
/// error somewhere among them.
pub fn load_from_disk_in(
    entry_path: &std::path::Path,
    resolved: &HashMap<String, std::path::PathBuf>,
    entry_override: Option<String>,
) -> Result<Program, String> {
    let entry_src_dir = entry_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let entry_display = entry_path.display().to_string();
    let entry_src = match entry_override {
        Some(src) => src,
        None => std::fs::read_to_string(entry_path)
            .map_err(|e| format!("cannot read `{entry_display}`: {e}"))?,
    };
    // A file inside a package can import that package's dependencies by name,
    // not just its own sibling files.
    let mut deps = package_dependencies(&entry_src_dir);
    // Caller-supplied entries win: they came from real resolution.
    for (name, dir) in resolved {
        deps.insert(name.clone(), dir.clone());
    }

    let dep_names: HashSet<String> = deps.keys().cloned().collect();
    let dirs = entry_src_dir.clone();
    let dep_dirs = deps.clone();
    // The same two candidates the provider tries, so the failure message says
    // where it actually looked instead of naming one of the two.
    let tried = move |name: &str| -> Vec<String> {
        let mut out = vec![dirs
            .join(format!("{}.gr", name.replace('.', "/")))
            .display()
            .to_string()];
        let root = name.split('.').next().unwrap_or(name);
        match dep_dirs.get(root) {
            Some(dir) => out.push(format!(
                "{} (entry file of dependency `{root}`)",
                package_entry(dir).display()
            )),
            None => out.push(format!(
                "no dependency named `{root}` in grasm.toml"
            )),
        }
        out
    };

    // Where each module actually came from. A diagnostic has to name a path a
    // reader can open, and the one the loader derives from a module name is
    // only right for a sibling file: a dependency's module `liba.helper`
    // became `liba/helper.gr`, which exists nowhere — not relative to the
    // working directory, not relative to the manifest. Any editor or CI
    // annotation that followed it landed on nothing.
    let found: std::cell::RefCell<HashMap<String, String>> =
        std::cell::RefCell::new(HashMap::new());
    let note = |name: &str, path: &std::path::Path| {
        found.borrow_mut().insert(name.to_string(), tidy(path));
    };
    let provider = |name: &str| -> Option<String> {
        if name == "main" {
            return Some(entry_src.clone());
        }
        let rel = format!("{}.gr", name.replace('.', "/"));
        let sibling = entry_src_dir.join(&rel);
        if let Ok(src) = std::fs::read_to_string(&sibling) {
            note(name, &sibling);
            return Some(src);
        }
        // Then a dependency declared in grasm.toml, loaded from its entry.
        let root = name.split('.').next().unwrap_or(name);
        let dep_dir = deps.get(root)?;
        let entry = package_entry(dep_dir);
        let path = match name.split_once('.') {
            None => entry,
            // `import dep.sub` — a file inside the dependency's source tree.
            Some((_, sub)) => entry
                .parent()?
                .to_path_buf()
                .join(format!("{}.gr", sub.replace('.', "/"))),
        };
        let src = std::fs::read_to_string(&path).ok()?;
        note(name, &path);
        Some(src)
    };
    // The name this file would answer to if a sibling imported it.
    let entry_alias = entry_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string());
    let mut program = load_with_alias(
        "main",
        &entry_display,
        &provider,
        &tried,
        &dep_names,
        entry_alias.as_deref(),
    );
    // The real path wins over the one derived from the module name.
    let found = found.into_inner();
    for f in &mut program.files {
        if let Some(real) = found.get(&f.name) {
            f.path.clone_from(real);
        }
    }
    Ok(program)
}

/// Dependencies of the package containing `dir`, as name → package directory.
///
/// Walks up looking for a `grasm.toml`, so any file in the package can import
/// what the package declared. Reads only what an import needs — the dependency
/// names and their paths — so the loader stays free of a manifest parser it
/// would otherwise have to depend on.
fn package_dependencies(dir: &std::path::Path) -> HashMap<String, std::path::PathBuf> {
    let mut out = HashMap::new();
    let Some((root, text)) = find_manifest(dir) else {
        return out;
    };
    let mut current: Option<String> = None;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest.trim_end_matches(']').trim();
            current = name
                .strip_prefix("dependencies.")
                .map(|n| n.trim().to_string());
            continue;
        }
        let (Some(dep), Some((key, value))) = (&current, line.split_once('=')) else {
            continue;
        };
        if key.trim() == "path" {
            let path = value.trim().trim_matches('"');
            out.insert(dep.clone(), root.join(path));
        }
    }
    out
}

fn find_manifest(dir: &std::path::Path) -> Option<(std::path::PathBuf, String)> {
    let mut here = Some(dir.to_path_buf());
    while let Some(d) = here {
        let manifest = d.join("grasm.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            return Some((d, text));
        }
        here = d.parent().map(|p| p.to_path_buf());
    }
    None
}

/// A package's entry file: its declared `entry`, or `src/main.gr`.
fn package_entry(dir: &std::path::Path) -> std::path::PathBuf {
    let entry = std::fs::read_to_string(dir.join("grasm.toml"))
        .ok()
        .and_then(|text| {
            text.lines()
                .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
                .find_map(|l| {
                    let (k, v) = l.split_once('=')?;
                    (k.trim() == "entry").then(|| v.trim().trim_matches('"').to_string())
                })
        })
        .unwrap_or_else(|| "src/main.gr".to_string());
    dir.join(entry)
}
