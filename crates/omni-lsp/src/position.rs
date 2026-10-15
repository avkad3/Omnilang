//! Conversions between byte offsets and LSP positions.
//!
//! LSP positions count UTF-16 code units, while Rust strings are UTF-8. In
//! Devanagari or Tamil every character is 3 bytes but 1 UTF-16 unit, so mixing
//! the two up puts every diagnostic and edit in the wrong place.

use lsp_types::{Position, Range};

pub struct LineIndex<'a> {
    text: &'a str,
    /// Byte offset of the start of each line.
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex { text, starts }
    }

    pub fn position(&self, offset: usize) -> Position {
        let offset = offset.min(self.text.len());
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let start = self.starts[line];
        let col = self.text[start..offset].encode_utf16().count();
        Position::new(line as u32, col as u32)
    }

    pub fn range(&self, start: usize, end: usize) -> Range {
        Range::new(self.position(start), self.position(end))
    }

    /// Byte offset of an LSP position (clamped to the line and the text).
    pub fn offset(&self, pos: Position) -> usize {
        let Some(&start) = self.starts.get(pos.line as usize) else {
            return self.text.len();
        };
        let line_end = self.text[start..]
            .find('\n')
            .map_or(self.text.len(), |i| start + i);
        let mut units = 0u32;
        for (i, c) in self.text[start..line_end].char_indices() {
            if units >= pos.character {
                return start + i;
            }
            units += c.len_utf16() as u32;
        }
        line_end
    }

    pub fn end(&self) -> Position {
        self.position(self.text.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devanagari_counts_utf16_units_not_bytes() {
        let text = "अगर x\nछापो y";
        let idx = LineIndex::new(text);
        let x = text.find('x').unwrap(); // byte 10
        assert_eq!(idx.position(x), Position::new(0, 4));
        assert_eq!(idx.offset(Position::new(0, 4)), x);
        let y = text.find('y').unwrap();
        assert_eq!(idx.position(y), Position::new(1, 5));
        assert_eq!(idx.offset(Position::new(1, 5)), y);
    }

    #[test]
    fn astral_characters_take_two_units() {
        let text = "a😀b";
        let idx = LineIndex::new(text);
        assert_eq!(idx.position(text.find('b').unwrap()), Position::new(0, 3));
        assert_eq!(idx.offset(Position::new(0, 3)), text.find('b').unwrap());
    }

    #[test]
    fn out_of_range_positions_are_clamped() {
        let idx = LineIndex::new("ab\ncd");
        assert_eq!(idx.offset(Position::new(0, 99)), 2);
        assert_eq!(idx.offset(Position::new(9, 0)), 5);
        assert_eq!(idx.end(), Position::new(1, 2));
    }
}
