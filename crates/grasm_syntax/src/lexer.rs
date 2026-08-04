//! Hand-written lexer. Error-tolerant: unknown input becomes `Error` tokens and
//! lexing continues, so the parser and IDE layers always get a full token stream.

use crate::span::Span;
use crate::token::{StrPart, Token, TokenKind};

pub struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

pub fn lex(src: &str) -> Vec<Token> {
    Lexer::new(src).run()
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    fn run(mut self) -> Vec<Token> {
        let mut out = Vec::new();
        loop {
            let tok = self.next_token();
            let is_eof = tok.kind == TokenKind::Eof;
            // Collapse consecutive newlines into one.
            if tok.kind == TokenKind::Newline
                && matches!(out.last().map(|t: &Token| &t.kind), Some(TokenKind::Newline))
            {
                if let Some(last) = out.last_mut() {
                    last.span = last.span.merge(tok.span);
                }
            } else {
                out.push(tok);
            }
            if is_eof {
                break;
            }
        }
        out
    }

    fn peek(&self) -> u8 {
        *self.bytes.get(self.pos).unwrap_or(&0)
    }

    fn peek2(&self) -> u8 {
        *self.bytes.get(self.pos + 1).unwrap_or(&0)
    }

    fn bump(&mut self) -> u8 {
        let b = self.peek();
        self.pos += 1;
        b
    }

    fn tok(&self, kind: TokenKind, start: usize) -> Token {
        Token {
            kind,
            span: Span::new(start as u32, self.pos as u32),
        }
    }

    fn next_token(&mut self) -> Token {
        self.skip_trivia();
        let start = self.pos;
        if self.pos >= self.bytes.len() {
            return self.tok(TokenKind::Eof, start);
        }
        let b = self.peek();
        match b {
            b'\n' => {
                self.bump();
                self.tok(TokenKind::Newline, start)
            }
            b'"' => self.lex_string(),
            b'0'..=b'9' => self.lex_number(),
            b'_' => {
                if is_ident_continue(self.peek2()) {
                    self.lex_ident()
                } else {
                    self.bump();
                    self.tok(TokenKind::Underscore, start)
                }
            }
            b if is_ident_start(b) => self.lex_ident(),
            _ => self.lex_operator(),
        }
    }

    /// Skips spaces, tabs, carriage returns, and comments (line + nested block).
    /// Does NOT skip newlines — they are tokens.
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                b' ' | b'\t' | b'\r' => {
                    self.bump();
                }
                b'/' if self.peek2() == b'/' => {
                    while self.pos < self.bytes.len() && self.peek() != b'\n' {
                        self.bump();
                    }
                }
                b'/' if self.peek2() == b'*' => {
                    self.bump();
                    self.bump();
                    let mut depth = 1u32;
                    while depth > 0 && self.pos < self.bytes.len() {
                        if self.peek() == b'/' && self.peek2() == b'*' {
                            self.bump();
                            self.bump();
                            depth += 1;
                        } else if self.peek() == b'*' && self.peek2() == b'/' {
                            self.bump();
                            self.bump();
                            depth -= 1;
                        } else {
                            self.bump();
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn lex_ident(&mut self) -> Token {
        let start = self.pos;
        while is_ident_continue(self.peek()) {
            self.bump();
        }
        let text = &self.src[start..self.pos];
        let kind = TokenKind::keyword_from(text)
            .unwrap_or_else(|| TokenKind::Ident(text.to_string()));
        self.tok(kind, start)
    }

    fn lex_number(&mut self) -> Token {
        let start = self.pos;
        // Hex / binary
        if self.peek() == b'0' && (self.peek2() == b'x' || self.peek2() == b'b') {
            let radix = if self.peek2() == b'x' { 16 } else { 2 };
            self.bump();
            self.bump();
            let digits_start = self.pos;
            while self.peek().is_ascii_alphanumeric() || self.peek() == b'_' {
                self.bump();
            }
            let text: String = self.src[digits_start..self.pos]
                .chars()
                .filter(|&c| c != '_')
                .collect();
            return match i64::from_str_radix(&text, radix) {
                Ok(v) => self.tok(TokenKind::Int(v), start),
                Err(_) => self.tok(
                    TokenKind::Error(format!("invalid numeric literal `{}`", &self.src[start..self.pos])),
                    start,
                ),
            };
        }

        let mut is_float = false;
        while self.peek().is_ascii_digit() || self.peek() == b'_' {
            self.bump();
        }
        // Fractional part — but `1..5` is a range, not a float.
        if self.peek() == b'.' && self.peek2().is_ascii_digit() {
            is_float = true;
            self.bump();
            while self.peek().is_ascii_digit() || self.peek() == b'_' {
                self.bump();
            }
        }
        // Exponent
        if (self.peek() == b'e' || self.peek() == b'E')
            && (self.peek2().is_ascii_digit()
                || ((self.peek2() == b'+' || self.peek2() == b'-')
                    && self.bytes.get(self.pos + 2).is_some_and(|b| b.is_ascii_digit())))
        {
            is_float = true;
            self.bump();
            if self.peek() == b'+' || self.peek() == b'-' {
                self.bump();
            }
            while self.peek().is_ascii_digit() {
                self.bump();
            }
        }

        let text: String = self.src[start..self.pos].chars().filter(|&c| c != '_').collect();
        if is_float {
            match text.parse::<f64>() {
                Ok(v) => self.tok(TokenKind::Float(v), start),
                Err(_) => self.tok(TokenKind::Error(format!("invalid float literal `{text}`")), start),
            }
        } else {
            match text.parse::<i64>() {
                Ok(v) => self.tok(TokenKind::Int(v), start),
                Err(_) => self.tok(
                    TokenKind::Error(format!("integer literal `{text}` is too large")),
                    start,
                ),
            }
        }
    }

    fn lex_string(&mut self) -> Token {
        let start = self.pos;
        self.bump(); // opening quote
        let mut parts: Vec<StrPart> = Vec::new();
        let mut cur = String::new();
        loop {
            if self.pos >= self.bytes.len() || self.peek() == b'\n' {
                return self.tok(TokenKind::Error("unterminated string literal".into()), start);
            }
            match self.peek() {
                b'"' => {
                    self.bump();
                    break;
                }
                b'\\' => {
                    self.bump();
                    let esc = self.bump();
                    match esc {
                        b'n' => cur.push('\n'),
                        b't' => cur.push('\t'),
                        b'r' => cur.push('\r'),
                        b'\\' => cur.push('\\'),
                        b'"' => cur.push('"'),
                        b'0' => cur.push('\0'),
                        b'{' => cur.push('{'),
                        b'}' => cur.push('}'),
                        _ => {
                            return self.tok(
                                TokenKind::Error(format!("unknown escape `\\{}`", esc as char)),
                                start,
                            )
                        }
                    }
                }
                b'{' => {
                    // Interpolation: collect raw expr source until balanced `}`.
                    self.bump();
                    let expr_start = self.pos;
                    let mut depth = 1u32;
                    while depth > 0 {
                        if self.pos >= self.bytes.len() || self.peek() == b'\n' {
                            return self.tok(
                                TokenKind::Error("unterminated interpolation in string".into()),
                                start,
                            );
                        }
                        match self.bump() {
                            b'{' => depth += 1,
                            b'}' => depth -= 1,
                            _ => {}
                        }
                    }
                    let expr_src = &self.src[expr_start..self.pos - 1];
                    if !cur.is_empty() {
                        parts.push(StrPart::Text(std::mem::take(&mut cur)));
                    }
                    parts.push(StrPart::Interp(expr_src.to_string()));
                }
                _ => {
                    // Copy one UTF-8 char (may be multi-byte) without splitting it.
                    let ch_len = utf8_len(self.peek());
                    let end = (self.pos + ch_len).min(self.bytes.len());
                    cur.push_str(&self.src[self.pos..end]);
                    self.pos = end;
                }
            }
        }
        if !cur.is_empty() || parts.is_empty() {
            parts.push(StrPart::Text(cur));
        }
        self.tok(TokenKind::Str(parts), start)
    }

    fn lex_operator(&mut self) -> Token {
        use TokenKind::*;
        let start = self.pos;
        let b = self.bump();
        let kind = match b {
            b'(' => LParen,
            b')' => RParen,
            b'{' => LBrace,
            b'}' => RBrace,
            b'[' => LBracket,
            b']' => RBracket,
            b',' => Comma,
            b'@' => At,
            b'.' => {
                if self.peek() == b'.' {
                    self.bump();
                    if self.peek() == b'=' {
                        self.bump();
                        DotDotEq
                    } else {
                        DotDot
                    }
                } else {
                    Dot
                }
            }
            b':' => {
                if self.peek() == b':' {
                    self.bump();
                    ColonColon
                } else {
                    Colon
                }
            }
            b'-' => {
                if self.peek() == b'>' {
                    self.bump();
                    Arrow
                } else if self.peek() == b'=' {
                    self.bump();
                    MinusEq
                } else {
                    Minus
                }
            }
            b'=' => {
                if self.peek() == b'=' {
                    self.bump();
                    EqEq
                } else if self.peek() == b'>' {
                    self.bump();
                    FatArrow
                } else {
                    Eq
                }
            }
            b'|' => {
                if self.peek() == b'>' {
                    self.bump();
                    PipeGt
                } else {
                    Pipe
                }
            }
            b'+' => {
                if self.peek() == b'=' {
                    self.bump();
                    PlusEq
                } else {
                    Plus
                }
            }
            b'*' => {
                if self.peek() == b'=' {
                    self.bump();
                    StarEq
                } else {
                    Star
                }
            }
            b'/' => {
                if self.peek() == b'=' {
                    self.bump();
                    SlashEq
                } else {
                    Slash
                }
            }
            b'%' => Percent,
            b'^' => Caret,
            b'!' => {
                if self.peek() == b'=' {
                    self.bump();
                    NotEq
                } else {
                    Bang
                }
            }
            b'<' => {
                if self.peek() == b'=' {
                    self.bump();
                    LtEq
                } else {
                    Lt
                }
            }
            b'>' => {
                if self.peek() == b'=' {
                    self.bump();
                    GtEq
                } else {
                    Gt
                }
            }
            b'?' => {
                if self.peek() == b'.' {
                    self.bump();
                    QuestionDot
                } else if self.peek() == b'?' {
                    self.bump();
                    QuestionQuestion
                } else {
                    Question
                }
            }
            _ => {
                // Consume the rest of a multibyte char so we don't split UTF-8.
                let extra = utf8_len(b).saturating_sub(1);
                for _ in 0..extra {
                    self.bump();
                }
                let text = &self.src[start..self.pos];
                Error(format!("unexpected character `{text}`"))
            }
        };
        self.tok(kind, start)
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Length in bytes of a UTF-8 sequence given its first byte.
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src).into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn empty_and_whitespace() {
        assert_eq!(kinds(""), vec![Eof]);
        assert_eq!(kinds("   \t "), vec![Eof]);
    }

    #[test]
    fn idents_and_keywords() {
        assert_eq!(
            kinds("fn main foo_bar _x x2"),
            vec![
                KwFn,
                Ident("main".into()),
                Ident("foo_bar".into()),
                Ident("_x".into()),
                Ident("x2".into()),
                Eof
            ]
        );
        assert_eq!(kinds("_"), vec![Underscore, Eof]);
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds("42 1_000 3.14 1e9 2.5e-3 0xFF 0b1010"),
            vec![
                Int(42),
                Int(1000),
                Float(3.14),
                Float(1e9),
                Float(2.5e-3),
                Int(255),
                Int(10),
                Eof
            ]
        );
    }

    #[test]
    fn range_is_not_float() {
        assert_eq!(kinds("1..5"), vec![Int(1), DotDot, Int(5), Eof]);
        assert_eq!(kinds("1..=5"), vec![Int(1), DotDotEq, Int(5), Eof]);
    }

    #[test]
    fn strings_plain_and_escapes() {
        use crate::token::StrPart;
        assert_eq!(
            kinds(r#""hello""#),
            vec![Str(vec![StrPart::Text("hello".into())]), Eof]
        );
        assert_eq!(
            kinds(r#""a\nb\t\"q\" \{x\}""#),
            vec![Str(vec![StrPart::Text("a\nb\t\"q\" {x}".into())]), Eof]
        );
        assert_eq!(
            kinds(r#""""#),
            vec![Str(vec![StrPart::Text("".into())]), Eof]
        );
    }

    #[test]
    fn string_interpolation() {
        use crate::token::StrPart;
        assert_eq!(
            kinds(r#""hi {name}, you are {age + 1}""#),
            vec![
                Str(vec![
                    StrPart::Text("hi ".into()),
                    StrPart::Interp("name".into()),
                    StrPart::Text(", you are ".into()),
                    StrPart::Interp("age + 1".into()),
                ]),
                Eof
            ]
        );
        // Nested braces inside interpolation
        assert_eq!(
            kinds(r#""{ if x { 1 } else { 2 } }""#),
            vec![
                Str(vec![StrPart::Interp(" if x { 1 } else { 2 } ".into())]),
                Eof
            ]
        );
    }

    #[test]
    fn unterminated_string_is_error() {
        let ks = kinds("\"abc\nlet");
        assert!(matches!(ks[0], Error(_)), "got {ks:?}");
        // Lexing continues after the error.
        assert!(ks.contains(&KwLet));
    }

    #[test]
    fn operators_maximal_munch() {
        assert_eq!(
            kinds("-> => == != <= >= |> ?? ?. .. ..= :: += -= *= /="),
            vec![
                Arrow, FatArrow, EqEq, NotEq, LtEq, GtEq, PipeGt, QuestionQuestion,
                QuestionDot, DotDot, DotDotEq, ColonColon, PlusEq, MinusEq, StarEq,
                SlashEq, Eof
            ]
        );
        assert_eq!(
            kinds("+-*/%^=<>!?|.,:"),
            vec![
                Plus, Minus, Star, Slash, Percent, Caret, Eq, Lt, Gt, Bang, Question,
                Pipe, Dot, Comma, Colon, Eof
            ]
        );
    }

    #[test]
    fn newlines_collapse() {
        assert_eq!(kinds("a\n\n\nb"), vec![Ident("a".into()), Newline, Ident("b".into()), Eof]);
        assert_eq!(kinds("a\r\nb"), vec![Ident("a".into()), Newline, Ident("b".into()), Eof]);
    }

    #[test]
    fn comments() {
        assert_eq!(kinds("a // comment\nb"), vec![Ident("a".into()), Newline, Ident("b".into()), Eof]);
        assert_eq!(kinds("a /* x /* nested */ y */ b"), vec![Ident("a".into()), Ident("b".into()), Eof]);
        assert_eq!(kinds("// only comment"), vec![Eof]);
    }

    #[test]
    fn error_token_recovers() {
        let ks = kinds("let $ x");
        assert_eq!(ks[0], KwLet);
        assert!(matches!(ks[1], Error(_)));
        assert_eq!(ks[2], Ident("x".into()));
    }

    #[test]
    fn unicode_in_strings_and_errors() {
        use crate::token::StrPart;
        assert_eq!(
            kinds("\"héllo ✓\""),
            vec![Str(vec![StrPart::Text("héllo ✓".into())]), Eof]
        );
        // Unknown unicode char outside string → single error token, no UTF-8 split.
        let ks = kinds("let é = 1");
        assert_eq!(ks[0], KwLet);
        assert!(matches!(&ks[1], Error(m) if m.contains('é')));
        assert_eq!(ks[2], Eq);
    }

    #[test]
    fn spans_are_correct() {
        let toks = lex("let x = 42");
        assert_eq!(toks[0].span.slice("let x = 42"), "let");
        assert_eq!(toks[1].span.slice("let x = 42"), "x");
        assert_eq!(toks[2].span.slice("let x = 42"), "=");
        assert_eq!(toks[3].span.slice("let x = 42"), "42");
    }

    #[test]
    fn realistic_program() {
        let src = r#"
pub fn main() {
    let xs = [1, 2, 3]
    var total = 0
    for x in xs {
        total += x * 2
    }
    print("total: {total}")
}
"#;
        let toks = lex(src);
        assert!(!toks.iter().any(|t| matches!(t.kind, Error(_))), "{toks:?}");
        assert_eq!(toks.last().unwrap().kind, Eof);
    }
}
