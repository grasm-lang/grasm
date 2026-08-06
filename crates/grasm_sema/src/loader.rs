//! Multi-file program loader.
//!
//! `import utils` loads `utils.gr` next to the entry file; `import lib.math`
//! loads `lib/math.gr`. Each file's spans are shifted into one virtual span
//! space (per-file base offsets), so every later stage keeps a single flat
//! diagnostic list; the CLI maps spans back to files for rendering.

use std::collections::HashMap;

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
    load_with_paths(entry_name, entry_display_path, provider, &|_| Vec::new())
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
        let mut imports = HashMap::new();
        for decl in &out.module.decls {
            if let grasm_syntax::ast::Decl::Import(imp) = decl {
                let full = imp.path.dotted();
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
    }
}

/// Disk loader: resolves module names relative to the entry file's directory.
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
    let entry_src_dir = entry_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let entry_display = entry_path.display().to_string();
    let entry_src = std::fs::read_to_string(entry_path)
        .map_err(|e| format!("cannot read `{entry_display}`: {e}"))?;
    // A file inside a package can import that package's dependencies by name,
    // not just its own sibling files.
    let mut deps = package_dependencies(&entry_src_dir);
    // Caller-supplied entries win: they came from real resolution.
    for (name, dir) in resolved {
        deps.insert(name.clone(), dir.clone());
    }

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

    let provider = move |name: &str| -> Option<String> {
        if name == "main" {
            return Some(entry_src.clone());
        }
        let rel = format!("{}.gr", name.replace('.', "/"));
        if let Ok(src) = std::fs::read_to_string(entry_src_dir.join(&rel)) {
            return Some(src);
        }
        // Then a dependency declared in grasm.toml, loaded from its entry.
        let root = name.split('.').next().unwrap_or(name);
        let dep_dir = deps.get(root)?;
        let entry = package_entry(dep_dir);
        match name.split_once('.') {
            None => std::fs::read_to_string(entry).ok(),
            // `import dep.sub` — a file inside the dependency's source tree.
            Some((_, sub)) => {
                let base = entry.parent()?.to_path_buf();
                std::fs::read_to_string(base.join(format!("{}.gr", sub.replace('.', "/")))).ok()
            }
        }
    };
    Ok(load_with_paths("main", &entry_display, &provider, &tried))
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
