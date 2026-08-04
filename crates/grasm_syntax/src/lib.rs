//! grasm_syntax — lexer, tokens, spans (parser + AST arrive in Phase 2).

pub mod lexer;
pub mod span;
pub mod token;

pub use lexer::lex;
pub use span::{LineCol, LineIndex, Span};
pub use token::{StrPart, Token, TokenKind};
