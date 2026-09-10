//! `textDocument/completion`.
//!
//! Two independent sources:
//!
//! * `$ref` values — driven by the document itself (which components exist),
//!   and only offered when the cursor sits on a `$ref` line whose container we
//!   recognise;
//! * keys and enum values — driven by the bundled OpenAPI JSON Schema, so
//!   user-named maps (`properties`, `paths`, `components/schemas`, …) suggest
//!   nothing instead of dumping schema keywords on you.

use openapi_core::document::Document;
use openapi_core::jsonschema::{KeyInfo, Schema};
use openapi_core::keys::ref_target_pointer;
use openapi_core::locate::pointer_at_yaml_line;
use openapi_core::pointer::{append_segment, get_at_pointer, mapping_keys};
use openapi_core::pos::{Position, Range};
use openapi_core::refs::{line_has_ref, parse_ref, ref_at_position, RefHit};
use openapi_core::{schemas, Workspace};
use serde_json::Value as JsonValue;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionItemLabelDetails, CompletionTextEdit,
    Documentation, InsertTextFormat, TextEdit,
};

use crate::server::to_lsp_range;

pub fn completion(ws: &Workspace, uri: &str, pos: Position) -> Vec<CompletionItem> {
    let Some(doc) = ws.document(uri) else {
        return Vec::new();
    };
    if !doc.version_kind.is_openapi() {
        return Vec::new();
    }
    let line = doc.text.lines().nth(pos.line as usize).unwrap_or("");
    if line_has_ref(line) {
        ref_completions(ws, doc, pos, line)
    } else {
        schema_completions(doc, pos, line)
    }
}

// ---------------------------------------------------------------- $ref values

fn ref_completions(
    ws: &Workspace,
    doc: &Document,
    pos: Position,
    line: &str,
) -> Vec<CompletionItem> {
    let hit = ref_at_position(&doc.text, pos);
    let parsed = hit
        .as_ref()
        .map(|h| h.parsed.clone())
        .unwrap_or_else(|| parse_ref(""));

    let Some(value) = doc.effective_value() else {
        return Vec::new();
    };
    // Unknown container: say nothing rather than guessing `/components/schemas`.
    let Some(target_ptr) = ref_target_pointer(doc.version_kind, &cursor_pointer(doc, pos)) else {
        return Vec::new();
    };

    let (file_prefix, target_value) = match parsed.file.as_deref() {
        Some(file) => match openapi_core::resolve::load_target(&ws.docs(), &doc.uri, Some(file)) {
            Some((_, _, loaded)) => (format!("{file}#"), loaded),
            None => return Vec::new(),
        },
        None => ("#".to_string(), value.clone()),
    };
    let Some(node) = get_at_pointer(&target_value, &target_ptr) else {
        return Vec::new();
    };

    mapping_keys(node)
        .into_iter()
        .map(|key| {
            let label = format!("{file_prefix}{target_ptr}/{key}");
            let text = ref_text(doc, hit.as_ref(), &label);
            CompletionItem {
                label: label.clone(),
                kind: Some(CompletionItemKind::REFERENCE),
                detail: Some(target_ptr.clone()),
                // Replacing the whole `$ref: …` fragment means the label alone
                // would not match what the user typed, hence filter_text.
                filter_text: Some(text.clone()),
                insert_text: Some(text.clone()),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                text_edit: replace_range(hit.as_ref(), line, pos).map(|range| {
                    CompletionTextEdit::Edit(TextEdit {
                        range: to_lsp_range(range),
                        new_text: text,
                    })
                }),
                ..Default::default()
            }
        })
        .collect()
}

/// The `$ref: "…"` fragment to write, keeping the quote style already typed.
fn ref_text(doc: &Document, hit: Option<&RefHit>, label: &str) -> String {
    if doc.is_json() {
        return format!("\"$ref\": \"{label}\"");
    }
    let quote = hit.and_then(|h| h.quote).unwrap_or('"');
    format!("$ref: {quote}{label}{quote}")
}

/// Range the completion overwrites: the `$ref` key through the end of its
/// value, so key and value stay consistent. Trailing commas are outside the
/// range already, since the value scanner stops at `,`.
fn replace_range(hit: Option<&RefHit>, line: &str, pos: Position) -> Option<Range> {
    match hit {
        Some(hit) => Some(hit.key_range),
        // `$ref` is on the line but the cursor is somewhere we cannot parse;
        // fall back to overwriting from the indent to the cursor.
        None => {
            let indent = line.chars().take_while(|c| *c == ' ').count() as u32;
            (pos.character >= indent).then(|| {
                Range::new(Position::new(pos.line, indent), Position::new(pos.line, pos.character))
            })
        }
    }
}

