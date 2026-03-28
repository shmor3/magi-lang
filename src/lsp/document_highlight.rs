//! Document highlight — highlight all occurrences of a symbol under the cursor.

use super::analysis::{char_col_to_utf16, find_word_at_position, is_ident_char, DocumentState};
use super::types::*;

/// Handle a document highlight request.
///
/// Finds all occurrences of the identifier under the cursor in the document,
/// returning them as `DocumentHighlight` entries with `Text` kind.
/// Skips matches inside string literals and line comments.
pub fn handle_document_highlight(
    state: &DocumentState,
    params: &TextDocumentPositionParams,
) -> Option<Vec<DocumentHighlight>> {
    let pos = params.position;
    let word = find_word_at_position(&state.source, pos.line, pos.character)?;

    let highlights = find_highlight_occurrences(&state.source, &word);

    // No point highlighting if only one occurrence
    if highlights.len() <= 1 {
        return None;
    }
    Some(highlights)
}

/// Find all occurrences of an identifier in a source string, returning document highlights.
///
/// Only matches whole identifiers (word-boundary aware), skipping occurrences
/// inside string literals and comments.
fn find_highlight_occurrences(source: &str, name: &str) -> Vec<DocumentHighlight> {
    let mut highlights = Vec::new();

    for (line_idx, line_text) in source.lines().enumerate() {
        let lsp_line = line_idx as u32;

        let mut search_start = 0;
        while let Some(byte_offset) = line_text[search_start..].find(name) {
            let abs_byte_offset = search_start + byte_offset;
            let after_pos = abs_byte_offset + name.len();

            let before_ok = abs_byte_offset == 0
                || !line_text[..abs_byte_offset]
                    .chars()
                    .next_back()
                    .is_some_and(|c| is_ident_char(c));
            let after_ok = after_pos >= line_text.len()
                || !line_text[after_pos..]
                    .chars()
                    .next()
                    .is_some_and(|c| is_ident_char(c));

            if before_ok && after_ok && !is_in_string_or_comment(line_text, abs_byte_offset) {
                let char_col = line_text[..abs_byte_offset].chars().count() as u32;
                let char_end = char_col + name.chars().count() as u32;
                let start_utf16 = char_col_to_utf16(line_text, char_col);
                let end_utf16 = char_col_to_utf16(line_text, char_end);

                highlights.push(DocumentHighlight {
                    range: Range {
                        start: Position {
                            line: lsp_line,
                            character: start_utf16,
                        },
                        end: Position {
                            line: lsp_line,
                            character: end_utf16,
                        },
                    },
                    kind: Some(DocumentHighlightKind::TEXT),
                });
            }

            // Advance past this match
            search_start = abs_byte_offset + name.len().max(1);
        }
    }

    highlights
}

/// Check if a byte offset in a line falls inside a string literal or comment.
fn is_in_string_or_comment(line: &str, target_byte_offset: usize) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && i < target_byte_offset {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                return true;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                if i > target_byte_offset {
                    return true;
                }
                continue;
            }
            b'\'' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'\'' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                if i > target_byte_offset {
                    return true;
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze_document;

    fn make_params(line: u32, character: u32) -> TextDocumentPositionParams {
        TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse("file:///test.magi").unwrap(),
            },
            position: Position { line, character },
        }
    }

    #[test]
    fn test_highlight_variable() {
        let source = "let x = 5;\nlet y = x + 1;\nlet z = x;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 4); // cursor on "x"
        let result = handle_document_highlight(&state, &params);
        assert!(result.is_some());
        let highlights = result.unwrap();
        assert_eq!(highlights.len(), 3); // definition + 2 usages
    }

    #[test]
    fn test_highlight_function() {
        let source = "fn greet() { null }\ngreet()\ngreet()";
        let (state, _) = analyze_document(source);
        let params = make_params(1, 2); // cursor on "greet"
        let result = handle_document_highlight(&state, &params);
        assert!(result.is_some());
        let highlights = result.unwrap();
        assert_eq!(highlights.len(), 3); // definition + 2 calls
    }

    #[test]
    fn test_no_highlight_for_whitespace() {
        let source = "let x = 5;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 6); // cursor on space
        let result = handle_document_highlight(&state, &params);
        assert!(result.is_none());
    }

    #[test]
    fn test_no_highlight_for_single_occurrence() {
        let source = "let unique_name = 5;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 4); // cursor on "unique_name"
        let result = handle_document_highlight(&state, &params);
        assert!(result.is_none()); // only one occurrence, no highlight
    }

    #[test]
    fn test_highlight_skips_comments() {
        let source = "let foo = 1;\n// foo is great\nlet bar = foo;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 4); // cursor on "foo"
        let result = handle_document_highlight(&state, &params);
        assert!(result.is_some());
        let highlights = result.unwrap();
        // definition + usage in bar, NOT the comment
        assert_eq!(highlights.len(), 2);
    }

    #[test]
    fn test_highlight_skips_strings() {
        let source = "let name = 1;\nlet msg = \"name is cool\";\nlet x = name;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 4); // cursor on "name"
        let result = handle_document_highlight(&state, &params);
        assert!(result.is_some());
        let highlights = result.unwrap();
        // definition + usage, NOT the string
        assert_eq!(highlights.len(), 2);
    }

    #[test]
    fn test_highlight_word_boundary() {
        let source = "let x = 1;\nlet xy = 2;\nlet ax = 3;\nlet z = x;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 4); // cursor on "x"
        let result = handle_document_highlight(&state, &params);
        assert!(result.is_some());
        let highlights = result.unwrap();
        // Only "x" in "let x = 1" and "let z = x", not "xy" or "ax"
        assert_eq!(highlights.len(), 2);
    }

    #[test]
    fn test_highlight_kind_is_text() {
        let source = "let a = 1;\nlet b = a;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 4); // cursor on "a"
        let result = handle_document_highlight(&state, &params);
        assert!(result.is_some());
        let highlights = result.unwrap();
        for h in &highlights {
            assert_eq!(h.kind, Some(DocumentHighlightKind::TEXT));
        }
    }
}
