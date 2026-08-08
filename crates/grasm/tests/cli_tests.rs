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
