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
    // Mode stack survives across lines (block comments, not strings — strings
    // cannot span lines in Grasm).
    let mut stack: Vec<Mode> = vec![Mode::Code];

    for line in src.split_inclusive('\n') {
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
            // leading pipeline op) sit one level deeper than their declaration.
            let pipe_extra = i32::from(trimmed.starts_with('|'));
            let this_indent = (depth - lead_closers + pipe_extra).max(0) as usize;
            for _ in 0..this_indent {
                out.push_str(INDENT);
            }
            out.push_str(trimmed);
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
        if depth < 0 {
            depth = 0;
        }
    }
    out
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
    fn comments_preserved() {
        let src = "// top { comment\nfn main() {\n// inner } comment\nlet x = 1 // trailing {\n}\n";
        let got = format_src(src);
        assert!(got.contains("// top { comment"));
        assert!(got.contains("    // inner } comment"));
        assert!(got.contains("    let x = 1 // trailing {"));
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
        assert_eq!(format_src(src), src, "already-canonical file must be unchanged");
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
