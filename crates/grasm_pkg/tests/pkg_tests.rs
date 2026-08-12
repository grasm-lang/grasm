//! Package manager tests: manifests, resolution, capability enforcement,
//! and lockfile integrity.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use grasm_pkg::{Manifest, MANIFEST_NAME};

/// A scratch workspace that cleans itself up.
struct Workspace(PathBuf);

/// Distinguishes concurrent workspaces without depending on the clock.
static WORKSPACE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Workspace {
    fn new(tag: &str) -> Workspace {
        // The name used to be process id + nanoseconds + a hand-written tag,
        // and two tests were given the same tag. Tests run concurrently, so
        // when their timestamps landed in the same bucket they shared a
        // directory — and one finishing deleted the other's files underneath
        // it. That failed roughly one run in fifty, which is the worst
        // frequency for a gate to fail at. A counter cannot collide.
        let n = WORKSPACE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "grasm_pkg_test_{}_{n}_{tag}",
            std::process::id(),
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
    assert_eq!(m.dependencies[0].path.as_deref(), Some("../utils"));
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
fn a_dependency_with_neither_a_path_nor_a_version_is_rejected() {
    // A path says where; a version says which. With neither there is nothing
    // to resolve the name to.
    let e = Manifest::parse(
        "[package]\nname = \"x\"\nversion = \"1.0.0\"\n\n[dependencies.thing]\n",
        Path::new("/tmp"),
    )
    .expect_err("should reject");
    assert!(e.contains("needs a `path` or a `version`"), "got: {e}");
}

#[test]
fn a_dependency_with_only_a_version_comes_from_the_registry() {
    let m = Manifest::parse(
        "[package]\nname = \"x\"\nversion = \"1.0.0\"\n\n[dependencies.thing]\nversion = \"^1.2.0\"\n",
        Path::new("/tmp"),
    )
    .expect("registry dependency");
    assert_eq!(m.dependencies[0].path, None);
    assert_eq!(m.dependencies[0].req, Some(VersionReq::parse("^1.2.0").unwrap()));
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
        capabilities: Some(caps.iter().map(|c| c.to_string()).collect()),
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
                if name == "alpha" && now.as_deref() == Some(["io".to_string()].as_slice())
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

// ---------------------------------------------------------------------------
// Capability names must mean something
// ---------------------------------------------------------------------------

/// The effect vocabulary a package's own sources define.
fn vocab_of(dir: &Path) -> BTreeSet<String> {
    let m = Manifest::load(dir).expect("manifest loads");
    grasm_pkg::effect_vocabulary_of_package(&m).expect("vocabulary")
}

#[test]
fn a_user_effect_is_a_capability_like_any_other() {
    // The ceiling is not limited to io/async: any declared effect counts, so a
    // library cannot silently start using `Network` either.
    let ws = Workspace::new("user_effect");
    let dir = ws.package(
        "lib",
        &manifest("lib", Some(""), &[]),
        "effect Network {\n    fn fetch(u: str) -> str\n}\n\npub fn grab(u: str) -> str / {Network} {\n    Network.fetch(u)\n}\n\nfn main() {\n}\n",
    );
    let m = Manifest::load(&dir).unwrap();
    let effects = grasm_pkg::effects_of_package(&m).unwrap();
    assert!(effects.contains("Network"), "inferred: {effects:?}");
    let v = grasm_pkg::check_capabilities(&m, &effects)
        .expect("performing Network under an empty ceiling is a violation");
    assert!(v.exceeded.contains(&"Network".to_string()));
}

#[test]
fn a_declared_capability_must_name_a_real_effect() {
    // A ceiling reads to a human as a permission. One naming nothing grants
    // nothing while looking like it grants something.
    let ws = Workspace::new("fictional_cap");
    let dir = ws.package(
        "lib",
        &manifest("lib", Some("\"Telemetry\""), &[]),
        "pub fn f() -> i64 {\n    1\n}\n\nfn main() {\n}\n",
    );
    let m = Manifest::load(&dir).unwrap();
    let bad = grasm_pkg::check_capability_names(&m, &vocab_of(&dir));
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert_eq!(bad[0].name, "Telemetry");
}

#[test]
fn a_typo_in_a_capability_gets_a_suggestion() {
    let ws = Workspace::new("typo_cap");
    let dir = ws.package(
        "lib",
        &manifest("lib", Some("\"Netwrok\""), &[]),
        "effect Network {\n    fn fetch(u: str) -> str\n}\n\npub fn f() -> i64 {\n    1\n}\n\nfn main() {\n}\n",
    );
    let m = Manifest::load(&dir).unwrap();
    let bad = grasm_pkg::check_capability_names(&m, &vocab_of(&dir));
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0].suggestion.as_deref(), Some("Network"));
    assert!(bad[0].to_string().contains("did you mean `Network`?"));
}

#[test]
fn the_builtin_and_declared_effects_are_all_accepted() {
    let ws = Workspace::new("good_caps");
    let dir = ws.package(
        "lib",
        &manifest("lib", Some("\"io\", \"async\", \"Network\""), &[]),
        "effect Network {\n    fn fetch(u: str) -> str\n}\n\npub fn f() -> i64 {\n    1\n}\n\nfn main() {\n}\n",
    );
    let m = Manifest::load(&dir).unwrap();
    assert!(grasm_pkg::check_capability_names(&m, &vocab_of(&dir)).is_empty());
}

#[test]
fn an_unrestricted_package_has_no_names_to_check() {
    let ws = Workspace::new("unrestricted");
    let dir = ws.package("lib", &manifest("lib", None, &[]), PURE_SRC);
    let m = Manifest::load(&dir).unwrap();
    assert!(grasm_pkg::check_capability_names(&m, &vocab_of(&dir)).is_empty());
}

// ---------------------------------------------------------------------------
// Version requirements
// ---------------------------------------------------------------------------

use grasm_pkg::{Version, VersionReq};

/// A manifest at an explicit version, optionally requiring one of its deps.
fn versioned(name: &str, version: &str, deps: &[(&str, &str, Option<&str>)]) -> String {
    let mut m = format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\n");
    for (dep, path, req) in deps {
        m.push_str(&format!("\n[dependencies.{dep}]\npath = \"{path}\"\n"));
        if let Some(r) = req {
            m.push_str(&format!("version = \"{r}\"\n"));
        }
    }
    m
}

fn req(s: &str) -> VersionReq {
    VersionReq::parse(s).unwrap()
}

fn ver(s: &str) -> Version {
    Version::parse(s).unwrap()
}

#[test]
fn a_bare_requirement_means_compatible_not_exact() {
    assert_eq!(req("1.2.3"), req("^1.2.3"));
    assert!(req("1.2.3").matches(&ver("1.2.3")));
    assert!(req("1.2.3").matches(&ver("1.9.0")));
    assert!(!req("1.2.3").matches(&ver("1.2.2")));
    assert!(!req("1.2.3").matches(&ver("2.0.0")));
}

#[test]
fn an_exact_requirement_admits_one_version() {
    assert!(req("=1.2.3").matches(&ver("1.2.3")));
    assert!(!req("=1.2.3").matches(&ver("1.2.4")));
    assert!(!req("=1.2.3").matches(&ver("1.3.0")));
}

#[test]
fn before_one_zero_the_minor_number_carries_the_weight_of_a_major() {
    // ^0.2.3 accepts 0.2.x but not 0.3.0 — the same rule the stability
    // policy states for the language itself.
    assert!(req("^0.2.3").matches(&ver("0.2.3")));
    assert!(req("^0.2.3").matches(&ver("0.2.9")));
    assert!(!req("^0.2.3").matches(&ver("0.3.0")));
    assert!(!req("^0.2.3").matches(&ver("0.2.2")));
    assert!(!req("^0.2.3").matches(&ver("1.0.0")));
}

#[test]
fn in_a_zero_zero_line_every_release_may_break() {
    assert!(req("^0.0.3").matches(&ver("0.0.3")));
    assert!(!req("^0.0.3").matches(&ver("0.0.4")));
    assert!(!req("^0.0.3").matches(&ver("0.1.0")));
}

#[test]
fn versions_order_by_number_not_by_text() {
    assert!(ver("0.2.0") < ver("0.10.0"));
    assert!(ver("1.0.0") > ver("0.99.99"));
}

#[test]
fn a_malformed_requirement_is_rejected_where_it_is_written() {
    let ws = Workspace::new("bad_req");
    ws.package("lib", &versioned("lib", "1.0.0", &[]), PURE_SRC);
    let app = ws.package(
        "app",
        &versioned("app", "0.1.0", &[("lib", "../lib", Some("^1.2"))]),
        PURE_SRC,
    );
    let err = Manifest::load(&app).unwrap_err();
    assert!(err.contains("dependency `lib`"), "{err}");
    assert!(err.contains("MAJOR.MINOR.PATCH"), "{err}");
}

#[test]
fn a_satisfied_requirement_resolves() {
    let ws = Workspace::new("req_ok");
    ws.package("lib", &versioned("lib", "1.4.1", &[]), PURE_SRC);
    let app = ws.package(
        "app",
        &versioned("app", "0.1.0", &[("lib", "../lib", Some("^1.2.0"))]),
        PURE_SRC,
    );
    let r = grasm_pkg::resolve(&app).unwrap();
    assert_eq!(r.packages.len(), 2);
    assert_eq!(r.root().name, "app");
}

#[test]
fn an_unsatisfied_requirement_names_the_requirer_and_both_versions() {
    let ws = Workspace::new("req_bad");
    ws.package("lib", &versioned("lib", "2.0.0", &[]), PURE_SRC);
    let app = ws.package(
        "app",
        &versioned("app", "0.1.0", &[("lib", "../lib", Some("^1.2.0"))]),
        PURE_SRC,
    );
    let err = grasm_pkg::resolve(&app).unwrap_err();
    assert!(err.contains("dependency `lib` of `app`"), "{err}");
    assert!(err.contains("requires ^1.2.0"), "{err}");
    assert!(err.contains("is 2.0.0"), "{err}");
}

#[test]
fn a_dependency_with_no_requirement_accepts_whatever_is_there() {
    let ws = Workspace::new("req_none");
    ws.package("lib", &versioned("lib", "9.9.9", &[]), PURE_SRC);
    let app = ws.package(
        "app",
        &versioned("app", "0.1.0", &[("lib", "../lib", None)]),
        PURE_SRC,
    );
    assert!(grasm_pkg::resolve(&app).is_ok());
}

#[test]
fn one_package_reached_two_ways_resolves_once() {
    // The diamond: app -> {left, right} -> lib. One lib, not two.
    let ws = Workspace::new("diamond");
    ws.package("lib", &versioned("lib", "1.0.0", &[]), PURE_SRC);
    ws.package(
        "left",
        &versioned("left", "1.0.0", &[("lib", "../lib", Some("^1.0.0"))]),
        PURE_SRC,
    );
    ws.package(
        "right",
        &versioned("right", "1.0.0", &[("lib", "../lib", Some("^1.0.0"))]),
        PURE_SRC,
    );
    let app = ws.package(
        "app",
        &versioned(
            "app",
            "0.1.0",
            &[("left", "../left", None), ("right", "../right", None)],
        ),
        PURE_SRC,
    );
    let r = grasm_pkg::resolve(&app).unwrap();
    let libs = r.packages.iter().filter(|p| p.name == "lib").count();
    assert_eq!(libs, 1, "lib should appear once, not once per dependent");
    assert_eq!(r.packages.len(), 4);
}

#[test]
fn two_directories_claiming_one_name_is_an_error_not_a_coin_flip() {
    // Before versions were checked, whichever copy the walk reached first
    // silently won and the other dependent got a package it never asked for.
    let ws = Workspace::new("two_libs");
    std::fs::create_dir_all(ws.path().join("vendor")).unwrap();
    ws.package("lib", &versioned("lib", "1.0.0", &[]), PURE_SRC);
    // A second `lib`, at a different path, at a different version.
    let other = ws.path().join("vendor/lib");
    std::fs::create_dir_all(other.join("src")).unwrap();
    std::fs::write(other.join(MANIFEST_NAME), versioned("lib", "2.0.0", &[])).unwrap();
    std::fs::write(other.join("src/main.gr"), PURE_SRC).unwrap();

    ws.package(
        "left",
        &versioned("left", "1.0.0", &[("lib", "../lib", None)]),
        PURE_SRC,
    );
    ws.package(
        "right",
        &versioned("right", "1.0.0", &[("lib", "../vendor/lib", None)]),
        PURE_SRC,
    );
    let app = ws.package(
        "app",
        &versioned(
            "app",
            "0.1.0",
            &[("left", "../left", None), ("right", "../right", None)],
        ),
        PURE_SRC,
    );
    let err = grasm_pkg::resolve(&app).unwrap_err();
    assert!(err.contains("two different directories"), "{err}");
    assert!(err.contains("is 1.0.0"), "{err}");
    assert!(err.contains("is 2.0.0"), "{err}");
    assert!(err.contains("`left`"), "{err}");
    assert!(err.contains("`right`"), "{err}");
}

// ---------------------------------------------------------------------------
// Registry and minimal version selection
// ---------------------------------------------------------------------------

use grasm_pkg::Registry;

impl Workspace {
    /// Publishes `name` at `version` into the workspace's registry, with the
    /// dependencies it in turn requires.
    fn publish(&self, name: &str, version: &str, deps: &[(&str, &str)]) {
        let dir = self.0.join("registry").join(name).join(version);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let mut m = format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\ncapabilities = []\n");
        for (dep, req) in deps {
            m.push_str(&format!("\n[dependencies.{dep}]\nversion = \"{req}\"\n"));
        }
        std::fs::write(dir.join(MANIFEST_NAME), m).unwrap();
        std::fs::write(dir.join("src/main.gr"), PURE_SRC).unwrap();
    }

    fn registry(&self) -> Registry {
        Registry::new(self.0.join("registry"))
    }
}

/// A root package requiring registry dependencies.
fn app_requiring(deps: &[(&str, &str)]) -> String {
    let mut m = String::from("[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = []\n");
    for (dep, req) in deps {
        m.push_str(&format!("\n[dependencies.{dep}]\nversion = \"{req}\"\n"));
    }
    m
}

fn versions_of(r: &grasm_pkg::Resolution) -> Vec<String> {
    let mut v: Vec<String> = r
        .packages
        .iter()
        .map(|p| format!("{} {}", p.name, p.version))
        .collect();
    v.sort();
    v
}

#[test]
fn selection_takes_the_lowest_version_that_satisfies_the_requirement() {
    // Not the newest. Asking for ^1.0.0 when 1.0.0, 1.2.0 and 1.9.0 exist
    // gets 1.0.0 — nothing moves unless a requirement says it must.
    let ws = Workspace::new("mvs_lowest");
    ws.publish("text", "1.0.0", &[]);
    ws.publish("text", "1.2.0", &[]);
    ws.publish("text", "1.9.0", &[]);
    let app = ws.package("app", &app_requiring(&[("text", "^1.0.0")]), PURE_SRC);
    let r = grasm_pkg::resolve_in(&app, Some(&ws.registry())).unwrap();
    assert_eq!(versions_of(&r), vec!["app 0.1.0", "text 1.0.0"]);
}

#[test]
fn the_highest_floor_anyone_states_is_the_one_that_wins() {
    // app wants ^1.0.0, its dependency wants ^1.2.0: everyone gets 1.2.0,
    // which is the lowest version satisfying both.
    let ws = Workspace::new("mvs_floor");
    ws.publish("text", "1.0.0", &[]);
    ws.publish("text", "1.2.0", &[]);
    ws.publish("text", "1.9.0", &[]);
    ws.publish("parser", "1.0.0", &[("text", "^1.2.0")]);
    let app = ws.package(
        "app",
        &app_requiring(&[("text", "^1.0.0"), ("parser", "^1.0.0")]),
        PURE_SRC,
    );
    let r = grasm_pkg::resolve_in(&app, Some(&ws.registry())).unwrap();
    assert_eq!(
        versions_of(&r),
        vec!["app 0.1.0", "parser 1.0.0", "text 1.2.0"]
    );
}

#[test]
fn the_result_does_not_depend_on_the_order_the_graph_is_walked() {
    // The same two requirements, declared in the opposite order, select the
    // same versions. This is the property that makes a lockfile an integrity
    // check rather than the thing holding the build still.
    let ws = Workspace::new("mvs_order");
    ws.publish("text", "1.0.0", &[]);
    ws.publish("text", "1.4.0", &[]);
    ws.publish("a", "1.0.0", &[("text", "^1.4.0")]);
    ws.publish("b", "1.0.0", &[("text", "^1.0.0")]);

    let one = ws.package("one", &{
        let mut m = app_requiring(&[("a", "^1.0.0"), ("b", "^1.0.0")]);
        m = m.replace("name = \"app\"", "name = \"one\"");
        m
    }, PURE_SRC);
    let two = ws.package("two", &{
        let mut m = app_requiring(&[("b", "^1.0.0"), ("a", "^1.0.0")]);
        m = m.replace("name = \"app\"", "name = \"two\"");
        m
    }, PURE_SRC);

    let r1 = grasm_pkg::resolve_in(&one, Some(&ws.registry())).unwrap();
    let r2 = grasm_pkg::resolve_in(&two, Some(&ws.registry())).unwrap();
    let pick = |r: &grasm_pkg::Resolution| {
        r.packages
            .iter()
            .find(|p| p.name == "text")
            .unwrap()
            .version
            .clone()
    };
    assert_eq!(pick(&r1), "1.4.0");
    assert_eq!(pick(&r1), pick(&r2));
}

#[test]
fn requirements_that_cannot_both_hold_name_both_sides() {
    let ws = Workspace::new("mvs_conflict");
    ws.publish("text", "1.0.0", &[]);
    ws.publish("text", "2.0.0", &[]);
    ws.publish("old", "1.0.0", &[("text", "^1.0.0")]);
    let app = ws.package(
        "app",
        &app_requiring(&[("text", "^2.0.0"), ("old", "^1.0.0")]),
        PURE_SRC,
    );
    let err = grasm_pkg::resolve_in(&app, Some(&ws.registry())).unwrap_err();
    assert!(err.contains("no version of `text`"), "{err}");
    assert!(err.contains("`app` requires ^2.0.0"), "{err}");
    assert!(err.contains("`old` requires ^1.0.0"), "{err}");
}

#[test]
fn a_transitive_requirement_pulls_its_own_dependencies_in() {
    let ws = Workspace::new("mvs_transitive");
    ws.publish("core", "1.0.0", &[]);
    ws.publish("mid", "1.0.0", &[("core", "^1.0.0")]);
    ws.publish("top", "1.0.0", &[("mid", "^1.0.0")]);
    let app = ws.package("app", &app_requiring(&[("top", "^1.0.0")]), PURE_SRC);
    let r = grasm_pkg::resolve_in(&app, Some(&ws.registry())).unwrap();
    assert_eq!(
        versions_of(&r),
        vec!["app 0.1.0", "core 1.0.0", "mid 1.0.0", "top 1.0.0"]
    );
    // Dependencies still come before dependents.
    let order: Vec<&str> = r.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(order, vec!["core", "mid", "top", "app"]);
}

#[test]
fn a_missing_package_says_where_it_looked() {
    let ws = Workspace::new("mvs_missing");
    let app = ws.package("app", &app_requiring(&[("ghost", "^1.0.0")]), PURE_SRC);
    let err = grasm_pkg::resolve_in(&app, Some(&ws.registry())).unwrap_err();
    assert!(err.contains("no package named `ghost`"), "{err}");
}

#[test]
fn a_version_below_every_available_one_lists_what_there_is() {
    let ws = Workspace::new("mvs_toolow");
    ws.publish("text", "1.0.0", &[]);
    ws.publish("text", "1.2.0", &[]);
    let app = ws.package("app", &app_requiring(&[("text", "^2.0.0")]), PURE_SRC);
    let err = grasm_pkg::resolve_in(&app, Some(&ws.registry())).unwrap_err();
    assert!(err.contains("no `text` at or above 2.0.0"), "{err}");
    assert!(err.contains("1.0.0, 1.2.0"), "{err}");
}

#[test]
fn a_registry_dependency_with_no_registry_says_how_to_set_one() {
    let ws = Workspace::new("mvs_noreg");
    let app = ws.package("app", &app_requiring(&[("text", "^1.0.0")]), PURE_SRC);
    let err = grasm_pkg::resolve_in(&app, None).unwrap_err();
    assert!(err.contains("no registry was found"), "{err}");
    assert!(err.contains("GRASM_REGISTRY"), "{err}");
}

#[test]
fn path_and_registry_dependencies_resolve_in_one_graph() {
    let ws = Workspace::new("mvs_mixed");
    ws.publish("text", "1.0.0", &[]);
    ws.package("local", &versioned("local", "0.5.0", &[]), PURE_SRC);
    let mut m = app_requiring(&[("text", "^1.0.0")]);
    m.push_str("\n[dependencies.local]\npath = \"../local\"\n");
    let app = ws.package("app", &m, PURE_SRC);
    let r = grasm_pkg::resolve_in(&app, Some(&ws.registry())).unwrap();
    assert_eq!(
        versions_of(&r),
        vec!["app 0.1.0", "local 0.5.0", "text 1.0.0"]
    );
}

#[test]
fn an_exact_requirement_beats_a_later_compatible_one_only_by_disagreeing() {
    // `=1.0.0` and `^1.2.0` cannot both hold; saying so beats picking one.
    let ws = Workspace::new("mvs_exact");
    ws.publish("text", "1.0.0", &[]);
    ws.publish("text", "1.2.0", &[]);
    ws.publish("pinned", "1.0.0", &[("text", "=1.0.0")]);
    let app = ws.package(
        "app",
        &app_requiring(&[("text", "^1.2.0"), ("pinned", "^1.0.0")]),
        PURE_SRC,
    );
    let err = grasm_pkg::resolve_in(&app, Some(&ws.registry())).unwrap_err();
    assert!(err.contains("no version of `text`"), "{err}");
    assert!(err.contains("=1.0.0"), "{err}");
}

#[test]
fn a_registry_package_is_held_to_its_ceiling_like_any_other() {
    // The supply-chain guarantee only matters for code you did not write.
    // A registry package that declares no capabilities and then reads a file
    // is caught by the same inference that catches a path dependency.
    let ws = Workspace::new("reg_caps");
    let dir = ws.0.join("registry").join("sneaky").join("1.0.0");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join(MANIFEST_NAME),
        "[package]\nname = \"sneaky\"\nversion = \"1.0.0\"\ncapabilities = []\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/main.gr"), IO_SRC).unwrap();

    let app = ws.package("app", &app_requiring(&[("sneaky", "^1.0.0")]), PURE_SRC);
    let r = grasm_pkg::resolve_in(&app, Some(&ws.registry())).unwrap();

    let sneaky = r.packages.iter().find(|p| p.name == "sneaky").unwrap();
    let performed = grasm_pkg::effects_of_package(sneaky).unwrap();
    let violation = grasm_pkg::check_capabilities(sneaky, &performed)
        .expect("a package that prints while permitting nothing must be caught");
    assert_eq!(violation.package, "sneaky");
    assert!(violation.to_string().contains("io"), "{violation}");
}

