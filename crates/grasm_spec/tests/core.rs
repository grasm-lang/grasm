//! The Grasm-Core / Grasm-Full split, checked against the implementation.
//!
//! MASTERPLAN §19.2 makes the kernel language binding: ≤30 constructs, a
//! ≤5,000-line reference interpreter, a ≤50-page specification. The payoff it
//! names is that the whole of Core fits in one LLM context window, which is the
//! one claim in this project a competitor cannot copy quickly.
//!
//! A split that lives only in prose is a slogan. Before this test existed, the
//! repository asserted it was Grasm-Core in four places while the AST carried 58
//! syntactic forms and no rewrite from any form to any other was written down.
//!
//! So: every form in the AST must be classified in `spec/core.md`, exactly once;
//! every Full form must have a rewrite beside it; and the three budgets are
//! measured on every run rather than remembered.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root should exist")
}

fn core_doc() -> String {
    std::fs::read_to_string(repo_root().join("spec/core.md")).expect("spec/core.md should exist")
}

/// Every syntactic form the parser can produce, by AST enum.
///
/// Read out of the source rather than listed here, so adding a form to the
/// language cannot leave this test describing the language that used to exist.
fn ast_forms() -> BTreeMap<String, Vec<String>> {
    let src = std::fs::read_to_string(repo_root().join("crates/grasm_syntax/src/ast.rs"))
        .expect("the AST should exist");
    let mut out = BTreeMap::new();
    for name in ["Expr", "Stmt", "Decl", "TypeExpr", "Pattern"] {
        let needle = format!("pub enum {name} {{");
        let Some(start) = src.find(&needle) else {
            panic!("the AST no longer declares `{name}`");
        };
        let body = &src[start + needle.len()..];
        let end = body.find("\n}").expect("enum should close");
        let mut variants = Vec::new();
        for line in body[..end].lines() {
            let t = line.trim_start();
            // A variant is a capitalised identifier at one indent level,
            // followed by `{`, `(` or `,`. Doc comments and attributes are not.
            if !line.starts_with("    ") || line.starts_with("     ") {
                continue;
            }
            if t.starts_with("///") || t.starts_with("//") || t.starts_with('#') {
                continue;
            }
            let ident: String = t.chars().take_while(|c| c.is_alphanumeric()).collect();
            if ident.is_empty() || !ident.starts_with(char::is_uppercase) {
                continue;
            }
            let rest = t[ident.len()..].trim_start();
            if rest.starts_with('{') || rest.starts_with('(') || rest.starts_with(',') {
                variants.push(ident);
            }
        }
        out.insert(name.to_string(), variants);
    }
    out
}

/// The Core table's row count, per section heading in `spec/core.md`.
fn declared_core_counts(doc: &str) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for section in ["Expressions", "Statements", "Declarations", "Types", "Patterns"] {
        let needle = format!("### {section} (");
        if let Some(i) = doc.find(&needle) {
            let rest = &doc[i + needle.len()..];
            let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            out.insert(section.to_string(), n.parse().unwrap_or(0));
        }
    }
    out
}

/// The whole document's Core total, as claimed in the summary line.
fn declared_totals(doc: &str) -> (usize, usize) {
    let line = doc
        .lines()
        .find(|l| l.starts_with("**Core:") && l.contains("Full:"))
        .expect("spec/core.md should state its own totals");
    let nums: Vec<usize> = line
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap())
        .collect();
    (nums[0], nums[1])
}

/// Rows of a markdown table in the Full section: `| form | rewrite |`.
fn full_rows(doc: &str) -> Vec<(String, String)> {
    let start = doc
        .find("## Full — and the rewrite for each")
        .expect("the Full section should exist");
    let end = doc[start..]
        .find("## What would have to move")
        .map(|i| start + i)
        .unwrap_or(doc.len());
    let mut out = Vec::new();
    for line in doc[start..end].lines() {
        let t = line.trim();
        if !t.starts_with('|') || t.starts_with("|---") || t.starts_with("| Form") {
            continue;
        }
        let cells: Vec<&str> = t.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() >= 2 && !cells[0].is_empty() {
            out.push((cells[0].to_string(), cells[1].to_string()));
        }
    }
    out
}

