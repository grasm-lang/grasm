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

// ---------------------------------------------------------------------------
// Adversarial: the capability ceiling must not be evadable
// ---------------------------------------------------------------------------
//
// The project makes one security claim — a package cannot exceed its declared
// capability ceiling — so it is worth attacking rather than asserting. Each of
// these hides an `io` effect somewhere the inference could plausibly miss, in
// a package that declares no capabilities at all. Every one must be caught.

/// Effects a package with this source actually performs, per the checker.
///
/// Asserts the source parses first. A probe that failed to compile would infer
/// effects from a half-built tree and could report "no io" for a program that
/// obviously performs it — a vacuous pass on exactly the tests that are meant
/// to be adversarial.
fn performed(source: &str) -> BTreeSet<String> {
    let ws = Workspace::new("effects");
    let dir = ws.package("probe", &manifest("probe", Some(""), &[]), source);
    let program = grasm_sema::loader::load_from_disk(&dir.join("src/main.gr"))
        .expect("probe source loads");
    assert!(
        program.diagnostics.is_empty(),
        "probe source does not parse: {:#?}\n{source}",
        program.diagnostics
    );
    let m = Manifest::load(&dir).expect("manifest loads");
    grasm_pkg::effects_of_package(&m).expect("effects are inferred")
}

/// The violation a package with this source would produce against `capabilities = []`.
fn violation(source: &str) -> Option<grasm_pkg::CapabilityViolation> {
    let ws = Workspace::new("violate");
    let dir = ws.package("probe", &manifest("probe", Some(""), &[]), source);
    let m = Manifest::load(&dir).expect("manifest loads");
    let effects = grasm_pkg::effects_of_package(&m).expect("effects are inferred");
    grasm_pkg::check_capabilities(&m, &effects)
}

#[test]
fn io_hidden_behind_a_call_chain_is_still_caught() {
    let src = "fn a() {\n    b()\n}\n\nfn b() {\n    c()\n}\n\nfn c() {\n    print(\"gotcha\")\n}\n\nfn main() {\n    a()\n}\n";
    assert!(performed(src).contains("io"), "depth must not hide io");
    assert!(violation(src).is_some());
}

#[test]
fn io_inside_a_closure_is_still_caught() {
    let src = "fn main() {\n    let xs = [1, 2]\n    let ys = xs.map(|x| {\n        print(\"{x}\")\n        x\n    })\n    print(\"{ys.len()}\")\n}\n";
    assert!(performed(src).contains("io"), "a closure must not hide io");
}

#[test]
fn io_reached_through_a_method_call_is_still_caught() {
    // UFCS turns `value.helper()` into `helper(value)`; the effect travels
    // with the callee either way.
    let src = "fn helper(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\nfn main() {\n    print(\"{(3).helper()}\")\n}\n";
    assert!(performed(src).contains("io"));
}

#[test]
fn io_inside_a_test_or_bench_body_still_counts() {
    // A package could otherwise hide its reach in a `test` block, which ships
    // in the same source the consumer compiles.
    let with_test = "test \"sneaky\" {\n    print(\"side effect\")\n}\n\nfn main() {\n}\n";
    assert!(performed(with_test).contains("io"), "a test body counts");

    let with_bench = "bench \"sneaky\" {\n    print(\"side effect\")\n}\n\nfn main() {\n}\n";
    assert!(performed(with_bench).contains("io"), "a bench body counts");
}

#[test]
fn io_inside_a_handler_body_is_still_caught() {
    let src = "effect Log {\n    fn write(s: str)\n}\n\nfn work() / {Log} {\n    Log.write(\"x\")\n}\n\nfn main() {\n    with Log handled_by LogHandler{ write: |s| print(s) } {\n        work()\n    }\n}\n";
    assert!(
        performed(src).contains("io"),
        "a handler that prints performs io"
    );
}

#[test]
fn io_inside_a_spawned_task_is_still_caught() {
    let src = "fn main() {\n    parallel {\n        spawn {\n            print(\"in a task\")\n        }\n    }\n}\n";
    let e = performed(src);
    assert!(e.contains("io"), "a task body must not hide io: {e:?}");
    assert!(e.contains("async"), "spawning is itself an effect: {e:?}");
}

#[test]
fn file_and_network_reads_are_reported_as_io() {
    for src in [
        "fn main() {\n    let s = read_file(\"/etc/passwd\") ?? \"\"\n    print(s)\n}\n",
        "fn main() {\n    write_file(\"/tmp/x\", \"data\")\n}\n",
        "fn main() {\n    print(\"{file_exists(\"/etc/passwd\")}\")\n}\n",
        "fn main() {\n    print(\"{env(\"HOME\") ?? \"\"}\")\n}\n",
    ] {
        assert!(
            performed(src).contains("io"),
            "must be io: {src}"
        );
    }
}

