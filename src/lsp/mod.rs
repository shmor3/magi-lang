//! Language Server Protocol implementation for the MAGI language.
//!
//! Provides diagnostics, hover, go-to-definition, type definition, completion,
//! signature help, document symbols, code lens, workspace symbols, selection ranges,
//! formatting, range formatting, on-type formatting, document highlight, linked
//! editing ranges, document links, call hierarchy, type hierarchy, execute command,
//! and publish diagnostics.

pub mod analysis;
pub mod call_hierarchy;
pub mod code_actions;
pub mod code_lens;
pub mod completion;
pub mod definition;
pub mod document_highlight;
pub mod document_links;
pub mod document_symbols;
pub mod folding;
pub mod hover;
pub mod inlay_hints;
pub mod linked_editing;
pub mod references;
pub mod rename;
pub mod selection_range;
pub mod semantic_tokens;
pub mod server;
pub mod signature_help;
pub mod types;
pub mod workspace_symbols;

use analysis::{analyze_document, to_lsp_diagnostic_with_source, DocumentState};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use types::*;

use crate::util::{json_int, JsonValue, OrderedMap};

/// The MAGI language server.
pub struct MagiLanguageServer {
    client: server::Client,
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
    workspace_root: Arc<RwLock<Option<String>>>,
}

