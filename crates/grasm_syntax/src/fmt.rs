//! `grasm fmt` — canonical indentation normalizer.
//!
//! Token-aware but layout-preserving: re-indents every line to 4 spaces per
//! brace/bracket/paren depth, leaving line contents (including comments and
//! strings with interpolation) untouched. Idempotent by construction.

const INDENT: &str = "    ";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Code,
    Str,
    /// Inside `"...{expr}"` — tracks its own brace nesting; interpolation
    /// braces never affect code indentation depth.
    Interp(u32),
    LineComment,
    BlockComment(u32),
    /// Inside a `"""…"""` block. Every character is the program's data, so
    /// the formatter must not touch a line of it — indenting one changed what
    /// the string *was*, and `--check` then demanded the change.
    RawStr,
}

pub fn format_src(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 64);
    let mut depth: i32 = 0;
    // One indent level per *line* that leaves brackets open, not one per
    // bracket. `take([` opens two, and counting both put the elements two
    // levels in while the closing `], 1)` — whose leading closers dedent it
    // once — sat one level out. Nobody writes that, and `fmt` rewrote code
    // that was already right. Each entry is the depth the level was opened at.
    let mut levels: Vec<i32> = Vec::new();
    // Mode stack survives across lines (block comments, not strings — strings
    // cannot span lines in Grasm).
    let mut stack: Vec<Mode> = vec![Mode::Code];
    let lines: Vec<&str> = src.split_inclusive('\n').collect();

    for (index, line) in lines.iter().enumerate() {
        let line: &str = line;
        let (content, newline) = match line.strip_suffix('\n') {
            Some(c) => (c.strip_suffix('\r').unwrap_or(c), "\n"),
            None => (line, ""),
        };
        let trimmed = content.trim_start();

        // A line-spanning block comment keeps its own indentation untouched,
        // and so does a raw string — there the indentation is the program's
        // data, not its layout.
        let in_block_comment = matches!(
            stack.last(),
            Some(Mode::BlockComment(_)) | Some(Mode::RawStr)
        );

        // Leading closers dedent this line before printing.
        let mut lead_closers = 0i32;
        if !in_block_comment {
            for ch in trimmed.chars() {
                match ch {
                    '}' | ')' | ']' => lead_closers += 1,
                    _ => break,
                }
            }
        }

        if trimmed.is_empty() && !matches!(stack.last(), Some(Mode::RawStr)) {
            out.push_str(newline);
        } else if in_block_comment {
            out.push_str(content);
            out.push_str(newline);
        } else {
            // Continuation lines starting with `|` (sum-type variants, or a
            // leading pipeline op) sit one level deeper than their declaration,
            // and so does a method chain broken onto its own lines — `.trim()`
            // under the value it applies to. Without this the continuation
            // landed at the same level as the statement that owns it.
            // A comment takes the level of the code it comments on, which is
            // the next line that is not a comment or blank: one sitting inside
            // a broken-up chain or between two variants stays with them, and
            // one after the last variant, above the next declaration, goes
            // back out with it. Inheriting from the line *above* instead
            // indented the doc comment of every function that followed a sum
            // type — 384 of them across 191 packages.
            // A contract clause continues the signature above it, between the
            // `-> T` and the body's brace ([T-CONTRACT-REQUIRES]).
            let continues = |text: &str| {
                text.starts_with('|')
                    || text.starts_with('.')
                    || contract_clause(text)
            };
            let pipe_extra = if trimmed.starts_with("//") {
                let next = lines[index + 1..]
                    .iter()
                    .map(|l| l.trim())
                    .find(|l| !l.is_empty() && !l.starts_with("//"));
                i32::from(next.is_some_and(continues))
            } else {
                i32::from(continues(trimmed))
            };
            // Leading closers land this line back at the level whose opening
            // line they close.
            let after_closers = depth - lead_closers;
            let open_levels = levels.iter().filter(|&&d| d <= after_closers).count() as i32;
            let this_indent = (open_levels + pipe_extra).max(0) as usize;
            for _ in 0..this_indent {
                out.push_str(INDENT);
            }
            out.push_str(&respace(trimmed));
            out.push_str(newline);
        }

        // Scan the line to update depth + mode stack.
        // Strings reset at end of line (unterminated = lex error anyway).
        let mut chars = trimmed.chars().peekable();
        while let Some(ch) = chars.next() {
            let mode = *stack.last().unwrap();
            match mode {
                Mode::Code => match ch {
                    '"' if chars.clone().take(2).eq(['"', '"'].iter().copied()) => {
                        chars.next();
                        chars.next();
                        stack.push(Mode::RawStr);
                    }
                    '"' => stack.push(Mode::Str),
                    '/' if chars.peek() == Some(&'/') => {
                        stack.push(Mode::LineComment);
                        chars.next();
                    }
                    '/' if chars.peek() == Some(&'*') => {
                        stack.push(Mode::BlockComment(1));
                        chars.next();
                    }
                    '{' | '(' | '[' => depth += 1,
                    '}' | ')' | ']' => depth -= 1,
                    _ => {}
                },
                Mode::Interp(n) => match ch {
                    '"' if chars.clone().take(2).eq(['"', '"'].iter().copied()) => {
                        chars.next();
                        chars.next();
                        stack.push(Mode::RawStr);
                    }
                    '"' => stack.push(Mode::Str),
                    '{' => *stack.last_mut().unwrap() = Mode::Interp(n + 1),
                    '}' => {
                        if n == 0 {
                            stack.pop(); // back to the enclosing string
                        } else {
                            *stack.last_mut().unwrap() = Mode::Interp(n - 1);
                        }
                    }
                    _ => {}
                },
                Mode::Str => match ch {
                    '\\' => {
                        chars.next();
                    }
                    '"' => {
                        stack.pop();
                    }
                    '{' => stack.push(Mode::Interp(0)),
                    _ => {}
                },
                Mode::RawStr => {
                    if ch == '"' && chars.clone().take(2).eq(['"', '"'].iter().copied()) {
                        chars.next();
                        chars.next();
                        stack.pop();
                    }
                }
                Mode::LineComment => {}
                Mode::BlockComment(n) => {
                    if ch == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        if n == 1 {
                            stack.pop();
                        } else {
                            *stack.last_mut().unwrap() = Mode::BlockComment(n - 1);
                        }
                    } else if ch == '/' && chars.peek() == Some(&'*') {
                        chars.next();
                        *stack.last_mut().unwrap() = Mode::BlockComment(n + 1);
                    }
                }
            }
        }
        // Line ended: line comments and strings terminate; interp cannot span
        // lines either (it lives inside a string).
        while matches!(
            stack.last(),
            Some(Mode::LineComment | Mode::Str | Mode::Interp(_))
        ) {
            stack.pop();
        }
        // Levels whose brackets this line closed are done with; a line that
        // leaves anything newly open adds exactly one.
        if !in_block_comment {
            while levels.last().is_some_and(|&d| d > depth) {
                levels.pop();
            }
            if levels.last().copied().unwrap_or(0) < depth {
                levels.push(depth);
            }
        }
        if depth < 0 {
            depth = 0;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Spacing
// ---------------------------------------------------------------------------
//
// Re-indenting alone left `let x=1` as `let x=1`, which is not a formatter —
// it is an indenter. Canonical *spacing* needs to know where one token ends
// and the next begins, so this splits a line into pieces and rejoins them by
// rule.
//
// A piece is a `Word` (identifier, keyword, number), an `Op` (punctuation), or
// something `Opaque` — a string literal or a comment, whose bytes are the
// program's data and are copied through untouched. Whitespace is *discarded*
// and regenerated, which is what makes the pass idempotent: the output does
// not depend on the input's spacing, only on its tokens.
//
// The safety property, checked over every `.gr` file in the tree by
// `scripts/fmttest.sh`: the token stream before and after is identical. A
// formatter that changes what the compiler reads is not a formatter.

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Word,
    Op,
    Opaque,
}

