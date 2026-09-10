//! Locate a JSON Pointer in YAML or JSON source text.

use crate::pointer::parse_pointer;
use crate::pos::{line_indent, Position, Range};

pub fn is_json_text(text: &str) -> bool {
    matches!(text.trim_start().as_bytes().first(), Some(b'{') | Some(b'['))
}

/// Return the source range of the final key (or the whole file if pointer is empty).
pub fn locate_pointer(text: &str, pointer: &str) -> Option<Range> {
    if pointer.is_empty() || pointer == "#" {
        return Some(Range::new(Position::new(0, 0), Position::new(0, 0)));
    }
    if is_json_text(text) {
        locate_json(text, pointer)
    } else {
        locate_yaml(text, pointer)
    }
}

fn locate_yaml(text: &str, pointer: &str) -> Option<Range> {
    let segments = parse_pointer(pointer);
    let lines: Vec<&str> = text.lines().collect();
    let mut start_line = 0usize;
    let mut parent_indent: isize = -1;

    for (i, seg) in segments.iter().enumerate() {
        let found = find_yaml_child(&lines, start_line, parent_indent, seg)?;
        if i + 1 == segments.len() {
            return Some(found.range);
        }
        start_line = found.line + 1;
        parent_indent = found.indent as isize;
    }
    None
}

struct FoundKey {
    line: usize,
    indent: usize,
    range: Range,
}

fn find_yaml_child(lines: &[&str], start: usize, parent_indent: isize, key: &str) -> Option<FoundKey> {
    let mut child_indent: Option<usize> = None;
    let mut seq_index = 0usize;
    let numeric: Option<usize> = key.parse().ok();

    for (i, line) in lines.iter().enumerate().skip(start) {
        let indent = line_indent(line);
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if parent_indent >= 0 && (indent as isize) <= parent_indent {
            return None;
        }

        let expected = *child_indent.get_or_insert(indent);
        if indent < expected {
            return None;
        }
        if indent > expected {
            continue;
        }

        if let Some(n) = numeric {
            if trimmed.starts_with("- ") || trimmed == "-" {
                if seq_index == n {
                    let col = indent as u32;
                    return Some(FoundKey {
                        line: i,
                        indent,
                        range: Range::new(
                            Position::new(i as u32, col),
                            Position::new(i as u32, col + 1),
                        ),
                    });
                }
                seq_index += 1;
            }
            continue;
        }

        if let Some((parsed_key, key_col)) = yaml_key_on_line(line, indent)
            && parsed_key == *key {
                let start = Position::new(i as u32, key_col);
                let end = Position::new(i as u32, key_col + key.chars().count() as u32);
                return Some(FoundKey {
                    line: i,
                    indent,
                    range: Range::new(start, end),
                });
            }
    }
    None
}

fn yaml_key_on_line(line: &str, indent: usize) -> Option<(String, u32)> {
    let body = line.get(indent..)?;
    let body = body.strip_prefix("- ").unwrap_or(body);
    let extra = if line.trim_start().starts_with("- ") {
        2usize
    } else {
        0
    };
    let (key, rel_col) = split_yaml_key(body)?;
    Some((key, (indent + extra + rel_col) as u32))
}

/// Split `key: value` / `"key": value`. Returns (key, column of key within `body`).
fn split_yaml_key(body: &str) -> Option<(String, usize)> {
    let trimmed_start = body.chars().take_while(|c| c.is_whitespace()).count();
    let s = &body[trimmed_start..];
    if s.is_empty() || s.starts_with('#') || s.starts_with('{') || s.starts_with('[') {
        return None;
    }
    if let Some(rest) = s.strip_prefix('"') {
        let end = rest.find('"')?;
        let key = rest[..end].to_string();
        let after = rest.get(end + 1..)?.trim_start();
        if after.starts_with(':') {
            return Some((key, trimmed_start + 1));
        }
        return None;
    }
    if let Some(rest) = s.strip_prefix('\'') {
        let end = rest.find('\'')?;
        let key = rest[..end].to_string();
        let after = rest.get(end + 1..)?.trim_start();
        if after.starts_with(':') {
            return Some((key, trimmed_start + 1));
        }
        return None;
    }
    let colon = s.find(':')?;
    let key = s[..colon].trim().to_string();
    if key.is_empty() {
        return None;
    }
    Some((key, trimmed_start))
}

