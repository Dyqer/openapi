//! `textDocument/completion`.
//!
//! Two independent sources:
//!
//! * `$ref` values — driven by the document itself (which components exist)
//!   and by the files around it, and only offered when the cursor sits on a
//!   `$ref` line whose container we recognise;
//! * keys and enum values — driven by the bundled OpenAPI JSON Schema, so
//!   user-named maps (`properties`, `paths`, `components/schemas`, …) suggest
//!   nothing instead of dumping schema keywords on you.

use openapi_core::document::{path_to_uri, Document};
use openapi_core::jsonschema::{KeyInfo, Schema};
use openapi_core::keys::ref_target_pointer;
use openapi_core::locate::pointer_at_yaml_line;
use openapi_core::pointer::{append_segment, get_at_pointer, mapping_keys};
use openapi_core::pos::{Position, Range};
use openapi_core::refs::{line_has_ref, ref_at_position, RefHit};
use openapi_core::resolve::load_target;
use openapi_core::{schemas, Workspace};
use serde_json::Value as JsonValue;
use yaml_serde::Value as YamlValue;
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

/// Everything the `$ref` items on one line have in common.
struct RefLine<'a> {
    doc: &'a Document,
    hit: Option<&'a RefHit>,
    line: &'a str,
    pos: Position,
}

fn ref_completions(
    ws: &Workspace,
    doc: &Document,
    pos: Position,
    line: &str,
) -> Vec<CompletionItem> {
    // Unknown container: say nothing rather than guessing `/components/schemas`.
    let Some(target_ptr) = ref_target_pointer(doc.version_kind, &cursor_pointer(doc, pos)) else {
        return Vec::new();
    };
    let hit = ref_at_position(&doc.text, pos);
    let raw = hit
        .as_ref()
        .map(|h| h.value.trim().to_string())
        .unwrap_or_default();
    let ctx = RefLine {
        doc,
        hit: hit.as_ref(),
        line,
        pos,
    };

    // A leading `#` points into this document; anything else names a file.
    let local = raw.is_empty() || raw.starts_with('#');
    let mut items = Vec::new();
    if local
        && let Some(node) = doc
            .effective_value()
            .and_then(|value| get_at_pointer(value, &target_ptr))
    {
        items.extend(pointer_items(
            &ctx,
            "#",
            &target_ptr,
            node,
            Group::Local,
            "this file",
        ));
    }

    match raw.split_once('#') {
        // `file#…`: the pointers that file offers.
        Some((file, _)) if !file.is_empty() => {
            items.extend(external_items(ws, &ctx, file, &target_ptr));
        }
        // Still typing the file name — or nothing at all yet.
        _ if !raw.starts_with('#') => items.extend(neighbour_items(ws, &ctx, &target_ptr)),
        _ => {}
    }
    items
}

/// Keys of `node`, written as `{prefix}{base}/{key}` and read as `{key}`.
fn pointer_items(
    ctx: &RefLine,
    prefix: &str,
    base: &str,
    node: &YamlValue,
    group: Group,
    source: &str,
) -> Vec<CompletionItem> {
    mapping_keys(node)
        .into_iter()
        .map(|key| {
            ref_item(
                ctx,
                RefCandidate {
                    reference: format!("{prefix}{base}/{key}"),
                    name: key,
                    source: source.to_string(),
                },
                group,
            )
        })
        .collect()
}

/// Pointers inside an already-named file.
fn external_items(
    ws: &Workspace,
    ctx: &RefLine,
    file: &str,
    target_ptr: &str,
) -> Vec<CompletionItem> {
    let Some((_, _, value)) = load_target(&ws.docs(), &ctx.doc.uri, Some(file)) else {
        return Vec::new();
    };
    let (base, node) = match get_at_pointer(&value, target_ptr) {
        Some(node) if !mapping_keys(node).is_empty() => (target_ptr, node),
        // A standalone schema file is not a whole spec and has no
        // `/components/schemas`: offer its top level instead.
        _ => ("", &value),
    };
    pointer_items(
        ctx,
        &format!("{file}#"),
        base,
        node,
        Group::External,
        &file_name(file),
    )
}

