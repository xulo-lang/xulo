//! Byte-offset ⟷ (line, UTF-16 column) mapping in LSP's coordinate space.
//!
//! Text positions in the Language Server Protocol are 0-based `(line,
//! character)` where `character` counts UTF-16 code units — so a character
//! like `你` or an emoji (surrogate pair) occupies one or two units. The
//! analyzer keeps byte spans internally (they are stable and match the AST) and
//! converts to/from LSP positions through this index.

use std::ops::Range as ByteRange;

/// A 0-based text position: `line` and UTF-16 `character` column, exactly the
/// shape LSP's `Position` uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Pos {
    pub line: u32,
    pub character: u32,
}

/// A source range in LSP coordinate space (a start and an end `Pos`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Range {
    pub start: Pos,
    pub end: Pos,
}

/// Line-start tables computing UTF-16 columns without re-scanning the whole
/// source on every query.
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// Byte offset of each line's first character (line 0 first).
    line_starts: Vec<usize>,
}

impl LineIndex {
    /// Build the index for `source`. Runs the cheap scan once; reflect the
    /// source length passed by the caller (clamped lookups).
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, c) in source.char_indices() {
            if c == '\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts }
    }

    fn line_starts(&self) -> &[usize] {
        &self.line_starts
    }

    /// Byte offset → 0-based (line, UTF-16 column). `offset` need not land on a
    /// char boundary; the result is relative to the containing line.
    pub fn byte_to_position(&self, source: &str, offset: usize) -> Option<Pos> {
        let line_starts = self.line_starts();
        let line = line_starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1);
        if line >= line_starts.len() {
            return None;
        }
        let line_start = line_starts[line];
        let line_end = line_starts.get(line + 1).copied().unwrap_or(source.len());
        let line_text = &source[line_start.min(source.len())..line_end.min(source.len())];
        let in_line = offset.min(source.len()).saturating_sub(line_start);
        let character = utf16_len(&line_text[..in_line.min(line_text.len())]);
        Some(Pos {
            line: line as u32,
            character: character as u32,
        })
    }

    /// (line, UTF-16 column) → byte offset. A column that lands in the middle
    /// of a surrogate pair (no char boundary there) is clamped to the nearest
    /// char boundary.
    pub fn position_to_byte(&self, source: &str, pos: Pos) -> Option<usize> {
        let line_starts = self.line_starts();
        let line = pos.line as usize;
        if line >= line_starts.len() {
            return None;
        }
        let line_start = line_starts[line];
        let line_text = &source[line_start..];
        let mut u16_seen: u32 = 0;
        for (i, c) in line_text.char_indices() {
            if u16_seen >= pos.character {
                return Some(line_start + i);
            }
            u16_seen += c.len_utf16() as u32;
        }
        Some(line_start + line_text.len())
    }

    /// Convert a bytes `span` (from the AST or checker record) into an LSP
    /// range. Offsets outside the source are clamped.
    pub fn span_to_range(&self, source: &str, span: &ByteRange<usize>) -> Option<Range> {
        Some(Range {
            start: self.byte_to_position(source, span.start)?,
            end: self.byte_to_position(source, span.end)?,
        })
    }
}

/// Number of UTF-16 code units in a (char-boundary-delimited) byte slice.
fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}
