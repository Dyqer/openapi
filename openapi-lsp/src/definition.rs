//! `textDocument/definition` and `textDocument/documentLink` for `$ref` values.

use openapi_core::pos::Position;
use openapi_core::refs::{collect_refs, ref_at_position};
use openapi_core::resolve::{resolve_ref, RefTarget};
use openapi_core::Workspace;
use tower_lsp::lsp_types::{
    DocumentLink, GotoDefinitionResponse, LocationLink, Range as LspRange, Url,
};

use crate::server::{to_lsp_range, to_url};

pub fn goto_definition(
    ws: &Workspace,
    uri: &str,
    pos: Position,
) -> Option<GotoDefinitionResponse> {
    let hit = ref_target(ws, uri, pos)?;
    let target_uri = to_url(&hit.target.uri)?;
    let target_range = to_lsp_range(hit.target.range);
    Some(GotoDefinitionResponse::Link(vec![LocationLink {
        origin_selection_range: Some(to_lsp_range(hit.origin_range)),
        target_uri,
        target_range,
        target_selection_range: target_range,
    }]))
}

pub fn document_links(ws: &Workspace, uri: &str) -> Vec<DocumentLink> {
    let Some(doc) = ws.document(uri) else {
        return Vec::new();
    };
    if !doc.version_kind.is_openapi() {
        return Vec::new();
    }
    let docs = ws.docs();
    collect_refs(&doc.text)
        .into_iter()
        .filter_map(|hit| {
            let target = resolve_ref(&docs, uri, &hit.parsed)?;
            Some(DocumentLink {
                range: to_lsp_range(hit.highlight_range),
                target: Some(link_target(&target.uri, to_lsp_range(target.range))?),
                tooltip: Some("Go to $ref target".to_string()),
                data: None,
            })
        })
        .collect()
}

/// Resolve the `$ref` under the cursor, if any.
pub fn ref_target(ws: &Workspace, uri: &str, pos: Position) -> Option<RefTarget> {
    let doc = ws.document(uri)?;
    if !doc.version_kind.is_openapi() {
        return None;
    }
    let hit = ref_at_position(&doc.text, pos)?;
    let target = resolve_ref(&ws.docs(), uri, &hit.parsed)?;
    Some(RefTarget {
        origin_range: hit.highlight_range,
        target,
    })
}

/// Editors that open document links do not honour a range, so encode the
/// target position as the `#line,character` fragment Zed understands.
fn link_target(uri: &str, range: LspRange) -> Option<Url> {
    let base = uri.split('#').next().unwrap_or(uri);
    let mut url = to_url(base)?;
    url.set_fragment(Some(&format!(
        "{},{}",
        range.start.line + 1,
        range.start.character + 1
    )));
    Some(url)
}
