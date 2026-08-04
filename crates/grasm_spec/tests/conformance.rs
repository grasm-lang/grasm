//! Specification conformance suite.
//!
//! Two obligations are enforced here, and the specification cannot drift from
//! the implementation without one of them failing:
//!
//! 1. Every program in `spec/conformance/` behaves exactly as its `//! expect:`
//!    header declares.
//! 2. Every normative tag in `spec/*.md` has a conformance program, and every
//!    conformance program names a tag that actually exists in the documents.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn spec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec")
        .canonicalize()
        .expect("spec/ directory should exist")
}

#[derive(Debug, PartialEq)]
enum Expect {
    /// Program compiles, runs, and prints exactly these lines (exit 0).
    Output(Vec<String>),
    /// Program compiles, runs, prints these lines, and exits non-zero.
    OutputFailing(Vec<String>),
    /// Program fails to compile; diagnostics contain these substrings.
    Error(Vec<String>),
    /// Program compiles then panics; the panic contains these substrings.
    Panic(Vec<String>),
    /// Program is run through the *test* runner (`grasm test`) rather than
    /// `main`; its report contains these substrings. Clauses about property
    /// testing and benchmarks are only observable this way.
    TestOutput(Vec<String>),
}

struct Case {
    name: String,
    tag: String,
    expect: Expect,
    source: String,
}

fn parse_case(path: &Path) -> Case {
    let source = std::fs::read_to_string(path).expect("readable conformance file");
    let name = path.file_name().unwrap().to_string_lossy().to_string();

    let mut tag = None;
    let mut kind = None;
    let mut lines: Vec<String> = Vec::new();

    for line in source.lines() {
        let Some(rest) = line.strip_prefix("//!") else {
            // Headers must be contiguous at the top of the file.
            if tag.is_some() {
                break;
            }
            continue;
        };
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        if let Some(t) = rest.strip_prefix("tag:") {
            tag = Some(t.trim().to_string());
        } else if let Some(k) = rest.strip_prefix("expect:") {
            kind = Some(k.trim().to_string());
        } else {
            lines.push(rest.to_string());
        }
    }

    let tag = tag.unwrap_or_else(|| panic!("{name}: missing `//! tag:` header"));
    let kind = kind.unwrap_or_else(|| panic!("{name}: missing `//! expect:` header"));
    let expect = match kind.as_str() {
        "output" => Expect::Output(lines),
        "output-failing" => Expect::OutputFailing(lines),
        "error" => Expect::Error(lines),
        "panic" => Expect::Panic(lines),
        "test-output" => Expect::TestOutput(lines),
        other => panic!("{name}: unknown expectation `{other}`"),
    };

    Case {
        name,
        tag,
        expect,
        source,
    }
}

fn all_cases() -> Vec<Case> {
    let dir = spec_dir().join("conformance");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("conformance/ directory should exist")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "gr"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no conformance programs found in {dir:?}");
    paths.iter().map(|p| parse_case(p)).collect()
}

/// Every tag mentioned in bold brackets in the normative documents.
fn declared_tags() -> BTreeSet<String> {
    let mut tags = BTreeSet::new();
    for doc in ["grammar.md", "types.md", "semantics.md"] {
        let text = std::fs::read_to_string(spec_dir().join(doc))
            .unwrap_or_else(|e| panic!("spec/{doc} should be readable: {e}"));
        // Tags are written as **[X-NAME]**.
        let mut rest = text.as_str();
        while let Some(start) = rest.find("**[") {
            rest = &rest[start + 3..];
            if let Some(end) = rest.find("]**") {
                let tag = &rest[..end];
                if tag
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
                {
                    tags.insert(tag.to_string());
                }
                rest = &rest[end + 3..];
            }
        }
    }
    assert!(!tags.is_empty(), "no normative tags found in spec/*.md");
    tags
}

