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
