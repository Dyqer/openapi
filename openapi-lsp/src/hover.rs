//! `textDocument/hover` and `textDocument/documentHighlight` over `$ref` values.

use openapi_core::pointer::get_at_pointer;
use openapi_core::pos::Position;
use openapi_core::refs::ref_at_position;
use openapi_core::resolve::{is_remote, load_target};
use openapi_core::Workspace;
use tower_lsp::lsp_types::{
    DocumentHighlight, DocumentHighlightKind, Hover, HoverContents, MarkupContent, MarkupKind,
};

use crate::server::to_lsp_range;

pub fn hover(ws: &Workspace, uri: &str, pos: Position) -> Option<Hover> {
    let doc = ws.document(uri)?;
    if !doc.version_kind.is_openapi() {
        return None;
    }
    let hit = ref_at_position(&doc.text, pos)?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown(ws, uri, &hit.value, &hit.parsed),
        }),
        range: Some(to_lsp_range(hit.highlight_range)),
    })
}

pub fn document_highlights(
    ws: &Workspace,
    uri: &str,
    pos: Position,
) -> Vec<DocumentHighlight> {
    let Some(doc) = ws.document(uri) else {
        return Vec::new();
    };
    if !doc.version_kind.is_openapi() {
        return Vec::new();
    }
    ref_at_position(&doc.text, pos)
        .map(|hit| {
            vec![DocumentHighlight {
                range: to_lsp_range(hit.highlight_range),
                kind: Some(DocumentHighlightKind::TEXT),
            }]
        })
        .unwrap_or_default()
}

/// `$ref` value plus, when it resolves locally, the target node itself.
fn markdown(
    ws: &Workspace,
    uri: &str,
    raw: &str,
    parsed: &openapi_core::Ref,
) -> String {
    let mut out = format!("`$ref`: `{raw}`");
    if parsed.file.as_deref().is_some_and(is_remote) {
        return out;
    }
    let Some((_, _, value)) = load_target(&ws.docs(), uri, parsed.file.as_deref()) else {
        return out;
    };
    let node = if parsed.pointer.is_empty() {
        Some(&value)
    } else {
        get_at_pointer(&value, &parsed.pointer)
    };
    if let Some(node) = node
        && let Ok(yaml) = yaml_serde::to_string(node) {
            out.push_str("\n\n```yaml\n");
            out.push_str(yaml.trim_end());
            out.push_str("\n```");
        }
    out
}