struct Piece<'a> {
    kind: Kind,
    text: &'a str,
    /// Byte offset of `text` within the line, kept so the whitespace a writer
    /// put before a trailing comment can be measured and preserved.
    at: usize,
}

/// Multi-character operators, longest first — `..=` must be tried before `..`,
/// and `..` before `.`, or `0..=n` splits into three pieces that rejoin wrong.
const OPS: &[&str] = &[
    "..=", "?.", "??", "|>", "->", "=>", "::", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=",
    "&&", "||", "..",
];

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}
fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Keywords that may be followed by a parenthesis they do not *call*.
/// `print(x)` is a call and closes up; `if (a)` is a keyword and does not.
const KEYWORDS: &[&str] = &[
    "fn",
    "pub",
    "let",
    "var",
    "own",
    "inout",
    "type",
    "interface",
    "impl",
    "effect",
    "with",
    "handled_by",
    "match",
    "if",
    "else",
    "for",
    "while",
    "unsafe",
    "ref",
    "in",
    "return",
    "try",
    "true",
    "false",
    "break",
    "continue",
    "and",
    "or",
    "not",
    "import",
    "test",
];

fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

/// Split one line of code into pieces, or `None` if the line holds something
/// the splitter does not recognise — in which case the caller leaves the line
/// exactly as written. Refusing is always safe; guessing is not.
fn split_pieces(line: &str) -> Option<Vec<Piece<'_>>> {
    let b = line.as_bytes();
    let mut out: Vec<Piece> = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Comment: the rest of the line, verbatim.
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            out.push(Piece {
                kind: Kind::Opaque,
                text: &line[i..],
                at: i,
            });
            return Some(out);
        }
        // Block comment. Only one that closes on this line is handled here;
        // one that spans lines is the outer loop's business, so refuse.
        //
        // These *nest*, and stopping at the first `*/` is wrong in a way that
        // does not look wrong: `/* a /* b */ c */` would end the piece early,
        // leave ` c */` to be tokenised, and respace the terminator into
        // `c * /` — turning the rest of the file into a comment.
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            let mut j = i + 2;
            let mut depth = 1usize;
            while j + 1 < b.len() {
                if b[j] == b'/' && b[j + 1] == b'*' {
                    depth += 1;
                    j += 2;
                } else if b[j] == b'*' && b[j + 1] == b'/' {
                    depth -= 1;
                    j += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            if depth != 0 {
                return None;
            }
            out.push(Piece {
                kind: Kind::Opaque,
                text: &line[i..j],
                at: i,
            });
            i = j;
            continue;
        }
        // Raw string. If it does not close on this line the rest is data.
        if line[i..].starts_with("\"\"\"") {
            match line[i + 3..].find("\"\"\"") {
                Some(end) => {
                    let stop = i + 3 + end + 3;
                    out.push(Piece {
                        kind: Kind::Opaque,
                        text: &line[i..stop],
                        at: i,
                    });
                    i = stop;
                }
                None => {
                    out.push(Piece {
                        kind: Kind::Opaque,
                        text: &line[i..],
                        at: i,
                    });
                    return Some(out);
                }
            }
            continue;
        }
        if c == b'"' {
            let stop = scan_string(b, i)?;
            out.push(Piece {
                kind: Kind::Opaque,
                text: &line[i..stop],
                at: i,
            });
            i = stop;
            continue;
        }
        if is_ident_start(c) {
            let start = i;
            while i < b.len() && is_ident_cont(b[i]) {
                i += 1;
            }
            out.push(Piece {
                kind: Kind::Word,
                text: &line[start..i],
                at: start,
            });
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            let hex = line[i..].starts_with("0x") || line[i..].starts_with("0X");
            while i < b.len() {
                let d = b[i];
                if is_ident_cont(d) {
                    // A decimal exponent takes the sign with it: `1e-9` is one
                    // literal, and splitting it at the `-` would rejoin as
                    // `1e - 9`, which is a different program.
                    let expo = !hex && (d == b'e' || d == b'E');
                    i += 1;
                    if expo && i < b.len() && (b[i] == b'-' || b[i] == b'+') {
                        i += 1;
                    }
                    continue;
                }
                // `1.5` is one literal; `0..n` is three pieces.
                if d == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
                    i += 1;
                    continue;
                }
                break;
            }
            out.push(Piece {
                kind: Kind::Word,
                text: &line[start..i],
                at: start,
            });
            continue;
        }
        if let Some(op) = OPS.iter().find(|o| line[i..].starts_with(**o)) {
            out.push(Piece {
                kind: Kind::Op,
                text: &line[i..i + op.len()],
                at: i,
            });
            i += op.len();
            continue;
        }
        if c.is_ascii_punctuation() {
            out.push(Piece {
                kind: Kind::Op,
                text: &line[i..i + 1],
                at: i,
            });
            i += 1;
            continue;
        }
        // A byte the splitter has no rule for. Leave the whole line alone.
        return None;
    }
    Some(out)
}