impl MagiLanguageServer {
    fn new(client: server::Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            workspace_root: Arc::new(RwLock::new(None)),
        }
    }

    fn on_change(&self, uri: Url, text: String) {
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
                self.documents.write().unwrap().insert(uri.clone(), state);
                let diag_json: Vec<JsonValue> =
                    lsp_diagnostics.iter().map(diagnostic_to_json).collect();
                self.client
                    .publish_diagnostics(&uri.to_string(), &diag_json, None);
            }
            Err(_) => {
                // Analysis panicked -- publish a generic error diagnostic
                let diag = Diagnostic {
                    range: Range::new(Position::new(0, 0), Position::new(0, 1)),
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: "Internal analysis error".to_string(),
                    source: Some("magi".to_string()),
                    ..Default::default()
                };
                let diag_json = vec![diagnostic_to_json(&diag)];
                self.client
                    .publish_diagnostics(&uri.to_string(), &diag_json, None);
            }
        }
    }

    fn handle_initialize(&self, params: &JsonValue) -> JsonValue {
        // Extract root_uri from params
        if let Some(root_uri_str) = params
            .as_object()
            .and_then(|o| o.get("rootUri"))
            .and_then(|v| v.as_str())
        {
            if let Ok(root_url) = Url::parse(root_uri_str) {
                if let Ok(path) = root_url.to_file_path() {
                    *self.workspace_root.write().unwrap() =
                        Some(path.to_string_lossy().to_string());
                }
            }
        }

        let capabilities = server_capabilities_to_json();
        let mut result = OrderedMap::new();
        result.insert("capabilities".into(), capabilities);
        result.insert(
            "serverInfo".into(),
            JsonValue::Object(OrderedMap::from([
                (
                    "name".into(),
                    JsonValue::String("magi-lsp".into()),
                ),
                (
                    "version".into(),
                    JsonValue::String(crate::version::version_string()),
                ),
            ])),
        );
        JsonValue::Object(result)
    }

    fn handle_initialized(&self) {
        self.client.log_message(
            3, // MessageType::INFO
            &format!(
                "MAGI Language Server v{} started",
                crate::version::version_string()
            ),
        );
    }

    fn handle_did_open(&self, params: &JsonValue) {
        if let Some(td) = params.as_object().and_then(|o| o.get("textDocument")) {
            let uri_str = td
                .as_object()
                .and_then(|o| o.get("uri"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let text = td
                .as_object()
                .and_then(|o| o.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Ok(uri) = Url::parse(uri_str) {
                self.on_change(uri, text.to_string());
            }
        }
    }

    fn handle_did_change(&self, params: &JsonValue) {
        let obj = match params.as_object() {
            Some(o) => o,
            None => return,
        };
        let uri_str = obj
            .get("textDocument")
            .and_then(|td| td.as_object())
            .and_then(|o| o.get("uri"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let uri = match Url::parse(uri_str) {
            Ok(u) => u,
            Err(_) => return,
        };

        let changes = obj
            .get("contentChanges")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if let Some(last) = changes.last() {
            let text = last
                .as_object()
                .and_then(|o| o.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            self.on_change(uri, text.to_string());
        } else {
            // Empty content_changes: re-analyze with current source to avoid stale state.
            let docs = self.documents.read().unwrap();
            if let Some(state) = docs.get(&uri) {
                let source = state.source.clone();
                drop(docs);
                self.on_change(uri, source);
            }
        }
    }

    fn handle_did_close(&self, params: &JsonValue) {
        let uri_str = params
            .as_object()
            .and_then(|o| o.get("textDocument"))
            .and_then(|td| td.as_object())
            .and_then(|o| o.get("uri"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if let Ok(uri) = Url::parse(uri_str) {
            self.documents.write().unwrap().remove(&uri);
            // Clear published diagnostics for the closed document
            self.client
                .publish_diagnostics(&uri.to_string(), &[], None);
        }
    }

    fn handle_hover(&self, params: &JsonValue) -> JsonValue {
        let (uri, hover_params) = match parse_hover_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hover::handle_hover(state, &hover_params)
        })) {
            Ok(Some(hover)) => hover_to_json(&hover),
            _ => JsonValue::Null,
        }
    }

    fn handle_goto_definition(&self, params: &JsonValue) -> JsonValue {
        let (uri, def_params) = match parse_goto_definition_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            definition::handle_goto_definition(state, &def_params, &uri)
        })) {
            Ok(Some(resp)) => goto_definition_response_to_json(&resp),
            _ => JsonValue::Null,
        }
    }

    fn handle_completion(&self, params: &JsonValue) -> JsonValue {
        let (uri, comp_params) = match parse_completion_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            completion::handle_completion(state, &comp_params)
        })) {
            Ok(result) => completion_response_to_json(&result),
            Err(_) => JsonValue::Null,
        }
    }

    fn handle_code_action(&self, params: &JsonValue) -> JsonValue {
        let (uri, ca_params) = match parse_code_action_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        let uri_clone = uri.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            code_actions::handle_code_actions(state, &ca_params, &uri_clone)
        })) {
            Ok(actions) if actions.is_empty() => JsonValue::Null,
            Ok(actions) => code_action_response_to_json(&actions),
            Err(_) => JsonValue::Null,
        }
    }

    fn handle_signature_help(&self, params: &JsonValue) -> JsonValue {
        let (uri, sh_params) = match parse_signature_help_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            signature_help::handle_signature_help(state, &sh_params)
        })) {
            Ok(Some(help)) => signature_help_to_json(&help),
            _ => JsonValue::Null,
        }
    }

    fn handle_document_symbol(&self, params: &JsonValue) -> JsonValue {
        let uri = match parse_text_document_uri(params) {
            Some(u) => u,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            document_symbols::handle_document_symbols(state, &uri)
        })) {
            Ok(Some(resp)) => document_symbol_response_to_json(&resp),
            _ => JsonValue::Null,
        }
    }

    fn handle_formatting(&self, params: &JsonValue) -> JsonValue {
        let (uri, fmt_params) = match parse_formatting_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let program = match &state.program {
                Some(p) => p,
                None => return None,
            };

            let config = crate::formatter::FormatConfig {
                indent_width: (fmt_params.options.tab_size as usize).clamp(1, 16),
                ..Default::default()
            };

            let formatted = crate::formatter::format_program(program, &config);
            // Calculate end position of the source document.
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
            Ok(Some(edits)) => text_edits_to_json(&edits),
            _ => JsonValue::Null,
        }
    }

    fn handle_code_lens(&self, params: &JsonValue) -> JsonValue {
        let uri = match parse_text_document_uri(params) {
            Some(u) => u,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            code_lens::handle_code_lens(state)
        })) {
            Ok(lenses) if lenses.is_empty() => JsonValue::Null,
            Ok(lenses) => code_lens_list_to_json(&lenses),
            Err(_) => JsonValue::Null,
        }
    }

    fn handle_folding_range(&self, params: &JsonValue) -> JsonValue {
        let uri = match parse_text_document_uri(params) {
            Some(u) => u,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            folding::handle_folding_ranges(state)
        })) {
            Ok(ranges) if ranges.is_empty() => JsonValue::Null,
            Ok(ranges) => folding_ranges_to_json(&ranges),
            Err(_) => JsonValue::Null,
        }
    }

    fn handle_selection_range(&self, params: &JsonValue) -> JsonValue {
        let (uri, positions) = match parse_selection_range_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            selection_range::handle_selection_ranges(state, &positions)
        })) {
            Ok(ranges) if ranges.is_empty() => JsonValue::Null,
            Ok(ranges) => selection_ranges_to_json(&ranges),
            Err(_) => JsonValue::Null,
        }
    }

    fn handle_document_link(&self, params: &JsonValue) -> JsonValue {
        let uri = match parse_text_document_uri(params) {
            Some(u) => u,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        let uri_clone = uri.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            document_links::handle_document_links(state, &uri_clone)
        })) {
            Ok(links) if links.is_empty() => JsonValue::Null,
            Ok(links) => document_links_to_json(&links),
            Err(_) => JsonValue::Null,
        }
    }

    fn handle_references(&self, params: &JsonValue) -> JsonValue {
        let (uri, ref_params) = match parse_reference_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        let uri_clone = uri.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            references::handle_references(state, &ref_params, &uri_clone)
        })) {
            Ok(Some(locs)) => locations_to_json(&locs),
            _ => JsonValue::Null,
        }
    }

    fn handle_rename(&self, params: &JsonValue) -> JsonValue {
        let (uri, ren_params) = match parse_rename_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        let uri_clone = uri.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rename::handle_rename(state, &ren_params, &uri_clone)
        })) {
            Ok(Some(edit)) => workspace_edit_to_json(&edit),
            _ => JsonValue::Null,
        }
    }

    fn handle_prepare_rename(&self, params: &JsonValue) -> JsonValue {
        // Simple prepare rename: return the word range at the cursor position.
        let (uri, position) = match parse_text_document_position(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            analysis::find_word_range_at_position(&state.source, position.line, position.character)
        })) {
            Ok(Some(range)) => range_to_json(&range),
            _ => JsonValue::Null,
        }
    }

    fn handle_semantic_tokens_full(&self, params: &JsonValue) -> JsonValue {
        let uri = match parse_text_document_uri(params) {
            Some(u) => u,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            semantic_tokens::handle_semantic_tokens_full(state)
        })) {
            Ok(result) => semantic_tokens_result_to_json(&result),
            Err(_) => JsonValue::Null,
        }
    }

    fn handle_inlay_hint(&self, params: &JsonValue) -> JsonValue {
        let (uri, range) = match parse_inlay_hint_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            inlay_hints::handle_inlay_hints(state, &range)
        })) {
            Ok(hints) if hints.is_empty() => JsonValue::Null,
            Ok(hints) => inlay_hints_to_json(&hints),
            Err(_) => JsonValue::Null,
        }
    }

    fn handle_linked_editing_range(&self, params: &JsonValue) -> JsonValue {
        let (uri, le_params) = match parse_linked_editing_range_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            linked_editing::handle_linked_editing_range(state, &le_params)
        })) {
            Ok(Some(ranges)) => linked_editing_ranges_to_json(&ranges),
            _ => JsonValue::Null,
        }
    }

    fn handle_prepare_call_hierarchy(&self, params: &JsonValue) -> JsonValue {
        let (uri, ch_params) = match parse_call_hierarchy_prepare_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        let uri_clone = uri.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            call_hierarchy::handle_prepare_call_hierarchy(state, &ch_params, &uri_clone)
        })) {
            Ok(Some(items)) => call_hierarchy_items_to_json(&items),
            _ => JsonValue::Null,
        }
    }

    fn handle_incoming_calls(&self, params: &JsonValue) -> JsonValue {
        let ic_params = match parse_incoming_calls_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let uri = ic_params.item.uri.clone();
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        let uri_clone = uri.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            call_hierarchy::handle_incoming_calls(state, &ic_params, &uri_clone)
        })) {
            Ok(calls) if calls.is_empty() => JsonValue::Null,
            Ok(calls) => incoming_calls_to_json(&calls),
            Err(_) => JsonValue::Null,
        }
    }

    fn handle_outgoing_calls(&self, params: &JsonValue) -> JsonValue {
        let oc_params = match parse_outgoing_calls_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let uri = oc_params.item.uri.clone();
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        let uri_clone = uri.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            call_hierarchy::handle_outgoing_calls(state, &oc_params, &uri_clone)
        })) {
            Ok(calls) if calls.is_empty() => JsonValue::Null,
            Ok(calls) => outgoing_calls_to_json(&calls),
            Err(_) => JsonValue::Null,
        }
    }

    fn handle_document_highlight(&self, params: &JsonValue) -> JsonValue {
        let (uri, tdpp) = match parse_text_document_position(params) {
            Some((u, pos)) => (
                u.clone(),
                TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: u },
                    position: pos,
                },
            ),
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            document_highlight::handle_document_highlight(state, &tdpp)
        })) {
            Ok(Some(highlights)) => document_highlights_to_json(&highlights),
            _ => JsonValue::Null,
        }
    }

    fn handle_range_formatting(&self, params: &JsonValue) -> JsonValue {
        let (uri, rf_params) = match parse_range_formatting_params(params) {
            Some(v) => v,
            None => return JsonValue::Null,
        };
        let docs = self.documents.read().unwrap();
        let state = match docs.get(&uri) {
            Some(s) => s,
            None => return JsonValue::Null,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let program = match &state.program {
                Some(p) => p,
                None => return None,
            };

            let config = crate::formatter::FormatConfig {
                indent_width: (rf_params.options.tab_size as usize).clamp(1, 16),
                ..Default::default()
            };

            // Format the entire program, then extract just the lines within the range.
            let formatted = crate::formatter::format_program(program, &config);
            let formatted_lines: Vec<&str> = formatted.lines().collect();

            let range_start = rf_params.range.start.line as usize;
            let range_end = rf_params.range.end.line as usize;

            // Extract the formatted lines for the requested range.
            let source_lines: Vec<&str> = state.source.lines().collect();
            let actual_end = range_end.min(source_lines.len().saturating_sub(1));
            let actual_end_formatted = range_end.min(formatted_lines.len().saturating_sub(1));

            // Build the replacement text from the formatted version for lines in the range.
            let formatted_range: Vec<&str> = formatted_lines
                .get(range_start..=actual_end_formatted)
                .unwrap_or(&[])
                .to_vec();

            if formatted_range.is_empty() {
                return None;
            }

            let mut new_text = formatted_range.join("\n");
            // If the original range ended with a newline (not the last line), include it.
            if actual_end < source_lines.len().saturating_sub(1)
                || state.source.ends_with('\n')
            {
                new_text.push('\n');
            }

            // Calculate the end character of the last line in the original source range.
            let last_line_len = source_lines
                .get(actual_end)
                .map(|l| l.chars().map(|c| c.len_utf16() as u32).sum())
                .unwrap_or(0);

            Some(vec![TextEdit {
                range: Range {
                    start: Position {
                        line: range_start as u32,
                        character: 0,
                    },
                    end: Position {
                        line: actual_end as u32,
                        character: last_line_len,
                    },
                },
                new_text,
            }])
        })) {
            Ok(Some(edits)) => text_edits_to_json(&edits),
            _ => JsonValue::Null,
        }
    }

    #[allow(deprecated)]
    fn handle_workspace_symbol(&self, params: &JsonValue) -> JsonValue {
        let query = params
            .as_object()
            .and_then(|o| o.get("query"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let root = self.workspace_root.read().unwrap();
        let root_path = match root.as_deref() {
            Some(r) => r.to_string(),
            None => return JsonValue::Null,
        };
        drop(root);

        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            workspace_symbols::handle_workspace_symbols(&root_path, &query)
        })) {
            Ok(symbols) if symbols.is_empty() => JsonValue::Null,
            Ok(symbols) => symbol_informations_to_json(&symbols),
            Err(_) => JsonValue::Null,
        }
    }

    fn handle_type_definition(&self, params: &JsonValue) -> JsonValue {
        // Type definition: return the definition location of the TYPE of the symbol.
        // Simplest implementation: reuse goto-definition (returns the symbol's own def).
        self.handle_goto_definition(params)
    }

    fn handle_document_color(&self, _params: &JsonValue) -> JsonValue {
        // Document color: return empty array (no color information).
        // Could be extended to detect color literals like #RGB, rgb(), etc.
        JsonValue::Array(vec![])
    }

    fn handle_on_type_formatting(&self, _params: &JsonValue) -> JsonValue {
        // On-type formatting: return empty edits (no-op).
        JsonValue::Array(vec![])
    }

    fn handle_execute_command(&self, _params: &JsonValue) -> JsonValue {
        // Execute command: return null for any command.
        JsonValue::Null
    }

    fn handle_prepare_type_hierarchy(&self, _params: &JsonValue) -> JsonValue {
        // Prepare type hierarchy: return null (no items).
        JsonValue::Null
    }

    fn handle_type_hierarchy_subtypes(&self, _params: &JsonValue) -> JsonValue {
        // Type hierarchy subtypes: return empty array.
        JsonValue::Array(vec![])
    }

    fn handle_type_hierarchy_supertypes(&self, _params: &JsonValue) -> JsonValue {
        // Type hierarchy supertypes: return empty array.
        JsonValue::Array(vec![])
    }
}

