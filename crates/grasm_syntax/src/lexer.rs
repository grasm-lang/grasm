//! Hand-written lexer. Error-tolerant: unknown input becomes `Error` tokens and
//! lexing continues, so the parser and IDE layers always get a full token stream.

use crate::span::Span;
use crate::token::{StrPart, Token, TokenKind};

pub struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    /// `///` lines, in source order: (span of the line, text after the slashes).
    docs: Vec<(Span, String)>,
    /// Where an unterminated `/*` began, if one was seen.
    unterminated_comment: Option<u32>,
    /// Whether the token just emitted was `.`.
    ///
    /// After one, a number is a *tuple component*, so its own `.` starts the
    /// next component rather than a fraction. Max-munch read `t.1.0` as
    /// `t` `.` `1.0` and reported "expected an identifier but found float
    /// `1`" — so a component of a nested tuple had no unparenthesised
    /// spelling at all ([L-LIB-TUPLE]).
    after_dot: bool,
}

pub fn lex(src: &str) -> Vec<Token> {
    Lexer::new(src).run().0
}

/// Tokens plus the `///` documentation lines, which the parser attaches to
/// whichever declaration follows them.
pub fn lex_with_docs(src: &str) -> (Vec<Token>, Vec<(Span, String)>) {
    Lexer::new(src).run()
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            docs: Vec::new(),
            after_dot: false,
            unterminated_comment: None,
        }
    }

    fn run(mut self) -> (Vec<Token>, Vec<(Span, String)>) {
        let mut out = Vec::new();
        loop {
            let tok = self.next_token();
            self.after_dot = tok.kind == TokenKind::Dot;
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
        if let Some(at) = self.unterminated_comment {
            // Reported as a token so the parser surfaces it like any other
            // lexical error, with a location.
            let last = out.len().saturating_sub(1);
            out.insert(
                last,
                Token {
                    kind: TokenKind::Error(
                        "unterminated `/* ... */` comment — these nest, so an inner `/*` needs its own `*/`"
                            .into(),
                    ),
                    span: Span::new(at, at + 2),
                },
            );
        }
        (out, self.docs)
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
            // A byte order mark at the very start of the file. It is legal
            // UTF-8 and every editor that writes one expects to be ignored;
            // lexed as a character it made the first declaration
            // "unexpected character `\u{feff}`", which renders as nothing and
            // names nothing.
            if self.pos == 0 && self.bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
                self.bump();
                self.bump();
                self.bump();
                continue;
            }
            match self.peek() {
                b' ' | b'\t' | b'\r' => {
                    self.bump();
                }
                b'/' if self.peek2() == b'/' => {
                    let start = self.pos;
                    // `///` is documentation; `//` and `////` are ordinary
                    // comments, matching the usual convention.
                    let is_doc = *self.bytes.get(self.pos + 2).unwrap_or(&0) == b'/'
                        && *self.bytes.get(self.pos + 3).unwrap_or(&0) != b'/';
                    while self.pos < self.bytes.len() && self.peek() != b'\n' {
                        self.bump();
                    }
                    if is_doc {
                        let text = self.src[start + 3..self.pos]
                            .strip_prefix(' ')
                            .unwrap_or(&self.src[start + 3..self.pos])
                            .to_string();
                        self.docs
                            .push((Span::new(start as u32, self.pos as u32), text));
                    }
                }
                b'/' if self.peek2() == b'*' => {
                    let opened = self.pos;
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
                    // Running out of file is not a way to end a comment.
                    // Swallowing the rest silently made `/* outer /* inner */`
                    // — an easy thing to write, since these nest — delete
                    // everything after it and still report `ok`; if `main` was
                    // in what it ate, only `run` said anything, and it said
                    // "no `main` function" ([L-COMMENT-BLOCK]).
                    if depth > 0 {
                        self.unterminated_comment = Some(opened as u32);
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
        // Fractional part — but `1..5` is a range, not a float, and `t.1.0`
        // is two tuple components.
        if !self.after_dot && self.peek() == b'.' && self.peek2().is_ascii_digit() {
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

    /// A `"""…"""` block: every byte between the delimiters, verbatim.
    ///
    /// A newline immediately after the opening delimiter is dropped, so the
    /// content can start on its own line without a leading blank one.
    fn lex_raw_string(&mut self) -> Token {
        let start = self.pos;
        self.pos += 3;
        if self.pos < self.bytes.len() && self.peek() == b'\n' {
            self.pos += 1;
        }
        let from = self.pos;
        while self.pos < self.bytes.len() && !self.bytes[self.pos..].starts_with(b"\"\"\"") {
            self.pos += 1;
        }
        if self.pos >= self.bytes.len() {
            return self.tok(
                TokenKind::Error("unterminated `\"\"\"` string".into()),
                start,
            );
        }
        let text = String::from_utf8_lossy(&self.bytes[from..self.pos]).into_owned();
        self.pos += 3;
        self.tok(TokenKind::Str(vec![StrPart::Text(text)]), start)
    }

    fn lex_string(&mut self) -> Token {
        // `"""…"""` — a raw block. No escapes, no interpolation, newlines
        // kept. Embedding a document (JSON, CSV, a fixture) in a program
        // otherwise means a list of strings joined on "\n" with every `{` and
        // `}` escaped, which is more noise than the data.
        if self.bytes[self.pos..].starts_with(b"\"\"\"") {
            return self.lex_raw_string();
        }
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
                        // `\u{1}`, `\u{1F600}` — a character named by its
                        // code point. Without it a control character had no
                        // spelling at all: a group separator was written
                        // `"{1.to_char()}"`, an interpolation whose only job
                        // was to reach a character the source could not say.
                        b'u' => {
                            if self.peek() != b'{' {
                                return self.tok(
                                    TokenKind::Error(
                                        "`\\u` needs braces, as in `\\u{1F600}`".to_string(),
                                    ),
                                    start,
                                );
                            }
                            self.bump();
                            let digits_at = self.pos;
                            // Bounded by the string, not by the file: an
                            // escape missing its `}` used to scan past the
                            // closing quote and report the rest of the line
                            // as the code point.
                            while self.pos < self.bytes.len()
                                && self.peek() != b'}'
                                && self.peek() != b'"'
                                && self.peek() != b'\n'
                                && self.pos - digits_at < 6
                            {
                                self.bump();
                            }
                            if self.pos >= self.bytes.len() || self.peek() != b'}' {
                                return self.tok(
                                    TokenKind::Error(
                                        "unterminated `\\u{...}` escape — write up to six hex digits, then `}`"
                                            .to_string(),
                                    ),
                                    start,
                                );
                            }
                            let digits = self.src[digits_at..self.pos].to_string();
                            self.bump();
                            let ok = !digits.is_empty()
                                && digits.len() <= 6
                                && digits.chars().all(|c| c.is_ascii_hexdigit());
                            let ch = if ok {
                                u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32)
                            } else {
                                None
                            };
                            match ch {
                                Some(c) => cur.push(c),
                                // A surrogate or a value past U+10FFFF is not
                                // a character, and the string type holds
                                // characters ([L-ENCODING]).
                                None => {
                                    return self.tok(
                                        TokenKind::Error(format!(
                                            "`\\u{{{digits}}}` is not a character — write up to six hex digits naming a code point below U+110000, and not a surrogate"
                                        )),
                                        start,
                                    )
                                }
                            }
                        }
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
                                TokenKind::Error(
                                    "unterminated `{...}` interpolation in string — for a literal `{` write `\\{`"
                                        .into(),
                                ),
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
                    parts.push(StrPart::Interp {
                        src: expr_src.to_string(),
                        offset: expr_start as u32,
                    });
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
                // A backslash reaching the lexer almost always means someone
                // escaped a quote inside a `{...}` interpolation. The
                // interpolated expression is lexed on its own, so `\"` is not
                // an escape there and a plain `"` is what closes the nested
                // string. That is surprising enough to say rather than leave
                // as "unexpected character".
                if text == "\\" {
                    return self.tok(
                        Error(
                            "unexpected character `\\` — a nested string inside a \
`{...}` interpolation needs no escaping; write the quotes plainly"
                                .to_string(),
                        ),
                        start,
                    );
                }
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
            kinds("42 1_000 3.25 1e9 2.5e-3 0xFF 0b1010"),
            vec![
                Int(42),
                Int(1000),
                Float(3.25),
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
                    StrPart::Interp { src: "name".into(), offset: 5 },
                    StrPart::Text(", you are ".into()),
                    StrPart::Interp { src: "age + 1".into(), offset: 21 },
                ]),
                Eof
            ]
        );
        // Nested braces inside interpolation
        assert_eq!(
            kinds(r#""{ if x { 1 } else { 2 } }""#),
            vec![
                Str(vec![StrPart::Interp {
                    src: " if x { 1 } else { 2 } ".into(),
                    offset: 2
                }]),
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