/// What of a path is worth putting in front of the user: `../../shared/x.yaml`
/// reads as `x.yaml`, since the full reference is one line below it anyway.
fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// The components the files around this one offer, as the whole reference a
/// `$ref` needs: `./schemas/pet.yaml#/components/schemas/Pet`.
///
/// Only the container the cursor calls for is listed — a neighbour with no
/// `/components/schemas` contributes nothing rather than its top level, which
/// keeps unrelated YAML out of the list. Naming that file explicitly still
/// offers its top level, see `external_items`.
fn neighbour_items(ws: &Workspace, ctx: &RefLine, target_ptr: &str) -> Vec<CompletionItem> {
    let candidates =
        openapi_core::files::ref_candidates(&ctx.doc.uri, &ws.settings().ref_files, ws.roots());
    let docs = ws.docs();
    let mut items = Vec::new();
    for candidate in candidates {
        // An open file may hold unsaved edits, so prefer it over the disk.
        let uri = path_to_uri(&candidate.path);
        let names = match docs.get(&uri) {
            Some(doc) => doc
                .effective_value()
                .and_then(|value| get_at_pointer(value, target_ptr))
                .map(mapping_keys)
                .unwrap_or_default(),
            None => openapi_core::files::component_names(&candidate.path, target_ptr),
        };
        let source = file_name(&candidate.relative);
        items.extend(names.into_iter().map(|name| {
            ref_item(
                ctx,
                RefCandidate {
                    reference: format!("{}#{target_ptr}/{name}", candidate.relative),
                    name,
                    source: source.clone(),
                },
                Group::External,
            )
        }));
    }
    items
}

/// Ordering between the kinds of `$ref` target, since an empty `$ref` offers
/// all of them at once.
#[derive(Clone, Copy)]
enum Group {
    Local,
    External,
}

/// One `$ref` target: `reference` is what gets written, `name` and `source`
/// are what the user reads.
struct RefCandidate {
    /// Last pointer segment — the component's own name.
    name: String,
    /// Where it comes from, short enough to sit next to the name.
    source: String,
    /// The whole `$ref` value, relative path and pointer included.
    reference: String,
}

fn ref_item(ctx: &RefLine, candidate: RefCandidate, group: Group) -> CompletionItem {
    let RefCandidate {
        name,
        source,
        reference,
    } = candidate;
    let text = ref_text(ctx, &reference);
    CompletionItem {
        kind: Some(CompletionItemKind::REFERENCE),
        // The name leads: a deeply nested target would otherwise be all
        // `../..` by the time the list is cut off at the popup's width.
        label: name.clone(),
        label_details: Some(CompletionItemLabelDetails {
            detail: Some(format!(" {source}")),
            description: Some(reference.clone()),
        }),
        detail: Some(reference),
        // Replacing the whole `$ref: …` fragment means the label alone would
        // not match what the user typed, hence filter_text: it holds the path
        // as well, so filtering on either the name or the path works.
        filter_text: Some(text.clone()),
        sort_text: Some(format!("{}{name}", group as u8)),
        insert_text: Some(text.clone()),
        insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
        text_edit: replace_range(ctx.hit, ctx.line, ctx.pos).map(|range| {
            CompletionTextEdit::Edit(TextEdit {
                range: to_lsp_range(range),
                new_text: text,
            })
        }),
        ..Default::default()
    }
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

/// The `$ref: …` fragment to write, keeping the quote style already typed.
fn ref_text(ctx: &RefLine, value: &str) -> String {
    let (key, quote) = if ctx.doc.is_json() {
        ("\"$ref\"", '"')
    } else {
        ("$ref", ctx.hit.and_then(|h| h.quote).unwrap_or('"'))
    };
    format!("{key}: {quote}{value}{quote}")
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
    let word = typed_text(line, typed);

    schema
        .keys(&schema.schemas_at(instance, &parent))
        .into_iter()
        .filter(|key| !existing.iter().any(|e| e == &key.name))
        // A `$`-prefixed word asks for a `$` keyword. Clients fuzzy-match on
        // word characters alone, so `$r` would otherwise still bring up
        // `readOnly` and `required`; filtering here is what keeps it to `$ref`.
        .filter(|key| !word.starts_with('$') || key.name.starts_with('$'))
        .map(|key| key_item(doc, typed, &parent, key))
        .collect()
}

/// What `typed_range` covers, without the opening quote JSON adds.
fn typed_text(line: &str, typed: Range) -> String {
    let start = typed.start.character as usize;
    let len = typed.end.character.saturating_sub(typed.start.character) as usize;
    let word: String = line.chars().skip(start).take(len).collect();
    word.trim_start_matches(['"', '\'']).to_string()
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
