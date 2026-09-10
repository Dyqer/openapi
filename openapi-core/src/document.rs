use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;
use yaml_serde::Value;

use crate::detect::{detect, OpenApiVersion};
use crate::pos::{offset_to_position, Range};

#[derive(Clone, Debug)]
pub struct ParseFailure {
    pub message: String,
    pub range: Range,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub uri: String,
    pub text: String,
    pub version: i32,
    pub value: Option<Value>,
    pub last_good: Option<Value>,
    /// `value` as JSON, for the JSON Schema layer. `None` when the document
    /// cannot be represented as JSON (e.g. non-string mapping keys).
    pub json: Option<JsonValue>,
    pub last_good_json: Option<JsonValue>,
    pub version_kind: OpenApiVersion,
    pub parse_error: Option<ParseFailure>,
}

impl Document {
    pub fn parse(uri: String, text: String, version: i32, previous: Option<&Document>) -> Self {
        match yaml_serde::from_str::<Value>(&text) {
            Ok(value) => {
                let version_kind = detect(&value);
                let json = serde_json::to_value(&value).ok();
                Self {
                    uri,
                    text,
                    version,
                    last_good: Some(value.clone()),
                    value: Some(value),
                    last_good_json: json.clone(),
                    json,
                    version_kind,
                    parse_error: None,
                }
            }
            Err(err) => {
                let range = err.location().map(|loc| {
                    // yaml_serde locations are 1-based.
                    let line = loc.line().saturating_sub(1) as u32;
                    let character = loc.column().saturating_sub(1) as u32;
                    Range::new(
                        crate::pos::Position::new(line, character),
                        crate::pos::Position::new(line, character + 1),
                    )
                });
                let range = range.unwrap_or_else(|| {
                    let end = offset_to_position(&text, text.len());
                    Range::new(crate::pos::Position::new(0, 0), end)
                });
                // Keep the version we knew; otherwise sniff it out of the
                // raw text so a file opened mid-edit still gets features.
                let version_kind = previous
                    .map(|p| p.version_kind)
                    .filter(|kind| kind.is_openapi())
                    .unwrap_or_else(|| crate::detect::detect_text(&text));
                Self {
                    uri,
                    text,
                    version,
                    value: None,
                    last_good: previous.and_then(|p| p.last_good.clone()),
                    json: None,
                    last_good_json: previous.and_then(|p| p.last_good_json.clone()),
                    version_kind,
                    parse_error: Some(ParseFailure {
                        message: err.to_string(),
                        range,
                    }),
                }
            }
        }
    }

    pub fn effective_value(&self) -> Option<&Value> {
        self.value.as_ref().or(self.last_good.as_ref())
    }

    /// JSON view of the document, falling back to the last parseable text so
    /// completion keeps working while the user types.
    pub fn effective_json(&self) -> Option<&JsonValue> {
        self.json.as_ref().or(self.last_good_json.as_ref())
    }

    pub fn path(&self) -> Option<PathBuf> {
        uri_to_path(&self.uri)
    }

    pub fn is_json(&self) -> bool {
        self.uri.ends_with(".json") || crate::locate::is_json_text(&self.text)
    }
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest);
    Some(PathBuf::from(decoded))
}

pub fn path_to_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

pub fn resolve_relative(base_uri: &str, relative: &str) -> Option<PathBuf> {
    let base = uri_to_path(base_uri)?;
    let parent = base.parent()?;
    Some(parent.join(relative))
}

fn percent_decode(s: &str) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(v as char);
                i += 3;
                continue;
            }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
