//! grasm_interp — the definitional tree-walking interpreter.

pub mod ffi;
pub mod interp;
pub mod journal;
pub mod testing;
pub mod value;

pub use interp::{DebugHook, Flow, Frame, Interp};
pub use value::Value;

use grasm_syntax::ast::Module;
use std::io::Write;

/// Check + run `main`. Returns (exit_code, diagnostics).
pub fn run_src<W: Write + Send>(src: &str, out: &mut W) -> (i32, Vec<grasm_syntax::Diagnostic>) {
    run_src_with_args(src, &[], out)
}

/// Like `run_src`, with program arguments for the `args()` builtin.
pub fn run_src_with_args<W: Write + Send>(
    src: &str,
    program_args: &[String],
    out: &mut W,
) -> (i32, Vec<grasm_syntax::Diagnostic>) {
    on_deep_stack(move || run_src_here(src, program_args, out))
}

fn run_src_here(
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
    report_fault(&interp.fault);
    (code, vec![])
}

/// Runs `f` on a thread with a large stack, so the depth guard is what stops
/// deep recursion rather than the host's stack running out.
///
/// This belongs here and not in the CLI. The interpreter recurses through
/// Rust frames, so *any* embedder — the conformance harness included — hits
/// the same wall, and an embedder should not have to know to spawn a thread
/// to get the recursion depth the language promises ([E-RECURSION-DEPTH]).
///
/// The stack is reserved, not committed: a program that does not recurse
/// deeply pays nothing.
/// On wasm there are no threads, so the interpreter runs on the stack it was
/// given.
///
/// This is a real difference and not a papered-over one: a wasm module's stack
/// is fixed at link time (1 MiB by default) and cannot be enlarged from inside,
/// so [E-RECURSION-DEPTH]'s promised depth is *not* available there. A program
/// that recurses deeply gets a wasm stack exhaustion — a trap, not a Grasm
/// panic with a sentence. Recorded in the §11 gap table rather than hidden,
/// because the alternative is a browser tab that dies without saying why.
#[cfg(target_arch = "wasm32")]
fn on_deep_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    f()
}

#[cfg(not(target_arch = "wasm32"))]
fn on_deep_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    // Reserved, not committed — see `STACK_BUDGET`, which this must exceed.
    #[cfg(debug_assertions)]
    const STACK_BYTES: usize = 2048 * 1024 * 1024;
    #[cfg(not(debug_assertions))]
    const STACK_BYTES: usize = 512 * 1024 * 1024;
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, f);
        match spawned {
            Ok(handle) => match handle.join() {
                Ok(v) => v,
                Err(payload) => std::panic::resume_unwind(payload),
            },
            // No thread available. The closure went with the failed spawn, so
            // there is nothing left to run inline.
            Err(e) => panic!("cannot start the interpreter thread: {e}"),
        }
    })
}

/// Runs a program and hands back its fault text instead of printing it.
///
/// The conformance harness needs to *read* a panic message, which is exactly
/// what sending it to stderr takes away from an in-process caller.
pub fn run_src_capturing_fault<W: Write + Send>(
    src: &str,
    out: &mut W,
) -> (i32, Option<String>, Vec<grasm_syntax::Diagnostic>) {
    run_src_capturing_fault_with_stdin(src, out, None)
}