/// Runs a program's `test` declarations with a fixed seed and case count, so
/// the report is reproducible from run to run.
fn run_test_suite(source: &str) -> (u32, u32, String, Vec<String>) {
    let (module, diags) = grasm_sema::analyze(source);
    if !diags.is_empty() {
        let messages = diags.iter().map(|d| d.message.clone()).collect();
        return (0, 0, String::new(), messages);
    }
    let mut out = Vec::new();
    let mut interp = grasm_interp::Interp::new(&module, &mut out);
    let mut passed = 0;
    let mut failed = 0;
    let opts = grasm_interp::testing::TestOptions {
        seed: 1234,
        cases: 500,
        ..Default::default()
    };
    for decl in &module.decls {
        let grasm_syntax::ast::Decl::Test(t) = decl else {
            continue;
        };
        if t.params.is_empty() {
            if interp.run_unit_test(t) {
                passed += 1;
            } else {
                failed += 1;
            }
            continue;
        }
        match interp.run_property(&t.params, &t.body, &opts) {
            grasm_interp::testing::PropertyOutcome::Passed(n) => {
                interp.report(&format!("test \"{}\" ... ok ({n} cases)", t.name));
                passed += 1;
            }
            grasm_interp::testing::PropertyOutcome::Failed { inputs, .. } => {
                interp.report(&format!(
                    "test \"{}\" ... FAILED counterexample: {}",
                    t.name,
                    grasm_interp::testing::render_inputs(&inputs)
                ));
                failed += 1;
            }
        }
    }
    (
        passed,
        failed,
        String::from_utf8_lossy(&out).into_owned(),
        Vec::new(),
    )
}

fn run_program(source: &str) -> (i32, String, Vec<String>) {
    let mut out = Vec::new();
    let (code, diags) = grasm_interp::run_src(source, &mut out);
    let messages = diags
        .iter()
        .map(|d| {
            let mut m = d.message.clone();
            if let Some(h) = &d.help {
                m.push_str(" | help: ");
                m.push_str(h);
            }
            m
        })
        .collect();
    (code, String::from_utf8_lossy(&out).into_owned(), messages)
}

