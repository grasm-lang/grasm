//! grasm_syntax — lexer, parser, AST, spans, diagnostics.

pub mod ast;
pub mod diag;
pub mod fmt;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod token;

pub use diag::{Diagnostic, Severity};
pub use lexer::lex;
pub use parser::{parse, parse_expr_src, ParseOutput};
pub use span::{LineCol, LineIndex, Span};
pub use token::{StrPart, Token, TokenKind};
