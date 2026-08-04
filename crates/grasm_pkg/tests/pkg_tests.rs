//! Package manager tests: manifests, resolution, capability enforcement,
//! and lockfile integrity.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use grasm_pkg::{Manifest, MANIFEST_NAME};

/// A scratch workspace that cleans itself up.
struct Workspace(PathBuf);

impl Workspace {
    fn new(tag: &str) -> Workspace {
        let dir = std::env::temp_dir().join(format!(
            "grasm_pkg_test_{}_{}_{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Workspace(dir)
    }

    /// Writes a package with a manifest and an entry source file.
    fn package(&self, name: &str, manifest: &str, source: &str) -> PathBuf {
        let dir = self.0.join(name);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join(MANIFEST_NAME), manifest).unwrap();
        std::fs::write(dir.join("src/main.gr"), source).unwrap();
        dir
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PURE_SRC: &str = "pub fn double(n: i64) -> i64 {\n    n * 2\n}\n\nfn main() {\n}\n";
const IO_SRC: &str = "pub fn shout(s: str) {\n    print(s)\n}\n\nfn main() {\n    shout(\"hi\")\n}\n";

fn manifest(name: &str, caps: Option<&str>, deps: &[(&str, &str)]) -> String {
    let mut m = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n");
    if let Some(c) = caps {
        m.push_str(&format!("capabilities = [{c}]\n"));
    }
    for (dep, path) in deps {
        m.push_str(&format!("\n[dependencies.{dep}]\npath = \"{path}\"\n"));
    }
    m
}

// ---------------------------------------------------------------------------
// Manifests
// ---------------------------------------------------------------------------

#[test]
fn parses_a_full_manifest() {
    let m = Manifest::parse(
        &manifest("demo", Some("\"io\", \"Clock\""), &[("utils", "../utils")]),
        Path::new("/tmp"),
    )
    .unwrap();
    assert_eq!(m.name, "demo");
    assert_eq!(m.version, "0.1.0");
    assert_eq!(m.entry, "src/main.gr");
    let caps = m.capabilities.unwrap();
    assert!(caps.contains("io") && caps.contains("Clock"));
    assert_eq!(m.dependencies.len(), 1);
    assert_eq!(m.dependencies[0].path, "../utils");
}

#[test]
fn absent_capabilities_means_unrestricted() {
    let m = Manifest::parse(&manifest("demo", None, &[]), Path::new("/tmp")).unwrap();
    assert!(m.capabilities.is_none());
}

#[test]
fn malformed_manifests_are_rejected_with_reasons() {
    let cases = [
        ("[package]\nversion = \"1.0.0\"\n", "missing `name`"),
        ("[package]\nname = \"x\"\n", "missing `version`"),
        ("[package]\nname = \"x\"\nversion = \"1.0\"\n", "MAJOR.MINOR.PATCH"),
        ("[package]\nname = \"x\"\nversion = \"a.b.c\"\n", "MAJOR.MINOR.PATCH"),
        ("name = \"x\"\n", "[package]"),
        (
            "[package]\nname = \"x\"\nversion = \"1.0.0\"\ncapabilities = \"io\"\n",
            "must be an array",
        ),
    ];
    for (text, needle) in cases {
        let e = Manifest::parse(text, Path::new("/tmp")).expect_err("should reject");
        assert!(e.contains(needle), "for {text:?} expected {needle:?}, got {e:?}");
    }
}

#[test]
fn dependencies_without_a_path_are_rejected() {
    let e = Manifest::parse(
        "[package]\nname = \"x\"\nversion = \"1.0.0\"\n\n[dependencies.thing]\nversion = \"1\"\n",
        Path::new("/tmp"),
    )
    .expect_err("registry deps are not implemented");
    assert!(e.contains("needs a `path`"), "got: {e}");
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

#[test]
fn resolves_dependencies_before_dependents() {
    let ws = Workspace::new("order");
    ws.package("leaf", &manifest("leaf", Some(""), &[]), PURE_SRC);
    ws.package(
        "mid",
        &manifest("mid", Some(""), &[("leaf", "../leaf")]),
        PURE_SRC,
    );
    let root = ws.package(
        "root",
        &manifest("root", Some(""), &[("mid", "../mid")]),
        PURE_SRC,
    );

    let r = grasm_pkg::resolve(&root).unwrap();
    let names: Vec<&str> = r.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["leaf", "mid", "root"]);
    assert_eq!(r.root().name, "root");
}

#[test]
fn a_diamond_loads_each_package_once() {
    let ws = Workspace::new("diamond");
    ws.package("base", &manifest("base", Some(""), &[]), PURE_SRC);
    ws.package(
        "left",
        &manifest("left", Some(""), &[("base", "../base")]),
        PURE_SRC,
    );
    ws.package(
        "right",
        &manifest("right", Some(""), &[("base", "../base")]),
        PURE_SRC,
    );
    let root = ws.package(
        "top",
        &manifest("top", Some(""), &[("left", "../left"), ("right", "../right")]),
        PURE_SRC,
    );

    let r = grasm_pkg::resolve(&root).unwrap();
    let names: Vec<&str> = r.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names.iter().filter(|n| **n == "base").count(), 1);
    assert_eq!(names.len(), 4);
}

#[test]
fn cycles_are_reported_not_hung_on() {
    let ws = Workspace::new("cycle");
    ws.package("a", &manifest("a", Some(""), &[("b", "../b")]), PURE_SRC);
    ws.package("b", &manifest("b", Some(""), &[("a", "../a")]), PURE_SRC);

    let e = grasm_pkg::resolve(&ws.path().join("a")).expect_err("cycle must be caught");
    assert!(e.contains("dependency cycle"), "got: {e}");
    assert!(e.contains("a") && e.contains("b"), "cycle should name the path: {e}");
}

#[test]
fn a_missing_dependency_names_the_path_it_looked_in() {
    let ws = Workspace::new("missing");
    let root = ws.package(
        "solo",
        &manifest("solo", Some(""), &[("ghost", "../ghost")]),
        PURE_SRC,
    );
    let e = grasm_pkg::resolve(&root).expect_err("missing dep must fail");
    assert!(e.contains("ghost"), "got: {e}");
    assert!(e.contains(MANIFEST_NAME), "got: {e}");
}

#[test]
fn a_dependency_whose_name_disagrees_is_rejected() {
    let ws = Workspace::new("mismatch");
    ws.package("real", &manifest("actually_other", Some(""), &[]), PURE_SRC);
    let root = ws.package(
        "app",
        &manifest("app", Some(""), &[("real", "../real")]),
        PURE_SRC,
    );
    let e = grasm_pkg::resolve(&root).expect_err("name mismatch must fail");
    assert!(e.contains("resolves to a package named"), "got: {e}");
}

// ---------------------------------------------------------------------------
// Capability enforcement — the supply-chain guarantee
// ---------------------------------------------------------------------------

fn effects(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_package_within_its_ceiling_passes() {
    let m = Manifest::parse(&manifest("ok", Some("\"io\""), &[]), Path::new("/tmp")).unwrap();
    assert!(grasm_pkg::check_capabilities(&m, &effects(&["io"])).is_none());
    assert!(grasm_pkg::check_capabilities(&m, &effects(&[])).is_none());
}

#[test]
fn a_package_exceeding_its_ceiling_is_reported() {
    let m = Manifest::parse(&manifest("pure", Some(""), &[]), Path::new("/tmp")).unwrap();
    let v = grasm_pkg::check_capabilities(&m, &effects(&["io"]))
        .expect("io beyond an empty ceiling must be caught");
    assert_eq!(v.package, "pure");
    assert_eq!(v.exceeded, vec!["io"]);
    let msg = v.to_string();
    assert!(msg.contains("performs `io`"), "got: {msg}");
    assert!(msg.contains("only permits nothing"), "got: {msg}");
}

#[test]
fn an_unrestricted_package_is_never_in_violation() {
    let m = Manifest::parse(&manifest("free", None, &[]), Path::new("/tmp")).unwrap();
    assert!(grasm_pkg::check_capabilities(&m, &effects(&["io", "async"])).is_none());
}

#[test]
fn effects_are_inferred_from_real_source() {
    let ws = Workspace::new("infer");
    let pure = ws.package("purepkg", &manifest("purepkg", Some(""), &[]), PURE_SRC);
    let noisy = ws.package("noisypkg", &manifest("noisypkg", Some(""), &[]), IO_SRC);

    let pure_m = Manifest::load(&pure).unwrap();
    let noisy_m = Manifest::load(&noisy).unwrap();

    assert!(grasm_pkg::effects_of_package(&pure_m).unwrap().is_empty());
    assert!(grasm_pkg::effects_of_package(&noisy_m)
        .unwrap()
        .contains("io"));
}

/// The scenario the feature exists for: a dependency starts doing something new
/// in a "safe" update, and consumers find out at build time.
#[test]
fn a_dependency_that_starts_doing_io_breaks_the_build() {
    let ws = Workspace::new("supplychain");
    let lib = ws.package("lib", &manifest("lib", Some(""), &[]), PURE_SRC);
    let app = ws.package(
        "app",
        &manifest("app", Some("\"io\""), &[("lib", "../lib")]),
        IO_SRC,
    );

    // Before: everything within its declared ceiling.
    let r = grasm_pkg::resolve(&app).unwrap();
    for pkg in &r.packages {
        let performed = grasm_pkg::effects_of_package(pkg).unwrap();
        assert!(
            grasm_pkg::check_capabilities(pkg, &performed).is_none(),
            "`{}` should be clean initially",
            pkg.name
        );
    }

    // The library ships an "innocuous" update that reads the filesystem.
    std::fs::write(
        lib.join("src/main.gr"),
        "pub fn double(n: i64) -> i64 {\n    n * 2\n}\n\nfn main() {\n    if file_exists(\"/etc/passwd\") {\n        print(\"gotcha\")\n    }\n}\n",
    )
    .unwrap();

    let r = grasm_pkg::resolve(&app).unwrap();
    let lib_m = r.packages.iter().find(|p| p.name == "lib").unwrap();
    let performed = grasm_pkg::effects_of_package(lib_m).unwrap();
    let violation = grasm_pkg::check_capabilities(lib_m, &performed)
        .expect("the update must be caught by the capability ceiling");
    assert_eq!(violation.package, "lib");
    assert!(violation.exceeded.contains(&"io".to_string()));
}

// ---------------------------------------------------------------------------
// Lockfile
// ---------------------------------------------------------------------------

#[test]
fn checksums_are_stable_and_content_sensitive() {
    let ws = Workspace::new("lock");
    let pkg = ws.package("thing", &manifest("thing", Some(""), &[]), PURE_SRC);
    let m = Manifest::load(&pkg).unwrap();

    let a = grasm_pkg::package_checksum(&m).unwrap();
    let b = grasm_pkg::package_checksum(&m).unwrap();
    assert_eq!(a, b, "hashing the same tree twice must agree");
    assert_eq!(a.len(), 64);

    std::fs::write(pkg.join("src/main.gr"), format!("{PURE_SRC}// changed\n")).unwrap();
    let c = grasm_pkg::package_checksum(&m).unwrap();
    assert_ne!(a, c, "a source change must change the checksum");
}

#[test]
fn lockfile_records_every_package_and_reparses() {
    let ws = Workspace::new("lockfile");
    ws.package("dep", &manifest("dep", Some(""), &[]), PURE_SRC);
    let root = ws.package(
        "main_pkg",
        &manifest("main_pkg", Some("\"io\""), &[("dep", "../dep")]),
        IO_SRC,
    );

    let r = grasm_pkg::resolve(&root).unwrap();
    let entries = grasm_pkg::build_lock(&r).unwrap();
    assert_eq!(entries.len(), 2);

    let text = grasm_pkg::render_lock(&entries);
    assert!(text.contains("[[package]]"));
    assert!(text.contains("name = \"dep\""));
    assert!(text.contains("name = \"main_pkg\""));
    assert!(text.contains("capabilities = [\"io\"]"));
    // Every checksum is a full-length hex digest.
    for e in &entries {
        assert_eq!(e.checksum.len(), 64, "{} has a short checksum", e.name);
    }
}

#[test]
fn entry_file_must_exist() {
    let ws = Workspace::new("noentry");
    let dir = ws.path().join("broken");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(MANIFEST_NAME), manifest("broken", Some(""), &[])).unwrap();
    let m = Manifest::load(&dir).unwrap();
    let e = grasm_pkg::effects_of_package(&m).expect_err("missing entry must fail");
    assert!(e.contains("does not exist"), "got: {e}");
}