/// Byte index just past the string literal starting at `i`, following escapes
/// and `{…}` interpolation (which may itself contain a string).
fn scan_string(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => return Some(j + 1),
            b'{' => {
                let mut depth = 0usize;
                j += 1;
                while j < b.len() {
                    match b[j] {
                        b'"' => j = scan_string(b, j)?,
                        b'{' => {
                            depth += 1;
                            j += 1;
                        }
                        b'}' => {
                            j += 1;
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                        }
                        b'\\' => j += 2,
                        _ => j += 1,
                    }
                }
            }
            _ => j += 1,
        }
    }
    // Unterminated: a lex error. Not the formatter's to repair.
    None
}

/// What an open brace turned out to be, which decides whether it gets inner
/// spaces. `Point{ x: 1 }` takes them and `/ {io}` does not — the corpus is
/// unanimous on both, and they are the same character.
#[derive(Clone, Copy, PartialEq)]
enum Brace {
    Struct,
    Effect,
    Map,
    Block,
}

fn respace(line: &str) -> String {
    let Some(ps) = split_pieces(line) else {
        return line.to_string();
    };
    if ps.is_empty() {
        return line.to_string();
    }
    // A `|` on a `type` line, or one opening the line, introduces a sum-type
    // variant and reads `| Circle(r: f64)`. Anywhere else it delimits closure
    // parameters and reads `|n| n * 2`. Both are the same token, so nothing
    // downstream can tell them apart — the line does.
    // A sum-type variant and a closure both open with `|` at the start of a
    // line. What separates them is what follows: a variant names a constructor
    // (`| Circle(r: f64)`), a closure names parameters (`|n| n + 100`), and
    // constructors are capitalised.
    let variant_line = (ps[0].kind == Kind::Word && ps[0].text == "type")
        || (ps[0].text == "|"
            && ps.get(1).is_some_and(|q| {
                q.kind == Kind::Word && q.text.starts_with(|c: char| c.is_ascii_uppercase())
            }));
    // `Point{ x: 1 }` closes up, but `effect Clock {`, `-> MathError {` and
    // `for p in PAYLOADS {` do not — and all four are an uppercase name
    // followed by a brace. What tells them apart is the line: where it opens a
    // declaration or a control-flow body, the brace is that body's, and the
    // name before it is being declared or tested rather than constructed.
    let body_line = ps[0].text == "}"
        || (ps[0].kind == Kind::Word
            && matches!(
                ps[0].text,
                "fn" | "pub"
                    | "type"
                    | "effect"
                    | "interface"
                    | "impl"
                    | "actor"
                    | "test"
                    | "for"
                    | "while"
                    | "if"
                    | "else"
                    | "match"
                    | "unsafe"
            ));
    // `with` is deliberately absent: `with Log handled_by LogHandler{ … } {`
    // carries a real constructor *and* a body brace on one line.
    let mut closure_open = false;
    let mut braces: Vec<Brace> = Vec::new();
    let mut depth = 0usize;
    // A `->`, `!` or `/` at the top level of the line marks a signature, which
    // is what tells `-> i64 ! MathError {` (a body) from `let c = Camera{` (a
    // constructor) when both end their line with a brace at depth zero.
    let type_marker = {
        let mut d = 0usize;
        let mut seen = false;
        for q in &ps {
            if q.kind != Kind::Op {
                continue;
            }
            match q.text {
                "(" | "[" | "{" => d += 1,
                ")" | "]" | "}" => d = d.saturating_sub(1),
                "->" | "!" | "/" if d == 0 => seen = true,
                _ => {}
            }
        }
        seen
    };

    let mut out = String::with_capacity(line.len() + 8);
    // `-` and `+` are unary when nothing before them can end a value.
    let mut prev_ends_value = false;
    let mut prev_was_unary = false;
    let mut prev_was_open_pipe = false;
    for (k, p) in ps.iter().enumerate() {
        let prev = if k == 0 { None } else { Some(&ps[k - 1]) };
        // A sign is unary only where nothing before it ends a value *and*
        // what follows can carry one. A string cannot be negated, so a line
        // continuing `… +` onto `+ "tail")` keeps its space.
        let unary = p.kind == Kind::Op
            && (p.text == "-" || p.text == "+")
            && !prev_ends_value
            && ps
                .get(k + 1)
                .is_some_and(|q| q.kind == Kind::Word || (q.kind == Kind::Op && q.text == "("));
        // A closure's parameter list opens with a `|` that cannot follow a
        // value — `|n| n * 2`, `map(|a, b| …)`, `let f = |x| x`. A `|` that
        // *does* follow a value separates alternatives instead, as in the
        // error union `! (IoError | DbError)`, and takes ordinary spacing.
        let pipe = p.kind == Kind::Op && p.text == "|" && !variant_line;
        let pipe_open = pipe && !closure_open && !prev_ends_value;
        let pipe_close = pipe && closure_open;
        let this_brace = if p.kind == Kind::Op && p.text == "{" {
            Some(brace_kind(
                prev,
                ps.get(k + 1),
                ps.get(k + 2),
                body_line,
                depth,
                k + 1 == ps.len(),
                type_marker,
            ))
        } else {
            None
        };

        // A comment at the end of a line keeps the run of spaces it was
        // written with. Columns of trailing comments are aligned on purpose —
        // opcode and byte tables in this tree are full of them — and this
        // formatter does not align, so collapsing them would only destroy
        // work it cannot redo.
        let aligned = k > 0
            && k + 1 == ps.len()
            && p.kind == Kind::Opaque
            && (p.text.starts_with("//") || p.text.starts_with("/*"))
            && {
                let before = &line[..p.at];
                before.len() - before.trim_end().len() > 1
            };
        if aligned {
            let before = &line[..p.at];
            for _ in 0..(before.len() - before.trim_end().len()) {
                out.push(' ');
            }
        } else if k > 0
            && !prev_was_unary
            && !prev_was_open_pipe
            && !pipe_close
            && needs_space(prev.unwrap(), p, &braces, this_brace)
        {
            out.push(' ');
        }
        out.push_str(p.text);

        if p.kind == Kind::Op {
            match p.text {
                "{" => {
                    braces.push(this_brace.unwrap_or(Brace::Block));
                    depth += 1;
                }
                "}" => {
                    braces.pop();
                    depth = depth.saturating_sub(1);
                }
                "(" | "[" => depth += 1,
                ")" | "]" => depth = depth.saturating_sub(1),
                "|" if pipe_open => closure_open = true,
                "|" if pipe_close => closure_open = false,
                _ => {}
            }
        }
        prev_ends_value = match p.kind {
            // `requires` and `ensures` open a contract clause, so what follows
            // starts a value: `ensures |r| …` opens closure parameters.
            Kind::Word => !matches!(
                p.text,
                "return" | "in" | "and" | "or" | "not" | "else" | "requires" | "ensures"
            ),
            Kind::Opaque => !p.text.starts_with("//") && !p.text.starts_with("/*"),
            Kind::Op => matches!(p.text, ")" | "]" | "}" | "?"),
        };
        // A unary sign binds to what follows, so nothing may separate them.
        if unary {
            prev_ends_value = false;
        }
        prev_was_unary = unary;
        prev_was_open_pipe = pipe_open;
    }
    out
}