/// Run the LSP server on stdin/stdout.
pub fn run_server() {
    let mut server = server::LspServer::new();
    let ls = Arc::new(MagiLanguageServer::new(server.client.clone()));

    // initialize
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "initialize",
            Box::new(move |params| ls.handle_initialize(params)),
        );
    }

    // initialized (notification)
    {
        let ls = Arc::clone(&ls);
        server.on_notification(
            "initialized",
            Box::new(move |_params| ls.handle_initialized()),
        );
    }

    // textDocument/didOpen (notification)
    {
        let ls = Arc::clone(&ls);
        server.on_notification(
            "textDocument/didOpen",
            Box::new(move |params| ls.handle_did_open(params)),
        );
    }

    // textDocument/didChange (notification)
    {
        let ls = Arc::clone(&ls);
        server.on_notification(
            "textDocument/didChange",
            Box::new(move |params| ls.handle_did_change(params)),
        );
    }

    // textDocument/didClose (notification)
    {
        let ls = Arc::clone(&ls);
        server.on_notification(
            "textDocument/didClose",
            Box::new(move |params| ls.handle_did_close(params)),
        );
    }

    // textDocument/hover
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/hover",
            Box::new(move |params| ls.handle_hover(params)),
        );
    }

    // textDocument/definition
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/definition",
            Box::new(move |params| ls.handle_goto_definition(params)),
        );
    }

    // textDocument/completion
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/completion",
            Box::new(move |params| ls.handle_completion(params)),
        );
    }

    // textDocument/codeAction
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/codeAction",
            Box::new(move |params| ls.handle_code_action(params)),
        );
    }

    // textDocument/signatureHelp
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/signatureHelp",
            Box::new(move |params| ls.handle_signature_help(params)),
        );
    }

    // textDocument/documentSymbol
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/documentSymbol",
            Box::new(move |params| ls.handle_document_symbol(params)),
        );
    }

    // textDocument/formatting
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/formatting",
            Box::new(move |params| ls.handle_formatting(params)),
        );
    }

    // textDocument/codeLens
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/codeLens",
            Box::new(move |params| ls.handle_code_lens(params)),
        );
    }

    // textDocument/foldingRange
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/foldingRange",
            Box::new(move |params| ls.handle_folding_range(params)),
        );
    }

    // textDocument/selectionRange
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/selectionRange",
            Box::new(move |params| ls.handle_selection_range(params)),
        );
    }

    // textDocument/documentLink
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/documentLink",
            Box::new(move |params| ls.handle_document_link(params)),
        );
    }

    // textDocument/rename
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/rename",
            Box::new(move |params| ls.handle_rename(params)),
        );
    }

    // textDocument/prepareRename
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/prepareRename",
            Box::new(move |params| ls.handle_prepare_rename(params)),
        );
    }

    // textDocument/references
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/references",
            Box::new(move |params| ls.handle_references(params)),
        );
    }

    // textDocument/semanticTokens/full
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/semanticTokens/full",
            Box::new(move |params| ls.handle_semantic_tokens_full(params)),
        );
    }

    // textDocument/inlayHint
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/inlayHint",
            Box::new(move |params| ls.handle_inlay_hint(params)),
        );
    }

    // textDocument/linkedEditingRange
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/linkedEditingRange",
            Box::new(move |params| ls.handle_linked_editing_range(params)),
        );
    }

    // callHierarchy/incomingCalls
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "callHierarchy/incomingCalls",
            Box::new(move |params| ls.handle_incoming_calls(params)),
        );
    }

    // callHierarchy/outgoingCalls
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "callHierarchy/outgoingCalls",
            Box::new(move |params| ls.handle_outgoing_calls(params)),
        );
    }

    // textDocument/prepareCallHierarchy
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/prepareCallHierarchy",
            Box::new(move |params| ls.handle_prepare_call_hierarchy(params)),
        );
    }

    // textDocument/documentHighlight
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/documentHighlight",
            Box::new(move |params| ls.handle_document_highlight(params)),
        );
    }

    // textDocument/rangeFormatting
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/rangeFormatting",
            Box::new(move |params| ls.handle_range_formatting(params)),
        );
    }

    // workspace/symbol
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "workspace/symbol",
            Box::new(move |params| ls.handle_workspace_symbol(params)),
        );
    }

    // textDocument/typeDefinition
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/typeDefinition",
            Box::new(move |params| ls.handle_type_definition(params)),
        );
    }

    // textDocument/documentColor
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/documentColor",
            Box::new(move |_params| ls.handle_document_color(_params)),
        );
    }

    // textDocument/onTypeFormatting
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/onTypeFormatting",
            Box::new(move |params| ls.handle_on_type_formatting(params)),
        );
    }

    // workspace/executeCommand
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "workspace/executeCommand",
            Box::new(move |params| ls.handle_execute_command(params)),
        );
    }

    // textDocument/prepareTypeHierarchy
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "textDocument/prepareTypeHierarchy",
            Box::new(move |params| ls.handle_prepare_type_hierarchy(params)),
        );
    }

    // typeHierarchy/subtypes
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "typeHierarchy/subtypes",
            Box::new(move |params| ls.handle_type_hierarchy_subtypes(params)),
        );
    }

    // typeHierarchy/supertypes
    {
        let ls = Arc::clone(&ls);
        server.on_request(
            "typeHierarchy/supertypes",
            Box::new(move |params| ls.handle_type_hierarchy_supertypes(params)),
        );
    }

    server.run();
}

// JSON param parsing helpers

fn get_str<'a>(obj: &'a OrderedMap<String, JsonValue>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(|v| v.as_str())
}

fn get_u32(obj: &OrderedMap<String, JsonValue>, key: &str) -> Option<u32> {
    obj.get(key).and_then(|v| v.as_u64()).map(|n| n as u32)
}

fn parse_position(val: &JsonValue) -> Option<Position> {
    let obj = val.as_object()?;
    let line = get_u32(obj, "line")?;
    let character = get_u32(obj, "character")?;
    Some(Position::new(line, character))
}

