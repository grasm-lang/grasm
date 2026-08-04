//! grasm_interp — the definitional tree-walking interpreter.

pub mod interp;
pub mod testing;
pub mod value;

pub use interp::{DebugHook, Flow, Frame, Interp};
pub use value::Value;

use grasm_syntax::ast::Module;
use std::io::Write;

/// Check + run `main`. Returns (exit_code, diagnostics).
pub fn run_src(
    src: &str,
    out: &mut impl Write,
) -> (i32, Vec<grasm_syntax::Diagnostic>) {
    run_src_with_args(src, &[], out)
}

/// Like `run_src`, with program arguments for the `args()` builtin.
pub fn run_src_with_args(
    src: &str,
    program_args: &[String],
    out: &mut impl Write,
) -> (i32, Vec<grasm_syntax::Diagnostic>) {
    let (module, diags) = grasm_sema::analyze(src);
    if !diags.is_empty() {
        return (1, diags);
    }
    let mut interp = Interp::new(&module, out);
    interp.program_args = program_args.to_vec();
    let code = interp.run_main();
    (code, vec![])
}

pub fn run_module(module: &Module, out: &mut impl Write) -> i32 {
    let mut interp = Interp::new(module, out);
    interp.run_main()
}

/// Run a loaded program, also returning the call stack if it panicked.
pub fn run_program_traced(
    program: &grasm_sema::loader::Program,
    program_args: &[String],
    out: &mut impl Write,
) -> (i32, Vec<Frame>) {
    let files: Vec<(String, &Module, std::collections::HashMap<String, String>)> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    let mut interp = Interp::new_program(&files, out);
    interp.program_args = program_args.to_vec();
    let code = interp.run_main();
    let trace = interp.panic_trace().unwrap_or_default().to_vec();
    (code, trace)
}

/// Run a loaded multi-file program. Assumes it already checked clean.
pub fn run_program(
    program: &grasm_sema::loader::Program,
    program_args: &[String],
    out: &mut impl Write,
) -> i32 {
    let files: Vec<(String, &Module, std::collections::HashMap<String, String>)> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    let mut interp = Interp::new_program(&files, out);
    interp.program_args = program_args.to_vec();
    interp.run_main()
}

/// Run the entry module's test blocks of a loaded program.
pub fn test_program(
    program: &grasm_sema::loader::Program,
    out: &mut impl Write,
) -> (u32, u32) {
    let summary = test_program_with(program, &testing::TestOptions::default(), out);
    (summary.passed, summary.failed)
}

/// Run every `test` declaration in the entry module: unit tests directly,
/// property tests over generated inputs, with optional line coverage.
pub fn test_program_with(
    program: &grasm_sema::loader::Program,
    opts: &testing::TestOptions,
    out: &mut impl Write,
) -> testing::TestSummary {
    use grasm_syntax::ast::Decl;

    let files: Vec<(String, &Module, std::collections::HashMap<String, String>)> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    let mut interp = Interp::new_program(&files, out);
    if opts.coverage {
        interp.enable_coverage();
    }

    let mut passed = 0;
    let mut failed = 0;
    for decl in &program.files[0].module.decls {
        let Decl::Test(t) = decl else { continue };
        if !opts.filter.is_empty() && !t.name.contains(&opts.filter) {
            continue;
        }
        if t.params.is_empty() {
            if interp.run_unit_test(t) {
                passed += 1;
            } else {
                failed += 1;
            }
            continue;
        }
        match interp.run_property(&t.params, &t.body, opts) {
            testing::PropertyOutcome::Passed(cases) => {
                interp.report(&format!("test \"{}\" ... ok ({cases} cases)", t.name));
                passed += 1;
            }
            testing::PropertyOutcome::Failed {
                inputs,
                message,
                case,
            } => {
                // The seed makes the failure replayable: the whole point of a
                // property failure is being able to get back to it.
                interp.report(&format!(
                    "test \"{}\" ... FAILED on case {case} of {}\n  {}\n  counterexample: {}\n  replay: --seed {} --cases {}",
                    t.name,
                    opts.cases,
                    message,
                    testing::render_inputs(&inputs),
                    opts.seed,
                    opts.cases,
                ));
                failed += 1;
            }
        }
    }

    let coverage = interp
        .take_coverage()
        .map(|hits| build_coverage(program, &hits));

    // Examples in `///` comments are tests. Documentation that stops being
    // true should fail the suite, not quietly mislead the next reader.
    let (doc_passed, doc_failed) = run_doc_examples(program, &opts.filter, out);
    testing::TestSummary {
        passed: passed + doc_passed,
        failed: failed + doc_failed,
        coverage,
    }
}

