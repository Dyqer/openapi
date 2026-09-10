//! JSON-RPC surface: capabilities, document sync, and request routing.

use std::path::PathBuf;

use openapi_core::config::Settings;
use openapi_core::pos;
use openapi_core::Workspace;
use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::{completion, definition, diagnostics, hover};

pub const SERVER_NAME: &str = "openapi-lsp";

pub struct Backend {
    client: Client,
    workspace: Mutex<Workspace>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            workspace: Mutex::new(Workspace::new()),
        }
    }

    async fn publish_diagnostics(&self, uri: Url, version: Option<i32>) {
        let diags = {
            let ws = self.workspace.lock().await;
            diagnostics::diagnostics(&ws, uri.as_str())
        };
        self.client.publish_diagnostics(uri, diags, version).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        {
            let mut ws = self.workspace.lock().await;
            ws.set_roots(workspace_roots(&params));
            if let Some(options) = params.initialization_options.as_ref() {
                ws.set_settings(Settings::from_json(options));
            }
        }
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: Default::default(),
                }),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    ..Default::default()
                }),
                completion_provider: Some(CompletionOptions {
                    // `$` opens the `$ref` key, `.` and `/` a relative path.
                    trigger_characters: Some(
                        ["#", "/", "\"", "'", ":", "$", "."]
                            .iter()
                            .map(|s| s.to_string())
                            .collect(),
                    ),
                    ..Default::default()
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: SERVER_NAME.to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, format!("{SERVER_NAME} ready"))
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        let mut ws = self.workspace.lock().await;
        ws.set_settings(Settings::from_json(&params.settings));
    }

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        let mut ws = self.workspace.lock().await;
        let mut roots: Vec<PathBuf> = ws.roots().to_vec();
        for removed in &params.event.removed {
            if let Some(path) = folder_path(removed) {
                roots.retain(|root| *root != path);
            }
        }
        for added in &params.event.added {
            if let Some(path) = folder_path(added)
                && !roots.contains(&path)
            {
                roots.push(path);
            }
        }
        ws.set_roots(roots);
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        {
            let mut ws = self.workspace.lock().await;
            ws.open(doc.uri.to_string(), doc.text, doc.version);
        }
        self.publish_diagnostics(doc.uri, Some(doc.version)).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        // Full sync: the last change carries the whole document.
        let Some(text) = params.content_changes.into_iter().last().map(|c| c.text) else {
            return;
        };
        {
            let mut ws = self.workspace.lock().await;
            ws.change(uri.as_str(), text, version);
        }
        self.publish_diagnostics(uri, Some(version)).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        {
            let mut ws = self.workspace.lock().await;
            ws.close(uri.as_str());
        }
        // Clear stale squiggles for a file we no longer track.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let doc = params.text_document_position_params;
        let ws = self.workspace.lock().await;
        Ok(definition::goto_definition(
            &ws,
            doc.text_document.uri.as_str(),
            from_lsp_position(doc.position),
        ))
    }

    async fn document_link(&self, params: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        let ws = self.workspace.lock().await;
        Ok(Some(definition::document_links(
            &ws,
            params.text_document.uri.as_str(),
        )))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let doc = params.text_document_position_params;
        let ws = self.workspace.lock().await;
        Ok(hover::hover(
            &ws,
            doc.text_document.uri.as_str(),
            from_lsp_position(doc.position),
        ))
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let doc = params.text_document_position_params;
        let ws = self.workspace.lock().await;
        Ok(Some(hover::document_highlights(
            &ws,
            doc.text_document.uri.as_str(),
            from_lsp_position(doc.position),
        )))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let doc = params.text_document_position;
        let ws = self.workspace.lock().await;
        let items = completion::completion(
            &ws,
            doc.text_document.uri.as_str(),
            from_lsp_position(doc.position),
        );
        Ok(Some(CompletionResponse::Array(items)))
    }
}

/// The directories the editor has open, newest protocol field first.
fn workspace_roots(params: &InitializeParams) -> Vec<PathBuf> {
    if let Some(folders) = params.workspace_folders.as_ref() {
        let roots: Vec<PathBuf> = folders.iter().filter_map(folder_path).collect();
        if !roots.is_empty() {
            return roots;
        }
    }
    // `rootUri` is deprecated but still all some clients send.
    params
        .root_uri
        .as_ref()
        .and_then(|uri| uri.to_file_path().ok())
        .into_iter()
        .collect()
}

fn folder_path(folder: &WorkspaceFolder) -> Option<PathBuf> {
    folder.uri.to_file_path().ok()
}

pub fn from_lsp_position(pos: Position) -> pos::Position {
    pos::Position::new(pos.line, pos.character)
}

pub fn to_lsp_position(pos: pos::Position) -> Position {
    Position::new(pos.line, pos.character)
}

pub fn to_lsp_range(range: pos::Range) -> Range {
    Range::new(to_lsp_position(range.start), to_lsp_position(range.end))
}

/// Parse a document URI produced by the core layer.
pub fn to_url(uri: &str) -> Option<Url> {
    Url::parse(uri)
        .ok()
        .or_else(|| Url::from_file_path(openapi_core::document::uri_to_path(uri)?).ok())
}