fn parse_range(val: &JsonValue) -> Option<Range> {
    let obj = val.as_object()?;
    let start = parse_position(obj.get("start")?)?;
    let end = parse_position(obj.get("end")?)?;
    Some(Range::new(start, end))
}

fn parse_text_document_uri(params: &JsonValue) -> Option<Url> {
    let obj = params.as_object()?;
    let td = obj.get("textDocument")?.as_object()?;
    let uri_str = get_str(td, "uri")?;
    Url::parse(uri_str).ok()
}

fn parse_text_document_position(params: &JsonValue) -> Option<(Url, Position)> {
    let obj = params.as_object()?;
    let td_obj = obj
        .get("textDocument")
        .and_then(|v| v.as_object());
    let pos_val = obj.get("position");
    if let (Some(td), Some(pos)) = (td_obj, pos_val) {
        let uri = Url::parse(get_str(td, "uri")?).ok()?;
        let position = parse_position(pos)?;
        return Some((uri, position));
    }
    None
}

fn parse_text_document_position_params(val: &JsonValue) -> Option<TextDocumentPositionParams> {
    let obj = val.as_object()?;
    let td = obj.get("textDocument")?.as_object()?;
    let uri = Url::parse(get_str(td, "uri")?).ok()?;
    let position = parse_position(obj.get("position")?)?;
    Some(TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri },
        position,
    })
}

/// Parse params for "textDocument/hover".
/// The LSP spec puts textDocument and position at the top level.
fn parse_hover_params(params: &JsonValue) -> Option<(Url, HoverParams)> {
    let tdpp = parse_text_document_position_params(params)?;
    let uri = tdpp.text_document.uri.clone();
    Some((
        uri,
        HoverParams {
            text_document_position_params: tdpp,
            work_done_progress_params: WorkDoneProgressParams {},
        },
    ))
}

fn parse_goto_definition_params(params: &JsonValue) -> Option<(Url, GotoDefinitionParams)> {
    let tdpp = parse_text_document_position_params(params)?;
    let uri = tdpp.text_document.uri.clone();
    Some((
        uri,
        GotoDefinitionParams {
            text_document_position_params: tdpp,
            work_done_progress_params: WorkDoneProgressParams {},
            partial_result_params: PartialResultParams {},
        },
    ))
}

fn parse_completion_params(params: &JsonValue) -> Option<(Url, CompletionParams)> {
    let tdpp = parse_text_document_position_params(params)?;
    let uri = tdpp.text_document.uri.clone();
    let obj = params.as_object()?;
    let context = obj.get("context").and_then(|c| {
        let co = c.as_object()?;
        Some(CompletionContext {
            trigger_kind: get_u32(co, "triggerKind").unwrap_or(1),
            trigger_character: get_str(co, "triggerCharacter").map(|s| s.to_string()),
        })
    });
    Some((
        uri,
        CompletionParams {
            text_document_position: tdpp,
            context,
            work_done_progress_params: WorkDoneProgressParams {},
            partial_result_params: PartialResultParams {},
        },
    ))
}

fn parse_code_action_params(params: &JsonValue) -> Option<(Url, CodeActionParams)> {
    let obj = params.as_object()?;
    let td = obj.get("textDocument")?.as_object()?;
    let uri = Url::parse(get_str(td, "uri")?).ok()?;
    let range = parse_range(obj.get("range")?)?;
    let ctx = obj.get("context").and_then(|v| v.as_object());
    let diagnostics = ctx
        .and_then(|c| c.get("diagnostics"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_diagnostic_from_json).collect())
        .unwrap_or_default();
    Some((
        uri.clone(),
        CodeActionParams {
            text_document: TextDocumentIdentifier { uri },
            range,
            context: CodeActionContext {
                diagnostics,
                only: None,
                trigger_kind: ctx.and_then(|c| get_u32(c, "triggerKind")),
            },
            work_done_progress_params: WorkDoneProgressParams {},
            partial_result_params: PartialResultParams {},
        },
    ))
}

fn parse_diagnostic_from_json(val: &JsonValue) -> Option<Diagnostic> {
    let obj = val.as_object()?;
    let range = parse_range(obj.get("range")?)?;
    let message = get_str(obj, "message").unwrap_or("").to_string();
    let severity = get_u32(obj, "severity").map(DiagnosticSeverity);
    let source = get_str(obj, "source").map(|s| s.to_string());
    let code = obj.get("code").and_then(|v| match v {
        JsonValue::Number(n) => n.as_i64().map(NumberOrString::Number),
        JsonValue::String(s) => Some(NumberOrString::String(s.clone())),
        _ => None,
    });
    Some(Diagnostic {
        range,
        severity,
        code,
        code_description: None,
        source,
        message,
        related_information: None,
        tags: None,
        data: None,
    })
}

