//! End-to-end tests for the test runner itself: unit tests, property tests
//! (including shrinking), benchmarks, and coverage.

use grasm_interp::testing::{PropertyOutcome, TestOptions};
use std::io::Write;

/// Loads a program from a temporary file so the multi-file loader — and with
/// it the span-offset space coverage depends on — is exercised for real.
struct Fixture {
    dir: std::path::PathBuf,
    entry: std::path::PathBuf,
}

impl Fixture {
    fn new(name: &str, src: &str) -> Fixture {
        let dir =
            std::env::temp_dir().join(format!("grasm_testing_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("main.gr");
        let mut f = std::fs::File::create(&entry).unwrap();
        f.write_all(src.as_bytes()).unwrap();
        Fixture { dir, entry }
    }

    fn program(&self) -> grasm_sema::loader::Program {
        let program = grasm_sema::loader::load_from_disk(&self.entry).expect("loads");
        // Parse errors live on the program; type errors come from the check.
        // Missing either lets a malformed fixture pass vacuously.
        assert!(
            program.diagnostics.is_empty(),
            "fixture failed to parse: {:#?}",
            program.diagnostics
        );
        let diags = grasm_sema::check::check_program(&program).diagnostics;
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:#?}");
        program
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run_tests(name: &str, src: &str, opts: &TestOptions) -> (u32, u32, String) {
    let fx = Fixture::new(name, src);
    let program = fx.program();
    let mut out = Vec::new();
    let summary = grasm_interp::test_program_with(&program, opts, &mut out);
    (
        summary.passed,
        summary.failed,
        String::from_utf8(out).unwrap(),
    )
}

#[test]
fn unit_tests_still_run_without_parameters() {
    let (passed, failed, out) = run_tests(
        "unit",
        "test \"arithmetic\" {\n    assert(2 + 2 == 4)\n}\n\ntest \"broken\" {\n    assert(1 == 2)\n}\n\nfn main() {\n}\n",
        &TestOptions::default(),
    );
    assert_eq!((passed, failed), (1, 1));
    assert!(out.contains("\"arithmetic\" ... ok"), "output: {out}");
    assert!(out.contains("\"broken\" ... FAILED"), "output: {out}");
}

#[test]
fn a_property_that_holds_reports_its_case_count() {
    let (passed, failed, out) = run_tests(
        "holds",
        "test \"addition commutes\" (a: i64, b: i64) {\n    assert(a + b == b + a)\n}\n\nfn main() {\n}\n",
        &TestOptions {
            cases: 25,
            ..Default::default()
        },
    );
    assert_eq!((passed, failed), (1, 0));
    assert!(out.contains("ok (25 cases)"), "output: {out}");
}

#[test]
fn a_false_property_fails_with_a_replayable_seed() {
    let opts = TestOptions {
        seed: 42,
        cases: 200,
        ..Default::default()
    };
    let src = "test \"every int is small\" (n: i64) {\n    assert(n < 5)\n}\n\nfn main() {\n}\n";
    let (passed, failed, out) = run_tests("false_prop", src, &opts);
    assert_eq!((passed, failed), (0, 1));
    assert!(out.contains("--seed 42"), "must report the seed: {out}");
    assert!(out.contains("counterexample: n ="), "output: {out}");

    // Same seed, same counterexample — a red property has to be reproducible.
    let (_, _, again) = run_tests("false_prop2", src, &opts);
    let line = |s: &str| {
        s.lines()
            .find(|l| l.contains("counterexample"))
            .unwrap()
            .to_string()
    };
    assert_eq!(line(&out), line(&again));
}

#[test]
fn shrinking_reports_the_minimal_counterexample() {
    // The smallest failing `n` is exactly 5; a run without shrinking would
    // report whatever large value the generator happened to produce.
    let (_, failed, out) = run_tests(
        "shrink_int",
        "test \"under five\" (n: i64) {\n    if n > 0 {\n        assert(n < 5)\n    }\n}\n\nfn main() {\n}\n",
        &TestOptions {
            cases: 300,
            ..Default::default()
        },
    );
    assert_eq!(failed, 1);
    assert!(
        out.contains("counterexample: n = 5"),
        "expected the minimal counterexample, got: {out}"
    );
}

#[test]
fn shrinking_minimizes_lists_and_strings() {
    let (_, failed, out) = run_tests(
        "shrink_list",
        "test \"no long lists\" (xs: [i64]) {\n    assert(xs.len() < 2)\n}\n\nfn main() {\n}\n",
        &TestOptions {
            cases: 300,
            ..Default::default()
        },
    );
    assert_eq!(failed, 1);
    // Two elements is the shortest failing list, and each shrinks to 0.
    assert!(
        out.contains("counterexample: xs = [0, 0]"),
        "expected a minimal list, got: {out}"
    );
}

#[test]
fn a_panicking_property_is_a_failure_not_a_crash() {
    let (passed, failed, out) = run_tests(
        "panic_prop",
        "test \"divides\" (n: i64) {\n    assert(10 / n != 999)\n}\n\nfn main() {\n}\n",
        &TestOptions {
            cases: 100,
            ..Default::default()
        },
    );
    assert_eq!((passed, failed), (0, 1));
    assert!(out.contains("division by zero"), "output: {out}");
    assert!(
        out.contains("counterexample: n = 0"),
        "the zero case is minimal: {out}"
    );
}

#[test]
fn the_filter_selects_tests_by_name() {
    let src = "test \"alpha\" {\n    assert(true)\n}\n\ntest \"beta\" {\n    assert(true)\n}\n\nfn main() {\n}\n";
    let (passed, _, out) = run_tests(
        "filter",
        src,
        &TestOptions {
            filter: "alph".to_string(),
            ..Default::default()
        },
    );
    assert_eq!(passed, 1);
    assert!(out.contains("alpha"), "output: {out}");
    assert!(!out.contains("beta"), "beta should be filtered out: {out}");
}

#[test]
fn coverage_reports_lines_the_tests_never_reached() {
    let src = "fn used() -> i64 {\n    1\n}\n\nfn unused() -> i64 {\n    2\n}\n\ntest \"only touches used\" {\n    assert(used() == 1)\n}\n\nfn main() {\n}\n";
    let fx = Fixture::new("coverage", src);
    let program = fx.program();
    let mut out = Vec::new();
    let summary = grasm_interp::test_program_with(
        &program,
        &TestOptions {
            coverage: true,
            ..Default::default()
        },
        &mut out,
    );
    let cov = summary.coverage.expect("coverage was requested");
    assert!(cov.percent() < 100.0, "unused() must show as uncovered");
    let misses = cov.misses();
    assert_eq!(misses.len(), 1, "one file: {misses:?}");
    assert!(
        misses[0].1.contains(&6),
        "line 6 is `unused`'s body: {misses:?}"
    );
    assert!(
        !misses[0].1.contains(&2),
        "line 2 is `used`'s body and did run: {misses:?}"
    );
}

#[test]
fn coverage_is_off_by_default() {
    let (_, _, _) = run_tests(
        "nocov",
        "test \"t\" {\n    assert(true)\n}\n\nfn main() {\n}\n",
        &TestOptions::default(),
    );
    let fx = Fixture::new(
        "nocov2",
        "test \"t\" {\n    assert(true)\n}\n\nfn main() {\n}\n",
    );
    let mut out = Vec::new();
    let summary = grasm_interp::test_program_with(&fx.program(), &TestOptions::default(), &mut out);
    assert!(summary.coverage.is_none());
}

#[test]
fn benchmarks_run_and_report_iterations() {
    let fx = Fixture::new(
        "bench",
        "bench \"trivial\" {\n    let x = 1 + 1\n    assert(x == 2)\n}\n\nfn main() {\n}\n",
    );
    let mut out = Vec::new();
    let results = grasm_interp::bench_program(&fx.program(), "", &mut out);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "trivial");
    assert!(results[0].error.is_none(), "{:?}", results[0].error);
    assert!(results[0].iterations > 0);
    assert!(
        results[0].ns_per_op > 0.0,
        "a measured body must take some time"
    );
}

#[test]
fn a_broken_benchmark_reports_instead_of_timing_noise() {
    let fx = Fixture::new(
        "bench_bad",
        "bench \"divides by zero\" {\n    let zero = 0\n    let x = 1 / zero\n    assert(x == 0)\n}\n\nfn main() {\n}\n",
    );
    let mut out = Vec::new();
    let results = grasm_interp::bench_program(&fx.program(), "", &mut out);
    assert_eq!(results.len(), 1);
    assert!(
        results[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("division by zero")),
        "expected the panic to be reported: {:?}",
        results[0].error
    );
    assert_eq!(results[0].iterations, 0, "a failed bench has no timing");
}

#[test]
fn generated_values_cover_the_declared_types() {
    // A weak generator would silently make properties vacuous, so check that
    // the interesting shapes actually appear across a run.
    use grasm_interp::testing::{generate, Rng};
    let (module, diags) = grasm_sema::analyze(
        "test \"t\" (n: i64, s: str, xs: [i64], b: bool, o: Option[i64]) {\n    assert(true)\n}\n\nfn main() {\n}\n",
    );
    assert!(diags.is_empty(), "{diags:#?}");
    let grasm_syntax::ast::Decl::Test(t) = &module.decls[0] else {
        panic!("expected the test declaration first");
    };

    let mut rng = Rng::new(7);
    let mut saw_negative = false;
    let mut saw_zero = false;
    let mut saw_nonempty_str = false;
    let mut saw_nonempty_list = false;
    let mut saw_both_bools = (false, false);
    let mut saw_none = false;
    let mut saw_some = false;
    for case in 0..200u32 {
        let size = 1 + case / 2;
        let vals: Vec<_> = t
            .params
            .iter()
            .map(|p| generate(&mut rng, &p.ty, size, None))
            .collect();
        match &vals[0] {
            grasm_interp::Value::Int(0) => saw_zero = true,
            grasm_interp::Value::Int(n) if *n < 0 => saw_negative = true,
            _ => {}
        }
        if let grasm_interp::Value::Str(s) = &vals[1] {
            saw_nonempty_str |= !s.is_empty();
        }
        if let grasm_interp::Value::List(xs) = &vals[2] {
            saw_nonempty_list |= !xs.is_empty();
        }
        if let grasm_interp::Value::Bool(b) = &vals[3] {
            if *b {
                saw_both_bools.0 = true;
            } else {
                saw_both_bools.1 = true;
            }
        }
        if let grasm_interp::Value::Variant { variant, .. } = &vals[4] {
            match variant.as_str() {
                "None" => saw_none = true,
                "Some" => saw_some = true,
                _ => {}
            }
        }
    }
    assert!(saw_zero, "generator never produced 0");
    assert!(saw_negative, "generator never produced a negative int");
    assert!(saw_nonempty_str, "generator never produced a non-empty str");
    assert!(
        saw_nonempty_list,
        "generator never produced a non-empty list"
    );
    assert!(
        saw_both_bools.0 && saw_both_bools.1,
        "bools were not varied"
    );
    assert!(saw_none && saw_some, "Option was not varied");
}

#[test]
fn a_property_over_two_arguments_shrinks_both() {
    let (_, failed, out) = run_tests(
        "two_args",
        "test \"never both\" (a: i64, b: i64) {\n    assert(not (a > 0 and b > 0))\n}\n\nfn main() {\n}\n",
        &TestOptions {
            cases: 300,
            ..Default::default()
        },
    );
    assert_eq!(failed, 1);
    assert!(
        out.contains("counterexample: a = 1, b = 1"),
        "both arguments should shrink to 1: {out}"
    );
}

#[test]
fn property_outcome_reports_the_case_that_failed() {
    let fx = Fixture::new(
        "case_no",
        "test \"p\" (n: i64) {\n    assert(n != n)\n}\n\nfn main() {\n}\n",
    );
    let program = fx.program();
    let files: Vec<_> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    let mut out = Vec::new();
    let mut interp = grasm_interp::Interp::new_program(&files, &mut out);
    let grasm_syntax::ast::Decl::Test(t) = &program.files[0].module.decls[0] else {
        panic!("expected a test");
    };
    match interp.run_property(&t.params, &t.body, &TestOptions::default()) {
        PropertyOutcome::Failed { case, .. } => {
            assert_eq!(case, 1, "an always-false property fails on the first case")
        }
        PropertyOutcome::Passed(_) => panic!("this property cannot hold"),
    }
}

// ---------------------------------------------------------------------------
// Stack traces
// ---------------------------------------------------------------------------

#[test]
fn a_panic_records_the_call_stack() {
    let fx = Fixture::new(
        "trace",
        "fn inner(xs: [i64]) -> i64 {\n    xs[10]\n}\n\nfn outer() -> i64 {\n    inner([1, 2])\n}\n\nfn main() {\n    print(\"{outer()}\")\n}\n",
    );
    let program = fx.program();
    let mut out = Vec::new();
    let (code, trace, _fault) = grasm_interp::run_program_traced(&program, &[], &mut out);
    assert_eq!(code, 101);
    let names: Vec<&str> = trace.iter().map(|f| f.function.as_str()).collect();
    assert_eq!(names, vec!["main::inner", "main::outer", "main::main"]);

    // The innermost frame points at the failing statement, the others at their
    // call sites, so every line in the trace is somewhere execution actually was.
    let index = grasm_syntax::span::LineIndex::new(&program.files[0].src);
    let lines: Vec<u32> = trace
        .iter()
        .map(|f| index.line_col(f.at.start).line)
        .collect();
    assert_eq!(lines, vec![2, 6, 10]);
}

#[test]
fn a_clean_run_records_no_trace() {
    let fx = Fixture::new("notrace", "fn main() {\n    print(\"fine\")\n}\n");
    let mut out = Vec::new();
    let (code, trace, _fault) = grasm_interp::run_program_traced(&fx.program(), &[], &mut out);
    assert_eq!(code, 0);
    assert!(trace.is_empty());
}

#[test]
fn the_first_panic_is_the_one_reported() {
    // A later panic while unwinding must not overwrite the original cause.
    let fx = Fixture::new(
        "firstpanic",
        "fn boom() -> i64 {\n    panic(\"original\")\n}\n\nfn main() {\n    print(\"{boom()}\")\n}\n",
    );
    let mut out = Vec::new();
    let (_, trace, fault) = grasm_interp::run_program_traced(&fx.program(), &[], &mut out);
    assert_eq!(trace[0].function, "main::boom");
    assert!(
        fault.as_deref().unwrap_or("").contains("original"),
        "fault: {:?}",
        fault
    );
}

#[test]
fn a_trace_spans_recursion_without_unbounded_growth() {
    let fx = Fixture::new(
        "recurse",
        "fn down(n: i64) -> i64 {\n    if n == 0 {\n        panic(\"bottom\")\n    }\n    down(n - 1)\n}\n\nfn main() {\n    print(\"{down(3)}\")\n}\n",
    );
    let mut out = Vec::new();
    let (_, trace, _fault) = grasm_interp::run_program_traced(&fx.program(), &[], &mut out);
    // 4 `down` frames (3, 2, 1, 0) plus `main`.
    assert_eq!(trace.len(), 5, "frames: {trace:?}");
    assert_eq!(trace[0].function, "main::down");
    assert_eq!(trace[4].function, "main::main");
}

/// Every boundary value the generator names is tried in every run.
///
/// They were drawn at random, which makes each one *likely* rather than
/// certain — and the default seed is fixed, so a value the list names on
/// purpose could be absent from every run anyone would ever make. `2` was: on
/// the default seed it first appeared at case 138, and the default run is 100
/// cases, so `assert(n != 2)` passed.
#[test]
fn a_run_tries_every_boundary_value() {
    use grasm_interp::testing::{generate, Rng};
    let fx = Fixture::new(
        "boundary_values",
        "test \"t\" (n: i64) {\n    assert(true)\n}\n\nfn main() {\n}\n",
    );
    let program = fx.program();
    let grasm_syntax::ast::Decl::Test(t) = &program.files[0].module.decls[0] else {
        panic!("expected the test declaration first");
    };
    let want: Vec<i64> = vec![0, 1, -1, 2, -2, 10, 100, i64::MIN, i64::MAX, i64::MIN + 1];
    // Whatever the seed: the opening cases do not consult it.
    for seed in [24301u64, 0, 1, 999] {
        let mut rng = Rng::new(seed);
        let mut seen = Vec::new();
        for case in 0..10u32 {
            let v = generate(&mut rng, &t.params[0].ty, 1 + case / 2, Some(case as usize));
            if let grasm_interp::Value::Int(n) = v {
                seen.push(n);
            }
        }
        for w in &want {
            assert!(
                seen.contains(w),
                "seed {seed} never generated {w}: {seen:?}"
            );
        }
    }
}

/// The same, through the whole runner rather than the generator alone: a
/// property false for a named boundary value fails inside the default run.
#[test]
fn a_property_false_at_a_boundary_fails_in_the_default_run() {
    for (value, spelling) in [
        (2i64, "2"),
        (-2, "0 - 2"),
        (100, "100"),
        (i64::MAX, "9223372036854775807"),
    ] {
        let src = format!(
            "test \"never\" (n: i64) {{\n    assert(n != ({spelling}))\n}}\n\nfn main() {{\n}}\n"
        );
        let fx = Fixture::new("boundary_run", &src);
        let program = fx.program();
        let files: Vec<_> = program
            .files
            .iter()
            .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
            .collect();
        let mut out = Vec::new();
        let mut interp = grasm_interp::Interp::new_program(&files, &mut out);
        let grasm_syntax::ast::Decl::Test(t) = &program.files[0].module.decls[0] else {
            panic!("expected the test declaration first");
        };
        let outcome = interp.run_property(&t.params, &t.body, &TestOptions::default());
        assert!(
            matches!(outcome, PropertyOutcome::Failed { .. }),
            "a property false at {value} passed the default run"
        );
    }
}

/// A counterexample whose parts must agree still shrinks.
///
/// Component-wise shrinking is greedy, and greedy cannot cross a failure the
/// parts share: `(107, 107)` failing "these differ" reported `(107, 107)`,
/// because `(0, 107)` and `(107, 0)` both hold, so every single step landed on
/// a passing value and the search stopped at whatever the generator produced.
#[test]
fn a_correlated_counterexample_shrinks_to_the_minimal_one() {
    let fx = Fixture::new(
        "correlated_shrink",
        "test \"pairs differ\" (p: (i64, i64)) {\n    assert(p.0 != p.1)\n}\n\n\
test \"all distinct\" (xs: [i64]) {\n    var i = 0\n    while i < xs.len() {\n        \
var j = i + 1\n        while j < xs.len() {\n            assert(xs[i] != xs[j])\n            \
j = j + 1\n        }\n        i = i + 1\n    }\n}\n\nfn main() {\n}\n",
    );
    let program = fx.program();
    let files: Vec<_> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    let mut out = Vec::new();
    let mut interp = grasm_interp::Interp::new_program(&files, &mut out);

    let grasm_syntax::ast::Decl::Test(pair) = &program.files[0].module.decls[0] else {
        panic!("expected the pair property first");
    };
    match interp.run_property(&pair.params, &pair.body, &TestOptions::default()) {
        PropertyOutcome::Failed { inputs, .. } => {
            assert_eq!(
                format!("{}", inputs[0].1),
                "(0, 0)",
                "not shrunk: {inputs:?}"
            );
        }
        PropertyOutcome::Passed(_) => panic!("this property cannot hold"),
    }

    let grasm_syntax::ast::Decl::Test(list) = &program.files[0].module.decls[1] else {
        panic!("expected the list property second");
    };
    match interp.run_property(&list.params, &list.body, &TestOptions::default()) {
        PropertyOutcome::Failed { inputs, .. } => {
            assert_eq!(
                format!("{}", inputs[0].1),
                "[0, 0]",
                "not shrunk: {inputs:?}"
            );
        }
        PropertyOutcome::Passed(_) => panic!("this property cannot hold"),
    }
}

/// A property false only when its parameters agree shrinks to the simplest
/// witness, not to whatever the generator produced ([E-TEST-SHRINK]).
///
/// The joint candidate existed inside a tuple and inside a list, and a `test`
/// with several parameters is the same shape one level out — where it did
/// not, so `(a, b)` reported `a = 107, b = 107` because `(0, 107)` and
/// `(107, 0)` both hold.
#[test]
fn parameters_that_must_agree_shrink_together() {
    let fx = Fixture::new(
        "joint_params",
        "test \"components differ\" (a: i64, b: i64) {\n    assert(a != b)\n}\n\nfn main() {\n}\n",
    );
    let program = fx.program();
    let files: Vec<_> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    let mut out = Vec::new();
    let mut interp = grasm_interp::Interp::new_program(&files, &mut out);
    let grasm_syntax::ast::Decl::Test(t) = &program.files[0].module.decls[0] else {
        panic!("expected the test declaration first");
    };
    match interp.run_property(&t.params, &t.body, &TestOptions::default()) {
        PropertyOutcome::Failed { inputs, .. } => {
            let shown: Vec<String> = inputs.iter().map(|(n, v)| format!("{n} = {v}")).collect();
            assert_eq!(shown, vec!["a = 0", "b = 0"], "not shrunk: {shown:?}");
        }
        PropertyOutcome::Passed(_) => panic!("this property cannot hold"),
    }
}
