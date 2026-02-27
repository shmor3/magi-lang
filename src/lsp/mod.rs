//! Language Server Protocol implementation for the MAGI language.
//!
//! Provides diagnostics, hover, go-to-definition, completion, and formatting.

pub mod analysis;
pub mod completion;
pub mod definition;
pub mod hover;

use analysis::{analyze_document, to_lsp_diagnostic_with_source, DocumentState};
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
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (state, diagnostics) = analyze_document(&text);
            let lsp_diagnostics: Vec<Diagnostic> = diagnostics
                .iter()
                .map(|d| to_lsp_diagnostic_with_source(d, Some(&text)))
                .collect();
            (state, lsp_diagnostics)
        }));

        match result {
            Ok((state, lsp_diagnostics)) => {
                self.documents.write().await.insert(uri.clone(), state);
                self.client
                    .publish_diagnostics(uri, lsp_diagnostics, None)
                    .await;
            }
            Err(_) => {
                // Analysis panicked — publish a generic error diagnostic
                let diag = Diagnostic {
                    range: Range::new(Position::new(0, 0), Position::new(0, 1)),
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: "Internal analysis error".to_string(),
                    source: Some("magi".to_string()),
                    ..Default::default()
                };
                self.client
                    .publish_diagnostics(uri, vec![diag], None)
                    .await;
            }
        }
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
        } else {
            // Empty content_changes: re-analyze with current source to avoid stale state.
            let docs = self.documents.read().await;
            if let Some(state) = docs.get(&params.text_document.uri) {
                let source = state.source.clone();
                drop(docs);
                self.on_change(params.text_document.uri, source).await;
            }
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.write().await.remove(&uri);
        // Clear published diagnostics for the closed document
        self.client
            .publish_diagnostics(uri, vec![], None)
            .await;
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
        // Calculate end position of the source document.
        // LSP uses 0-based line numbers. str::lines() omits trailing empty line.
        let lines: Vec<&str> = state.source.lines().collect();
        let (last_line, last_line_len) = if state.source.ends_with('\n') {
            (lines.len() as u32, 0u32)
        } else if lines.is_empty() {
            (0u32, 0u32)
        } else {
            let last = lines.last().unwrap();
            let utf16_len: u32 = last.chars().map(|c| c.len_utf16() as u32).sum();
            ((lines.len().saturating_sub(1)) as u32, utf16_len)
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
