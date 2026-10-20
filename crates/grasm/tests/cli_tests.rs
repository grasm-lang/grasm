//! End-to-end tests of the `grasm` binary itself.
//!
//! Some guarantees are about what the *command* does, and can only be tested
//! by running it. A capability ceiling is one: the library check was correct
//! the whole time and nothing called it on the build path.

use std::path::{Path, PathBuf};
use std::process::Command;

const GRASM: &str = env!("CARGO_BIN_EXE_grasm");

/// A throwaway two-package workspace, named by a counter so concurrent tests
/// cannot share a directory.
static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct Work(PathBuf);

impl Work {
    fn new(tag: &str) -> Work {
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("grasm_cli_test_{}_{n}_{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Work(dir)
    }

    fn package(&self, name: &str, manifest: &str, source: &str) -> PathBuf {
        let dir = self.0.join(name);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("grasm.toml"), manifest).unwrap();
        std::fs::write(dir.join("src/main.gr"), source).unwrap();
        dir
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(GRASM).args(args).output().expect("runs");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

fn app_entry(app: &Path) -> String {
    app.join("src/main.gr").to_string_lossy().into_owned()
}

const OVER_CEILING: &str =
    "[package]\nname = \"quietlib\"\nversion = \"0.1.0\"\ncapabilities = []\n";
const AT_CEILING: &str =
    "[package]\nname = \"quietlib\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n";
const LIB_SRC: &str = "pub fn greet() -> i64 {\n    print(\"side effect\")\n    5\n}\n";
const APP_MANIFEST: &str = "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.quietlib]\npath = \"../lib\"\n";
const APP_SRC: &str = "import quietlib\n\nfn main() {\n    print(\"{quietlib.greet()}\")\n}\n";

/// A dependency past its declared ceiling fails `check`, `run` and `build` —
/// not just `deps`.
///
/// Two independent evaluations found this: `grasm deps` reported the
/// violation and every other command succeeded, so a dependency that quietly
/// started reading the filesystem shipped unless a reviewer happened to run a
/// separate command. The documentation promised "a compile error, not a
/// runtime surprise, and not something a reviewer has to happen to notice".
#[test]
fn a_dependency_over_its_ceiling_fails_every_build_command() {
    let w = Work::new("ceiling");
    w.package("lib", OVER_CEILING, LIB_SRC);
    let app = w.package("app", APP_MANIFEST, APP_SRC);
    let entry = app_entry(&app);
    let bin = w.0.join("out");

    for args in [
        vec!["check", entry.as_str()],
        vec!["run", entry.as_str()],
        vec![
            "build",
            entry.as_str(),
            "--native",
            "-o",
            bin.to_str().unwrap(),
        ],
    ] {
        let (code, text) = run(&args);
        assert_eq!(code, 1, "`grasm {}` should fail:\n{text}", args[0]);
        assert!(
            text.contains("performs `io` but its manifest only permits nothing"),
            "`grasm {}` should name the violation:\n{text}",
            args[0]
        );
    }
    assert!(!bin.exists(), "a failed build must not leave a binary");
}

/// And a ceiling that covers what the package does gets out of the way.
#[test]
fn a_dependency_within_its_ceiling_builds_and_runs() {
    let w = Work::new("ceiling_ok");
    w.package("lib", AT_CEILING, LIB_SRC);
    let app = w.package("app", APP_MANIFEST, APP_SRC);
    let (code, text) = run(&["run", &app_entry(&app)]);
    assert_eq!(code, 0, "should run:\n{text}");
    assert!(text.contains("side effect"), "should run the body:\n{text}");
    assert!(text.contains('5'), "should print the result:\n{text}");
}

/// `grasm test` runs the tests of every module in the program, not just the
/// entry file's.
///
/// A library's tests used to be skipped in silence: importing a module whose
/// tests were failing reported "1 passed, 0 failed" and exit 0. Doc examples
/// from that same module *did* run, so one kind of test crossed the module
/// boundary and the other did not.
#[test]
fn tests_in_imported_modules_run() {
    let w = Work::new("modtests");
    std::fs::create_dir_all(&w.0).unwrap();
    std::fs::write(
        w.0.join("lib.gr"),
        "pub fn double(n: i64) -> i64 {\n    n * 2\n}\n\n\
         test \"double works\" {\n    assert_eq(double(2), 4)\n}\n\n\
         test \"double is broken\" {\n    assert_eq(double(3), 999)\n}\n",
    )
    .unwrap();
    std::fs::write(
        w.0.join("main.gr"),
        "import lib\n\ntest \"entry test\" {\n    assert_eq(lib.double(1), 2)\n}\n\n\
         fn main() {\n    print(\"{lib.double(21)}\")\n}\n",
    )
    .unwrap();

    let entry = w.0.join("main.gr");
    let (code, text) = run(&["test", entry.to_str().unwrap()]);
    assert_eq!(code, 1, "a failing library test must fail the run:\n{text}");
    assert!(
        text.contains("lib::double works") && text.contains("lib::double is broken"),
        "the library's tests must run and be named by module:\n{text}"
    );
    assert!(
        text.contains("main::entry test"),
        "the entry module's test must still run:\n{text}"
    );
    assert!(
        text.contains("2 passed, 1 failed"),
        "the summary must count every module's tests:\n{text}"
    );
}

/// A dependency's own `import` resolves inside that dependency.
///
/// It used to resolve against the *consumer's* source directory, so a
/// same-named file there silently replaced the library's private module. The
/// program ran, printed the wrong answer, and exited 0.
#[test]
fn a_dependencys_private_module_is_not_replaceable_by_the_consumer() {
    let w = Work::new("modres");
    let lib = w.0.join("liba");
    let app = w.0.join("app");
    std::fs::create_dir_all(lib.join("src/sub")).unwrap();
    std::fs::create_dir_all(app.join("src/sub")).unwrap();
    std::fs::write(
        lib.join("grasm.toml"),
        "[package]\nname = \"liba\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        lib.join("src/main.gr"),
        "import sub.helper\n\npub fn f() -> i64 {\n    helper.g()\n}\n",
    )
    .unwrap();
    std::fs::write(
        lib.join("src/sub/helper.gr"),
        "pub fn g() -> i64 {\n    7\n}\n",
    )
    .unwrap();
    std::fs::write(
        app.join("grasm.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies.liba]\npath = \"../liba\"\n",
    )
    .unwrap();
    std::fs::write(
        app.join("src/main.gr"),
        "import liba\n\nfn main() {\n    print(\"{liba.f()}\")\n}\n",
    )
    .unwrap();
    // The consumer has a file at the same relative path. The library's own
    // must win: the library says what its imports mean.
    std::fs::write(
        app.join("src/sub/helper.gr"),
        "pub fn g() -> i64 {\n    999\n}\n",
    )
    .unwrap();

    let (code, text) = run(&["run", app.join("src/main.gr").to_str().unwrap()]);
    assert_eq!(code, 0, "should run:\n{text}");
    assert!(
        text.contains('7') && !text.contains("999"),
        "the dependency's own module must be the one loaded:\n{text}"
    );
}

/// A doc example runs even when the entry module declares tests of its own.
///
/// The runner picked the *first* test in the module, and the example is
/// appended after the module's own declarations — so any project with a test
/// in its entry module had every doc example reported `ok` without being
/// executed. One evaluation had 13 falsified examples pass at once.
#[test]
fn doc_examples_run_even_when_the_entry_module_has_tests() {
    let w = Work::new("docex");
    std::fs::create_dir_all(&w.0).unwrap();
    std::fs::write(
        w.0.join("lib.gr"),
        "/// ```\n/// assert(1 == 2)\n/// ```\npub fn f() -> i64 {\n    1\n}\n",
    )
    .unwrap();
    std::fs::write(
        w.0.join("main.gr"),
        "import lib\n\nfn main() {\n    print(\"{lib.f()}\")\n}\n\n\
         test \"a test of its own\" {\n    assert(true)\n}\n",
    )
    .unwrap();

    let (code, text) = run(&["test", w.0.join("main.gr").to_str().unwrap()]);
    assert_eq!(code, 1, "a false doc example must fail the suite:\n{text}");
    assert!(
        text.contains("doc example lib::f ... FAILED"),
        "the example must actually run:\n{text}"
    );
}

/// Deep recursion in a *compiled* program panics rather than crashing.
///
/// Cranelift emits no stack check, so past a few hundred thousand frames the
/// binary took SIGSEGV — exit 139, nothing on stderr — while
/// [E-RECURSION-DEPTH] requires a panic and exit 101 and says outright that it
/// "does not abort, crash, or corrupt the process".
#[test]
fn deep_recursion_in_a_native_binary_panics_rather_than_crashing() {
    let w = Work::new("deeprec");
    std::fs::create_dir_all(&w.0).unwrap();
    let src = w.0.join("deep.gr");
    std::fs::write(
        &src,
        "fn down(n: i64) -> i64 {\n    if n <= 0 { 0 } else { down(n - 1) + 1 }\n}\n\
         fn main() {\n    print(\"{down(2000000)}\")\n}\n",
    )
    .unwrap();
    let bin = w.0.join("deep");
    let (code, _) = run(&[
        "build",
        src.to_str().unwrap(),
        "--native",
        "-o",
        bin.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "should build");

    let out = Command::new(&bin).output().expect("runs");
    assert_eq!(
        out.status.code(),
        Some(101),
        "expected a panic and exit 101, got {:?}",
        out.status
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("stack overflow"),
        "the panic must say what happened: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// And the guaranteed depth still works, guard and all.
#[test]
fn ten_thousand_frames_work_in_a_native_binary() {
    let w = Work::new("depth10k");
    std::fs::create_dir_all(&w.0).unwrap();
    let src = w.0.join("ok.gr");
    std::fs::write(
        &src,
        "fn down(n: i64) -> i64 {\n    if n <= 0 { 0 } else { down(n - 1) + 1 }\n}\n\
         fn main() {\n    print(\"{down(10000)}\")\n}\n",
    )
    .unwrap();
    let bin = w.0.join("ok");
    run(&[
        "build",
        src.to_str().unwrap(),
        "--native",
        "-o",
        bin.to_str().unwrap(),
    ]);
    let out = Command::new(&bin).output().expect("runs");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "10000");
}

/// `grasm bench` covers every module, the way `grasm test` does.
///
/// Scanning only the entry file reported "no benchmarks found" for a library
/// whose submodules each declared one.
#[test]
fn benchmarks_in_imported_modules_run() {
    let w = Work::new("modbench");
    std::fs::create_dir_all(&w.0).unwrap();
    std::fs::write(
        w.0.join("lib.gr"),
        "pub fn work(n: i64) -> i64 {\n    n * 2\n}\n\nbench \"lib work\" {\n    work(21)\n}\n",
    )
    .unwrap();
    std::fs::write(
        w.0.join("main.gr"),
        "import lib\n\nfn main() {\n    print(\"{lib.work(1)}\")\n}\n",
    )
    .unwrap();
    let (code, text) = run(&["bench", w.0.join("main.gr").to_str().unwrap()]);
    assert_eq!(code, 0, "should run:\n{text}");
    assert!(
        text.contains("lib::lib work"),
        "the library's benchmark must run:\n{text}"
    );
}

/// `grasm --help` is a request, not an error.
#[test]
fn help_succeeds_and_an_unknown_command_does_not() {
    for arg in ["--help", "help", "-h"] {
        let (code, text) = run(&[arg]);
        assert_eq!(code, 0, "`grasm {arg}` should succeed:\n{text}");
        assert!(text.contains("usage: grasm"), "should print usage:\n{text}");
    }
    let (code, text) = run(&["definitely-not-a-command"]);
    assert_eq!(code, 1, "an unknown command should fail:\n{text}");
    assert!(text.contains("unknown command"), "should say so:\n{text}");
}

/// `grasm fmt` refuses source that does not parse.
///
/// The formatter works from the token stream, so it rewrote what it could and
/// reported success on source the compiler rejects — and `--check` called it
/// canonically formatted.
#[test]
fn fmt_refuses_source_that_does_not_parse() {
    let w = Work::new("fmtbad");
    std::fs::create_dir_all(&w.0).unwrap();
    let bad = w.0.join("bad.gr");
    std::fs::write(&bad, "fn main( {\n  broken\n").unwrap();
    for args in [
        vec!["fmt", bad.to_str().unwrap()],
        vec!["fmt", bad.to_str().unwrap(), "--check"],
    ] {
        let (code, text) = run(&args);
        assert_eq!(code, 1, "should fail:\n{text}");
        assert!(text.contains("does not parse"), "should say why:\n{text}");
    }
}

/// A doc example passes in both positions, or fails in both.
///
/// It used to compile in the *program's entry* module, so a library's own
/// suite wanted `describe(4)` and a consumer's wanted `lib.describe(4)`, and
/// no spelling passed in both. A library's suite was green standalone and red
/// from its consumer, permanently.
#[test]
fn a_doc_example_behaves_the_same_from_the_library_and_from_a_consumer() {
    for (body, expected_code) in [("8", 0), ("999", 1)] {
        let w = Work::new("docpos");
        std::fs::create_dir_all(&w.0).unwrap();
        std::fs::write(
            w.0.join("lib.gr"),
            format!(
                "/// ```\n/// assert_eq(describe(4), {body})\n/// ```\n\
                 pub fn describe(n: i64) -> i64 {{\n    n * 2\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            w.0.join("main.gr"),
            "import lib\n\nfn main() {\n    print(\"{lib.describe(2)}\")\n}\n\n\
             test \"own\" {\n    assert(true)\n}\n",
        )
        .unwrap();
        for entry in ["lib.gr", "main.gr"] {
            let (code, text) = run(&["test", w.0.join(entry).to_str().unwrap()]);
            assert_eq!(
                code, expected_code,
                "`{entry}` with example `{body}` should exit {expected_code}:\n{text}"
            );
        }
    }
}

/// The same, end to end: the two engines must agree on what it prints.
#[test]
fn a_closure_crossing_a_module_boundary_agrees_between_engines() {
    let w = Work::new("closmod");
    std::fs::create_dir_all(&w.0).unwrap();
    std::fs::write(
        w.0.join("helper.gr"),
        "pub fn value() -> str {\n    \"from helper\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        w.0.join("engine.gr"),
        "pub fn apply(f: fn() -> str) -> str {\n    f()\n}\n",
    )
    .unwrap();
    std::fs::write(
        w.0.join("main.gr"),
        "import engine\nimport helper\n\nfn main() {\n\
         \x20   print(\"{engine.apply(|| helper.value())}\")\n}\n",
    )
    .unwrap();
    let entry = w.0.join("main.gr");
    let (code, text) = run(&["run", entry.to_str().unwrap()]);
    assert_eq!(code, 0, "the interpreter should run it:\n{text}");
    assert!(text.contains("from helper"), "wrong output:\n{text}");

    let bin = w.0.join("out");
    run(&[
        "build",
        entry.to_str().unwrap(),
        "--native",
        "-o",
        bin.to_str().unwrap(),
    ]);
    let out = Command::new(&bin).output().expect("runs");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        text.trim(),
        "the two engines must agree"
    );
}

/// `grasm fmt` leaves a raw block exactly as written.
///
/// The formatter is line-based and was written when strings could not span
/// lines. Raw blocks changed that, and it indented their *contents* — which
/// changes what the string is — then `--check` reported the correct file as
/// unformatted, so CI would have forced the corruption.
#[test]
fn fmt_does_not_touch_raw_string_blocks() {
    let w = Work::new("fmtraw");
    std::fs::create_dir_all(&w.0).unwrap();
    let src = w.0.join("raw.gr");
    let program = "fn main() {\n\
                   \x20   if true {\n\
                   \x20       let doc = \"\"\"\n\
                   alpha\n\
                   \x20 beta\n\
                   \n\
                   \"\"\"\n\
                   \x20       print(\"{doc.len()}\")\n\
                   \x20   }\n\
                   }\n";
    std::fs::write(&src, program).unwrap();

    let (check_code, check_text) = run(&["fmt", src.to_str().unwrap(), "--check"]);
    assert_eq!(
        check_code, 0,
        "a correctly formatted file must pass --check:\n{check_text}"
    );
    let (code, text) = run(&["fmt", src.to_str().unwrap()]);
    assert_eq!(code, 0, "fmt should succeed:\n{text}");
    let after = std::fs::read_to_string(&src).unwrap();
    assert_eq!(
        after, program,
        "the raw block's contents are the program's data, not its layout"
    );
}

/// `grasm api` derives the release kind from the public surface.
///
/// A version number is supposed to answer "will this break me", and one a
/// human typed cannot. This one is read off the code: an item that
/// disappeared or changed shape is breaking, whatever the changelog claims.
#[test]
fn api_classifies_changes_by_what_actually_changed() {
    let w = Work::new("api");
    std::fs::create_dir_all(&w.0).unwrap();
    let lib = w.0.join("lib.gr");
    std::fs::write(
        &lib,
        "type Point = { x: i64, y: i64 }\n\
         pub fn origin() -> Point {\n    Point{ x: 0, y: 0 }\n}\n\
         fn helper() -> i64 {\n    1\n}\n",
    )
    .unwrap();
    let main = w.0.join("main.gr");
    std::fs::write(
        &main,
        "import lib\n\nfn main() {\n    print(\"{lib.origin().x}\")\n}\n",
    )
    .unwrap();

    let (code, surface) = run(&["api", main.to_str().unwrap()]);
    assert_eq!(code, 0, "should print a surface:\n{surface}");
    assert!(surface.contains("lib\torigin"), "surface:\n{surface}");
    // A private function is not surface.
    assert!(
        !surface.contains("helper"),
        "private item leaked:\n{surface}"
    );
    let saved = w.0.join("v1.api");
    std::fs::write(&saved, &surface).unwrap();

    // Unchanged.
    let (code, text) = run(&[
        "api",
        main.to_str().unwrap(),
        "--against",
        saved.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    assert!(text.contains("PATCH"), "{text}");

    // Additive.
    std::fs::write(
        &lib,
        "type Point = { x: i64, y: i64 }\n\
         pub fn origin() -> Point {\n    Point{ x: 0, y: 0 }\n}\n\
         pub fn unit() -> Point {\n    Point{ x: 1, y: 1 }\n}\n\
         fn helper() -> i64 {\n    1\n}\n",
    )
    .unwrap();
    let (code, text) = run(&[
        "api",
        main.to_str().unwrap(),
        "--against",
        saved.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "additive must not fail:\n{text}");
    assert!(text.contains("MINOR") && text.contains("ADDED"), "{text}");

    // Breaking: a field's type changed.
    std::fs::write(
        &lib,
        "type Point = { x: f64, y: i64 }\n\
         pub fn origin() -> Point {\n    Point{ x: 0.0, y: 0 }\n}\n\
         fn helper() -> i64 {\n    1\n}\n",
    )
    .unwrap();
    let (code, text) = run(&[
        "api",
        main.to_str().unwrap(),
        "--against",
        saved.to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "breaking must fail the check:\n{text}");
    assert!(text.contains("MAJOR") && text.contains("CHANGED"), "{text}");
}

/// A type alias substituted into a signature is not a breaking change.
///
/// `[T-TYPE-ALIAS]`: an alias is not a new type, and the two are
/// interchangeable everywhere. Comparing the written signatures made
/// `-> str` becoming `-> Text` a MAJOR release and exit 1 on a release gate,
/// for a change no consumer can observe.
#[test]
fn an_alias_substitution_is_not_a_breaking_api_change() {
    let w = Work::new("api");
    let v1 = w.0.join("v1.gr");
    let v2 = w.0.join("v2.gr");
    let v3 = w.0.join("v3.gr");
    let saved = w.0.join("v1.api");
    std::fs::write(
        &v1,
        "pub fn show(n: i64) -> str {\n    \"{n}\"\n}\n\nfn main() {\n    print(\"{show(1)}\")\n}\n",
    )
    .unwrap();
    std::fs::write(
        &v2,
        "type Text = str\n\npub fn show(n: i64) -> Text {\n    \"{n}\"\n}\n\nfn main() {\n    print(\"{show(1)}\")\n}\n",
    )
    .unwrap();
    std::fs::write(
        &v3,
        "pub fn show(n: i64) -> i64 {\n    n\n}\n\nfn main() {\n    print(\"{show(1)}\")\n}\n",
    )
    .unwrap();
    let (code, out) = run(&["api", v1.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    std::fs::write(&saved, out).unwrap();

    let (code, out) = run(&[
        "api",
        v2.to_str().unwrap(),
        "--against",
        saved.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "an alias substitution is not breaking:\n{out}");

    // The other direction still works: a real change is still MAJOR.
    let (code, out) = run(&[
        "api",
        v3.to_str().unwrap(),
        "--against",
        saved.to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "a changed return type is breaking:\n{out}");
}

/// A doc example is a test: `grasm test` runs the examples of every imported
/// module, so a dependency's `///` block executes in the consumer's process
/// and counts against that dependency's declared ceiling.
#[test]
fn a_doc_example_counts_toward_the_capability_ceiling() {
    let w = Work::new("docex");
    w.package(
        "lib",
        OVER_CEILING,
        "/// Doubles a number.\n///\n/// ```\n/// print(\"{twice(2)}\")\n/// ```\npub fn twice(n: i64) -> i64 {\n    n * 2\n}\n",
    );
    let app = w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.quietlib]\npath = \"../lib\"\n",
        "import quietlib\n\nfn main() {\n    print(\"{quietlib.twice(2)}\")\n}\n",
    );
    let entry = app_entry(&app);
    let (code, out) = run(&["check", entry.as_str()]);
    assert_eq!(
        code, 1,
        "a printing doc example is over the ceiling:\n{out}"
    );
    assert!(out.contains("performs `io`"), "{out}");

    // And the form that does not perform anything is accepted.
    std::fs::write(
        w.0.join("lib/src/main.gr"),
        "/// Doubles a number.\n///\n/// ```\n/// assert(twice(2) == 4)\n/// ```\npub fn twice(n: i64) -> i64 {\n    n * 2\n}\n",
    )
    .unwrap();
    let (code, out) = run(&["check", entry.as_str()]);
    assert_eq!(code, 0, "an asserting doc example performs nothing:\n{out}");
}

/// `exit(code)` reports the low eight bits, so `exit(-1)` is a failure.
#[test]
fn exit_reports_the_low_eight_bits() {
    let w = Work::new("exit");
    let f = w.0.join("e.gr");
    std::fs::write(&f, "fn main() {\n    exit(-1)\n}\n").unwrap();
    let (code, out) = run(&["run", f.to_str().unwrap()]);
    assert_eq!(code, 255, "exit(-1) must not report success:\n{out}");
}

/// A consumer's source file cannot replace a dependency's module.
///
/// `[T-MODULE-RESOLVE]`: "a dependency's imports resolve inside that
/// dependency, not in whatever consumer happens to be building it. Otherwise a
/// file at the same relative path in the consumer silently replaced the
/// library's own private module, and the program ran and printed the wrong
/// answer." A planted `app/src/util.gr` did exactly that, and `grasm deps`
/// reported the library as performing nothing while the planted file read the
/// filesystem.
#[test]
fn a_consumer_cannot_shadow_a_dependencys_module() {
    let w = Work::new("shadow");
    // The real `util`, and a `lib` that imports it.
    w.package(
        "util",
        "[package]\nname = \"util\"\nversion = \"0.1.0\"\ncapabilities = []\n",
        "pub fn who() -> str {\n    \"the real util\"\n}\n",
    );
    w.package(
        "lib",
        "[package]\nname = \"lib\"\nversion = \"0.1.0\"\ncapabilities = []\n\n[dependencies.util]\npath = \"../util\"\n",
        "import util\n\npub fn ask() -> str {\n    util.who()\n}\n",
    );
    let app = w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.lib]\npath = \"../lib\"\n",
        "import lib\n\nfn main() {\n    print(\"{lib.ask()}\")\n}\n",
    );
    // The consumer plants a file with the name the library imports.
    std::fs::write(
        app.join("src/util.gr"),
        "pub fn who() -> str {\n    \"hijacked\"\n}\n",
    )
    .unwrap();
    let entry = app_entry(&app);
    let (code, out) = run(&["run", entry.as_str()]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("the real util"),
        "the dependency's own module must win:\n{out}"
    );
}

/// A package may import only what its own manifest declares.
///
/// `[T-MODULE-DEP]`: "A package may also import a dependency declared in its
/// `grasm.toml`." Reaching a dependency's dependency by name gave the consumer
/// a hard dependency with no version requirement and no manifest record.
#[test]
fn an_undeclared_transitive_dependency_is_not_importable() {
    let w = Work::new("undeclared");
    w.package(
        "deep",
        "[package]\nname = \"deep\"\nversion = \"0.1.0\"\ncapabilities = []\n",
        "pub fn secret() -> i64 {\n    42\n}\n",
    );
    w.package(
        "mid",
        "[package]\nname = \"mid\"\nversion = \"0.1.0\"\ncapabilities = []\n\n[dependencies.deep]\npath = \"../deep\"\n",
        "import deep\n\npub fn m() -> i64 {\n    deep.secret()\n}\n",
    );
    let app = w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.mid]\npath = \"../mid\"\n",
        "import mid\nimport deep\n\nfn main() {\n    print(\"{mid.m()} {deep.secret()}\")\n}\n",
    );
    let entry = app_entry(&app);
    let (code, out) = run(&["check", entry.as_str()]);
    assert_eq!(code, 1, "an undeclared package is not importable:\n{out}");
    assert!(out.contains("cannot load module `deep`"), "{out}");
}

/// A handler's effects are charged to the package that *wrote* it, not to the
/// one that installs it.
///
/// `[E-HANDLER-OWN-ROW]` charges an installed non-literal handler with what a
/// handler for that effect does, which is right for the row. Applied across
/// package boundaries it made a library's *ceiling* a statement about its
/// consumer's code: a `capabilities = []` library that installs a handler its
/// consumer passes in failed to build for a `print` written entirely in the
/// consumer, and the help text told the library's author to widen a manifest
/// with nothing wrong in it. Authority flows from whoever wrote the handler.
#[test]
fn a_handler_is_charged_to_the_package_that_wrote_it() {
    let w = Work::new("handlerparam");
    w.package(
        "pure",
        OVER_CEILING.replace("quietlib", "pure").as_str(),
        "effect Log {\n    fn info(m: str)\n}\n\npub fn run(h: LogHandler) {\n    with Log handled_by h {\n        Log.info(\"x\")\n    }\n}\n",
    );
    let app = w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.pure]\npath = \"../pure\"\n",
        "import pure\n\nfn main() {\n    pure.run(LogHandler{ info: |m| print(\"{m}\") })\n}\n",
    );
    let entry = app_entry(&app);
    let (code, out) = run(&["run", entry.as_str()]);
    assert_eq!(code, 0, "the library performs nothing of its own:\n{out}");
    assert!(out.contains('x'), "{out}");
    // And `deps` says the same thing, which is the whole point: two commands,
    // one graph, one answer.
    let out = Command::new(GRASM)
        .arg("deps")
        .arg(".")
        .current_dir(&app)
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("pure             performs []"), "{text}");
    assert!(text.contains("app              performs [io]"), "{text}");
}

/// A library that writes its own printing handler is still charged for it.
///
/// The safety half of the rule above: "charged where it was written" only
/// helps if a handler written *here* is charged here.
#[test]
fn a_handler_a_library_wrote_itself_is_charged_to_it() {
    let w = Work::new("ownhandler");
    w.package(
        "loud",
        "[package]\nname = \"loud\"\nversion = \"0.1.0\"\ncapabilities = []\n",
        "effect Log {\n    fn info(m: str)\n}\n\nfn noisy() -> LogHandler {\n    \
LogHandler{ info: |m| print(\"loud: {m}\") }\n}\n\npub fn run() {\n    let h = noisy()\n    \
with Log handled_by h {\n        Log.info(\"x\")\n    }\n}\n",
    );
    let app = w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.loud]\npath = \"../loud\"\n",
        "import loud\n\nfn main() {\n    loud.run()\n}\n",
    );
    let (code, out) = run(&["check", &app_entry(&app)]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("package `loud` performs `io`"), "{out}");
}

/// An option the command does not know is a mistake, not a comment.
#[test]
fn an_unknown_flag_is_refused() {
    let w = Work::new("flags");
    let f = w.0.join("h.gr");
    std::fs::write(&f, "fn main() {\n    print(\"hi\")\n}\n").unwrap();
    let out_path = w.0.join("out");
    let (code, out) = run(&[
        "build",
        f.to_str().unwrap(),
        "--natives",
        "-o",
        out_path.to_str().unwrap(),
    ]);
    assert_eq!(
        code, 1,
        "a typo in --native must not build a bundle:\n{out}"
    );
    assert!(out.contains("has no option `--natives`"), "{out}");

    // And a value-taking option with no value is a lost operand.
    let (code, out) = run(&["api", f.to_str().unwrap(), "--against"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("needs a value"), "{out}");
}

/// `main`'s discarded failure is reported and exits 1, in every engine
/// ([E-MAIN-ERR]).
#[test]
fn a_discarded_failure_in_main_is_reported() {
    let w = Work::new("mainerr");
    let f = w.0.join("m.gr");
    std::fs::write(
        &f,
        "fn main() {\n    print(\"before\")\n    write_file(\"/no/such/dir/x.txt\", \"data\")\n}\n",
    )
    .unwrap();
    let native = w.0.join("m_native");
    let bundle = w.0.join("m_bundle");
    assert_eq!(
        run(&[
            "build",
            f.to_str().unwrap(),
            "--native",
            "-o",
            native.to_str().unwrap()
        ])
        .0,
        0
    );
    assert_eq!(
        run(&["build", f.to_str().unwrap(), "-o", bundle.to_str().unwrap()]).0,
        0
    );
    let interp = run(&["run", f.to_str().unwrap()]);
    for (name, out) in [
        ("native", Command::new(&native).output().unwrap()),
        ("bundle", Command::new(&bundle).output().unwrap()),
    ] {
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        assert_eq!(
            out.status.code().unwrap_or(-1),
            1,
            "{name} must exit 1:\n{text}"
        );
        assert!(
            text.contains("unhandled error from main"),
            "{name}:\n{text}"
        );
    }
    assert_eq!(interp.0, 1, "{}", interp.1);
}

/// A second file operand is a lost argument, not a second program.
#[test]
fn a_stray_second_operand_is_refused() {
    let w = Work::new("operand");
    let f = w.0.join("h.gr");
    std::fs::write(&f, "fn main() {\n    print(\"hi\")\n}\n").unwrap();
    let (code, out) = run(&["check", f.to_str().unwrap(), "/nonexistent.gr"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("takes one file"), "{out}");
    // Program arguments after `--` are still the program's.
    let (code, out) = run(&["run", f.to_str().unwrap(), "--", "a", "b"]);
    assert_eq!(code, 0, "{out}");
}

/// The "looked in" list names where the loader actually went.
///
/// An import made *inside* a dependency was reported against the consumer's
/// source tree — paths the loader never opens for it — so the one place worth
/// looking was the one place the message did not name.
#[test]
fn a_failed_import_names_the_paths_that_were_tried() {
    let w = Work::new("lookedin");
    w.package(
        "libb",
        "[package]\nname = \"libb\"\nversion = \"0.1.0\"\ncapabilities = []\n",
        "pub fn inc(n: i64) -> i64 {\n    n + 1\n}\n",
    );
    w.package(
        "liba",
        "[package]\nname = \"liba\"\nversion = \"0.1.0\"\ncapabilities = []\n\n[dependencies.libb]\npath = \"../libb\"\n",
        "import libb\nimport nope.here\n\npub fn twice(n: i64) -> i64 {\n    libb.inc(n) * 2\n}\n",
    );
    let app = w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.liba]\npath = \"../liba\"\n",
        "import liba\n\nfn main() {\n    print(\"{liba.twice(4)}\")\n}\n",
    );
    let (code, out) = run(&["check", &app_entry(&app)]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("liba/src/nope/here.gr"),
        "the candidate inside `liba` must be named:\n{out}"
    );
    assert!(
        !out.contains("app/src/liba"),
        "the consumer's tree is not where this import is looked for:\n{out}"
    );
    assert!(
        !out.contains(".."),
        "paths are collapsed, not chained:\n{out}"
    );

    // A module two packages down names both real candidates, in order.
    std::fs::write(
        w.0.join("liba/src/main.gr"),
        "import libb\nimport libb.gone\n\npub fn twice(n: i64) -> i64 {\n    libb.inc(n) * 2\n}\n",
    )
    .unwrap();
    let (code, out) = run(&["check", &app_entry(&app)]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("liba/src/libb/gone.gr"), "{out}");
    assert!(out.contains("libb/src/gone.gr"), "{out}");
}

/// A function whose whole body leaves compiles natively ([E-PANIC-EXIT]).
///
/// `fn unreachable() -> T { panic("…") }` is ordinary code and §11 lists no
/// row for it, so refusing it made the gap table wrong either way.
#[test]
fn a_body_that_only_panics_compiles_natively() {
    let w = Work::new("divergebody");
    let f = w.0.join("d.gr");
    std::fs::write(
        &f,
        "fn boom() -> i64 {\n    panic(\"mid\")\n}\nfn main() {\n    print(\"{boom()}\")\n}\n",
    )
    .unwrap();
    let bin = w.0.join("d");
    let (code, out) = run(&[
        "build",
        f.to_str().unwrap(),
        "--native",
        "-o",
        bin.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{out}");
    let got = Command::new(&bin).output().expect("runs");
    assert_eq!(got.status.code(), Some(101));
    assert!(
        String::from_utf8_lossy(&got.stderr).contains("panic: mid"),
        "{:?}",
        String::from_utf8_lossy(&got.stderr)
    );
}

/// Recursion reached through a constant's initializer panics rather than
/// crashing, in a compiled build too ([E-RECURSION-DEPTH]).
///
/// The call graph the stack check is installed from had functions in it and
/// not constants, so `let A = f()` beside `fn f() { A }` closed a cycle it
/// could not see: SIGSEGV, exit 139, empty stderr.
#[test]
fn recursion_through_a_constant_panics_in_a_native_binary() {
    let w = Work::new("constrec");
    let f = w.0.join("c.gr");
    std::fs::write(
        &f,
        "let A = f()\n\nfn f() -> i64 {\n    A\n}\n\nfn main() {\n    print(\"{A}\")\n}\n",
    )
    .unwrap();
    let bin = w.0.join("c");
    let (code, out) = run(&[
        "build",
        f.to_str().unwrap(),
        "--native",
        "-o",
        bin.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{out}");
    let got = Command::new(&bin).output().expect("runs");
    assert_eq!(
        got.status.code(),
        Some(101),
        "stderr: {}",
        String::from_utf8_lossy(&got.stderr)
    );
    assert!(
        String::from_utf8_lossy(&got.stderr).contains("stack overflow"),
        "{:?}",
        String::from_utf8_lossy(&got.stderr)
    );
}

/// A binding annotated `T ! E` may be given either half, and the compiled
/// build must agree with the interpreter ([T-OK-ERR-RETURN]).
///
/// The bare half was bound under a type that says it is a boxed variant, so
/// reading it back dereferenced the payload as one: a wrong answer for some
/// types and a misaligned-pointer abort for others.
#[test]
fn an_annotated_fallible_binding_agrees_between_engines() {
    let w = Work::new("annbind");
    for (tag, src) in [
        (
            "bare-both",
            "type E = | Bad\nfn f(n: i64) -> str ! E {\n    let v: str ! E = if n == 0 {\n        \"c\"\n    } else {\n        \"d\"\n    }\n    v\n}\nfn main() {\n    let a = f(0) ?? \"-\"\n    let b = f(1) ?? \"-\"\n    print(\"{a} {b}\")\n}\n",
        ),
        (
            "bare-and-fail",
            "type E = | Bad\nfn f(n: i64) -> i64 ! E {\n    let v: i64 ! E = if n == 0 {\n        fail(Bad)\n    } else {\n        0 - 7\n    }\n    v\n}\nfn main() {\n    print(\"{f(0) ?? 0 - 1}\")\n    print(\"{f(1) ?? 0 - 1}\")\n}\n",
        ),
    ] {
        let f = w.0.join(format!("{tag}.gr"));
        std::fs::write(&f, src).unwrap();
        let bin = w.0.join(tag);
        let (code, out) = run(&[
            "build",
            f.to_str().unwrap(),
            "--native",
            "-o",
            bin.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{tag}: {out}");
        let interp = run(&["run", f.to_str().unwrap()]);
        let native = Command::new(&bin).output().expect("runs");
        let native_text = String::from_utf8_lossy(&native.stdout).into_owned();
        assert_eq!(interp.1, native_text, "{tag}: engines disagree");
        assert_eq!(native.status.code(), Some(0), "{tag}");
    }
}

/// A function named through its module carries its effect row, exactly as a
/// bare name does ([T-FN-VALUE-ROW], [T-MODULE-PUB]).
///
/// It carried an empty one, so a single `import` was enough to hand a printing
/// function to a `/ {}` row, to `par_map`, to a constant table and to a
/// package declaring `capabilities = []` — every refusal the spec calls closed
/// was open one word longer.
#[test]
fn a_qualified_function_value_keeps_its_row() {
    let w = Work::new("qualrow");
    std::fs::create_dir_all(w.0.join("src")).unwrap();
    std::fs::write(
        w.0.join("src/lib.gr"),
        "pub fn shout(n: i64) -> i64 {\n    print(\"noisy {n}\")\n    n\n}\n",
    )
    .unwrap();
    for (tag, src, needle) in [
        (
            "empty-row",
            "import lib\nfn quiet() -> [i64] / {} {\n    [1, 2].map(lib.shout)\n}\nfn main() {\n    print(\"{quiet()}\")\n}\n",
            "not in its declared row",
        ),
        (
            "par-map",
            "import lib\nfn main() {\n    print(\"{[1, 2].par_map(lib.shout)}\")\n}\n",
            "needs an effect-free function",
        ),
        (
            "written-type",
            "import lib\nfn main() {\n    let f: fn(i64) -> i64 = lib.shout\n    print(\"{f(1)}\")\n}\n",
            "performs nothing, but this one performs `io`",
        ),
    ] {
        let f = w.0.join(format!("src/{tag}.gr"));
        std::fs::write(&f, src).unwrap();
        let (code, out) = run(&["check", f.to_str().unwrap()]);
        assert_eq!(code, 1, "{tag} must be refused:\n{out}");
        assert!(out.contains(needle), "{tag}: {out}");
    }
    // Calling it is still ordinary.
    let ok = w.0.join("src/ok.gr");
    std::fs::write(
        &ok,
        "import lib\nfn main() {\n    print(\"{lib.shout(1)}\")\n}\n",
    )
    .unwrap();
    let (code, out) = run(&["check", ok.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
}

/// A dependency's `entry` outside its own package is refused before the file
/// is opened.
///
/// The loader read the field directly and so opened the outside file first,
/// quoting a line of it into the diagnostic — a dependency could point at any
/// file the build user can read and have its contents echoed.
#[test]
fn an_entry_outside_the_package_is_not_read() {
    let w = Work::new("entryleak");
    std::fs::create_dir_all(w.0.join("secrets")).unwrap();
    std::fs::write(
        w.0.join("secrets/creds.env"),
        "AWS_SECRET_ACCESS_KEY=hunter2-do-not-share\n",
    )
    .unwrap();
    let secret = w.0.join("secrets/creds.env");
    w.package(
        "lib",
        &format!(
            "[package]\nname = \"lib\"\nversion = \"0.1.0\"\nentry = \"{}\"\n",
            secret.display()
        ),
        "pub fn f() -> i64 {\n    1\n}\n",
    );
    let app = w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.lib]\npath = \"../lib\"\n",
        "import lib\n\nfn main() {\n    print(\"{lib.f()}\")\n}\n",
    );
    let (code, out) = run(&["check", &app_entry(&app)]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("must be a path inside the package"), "{out}");
    assert!(
        !out.contains("hunter2"),
        "the file's contents must not reach the diagnostic:\n{out}"
    );
    // And the message is one line of prose, not Rust's source indentation.
    assert!(!out.contains("  \n") && !out.contains("—       "), "{out}");
}

/// A deep package chain builds natively: the incremental cache names an
/// object by a digest, not by every package between it and the entry.
#[test]
fn a_deep_package_chain_builds_natively() {
    let w = Work::new("longname");
    let n = 10;
    for i in 0..n {
        let name = format!("acme_pkg_component_{i}");
        let mut manifest =
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\ncapabilities = []\n");
        let src = if i + 1 < n {
            let next = format!("acme_pkg_component_{}", i + 1);
            manifest.push_str(&format!("\n[dependencies.{next}]\npath = \"../{next}\"\n"));
            format!("import {next}\n\npub fn v() -> i64 {{\n    {i} + {next}.v()\n}}\n")
        } else {
            format!("pub fn v() -> i64 {{\n    {i}\n}}\n")
        };
        w.package(&name, &manifest, &src);
    }
    let app = w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.acme_pkg_component_0]\npath = \"../acme_pkg_component_0\"\n",
        "import acme_pkg_component_0\n\nfn main() {\n    print(\"{acme_pkg_component_0.v()}\")\n}\n",
    );
    let bin = w.0.join("chained");
    let (code, out) = run(&[
        "build",
        &app_entry(&app),
        "--native",
        "-o",
        bin.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{out}");
    let got = Command::new(&bin).output().expect("runs");
    assert_eq!(String::from_utf8_lossy(&got.stdout).trim(), "45");
}

/// Builds a chain of `n` packages, `p0` depending on `p1` and so on, under a
/// fresh working directory. Returns the app package that imports `p0`.
fn chain(w: &Work, n: usize) -> PathBuf {
    for i in 0..n {
        let mut manifest =
            format!("[package]\nname = \"p{i}\"\nversion = \"0.1.0\"\ncapabilities = []\n");
        let src = if i + 1 < n {
            manifest.push_str(&format!(
                "\n[dependencies.p{}]\npath = \"../p{}\"\n",
                i + 1,
                i + 1
            ));
            format!(
                "import p{}\n\npub fn v() -> i64 {{\n    1 + p{}.v()\n}}\n",
                i + 1,
                i + 1
            )
        } else {
            "pub fn v() -> i64 {\n    0\n}\n".to_string()
        };
        w.package(&format!("p{i}"), &manifest, &src);
    }
    w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.p0]\npath = \"../p0\"\n",
        "import p0\n\nfn main() {\n    print(\"{p0.v()}\")\n}\n",
    )
}

/// Past the depth backstop the loader says the limit, instead of listing the
/// paths its degraded resolution invented.
///
/// A dependency name past the limit has no package prefix, so the import fell
/// through to dotted-module-path resolution: sixty-six candidates, each a
/// four-hundred-character path made of every package between the entry and
/// this one, naming a file nobody wrote.
#[test]
fn a_graph_past_the_depth_limit_says_the_limit() {
    let w = Work::new("toodeep");
    let app = chain(&w, 70);
    let (code, out) = run(&["check", &app_entry(&app)]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("more than 64 deep from the entry"), "{out}");
    assert!(
        !out.contains("/p10/p11/p12"),
        "concatenated path in:\n{out}"
    );
    assert!(out.lines().count() < 12, "{out}");
}

/// A chain just under the limit resolves and runs.
#[test]
fn a_chain_under_the_depth_limit_runs() {
    let w = Work::new("deepok");
    let app = chain(&w, 60);
    let (code, out) = run(&["run", &app_entry(&app)]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.trim(), "59");
}

/// A "looked in" list one line per package hop is capped, keeping the tail —
/// the package that wrote the import, and the manifest missing the name.
#[test]
fn a_long_looked_in_list_keeps_its_tail() {
    let w = Work::new("longlist");
    let app = chain(&w, 20);
    std::fs::write(
        w.0.join("p19/src/main.gr"),
        "import nope.here\n\npub fn v() -> i64 {\n    0\n}\n",
    )
    .unwrap();
    let (code, out) = run(&["check", &app_entry(&app)]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("earlier candidate(s)"), "{out}");
    assert!(
        out.contains("no dependency named `nope` in `p19`'s grasm.toml"),
        "the last line has to be the one worth reading:\n{out}"
    );
    assert!(out.contains("p19/src/nope/here.gr"), "{out}");
}

/// A syntax error inside a dependency is reported against the dependency's own
/// file, at a path a reader can open.
///
/// `deps` named the *consumer's* entry for it — `./src/main.gr`, a file with
/// nothing wrong in it — and spelled the dependency's path
/// `./../dep/src/main.gr`.
#[test]
fn deps_blames_the_file_the_error_is_in() {
    let w = Work::new("blame");
    w.package(
        "dep",
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\ncapabilities = []\n",
        "pub fn f( -> i64 {\n    1\n}\n",
    );
    let app = w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.dep]\npath = \"../dep\"\n",
        "import dep\n\nfn main() {\n    print(\"{dep.f()}\")\n}\n",
    );
    let out = Command::new(GRASM)
        .arg("deps")
        .arg(".")
        .current_dir(&app)
        .output()
        .expect("runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(!text.contains("./../"), "un-normalized path in:\n{text}");
    assert!(!text.contains("`app`: package `app`"), "{text}");
    // Both rows name the dependency's file; the consumer's row says why it is
    // being told about a file it does not own.
    assert_eq!(
        text.matches("../dep/src/main.gr").count(),
        2,
        "both rows name the failing file:\n{text}"
    );
    assert!(
        text.contains(
            "cannot analyze package `app`: `../dep/src/main.gr` (reached through a dependency)"
        ),
        "{text}"
    );
    assert!(
        !text.contains("cannot analyze package `app`: `src/main.gr`"),
        "the consumer's own entry is not the failing file:\n{text}"
    );
}

/// A bundled executable's stack trace names the entry file, the same way it
/// already named every imported module.
///
/// Every frame but the entry's was labelled from its module name, so a trace
/// read `helper::crash at helper.gr:2:5` and then
/// `main::main at <bundled>:5:5` — the same trace naming the same kind of
/// thing two ways, one of which is not a file. Only the basename travels: the
/// directory the build happened in names nothing on the machine running the
/// binary.
#[test]
fn a_bundles_trace_names_the_entry_file() {
    let w = Work::new("bundletrace");
    let dir = w.0.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("helper.gr"),
        "pub fn crash(n: i64) -> i64 {\n    n / 0\n}\n",
    )
    .unwrap();
    let entry = dir.join("app.gr");
    std::fs::write(
        &entry,
        "import helper\n\nfn main() {\n    print(\"{helper.crash(2)}\")\n}\n",
    )
    .unwrap();
    let bin = w.0.join("bundled");
    let (code, out) = run(&[
        "build",
        entry.to_str().unwrap(),
        "-o",
        bin.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{out}");

    let got = Command::new(&bin).output().expect("runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&got.stdout),
        String::from_utf8_lossy(&got.stderr)
    );
    assert_eq!(got.status.code(), Some(101), "{text}");
    assert!(!text.contains("<bundled>"), "{text}");
    assert!(text.contains("helper::crash at helper.gr:2:5"), "{text}");
    assert!(text.contains("main::main at app.gr:4:5"), "{text}");
    // The build directory is not carried into the binary.
    assert!(
        !text.contains(w.0.to_str().unwrap()),
        "the build path must not travel with the bundle:\n{text}"
    );
}

/// `grasm deps` refuses to certify a package that does not typecheck.
///
/// Only *parse* failures produced `CANNOT ANALYZE`. A package that parsed but
/// did not typecheck was reported `performs []` and written into a lockfile —
/// and "does not typecheck" is exactly where an effect the ceiling would have
/// caught can hide, since a refused declaration performs nothing the inference
/// can see.
#[test]
fn deps_refuses_a_package_that_does_not_typecheck() {
    let w = Work::new("depstypes");
    w.package(
        "lib",
        "[package]\nname = \"lib\"\nversion = \"0.1.0\"\ncapabilities = []\n",
        "pub fn go(n: i64) -> i64 {\n    \"not an int\"\n}\n",
    );
    let app = w.package(
        "con",
        "[package]\nname = \"con\"\nversion = \"0.1.0\"\n\n[dependencies.lib]\npath = \"../lib\"\n",
        "import lib\n\nfn main() {\n    print(\"{lib.go(3)}\")\n}\n",
    );
    let out = Command::new(GRASM)
        .arg("deps")
        .arg(".")
        .current_dir(&app)
        .output()
        .expect("runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("CANNOT ANALYZE"), "{text}");
    assert!(text.contains("must return `i64`"), "{text}");
    assert!(!text.contains("performs []"), "{text}");
    assert!(
        !app.join("grasm.lock").exists(),
        "a package that cannot be analysed must not be written into a lockfile"
    );
}

/// A dependency cannot declare away the capability ceiling.
///
/// `effect io { … }` parsed, and the `with io handled_by …` it made available
/// discharged the *builtin* `io` — so a `capabilities = []` dependency printed
/// and wrote files while `grasm deps` certified it as performing nothing.
#[test]
fn a_dependency_cannot_declare_away_its_ceiling() {
    let w = Work::new("ioceiling");
    w.package(
        "lib",
        "[package]\nname = \"lib\"\nversion = \"0.1.0\"\ncapabilities = []\n",
        "effect io {\n    fn nothing() -> i64\n}\n\npub fn go(n: i64) -> i64 {\n    \
with io handled_by ioHandler{ nothing: || 1 } {\n        print(\"lib wrote to your stdout\")\n    }\n    \
n * 2\n}\n",
    );
    let app = w.package(
        "con",
        "[package]\nname = \"con\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n\n[dependencies.lib]\npath = \"../lib\"\n",
        "import lib\n\nfn main() {\n    print(\"{lib.go(3)}\")\n}\n",
    );
    let entry = app_entry(&app);
    for args in [vec!["check", entry.as_str()], vec!["run", entry.as_str()]] {
        let (code, out) = run(&args);
        assert_eq!(code, 1, "{args:?}: {out}");
        assert!(
            out.contains("is reserved by the language"),
            "{args:?}: {out}"
        );
        assert!(!out.contains("lib wrote to your stdout"), "{args:?}: {out}");
    }
    let out = Command::new(GRASM)
        .arg("deps")
        .arg(".")
        .current_dir(&app)
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(1));
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("CANNOT ANALYZE"), "{text}");
    assert!(!text.contains("performs []"), "{text}");
}

/// A build that cannot write its output says so in the tool's own voice.
///
/// The bundle path reported a bare `No such file or directory (os error 2)`
/// naming nothing, and the native path surfaced four lines of the linker's own
/// words — including grasm's temporary object file, a path with no meaning
/// outside that process.
#[test]
fn a_build_that_cannot_write_its_output_says_so_plainly() {
    let w = Work::new("outpath");
    let dir = w.0.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("t.gr");
    std::fs::write(&entry, "fn main() {\n    print(\"x\")\n}\n").unwrap();
    let missing = w.0.join("no/such/dir/out");
    for extra in [vec![], vec!["--native"]] {
        let mut args = vec!["build", entry.to_str().unwrap()];
        args.extend(extra.iter().copied());
        args.extend(["-o", missing.to_str().unwrap()]);
        let (code, out) = run(&args);
        assert_eq!(code, 1, "{args:?}: {out}");
        assert!(out.contains("cannot write"), "{args:?}: {out}");
        assert!(out.contains("is not a directory"), "{args:?}: {out}");
        assert!(
            !out.contains("grasm_native_"),
            "internal path leaked:\n{out}"
        );
        assert!(!out.contains("out.o"), "internal path leaked:\n{out}");
    }
    // And a build that does work is unaffected.
    let good = w.0.join("good");
    let (code, out) = run(&[
        "build",
        entry.to_str().unwrap(),
        "--native",
        "-o",
        good.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{out}");
    let got = Command::new(&good).output().expect("runs");
    assert_eq!(String::from_utf8_lossy(&got.stdout).trim(), "x");
}

/// A `capabilities = []` package cannot reach a C function by taking a
/// function value that declares the row.
///
/// The row on a `[fn(i64) -> i64 / {io, unsafe}]` element was dropped by the
/// `for` that bound it, so the library called into `getpid` while
/// `grasm deps` certified it as performing nothing and wrote that into the
/// lockfile.
#[test]
fn a_ceiling_holds_against_a_function_value_parameter() {
    let w = Work::new("rowceiling");
    w.package(
        "lib",
        "[package]\nname = \"lib\"\nversion = \"0.1.0\"\ncapabilities = []\n",
        "pub fn call_all(fs: [fn(i64) -> i64 / {io, unsafe}]) -> i64 {\n    \
for f in fs {\n        return f(0)\n    }\n    0\n}\n",
    );
    let app = w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = [\"io\", \"unsafe\"]\n\n[dependencies.lib]\npath = \"../lib\"\n",
        "import lib\n\nextern fn getpid() -> i64\n\nfn pid(x: i64) -> i64 {\n    \
unsafe {\n        getpid() + x\n    }\n}\n\nfn main() {\n    print(\"{lib.call_all([pid])}\")\n}\n",
    );
    let entry = app_entry(&app);
    for args in [vec!["check", entry.as_str()], vec!["run", entry.as_str()]] {
        let (code, out) = run(&args);
        assert_eq!(code, 1, "{args:?}: {out}");
        assert!(out.contains("package `lib` performs"), "{args:?}: {out}");
    }
    let out = Command::new(GRASM)
        .arg("deps")
        .arg(".")
        .current_dir(&app)
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(1));
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(!text.contains("lib              performs []"), "{text}");
    assert!(!app.join("grasm.lock").exists(), "{text}");
}

/// A flag written before the directory does not swallow its place.
///
/// `args.get(1)` was the whole operand scan, so `grasm deps --locked <dir>`
/// reported on the *working directory* and exited 0 with a clean bill of
/// health for a package that was never named — including when `<dir>` did not
/// exist.
#[test]
fn a_flag_before_the_directory_does_not_swallow_it() {
    let w = Work::new("depsflag");
    let pkg = w.package(
        "dd",
        "[package]\nname = \"dd\"\nversion = \"0.1.0\"\ncapabilities = []\n",
        "fn main() {\n    let x = 1\n}\n",
    );
    // Establish a lockfile so `--locked` has something to compare against.
    let (code, out) = run(&["deps", pkg.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");

    let missing = w.0.join("nowhere");
    let out = Command::new(GRASM)
        .args(["deps", "--locked", missing.to_str().unwrap()])
        .current_dir(&pkg)
        .output()
        .expect("runs");
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(!text.contains("up to date"), "{text}");
    assert!(text.contains("nowhere"), "{text}");

    // And the flag still works when the directory is real.
    let out = Command::new(GRASM)
        .args(["deps", "--locked", pkg.to_str().unwrap()])
        .current_dir(&w.0)
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0));
}

/// A flag written before the file does not take its place.
///
/// The operand scan read position 1 only, so `grasm test --cases 5 f.gr` —
/// the ordinary spelling — printed the usage line, which reads as "that file
/// is wrong".
#[test]
fn a_flag_before_the_file_does_not_take_its_place() {
    let w = Work::new("flagfirst");
    let dir = w.0.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("t.gr");
    std::fs::write(
        &entry,
        "test \"one\" {\n    assert(1 == 1)\n}\n\nfn main() {\n    print(\"x\")\n}\n",
    )
    .unwrap();
    let p = entry.to_str().unwrap();

    let (code, out) = run(&["test", "--cases", "5", p]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("1 passed"), "{out}");
    // The same flags after the file still work.
    let (code, out) = run(&["test", p, "--cases", "5"]);
    assert_eq!(code, 0, "{out}");

    // A bad flag value is still named, whichever side it is written on.
    for args in [
        vec!["test", "--cases", "0", p],
        vec!["test", p, "--cases", "0"],
    ] {
        let (code, out) = run(&args);
        assert_eq!(code, 1, "{args:?}: {out}");
        assert!(
            out.contains("--cases must be at least 1"),
            "{args:?}: {out}"
        );
    }

    let md = w.0.join("out.md");
    let (code, out) = run(&["doc", "-o", md.to_str().unwrap(), p]);
    assert_eq!(code, 0, "{out}");
    assert!(md.exists(), "{out}");

    let (code, out) = run(&["bench", "--filter", "nothing", p]);
    assert_eq!(code, 0, "{out}");
}

/// A doc example in an *imported* module credits the lines it ran.
///
/// A group's examples run in a synthesized program where the owner's source
/// is inlined into a new entry, so the offsets they report belong to that
/// entry's span space. Read against the original they landed on the wrong
/// file, and the function the example had just run was reported uncovered —
/// the failure the coverage rule exists to prevent, one import away.
#[test]
fn a_doc_example_in_an_imported_module_counts_toward_coverage() {
    let w = Work::new("doccov");
    let dir = w.0.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("helper.gr"),
        "/// Doubles a number.\n///\n/// ```\n/// assert(dbl(2) == 4)\n/// ```\n\
pub fn dbl(n: i64) -> i64 {\n    n * 2\n}\n",
    )
    .unwrap();
    let entry = dir.join("app.gr");
    std::fs::write(
        &entry,
        "import helper\n\nfn main() {\n    print(\"{helper.dbl(1)}\")\n}\n",
    )
    .unwrap();
    let (code, out) = run(&["test", entry.to_str().unwrap(), "--coverage"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("doc example helper::dbl ... ok"), "{out}");
    // `helper.gr:7` is the line the example ran; only `app.gr:4` is left.
    assert!(!out.contains("helper.gr: 1 uncovered"), "{out}");
    assert!(out.contains("app.gr: 1 uncovered"), "{out}");
    assert!(!out.contains("coverage: 0.0%"), "{out}");
}

/// A bare identifier in a pattern binds unless it names a nullary variant of
/// *the scrutinee's type* ([G-PAT-BIND]) — in every engine.
///
/// The compiled backend asked whether the name was a nullary variant of any
/// type in the program, so `match n { None -> … }` over an `i64` became a
/// variant test on a scalar: the arm's body read a name nothing had bound and
/// the binary died with SIGSEGV and no output, while the interpreter printed
/// the answer.
#[test]
fn a_pattern_binder_that_shares_a_variants_name_agrees_across_engines() {
    let w = Work::new("patbind");
    let dir = w.0.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("p.gr");
    std::fs::write(
        &entry,
        "type A = | On | Off\ntype B = | Yes | No\ntype W = | Wrap(v: i64)\n\n\
fn main() {\n    \
let n = 7\n    let a = match n { None -> \"bound\" }\n    print(a)\n    \
let o: Option[i64] = Some(3)\n    let b = match o { Yes -> \"bound-o\" }\n    print(b)\n    \
let c = match o { Some(v) -> \"some\" None -> \"none\" }\n    print(c)\n    \
let p = (1, 2)\n    let d = match p { (Yes, No) -> \"bound-t\" }\n    print(d)\n    \
let e = match W.Wrap(5) { Wrap(Yes) -> \"bound-w\" }\n    print(e)\n    \
let f = match A.Off { On -> \"on\" Off -> \"off\" }\n    print(f)\n}\n",
    )
    .unwrap();
    let p = entry.to_str().unwrap();
    let (code, expected) = run(&["run", p]);
    assert_eq!(code, 0, "{expected}");
    assert!(expected.contains("bound"), "{expected}");

    for extra in [vec![], vec!["--native"]] {
        let bin = w.0.join(if extra.is_empty() { "b" } else { "n" });
        let mut args = vec!["build", p];
        args.extend(extra.iter().copied());
        args.extend(["-o", bin.to_str().unwrap()]);
        let (code, out) = run(&args);
        assert_eq!(code, 0, "{args:?}: {out}");
        let got = Command::new(&bin).output().expect("runs");
        assert_eq!(got.status.code(), Some(0), "{args:?}");
        assert_eq!(
            String::from_utf8_lossy(&got.stdout),
            expected,
            "{args:?} disagrees with the interpreter"
        );
    }
}

/// A doc example that prints keeps its own status line.
///
/// The runner recovered each example's status by splitting one shared output
/// buffer into lines, so a `print` inside a body slid the mapping by one: the
/// example's own status was lost and the *next* example was reported with it.
/// A failing example printed `ok`, with its reason gone — a library whose
/// tests are failing looking exactly like one whose tests passed.
#[test]
fn a_printing_doc_example_does_not_steal_the_next_ones_status() {
    let w = Work::new("docprint");
    let dir = w.0.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("d.gr");
    std::fs::write(
        &entry,
        "/// ```\n/// print(\"x\")\n/// ```\nfn a() -> i64 {\n    1\n}\n\n\
/// ```\n/// assert(b() == 99)\n/// ```\nfn b() -> i64 {\n    2\n}\n\n\
fn main() {\n    print(\"{a()}{b()}\")\n}\n",
    )
    .unwrap();
    let (code, out) = run(&["test", entry.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    // The example's own output reaches stdout.
    assert!(out.contains("\nx\n") || out.starts_with("x\n"), "{out}");
    assert!(out.contains("doc example main::a ... ok"), "{out}");
    assert!(
        out.contains("doc example main::b ... FAILED: assertion failed"),
        "{out}"
    );
    assert!(out.contains("1 passed, 1 failed"), "{out}");
}

/// A handler's operations capture at the moment the handler is *created*, in
/// every engine ([E-CLOSURE-CAPTURE]).
///
/// The compiled backend filled the environment at the `with` instead, so a
/// binding changed between the two was read at its new value: the interpreter
/// answered 1 and the binary answered 100, with matching exit codes and
/// nothing on stderr to say the two disagreed.
#[test]
fn a_handlers_captures_are_taken_where_it_is_written() {
    let w = Work::new("hcap");
    let dir = w.0.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("h.gr");
    std::fs::write(
        &entry,
        "effect Src {\n    fn get() -> i64\n}\n\ntype Sub = { n: i64 }\n\n\
fn main() {\n    \
var n = 1\n    let h = SrcHandler{ get: || n }\n    n = 100\n    \
with Src handled_by h {\n        print(\"{Src.get()}\")\n    }\n    \
var xs = [1, 2, 3]\n    let g = SrcHandler{ get: || xs.len() }\n    xs.push(4)\n    \
with Src handled_by g {\n        print(\"{Src.get()}\")\n    }\n    \
var sub = Sub{ n: 5 }\n    let u = SrcHandler{ get: || sub.n }\n    sub.n = 50\n    \
with Src handled_by u {\n        print(\"{Src.get()}\")\n    }\n    \
var k = 1\n    let a = SrcHandler{ get: || k }\n    k = 2\n    \
let b = SrcHandler{ get: || k * 10 }\n    k = 3\n    \
with Src handled_by a {\n        print(\"{Src.get()}\")\n    }\n    \
with Src handled_by b {\n        print(\"{Src.get()}\")\n    }\n    \
with Src handled_by SrcHandler{ get: || k } {\n        print(\"{Src.get()}\")\n    }\n}\n",
    )
    .unwrap();
    let p = entry.to_str().unwrap();
    let (code, expected) = run(&["run", p]);
    assert_eq!(code, 0, "{expected}");
    assert_eq!(expected, "1\n3\n5\n1\n20\n3\n", "{expected}");

    for extra in [vec![], vec!["--native"]] {
        let bin = w.0.join(if extra.is_empty() { "b" } else { "n" });
        let mut args = vec!["build", p];
        args.extend(extra.iter().copied());
        args.extend(["-o", bin.to_str().unwrap()]);
        let (code, out) = run(&args);
        assert_eq!(code, 0, "{args:?}: {out}");
        let got = Command::new(&bin).output().expect("runs");
        assert_eq!(
            String::from_utf8_lossy(&got.stdout),
            expected,
            "{args:?} disagrees with the interpreter"
        );
    }
}

/// A dependency's doc example is the dependency's, not the consumer's.
///
/// Every other loop in the capability pass skips a foreign module and this one
/// did not, so a consumer performing nothing at all was charged for a `print`
/// in a `///` block in someone else's package — with no `performed:` location,
/// so its author had no pointer to a file they did not write.
#[test]
fn a_dependencys_doc_example_is_not_the_consumers() {
    let w = Work::new("depdoc");
    w.package(
        "dep",
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\ncapabilities = [\"io\"]\n",
        "/// Adds.\n///\n/// ```\n/// print(\"{add(2, 3)}\")\n/// ```\n\
pub fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n",
    );
    let app = w.package(
        "app",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\ncapabilities = []\n\n[dependencies.dep]\npath = \"../dep\"\n",
        "import dep\n\nfn main() {\n    let n = dep.add(1, 2)\n}\n",
    );
    let (code, out) = run(&["check", &app_entry(&app)]);
    assert_eq!(code, 0, "the consumer performs nothing:\n{out}");
    let out = Command::new(GRASM)
        .arg("deps")
        .arg(".")
        .current_dir(&app)
        .output()
        .expect("runs");
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("app              performs []"), "{text}");
}

/// A directory given where a file belongs is named in the refusal.
///
/// The path was collapsed for display and `.` is all current-dir components,
/// so nothing survived: `error: cannot read ``: Is a directory` — a message
/// naming nothing, which is the failure [L-ENCODING] records for the BOM.
#[test]
fn a_directory_where_a_file_belongs_is_named() {
    let w = Work::new("dirarg");
    let dir = w.0.to_str().unwrap().to_string();
    for cmd in ["run", "check", "test", "doc", "fmt"] {
        let (code, out) = run(&[cmd, "."]);
        assert_eq!(code, 1, "{cmd}: {out}");
        assert!(!out.contains("``"), "{cmd} names nothing:\n{out}");
        assert!(out.contains("`.`"), "{cmd}: {out}");
        // And with a real directory path, not just `.`.
        let (code, out) = run(&[cmd, dir.as_str()]);
        assert_eq!(code, 1, "{cmd}: {out}");
        assert!(!out.contains("``"), "{cmd} names nothing:\n{out}");
    }
}

/// A native build leaves no temporary directory behind.
///
/// The scratch directory holding the object file was created and never
/// removed, on any path: 33,000 of them and 2.2 GB accumulated on one machine
/// during this project's own test runs, which is a compiler filling the disk
/// of anyone who builds in a loop.
#[test]
fn a_native_build_leaves_no_scratch_directory() {
    let w = Work::new("scratch");
    let dir = w.0.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("t.gr");
    std::fs::write(&entry, "fn main() {\n    print(\"x\")\n}\n").unwrap();
    let p = entry.to_str().unwrap();

    // A temp directory of this test's own. Counting in the shared one also
    // counted the scratch directories of other tests' builds running in
    // parallel, so the test failed whenever one was in flight at the moment
    // it looked -- twice in a row on 2026-09-28 -- without any leak.
    let tmp = w.0.join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let leaked = || -> usize {
        std::fs::read_dir(&tmp)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with("grasm_native_"))
                    .count()
            })
            .unwrap_or(0)
    };
    let build = |out: &str| -> i32 {
        Command::new(GRASM)
            .args(["build", p, "--native", "-o", out])
            .env("TMPDIR", &tmp)
            .output()
            .expect("runs")
            .status
            .code()
            .unwrap_or(-1)
    };
    let before = leaked();

    let good = w.0.join("out");
    for _ in 0..3 {
        assert_eq!(build(good.to_str().unwrap()), 0);
    }
    // And on the failure path, where a dozen `?` return early.
    assert_eq!(build("/nonexistent/dir/x"), 1);

    assert_eq!(
        leaked(),
        before,
        "a native build left a scratch directory behind"
    );
}

/// The bare ok half of a mixed branch reads back the same in every engine.
///
/// The interpreter stored the raw value in every annotated position that does
/// not re-wrap — a `let`, a struct field, a list element — so it printed `3`
/// where a native build printed `Ok(3)`, compared unequal to `Ok(3)`, and
/// panicked `no match arm matched` on a `match` a native build satisfied. The
/// normative engine was the wrong one, at exit 0 with nothing on stderr.
#[test]
fn a_mixed_branch_reads_back_the_same_in_every_engine() {
    let w = Work::new("okwrap");
    let dir = w.0.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("m.gr");
    std::fs::write(
        &entry,
        "type E = | Bad\ntype Box = { v: i64 ! E }\n\nfn main() {\n    \
let b = Box{ v: if true { 3 } else { Ok(4) } }\n    print(\"boxed: {b}\")\n    \
let l: [i64 ! E] = [if true { 5 } else { Ok(6) }]\n    print(\"list: {l}\")\n    \
let a: i64 ! E = if true { 1 } else { Ok(2) }\n    print(\"eq: {a == Ok(1)}\")\n    \
let c: i64 ! E = match 1 {\n        1 -> 7\n        _ -> Ok(8)\n    }\n    \
match c {\n        Ok(n) -> print(\"Ok({n})\")\n        Err(x) -> print(\"Err\")\n    }\n}\n",
    )
    .unwrap();
    let p = entry.to_str().unwrap();
    let (code, expected) = run(&["run", p]);
    assert_eq!(code, 0, "{expected}");
    assert_eq!(
        expected, "boxed: Box{v: Ok(3)}\nlist: [Ok(5)]\neq: true\nOk(7)\n",
        "the interpreter is normative and has to be right"
    );
    for extra in [vec![], vec!["--native"]] {
        let bin = w.0.join(if extra.is_empty() { "b" } else { "n" });
        let mut args = vec!["build", p];
        args.extend(extra.iter().copied());
        args.extend(["-o", bin.to_str().unwrap()]);
        let (code, out) = run(&args);
        assert_eq!(code, 0, "{args:?}: {out}");
        let got = Command::new(&bin).output().expect("runs");
        assert_eq!(String::from_utf8_lossy(&got.stdout), expected, "{args:?}");
        let _ = std::fs::remove_file(&bin);
    }
}

/// An incremental `--native` rebuild picks up a changed constant in a module it
/// imports.
///
/// A module's cached object is keyed on its own source and on the program's
/// signature surface — and a signature deliberately omits a constant's *value*,
/// because a constant changing value is not an API break and `grasm api` is
/// right to ignore it. The cache key was wrong to: [E-CONST] makes a
/// module-level `let` "one value for the whole program", the importer's object
/// materialises that value, and changing `pub let K = 10` to `999` left the
/// importer's entry valid. The rebuilt binary printed `10` while `grasm run`
/// and the bundle printed `999` — bump a dependency, rebuild, ship the old
/// constant, across a package boundary too.
///
/// The rule the fix follows is rustc's: key an artifact on a fingerprint of
/// everything it *read*, not on the file it was written in.
#[test]
fn an_incremental_rebuild_sees_a_changed_constant() {
    let w = Work::new("inccache");
    let dir = w.0.join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    let helper = dir.join("helper.gr");
    let main = dir.join("main.gr");
    let out = dir.join("app");
    std::fs::write(
        &main,
        "import helper\n\nfn main() {\n    print(\"{helper.K} {helper.L} {helper.f()}\")\n}\n",
    )
    .unwrap();

    let build_and_run = |expect: &str, what: &str| {
        let (code, msg) = run(&[
            "build",
            main.to_str().unwrap(),
            "--native",
            "-o",
            out.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{what}: {msg}");
        let got = Command::new(&out).output().expect("runs");
        assert_eq!(
            String::from_utf8_lossy(&got.stdout).trim(),
            expect,
            "{what}"
        );
    };

    std::fs::write(
        &helper,
        "pub let K = 10\npub let L = [1, 2]\n\npub fn f() -> i64 {\n    5\n}\n",
    )
    .unwrap();
    build_and_run("10 [1, 2] 5", "first build");

    // Only the dependency changes; the importer's source is untouched, which is
    // exactly the case its cache entry survived.
    std::fs::write(
        &helper,
        "pub let K = 999\npub let L = [7, 8, 9]\n\npub fn f() -> i64 {\n    101\n}\n",
    )
    .unwrap();
    build_and_run("999 [7, 8, 9] 101", "after the dependency changed");

    // And the cache must still *be* a cache: a rebuild that changes nothing
    // reuses every object rather than writing new ones.
    let cache = dir.join(".grasm-cache");
    let before: Vec<_> = std::fs::read_dir(&cache)
        .expect("the cache should exist")
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    assert!(!before.is_empty(), "the incremental build wrote no objects");
    build_and_run("999 [7, 8, 9] 101", "no-op rebuild");
    let after: Vec<_> = std::fs::read_dir(&cache)
        .expect("the cache should exist")
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    assert_eq!(
        before.len(),
        after.len(),
        "a rebuild that changed nothing did not reuse its objects"
    );
}

/// An incremental `--native` rebuild recompiles a module whose struct changed
/// layout because of an edit somewhere else.
///
/// Whether a struct is a value struct is decided over the whole program:
/// `main` capturing it in a closure keeps it boxed, `main` only reading a
/// field lets it be a value. The helper's object was keyed on the helper's
/// source and the program's signatures, neither of which moves when only
/// `main`'s body does, so the second build linked the helper compiled for a
/// box against a `main` reading a value -- and the binary segfaulted until
/// the cache was deleted. Found through a library whose demo read a table
/// struct after an earlier build had passed it to a closure.
#[test]
fn an_incremental_rebuild_sees_a_changed_struct_layout() {
    let w = Work::new("inclayout");
    let dir = w.0.join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    let helper = dir.join("helper.gr");
    let main = dir.join("main.gr");
    let out = dir.join("app");
    std::fs::write(
        &helper,
        "type TabH = { xs: [str], ys: [str] }\n\npub fn make() -> TabH {\n    return TabH{ xs: [\"a\", \"b\", \"c\"], ys: [\"d\"] }\n}\n\n\
         pub fn has(t: TabH, s: str) -> bool {\n    return t.xs.contains(s)\n}\n",
    )
    .unwrap();

    let build_and_run = |expect: &str, what: &str| {
        let (code, msg) = run(&[
            "build",
            main.to_str().unwrap(),
            "--native",
            "-o",
            out.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{what}: {msg}");
        let got = Command::new(&out).output().expect("runs");
        assert!(
            got.status.success(),
            "{what}: the binary failed with {:?}",
            got.status
        );
        assert_eq!(
            String::from_utf8_lossy(&got.stdout).trim(),
            expect,
            "{what}"
        );
    };

    // Captured in a closure: the struct stays boxed.
    std::fs::write(
        &main,
        "import helper\n\nfn main() {\n    let t = helper.make()\n    print([\"a\", \"z\"].map(|s| if helper.has(t, s) { \"y\" } else { \"n\" }).join(\",\"))\n}\n",
    )
    .unwrap();
    build_and_run("y,n", "boxed");

    // Only read: now a value struct. The helper's source has not changed.
    std::fs::write(&main, "import helper\n\nfn main() {\n    let t = helper.make()\n    print(\"{t.xs.len()} {t.ys.len()}\")\n}\n").unwrap();
    build_and_run("3 1", "after main changed the layout");
}

/// `grasm test --contracts` fuzzes each function's `ensures` with inputs its
/// `requires` accepts, and reports a counterexample when one is broken
/// ([T-CONTRACT-ENSURES]; MASTERPLAN §9.5's third level).
///
/// The wrong function is wrong only for some inputs -- perfect squares above
/// 100 -- so the run has to find one rather than trip over it. The generic one
/// cannot be fuzzed, and says so instead of being skipped in silence. Without
/// the flag none of this runs.
#[test]
fn contracts_are_fuzzed_on_request() {
    let w = Work::new("contracts");
    let src = w.0.join("c.gr");
    std::fs::write(
        &src,
        "fn isqrt(n: i64) -> i64\n    requires n >= 0 and n < 1000000\n    ensures |r| r * r <= n and (r + 1) * (r + 1) > n\n{\n    var r = 0\n    while (r + 1) * (r + 1) <= n {\n        r += 1\n    }\n    r\n}\n\nfn sloppy(n: i64) -> i64\n    requires n >= 0 and n < 10000\n    ensures |r| r * r <= n and (r + 1) * (r + 1) > n\n{\n    var r = 0\n    while (r + 1) * (r + 1) < n or ((r + 1) * (r + 1) == n and n <= 100) {\n        r += 1\n    }\n    r\n}\n\nfn first[T](xs: [T]) -> T\n    requires xs.len() > 0\n    ensures |f| f == xs[0]\n{\n    xs[0]\n}\n\nfn main() {\n    print(\"{isqrt(10)}\")\n}\n",
    )
    .unwrap();
    let p = src.to_str().unwrap();

    let (code, out) = run(&["test", p, "--contracts", "--cases", "300"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("test \"contracts of isqrt\" ... ok (300 cases"),
        "{out}"
    );
    assert!(
        out.contains("test \"contracts of sloppy\" ... FAILED"),
        "{out}"
    );
    assert!(
        out.contains("ensures `|r| r * r <= n and (r + 1) * (r + 1) > n` failed in `sloppy`"),
        "{out}"
    );
    assert!(out.contains("counterexample: n = "), "{out}");
    assert!(out.contains("contracts of first\" ... not fuzzed"), "{out}");

    let (code, out) = run(&["test", p]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("contracts of"), "{out}");
}

/// A literal shift count outside `0..63` is masked, not refused -- and the
/// lint is the only thing that says so.
///
/// `[L-LIB-BITS]` makes the masking deliberate, so the program must keep
/// running and printing what the clause promises. What this checks is that
/// the three shapes a reader actually writes are all seen: a count past the
/// width, a negative count (which parses as a negation of a literal, not as a
/// negative literal, and was missed the first time), and a count in range,
/// which must stay silent.
#[test]
fn a_literal_shift_count_outside_the_width_is_linted_but_still_runs() {
    let work = Work::new("shiftcount");
    let dir = work.package(
        "shifty",
        "[package]\nname = \"shifty\"\nversion = \"1.0.0\"\n",
        "fn main() {\n    \
         print(\"{1.shl(64)}\")\n    \
         print(\"{256.shr(-1)}\")\n    \
         print(\"{1.shl(63)}\")\n}\n",
    );
    let src = dir.join("src/main.gr");

    let lint = Command::new(GRASM)
        .args(["lint", src.to_str().unwrap(), "--only", "shift-count"])
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&lint.stdout);
    assert!(
        out.contains("shift count 64 is masked to 0"),
        "a count past the width was not reported:\n{out}"
    );
    assert!(
        out.contains("shift count -1 is masked to 63"),
        "a negated literal was not reported:\n{out}"
    );
    assert_eq!(
        out.matches("shift-count").count(),
        2,
        "63 is in range and must not be reported:\n{out}"
    );

    // The clause promises these values. Linting them must not change them.
    let run = Command::new(GRASM)
        .args(["run", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "1\n0\n-9223372036854775808\n",
        "masking is specified behaviour and must be untouched"
    );
}

/// Writing into a `bytes` ([L-LIB-BYTES-WRITE]), and the value semantics that
/// make it safe to.
///
/// The window case is the one worth a test. In the compiled runtime `slice`
/// does not copy -- it takes a window on the same allocation -- so a write
/// that did not copy first would be visible through a slice taken before it,
/// which is the one thing `bytes` being immutable used to guarantee for free.
#[test]
fn writing_a_bytes_buffer_does_not_reach_a_slice_taken_before_the_write() {
    let work = Work::new("byteswrite");
    let dir = work.package(
        "painter",
        "[package]\nname = \"painter\"\nversion = \"1.0.0\"\n",
        "fn main() {\n    \
         var b = bytes_filled(4, 9)\n    \
         let win = b.slice(0, 2)\n    \
         b.set(0, 1)\n    \
         b.fill(7)\n    \
         b.copy_from(0, bytes_of([3, 4]))\n    \
         print(\"{b.to_list()} {win.to_list()} {b.len()}\")\n}\n",
    );
    let src = dir.join("src/main.gr");

    let run = Command::new(GRASM)
        .args(["run", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "[3, 4, 7, 7] [9, 9] 4\n",
        "a write reached a value that was taken before it"
    );

    // And the compiled engine must answer the same, because it is the one
    // whose `slice` shares the allocation.
    let exe = dir.join("painter.exe");
    let built = Command::new(GRASM)
        .args([
            "build",
            src.to_str().unwrap(),
            "--native",
            "-o",
            exe.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "the native build failed:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let native = Command::new(&exe).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&native.stdout),
        "[3, 4, 7, 7] [9, 9] 4\n",
        "the two engines disagree about a write through a shared buffer"
    );
}

/// A buffer larger than this implementation will allocate is refused with a
/// sentence, by both engines, rather than by the allocator.
///
/// The compiled runtime's panic handler cannot format a message -- formatting
/// allocates, and the panic may *be* the allocator failing -- so an allocator
/// abort there prints "(message needs formatting)" while the interpreter
/// prints Rust's own text. Two engines, two answers, for a program that only
/// asked for too much.
#[test]
fn a_bytes_buffer_too_large_to_allocate_is_refused_the_same_way_by_both_engines() {
    let work = Work::new("bytesbig");
    let dir = work.package(
        "painter",
        "[package]\nname = \"painter\"\nversion = \"1.0.0\"\n",
        "fn main() {\n    \
         let b = bytes_filled(4611686018427387904, 0)\n    \
         print(\"{b.len()}\")\n}\n",
    );
    let src = dir.join("src/main.gr");
    let want = "bytes_filled would need more memory than a buffer can hold";

    let run = Command::new(GRASM)
        .args(["run", src.to_str().unwrap()])
        .output()
        .unwrap();
    let interp = String::from_utf8_lossy(&run.stderr).to_string();
    assert!(
        interp.contains(want),
        "the interpreter let the allocator answer:\n{interp}"
    );

    let exe = dir.join("painter.exe");
    assert!(
        Command::new(GRASM)
            .args([
                "build",
                src.to_str().unwrap(),
                "--native",
                "-o",
                exe.to_str().unwrap(),
            ])
            .output()
            .unwrap()
            .status
            .success(),
        "the native build failed"
    );
    let native = Command::new(&exe).output().unwrap();
    let text = String::from_utf8_lossy(&native.stderr).to_string();
    assert!(
        text.contains(want),
        "the compiled engine let the allocator answer:\n{text}"
    );
}

/// A write to an immutable binding is refused, as every other in-place write
/// is.
#[test]
fn writing_a_bytes_buffer_needs_a_mutable_binding() {
    let work = Work::new("byteslet");
    let dir = work.package(
        "painter",
        "[package]\nname = \"painter\"\nversion = \"1.0.0\"\n",
        "fn main() {\n    \
         let b = bytes_filled(2, 0)\n    \
         b.set(0, 1)\n}\n",
    );
    let out = Command::new(GRASM)
        .args(["check", dir.join("src/main.gr").to_str().unwrap()])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("cannot mutate `b`") && text.contains("declare it with `var b`"),
        "writing through a `let` was not refused with the usual message:\n{text}"
    );
}
