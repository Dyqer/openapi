//! `$ref` resolution: open documents first, on-disk files as a fallback.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use yaml_serde::Value;

use crate::document::{path_to_uri, resolve_relative, uri_to_path, Document};
use crate::locate::locate_pointer;
use crate::pointer::get_at_pointer;
use crate::pos::{Location, Position, Range};
use crate::refs::Ref;

/// A resolved `$ref`: where the reference was written, and where it points.
#[derive(Clone, Debug)]
pub struct RefTarget {
    /// Range of the `$ref` value in the referencing document, quotes included.
    pub origin_range: Range,
    pub target: Location,
}

/// Open documents plus an on-disk fallback. Thin read-only view over a map.
pub struct Docs<'a> {
    inner: &'a HashMap<String, Document>,
}

impl<'a> Docs<'a> {
    pub fn new(inner: &'a HashMap<String, Document>) -> Self {
        Self { inner }
    }

    pub fn get(&self, uri: &str) -> Option<&'a Document> {
        self.inner.get(uri).or_else(|| {
            let path = uri_to_path(uri)?;
            self.inner
                .values()
                .find(|d| d.path().as_deref() == Some(path.as_path()))
        })
    }
}

/// Resolve a parsed `$ref` into a source location.
pub fn resolve_ref(docs: &Docs, from_uri: &str, parsed: &Ref) -> Option<Location> {
    let (target_uri, target_text, value) = load_target(docs, from_uri, parsed.file.as_deref())?;
    if parsed.pointer.is_empty() {
        return Some(Location {
            uri: target_uri,
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
        });
    }
    get_at_pointer(&value, &parsed.pointer)?;
    let range = locate_pointer(&target_text, &parsed.pointer)
        .unwrap_or(Range::new(Position::new(0, 0), Position::new(0, 0)));
    Some(Location {
        uri: target_uri,
        range,
    })
}

/// Load the document a `$ref` points into: `None` file means the current one.
pub fn load_target(
    docs: &Docs,
    from_uri: &str,
    file: Option<&str>,
) -> Option<(String, String, Value)> {
    match file {
        None => {
            let doc = docs.get(from_uri)?;
            let value = doc.effective_value()?.clone();
            Some((from_uri.to_string(), doc.text.clone(), value))
        }
        Some(rel) => {
            if is_remote(rel) {
                return None;
            }
            let path = resolve_relative(from_uri, rel)?;
            let uri = path_to_uri(&path);
            if let Some(doc) = docs.get(&uri) {
                return Some((uri, doc.text.clone(), doc.effective_value()?.clone()));
            }
            let text = fs::read_to_string(&path).ok()?;
            let value: Value = yaml_serde::from_str(&text).ok()?;
            Some((uri, text, value))
        }
    }
}

/// `$ref` targets we never try to resolve from disk.
pub fn is_remote(file: &str) -> bool {
    file.starts_with("http://") || file.starts_with("https://")
}

pub fn read_file_document(path: &Path) -> Option<Document> {
    let text = fs::read_to_string(path).ok()?;
    Some(Document::parse(path_to_uri(path), text, 0, None))
}