#[test]
fn the_lockfile_records_the_versions_selection_chose() {
    let ws = Workspace::new("reg_lock");
    ws.publish("text", "1.0.0", &[]);
    ws.publish("text", "1.4.0", &[]);
    ws.publish("dep", "1.0.0", &[("text", "^1.4.0")]);
    let app = ws.package(
        "app",
        &app_requiring(&[("text", "^1.0.0"), ("dep", "^1.0.0")]),
        PURE_SRC,
    );
    let r = grasm_pkg::resolve_in(&app, Some(&ws.registry())).unwrap();
    let lock = grasm_pkg::build_lock(&r).unwrap();
    let text = lock.iter().find(|e| e.name == "text").unwrap();
    // 1.4.0, not the 1.0.0 the root asked for and not a newest-wins pick.
    assert_eq!(text.version, "1.4.0");
    assert!(!text.checksum.is_empty());
}


/// An effect handled inside the package is not a capability.
///
/// A capability is what a package needs from *outside* itself. `Cfg` is
/// declared here, performed by a private helper, and handled three lines later
/// in `main` — it never leaves this source, so nothing outside has to provide
/// it. Counting it anyway made this package fail to build against
/// `capabilities = ["io"]` demanding a capability named `Cfg`, and the only
/// way through was to publish an internal name in the manifest.
///
/// The opposite case still counts: see
/// `a_user_effect_is_a_capability_like_any_other`, where `Network` is in a
/// `pub fn`'s row with no handler, so a consumer really does have to supply
/// one.
#[test]
fn an_effect_handled_inside_the_package_is_not_a_capability() {
    let ws = Workspace::new("usereffect");
    let src = "effect Cfg {\n    fn value() -> i64\n}\n\n\
               fn read() -> i64 / {Cfg} {\n    Cfg.value()\n}\n\n\
               fn main() {\n    let h = CfgHandler{ value: || 7 }\n    \
               with Cfg handled_by h {\n        print(\"{read()}\")\n    }\n}\n";
    let dir = ws.package("cfgpkg", &manifest("cfgpkg", Some("\"io\""), &[]), src);
    let m = Manifest::load(&dir).unwrap();
    let performed = grasm_pkg::effects_of_package(&m).unwrap();
    assert!(!performed.contains("Cfg"), "got: {performed:?}");
    assert!(performed.contains("io"), "the print still counts: {performed:?}");
    assert!(grasm_pkg::check_capabilities(&m, &performed).is_none());
}