/// What an open brace turned out to be. Four things share the character, and
/// the corpus spells each one differently:
///
/// * a body — `fn f() {`, `effect Clock {`, `for p in PAYLOADS {`
/// * a constructor — `Point{ x: 1 }`, spread included: `Stats{ ..s, n: 1 }`
/// * an effect row — `/ {io, net}`
/// * a map literal — `{"a": 1}`
///
/// A brace with something after it on the line is judged by what it *holds*,
/// which is decisive: a constructor holds `name:`, a map holds `"key":`, a
/// spread holds `..`. Only a brace that ends its line has no contents to read,
/// and that one is judged by where it sits — inside a call it is a constructor
/// (`Some(at) -> headers.push(Header{`), and at the top level of a line that
/// declares something it is a body.
fn brace_kind(
    prev: Option<&Piece>,
    next: Option<&Piece>,
    after: Option<&Piece>,
    body_line: bool,
    depth: usize,
    line_final: bool,
    type_marker: bool,
) -> Brace {
    // `/ {io, net}` — an effect row, whatever else the line is doing.
    if prev.is_some_and(|p| p.kind == Kind::Op && p.text == "/") {
        return Brace::Effect;
    }
    if line_final {
        // Inside a call or an index, a name followed by a brace constructs.
        if depth > 0 {
            return Brace::Struct;
        }
        if body_line || type_marker {
            return Brace::Block;
        }
        return match prev {
            Some(p)
                if p.kind == Kind::Word && p.text.starts_with(|c: char| c.is_ascii_uppercase()) =>
            {
                Brace::Struct
            }
            _ => Brace::Block,
        };
    }
    // `Stats{ ..s, n: 1 }`
    if next.is_some_and(|n| n.kind == Kind::Op && n.text == "..") {
        return Brace::Struct;
    }
    if after.is_some_and(|a| a.kind == Kind::Op && a.text == ":") {
        return match next {
            // `{"a": 1}` — a map literal leads with its key.
            Some(n) if n.kind == Kind::Opaque && n.text.starts_with('"') => Brace::Map,
            Some(n) if n.kind == Kind::Word => Brace::Struct,
            _ => Brace::Block,
        };
    }
    Brace::Block
}

