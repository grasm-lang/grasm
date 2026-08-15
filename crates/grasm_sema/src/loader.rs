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
    load_with_canonical(
        entry_name,
        entry_display_path,
        provider,
        tried,
        deps,
        entry_alias,
        &|_, _| None,
    )
}

/// `load_with_alias`, told how to canonicalize a module name.
///
/// One package can be reached by two paths — declared directly *and* through a
/// dependency that also uses it — and it must be one module either way. Types
/// are program-wide ([T-MODULE-NS]), so loading it twice reported every type
/// in it as a duplicate of itself.
pub(crate) fn load_with_canonical(
    entry_name: &str,
    entry_display_path: &str,
    provider: &dyn Fn(&str) -> Option<String>,
    tried: &dyn Fn(&str) -> Vec<String>,
    deps: &HashSet<String>,
    entry_alias: Option<&str>,
    canonical: &dyn Fn(&str, &str) -> Option<String>,
) -> Program {
    load_with_resolved_imports(
        entry_name,
        entry_display_path,
        provider,
        tried,
        deps,
        entry_alias,
        canonical,
        &HashMap::new(),
    )
}

/// `load_with_canonical`, with some import targets already decided.
///
/// A bundled program carries the module names the build gave it, and those
/// names come from the package graph on disk — which the bundle does not
/// have. Re-deriving them made a three-package chain load as `b.c` where the
/// build had named it `c`, so the bundled build of a program `grasm run` and
/// `grasm build --native` both ran correctly reported a missing module.
/// Keyed by (importing module, written path).
pub fn load_with_resolved_imports(
    entry_name: &str,
    entry_display_path: &str,
    provider: &dyn Fn(&str) -> Option<String>,
    tried: &dyn Fn(&str) -> Vec<String>,
    deps: &HashSet<String>,
    entry_alias: Option<&str>,
    canonical: &dyn Fn(&str, &str) -> Option<String>,
    resolved_imports: &HashMap<(String, String), String>,
) -> Program {
    let mut files: Vec<LoadedFile> = Vec::new();
    let mut diagnostics = Vec::new();
    let mut loaded: HashMap<String, ()> = HashMap::new();
    // The span of the `import` that asked for each module, so a failure
    // points at the line that wrote it rather than at the first import in the
    // file.
    // The fourth field is the module name *as the import wrote it*, which is
    // the only spelling a reader can look for in their own file.
    let mut queue: Vec<(String, String, Span, String)> = vec![(
        entry_name.to_string(),
        entry_display_path.to_string(),
        Span::new(0, 1),
        entry_name.to_string(),
    )];
    let mut base = 0u32;

    while let Some((name, display, asked_at, written)) = queue.pop() {
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
                asked_at,
                // Named the way the import wrote it. Quoting the loader's
                // qualified form instead put a module path in front of the
                // reader that appears in no source file — `dep.helpers.math`
                // for `import helpers.math`, and one segment more per package
                // hop, so a ten-deep graph reported
                // `p8.p7.p6.p5.p4.p3.p2.p1.p0.sub.deep`.
                format!("cannot load module `{written}` ({where_})"),
            ));
            continue;
        };
        let out = grasm_syntax::parser::parse_with_offset(&src, base);
        diagnostics.extend(out.diagnostics);
        // Which package this module lives inside, as its qualified prefix.
        // The *longest* one: a dependency's own dependency is a package too,
        // and `typolog.typo` is where `typo`'s files live. Taking the first
        // segment instead made `typo`'s `import measure` resolve to
        // `typolog.measure`, a module in the wrong package.
        let owner: Option<String> = deps
            .iter()
            .filter(|d| name == **d || name.starts_with(&format!("{d}.")))
            .max_by_key(|d| d.len())
            .cloned();
        let mut imports = HashMap::new();
        for decl in &out.module.decls {
            if let grasm_syntax::ast::Decl::Import(imp) = decl {
                let written = imp.path.dotted();
                // The name this import is called by, which is what a
                // resolved map is keyed on: aliases are unique per module
                // (a duplicate is an error), and a written path is not
                // reconstructible from a bundle.
                let alias = imp
                    .path
                    .segments
                    .last()
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                // Inside a package, an import is relative to that package —
                // whether it names a sibling file or one of that package's
                // own dependencies. Nesting is carried by the prefix, so both
                // cases are the same string.
                let full = match &owner {
                    Some(pkg) => format!("{pkg}.{written}"),
                    None => written.clone(),
                };
                // The same package reached by two paths is one module —
                // asked with who is importing, because a file inside the
                // importing package wins over any package of that name.
                let full = canonical(&name, &written).unwrap_or(full);
                // A name the build already decided wins over any derivation.
                let full = resolved_imports
                    .get(&(name.clone(), alias.clone()))
                    .cloned()
                    .unwrap_or(full);
                // An import that names the entry file resolves to it.
                let full = match entry_alias {
                    Some(a) if full == a => entry_name.to_string(),
                    _ => full,
                };
                if imports.contains_key(&alias) {
                    diagnostics.push(Diagnostic::error(
                        imp.span,
                        format!("duplicate import alias `{alias}` in module `{name}`"),
                    ));
                }
                imports.insert(alias, full.clone());
                let disp = format!("{}.gr", full.replace('.', "/"));
                queue.push((full, disp, imp.span, written.clone()));
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
    fn is_root(p: &std::ffi::OsString) -> bool {
        p == std::path::MAIN_SEPARATOR_STR
    }
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for c in path.components() {
        match c {
            // `/..` is `/`, and popping the root left an empty string where
            // a path belonged.
            Component::ParentDir if parts.last().is_some_and(is_root) => {}
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
    // Collapsed like every other path a diagnostic names. A dependency is
    // reached from the consumer's directory, so its entry came out as
    // `app/../lib/src/main.gr` — correct, and not what anyone would type. The
    // capability ceiling's "performed: `io` at ..." line is the one a reader
    // is most likely to go and open.
    let entry_display = tidy(entry_path);
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

    // Every package in the graph, as the qualified prefix its modules carry.
    // A dependency's own dependencies are packages too, and a module name has
    // to say which one it belongs to.
    // Seeded from what the entry package *declares*, not from every package
    // the graph resolution found: a package reachable only through a
    // dependency is not one this package may import by name.
    let mut all_prefixes: HashMap<String, std::path::PathBuf> = HashMap::new();
    for name in dependency_names(&entry_src_dir) {
        if let Some(dir) = deps.get(&name) {
            collect_packages(&name, dir, resolved, &mut all_prefixes, 0);
        }
    }
    // One name per package. The shortest prefix wins, so a package a consumer
    // declares itself keeps the name the consumer wrote.
    let mut canon_of_dir: HashMap<std::path::PathBuf, String> = HashMap::new();
    let mut prefixes: Vec<(&String, &std::path::PathBuf)> = all_prefixes.iter().collect();
    prefixes.sort_by(|a, b| (a.0.len(), a.0).cmp(&(b.0.len(), b.0)));
    for (prefix, dir) in prefixes {
        let key = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
        canon_of_dir.entry(key).or_insert_with(|| prefix.clone());
    }
    let dep_names: HashSet<String> = canon_of_dir.values().cloned().collect();
    // A module name written under a non-canonical prefix, rewritten to the
    // canonical one: `typolog.typo.model` is `typo.model` when the consumer
    // declares `typo` itself.
    let prefix_dirs = all_prefixes.clone();
    let canon = canon_of_dir.clone();
    let entry_dir_for_canon = entry_src_dir.clone();
    let resolved_for_canon = resolved.clone();
    // Given the importing module and what it wrote, the module name this
    // import resolves to — or None to keep the one derived from the prefix.
    //
    // A file inside the *importing* package wins, which is what makes
    // `import helper` mean your own `helper.gr` ([T-MODULE-DEP]). Only when
    // there is no such file does the name mean a package, and then it means
    // the one canonical name that package has however it was reached.
    let canonical = move |importer: &str, written: &str| -> Option<String> {
        let pkg_dir: std::path::PathBuf = prefix_dirs
            .iter()
            .filter(|(p, _)| importer == p.as_str() || importer.starts_with(&format!("{p}.")))
            .max_by_key(|(p, _)| p.len())
            .map(|(_, d)| package_entry(d).parent().map(|s| s.to_path_buf()).unwrap_or_else(|| d.clone()))
            .unwrap_or_else(|| entry_dir_for_canon.clone());
        if pkg_dir
            .join(format!("{}.gr", written.replace('.', "/")))
            .exists()
        {
            return None;
        }
        let root = written.split('.').next().unwrap_or(written);
        // A package the importing package declares, by whatever name the
        // graph settled on. A *registry* dependency has no path in the
        // manifest, so without the resolved map a package reached both
        // directly and through another was loaded under two names — and the
        // second one prefixed its own private modules with a sibling
        // package's name, which resolved nowhere.
        let subs = package_dependencies(&pkg_dir);
        let dir = subs
            .get(root)
            .or_else(|| {
                dependency_names(&pkg_dir)
                    .iter()
                    .any(|d| d == root)
                    .then(|| resolved_for_canon.get(root))
                    .flatten()
            })?;
        let key = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
        let want = canon.get(&key)?;
        Some(match written.split_once('.') {
            Some((_, rest)) => format!("{want}.{rest}"),
            None => want.clone(),
        })
    };
    let dirs = entry_src_dir.clone();
    let dep_dirs = deps.clone();
    let declared = dependency_names(&entry_src_dir);
    let prefix_dirs_for_tried = all_prefixes.clone();
    // The same two candidates the provider tries, so the failure message says
    // where it actually looked instead of naming one of the two.
    let tried = move |name: &str| -> Vec<String> {
        let root = name.split('.').next().unwrap_or(name);
        // A module that belongs to a package is looked for *inside that
        // package*, and the message has to say so. Built from the entry's own
        // source directory regardless, it named `app/src/liba/nope/here.gr`
        // and the dependency's entry file — two paths the loader never opens
        // for this import — for a module whose only real candidates are in
        // `liba/src`. A "looked in" list that did not is worse than none.
        if let Some(dir) = prefix_dirs_for_tried.get(root) {
            let rest = name.split_once('.').map(|(_, r)| r).unwrap_or("");
            let mut out = Vec::new();
            candidates_in_package(dir, rest, root, 0, &mut out);
            return out;
        }
        let mut out = vec![tidy(&dirs.join(format!("{}.gr", name.replace('.', "/"))))];
        match (declared.iter().any(|d| d == root), dep_dirs.get(root)) {
            (true, Some(dir)) => out.push(format!(
                "{} (entry file of dependency `{root}`)",
                tidy(&package_entry(dir))
            )),
            // In the graph, but this package does not declare it. Naming the
            // path it would have had is worse than useless — the loader built
            // one by chaining every hop, `../a/../b/../c/src/main.gr`, which
            // exists nowhere.
            (false, Some(_)) => out.push(format!(
                "no dependency named `{root}` in grasm.toml — it is somewhere in the graph, reached through another package; add it here to import it"
            )),
            (_, None) => out.push(format!(
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
    let pkg_dirs = all_prefixes.clone();
    let entry_declares = dependency_names(&entry_src_dir);
    let provider = |name: &str| -> Option<String> {
        if name == "main" {
            return Some(entry_src.clone());
        }
        // A module that belongs to a package resolves *inside that package*.
        // Trying the entry's own source directory first let a consumer plant
        // a file with the right relative name and silently replace a
        // dependency's module — the program ran, printed the wrong answer,
        // and `grasm deps` reported the library as performing nothing while
        // the planted file read the filesystem ([T-MODULE-RESOLVE]).
        if let Some((prefix, dir)) = pkg_dirs
            .iter()
            .filter(|(p, _)| name == p.as_str() || name.starts_with(&format!("{p}.")))
            .max_by_key(|(p, _)| p.len())
        {
            let rest = name
                .strip_prefix(prefix.as_str())
                .map(|r| r.trim_start_matches('.'))
                .unwrap_or("");
            let path = resolve_in_package(dir, rest, 0)?;
            let src = std::fs::read_to_string(&path).ok()?;
            note(name, &path);
            return Some(src);
        }
        let rel = format!("{}.gr", name.replace('.', "/"));
        let sibling = entry_src_dir.join(&rel);
        if let Ok(src) = std::fs::read_to_string(&sibling) {
            note(name, &sibling);
            return Some(src);
        }
        // Then a dependency this package declares. Only its own: the graph
        // resolution hands down every package it found, and taking any of
        // them let `import deep` compile in a package whose manifest names
        // only `mid` — a hard dependency with no version requirement, absent
        // from the manifest and from minimal version selection
        // ([T-MODULE-DEP]: "a dependency the manifest declares").
        let root = name.split('.').next().unwrap_or(name);
        if !entry_declares.iter().any(|d| d == root) {
            return None;
        }
        let dep_dir = deps.get(root)?;
        let rest = name.split_once('.').map(|(_, s)| s).unwrap_or("");
        let path = resolve_in_package(dep_dir, rest, 0)?;
        let src = std::fs::read_to_string(&path).ok()?;
        note(name, &path);
        Some(src)
    };
    // The name this file would answer to if a sibling imported it.
    let entry_alias = entry_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string());
    let mut program = load_with_canonical(
        "main",
        &entry_display,
        &provider,
        &tried,
        &dep_names,
        entry_alias.as_deref(),
        &canonical,
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

/// How deep a package graph may nest.
///
/// A backstop against a cycle in a path dependency, not a limit anyone should
/// meet: at 16 a perfectly ordinary twenty-package chain stopped resolving
/// *and* stopped saying so — dependency-name resolution degraded silently
/// into dotted-module-path resolution, and the error named a file whose path
/// was every package in the chain concatenated.
const MAX_PACKAGE_DEPTH: usize = 64;

/// Every package reachable from `dir`, named by the prefix its modules carry.
fn collect_packages(
    prefix: &str,
    dir: &std::path::Path,
    resolved: &HashMap<String, std::path::PathBuf>,
    out: &mut HashMap<String, std::path::PathBuf>,
    depth: usize,
) {
    let mut expanded = std::collections::HashSet::new();
    collect_packages_in(prefix, dir, resolved, out, depth, &mut expanded)
}

fn collect_packages_in(
    prefix: &str,
    dir: &std::path::Path,
    resolved: &HashMap<String, std::path::PathBuf>,
    out: &mut HashMap<String, std::path::PathBuf>,
    depth: usize,
    expanded: &mut std::collections::HashSet<std::path::PathBuf>,
) {
    if depth > MAX_PACKAGE_DEPTH || out.contains_key(prefix) {
        return;
    }
    out.insert(prefix.to_string(), dir.to_path_buf());
    // One key per *path* through the graph, and a graph has exponentially
    // many of those: twenty ordinary packages with three dependencies each
    // took over a minute, ×3.3 for every two added. A package reached a
    // second time under a longer prefix has the same dependencies, and every
    // prefix that walk would add loses to the shorter one already recorded —
    // so the sub-tree is expanded once.
    let key = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if !expanded.insert(key) {
        return;
    }
    let src_dir = package_entry(dir)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dir.to_path_buf());
    let mut subs = package_dependencies(&src_dir);
    // A registry dependency has no path in the manifest; the toolchain
    // resolved the graph and passed the directories down.
    for name in dependency_names(&src_dir) {
        if let (false, Some(dir)) = (subs.contains_key(&name), resolved.get(&name)) {
            subs.insert(name, dir.clone());
        }
    }
    for (name, sub) in subs {
        collect_packages_in(
            &format!("{prefix}.{name}"),
            &sub,
            resolved,
            out,
            depth + 1,
            expanded,
        );
    }
}

/// A module path *inside* a package, following that package's own
/// dependencies.
///
/// `import dep.sub` is a file in the dependency's source tree — unless `sub`
/// names a dependency *of that dependency*, in which case it is that package's
/// entry file. Without the second case a library could not use a library:
/// `thin` depending on `typolog` depending on `typo` looked for
/// `typolog/typo/model.gr` under the *consumer's* source directory and
/// reported a module nobody wrote. The only way to build was for every
/// consumer to re-declare the whole transitive graph in its own manifest.
fn resolve_in_package(
    pkg_dir: &std::path::Path,
    rest: &str,
    depth: usize,
) -> Option<std::path::PathBuf> {
    let entry = package_entry(pkg_dir);
    if rest.is_empty() {
        return Some(entry);
    }
    // A path dependency can name a directory that names it back; the graph is
    // resolved elsewhere, and this is only the file lookup.
    if depth > MAX_PACKAGE_DEPTH {
        return None;
    }
    let src_dir = entry.parent()?.to_path_buf();
    // A file of this package wins, matching the entry package's own order.
    let direct = src_dir.join(format!("{}.gr", rest.replace('.', "/")));
    if direct.exists() {
        return Some(direct);
    }
    let head = rest.split('.').next().unwrap_or(rest);
    let tail = rest.split_once('.').map(|(_, t)| t).unwrap_or("");
    if let Some(dir) = package_dependencies(&src_dir).get(head) {
        return resolve_in_package(dir, tail, depth + 1);
    }
    // Nothing found: hand back the path a reader should have written, so the
    // diagnostic names something openable.
    Some(direct)
}

/// The paths `resolve_in_package` would open, in the order it opens them.
///
/// Written separately from the resolver once, it drifted: the message named
/// the consumer's source tree for an import made *inside* a dependency, and
/// the dependency's entry file for a module two levels down. A "looked in"
/// list is a promise about where the loader went, and one that is wrong is
/// worse than none — so this mirrors the resolver step for step.
fn candidates_in_package(
    pkg_dir: &std::path::Path,
    rest: &str,
    pkg_name: &str,
    depth: usize,
    out: &mut Vec<String>,
) {
    let entry = package_entry(pkg_dir);
    if rest.is_empty() {
        out.push(format!(
            "{} (entry file of dependency `{pkg_name}`)",
            tidy(&entry)
        ));
        return;
    }
    if depth > MAX_PACKAGE_DEPTH {
        // Say the limit rather than degrade into dotted-module-path
        // resolution, which produced a path made of every package name in the
        // chain and named a file nobody wrote.
        out.push(format!(
            "the package graph is more than {MAX_PACKAGE_DEPTH} deep from the entry, \
so this import was not resolved as a dependency name"
        ));
        return;
    }
    let Some(src_dir) = entry.parent().map(|p| p.to_path_buf()) else {
        return;
    };
    out.push(tidy(&src_dir.join(format!("{}.gr", rest.replace('.', "/")))));
    let head = rest.split('.').next().unwrap_or(rest);
    let tail = rest.split_once('.').map(|(_, t)| t).unwrap_or("");
    match package_dependencies(&src_dir).get(head) {
        Some(dir) => candidates_in_package(dir, tail, head, depth + 1, out),
        // The same note the entry package gets. Without it a failure inside a
        // dependency listed one path and stopped, while the identical failure
        // in the entry package said which manifest was missing the name.
        None if rest.contains('.') => out.push(format!(
            "no dependency named `{head}` in `{pkg_name}`'s grasm.toml"
        )),
        None => {}
    }
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

/// Every dependency name a package declares, whether or not it has a path.
///
/// A registry dependency has no path here; the toolchain resolves the graph
/// and passes the directories down, and this says which names to look for.
fn dependency_names(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let Some((_, text)) = find_manifest(dir) else {
        return out;
    };
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some(rest) = line.strip_prefix('[') {
            if let Some(name) = rest.trim_end_matches(']').trim().strip_prefix("dependencies.") {
                out.push(name.trim().to_string());
            }
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
