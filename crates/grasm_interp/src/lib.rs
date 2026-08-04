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
    let (module, diags) = grasm_sema::analyze(src);
    if !diags.is_empty() {
        return (1, diags);
    }
    let code = run_module(&module, out);
    (code, vec![])
}

pub fn run_module(module: &Module, out: &mut impl Write) -> i32 {
    let mut interp = Interp::new(module, out);
    interp.run_main()
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
