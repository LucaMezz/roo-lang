//! Converts roo's byte-offset [`ast::Span`]s into the line/UTF-16-column
//! [`lsp_types::Position`]s the LSP protocol speaks. Built once per
//! document version so every span in a batch of diagnostics converts in
//! `O(log n)` rather than rescanning the source per span.

use lsp_types::{Position, Range};

pub struct LineIndex {
    /// `line_starts[i]` is the byte offset the `i`th line begins at;
    /// always starts with `0`.
    line_starts: Vec<usize>,
    source_len: usize,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            line_starts,
            source_len: source.len(),
        }
    }

    /// Converts a byte offset into a `line`/UTF-16 `character`
    /// [`Position`]. Clamps to the end of the document rather than
    /// panicking on an out-of-range offset -- a diagnostic computed
    /// against a slightly stale document version shouldn't be able to
    /// crash the server.
    pub fn position(&self, source: &str, offset: usize) -> Position {
        let offset = offset.min(self.source_len);
        let line = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next_line) => next_line - 1,
        };
        let line_start = self.line_starts[line];
        let utf16_col = source[line_start..offset].encode_utf16().count();
        Position {
            line: line as u32,
            character: utf16_col as u32,
        }
    }

    pub fn range(&self, source: &str, span: ast::Span) -> Range {
        Range {
            start: self.position(source, span.start),
            end: self.position(source, span.end),
        }
    }

    /// Converts a `line`/UTF-16 `character` [`Position`] back into a
    /// byte offset -- the inverse of [`Self::position`]. Clamps an
    /// out-of-range line or character to the nearest valid offset
    /// (the end of the document, or the end of that line) rather than
    /// panicking, for the same reason `position` clamps: a stale
    /// position from a client that raced an edit shouldn't be able to
    /// crash the server.
    pub fn offset(&self, source: &str, position: Position) -> usize {
        let line = (position.line as usize).min(self.line_starts.len() - 1);
        let line_start = self.line_starts[line];
        let line_end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.source_len);
        let line_text = &source[line_start..line_end];

        let mut utf16_remaining = position.character as usize;
        let mut byte_offset = line_start;
        for ch in line_text.chars() {
            if utf16_remaining == 0 {
                break;
            }
            let units = ch.len_utf16();
            if units > utf16_remaining {
                break;
            }
            utf16_remaining -= units;
            byte_offset += ch.len_utf8();
        }
        byte_offset
    }
}
