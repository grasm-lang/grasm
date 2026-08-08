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