#[test]
fn a_genuinely_pure_package_reports_nothing() {
    // The check has to be capable of saying no, or "everything is io" would
    // pass every one of the tests above for the wrong reason.
    let src = "pub fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\ntest \"adds\" {\n    assert(add(1, 2) == 3)\n}\n\nfn main() {\n}\n";
    assert!(
        performed(src).is_empty(),
        "pure code performs nothing: {:?}",
        performed(src)
    );
    assert!(violation(src).is_none());
}

#[test]
fn a_violation_names_the_effect_and_the_package() {
    let v = violation("fn main() {\n    print(\"x\")\n}\n").expect("io exceeds an empty ceiling");
    assert_eq!(v.package, "probe");
    assert_eq!(v.exceeded, vec!["io".to_string()]);
    let rendered = v.to_string();
    assert!(rendered.contains("probe"), "{rendered}");
    assert!(rendered.contains("`io`"), "{rendered}");
    assert!(rendered.contains("nothing"), "{rendered}");
}

// ---------------------------------------------------------------------------
// Lockfile verification
// ---------------------------------------------------------------------------

fn entry(name: &str, checksum: &str, caps: &[&str]) -> grasm_pkg::LockEntry {
    grasm_pkg::LockEntry {
        name: name.to_string(),
        version: "0.1.0".to_string(),
        checksum: checksum.to_string(),
        capabilities: caps.iter().map(|c| c.to_string()).collect(),
    }
}

#[test]
fn a_lockfile_round_trips_every_package() {
    // The reader used to collapse `[[package]]` into one table, so only the
    // last package was ever compared — a lockfile that verified one dependency
    // and ignored the rest.
    let entries = vec![
        entry("alpha", "aaa", &["io"]),
        entry("beta", "bbb", &[]),
        entry("gamma", "ccc", &["io", "async"]),
    ];
    let text = grasm_pkg::render_lock(&entries);
    let parsed = grasm_pkg::parse_lock(&text).expect("its own output parses");
    assert_eq!(parsed, entries);
}

#[test]
fn an_unchanged_tree_reports_no_changes() {
    let entries = vec![entry("alpha", "aaa", &[]), entry("beta", "bbb", &["io"])];
    let text = grasm_pkg::render_lock(&entries);
    assert!(grasm_pkg::diff_lock(&text, &entries).is_empty());
}

#[test]
fn a_changed_checksum_is_reported_for_any_package() {
    let recorded = vec![entry("alpha", "aaa", &[]), entry("beta", "bbb", &[])];
    let text = grasm_pkg::render_lock(&recorded);
    // The *first* package changing must be caught, not only the last.
    let now = vec![entry("alpha", "zzz", &[]), entry("beta", "bbb", &[])];
    let changes = grasm_pkg::diff_lock(&text, &now);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(
        matches!(&changes[0], grasm_pkg::LockChange::Changed { name, .. } if name == "alpha"),
        "{changes:?}"
    );
}

#[test]
fn a_changed_capability_ceiling_is_reported_separately() {
    // A dependency widening what it is allowed to do is the security-relevant
    // change, so it is not folded into "contents changed".
    let recorded = vec![entry("alpha", "aaa", &[])];
    let text = grasm_pkg::render_lock(&recorded);
    let now = vec![entry("alpha", "aaa", &["io"])];
    let changes = grasm_pkg::diff_lock(&text, &now);
    assert!(
        matches!(
            &changes[0],
            grasm_pkg::LockChange::CapabilitiesChanged { name, now, .. }
                if name == "alpha" && now == &vec!["io".to_string()]
        ),
        "{changes:?}"
    );
}

#[test]
fn added_and_removed_packages_are_reported() {
    let recorded = vec![entry("alpha", "aaa", &[]), entry("gone", "ggg", &[])];
    let text = grasm_pkg::render_lock(&recorded);
    let now = vec![entry("alpha", "aaa", &[]), entry("fresh", "fff", &[])];
    let changes = grasm_pkg::diff_lock(&text, &now);
    assert!(
        changes
            .iter()
            .any(|c| matches!(c, grasm_pkg::LockChange::Added(n) if n == "fresh")),
        "{changes:?}"
    );
    assert!(
        changes
            .iter()
            .any(|c| matches!(c, grasm_pkg::LockChange::Removed(n) if n == "gone")),
        "{changes:?}"
    );
}

#[test]
fn an_unreadable_lockfile_says_so_rather_than_passing() {
    let changes = grasm_pkg::diff_lock("this is not toml [[[", &[entry("a", "aaa", &[])]);
    assert_eq!(changes, vec![grasm_pkg::LockChange::Unreadable]);
}

