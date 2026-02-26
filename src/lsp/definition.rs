//! Go-to-definition provider for the MAGI LSP.

use super::analysis::{find_word_at_position, DocumentState};
use tower_lsp::lsp_types::*;

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
            range: Range {
                start: Position {
                    line: func.line.saturating_sub(1),
                    character: func.col.saturating_sub(1),
                },
                end: Position {
                    line: func.line.saturating_sub(1),
                    character: func.col.saturating_sub(1) + word.len() as u32,
                },
            },
        }));
    }

    // Search in variables
    if let Some(var) = state.variables.get(&word) {
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: Range {
                start: Position {
                    line: var.line.saturating_sub(1),
                    character: var.col.saturating_sub(1),
                },
                end: Position {
                    line: var.line.saturating_sub(1),
                    character: var.col.saturating_sub(1) + word.len() as u32,
                },
            },
        }));
    }

    // Search in enums
    if let Some(en) = state.enums.get(&word) {
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: Range {
                start: Position {
                    line: en.line.saturating_sub(1),
                    character: en.col.saturating_sub(1),
                },
                end: Position {
                    line: en.line.saturating_sub(1),
                    character: en.col.saturating_sub(1) + word.len() as u32,
                },
            },
        }));
    }

    // Search in structs
    if let Some(st) = state.structs.get(&word) {
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: Range {
                start: Position {
                    line: st.line.saturating_sub(1),
                    character: st.col.saturating_sub(1),
                },
                end: Position {
                    line: st.line.saturating_sub(1),
                    character: st.col.saturating_sub(1) + word.len() as u32,
                },
            },
        }));
    }

    None
}