/// Runs every fenced example found in the program's doc comments.
///
/// Each example is compiled as its own program with the module's declarations
/// in scope, so it can call the thing it documents.
fn run_doc_examples(
    program: &grasm_sema::loader::Program,
    filter: &str,
    out: &mut impl Write,
) -> (u32, u32) {
    let mut passed = 0;
    let mut failed = 0;
    for example in grasm_sema::docs::examples(program) {
        if !filter.is_empty() && !example.owner.contains(filter) {
            continue;
        }
        let ex_program = grasm_sema::docs::example_program(program, &example);
        let mut diags = ex_program.diagnostics.clone();
        diags.extend(grasm_sema::check::check_program(&ex_program).diagnostics);
        if !diags.is_empty() {
            let _ = writeln!(
                out,
                "doc example {} ... FAILED to compile: {}",
                example.owner, diags[0].message
            );
            failed += 1;
            continue;
        }
        let entry = &ex_program.files[0].module;
        let files: Vec<(String, &Module, std::collections::HashMap<String, String>)> = ex_program
            .files
            .iter()
            .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
            .collect();
        let mut buf = Vec::new();
        let ok = {
            let mut interp = Interp::new_program(&files, &mut buf);
            let Some(grasm_syntax::ast::Decl::Test(t)) = entry
                .decls
                .iter()
                .find(|d| matches!(d, grasm_syntax::ast::Decl::Test(_)))
            else {
                // example_program always appends one, so this cannot happen.
                let _ = writeln!(out, "doc example {} ... FAILED: no test", example.owner);
                failed += 1;
                continue;
            };
            interp.run_unit_test(t)
        };
        // The example's own report names it as a test; relabel so a reader can
        // tell a doc example from a declared one.
        let report = String::from_utf8_lossy(&buf);
        let detail = report
            .split_once("... ")
            .map(|(_, rest)| rest.trim_end().to_string())
            .unwrap_or_default();
        let _ = writeln!(out, "doc example {} ... {detail}", example.owner);
        if ok {
            passed += 1;
        } else {
            failed += 1;
        }
    }
    (passed, failed)
}

/// Maps executed span offsets back to (file, line), against every line that
/// could have executed.
fn build_coverage(
    program: &grasm_sema::loader::Program,
    hits: &std::collections::BTreeSet<u32>,
) -> testing::Coverage {
    let mut cov = testing::Coverage::default();
    let indexes: Vec<grasm_syntax::span::LineIndex> = program
        .files
        .iter()
        .map(|f| grasm_syntax::span::LineIndex::new(&f.src))
        .collect();
    for (i, f) in program.files.iter().enumerate() {
        testing::executable_lines(&f.path, &f.module, f.base, &indexes[i], &mut cov.executable);
    }
    for &offset in hits {
        let i = program
            .files
            .iter()
            .rposition(|f| offset >= f.base)
            .unwrap_or(0);
        let f = &program.files[i];
        let line = indexes[i].line_col(offset - f.base).line;
        cov.covered.insert((f.path.clone(), line));
    }
    cov
}

/// Run every `bench` declaration in the entry module.
pub fn bench_program(
    program: &grasm_sema::loader::Program,
    filter: &str,
    out: &mut impl Write,
) -> Vec<testing::BenchResult> {
    use grasm_syntax::ast::Decl;
    let files: Vec<(String, &Module, std::collections::HashMap<String, String>)> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    let mut interp = Interp::new_program(&files, out);
    let mut results = Vec::new();
    for decl in &program.files[0].module.decls {
        let Decl::Bench(b) = decl else { continue };
        if !filter.is_empty() && !b.name.contains(filter) {
            continue;
        }
        results.push(interp.run_bench(&b.name, &b.body));
    }
    results
}

/// Check + run all `test` blocks. Returns (passed, failed, diagnostics).
pub fn test_src(
    src: &str,
    out: &mut impl Write,
) -> (u32, u32, Vec<grasm_syntax::Diagnostic>) {
    let (module, diags) = grasm_sema::analyze(src);
    if !diags.is_empty() {
        return (0, 0, diags);
    }
    let mut interp = Interp::new(&module, out);
    let (p, f) = interp.run_tests(&module);
    (p, f, vec![])
}
