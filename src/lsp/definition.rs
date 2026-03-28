//! Go-to-definition provider for the MAGI LSP.

use super::analysis::{char_col_to_utf16, find_enum_variant_at_position, find_word_at_position, DocumentState};
use super::types::*;

/// Convert a 1-based (line, col) span position to a 0-based UTF-16 LSP range.
/// `name_char_len` must be in characters (not bytes).
fn span_to_lsp_range(source: &str, line: u32, col: u32, name_char_len: usize) -> Range {
    let lsp_line = line.saturating_sub(1);
    let char_col = col.saturating_sub(1);
    let line_text = source.lines().nth(lsp_line as usize).unwrap_or("");
    let start_utf16 = char_col_to_utf16(line_text, char_col);
    let end_utf16 = char_col_to_utf16(line_text, char_col.saturating_add(name_char_len as u32));
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

    // Check for enum variant pattern (EnumName::Variant) first
    if let Some((enum_name, _variant_name)) = find_enum_variant_at_position(&state.source, pos.line, pos.character) {
        if let Some(en) = state.enums.get(&enum_name) {
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: span_to_lsp_range(&state.source, en.line, en.col, enum_name.chars().count()),
            }));
        }
    }

    let word = find_word_at_position(&state.source, pos.line, pos.character)?;

    if let Some(func) = state.functions.get(&word) {
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: span_to_lsp_range(&state.source, func.line, func.col, word.chars().count()),
        }));
    }

    if let Some(var) = state.variables.get(&word) {
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: span_to_lsp_range(&state.source, var.line, var.col, word.chars().count()),
        }));
    }

    if let Some(en) = state.enums.get(&word) {
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: span_to_lsp_range(&state.source, en.line, en.col, word.chars().count()),
        }));
    }

    if let Some(st) = state.structs.get(&word) {
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: span_to_lsp_range(&state.source, st.line, st.col, word.chars().count()),
        }));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze_document;

    fn test_uri() -> Url {
        Url::parse("file:///test.magi").unwrap()
    }

    fn make_goto_params(line: u32, character: u32) -> GotoDefinitionParams {
        GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: test_uri(),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        }
    }

    #[test]
    fn test_goto_function_definition() {
        let source = "fn greet() { null }\ngreet()";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_goto_params(1, 2); // cursor on "greet" call
        let result = handle_goto_definition(&state, &params, &uri);
        assert!(result.is_some());
        if let Some(GotoDefinitionResponse::Scalar(loc)) = result {
            assert_eq!(loc.range.start.line, 0); // defined on line 0
        }
    }

    #[test]
    fn test_goto_variable_definition() {
        let source = "let x = 5;\nlet y = x;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_goto_params(1, 8); // cursor on "x" in "let y = x"
        let result = handle_goto_definition(&state, &params, &uri);
        assert!(result.is_some());
        if let Some(GotoDefinitionResponse::Scalar(loc)) = result {
            assert_eq!(loc.range.start.line, 0); // x defined on line 0
        }
    }

    #[test]
    fn test_goto_enum_definition() {
        let source = "enum Color { Red, Green, Blue }\nlet c = Color::Red;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        // Cursor on "Color" in "Color::Red"
        let params = make_goto_params(1, 9);
        let result = handle_goto_definition(&state, &params, &uri);
        assert!(result.is_some());
        if let Some(GotoDefinitionResponse::Scalar(loc)) = result {
            assert_eq!(loc.range.start.line, 0); // enum defined on line 0
        }
    }

    #[test]
    fn test_goto_struct_definition() {
        let source = "struct Point { x: float64, y: float64 }\nlet p = Point { x: 1.0, y: 2.0 };";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        // Cursor on "Point" in the constructor
        let params = make_goto_params(1, 9);
        let result = handle_goto_definition(&state, &params, &uri);
        assert!(result.is_some());
        if let Some(GotoDefinitionResponse::Scalar(loc)) = result {
            assert_eq!(loc.range.start.line, 0);
        }
    }

    #[test]
    fn test_goto_unknown_returns_none() {
        let source = "let x = 5;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        // Cursor on "5" (a number literal, not a defined symbol)
        let params = make_goto_params(0, 8);
        let result = handle_goto_definition(&state, &params, &uri);
        // "5" is not in any symbol table (it is a number in find_word)
        assert!(result.is_none());
    }

    #[test]
    fn test_goto_empty_document() {
        let source = "";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_goto_params(0, 0);
        let result = handle_goto_definition(&state, &params, &uri);
        assert!(result.is_none());
    }

    #[test]
    fn test_span_to_lsp_range_basic() {
        let source = "let foo = 5;";
        let range = span_to_lsp_range(source, 1, 5, 3); // "foo" at col 5 (1-based)
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 4); // 0-based
        assert_eq!(range.end.character, 7); // 4 + 3
    }

    #[test]
    fn test_span_to_lsp_range_line_0_col_0() {
        let source = "hello";
        // Edge case: line 0 / col 0 saturating_sub
        let range = span_to_lsp_range(source, 0, 0, 5);
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
    }
}
