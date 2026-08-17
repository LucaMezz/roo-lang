use lsp_types::{Position, Range};

pub struct LineIndex {
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