/// The other half: the handler's *real* effect is still counted.
///
/// This is what keeps the rule above from being a hole. `Log` is abstract, but
/// the handler prints, and printing is `io` whoever asked for it.
#[test]
fn an_effect_handler_still_answers_for_what_it_does() {
    let ws = Workspace::new("handlereffect");
    let src = "effect Log {\n    fn note(m: str)\n}\n\n\
               fn work() / {Log} {\n    Log.note(\"hi\")\n}\n\n\
               fn main() {\n    let h = LogHandler{ note: |m| print(\"{m}\") }\n    \
               with Log handled_by h {\n        work()\n    }\n}\n";
    let dir = ws.package("logpkg", &manifest("logpkg", Some(""), &[]), src);
    let m = Manifest::load(&dir).unwrap();
    let performed = grasm_pkg::effects_of_package(&m).unwrap();
    let v = grasm_pkg::check_capabilities(&m, &performed)
        .expect("the handler's io must still be caught");
    assert_eq!(v.exceeded, vec!["io"]);
}

/// A pure package can do trigonometry.
///
/// This is the reason the math methods exist. Reaching libm through
/// `extern fn cos` works and costs a capability — a declared C function
/// performs `io` — so a package declaring `capabilities = []` could not
/// compute a cosine, and a geometry library had to claim a permission it had
/// no use for.
#[test]
fn a_pure_package_can_do_trigonometry() {
    let ws = Workspace::new("puremath");
    let src = "pub fn deg(r: f64) -> f64 {\n    r.to_degrees()\n}\n\n\
               pub fn dist(x: f64, y: f64) -> f64 {\n    x.hypot(y)\n}\n\n\
               pub fn angle(x: f64) -> f64 {\n    (x / pi()).atan()\n}\n\n\
               fn main() {\n}\n";
    let dir = ws.package("geom", &manifest("geom", Some(""), &[]), src);
    let m = Manifest::load(&dir).unwrap();
    let performed = grasm_pkg::effects_of_package(&m).unwrap();
    assert!(performed.is_empty(), "math is pure: {performed:?}");
    assert!(grasm_pkg::check_capabilities(&m, &performed).is_none());
}