fn needs_space(prev: &Piece, next: &Piece, braces: &[Brace], this_brace: Option<Brace>) -> bool {
    let (p, n) = (prev.text, next.text);
    let p_op = prev.kind == Kind::Op;
    let n_op = next.kind == Kind::Op;

    // A comment keeps one space before it and nothing after — it is the line's
    // last piece by construction.
    if next.kind == Kind::Opaque && (n.starts_with("//") || n.starts_with("/*")) {
        return true;
    }

    // An empty brace stays empty.
    if p_op && n_op && p == "{" && n == "}" {
        return false;
    }
    // Inner spacing belongs to the brace, and it is decided before the
    // punctuation lists below — otherwise the struct-update spread in
    // `Stats{ ..s, n: 1 }` loses its space to the rule that keeps `0..n` tight.
    if p_op && p == "{" {
        return matches!(
            braces.last(),
            Some(Brace::Struct) | Some(Brace::Block) | None
        );
    }
    if n_op && n == "}" {
        return matches!(
            braces.last(),
            Some(Brace::Struct) | Some(Brace::Block) | None
        );
    }
    // Never a space before these.
    if n_op
        && matches!(
            n,
            "," | ";" | ")" | "]" | "?" | ":" | "." | ".." | "..=" | "?." | "::"
        )
    {
        return false;
    }
    // Never a space after these.
    if p_op && matches!(p, "(" | "[" | "." | ".." | "..=" | "?." | "::" | "@") {
        return false;
    }
    // Two keywords close up against a parenthesis and the rest do not, and
    // the corpus is unanimous on every one of them: `fn(` 54 times and `fn (`
    // never (it is a function *type*, `fn(i64) -> i64`); `ref(` 145 and
    // `ref (` never. `return (`, `if (`, `and (`, `let (` and the others are
    // spaced every time they appear.
    if n_op && n == "(" && prev.kind == Kind::Word && matches!(p, "fn" | "ref") {
        return false;
    }
    // A call or an index closes up against the thing it applies to; a keyword
    // does not, because `if (a)` is not calling `if`.
    if n_op && (n == "(" || n == "[") {
        let callee = match prev.kind {
            Kind::Word => !is_keyword(p),
            // `test "reverse round-trips" (xs: [i64])` — a string is a value,
            // never the thing being called.
            Kind::Opaque => false,
            Kind::Op => matches!(p, ")" | "]"),
        };
        if callee {
            return false;
        }
    }
    // A constructor closes up against the *name* before it, and only that.
    // An anonymous struct type is written `type Row = { a: i64 }` with no name
    // to close against, and a block opening after closure parameters —
    // `xs.map(|n| {` — has none either.
    if n_op
        && n == "{"
        && this_brace == Some(Brace::Struct)
        && prev.kind == Kind::Word
        && p.starts_with(|c: char| c.is_ascii_uppercase())
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_reindent() {
        let src = "fn main() {\nlet x = 1\nif x > 0 {\nprint(\"hi\")\n}\n}\n";
        let want = "fn main() {\n    let x = 1\n    if x > 0 {\n        print(\"hi\")\n    }\n}\n";
        assert_eq!(format_src(src), want);
    }

    #[test]
    fn idempotent() {
        let src = "fn main() {\n    let xs = [\n        1,\n        2,\n    ]\n}\n";
        let once = format_src(src);
        assert_eq!(format_src(&once), once);
    }

    #[test]
    fn braces_in_strings_ignored() {
        let src = "fn main() {\nprint(\"a {x} } { b\")\nlet y = 1\n}\n";
        let got = format_src(src);
        assert!(got.contains("    print(\"a {x} } { b\")"));
        assert!(got.contains("    let y = 1"));
        assert!(got.ends_with("}\n"));
    }

    #[test]
    fn doc_comment_after_sum_type_is_not_indented() {
        let src = "type A =\n    | B\n    /// Doc for C.\n    | C\n\n/// Doc.\npub fn f() -> i64 {\n    return 1\n}\n";
        let got = format_src(src);
        assert!(got.contains("\n    /// Doc for C.\n    | C\n"), "{got}");
        assert!(got.contains("\n\n/// Doc.\npub fn f()"), "{got}");
        let chain = "fn f() {\n    let x = y\n        // why\n        .trim()\n}\n";
        assert_eq!(format_src(chain), chain);
    }

    #[test]
    fn comments_preserved() {
        let src = "// top { comment\nfn main() {\n// inner } comment\nlet x = 1 // trailing {\n}\n";
        let got = format_src(src);
        assert!(got.contains("// top { comment"));
        assert!(got.contains("    // inner } comment"));
        assert!(got.contains("    let x = 1 // trailing {"));
    }

    #[test]
    fn a_line_opening_two_brackets_is_one_level() {
        // `take([` opens a paren and a bracket. Counting both put the
        // elements two levels in while the closing `], 1)` sat one level out
        // — a shape nobody writes, which `fmt` then imposed on code that was
        // already right.
        let src = "fn main() {\n    let n = take([\n        \"a\",\n    ], 1)\n}\n";
        assert_eq!(format_src(src), src);
    }

    #[test]
    fn multiline_call_indents() {
        let src = "fn f() {\nlet x = g(\n1,\n2,\n)\n}\n";
        let want = "fn f() {\n    let x = g(\n        1,\n        2,\n    )\n}\n";
        assert_eq!(format_src(src), want);
    }

    #[test]
    fn blank_lines_stay_blank() {
        let src = "fn a() {\n}\n\nfn b() {\n}\n";
        assert_eq!(format_src(src), src);
    }

    #[test]
    fn escaped_quote_in_string() {
        let src = "fn main() {\nprint(\"say \\\"{\\\" ok\")\nlet z = 2\n}\n";
        let got = format_src(src);
        assert!(got.contains("    let z = 2"));
        assert!(got.trim_end().ends_with('}'));
    }
}

