//! Diagnostics shared by all compiler stages.

use crate::span::{LineIndex, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub span: Span,
    pub message: String,
    /// Optional "help: ..." line suggesting the fix.
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            span,
            message: message.into(),
            help: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Renders like:
    /// ```text
    /// examples/x.gr:3:9: error: expected `)` but found end of line
    ///     let y = (1 + 2
    ///                   ^
    ///   help: add a closing `)`
    /// ```
    pub fn render(&self, file: &str, src: &str, index: &LineIndex) -> String {
        let lc = index.line_col(self.span.start);
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let full = index.line_text(src, lc.line);
        let width = ((self.span.end.saturating_sub(self.span.start)).max(1) as usize)
            .min(full.len().saturating_sub((lc.col - 1) as usize).max(1));
        // A quoted line is there to be read. Generated or pathological source
        // can put the whole program on one of them — a deeply nested pattern
        // printed twenty kilobytes and its carets under it — so the quote is a
        // window around the span rather than the line.
        let (line_text, caret_col) = window(&full, (lc.col - 1) as usize, width);
        let caret_pad = " ".repeat(caret_col);
        let carets = "^".repeat(width.min(line_text.chars().count().saturating_sub(caret_col)).max(1));
        let mut out = format!(
            "{file}:{}:{}: {sev}: {}\n    {line_text}\n    {caret_pad}{carets}",
            lc.line, lc.col, self.message
        );
        if let Some(h) = &self.help {
            out.push_str(&format!("\n  help: {h}"));
        }
        out
    }
}

/// A readable slice of a source line, and where the caret goes in it.
///
/// Characters, not bytes: a window that split a multi-byte character would
/// render as a replacement glyph, and the column count would be wrong.
fn window(line: &str, col: usize, width: usize) -> (String, usize) {
    const MAX: usize = 120;
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= MAX {
        return (line.to_string(), col);
    }
    let want = width.min(MAX / 2);
    let start = col.saturating_sub((MAX - want) / 2);
    let end = (start + MAX).min(chars.len());
    let start = end.saturating_sub(MAX);
    let mut out = String::new();
    if start > 0 {
        out.push_str("...");
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push_str("...");
    }
    let lead = if start > 0 { 3 } else { 0 };
    (out, col.saturating_sub(start) + lead)
}
