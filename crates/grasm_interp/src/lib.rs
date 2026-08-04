//! grasm_interp — the definitional tree-walking interpreter.

pub mod interp;
pub mod value;

pub use interp::{Flow, Interp};
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
    let files: Vec<(String, &Module, std::collections::HashMap<String, String>)> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    let mut interp = Interp::new_program(&files, out);
    interp.run_tests(&program.files[0].module)
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
