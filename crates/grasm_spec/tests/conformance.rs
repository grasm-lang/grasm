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