fn parse_signature_help_params(params: &JsonValue) -> Option<(Url, SignatureHelpParams)> {
    let tdpp = parse_text_document_position_params(params)?;
    let uri = tdpp.text_document.uri.clone();
    let obj = params.as_object()?;
    let context = obj.get("context").and_then(|c| {
        let co = c.as_object()?;
        Some(SignatureHelpContext {
            trigger_kind: get_u32(co, "triggerKind").unwrap_or(1),
            trigger_character: get_str(co, "triggerCharacter").map(|s| s.to_string()),
            is_retrigger: co
                .get("isRetrigger")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    });
    Some((
        uri,
        SignatureHelpParams {
            text_document_position_params: tdpp,
            context,
            work_done_progress_params: WorkDoneProgressParams {},
        },
    ))
}

fn parse_formatting_params(params: &JsonValue) -> Option<(Url, DocumentFormattingParams)> {
    let obj = params.as_object()?;
    let td = obj.get("textDocument")?.as_object()?;
    let uri = Url::parse(get_str(td, "uri")?).ok()?;
    let options_obj = obj.get("options").and_then(|v| v.as_object());
    let tab_size = options_obj
        .and_then(|o| get_u32(o, "tabSize"))
        .unwrap_or(4);
    let insert_spaces = options_obj
        .and_then(|o| o.get("insertSpaces"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    Some((
        uri.clone(),
        DocumentFormattingParams {
            text_document: TextDocumentIdentifier { uri },
            options: FormattingOptions {
                tab_size,
                insert_spaces,
            },
        },
    ))
}

fn parse_range_formatting_params(
    params: &JsonValue,
) -> Option<(Url, DocumentRangeFormattingParams)> {
    let obj = params.as_object()?;
    let td = obj.get("textDocument")?.as_object()?;
    let uri = Url::parse(get_str(td, "uri")?).ok()?;
    let range = parse_range(obj.get("range")?)?;
    let options_obj = obj.get("options").and_then(|v| v.as_object());
    let tab_size = options_obj
        .and_then(|o| get_u32(o, "tabSize"))
        .unwrap_or(4);
    let insert_spaces = options_obj
        .and_then(|o| o.get("insertSpaces"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    Some((
        uri.clone(),
        DocumentRangeFormattingParams {
            text_document: TextDocumentIdentifier { uri },
            range,
            options: FormattingOptions {
                tab_size,
                insert_spaces,
            },
        },
    ))
}

fn parse_selection_range_params(params: &JsonValue) -> Option<(Url, Vec<Position>)> {
    let obj = params.as_object()?;
    let uri = parse_text_document_uri(params)?;
    let positions: Vec<Position> = obj
        .get("positions")?
        .as_array()?
        .iter()
        .filter_map(parse_position)
        .collect();
    Some((uri, positions))
}

fn parse_reference_params(params: &JsonValue) -> Option<(Url, ReferenceParams)> {
    let tdpp = parse_text_document_position_params(params)?;
    let uri = tdpp.text_document.uri.clone();
    let obj = params.as_object()?;
    let include_declaration = obj
        .get("context")
        .and_then(|c| c.as_object())
        .and_then(|co| co.get("includeDeclaration"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Some((
        uri,
        ReferenceParams {
            text_document_position: tdpp,
            context: ReferenceContext {
                include_declaration,
            },
            work_done_progress_params: WorkDoneProgressParams {},
            partial_result_params: PartialResultParams {},
        },
    ))
}

fn parse_rename_params(params: &JsonValue) -> Option<(Url, RenameParams)> {
    let tdpp = parse_text_document_position_params(params)?;
    let uri = tdpp.text_document.uri.clone();
    let obj = params.as_object()?;
    let new_name = get_str(obj, "newName")?.to_string();
    Some((
        uri,
        RenameParams {
            text_document_position: tdpp,
            new_name,
            work_done_progress_params: WorkDoneProgressParams {},
        },
    ))
}

fn parse_inlay_hint_params(params: &JsonValue) -> Option<(Url, Range)> {
    let obj = params.as_object()?;
    let uri = parse_text_document_uri(params)?;
    let range = parse_range(obj.get("range")?)?;
    Some((uri, range))
}

fn parse_linked_editing_range_params(
    params: &JsonValue,
) -> Option<(Url, LinkedEditingRangeParams)> {
    let tdpp = parse_text_document_position_params(params)?;
    let uri = tdpp.text_document.uri.clone();
    Some((
        uri,
        LinkedEditingRangeParams {
            text_document_position_params: tdpp,
            work_done_progress_params: WorkDoneProgressParams {},
        },
    ))
}

fn parse_call_hierarchy_prepare_params(
    params: &JsonValue,
) -> Option<(Url, CallHierarchyPrepareParams)> {
    let tdpp = parse_text_document_position_params(params)?;
    let uri = tdpp.text_document.uri.clone();
    Some((
        uri,
        CallHierarchyPrepareParams {
            text_document_position_params: tdpp,
            work_done_progress_params: WorkDoneProgressParams {},
        },
    ))
}

fn parse_call_hierarchy_item(val: &JsonValue) -> Option<CallHierarchyItem> {
    let obj = val.as_object()?;
    let name = get_str(obj, "name")?.to_string();
    let kind = SymbolKind(get_u32(obj, "kind").unwrap_or(12)); // FUNCTION default
    let uri = Url::parse(get_str(obj, "uri")?).ok()?;
    let range = parse_range(obj.get("range")?)?;
    let selection_range = parse_range(obj.get("selectionRange")?)?;
    let detail = get_str(obj, "detail").map(|s| s.to_string());
    Some(CallHierarchyItem {
        name,
        kind,
        tags: None,
        detail,
        uri,
        range,
        selection_range,
        data: None,
    })
}

fn parse_incoming_calls_params(params: &JsonValue) -> Option<CallHierarchyIncomingCallsParams> {
    let obj = params.as_object()?;
    let item = parse_call_hierarchy_item(obj.get("item")?)?;
    Some(CallHierarchyIncomingCallsParams {
        item,
        work_done_progress_params: WorkDoneProgressParams {},
        partial_result_params: PartialResultParams {},
    })
}

fn parse_outgoing_calls_params(params: &JsonValue) -> Option<CallHierarchyOutgoingCallsParams> {
    let obj = params.as_object()?;
    let item = parse_call_hierarchy_item(obj.get("item")?)?;
    Some(CallHierarchyOutgoingCallsParams {
        item,
        work_done_progress_params: WorkDoneProgressParams {},
        partial_result_params: PartialResultParams {},
    })
}


fn position_to_json(pos: &Position) -> JsonValue {
    JsonValue::Object(OrderedMap::from([
        ("line".into(), json_int(pos.line as i64)),
        ("character".into(), json_int(pos.character as i64)),
    ]))
}

fn range_to_json(range: &Range) -> JsonValue {
    JsonValue::Object(OrderedMap::from([
        ("start".into(), position_to_json(&range.start)),
        ("end".into(), position_to_json(&range.end)),
    ]))
}

fn location_to_json(loc: &Location) -> JsonValue {
    JsonValue::Object(OrderedMap::from([
        ("uri".into(), JsonValue::String(loc.uri.to_string())),
        ("range".into(), range_to_json(&loc.range)),
    ]))
}

fn locations_to_json(locs: &[Location]) -> JsonValue {
    JsonValue::Array(locs.iter().map(location_to_json).collect())
}

fn text_edit_to_json(edit: &TextEdit) -> JsonValue {
    JsonValue::Object(OrderedMap::from([
        ("range".into(), range_to_json(&edit.range)),
        ("newText".into(), JsonValue::String(edit.new_text.clone())),
    ]))
}

fn text_edits_to_json(edits: &[TextEdit]) -> JsonValue {
    JsonValue::Array(edits.iter().map(text_edit_to_json).collect())
}

fn diagnostic_to_json(d: &Diagnostic) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("range".into(), range_to_json(&d.range));
    if let Some(sev) = &d.severity {
        obj.insert("severity".into(), json_int(sev.0 as i64));
    }
    if let Some(code) = &d.code {
        match code {
            NumberOrString::Number(n) => {
                obj.insert("code".into(), json_int(*n));
            }
            NumberOrString::String(s) => {
                obj.insert("code".into(), JsonValue::String(s.clone()));
            }
        }
    }
    if let Some(src) = &d.source {
        obj.insert("source".into(), JsonValue::String(src.clone()));
    }
    obj.insert("message".into(), JsonValue::String(d.message.clone()));
    if let Some(tags) = &d.tags {
        obj.insert(
            "tags".into(),
            JsonValue::Array(tags.iter().map(|t| json_int(t.0 as i64)).collect()),
        );
    }
    if let Some(related) = &d.related_information {
        obj.insert(
            "relatedInformation".into(),
            JsonValue::Array(
                related
                    .iter()
                    .map(|ri| {
                        JsonValue::Object(OrderedMap::from([
                            ("location".into(), location_to_json(&ri.location)),
                            ("message".into(), JsonValue::String(ri.message.clone())),
                        ]))
                    })
                    .collect(),
            ),
        );
    }
    JsonValue::Object(obj)
}

fn markup_content_to_json(mc: &MarkupContent) -> JsonValue {
    let kind_str = match mc.kind {
        MarkupKind::PlainText => "plaintext",
        MarkupKind::Markdown => "markdown",
    };
    JsonValue::Object(OrderedMap::from([
        ("kind".into(), JsonValue::String(kind_str.into())),
        ("value".into(), JsonValue::String(mc.value.clone())),
    ]))
}

fn hover_to_json(hover: &Hover) -> JsonValue {
    let mut obj = OrderedMap::new();
    match &hover.contents {
        HoverContents::Markup(mc) => {
            obj.insert("contents".into(), markup_content_to_json(mc));
        }
        HoverContents::Scalar(MarkedString::String(s)) => {
            obj.insert("contents".into(), JsonValue::String(s.clone()));
        }
        HoverContents::Scalar(MarkedString::LanguageString { language, value }) => {
            obj.insert(
                "contents".into(),
                JsonValue::Object(OrderedMap::from([
                    ("language".into(), JsonValue::String(language.clone())),
                    ("value".into(), JsonValue::String(value.clone())),
                ])),
            );
        }
        HoverContents::Array(items) => {
            let arr: Vec<JsonValue> = items
                .iter()
                .map(|ms| match ms {
                    MarkedString::String(s) => JsonValue::String(s.clone()),
                    MarkedString::LanguageString { language, value } => {
                        JsonValue::Object(OrderedMap::from([
                            ("language".into(), JsonValue::String(language.clone())),
                            ("value".into(), JsonValue::String(value.clone())),
                        ]))
                    }
                })
                .collect();
            obj.insert("contents".into(), JsonValue::Array(arr));
        }
    }
    if let Some(range) = &hover.range {
        obj.insert("range".into(), range_to_json(range));
    }
    JsonValue::Object(obj)
}

fn goto_definition_response_to_json(resp: &GotoDefinitionResponse) -> JsonValue {
    match resp {
        GotoDefinitionResponse::Scalar(loc) => location_to_json(loc),
        GotoDefinitionResponse::Array(locs) => locations_to_json(locs),
        GotoDefinitionResponse::Link(links) => JsonValue::Array(
            links
                .iter()
                .map(|link| {
                    let mut obj = OrderedMap::new();
                    if let Some(osr) = &link.origin_selection_range {
                        obj.insert("originSelectionRange".into(), range_to_json(osr));
                    }
                    obj.insert(
                        "targetUri".into(),
                        JsonValue::String(link.target_uri.to_string()),
                    );
                    obj.insert("targetRange".into(), range_to_json(&link.target_range));
                    obj.insert(
                        "targetSelectionRange".into(),
                        range_to_json(&link.target_selection_range),
                    );
                    JsonValue::Object(obj)
                })
                .collect(),
        ),
    }
}

fn documentation_to_json(doc: &Documentation) -> JsonValue {
    match doc {
        Documentation::String(s) => JsonValue::String(s.clone()),
        Documentation::MarkupContent(mc) => markup_content_to_json(mc),
    }
}

fn completion_item_to_json(item: &CompletionItem) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("label".into(), JsonValue::String(item.label.clone()));
    if let Some(kind) = &item.kind {
        obj.insert("kind".into(), json_int(kind.0 as i64));
    }
    if let Some(detail) = &item.detail {
        obj.insert("detail".into(), JsonValue::String(detail.clone()));
    }
    if let Some(doc) = &item.documentation {
        obj.insert("documentation".into(), documentation_to_json(doc));
    }
    if let Some(dep) = &item.deprecated {
        obj.insert("deprecated".into(), JsonValue::Bool(*dep));
    }
    if let Some(pre) = &item.preselect {
        obj.insert("preselect".into(), JsonValue::Bool(*pre));
    }
    if let Some(st) = &item.sort_text {
        obj.insert("sortText".into(), JsonValue::String(st.clone()));
    }
    if let Some(ft) = &item.filter_text {
        obj.insert("filterText".into(), JsonValue::String(ft.clone()));
    }
    if let Some(it) = &item.insert_text {
        obj.insert("insertText".into(), JsonValue::String(it.clone()));
    }
    if let Some(itf) = &item.insert_text_format {
        obj.insert("insertTextFormat".into(), json_int(itf.0 as i64));
    }
    if let Some(te) = &item.text_edit {
        match te {
            CompletionTextEdit::Edit(edit) => {
                obj.insert("textEdit".into(), text_edit_to_json(edit));
            }
            CompletionTextEdit::InsertAndReplace(ire) => {
                obj.insert(
                    "textEdit".into(),
                    JsonValue::Object(OrderedMap::from([
                        ("newText".into(), JsonValue::String(ire.new_text.clone())),
                        ("insert".into(), range_to_json(&ire.insert)),
                        ("replace".into(), range_to_json(&ire.replace)),
                    ])),
                );
            }
        }
    }
    if let Some(ate) = &item.additional_text_edits {
        obj.insert(
            "additionalTextEdits".into(),
            JsonValue::Array(ate.iter().map(text_edit_to_json).collect()),
        );
    }
    if let Some(cmd) = &item.command {
        obj.insert("command".into(), command_to_json(cmd));
    }
    JsonValue::Object(obj)
}

fn completion_response_to_json(resp: &CompletionResponse) -> JsonValue {
    match resp {
        CompletionResponse::Array(items) => {
            JsonValue::Array(items.iter().map(completion_item_to_json).collect())
        }
        CompletionResponse::List(list) => JsonValue::Object(OrderedMap::from([
            (
                "isIncomplete".into(),
                JsonValue::Bool(list.is_incomplete),
            ),
            (
                "items".into(),
                JsonValue::Array(list.items.iter().map(completion_item_to_json).collect()),
            ),
        ])),
    }
}

fn command_to_json(cmd: &Command) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("title".into(), JsonValue::String(cmd.title.clone()));
    obj.insert("command".into(), JsonValue::String(cmd.command.clone()));
    if let Some(args) = &cmd.arguments {
        // Command arguments are serde_json::Value -- convert to string repr
        obj.insert(
            "arguments".into(),
            JsonValue::Array(
                args.iter()
                    .map(|a| JsonValue::String(a.to_string()))
                    .collect(),
            ),
        );
    }
    JsonValue::Object(obj)
}

fn code_action_response_to_json(actions: &[CodeActionOrCommand]) -> JsonValue {
    JsonValue::Array(
        actions
            .iter()
            .map(|aoc| match aoc {
                CodeActionOrCommand::Command(cmd) => command_to_json(cmd),
                CodeActionOrCommand::CodeAction(ca) => {
                    let mut obj = OrderedMap::new();
                    obj.insert("title".into(), JsonValue::String(ca.title.clone()));
                    if let Some(kind) = &ca.kind {
                        obj.insert("kind".into(), JsonValue::String(kind.as_str().to_string()));
                    }
                    if let Some(diags) = &ca.diagnostics {
                        obj.insert(
                            "diagnostics".into(),
                            JsonValue::Array(diags.iter().map(diagnostic_to_json).collect()),
                        );
                    }
                    if let Some(pref) = &ca.is_preferred {
                        obj.insert("isPreferred".into(), JsonValue::Bool(*pref));
                    }
                    if let Some(edit) = &ca.edit {
                        obj.insert("edit".into(), workspace_edit_to_json(edit));
                    }
                    if let Some(cmd) = &ca.command {
                        obj.insert("command".into(), command_to_json(cmd));
                    }
                    JsonValue::Object(obj)
                }
            })
            .collect(),
    )
}

fn signature_help_to_json(help: &SignatureHelp) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert(
        "signatures".into(),
        JsonValue::Array(
            help.signatures
                .iter()
                .map(|sig| {
                    let mut so = OrderedMap::new();
                    so.insert("label".into(), JsonValue::String(sig.label.clone()));
                    if let Some(doc) = &sig.documentation {
                        so.insert("documentation".into(), documentation_to_json(doc));
                    }
                    if let Some(params) = &sig.parameters {
                        so.insert(
                            "parameters".into(),
                            JsonValue::Array(
                                params
                                    .iter()
                                    .map(|pi| {
                                        let mut po = OrderedMap::new();
                                        match &pi.label {
                                            ParameterLabel::Simple(s) => {
                                                po.insert(
                                                    "label".into(),
                                                    JsonValue::String(s.clone()),
                                                );
                                            }
                                            ParameterLabel::Offsets(offs) => {
                                                po.insert(
                                                    "label".into(),
                                                    JsonValue::Array(vec![
                                                        json_int(offs[0] as i64),
                                                        json_int(offs[1] as i64),
                                                    ]),
                                                );
                                            }
                                        }
                                        if let Some(doc) = &pi.documentation {
                                            po.insert(
                                                "documentation".into(),
                                                documentation_to_json(doc),
                                            );
                                        }
                                        JsonValue::Object(po)
                                    })
                                    .collect(),
                            ),
                        );
                    }
                    if let Some(ap) = &sig.active_parameter {
                        so.insert("activeParameter".into(), json_int(*ap as i64));
                    }
                    JsonValue::Object(so)
                })
                .collect(),
        ),
    );
    if let Some(as_) = &help.active_signature {
        obj.insert("activeSignature".into(), json_int(*as_ as i64));
    }
    if let Some(ap) = &help.active_parameter {
        obj.insert("activeParameter".into(), json_int(*ap as i64));
    }
    JsonValue::Object(obj)
}

fn document_symbol_to_json(sym: &DocumentSymbol) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("name".into(), JsonValue::String(sym.name.clone()));
    if let Some(detail) = &sym.detail {
        obj.insert("detail".into(), JsonValue::String(detail.clone()));
    }
    obj.insert("kind".into(), json_int(sym.kind.0 as i64));
    if let Some(tags) = &sym.tags {
        obj.insert(
            "tags".into(),
            JsonValue::Array(tags.iter().map(|t| json_int(t.0 as i64)).collect()),
        );
    }
    if let Some(dep) = &sym.deprecated {
        obj.insert("deprecated".into(), JsonValue::Bool(*dep));
    }
    obj.insert("range".into(), range_to_json(&sym.range));
    obj.insert("selectionRange".into(), range_to_json(&sym.selection_range));
    if let Some(children) = &sym.children {
        obj.insert(
            "children".into(),
            JsonValue::Array(children.iter().map(document_symbol_to_json).collect()),
        );
    }
    JsonValue::Object(obj)
}

fn symbol_information_to_json(sym: &SymbolInformation) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("name".into(), JsonValue::String(sym.name.clone()));
    obj.insert("kind".into(), json_int(sym.kind.0 as i64));
    if let Some(tags) = &sym.tags {
        obj.insert(
            "tags".into(),
            JsonValue::Array(tags.iter().map(|t| json_int(t.0 as i64)).collect()),
        );
    }
    if let Some(dep) = &sym.deprecated {
        obj.insert("deprecated".into(), JsonValue::Bool(*dep));
    }
    obj.insert("location".into(), location_to_json(&sym.location));
    if let Some(cn) = &sym.container_name {
        obj.insert("containerName".into(), JsonValue::String(cn.clone()));
    }
    JsonValue::Object(obj)
}

fn symbol_informations_to_json(syms: &[SymbolInformation]) -> JsonValue {
    JsonValue::Array(syms.iter().map(symbol_information_to_json).collect())
}

fn document_symbol_response_to_json(resp: &DocumentSymbolResponse) -> JsonValue {
    match resp {
        DocumentSymbolResponse::Flat(syms) => symbol_informations_to_json(syms),
        DocumentSymbolResponse::Nested(syms) => {
            JsonValue::Array(syms.iter().map(document_symbol_to_json).collect())
        }
    }
}

fn workspace_edit_to_json(edit: &WorkspaceEdit) -> JsonValue {
    let mut obj = OrderedMap::new();
    if let Some(changes) = &edit.changes {
        let mut ch = OrderedMap::new();
        for (uri, edits) in changes {
            ch.insert(
                uri.to_string(),
                JsonValue::Array(edits.iter().map(text_edit_to_json).collect()),
            );
        }
        obj.insert("changes".into(), JsonValue::Object(ch));
    }
    if let Some(doc_changes) = &edit.document_changes {
        obj.insert(
            "documentChanges".into(),
            JsonValue::Array(
                doc_changes
                    .iter()
                    .map(|tde| {
                        JsonValue::Object(OrderedMap::from([
                            (
                                "textDocument".into(),
                                JsonValue::Object(OrderedMap::from([
                                    (
                                        "uri".into(),
                                        JsonValue::String(tde.text_document.uri.to_string()),
                                    ),
                                    (
                                        "version".into(),
                                        json_int(tde.text_document.version as i64),
                                    ),
                                ])),
                            ),
                            (
                                "edits".into(),
                                JsonValue::Array(
                                    tde.edits.iter().map(text_edit_to_json).collect(),
                                ),
                            ),
                        ]))
                    })
                    .collect(),
            ),
        );
    }
    JsonValue::Object(obj)
}

fn code_lens_to_json(lens: &CodeLens) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("range".into(), range_to_json(&lens.range));
    if let Some(cmd) = &lens.command {
        obj.insert("command".into(), command_to_json(cmd));
    }
    JsonValue::Object(obj)
}

fn code_lens_list_to_json(lenses: &[CodeLens]) -> JsonValue {
    JsonValue::Array(lenses.iter().map(code_lens_to_json).collect())
}

fn folding_range_to_json(fr: &FoldingRange) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("startLine".into(), json_int(fr.start_line as i64));
    if let Some(sc) = &fr.start_character {
        obj.insert("startCharacter".into(), json_int(*sc as i64));
    }
    obj.insert("endLine".into(), json_int(fr.end_line as i64));
    if let Some(ec) = &fr.end_character {
        obj.insert("endCharacter".into(), json_int(*ec as i64));
    }
    if let Some(kind) = &fr.kind {
        obj.insert("kind".into(), JsonValue::String(kind.as_str().to_string()));
    }
    if let Some(ct) = &fr.collapsed_text {
        obj.insert("collapsedText".into(), JsonValue::String(ct.clone()));
    }
    JsonValue::Object(obj)
}

