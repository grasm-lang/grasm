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
        let dir = std::env::temp_dir().join(format!(
            "grasm_cli_test_{}_{n}_{tag}",
            std::process::id()
        ));
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

const OVER_CEILING: &str = "[package]\nname = \"quietlib\"\nversion = \"0.1.0\"\ncapabilities = []\n";
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
    std::fs::write(lib.join("src/sub/helper.gr"), "pub fn g() -> i64 {\n    7\n}\n").unwrap();
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
    std::fs::write(w.0.join("helper.gr"), "pub fn value() -> str {\n    \"from helper\"\n}\n").unwrap();
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
    run(&["build", entry.to_str().unwrap(), "--native", "-o", bin.to_str().unwrap()]);
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
    assert!(!surface.contains("helper"), "private item leaked:\n{surface}");
    let saved = w.0.join("v1.api");
    std::fs::write(&saved, &surface).unwrap();

    // Unchanged.
    let (code, text) = run(&["api", main.to_str().unwrap(), "--against", saved.to_str().unwrap()]);
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
    let (code, text) = run(&["api", main.to_str().unwrap(), "--against", saved.to_str().unwrap()]);
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
    let (code, text) = run(&["api", main.to_str().unwrap(), "--against", saved.to_str().unwrap()]);
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
    assert_eq!(code, 1, "a printing doc example is over the ceiling:\n{out}");
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

/// A package that installs a handler it did not construct is charged what that
/// handler does, in the program being built.
///
/// `[E-HANDLER-OWN-ROW]` charges "what *any* handler for that effect does,
/// anywhere in the program". Computed per package in isolation, a
/// `capabilities = []` library printed through a handler the consumer passed
/// it and every command reported it as performing nothing.
#[test]
fn a_handler_from_the_consumer_counts_against_the_library() {
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
    let (code, out) = run(&["check", entry.as_str()]);
    assert_eq!(code, 1, "the library is over its ceiling:\n{out}");
    assert!(out.contains("package `pure` performs `io`"), "{out}");
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
    assert_eq!(code, 1, "a typo in --native must not build a bundle:\n{out}");
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
        run(&["build", f.to_str().unwrap(), "--native", "-o", native.to_str().unwrap()]).0,
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
    assert!(!out.contains(".."), "paths are collapsed, not chained:\n{out}");

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
    assert!(!out.contains("/p10/p11/p12"), "concatenated path in:\n{out}");
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
        text.contains("cannot analyze package `app`: `../dep/src/main.gr` (reached through a dependency)"),
        "{text}"
    );
    assert!(
        !text.contains("cannot analyze package `app`: `src/main.gr`"),
        "the consumer's own entry is not the failing file:\n{text}"
    );
}