#[cfg(test)]
mod interp_regression {
    use super::format_src;

    #[test]
    fn interpolation_braces_do_not_affect_depth() {
        // Regression: `"{n}"` used to eat one indent level.
        let src = "fn f(n: i64) -> str {\n    match n {\n        0 -> \"{n}\"\n        _ -> \"x\"\n    }\n}\n";
        assert_eq!(
            format_src(src),
            src,
            "already-canonical file must be unchanged"
        );
    }

    #[test]
    fn nested_interp_braces() {
        let src = "fn f() {\n    print(\"{ if x { 1 } else { 2 } }\")\n    let y = 1\n}\n";
        assert_eq!(format_src(src), src);
    }
}

#[cfg(test)]
mod variant_indent_regression {
    use super::format_src;

    #[test]
    fn sum_type_variants_keep_one_indent() {
        let src = "type Shape =\n    | Circle(r: f64)\n    | Rect(w: f64, h: f64)\n\nfn f() {\n}\n";
        assert_eq!(format_src(src), src);
    }
}

/// A line that is a contract clause rather than a statement about a variable
/// that happens to be called `requires` or `ensures`: the word is followed by
/// something other than an assignment.
fn contract_clause(text: &str) -> bool {
    ["requires ", "ensures "].iter().any(|kw| {
        text.strip_prefix(kw).is_some_and(|rest| {
            let rest = rest.trim_start();
            !(rest.starts_with('=') && !rest.starts_with("=="))
                && !["+=", "-=", "*=", "/="].iter().any(|op| rest.starts_with(op))
        })
    })
}