fn folding_ranges_to_json(ranges: &[FoldingRange]) -> JsonValue {
    JsonValue::Array(ranges.iter().map(folding_range_to_json).collect())
}

fn selection_range_to_json(sr: &SelectionRange) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("range".into(), range_to_json(&sr.range));
    if let Some(parent) = &sr.parent {
        obj.insert("parent".into(), selection_range_to_json(parent));
    }
    JsonValue::Object(obj)
}

fn selection_ranges_to_json(ranges: &[SelectionRange]) -> JsonValue {
    JsonValue::Array(ranges.iter().map(selection_range_to_json).collect())
}

fn document_link_to_json(link: &DocumentLink) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("range".into(), range_to_json(&link.range));
    if let Some(target) = &link.target {
        obj.insert("target".into(), JsonValue::String(target.to_string()));
    }
    if let Some(tooltip) = &link.tooltip {
        obj.insert("tooltip".into(), JsonValue::String(tooltip.clone()));
    }
    JsonValue::Object(obj)
}

fn document_links_to_json(links: &[DocumentLink]) -> JsonValue {
    JsonValue::Array(links.iter().map(document_link_to_json).collect())
}

fn semantic_tokens_result_to_json(result: &SemanticTokensResult) -> JsonValue {
    match result {
        SemanticTokensResult::Tokens(tokens) => {
            let mut obj = OrderedMap::new();
            if let Some(id) = &tokens.result_id {
                obj.insert("resultId".into(), JsonValue::String(id.clone()));
            }
            let data: Vec<JsonValue> = tokens
                .data
                .iter()
                .flat_map(|t| {
                    vec![
                        json_int(t.delta_line as i64),
                        json_int(t.delta_start as i64),
                        json_int(t.length as i64),
                        json_int(t.token_type as i64),
                        json_int(t.token_modifiers_bitset as i64),
                    ]
                })
                .collect();
            obj.insert("data".into(), JsonValue::Array(data));
            JsonValue::Object(obj)
        }
        SemanticTokensResult::Partial(partial) => {
            let data: Vec<JsonValue> = partial
                .data
                .iter()
                .flat_map(|t| {
                    vec![
                        json_int(t.delta_line as i64),
                        json_int(t.delta_start as i64),
                        json_int(t.length as i64),
                        json_int(t.token_type as i64),
                        json_int(t.token_modifiers_bitset as i64),
                    ]
                })
                .collect();
            JsonValue::Object(OrderedMap::from([(
                "data".into(),
                JsonValue::Array(data),
            )]))
        }
    }
}