/// The same, with the lines `input()` should read supplied by the caller.
///
/// `None` uses the process's stdin, which is what the command line wants.
/// `Some` is for a host that has none — a wasm module in a browser — where the
/// alternative is that no program using `input()` can run at all.
pub fn run_src_capturing_fault_with_stdin<W: Write + Send>(
    src: &str,
    out: &mut W,
    stdin: Option<Vec<String>>,
) -> (i32, Option<String>, Vec<grasm_syntax::Diagnostic>) {
    on_deep_stack(move || {
        // A whole program may arrive as one string. An embedder with no
        // filesystem — the wasm engine is the one that matters — cannot
        // resolve an `import` by reading the file it names, so a program with
        // one could not be run in a browser at all. A source bundle is that
        // program with its modules laid end to end, and it loads through the
        // same function the disk does.
        if grasm_sema::loader::is_source_bundle(src) {
            let program = grasm_sema::loader::load_from_source_bundle(src);
            if !program.diagnostics.is_empty() {
                return (1, None, program.diagnostics);
            }
            let analysis = grasm_sema::check::check_program(&program);
            if !analysis.diagnostics.is_empty() {
                return (1, None, analysis.diagnostics);
            }
            let files: Vec<(String, &Module, std::collections::HashMap<String, String>)> = program
                .files
                .iter()
                .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
                .collect();
            let mut interp = Interp::new_program(&files, out);
            interp.set_float_literals(analysis.float_literals);
            interp.set_ok_wrapped(analysis.ok_wrapped);
            interp.set_field_calls(analysis.field_calls);
            interp.set_fallible_closures(analysis.fallible_closures);
            interp.set_float_sums(analysis.float_sums);
            if let Some(lines) = stdin {
                interp.set_stdin(lines);
            }
            let code = interp.run_main();
            return (code, interp.fault.clone(), vec![]);
        }
        let (module, diags) = grasm_sema::analyze(src);
        if !diags.is_empty() {
            return (1, None, diags);
        }
        let mut interp = Interp::new(&module, out);
        if let Some(lines) = stdin {
            interp.set_stdin(lines);
        }
        let code = interp.run_main();
        (code, interp.fault.clone(), vec![])
    })
}

/// Sends a panic or an unhandled `main` error to stderr.
///
/// It is not the program's output: on stdout it lands in the middle of
/// whatever is reading the program, which is the one place it cannot be
/// noticed and the one place it does damage ([E-PANIC-EXIT]).
fn report_fault(fault: &Option<String>) {
    if let Some(f) = fault {
        eprintln!("{f}");
    }
}

pub fn run_module<W: Write + Send>(module: &Module, out: &mut W) -> i32 {
    on_deep_stack(move || {
        let mut interp = Interp::new(module, out);
        let code = interp.run_main();
        report_fault(&interp.fault);
        code
    })
}

/// Run a loaded program, also returning the call stack if it panicked and
/// the fault text, which has already been reported on stderr.
pub fn run_program_traced<W: Write + Send>(
    program: &grasm_sema::loader::Program,
    program_args: &[String],
    out: &mut W,
) -> (i32, Vec<Frame>, Option<String>) {
    on_deep_stack(move || run_program_traced_here(program, program_args, out))
}

fn run_program_traced_here(
    program: &grasm_sema::loader::Program,
    program_args: &[String],
    out: &mut impl Write,
) -> (i32, Vec<Frame>, Option<String>) {
    let files: Vec<(String, &Module, std::collections::HashMap<String, String>)> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    let mut interp = Interp::new_program(&files, out);
    // Which integer literals are `f64` is a fact only the checker has
    // ([T-INT-TO-FLOAT]); this engine has no types of its own.
    let analysis = grasm_sema::check::check_program(program);
    interp.set_float_literals(analysis.float_literals);
    interp.set_ok_wrapped(analysis.ok_wrapped);
    interp.set_field_calls(analysis.field_calls);
    interp.set_fallible_closures(analysis.fallible_closures);
    interp.set_float_sums(analysis.float_sums);
    interp.program_args = program_args.to_vec();
    let code = interp.run_main();
    report_fault(&interp.fault);
    let trace = interp.panic_trace().unwrap_or_default().to_vec();
    (code, trace, interp.fault.clone())
}

/// Run a loaded multi-file program. Assumes it already checked clean.
pub fn run_program<W: Write + Send>(
    program: &grasm_sema::loader::Program,
    program_args: &[String],
    out: &mut W,
) -> i32 {
    on_deep_stack(move || run_program_here(program, program_args, out))
}

fn run_program_here(
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
    // Which integer literals are `f64` is a fact only the checker has
    // ([T-INT-TO-FLOAT]); this engine has no types of its own.
    let analysis = grasm_sema::check::check_program(program);
    interp.set_float_literals(analysis.float_literals);
    interp.set_ok_wrapped(analysis.ok_wrapped);
    interp.set_field_calls(analysis.field_calls);
    interp.set_fallible_closures(analysis.fallible_closures);
    interp.set_float_sums(analysis.float_sums);
    interp.program_args = program_args.to_vec();
    let code = interp.run_main();
    report_fault(&interp.fault);
    code
}