/// `--locked` catches a version change with unchanged sources.
///
/// The diff compared checksums and capabilities, and a metadata-only release
/// changes neither: same sources, same ceiling, different version. So the
/// change passed CI and the lockfile went on recording a version that was no
/// longer in the build — a record that is not merely stale but wrong.
#[test]
fn a_version_change_alone_is_a_lockfile_change() {
    let before = grasm_pkg::LockEntry {
        name: "lib".into(),
        version: "0.1.0".into(),
        checksum: "abc".into(),
        capabilities: Some(vec![]),
    };
    let after = grasm_pkg::LockEntry {
        version: "0.1.5".into(),
        ..before.clone()
    };
    let text = grasm_pkg::render_lock(&[before]);
    let changes = grasm_pkg::diff_lock(&text, &[after]);
    assert_eq!(changes.len(), 1, "{changes:?}");
    let msg = changes[0].to_string();
    assert!(msg.contains("0.1.5"), "{msg}");
    assert!(msg.contains("0.1.0"), "{msg}");
}

/// Tier 3 is visible in the manifest and refusable by a consumer.
///
/// An `unsafe` block puts `unsafe` in the effect row, so it reaches the
/// capability ceiling by the same path every other effect does. Without that
/// the block would be a comment: a library could contain one and nothing
/// downstream would know.
#[test]
fn an_unsafe_block_needs_the_unsafe_capability() {
    let ws = Workspace::new("tier3");
    let src = "extern fn sqrt(x: f64) -> f64\n\n\
               fn main() {\n    let r = unsafe {\n        sqrt(9.0)\n    }\n    \
               print(\"{r}\")\n}\n";
    let dir = ws.package("ffipkg", &manifest("ffipkg", Some("\"io\""), &[]), src);
    let m = Manifest::load(&dir).unwrap();
    let performed = grasm_pkg::effects_of_package(&m).unwrap();
    assert!(performed.contains("unsafe"), "{performed:?}");
    let v = grasm_pkg::check_capabilities(&m, &performed)
        .expect("unsafe beyond an io-only ceiling must be caught");
    assert_eq!(v.exceeded, vec!["unsafe"]);

    // And it is declarable: `unsafe` is a real capability name, not a typo.
    let ok_dir = ws.package(
        "ffiok",
        &manifest("ffiok", Some("\"io\", \"unsafe\""), &[]),
        src,
    );
    let ok = Manifest::load(&ok_dir).unwrap();
    let performed = grasm_pkg::effects_of_package(&ok).unwrap();
    assert!(grasm_pkg::check_capabilities(&ok, &performed).is_none());
    let vocab = grasm_pkg::effect_vocabulary_of_package(&ok).unwrap();
    assert!(grasm_pkg::check_capability_names(&ok, &vocab).is_empty());
}