fn inlay_hint_to_json(hint: &InlayHint) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("position".into(), position_to_json(&hint.position));
    match &hint.label {
        InlayHintLabel::String(s) => {
            obj.insert("label".into(), JsonValue::String(s.clone()));
        }
        InlayHintLabel::LabelParts(parts) => {
            obj.insert(
                "label".into(),
                JsonValue::Array(
                    parts
                        .iter()
                        .map(|p| {
                            let mut po = OrderedMap::new();
                            po.insert("value".into(), JsonValue::String(p.value.clone()));
                            if let Some(loc) = &p.location {
                                po.insert("location".into(), location_to_json(loc));
                            }
                            if let Some(cmd) = &p.command {
                                po.insert("command".into(), command_to_json(cmd));
                            }
                            JsonValue::Object(po)
                        })
                        .collect(),
                ),
            );
        }
    }
    if let Some(kind) = &hint.kind {
        obj.insert("kind".into(), json_int(kind.0 as i64));
    }
    if let Some(pl) = &hint.padding_left {
        obj.insert("paddingLeft".into(), JsonValue::Bool(*pl));
    }
    if let Some(pr) = &hint.padding_right {
        obj.insert("paddingRight".into(), JsonValue::Bool(*pr));
    }
    JsonValue::Object(obj)
}