/// Run the entry module's test blocks of a loaded program.
pub fn test_program<W: Write + Send>(
    program: &grasm_sema::loader::Program,
    out: &mut W,
) -> (u32, u32) {
    let summary = test_program_with(program, &testing::TestOptions::default(), out);
    (summary.passed, summary.failed)
}

/// Run every `test` declaration in the entry module: unit tests directly,
/// property tests over generated inputs, with optional line coverage.
pub fn test_program_with<W: Write + Send>(
    program: &grasm_sema::loader::Program,
    opts: &testing::TestOptions,
    out: &mut W,
) -> testing::TestSummary {
    on_deep_stack(move || test_program_here(program, opts, out))
}

fn test_program_here(
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
    // Which integer literals are `f64` is a fact only the checker has
    // ([T-INT-TO-FLOAT]); this engine has no types of its own.
    let analysis = grasm_sema::check::check_program(program);
    interp.set_float_literals(analysis.float_literals);
    interp.set_ok_wrapped(analysis.ok_wrapped);
    interp.set_field_calls(analysis.field_calls);
    interp.set_fallible_closures(analysis.fallible_closures);
    interp.set_float_sums(analysis.float_sums);
    if opts.coverage {
        interp.enable_coverage();
    }

    let mut passed = 0;
    let mut failed = 0;
    // Every module's tests, not just the entry file's. A library's tests used
    // to be skipped in silence — `grasm test main.gr` reported "1 passed, 0
    // failed" and exit 0 while a deliberately broken test in an imported
    // module never ran. Doc examples already crossed the module boundary, so
    // one kind of test in a file ran and the other did not.
    let mut units: Vec<(&str, &grasm_syntax::ast::TestDecl)> = Vec::new();
    for file in &program.files {
        for decl in &file.module.decls {
            if let Decl::Test(t) = decl {
                units.push((file.name.as_str(), t));
            }
        }
    }
    let multi = program.files.len() > 1;
    for (module, t) in units {
        interp.set_current_module(module);
        let label = if multi {
            format!("{module}::{}", t.name)
        } else {
            t.name.clone()
        };
        let t = &grasm_syntax::ast::TestDecl {
            name: label,
            ..t.clone()
        };
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

    let unit_hits = interp.take_coverage();

    // Examples in `///` comments are tests. Documentation that stops being
    // true should fail the suite, not quietly mislead the next reader.
    let (doc_passed, doc_failed, doc_hits) =
        run_doc_examples(program, &opts.filter, out, opts.coverage);
    // Their lines count as covered. They run in their own interpreter — each
    // group is its own synthesized program — so their hits were dropped, and
    // a function reached *only* by its documented example was reported
    // uncovered even though the suite had just run it. The hits come back as
    // (file, line) rather than as offsets, because the offsets belong to a
    // program with a different span space and a different set of files.
    let coverage = unit_hits.map(|hits| {
        let mut cov = build_coverage(program, &hits);
        cov.covered.extend(doc_hits);
        cov
    });
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
    coverage: bool,
) -> (u32, u32, std::collections::BTreeSet<(String, u32)>) {
    let mut passed = 0;
    let mut failed = 0;
    let mut hits: std::collections::BTreeSet<(String, u32)> = std::collections::BTreeSet::new();
    let all: Vec<grasm_sema::docs::Example> = grasm_sema::docs::examples(program)
        .into_iter()
        .filter(|e| filter.is_empty() || e.owner.contains(filter))
        .collect();
    // The examples of one module share a synthesized program, so the whole
    // graph is parsed and checked once per module rather than once per
    // example. A module with sixty documented items checked its dependencies
    // sixty times, which is what made `grasm test` on a real project spend
    // nearly all of its time in the checker.
    let mut groups: Vec<(String, Vec<&grasm_sema::docs::Example>)> = Vec::new();
    for e in &all {
        let owner_module = e.owner.split("::").next().unwrap_or("main").to_string();
        match groups.iter_mut().find(|(m, _)| *m == owner_module) {
            Some((_, v)) => v.push(e),
            None => groups.push((owner_module, vec![e])),
        }
    }
    for (owner, group) in &groups {
        // A group that does not compile is retried one example at a time, so
        // a single bad example still reports against itself rather than
        // taking its module's other examples down with it.
        let grouped = grasm_sema::docs::example_group_program(program, group);
        let mut gdiags = grouped.diagnostics.clone();
        let ganalysis = grasm_sema::check::check_program(&grouped);
        gdiags.extend(ganalysis.diagnostics.clone());
        if gdiags.is_empty() {
            // One interpreter for the whole group. Building one per example
            // registers the module's declarations — its other examples
            // included — once per example, which made `grasm test` quadratic
            // in the number of documented items: 8,000 of them took 7.7 s
            // where the same count of plain `test` blocks took 0.13 s.
            let names: Vec<String> = group.iter().map(|e| e.owner.clone()).collect();
            let (oks, h) = run_examples_together(&grouped, &ganalysis, &names, out, coverage);
            hits.extend(doc_hits_in_original(&grouped, program, owner, &h));
            for ok in oks {
                if ok {
                    passed += 1;
                } else {
                    failed += 1;
                }
            }
            continue;
        }
        for example in group {
            let (p, f, h) = run_example_alone(program, example, out, coverage);
            passed += p;
            failed += f;
            hits.extend(h);
        }
    }
    (passed, failed, hits)
}

/// One example, in its own program — the slow path, for a module whose group
/// did not compile.
fn run_example_alone(
    program: &grasm_sema::loader::Program,
    example: &grasm_sema::docs::Example,
    out: &mut impl Write,
    coverage: bool,
) -> (u32, u32, std::collections::BTreeSet<(String, u32)>) {
    let ex_program = grasm_sema::docs::example_program(program, example);
    let mut diags = ex_program.diagnostics.clone();
    let analysis = grasm_sema::check::check_program(&ex_program);
    diags.extend(analysis.diagnostics.clone());
    if !diags.is_empty() {
        let _ = writeln!(
            out,
            "doc example {} ... FAILED to compile: {}",
            example.owner, diags[0].message
        );
        return (0, 1, std::collections::BTreeSet::new());
    }
    let (ok, hits) = run_one_example(&ex_program, &analysis, &example.owner, out, coverage);
    let owner_module = example.owner.split("::").next().unwrap_or("main");
    let hits = doc_hits_in_original(&ex_program, program, owner_module, &hits);
    if ok {
        (1, 0, hits)
    } else {
        (0, 1, hits)
    }
}

/// Runs every named test a synthesized program holds, in one interpreter.
///
/// Each test's own report line is relabelled as a doc example, so the output
/// is what running them one at a time produced.
fn run_examples_together(
    ex_program: &grasm_sema::loader::Program,
    analysis: &grasm_sema::check::Analysis,
    wanted: &[String],
    out: &mut impl Write,
    coverage: bool,
) -> (Vec<bool>, std::collections::BTreeSet<u32>) {
    let mut hits: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    let entry = &ex_program.files[0].module;
    let files: Vec<(String, &Module, std::collections::HashMap<String, String>)> = ex_program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    let mut oks = Vec::with_capacity(wanted.len());
    // What each example wrote, and how it ended. The two used to share one
    // buffer that was then split into lines, so an example containing a
    // `print` slid the mapping by one: its own status was lost and the *next*
    // example was reported with it — a failing example printed `ok`, with its
    // reason gone. The buffer is shared so the boundaries can be read while
    // the interpreter still holds the writer.
    let shared = SharedBuf::default();
    let mut chunks: Vec<(usize, usize, Result<(), String>)> = Vec::new();
    {
        let mut sink = shared.clone();
        let mut interp = Interp::new_program(&files, &mut sink);
        interp.set_float_literals(analysis.float_literals.clone());
        interp.set_ok_wrapped(analysis.ok_wrapped.clone());
        interp.set_field_calls(analysis.field_calls.clone());
        interp.set_fallible_closures(analysis.fallible_closures.clone());
        interp.set_float_sums(analysis.float_sums.clone());
        if coverage {
            interp.enable_coverage();
        }
        for name in wanted {
            let quoted = name.replace('"', "'");
            let found = entry
                .decls
                .iter()
                .find(|d| matches!(d, grasm_syntax::ast::Decl::Test(t) if t.name == quoted));
            let start = shared.len();
            let outcome = match found {
                Some(grasm_syntax::ast::Decl::Test(t)) => interp.run_unit_test_result(t),
                _ => Err("no test".to_string()),
            };
            let end = shared.len();
            oks.push(outcome.is_ok());
            chunks.push((start, end, outcome));
        }
        if let Some(h) = interp.take_coverage() {
            hits.extend(h);
        }
    }
    // Each example's own output, then its report line — the order running
    // them one at a time produced.
    let written = shared.take();
    for (name, (start, end, outcome)) in wanted.iter().zip(chunks) {
        let _ = out.write_all(&written[start..end]);
        match outcome {
            Ok(()) => {
                let _ = writeln!(out, "doc example {name} ... ok");
            }
            Err(message) => {
                let _ = writeln!(out, "doc example {name} ... FAILED: {message}");
            }
        }
    }
    (oks, hits)
}

/// A byte sink whose contents can be read while something else is writing to
/// it, so the boundary between two runs can be recorded as it passes.
#[derive(Clone, Default)]
struct SharedBuf(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

impl SharedBuf {
    fn len(&self) -> usize {
        self.0.borrow().len()
    }

    fn take(self) -> Vec<u8> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

impl Write for SharedBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Runs the test a synthesized program holds under `wanted`.
fn run_one_example(
    ex_program: &grasm_sema::loader::Program,
    analysis: &grasm_sema::check::Analysis,
    wanted: &str,
    out: &mut impl Write,
    coverage: bool,
) -> (bool, std::collections::BTreeSet<u32>) {
    let mut hits: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    let entry = &ex_program.files[0].module;
    let files: Vec<(String, &Module, std::collections::HashMap<String, String>)> = ex_program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    let mut buf = Vec::new();
    // By name, not by position. The synthesized entry inlines the
    // documented module's own declarations — its tests included — and
    // appends the examples after them, so taking the *first* test ran one
    // of the module's own and reported the example as `ok`. Any real
    // project has a test in its entry module, so this reported every doc
    // example in it as passing without executing one.
    let quoted = wanted.replace('"', "'");
    let Some(grasm_syntax::ast::Decl::Test(t)) = entry
        .decls
        .iter()
        .find(|d| matches!(d, grasm_syntax::ast::Decl::Test(t) if t.name == quoted))
    else {
        let _ = writeln!(out, "doc example {wanted} ... FAILED: no test");
        return (false, hits);
    };
    let outcome = {
        let mut interp = Interp::new_program(&files, &mut buf);
        interp.set_float_literals(analysis.float_literals.clone());
        interp.set_ok_wrapped(analysis.ok_wrapped.clone());
        interp.set_field_calls(analysis.field_calls.clone());
        interp.set_fallible_closures(analysis.fallible_closures.clone());
        interp.set_float_sums(analysis.float_sums.clone());
        if coverage {
            interp.enable_coverage();
        }
        let outcome = interp.run_unit_test_result(t);
        if let Some(h) = interp.take_coverage() {
            hits.extend(h);
        }
        outcome
    };
    // The example's own output, then a line naming it as a doc example rather
    // than as the declared test it was synthesized into. Recovering that line
    // by splitting the buffer took whatever the *body* printed as the status,
    // and dropped the output entirely.
    let _ = out.write_all(&buf);
    match &outcome {
        Ok(()) => {
            let _ = writeln!(out, "doc example {wanted} ... ok");
        }
        Err(message) => {
            let _ = writeln!(out, "doc example {wanted} ... FAILED: {message}");
        }
    }
    (outcome.is_ok(), hits)
}

/// Maps executed span offsets back to (file, line), against every line that
/// could have executed.
/// Coverage from a synthesized doc-example program, read back as (file, line)
/// in the original one.
///
/// A group's program is not the program under test: the owner's source is
/// *inlined* into a synthesized entry, and the offsets a run reports belong to
/// that entry's span space. Read against the original they landed on the wrong
/// file or on nothing at all, so a function reached only by its documented
/// example was reported uncovered — the failure the coverage rule exists to
/// prevent, one import away. The inlining is line for line, so the whole
/// translation is a fixed line offset plus a name lookup.
fn doc_hits_in_original(
    synth: &grasm_sema::loader::Program,
    original: &grasm_sema::loader::Program,
    owner_module: &str,
    hits: &std::collections::BTreeSet<u32>,
) -> std::collections::BTreeSet<(String, u32)> {
    let mut out = std::collections::BTreeSet::new();
    if hits.is_empty() {
        return out;
    }
    let prefix = grasm_sema::docs::example_group_prefix_lines(original, owner_module) as u32;
    let indexes: Vec<grasm_syntax::span::LineIndex> = synth
        .files
        .iter()
        .map(|f| grasm_syntax::span::LineIndex::new(&f.src))
        .collect();
    // How many lines the owner's own source has, so the appended `test`
    // blocks — which belong to no file anyone wrote — are dropped.
    let owner_lines = original
        .files
        .iter()
        .find(|f| f.name == owner_module)
        .map(|f| f.src.lines().count() as u32);
    for &offset in hits {
        let Some(i) = synth.files.iter().rposition(|f| offset >= f.base) else {
            continue;
        };
        let f = &synth.files[i];
        let line = indexes[i].line_col(offset - f.base).line;
        if f.name == "main" {
            // The synthesized entry: the owner's lines, shifted.
            if line <= prefix {
                continue;
            }
            let owner_line = line - prefix;
            if owner_lines.is_some_and(|n| owner_line > n) {
                continue;
            }
            if let Some(orig) = original.files.iter().find(|o| o.name == owner_module) {
                out.insert((orig.path.clone(), owner_line));
            }
            continue;
        }
        // Every other module is carried over unchanged, so its lines are its
        // own — but under the name the original program gave it.
        let name = f.name.trim_start_matches("__doc_host");
        if let Some(orig) = original
            .files
            .iter()
            .find(|o| o.name == f.name || (name.is_empty() && o.name == "main"))
        {
            out.insert((orig.path.clone(), line));
        }
    }
    out
}

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
pub fn bench_program<W: Write + Send>(
    program: &grasm_sema::loader::Program,
    filter: &str,
    out: &mut W,
) -> Vec<testing::BenchResult> {
    on_deep_stack(move || bench_program_here(program, filter, out))
}

fn bench_program_here(
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
    // Which integer literals are `f64` is a fact only the checker has
    // ([T-INT-TO-FLOAT]); this engine has no types of its own.
    let analysis = grasm_sema::check::check_program(program);
    interp.set_float_literals(analysis.float_literals);
    interp.set_ok_wrapped(analysis.ok_wrapped);
    interp.set_field_calls(analysis.field_calls);
    interp.set_fallible_closures(analysis.fallible_closures);
    interp.set_float_sums(analysis.float_sums);
    let mut results = Vec::new();
    // Every module's benchmarks, the way `grasm test` runs every module's
    // tests. Scanning only the entry file reported "no benchmarks found" for a
    // library whose submodules each declared one.
    let multi = program.files.len() > 1;
    for file in &program.files {
        for decl in &file.module.decls {
            let Decl::Bench(b) = decl else { continue };
            if !filter.is_empty() && !b.name.contains(filter) {
                continue;
            }
            interp.set_current_module(&file.name);
            let label = if multi {
                format!("{}::{}", file.name, b.name)
            } else {
                b.name.clone()
            };
            results.push(interp.run_bench(&label, &b.body));
        }
    }
    results
}

/// Check + run all `test` blocks. Returns (passed, failed, diagnostics).
pub fn test_src(src: &str, out: &mut impl Write) -> (u32, u32, Vec<grasm_syntax::Diagnostic>) {
    let (module, diags) = grasm_sema::analyze(src);
    if !diags.is_empty() {
        return (0, 0, diags);
    }
    let mut interp = Interp::new(&module, out);
    let (p, f) = interp.run_tests(&module);
    (p, f, vec![])
}