fn locate_json(text: &str, pointer: &str) -> Option<Range> {
    let mut p = JsonLoc { text, i: 0, line: 0, col: 0 };
    p.skip_ws();
    p.locate_value("", &parse_pointer(pointer))
}

struct JsonLoc<'a> {
    text: &'a str,
    i: usize,
    line: u32,
    col: u32,
}

impl JsonLoc<'_> {
    fn pos(&self) -> Position {
        Position::new(self.line, self.col)
    }

    fn peek(&self) -> Option<char> {
        self.text[self.i..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.i += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.col = 0;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.bump();
        }
    }

    fn locate_value(&mut self, current: &str, target: &[String]) -> Option<Range> {
        self.skip_ws();
        match self.peek()? {
            '{' => self.locate_object(current, target),
            '[' => self.locate_array(current, target),
            '"' => {
                self.parse_string()?;
                None
            }
            _ => {
                self.skip_scalar();
                None
            }
        }
    }

    fn locate_object(&mut self, current: &str, target: &[String]) -> Option<Range> {
        self.bump(); // {
        loop {
            self.skip_ws();
            if self.peek() == Some('}') {
                self.bump();
                return None;
            }
            self.skip_ws();
            let key_start = self.pos();
            let key = self.parse_string()?;
            let key_end = self.pos();
            self.skip_ws();
            if self.bump() != Some(':') {
                return None;
            }
            let child = if current.is_empty() {
                format!("/{}", crate::pointer::encode_segment(&key))
            } else {
                crate::pointer::append_segment(current, &key)
            };
            if parse_pointer(&child) == target {
                return Some(Range::new(key_start, key_end));
            }
            if let Some(hit) = self.locate_value(&child, target) {
                return Some(hit);
            }
            self.skip_ws();
            match self.peek()? {
                ',' => {
                    self.bump();
                }
                '}' => {
                    self.bump();
                    return None;
                }
                _ => return None,
            }
        }
    }

    fn locate_array(&mut self, current: &str, target: &[String]) -> Option<Range> {
        self.bump(); // [
        let mut idx = 0usize;
        loop {
            self.skip_ws();
            if self.peek() == Some(']') {
                self.bump();
                return None;
            }
            let child = crate::pointer::append_segment(current, &idx.to_string());
            if parse_pointer(&child) == target {
                let start = self.pos();
                return Some(Range::new(start, start));
            }
            if let Some(hit) = self.locate_value(&child, target) {
                return Some(hit);
            }
            self.skip_ws();
            match self.peek()? {
                ',' => {
                    self.bump();
                    idx += 1;
                }
                ']' => {
                    self.bump();
                    return None;
                }
                _ => return None,
            }
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        if self.bump() != Some('"') {
            return None;
        }
        let mut out = String::new();
        loop {
            match self.bump()? {
                '"' => return Some(out),
                '\\' => {
                    let esc = self.bump()?;
                    out.push(match esc {
                        '"' | '\\' | '/' => esc,
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => other,
                    });
                }
                ch => out.push(ch),
            }
        }
    }

    fn skip_scalar(&mut self) {
        match self.peek() {
            Some('"') => {
                let _ = self.parse_string();
            }
            _ => {
                while matches!(self.peek(), Some(c) if !matches!(c, ',' | '}' | ']' | '\n')) {
                    self.bump();
                }
            }
        }
    }
}

/// JSON Pointer of the mapping that owns `line` (YAML indent walk).
pub fn pointer_at_yaml_line(text: &str, line: u32, character: u32) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(current) = lines.get(line as usize) else {
        return String::new();
    };
    let current_indent = {
        let leading = line_indent(current);
        if current.trim().is_empty() {
            character as usize
        } else {
            leading
        }
    };

    let mut stack: Vec<(usize, String)> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        if i > line as usize {
            break;
        }
        let indent = line_indent(raw);
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        while stack.last().is_some_and(|(ind, _)| *ind >= indent) {
            stack.pop();
        }
        if i == line as usize {
            break;
        }
        if let Some((key, _)) = yaml_key_on_line(raw, indent)
            && indent < current_indent {
                stack.push((indent, key));
            }
    }
    crate::pointer::join_pointer(&stack.into_iter().map(|(_, k)| k).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
openapi: "3.0.3"
components:
  schemas:
    Pet:
      type: object
"#;

    #[test]
    fn locates_schema_key() {
        let range = locate_pointer(YAML, "/components/schemas/Pet").unwrap();
        let line = YAML.lines().nth(range.start.line as usize).unwrap();
        assert!(line.contains("Pet:"));
    }
}
