//! Go-to-definition provider for the MAGI LSP.

use super::analysis::{char_col_to_utf16, find_word_at_position, DocumentState};
use tower_lsp::lsp_types::*;

/// Convert a 1-based (line, col) span position to a 0-based UTF-16 LSP range.
fn span_to_lsp_range(source: &str, line: u32, col: u32, name_len: usize) -> Range {
    let lsp_line = line.saturating_sub(1);
    let char_col = col.saturating_sub(1);
    let line_text = source.lines().nth(lsp_line as usize).unwrap_or("");
    let start_utf16 = char_col_to_utf16(line_text, char_col);
    let end_utf16 = char_col_to_utf16(line_text, char_col + name_len as u32);
    Range {
        start: Position { line: lsp_line, character: start_utf16 },
        end: Position { line: lsp_line, character: end_utf16 },
    }
}

/// Handle a go-to-definition request.
pub fn handle_goto_definition(
    state: &DocumentState,
    params: &GotoDefinitionParams,
    uri: &Url,
) -> Option<GotoDefinitionResponse> {
    let pos = params.text_document_position_params.position;
    let word = find_word_at_position(&state.source, pos.line, pos.character)?;

    // Search in functions
    if let Some(func) = state.functions.get(&word) {
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: span_to_lsp_range(&state.source, func.line, func.col, word.len()),
        }));
    }

    // Search in variables
    if let Some(var) = state.variables.get(&word) {
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: span_to_lsp_range(&state.source, var.line, var.col, word.len()),
        }));
    }

    // Search in enums
    if let Some(en) = state.enums.get(&word) {
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: span_to_lsp_range(&state.source, en.line, en.col, word.len()),
        }));
    }

    // Search in structs
    if let Some(st) = state.structs.get(&word) {
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: span_to_lsp_range(&state.source, st.line, st.col, word.len()),
        }));
    }

    None
}