#[test]
fn every_conformance_program_behaves_as_specified() {
    let mut failures: Vec<String> = Vec::new();

    for case in all_cases() {
        if let Expect::TestOutput(expected) = &case.expect {
            let (_, _, report, diags) = run_test_suite(&case.source);
            if !diags.is_empty() {
                failures.push(format!(
                    "{}: expected it to compile, but got diagnostics: {diags:?}",
                    case.name
                ));
                continue;
            }
            for needle in expected {
                if !report.contains(needle.trim()) {
                    failures.push(format!(
                        "{}: test report missing {:?}\n  actual: {report}",
                        case.name,
                        needle.trim()
                    ));
                }
            }
            continue;
        }
        let (code, stdout, diags) = run_program(&case.source);
        match &case.expect {
            Expect::Output(expected) => {
                if !diags.is_empty() {
                    failures.push(format!(
                        "{}: expected it to compile, but got diagnostics: {diags:?}",
                        case.name
                    ));
                    continue;
                }
                let want = expected.join("\n");
                let got = stdout.trim_end_matches('\n');
                if got != want {
                    failures.push(format!(
                        "{}: output mismatch\n  expected: {want:?}\n  actual:   {got:?}",
                        case.name
                    ));
                }
                if code != 0 {
                    failures.push(format!("{}: expected exit 0, got {code}", case.name));
                }
            }
            Expect::OutputFailing(expected) => {
                if !diags.is_empty() {
                    failures.push(format!(
                        "{}: expected it to compile, but got diagnostics: {diags:?}",
                        case.name
                    ));
                    continue;
                }
                let want = expected.join("\n");
                let got = stdout.trim_end_matches('\n');
                if got != want {
                    failures.push(format!(
                        "{}: output mismatch\n  expected: {want:?}\n  actual:   {got:?}",
                        case.name
                    ));
                }
                if code == 0 {
                    failures.push(format!(
                        "{}: expected a non-zero exit, got 0",
                        case.name
                    ));
                }
            }
            Expect::Error(needles) => {
                if diags.is_empty() {
                    failures.push(format!(
                        "{}: expected a compile error, but it compiled cleanly",
                        case.name
                    ));
                    continue;
                }
                let joined = diags.join("\n");
                for needle in needles {
                    if !joined.contains(needle.as_str()) {
                        failures.push(format!(
                            "{}: diagnostics missing {needle:?}\n  actual: {joined}",
                            case.name
                        ));
                    }
                }
            }
            Expect::TestOutput(_) => unreachable!("handled before this match"),
            Expect::Panic(needles) => {
                if !diags.is_empty() {
                    failures.push(format!(
                        "{}: expected a runtime panic, but it failed to compile: {diags:?}",
                        case.name
                    ));
                    continue;
                }
                if code != 101 {
                    failures.push(format!(
                        "{}: expected exit 101 (panic), got {code}; output: {stdout:?}",
                        case.name
                    ));
                }
                for needle in needles {
                    if !stdout.contains(needle.as_str()) {
                        failures.push(format!(
                            "{}: panic message missing {needle:?}\n  actual: {stdout:?}",
                            case.name
                        ));
                    }
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} conformance failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn every_normative_clause_has_a_conformance_program() {
    let declared = declared_tags();
    let covered: BTreeSet<String> = all_cases().into_iter().map(|c| c.tag).collect();

    let uncovered: Vec<&String> = declared.difference(&covered).collect();
    assert!(
        uncovered.is_empty(),
        "{} normative clause(s) have no conformance program — either write one \
         in spec/conformance/<TAG>.gr or remove the clause:\n  {}",
        uncovered.len(),
        uncovered
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

#[test]
fn every_conformance_program_names_a_real_clause() {
    let declared = declared_tags();
    let mut orphans: Vec<String> = Vec::new();
    for case in all_cases() {
        if !declared.contains(&case.tag) {
            orphans.push(format!("{} claims tag `{}`", case.name, case.tag));
        }
    }
    assert!(
        orphans.is_empty(),
        "conformance programs reference tags that no longer exist in spec/*.md:\n  {}",
        orphans.join("\n  ")
    );
}

#[test]
fn conformance_file_names_match_their_tags() {
    let mut mismatched = Vec::new();
    for case in all_cases() {
        let stem = case.name.trim_end_matches(".gr");
        if stem != case.tag {
            mismatched.push(format!("{} declares tag `{}`", case.name, case.tag));
        }
    }
    assert!(
        mismatched.is_empty(),
        "file names must match their tag so coverage is greppable:\n  {}",
        mismatched.join("\n  ")
    );
}

/// Fenced code blocks in `docs/tutorial.md` that look like whole programs.
///
/// A tutorial whose code no longer compiles is worse than no tutorial, and
/// prose is exactly where drift hides, so the suite compiles it.
fn tutorial_programs() -> Vec<(usize, String)> {
    let path = spec_dir().parent().unwrap().join("docs/tutorial.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} should be readable: {e}", path.display()));

    let mut out = Vec::new();
    // (start line, fence width, lines, is_grasm). Width matters: a ```` block
    // wraps an example that itself contains ``` fences, and only a fence at
    // least as wide as the opener closes it.
    let mut current: Option<(usize, usize, Vec<String>, bool)> = None;
    for (i, line) in text.lines().enumerate() {
        let width = line.chars().take_while(|c| *c == '`').count();
        let is_fence = width >= 3;
        match (&mut current, is_fence) {
            (Some((start, open_width, lines, is_grasm)), true) if width >= *open_width => {
                if *is_grasm {
                    out.push((*start, lines.join("\n")));
                }
                current = None;
            }
            (None, true) => {
                let info = line[width..].trim();
                current = Some((
                    i + 2,
                    width,
                    Vec::new(),
                    info.is_empty() || info == "grasm",
                ));
            }
            (Some((_, _, lines, _)), _) => lines.push(line.to_string()),
            (None, false) => {}
        }
    }
    // Blocks with `import` need sibling files a single-source run cannot
    // provide, and a TOML manifest is not Grasm at all. Everything else with a
    // declaration in it should compile; a block of only `test`/`bench` gets an
    // empty `main` so it is a complete program.
    out.retain(|(_, src)| !src.contains("import ") && !src.contains("[package]"));
    out.retain(|(_, src)| {
        src.contains("fn ") || src.contains("test \"") || src.contains("bench \"")
    });
    for (_, src) in &mut out {
        if !src.contains("fn main(") {
            src.push_str("\n\nfn main() {\n}\n");
        }
    }
    out
}

#[test]
fn every_tutorial_program_compiles_and_runs() {
    let programs = tutorial_programs();
    assert!(
        programs.len() >= 12,
        "expected the tutorial to contain runnable programs, found {}",
        programs.len()
    );

    let mut failures = Vec::new();
    for (line, src) in programs {
        // A block declaring tests is checked through the test runner so its
        // `test` declarations are actually executed, not merely parsed.
        if src.contains("test \"") || src.contains("bench \"") {
            let (_, failed, _, diags) = run_test_suite(&src);
            if !diags.is_empty() {
                failures.push(format!(
                    "docs/tutorial.md:{line}: does not compile: {diags:?}\n{src}"
                ));
            } else if failed > 0 {
                failures.push(format!("docs/tutorial.md:{line}: {failed} test(s) failed\n{src}"));
            }
            continue;
        }
        let (code, _stdout, diags) = run_program(&src);
        if !diags.is_empty() {
            failures.push(format!(
                "docs/tutorial.md:{line}: does not compile: {diags:?}\n{src}"
            ));
            continue;
        }
        if code != 0 {
            failures.push(format!("docs/tutorial.md:{line}: exited {code}\n{src}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} tutorial program(s) are broken:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Every clause's stability level, from `spec/stability.md`.
fn stability_levels() -> std::collections::BTreeMap<String, String> {
    let path = spec_dir().join("stability.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} should be readable: {e}", path.display()));
    let mut out = std::collections::BTreeMap::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("- `") else {
            continue;
        };
        let Some((tag, tail)) = rest.split_once('`') else {
            panic!("malformed stability entry: {line}");
        };
        let level = tail.trim_start_matches(" — ").trim().to_string();
        if let Some(previous) = out.insert(tag.to_string(), level) {
            panic!("`{tag}` is listed twice in spec/stability.md (first as {previous})");
        }
    }
    out
}

#[test]
fn every_clause_has_a_stability_level() {
    // A clause with no level would make docs/stability.md a claim about
    // nothing in particular; an entry with no clause would be a promise about
    // a rule that no longer exists.
    let declared = declared_tags();
    let levels = stability_levels();
    let known = ["stable", "provisional", "experimental"];

    let missing: Vec<&String> = declared.iter().filter(|t| !levels.contains_key(*t)).collect();
    assert!(
        missing.is_empty(),
        "{} clause(s) have no stability level — add them to spec/stability.md:\n  {}",
        missing.len(),
        missing
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    let orphaned: Vec<&String> = levels.keys().filter(|t| !declared.contains(*t)).collect();
    assert!(
        orphaned.is_empty(),
        "{} stability entr(ies) name a clause that does not exist:\n  {}",
        orphaned.len(),
        orphaned
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    let bad: Vec<String> = levels
        .iter()
        .filter(|(_, v)| !known.contains(&v.as_str()))
        .map(|(k, v)| format!("{k} = {v:?}"))
        .collect();
    assert!(
        bad.is_empty(),
        "unknown stability level(s); expected one of {known:?}:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn the_stability_policy_and_the_table_agree_on_the_levels() {
    // The prose names three levels. If it ever names a fourth, or renames one,
    // the table would silently keep using the old vocabulary.
    let policy = std::fs::read_to_string(spec_dir().parent().unwrap().join("docs/stability.md"))
        .expect("docs/stability.md should be readable");
    for level in ["stable", "provisional", "experimental"] {
        assert!(
            policy.contains(&format!("**`{level}`**")),
            "docs/stability.md does not define the `{level}` level"
        );
    }
}
