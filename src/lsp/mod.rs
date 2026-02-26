//! Language Server Protocol implementation for the MAGI language.
//!
//! Provides diagnostics, hover, go-to-definition, completion, and formatting.

pub mod analysis;
pub mod completion;
pub mod definition;
pub mod hover;

use analysis::{analyze_document, to_lsp_diagnostic, DocumentState};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

/// The MAGI language server.
pub struct MagiLanguageServer {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
}

impl MagiLanguageServer {
    fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn on_change(&self, uri: Url, text: String) {
        let (state, diagnostics) = analyze_document(&text);

        let lsp_diagnostics: Vec<Diagnostic> = diagnostics.iter().map(to_lsp_diagnostic).collect();

        self.documents.write().await.insert(uri.clone(), state);
        self.client
            .publish_diagnostics(uri, lsp_diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for MagiLanguageServer {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string(), ":".to_string()]),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "magi-lsp".to_string(),
                version: Some(crate::version::version_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(
                MessageType::INFO,
                format!("MAGI Language Server v{} started", crate::version::version_string()),
            )
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.on_change(params.text_document.uri, params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // We advertise FULL sync, so take the last change which contains the full document.
        // For FULL sync, there should be exactly one change, but last() is safe regardless.
        if let Some(change) = params.content_changes.into_iter().last() {
            self.on_change(params.text_document.uri, change.text)
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .write()
            .await
            .remove(&params.text_document.uri);
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let docs = self.documents.read().await;
        let state = match docs.get(uri) {
            Some(s) => s,
            None => return Ok(None),
        };
        Ok(hover::handle_hover(state, &params))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let docs = self.documents.read().await;
        let state = match docs.get(uri) {
            Some(s) => s,
            None => return Ok(None),
        };
        Ok(definition::handle_goto_definition(state, &params, uri))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let docs = self.documents.read().await;
        let state = match docs.get(uri) {
            Some(s) => s,
            None => return Ok(None),
        };
        Ok(Some(completion::handle_completion(state, &params)))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;
        let docs = self.documents.read().await;
        let state = match docs.get(uri) {
            Some(s) => s,
            None => return Ok(None),
        };

        let program = match &state.program {
            Some(p) => p,
            None => return Ok(None),
        };

        let config = crate::formatter::FormatConfig {
            indent_width: params.options.tab_size as usize,
            ..Default::default()
        };

        let formatted = crate::formatter::format_program(program, &config);
        // Calculate end position accounting for trailing newlines
        // (str::lines() doesn't include a trailing empty line)
        let (last_line, last_line_len) = if state.source.ends_with('\n') {
            (state.source.lines().count() as u32, 0u32)
        } else {
            let count = state.source.lines().count();
            let len = state.source.lines().last().map_or(0, |l| l.len()) as u32;
            (count.saturating_sub(1) as u32, len)
        };

        Ok(Some(vec![TextEdit {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: last_line,
                    character: last_line_len,
                },
            },
            new_text: formatted,
        }]))
    }
}

/// Run the LSP server on stdin/stdout.
pub async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(MagiLanguageServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
