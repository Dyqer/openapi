//! `$ref` parsing and cursor detection.

use crate::pos::{Position, Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref {
    /// Relative or absolute file path; `None` means the current document.
    pub file: Option<String>,
    /// JSON pointer beginning with `/`, or empty for the whole file.
    pub pointer: String,
}

pub fn parse_ref(raw: &str) -> Ref {
    let s = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if s.is_empty() {
        return Ref {
            file: None,
            pointer: String::new(),
        };
    }
    if let Some(rest) = s.strip_prefix('#') {
        return Ref {
            file: None,
            pointer: normalize_pointer(rest),
        };
    }
    if let Some((file, ptr)) = s.split_once('#') {
        Ref {
            file: if file.is_empty() {
                None
            } else {
                Some(file.to_string())
            },
            pointer: normalize_pointer(ptr),
        }
    } else {
        Ref {
            file: Some(s.to_string()),
            pointer: String::new(),
        }
    }
}

fn normalize_pointer(ptr: &str) -> String {
    if ptr.is_empty() {
        String::new()
    } else if ptr.starts_with('/') {
        ptr.to_string()
    } else {
        format!("/{ptr}")
    }
}

#[derive(Clone, Debug)]
pub struct RefHit {
    pub value: String,
    pub parsed: Ref,
    /// Range of the pointer text, excluding surrounding quotes.
    pub value_range: Range,
    /// Range Zed should underline on Cmd-click, including quotes when present.
    pub highlight_range: Range,
    /// The whole `$ref: "..."` fragment, quotes included, trailing comma not.
    /// Completion replaces this so it can rewrite key and value together.
    pub key_range: Range,
    /// Quote style the user already wrote, if any.
    pub quote: Option<char>,
    pub line_text: String,
}

/// Find a `$ref` whose value contains `pos`.
pub fn ref_at_position(text: &str, pos: Position) -> Option<RefHit> {
    for (i, line) in text.lines().enumerate() {
        if i as u32 != pos.line {
            continue;
        }
        return ref_on_line(i as u32, line, pos.character);
    }
    None
}

pub fn collect_refs(text: &str) -> Vec<RefHit> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| ref_on_line(i as u32, line, u32::MAX))
        .collect()
}

fn ref_on_line(line: u32, text: &str, character: u32) -> Option<RefHit> {
    let yaml = find_ref_prefix(text)?;
    let after = &text[yaml.end..];
    let extracted = extract_ref_value(after)?;
    let value_start = (yaml.end + extracted.value_start) as u32;
    let value_end = (yaml.end + extracted.value_end) as u32;
    let highlight_start = (yaml.end + extracted.highlight_start) as u32;
    let highlight_end = (yaml.end + extracted.highlight_end) as u32;
    let value_range = Range::new(
        Position::new(line, value_start),
        Position::new(line, value_end),
    );
    let highlight_range = Range::new(
        Position::new(line, highlight_start),
        Position::new(line, highlight_end),
    );
    let key_range = Range::new(
        Position::new(line, yaml.start as u32),
        Position::new(line, highlight_end),
    );
    if character != u32::MAX && !key_range.contains(Position::new(line, character)) {
        return None;
    }
    Some(RefHit {
        parsed: parse_ref(&extracted.value),
        value: extracted.value,
        value_range,
        highlight_range,
        key_range,
        quote: extracted.quote,
        line_text: text.to_string(),
    })
}

struct ByteSpan {
    start: usize,
    end: usize,
}

fn find_ref_prefix(line: &str) -> Option<ByteSpan> {
    // JSON uses "$ref"
    if let Some(idx) = line.find("\"$ref\"") {
        let end = idx + "\"$ref\"".len();
        let rest = &line[end..];
        let colon = rest.find(':')?;
        return Some(ByteSpan {
            start: idx,
            end: end + colon + 1,
        });
    }
    if let Some(idx) = line.find("$ref:") {
        return Some(ByteSpan {
            start: idx,
            end: idx + "$ref:".len(),
        });
    }
    None
}

struct ExtractedRef {
    value: String,
    quote: Option<char>,
    value_start: usize,
    value_end: usize,
    highlight_start: usize,
    highlight_end: usize,
}

fn extract_ref_value(after: &str) -> Option<ExtractedRef> {
    let bytes = after.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return Some(ExtractedRef {
            value: String::new(),
            quote: None,
            value_start: i,
            value_end: i,
            highlight_start: i,
            highlight_end: i,
        });
    }
    match bytes[i] {
        b'"' | b'\'' => {
            let quote = bytes[i];
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != quote {
                j += 1;
            }
            let value = after[start..j.min(after.len())].to_string();
            let highlight_end = if j < bytes.len() { j + 1 } else { j };
            Some(ExtractedRef {
                value,
                quote: Some(quote as char),
                value_start: start,
                value_end: j,
                highlight_start: i,
                highlight_end,
            })
        }
        _ => {
            let start = i;
            let mut j = i;
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() && bytes[j] != b',' {
                j += 1;
            }
            Some(ExtractedRef {
                value: after[start..j].to_string(),
                quote: None,
                value_start: start,
                value_end: j,
                highlight_start: start,
                highlight_end: j,
            })
        }
    }
}

/// True when the current line is a `$ref` being edited (including empty value).
pub fn line_has_ref(line: &str) -> bool {
    line.contains("$ref:") || line.contains("\"$ref\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_and_file_refs() {
        assert_eq!(
            parse_ref("#/components/schemas/Pet"),
            Ref {
                file: None,
                pointer: "/components/schemas/Pet".into(),
            }
        );
        assert_eq!(
            parse_ref("./pet.yaml#/PetName"),
            Ref {
                file: Some("./pet.yaml".into()),
                pointer: "/PetName".into(),
            }
        );
        assert_eq!(
            parse_ref("./pet.yaml"),
            Ref {
                file: Some("./pet.yaml".into()),
                pointer: String::new(),
            }
        );
    }

    #[test]
    fn finds_ref_on_yaml_line() {
        let text = "                $ref: '#/components/schemas/Pet'\n";
        let hit = ref_at_position(text, Position::new(0, 30)).unwrap();
        assert_eq!(hit.parsed.pointer, "/components/schemas/Pet");
        let line = &text[..text.len() - 1];
        let highlighted = &line[hit.highlight_range.start.character as usize
            ..hit.highlight_range.end.character as usize];
        assert_eq!(highlighted, "'#/components/schemas/Pet'");
        let inner = &line
            [hit.value_range.start.character as usize..hit.value_range.end.character as usize];
        assert_eq!(inner, "#/components/schemas/Pet");
    }
}
