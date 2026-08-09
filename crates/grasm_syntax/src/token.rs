//! Token definitions for Grasm-Core.

use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Ident(String),
    Int(i64),
    Float(f64),
    /// String literal parts: text segments interleaved with `{expr}` interpolation
    /// slots. A plain string is a single `Text` part.
    Str(Vec<StrPart>),

    // Keywords
    KwFn,
    KwPub,
    KwLet,
    KwVar,
    KwOwn,
    KwInout,
    KwType,
    KwInterface,
    KwImpl,
    KwEffect,
    KwWith,
    KwHandledBy,
    KwMatch,
    KwIf,
    KwElse,
    KwFor,
    KwWhile,
    KwUnsafe,
    KwRef,
    KwIn,
    KwReturn,
    KwTry,
    KwTrue,
    KwFalse,
    KwBreak,
    KwContinue,
    KwAnd,
    KwOr,
    KwNot,
    KwImport,
    KwTest,

    // Punctuation & operators
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Dot,
    DotDot,     // ..
    DotDotEq,   // ..=
    Colon,
    ColonColon,
    Arrow,      // ->
    FatArrow,   // =>
    Pipe,       // |
    PipeGt,     // |>
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,      // ^ (power)
    Eq,         // =
    EqEq,
    NotEq,      // !=
    Lt,
    LtEq,
    Gt,
    GtEq,
    Bang,       // !  (error union in types)
    Question,   // ?
    QuestionDot,     // ?.
    QuestionQuestion, // ??
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    Underscore, // _ wildcard pattern
    At,         // @ (annotations like @derive)

    /// One or more newlines (collapsed). Statement terminator.
    Newline,
    /// Lexical error; message kept for diagnostics, lexing continues.
    Error(String),
    Eof,
}

/// A piece of a string literal.
#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Text(String),
    /// Raw source of an interpolated expression: `"hi {name}"`.
    /// Parsed into an expression by the parser. `offset` is the byte offset of
    /// `src` within the file, so sub-expression spans stay accurate.
    Interp { src: String, offset: u32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl TokenKind {
    pub fn keyword_from(s: &str) -> Option<TokenKind> {
        use TokenKind::*;
        Some(match s {
            "fn" => KwFn,
            "pub" => KwPub,
            "let" => KwLet,
            "var" => KwVar,
            "own" => KwOwn,
            "inout" => KwInout,
            "type" => KwType,
            "interface" => KwInterface,
            "impl" => KwImpl,
            "effect" => KwEffect,
            "with" => KwWith,
            "handled_by" => KwHandledBy,
            "match" => KwMatch,
            "if" => KwIf,
            "else" => KwElse,
            "for" => KwFor,
            "while" => KwWhile,
            "unsafe" => KwUnsafe,
            "ref" => KwRef,
            "in" => KwIn,
            "return" => KwReturn,
            "try" => KwTry,
            "true" => KwTrue,
            "false" => KwFalse,
            "break" => KwBreak,
            "continue" => KwContinue,
            "and" => KwAnd,
            "or" => KwOr,
            "not" => KwNot,
            "import" => KwImport,
            "test" => KwTest,
            _ => return None,
        })
    }

    /// Human-readable name for diagnostics.
    pub fn describe(&self) -> String {
        use TokenKind::*;
        match self {
            Ident(n) => format!("identifier `{n}`"),
            Int(v) => format!("integer `{v}`"),
            Float(v) => format!("float `{v}`"),
            Str(_) => "string literal".into(),
            Newline => "end of line".into(),
            Eof => "end of file".into(),
            Error(m) => format!("invalid token ({m})"),
            k => format!("`{}`", k.text()),
        }
    }

    fn text(&self) -> &'static str {
        use TokenKind::*;
        match self {
            KwFn => "fn",
            KwPub => "pub",
            KwLet => "let",
            KwVar => "var",
            KwOwn => "own",
            KwUnsafe => "unsafe",
            KwRef => "ref",
            KwInout => "inout",
            KwType => "type",
            KwInterface => "interface",
            KwImpl => "impl",
            KwEffect => "effect",
            KwWith => "with",
            KwHandledBy => "handled_by",
            KwMatch => "match",
            KwIf => "if",
            KwElse => "else",
            KwFor => "for",
            KwWhile => "while",
            KwIn => "in",
            KwReturn => "return",
            KwTry => "try",
            KwTrue => "true",
            KwFalse => "false",
            KwBreak => "break",
            KwContinue => "continue",
            KwAnd => "and",
            KwOr => "or",
            KwNot => "not",
            KwImport => "import",
            KwTest => "test",
            LParen => "(",
            RParen => ")",
            LBrace => "{",
            RBrace => "}",
            LBracket => "[",
            RBracket => "]",
            Comma => ",",
            Dot => ".",
            DotDot => "..",
            DotDotEq => "..=",
            Colon => ":",
            ColonColon => "::",
            Arrow => "->",
            FatArrow => "=>",
            Pipe => "|",
            PipeGt => "|>",
            Plus => "+",
            Minus => "-",
            Star => "*",
            Slash => "/",
            Percent => "%",
            Caret => "^",
            Eq => "=",
            EqEq => "==",
            NotEq => "!=",
            Lt => "<",
            LtEq => "<=",
            Gt => ">",
            GtEq => ">=",
            Bang => "!",
            Question => "?",
            QuestionDot => "?.",
            QuestionQuestion => "??",
            PlusEq => "+=",
            MinusEq => "-=",
            StarEq => "*=",
            SlashEq => "/=",
            Underscore => "_",
            At => "@",
            Ident(_) | Int(_) | Float(_) | Str(_) | Newline | Error(_) | Eof => unreachable!(),
        }
    }
}
