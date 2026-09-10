//! `textDocument/publishDiagnostics`: parse errors, unresolvable `$ref`s, and
//! JSON Schema violations.

use openapi_core::document::Document;
use openapi_core::jsonschema::Schema;
use openapi_core::locate::locate_pointer;
use openapi_core::pointer::get_at_pointer;
use openapi_core::pos::{Position, Range};
use openapi_core::refs::collect_refs;
use openapi_core::resolve::{is_remote, load_target};
use openapi_core::{schemas, Workspace};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range as LspRange};

use crate::server::{to_lsp_range, SERVER_NAME};

pub fn diagnostics(ws: &Workspace, uri: &str) -> Vec<Diagnostic> {
    let Some(doc) = ws.document(uri) else {
        return Vec::new();
    };
    if let Some(err) = &doc.parse_error {
        return vec![error(to_lsp_range(err.range), err.message.clone())];
    }
    if !doc.version_kind.is_openapi() {
        return Vec::new();
    }

    let mut out = unresolved_refs(ws, doc, uri);
    out.extend(schema_violations(doc));
    out
}

fn unresolved_refs(ws: &Workspace, doc: &Document, uri: &str) -> Vec<Diagnostic> {
    let docs = ws.docs();
    let mut out = Vec::new();
    for hit in collect_refs(&doc.text) {
        let parsed = &hit.parsed;
        if parsed.file.as_deref().is_some_and(is_remote) {
            continue;
        }
        match load_target(&docs, uri, parsed.file.as_deref()) {
            None => out.push(error(
                to_lsp_range(hit.value_range),
                format!("Cannot resolve $ref `{}`", hit.value),
            )),
            Some((_, _, value)) => {
                if !parsed.pointer.is_empty() && get_at_pointer(&value, &parsed.pointer).is_none() {
                    out.push(error(
                        to_lsp_range(hit.value_range),
                        format!("JSON Pointer `{}` not found", parsed.pointer),
                    ));
                }
            }
        }
    }
    out
}

/// Validate against the bundled schema for this OAS version. Only the current
/// text is checked (never `last_good`), so ranges always match what is on
/// screen; nodes we cannot locate in the source are skipped.
fn schema_violations(doc: &Document) -> Vec<Diagnostic> {
    let (Some(root), Some(instance)) = (schemas::schema_for(doc.version_kind), doc.json.as_ref())
    else {
        return Vec::new();
    };
    Schema::new(root)
        .validate(instance)
        .into_iter()
        .filter_map(|issue| {
            let range = if issue.pointer.is_empty() {
                Range::new(Position::new(0, 0), Position::new(0, 0))
            } else {
                locate_pointer(&doc.text, &issue.pointer)?
            };
            Some(error(to_lsp_range(range), issue.message))
        })
        .collect()
}

fn error(range: LspRange, message: String) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some(SERVER_NAME.to_string()),
        message,
        ..Default::default()
    }
}