/// A package's sources are the files under its own directory. An `entry`
/// outside it left the lockfile checksumming a tree the program did not use
/// and the capability ceiling scanning one the entry had left.
#[test]
fn an_entry_outside_the_package_is_refused() {
    let text = "[package]\nname = \"a\"\nversion = \"0.1.0\"\nentry = \"../shared/main.gr\"\n";
    let err = Manifest::parse(text, Path::new("/tmp")).unwrap_err();
    assert!(err.contains("inside the package"), "{err}");
    assert!(err.contains("../shared/main.gr"), "{err}");
}

#[test]
fn an_absolute_entry_is_refused() {
    let text = "[package]\nname = \"a\"\nversion = \"0.1.0\"\nentry = \"/etc/main.gr\"\n";
    let err = Manifest::parse(text, Path::new("/tmp")).unwrap_err();
    assert!(err.contains("is absolute"), "{err}");
}

/// The checksum covers the manifest, not only the sources.
///
/// Two releases with identical sources and different `capabilities` lines
/// hashed the same, so the lockfile could not tell them apart — a hole in the
/// record at exactly the security-relevant field.
#[test]
fn the_checksum_covers_the_manifest() {
    let w = Workspace::new("cksum");
    let dir = w.0.join("lib");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.gr"), "pub fn f() -> i64 {\n    1\n}\n").unwrap();

    std::fs::write(
        dir.join(MANIFEST_NAME),
        "[package]\nname = \"lib\"\nversion = \"0.1.0\"\ncapabilities = []\n",
    )
    .unwrap();
    let closed = grasm_pkg::package_checksum(&Manifest::load(&dir).unwrap()).unwrap();

    std::fs::write(
        dir.join(MANIFEST_NAME),
        "[package]\nname = \"lib\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n",
    )
    .unwrap();
    let open = grasm_pkg::package_checksum(&Manifest::load(&dir).unwrap()).unwrap();

    assert_ne!(
        closed, open,
        "a changed capability ceiling must change the checksum"
    );
}