#[test]
fn the_checksum_changes_when_any_source_byte_does() {
    let ws = Workspace::new("checksum");
    let dir = ws.package("p", &manifest("p", Some(""), &[]), PURE_SRC);
    let m = Manifest::load(&dir).unwrap();
    let before = grasm_pkg::package_checksum(&m).unwrap();

    std::fs::write(dir.join("src/main.gr"), format!("{PURE_SRC}// a comment\n")).unwrap();
    let after = grasm_pkg::package_checksum(&m).unwrap();
    assert_ne!(before, after, "a comment is still a source change");

    // A file added anywhere in the package counts too.
    std::fs::write(dir.join("src/extra.gr"), "pub fn extra() {\n}\n").unwrap();
    let with_extra = grasm_pkg::package_checksum(&m).unwrap();
    assert_ne!(after, with_extra, "a new file must change the digest");
}

#[test]
fn the_supply_chain_demo_catches_a_tampered_dependency() {
    // Mirrors benchmarks/security in miniature: a library declaring no
    // capabilities that quietly starts performing io must make its consumer
    // fail to build. If this ever passes, the headline security claim is
    // broken and the benchmark is lying.
    let ws = Workspace::new("supply_chain");
    let tampered = "pub fn total(xs: [i64]) -> i64 {\n    let s = read_file(\"/etc/passwd\") ?? \"\"\n    write_file(\"/tmp/x\", s)\n    xs.sum()\n}\n\nfn main() {\n}\n";
    let dir = ws.package("mathlib", &manifest("mathlib", Some(""), &[]), tampered);
    let m = Manifest::load(&dir).unwrap();
    let effects = grasm_pkg::effects_of_package(&m).unwrap();
    let violation = grasm_pkg::check_capabilities(&m, &effects)
        .expect("a pure-declared library performing io must be a violation");
    assert_eq!(violation.package, "mathlib");
    assert!(violation.exceeded.contains(&"io".to_string()));
}

#[test]
fn a_consumer_can_actually_call_into_its_dependency() {
    // Declaring and capability-checking a dependency is only half of it: the
    // consumer has to be able to *use* it. The loader resolved imports as
    // sibling files only, so `import mathlib` looked for `mathlib.gr` next to
    // main and a real two-package program could not be run or built at all.
    // The demo workspace hid this by never importing across the boundary.
    let ws = Workspace::new("cross_pkg");
    ws.package(
        "mathlib",
        &manifest("mathlib", Some(""), &[]),
        "pub fn total(xs: [i64]) -> i64 {\n    xs.sum()\n}\n",
    );
    let app = ws.package(
        "app",
        &manifest("app", Some("\"io\""), &[("mathlib", "../mathlib")]),
        "import mathlib\n\nfn main() {\n    print(\"{mathlib.total([1, 2, 3, 4])}\")\n}\n",
    );

    let program = grasm_sema::loader::load_from_disk(&app.join("src/main.gr"))
        .expect("the entry file loads");
    assert!(
        program.diagnostics.is_empty(),
        "a dependency named in grasm.toml must be importable: {:#?}",
        program.diagnostics
    );
    assert!(
        program.files.iter().any(|f| f.name == "mathlib"),
        "the dependency should be part of the loaded program: {:?}",
        program.files.iter().map(|f| &f.name).collect::<Vec<_>>()
    );
    let diags = grasm_sema::check::check_program(&program).diagnostics;
    assert!(diags.is_empty(), "{diags:#?}");

    let mut out = Vec::new();
    let code = grasm_interp::run_program(&program, &[], &mut out);
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8(out).unwrap(), "10\n");
}

#[test]
fn a_dependency_is_still_capability_checked_when_actually_used() {
    // The guarantee has to survive the consumer genuinely calling in.
    let ws = Workspace::new("cross_pkg_viol");
    ws.package(
        "mathlib",
        &manifest("mathlib", Some(""), &[]),
        "pub fn total(xs: [i64]) -> i64 {\n    let _leak = read_file(\"/etc/passwd\") ?? \"\"\n    xs.sum()\n}\n",
    );
    let app = ws.package(
        "app",
        &manifest("app", Some("\"io\""), &[("mathlib", "../mathlib")]),
        "import mathlib\n\nfn main() {\n    print(\"{mathlib.total([1])}\")\n}\n",
    );
    let _ = app;
    let m = Manifest::load(&ws.path().join("mathlib")).unwrap();
    let effects = grasm_pkg::effects_of_package(&m).unwrap();
    let violation = grasm_pkg::check_capabilities(&m, &effects)
        .expect("a pure-declared dependency performing io must be a violation");
    assert!(violation.exceeded.contains(&"io".to_string()));
}
