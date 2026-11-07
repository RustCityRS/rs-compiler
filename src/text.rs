//! Source coordinates independent of filenames, storage and editor protocol.
use std::ops::Range;

/// Zero-based line and UTF-16 code-unit column, as used by most editors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: usize,
    pub character: usize,
}

#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(offset, _)| offset + 1));
        Self {
            starts,
            len: text.len(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    pub fn line(&self, line: usize) -> Option<Range<usize>> {
        let start = *self.starts.get(line)?;
        Some(start..self.starts.get(line + 1).copied().unwrap_or(self.len))
    }

    pub fn position(&self, text: &str, offset: usize) -> Option<Position> {
        if offset > self.len || !text.is_char_boundary(offset) {
            return None;
        }
        let line = self.starts.partition_point(|&start| start <= offset) - 1;
        Some(Position {
            line,
            character: text[self.starts[line]..offset].encode_utf16().count(),
        })
    }

    /// Rejects positions inside a surrogate pair or past the line end.
    pub fn offset(&self, text: &str, position: Position) -> Option<usize> {
        let range = self.line(position.line)?;
        let line = text.get(range.clone())?.trim_end_matches(['\r', '\n']);
        let mut units = 0;
        for (byte, ch) in line.char_indices() {
            if units == position.character {
                return Some(range.start + byte);
            }
            units += ch.len_utf16();
        }
        (units == position.character).then_some(range.start + line.len())
    }

    /// Compiler diagnostics use one-based lines and byte columns.
    pub fn diagnostic_range(&self, text: &str, line: usize, column: usize) -> Range<usize> {
        let Some(range) = self.line(line.saturating_sub(1)) else {
            return self.len..self.len;
        };
        if column == 0 {
            return range.start
                ..range.start + text[range.clone()].trim_end_matches(['\r', '\n']).len();
        }
        let mut start = (range.start + column.saturating_sub(1)).min(range.end);
        while !text.is_char_boundary(start) {
            start -= 1;
        }
        let end = text[start..range.end]
            .chars()
            .next()
            .map_or(start, |ch| start + ch.len_utf8());
        start..end
    }
}