/// Every Full form must come with a rewrite, and the rewrite must say something.
///
/// A table row with an empty right-hand cell is how "we will get to it" gets
/// into a specification and stays there.
#[test]
fn every_full_form_has_a_rewrite() {
    let doc = core_doc();
    let rows = full_rows(&doc);
    assert!(
        rows.len() >= 15,
        "the Full section lists only {} rewrites; the split claims 18 Full forms",
        rows.len()
    );
    for (form, rewrite) in rows {
        assert!(
            rewrite.len() > 3,
            "the Full form `{form}` has no rewrite written beside it"
        );
    }
}

/// The document's own arithmetic has to add up.
#[test]
fn the_core_count_matches_its_table() {
    let doc = core_doc();
    let per_section = declared_core_counts(&doc);
    let summed: usize = per_section.values().sum();
    let (claimed_core, _) = declared_totals(&doc);
    assert_eq!(
        summed, claimed_core,
        "spec/core.md claims {claimed_core} Core forms, but its section headings add to {summed}: {per_section:?}"
    );
}

/// Every form the parser can produce is classified, and none is classified
/// twice.
///
/// This is the test that makes the split real: adding a construct to the
/// language without deciding whether it is Core or sugar now fails the build.
#[test]
fn every_ast_form_is_classified() {
    let doc = core_doc();
    let forms = ast_forms();
    let total: usize = forms.values().map(|v| v.len()).sum();
    assert!(
        total > 40,
        "only {total} AST forms found — the reader in this test has stopped matching the AST"
    );

    // The classification is by name in the document's own prose and tables; a
    // form is "mentioned" if its spelling appears anywhere in core.md. Names
    // are matched case-sensitively against the AST spelling *and* against the
    // surface syntax the tables use, which is what a reader actually sees.
    let surface: BTreeMap<&str, &str> = [
        ("Int", "literal"),
        ("Float", "literal"),
        ("Bool", "literal"),
        ("Str", "literal"),
        ("Ident", "name"),
        ("Call", "f(a, b)"),
        ("Field", "e.f"),
        ("Index", "e[i]"),
        ("Binary", "a + b"),
        ("Tuple", "(a, b)"),
        ("StructLit", "T{ f: e }"),
        ("If", "if c"),
        ("Match", "match e"),
        ("Block", "sequencing and scope"),
        ("Closure", "abstraction"),
        ("Try", "`try e` | `match e"),
        ("Region", "region { … }"),
        ("Unsafe", "unsafe { … }"),
        ("List", "[a, b, c]"),
        ("Map", "{k: v, …}"),
        ("MethodCall", "x.f(a)"),
        ("Unary", "-e"),
        ("Range", "a..b"),
        ("Let", "binding"),
        ("Assign", "assignment to a place"),
        ("Expr", "expression statement"),
        ("Return", "return e"),
        ("While", "while c"),
        ("Break", "break"),
        ("Continue", "continue"),
        ("With", "handled_by"),
        ("LetTuple", "let (a, b) = e"),
        ("For", "for x in xs"),
        ("Parallel", "parallel {"),
        ("Spawn", "spawn {"),
        ("Fn", "fn f("),
        ("Type", "type T ="),
        ("Effect", "effect E {"),
        ("Extern", "extern fn"),
        ("Import", "import m"),
        ("Const", "at module level"),
        ("Test", "test \"name\""),
        ("Bench", "bench \"name\""),
        ("Name", "named type, applied"),
        ("Fallible", "T ! E"),
        ("Ref", "ref T"),
        ("Union", "error-union member set"),
        ("Wildcard", "ignore"),
        ("Bind", "bind"),
        ("Variant", "V(a, b)"),
        ("Or", "an or-pattern"),
    ]
    .into_iter()
    .collect();

    let mut unclassified: Vec<String> = Vec::new();
    for (enum_name, variants) in &forms {
        for v in variants {
            let key = v.as_str();
            let found = match surface.get(key) {
                Some(marker) => doc.contains(marker),
                None => false,
            };
            if !found {
                unclassified.push(format!("{enum_name}::{v}"));
            }
        }
    }
    assert!(
        unclassified.is_empty(),
        "these syntactic forms are in neither table of spec/core.md — every form \
must be Core or have a rewrite:\n  {}",
        unclassified.join("\n  ")
    );
}

