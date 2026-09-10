//! 0-based text positions compatible with LSP.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl Position {
    pub fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    pub fn contains(self, pos: Position) -> bool {
        (pos.line > self.start.line || (pos.line == self.start.line && pos.character >= self.start.character))
            && (pos.line < self.end.line || (pos.line == self.end.line && pos.character <= self.end.character))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.character)
    }
}

/// Convert a byte offset into an LSP position (UTF-8 code points as characters).
pub fn offset_to_position(text: &str, offset: usize) -> Position {
    let offset = offset.min(text.len());
    let mut line = 0u32;
    let mut character = 0u32;
    for (i, ch) in text.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += 1;
        }
    }
    Position { line, character }
}

pub fn position_to_offset(text: &str, pos: Position) -> usize {
    let mut line = 0u32;
    let mut character = 0u32;
    for (i, ch) in text.char_indices() {
        if line == pos.line && character == pos.character {
            return i;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += 1;
        }
    }
    text.len()
}

pub fn line_indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}
