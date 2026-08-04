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
        let line_text = index.line_text(src, lc.line);
        let caret_pad = " ".repeat((lc.col - 1) as usize);
        let width = ((self.span.end.saturating_sub(self.span.start)).max(1) as usize)
            .min(line_text.len().saturating_sub((lc.col - 1) as usize).max(1));
        let carets = "^".repeat(width);
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
