//! Source positions. Byte-offset based; line/col derived on demand.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        Span { start, end }
    }

    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn slice<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start as usize..self.end as usize]
    }
}

/// 1-based line/column, for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

/// Computes line/col for byte offsets. Built once per file.
pub struct LineIndex {
    line_starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(src: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        LineIndex { line_starts }
    }

    pub fn line_col(&self, offset: u32) -> LineCol {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(l) => l,
            Err(l) => l - 1,
        };
        LineCol {
            line: line as u32 + 1,
            col: offset - self.line_starts[line] + 1,
        }
    }

    /// The full text of the (1-based) line, without trailing newline.
    pub fn line_text<'a>(&self, src: &'a str, line: u32) -> &'a str {
        let idx = (line - 1) as usize;
        let start = self.line_starts[idx] as usize;
        let end = self
            .line_starts
            .get(idx + 1)
            .map(|&e| e as usize)
            .unwrap_or(src.len());
        src[start..end].trim_end_matches('\n')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_basics() {
        let src = "ab\ncd\n\nef";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_col(0), LineCol { line: 1, col: 1 });
        assert_eq!(idx.line_col(1), LineCol { line: 1, col: 2 });
        assert_eq!(idx.line_col(3), LineCol { line: 2, col: 1 });
        assert_eq!(idx.line_col(6), LineCol { line: 3, col: 1 });
        assert_eq!(idx.line_col(7), LineCol { line: 4, col: 1 });
        assert_eq!(idx.line_text(src, 2), "cd");
        assert_eq!(idx.line_text(src, 4), "ef");
    }
}