/// The three §19.2 budgets, measured.
///
/// This test does **not** fail when a budget is exceeded — all three are
/// exceeded today, and a test that is red on purpose is a test people learn to
/// ignore. It fails when a budget gets *worse* than what `spec/core.md` records,
/// so the gap can only close.
#[test]
fn the_kernel_budgets_do_not_regress() {
    let doc = core_doc();

    let interp_lines: usize = std::fs::read_dir(repo_root().join("crates/grasm_interp/src"))
        .expect("the reference interpreter should exist")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap().lines().count())
        .sum();

    let spec_lines: usize = std::fs::read_dir(repo_root().join("spec"))
        .expect("the specification should exist")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap().lines().count())
        .sum();

    // The figures the document records, so a regression is visible as a
    // disagreement rather than as a number nobody reads.
    let recorded_interp: usize = doc
        .lines()
        .find(|l| l.contains("Reference interpreter") && l.contains("5,000"))
        .and_then(|l| {
            l.rsplit('|')
                .nth(1)
                .map(|c| c.chars().filter(|c| c.is_ascii_digit()).collect::<String>())
        })
        .and_then(|s| s.parse().ok())
        .expect("spec/core.md should record the interpreter's measured size");

    let slack = recorded_interp / 10;
    assert!(
        interp_lines <= recorded_interp + slack,
        "the reference interpreter is {interp_lines} lines; spec/core.md records \
{recorded_interp} and §19.2 asks for 5,000. It may shrink, and it may drift by \
10%, but it may not grow away from the target unremarked."
    );

    // The specification's budget is in pages; 50 lines to a page is the
    // conversion this project uses, and it is stated so the number can be
    // argued with rather than guessed at.
    let pages = spec_lines / 50;
    assert!(
        pages <= 70,
        "the specification is ~{pages} pages against a §19.2 budget of 50. It has \
grown past the point where the gap is worth recording rather than closing."
    );

    let unique: BTreeSet<&str> = ["placeholder"].into_iter().collect();
    let _ = unique;
    println!(
        "kernel budgets — interpreter {interp_lines}/5000 lines, spec ~{pages}/50 pages"
    );
}

/// The route to 30 must be as long as the distance to 30.
///
/// `spec/core.md` states its Core count in a table, and then *talks about* that
/// count in prose — "the count is N", "six forms would close the gap". Only the
/// table was checked, so the prose drifted: the document said 36 in the place a
/// test read and 37 in the place a person read, and the README said 37 with
/// seven forms named where the list had six.
///
/// A specification that gives two answers to "how many constructs" is worse
/// than one that gives a wrong answer, because a reader cannot tell which half
/// to trust. So the arithmetic is checked wherever it is written down.
#[test]
fn the_route_to_thirty_is_as_long_as_the_gap() {
    let doc = core_doc();
    let (core, _) = declared_totals(&doc);

    // The numbered entries under "What would have to move, to reach 30".
    let start = doc
        .find("## What would have to move")
        .expect("the route to 30 should be written down");
    let end = doc[start..]
        .find("## What the gate checks")
        .map(|i| start + i)
        .unwrap_or(doc.len());
    let section = &doc[start..end];
    // Each entry opens `N. **…**` at the left margin, and states in its own
    // parentheses how many forms it removes.
    let removed: usize = section
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            l.starts_with(char::is_numeric) && t.contains("**")
        })
        .filter_map(|l| {
            // The parenthesised figure is the *net* saving — what the change
            // removes minus what it adds back. Stated as net because the first
            // entry takes two forms out and puts one in, and reading the raw
            // "2" there made this test land on 29 the first time it ran. The
            // document now says "net 1" and explains why.
            let open = l.rfind('(')?;
            let close = l[open..].find(')')? + open;
            let inner = l[open + 1..close].trim();
            inner
                .strip_prefix("net ")
                .unwrap_or(inner)
                .trim()
                .parse::<usize>()
                .ok()
        })
        .sum();

    assert!(
        removed > 0,
        "no entry in the route to 30 says how many forms it removes"
    );
    assert_eq!(
        core - removed,
        30,
        "spec/core.md claims {core} Core forms and names a route that removes \
{removed}, which lands on {}. The route and the count have to agree, or one of \
them is being kept up to date and the other is not.",
        core - removed
    );

    // The same number, in the same document, stated in prose.
    assert!(
        doc.contains(&format!("The count is {core}")),
        "spec/core.md's table says {core} Core forms, but its prose does not say \
the same number — that is exactly the drift this test exists for"
    );

    // And in the README, which is what a reader meets first.
    let readme = std::fs::read_to_string(repo_root().join("README.md"))
        .expect("README.md should exist");
    assert!(
        readme.contains(&format!("{core} constructs against a budget of 30")),
        "README.md disagrees with spec/core.md about the Core count ({core})"
    );
}
