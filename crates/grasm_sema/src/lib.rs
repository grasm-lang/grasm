//! grasm_sema — name resolution, type inference, and static checks.

pub mod builtins;
pub mod check;
pub mod docs;
pub mod effects;
pub mod loader;
pub mod tirfacts;
pub mod types;

pub use check::{check, Analysis};
pub use types::{Type, TypeDef, TypeDefKind, TypeDefs};

/// Convenience: parse + check in one call.
pub fn analyze(src: &str) -> (grasm_syntax::ast::Module, Vec<grasm_syntax::Diagnostic>) {
    let parsed = grasm_syntax::parse(src);
    let mut diags = parsed.diagnostics;
    if diags.is_empty() {
        let analysis = check(&parsed.module);
        diags.extend(analysis.diagnostics);
    }
    (parsed.module, diags)
}