// ------------------------------------------------------- keys and enum values

/// Stands in for a document that has never parsed: pointer navigation then
/// relies on the schema alone, which is what we want while typing.
const NO_INSTANCE: JsonValue = JsonValue::Null;

fn schema_completions(doc: &Document, pos: Position, line: &str) -> Vec<CompletionItem> {
    let Some(root) = schemas::schema_for(doc.version_kind) else {
        return Vec::new();
    };
    let instance = doc.effective_json().unwrap_or(&NO_INSTANCE);
    let schema = Schema::new(root);
    let parent = cursor_pointer(doc, pos);
    // What the user has typed so far; the completion replaces exactly this, so
    // the surrounding indentation is left alone.
    let typed = typed_range(line, pos);

    if let Some(key) = value_position_key(line, pos.character) {
        let pointer = append_segment(&parent, &key);
        return schema
            .enum_values(&schema.schemas_at(instance, &pointer))
            .into_iter()
            .map(|value| CompletionItem {
                label: value.clone(),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                detail: Some(pointer.clone()),
                text_edit: Some(edit(typed, value.clone())),
                insert_text: Some(value),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                ..Default::default()
            })
            .collect();
    }

    let existing = doc
        .effective_value()
        .and_then(|value| get_at_pointer(value, &parent))
        .map(mapping_keys)
        .unwrap_or_default();

    schema
        .keys(&schema.schemas_at(instance, &parent))
        .into_iter()
        .filter(|key| !existing.iter().any(|e| e == &key.name))
        .map(|key| key_item(doc, typed, &parent, key))
        .collect()
}

fn key_item(doc: &Document, typed: Range, parent: &str, key: KeyInfo) -> CompletionItem {
    let label = key.name.clone();
    let text = if doc.is_json() {
        format!("\"{label}\": ")
    } else {
        format!("{label}: ")
    };
    CompletionItem {
        label: label.clone(),
        kind: Some(CompletionItemKind::PROPERTY),
        label_details: Some(CompletionItemLabelDetails {
            detail: key.ty.map(|ty| format!(" {ty}")),
            description: key.required.then(|| "required".to_string()),
        }),
        detail: Some(parent.to_string()),
        documentation: key.description.map(Documentation::String),
        deprecated: key.deprecated.then_some(true),
        // Required keys first, matching `Schema::keys` order.
        sort_text: Some(format!("{}{label}", if key.required { 0 } else { 1 })),
        text_edit: Some(edit(typed, text.clone())),
        insert_text: Some(text),
        insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
        ..Default::default()
    }
}

fn edit(range: Range, new_text: String) -> CompletionTextEdit {
    CompletionTextEdit::Edit(TextEdit {
        range: to_lsp_range(range),
        new_text,
    })
}

/// The word (plus an opening quote, in JSON) immediately before the cursor.
/// Replacing it — rather than inserting at the cursor — is what keeps the
/// line's indentation intact.
fn typed_range(line: &str, pos: Position) -> Range {
    let chars: Vec<char> = line.chars().collect();
    let mut start = (pos.character as usize).min(chars.len());
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    if start > 0 && matches!(chars[start - 1], '"' | '\'') {
        start -= 1;
    }
    Range::new(
        Position::new(pos.line, start as u32),
        Position::new(pos.line, pos.character),
    )
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '$' | '.')
}

/// `Some(key)` when the cursor sits after the `key:` of a scalar line, i.e.
/// where a value (possibly from an enum) is being typed.
fn value_position_key(line: &str, character: u32) -> Option<String> {
    let colon = line.find(':')?;
    if (character as usize) <= colon {
        return None;
    }
    let key = line[..colon].trim().trim_matches('"').trim_matches('\'');
    let key = key.strip_prefix("- ").unwrap_or(key).trim();
    (!key.is_empty()).then(|| key.to_string())
}

/// JSON Pointer of the mapping the cursor sits in. Pretty-printed JSON is
/// indented like YAML, so the same indent walk works for both.
fn cursor_pointer(doc: &Document, pos: Position) -> String {
    pointer_at_yaml_line(&doc.text, pos.line, pos.character)
}
