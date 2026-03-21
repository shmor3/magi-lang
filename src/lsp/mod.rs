//! Language Server Protocol implementation for the MAGI language.
//!
//! Provides diagnostics, hover, go-to-definition, completion, signature help,
//! document symbols, code lens, workspace symbols, selection ranges, and formatting.

pub mod analysis;
pub mod code_actions;
pub mod code_lens;
pub mod completion;
pub mod definition;
pub mod document_symbols;
pub mod hover;
pub mod selection_range;
pub mod signature_help;
pub mod workspace_symbols;

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
    workspace_root: Arc<RwLock<Option<String>>>,
}

impl MagiLanguageServer {
    fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            workspace_root: Arc::new(RwLock::new(None)),
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
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Store workspace root for workspace symbol queries.
        #[allow(deprecated)] // root_uri is deprecated in favor of workspace_folders
        if let Some(root_uri) = params.root_uri {
            if let Ok(path) = root_uri.to_file_path() {
                *self.workspace_root.write().await = Some(path.to_string_lossy().to_string());
            }
        }

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
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: Some(vec![",".to_string()]),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
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
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hover::handle_hover(state, &params)
        })) {
            Ok(result) => Ok(result),
            Err(_) => Ok(None),
        }
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
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            definition::handle_goto_definition(state, &params, uri)
        })) {
            Ok(result) => Ok(result),
            Err(_) => Ok(None),
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        let docs = self.documents.read().await;
        let state = match docs.get(uri) {
            Some(s) => s,
            None => return Ok(None),
        };
        let uri_clone = uri.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            code_actions::handle_code_actions(state, &params, &uri_clone)
        })) {
            Ok(actions) if actions.is_empty() => Ok(None),
            Ok(actions) => Ok(Some(actions)),
            Err(_) => Ok(None),
        }
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let docs = self.documents.read().await;
        let state = match docs.get(uri) {
            Some(s) => s,
            None => return Ok(None),
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            completion::handle_completion(state, &params)
        })) {
            Ok(result) => Ok(Some(result)),
            Err(_) => Ok(None),
        }
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let docs = self.documents.read().await;
        let state = match docs.get(uri) {
            Some(s) => s,
            None => return Ok(None),
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            signature_help::handle_signature_help(state, &params)
        })) {
            Ok(result) => Ok(result),
            Err(_) => Ok(None),
        }
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;
        let docs = self.documents.read().await;
        let state = match docs.get(uri) {
            Some(s) => s,
            None => return Ok(None),
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            document_symbols::handle_document_symbols(state, uri)
        })) {
            Ok(result) => Ok(result),
            Err(_) => Ok(None),
        }
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;
        let docs = self.documents.read().await;
        let state = match docs.get(uri) {
            Some(s) => s,
            None => return Ok(None),
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let program = match &state.program {
                Some(p) => p,
                None => return None,
            };

            let config = crate::formatter::FormatConfig {
                indent_width: (params.options.tab_size as usize).clamp(1, 16),
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
                let last = lines.last().unwrap_or(&"");
                let utf16_len: u32 = last.chars().map(|c| c.len_utf16() as u32).sum();
                ((lines.len().saturating_sub(1)) as u32, utf16_len)
            };

            Some(vec![TextEdit {
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
            }])
        })) {
            Ok(result) => Ok(result),
            Err(_) => Ok(None),
        }
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        let uri = &params.text_document.uri;
        let docs = self.documents.read().await;
        let state = match docs.get(uri) {
            Some(s) => s,
            None => return Ok(None),
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            code_lens::handle_code_lens(state)
        })) {
            Ok(lenses) if lenses.is_empty() => Ok(None),
            Ok(lenses) => Ok(Some(lenses)),
            Err(_) => Ok(None),
        }
    }

    #[allow(deprecated)] // WorkspaceSymbolParams::query
    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let root = self.workspace_root.read().await;
        let root_path = match root.as_deref() {
            Some(r) => r.to_string(),
            None => return Ok(None),
        };
        drop(root);

        let query = params.query.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            workspace_symbols::handle_workspace_symbols(&root_path, &query)
        })) {
            Ok(symbols) if symbols.is_empty() => Ok(None),
            Ok(symbols) => Ok(Some(symbols)),
            Err(_) => Ok(None),
        }
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        let uri = &params.text_document.uri;
        let docs = self.documents.read().await;
        let state = match docs.get(uri) {
            Some(s) => s,
            None => return Ok(None),
        };
        let positions = params.positions.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            selection_range::handle_selection_ranges(state, &positions)
        })) {
            Ok(ranges) if ranges.is_empty() => Ok(None),
            Ok(ranges) => Ok(Some(ranges)),
            Err(_) => Ok(None),
        }
    }
}

/// Run the LSP server on stdin/stdout.
pub async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(MagiLanguageServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