fn inlay_hints_to_json(hints: &[InlayHint]) -> JsonValue {
    JsonValue::Array(hints.iter().map(inlay_hint_to_json).collect())
}

fn document_highlight_to_json(highlight: &DocumentHighlight) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("range".into(), range_to_json(&highlight.range));
    if let Some(kind) = &highlight.kind {
        obj.insert("kind".into(), json_int(kind.0 as i64));
    }
    JsonValue::Object(obj)
}

fn document_highlights_to_json(highlights: &[DocumentHighlight]) -> JsonValue {
    JsonValue::Array(highlights.iter().map(document_highlight_to_json).collect())
}

fn linked_editing_ranges_to_json(ler: &LinkedEditingRanges) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert(
        "ranges".into(),
        JsonValue::Array(ler.ranges.iter().map(range_to_json).collect()),
    );
    if let Some(wp) = &ler.word_pattern {
        obj.insert("wordPattern".into(), JsonValue::String(wp.clone()));
    }
    JsonValue::Object(obj)
}

fn call_hierarchy_item_to_json(item: &CallHierarchyItem) -> JsonValue {
    let mut obj = OrderedMap::new();
    obj.insert("name".into(), JsonValue::String(item.name.clone()));
    obj.insert("kind".into(), json_int(item.kind.0 as i64));
    if let Some(tags) = &item.tags {
        obj.insert(
            "tags".into(),
            JsonValue::Array(tags.iter().map(|t| json_int(t.0 as i64)).collect()),
        );
    }
    if let Some(detail) = &item.detail {
        obj.insert("detail".into(), JsonValue::String(detail.clone()));
    }
    obj.insert("uri".into(), JsonValue::String(item.uri.to_string()));
    obj.insert("range".into(), range_to_json(&item.range));
    obj.insert(
        "selectionRange".into(),
        range_to_json(&item.selection_range),
    );
    JsonValue::Object(obj)
}

fn call_hierarchy_items_to_json(items: &[CallHierarchyItem]) -> JsonValue {
    JsonValue::Array(items.iter().map(call_hierarchy_item_to_json).collect())
}

fn incoming_calls_to_json(calls: &[CallHierarchyIncomingCall]) -> JsonValue {
    JsonValue::Array(
        calls
            .iter()
            .map(|c| {
                JsonValue::Object(OrderedMap::from([
                    ("from".into(), call_hierarchy_item_to_json(&c.from)),
                    (
                        "fromRanges".into(),
                        JsonValue::Array(c.from_ranges.iter().map(range_to_json).collect()),
                    ),
                ]))
            })
            .collect(),
    )
}

fn outgoing_calls_to_json(calls: &[CallHierarchyOutgoingCall]) -> JsonValue {
    JsonValue::Array(
        calls
            .iter()
            .map(|c| {
                JsonValue::Object(OrderedMap::from([
                    ("to".into(), call_hierarchy_item_to_json(&c.to)),
                    (
                        "fromRanges".into(),
                        JsonValue::Array(c.from_ranges.iter().map(range_to_json).collect()),
                    ),
                ]))
            })
            .collect(),
    )
}

// Server capabilities JSON

fn server_capabilities_to_json() -> JsonValue {
    let token_types: Vec<JsonValue> = semantic_tokens::TOKEN_TYPES
        .iter()
        .map(|t| JsonValue::String(t.as_str().to_string()))
        .collect();
    let token_modifiers: Vec<JsonValue> = semantic_tokens::TOKEN_MODIFIERS
        .iter()
        .map(|m| JsonValue::String(m.as_str().to_string()))
        .collect();

    JsonValue::Object(OrderedMap::from([
        (
            "textDocumentSync".into(),
            json_int(TextDocumentSyncKind::FULL.0 as i64),
        ),
        ("hoverProvider".into(), JsonValue::Bool(true)),
        (
            "completionProvider".into(),
            JsonValue::Object(OrderedMap::from([(
                "triggerCharacters".into(),
                JsonValue::Array(vec![
                    JsonValue::String(".".into()),
                    JsonValue::String(":".into()),
                ]),
            )])),
        ),
        (
            "signatureHelpProvider".into(),
            JsonValue::Object(OrderedMap::from([
                (
                    "triggerCharacters".into(),
                    JsonValue::Array(vec![
                        JsonValue::String("(".into()),
                        JsonValue::String(",".into()),
                    ]),
                ),
                (
                    "retriggerCharacters".into(),
                    JsonValue::Array(vec![JsonValue::String(",".into())]),
                ),
            ])),
        ),
        ("definitionProvider".into(), JsonValue::Bool(true)),
        ("codeActionProvider".into(), JsonValue::Bool(true)),
        ("documentSymbolProvider".into(), JsonValue::Bool(true)),
        ("documentFormattingProvider".into(), JsonValue::Bool(true)),
        (
            "codeLensProvider".into(),
            JsonValue::Object(OrderedMap::from([(
                "resolveProvider".into(),
                JsonValue::Bool(false),
            )])),
        ),
        ("workspaceSymbolProvider".into(), JsonValue::Bool(true)),
        ("selectionRangeProvider".into(), JsonValue::Bool(true)),
        ("linkedEditingRangeProvider".into(), JsonValue::Bool(true)),
        (
            "documentLinkProvider".into(),
            JsonValue::Object(OrderedMap::from([(
                "resolveProvider".into(),
                JsonValue::Bool(false),
            )])),
        ),
        ("callHierarchyProvider".into(), JsonValue::Bool(true)),
        ("referencesProvider".into(), JsonValue::Bool(true)),
        ("renameProvider".into(), JsonValue::Bool(true)),
        (
            "inlayHintProvider".into(),
            JsonValue::Object(OrderedMap::from([(
                "resolveProvider".into(),
                JsonValue::Bool(false),
            )])),
        ),
        (
            "semanticTokensProvider".into(),
            JsonValue::Object(OrderedMap::from([
                (
                    "legend".into(),
                    JsonValue::Object(OrderedMap::from([
                        ("tokenTypes".into(), JsonValue::Array(token_types)),
                        ("tokenModifiers".into(), JsonValue::Array(token_modifiers)),
                    ])),
                ),
                (
                    "full".into(),
                    JsonValue::Bool(true),
                ),
            ])),
        ),
        ("foldingRangeProvider".into(), JsonValue::Bool(true)),
        ("documentHighlightProvider".into(), JsonValue::Bool(true)),
        (
            "documentRangeFormattingProvider".into(),
            JsonValue::Bool(true),
        ),
        ("typeDefinitionProvider".into(), JsonValue::Bool(true)),
        ("colorProvider".into(), JsonValue::Bool(true)),
        (
            "documentOnTypeFormattingProvider".into(),
            JsonValue::Object(OrderedMap::from([
                (
                    "firstTriggerCharacter".into(),
                    JsonValue::String("}".into()),
                ),
                (
                    "moreTriggerCharacter".into(),
                    JsonValue::Array(vec![
                        JsonValue::String(";".into()),
                        JsonValue::String("\n".into()),
                    ]),
                ),
            ])),
        ),
        (
            "executeCommandProvider".into(),
            JsonValue::Object(OrderedMap::from([(
                "commands".into(),
                JsonValue::Array(vec![
                    JsonValue::String("magi.runTest".into()),
                    JsonValue::String("magi.runFile".into()),
                ]),
            )])),
        ),
        ("typeHierarchyProvider".into(), JsonValue::Bool(true)),
        (
            "workspace".into(),
            JsonValue::Object(OrderedMap::from([(
                "workspaceFolders".into(),
                JsonValue::Object(OrderedMap::from([
                    ("supported".into(), JsonValue::Bool(true)),
                    ("changeNotifications".into(), JsonValue::Bool(true)),
                ])),
            )])),
        ),
    ]))
}
