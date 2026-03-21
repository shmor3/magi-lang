//! Linked editing ranges provider for the MAGI LSP.
//!
//! When the cursor is on a variable name, returns all positions where that
//! same variable is referenced, enabling simultaneous editing.

use super::analysis::{char_col_to_utf16, find_word_at_position, is_ident_char, DocumentState};
use tower_lsp::lsp_types::*;

/// Handle a linked editing range request.
///
/// Finds all occurrences of the identifier under the cursor in the document
/// and returns them as linked editing ranges.
pub fn handle_linked_editing_range(
    state: &DocumentState,
    params: &LinkedEditingRangeParams,
) -> Option<LinkedEditingRanges> {
    let pos = params.text_document_position_params.position;
    let word = find_word_at_position(&state.source, pos.line, pos.character)?;

    let ranges = find_identifier_ranges(&state.source, &word);

    if ranges.len() < 2 {
        // No point in linked editing if there's only one occurrence
        return None;
    }

    Some(LinkedEditingRanges {
        ranges,
        word_pattern: None,
    })
}

/// Find all occurrences of an identifier in source text, returning LSP ranges.
///
/// Only matches whole identifiers (word-boundary aware), skipping occurrences
/// inside string literals and comments.
fn find_identifier_ranges(source: &str, name: &str) -> Vec<Range> {
    let mut ranges = Vec::new();

    for (line_idx, line_text) in source.lines().enumerate() {
        let lsp_line = line_idx as u32;

        let mut search_start = 0;
        while let Some(byte_offset) = line_text[search_start..].find(name) {
            let abs_byte_offset = search_start + byte_offset;
            let after_pos = abs_byte_offset + name.len();

            // Check word boundaries
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

                ranges.push(Range {
                    start: Position {
                        line: lsp_line,
                        character: start_utf16,
                    },
                    end: Position {
                        line: lsp_line,
                        character: end_utf16,
                    },
                });
            }

            search_start = abs_byte_offset + name.len().max(1);
        }
    }

    ranges
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

    fn make_params(line: u32, character: u32) -> LinkedEditingRangeParams {
        LinkedEditingRangeParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse("file:///test.magi").unwrap(),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        }
    }

    #[test]
    fn test_linked_editing_variable() {
        let source = "let x = 5;\nlet y = x + 1;\nlet z = x;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 4); // cursor on "x"
        let result = handle_linked_editing_range(&state, &params);
        assert!(result.is_some());
        let ranges = result.unwrap().ranges;
        assert_eq!(ranges.len(), 3); // definition + 2 usages
    }

    #[test]
    fn test_linked_editing_function() {
        let source = "fn greet() { null }\ngreet()\ngreet()";
        let (state, _) = analyze_document(source);
        let params = make_params(1, 2); // cursor on "greet" call
        let result = handle_linked_editing_range(&state, &params);
        assert!(result.is_some());
        let ranges = result.unwrap().ranges;
        assert_eq!(ranges.len(), 3); // definition + 2 calls
    }

    #[test]
    fn test_linked_editing_single_occurrence() {
        let source = "let unique_var = 42;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 4); // cursor on "unique_var"
        let result = handle_linked_editing_range(&state, &params);
        // Single occurrence should return None (no linked editing makes sense)
        assert!(result.is_none());
    }

    #[test]
    fn test_linked_editing_whitespace() {
        let source = "let x = 5;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 6); // cursor on space
        let result = handle_linked_editing_range(&state, &params);
        assert!(result.is_none());
    }

    #[test]
    fn test_linked_editing_skips_comments() {
        let source = "let foo = 1;\n// foo is great\nlet bar = foo;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 4); // cursor on "foo"
        let result = handle_linked_editing_range(&state, &params);
        assert!(result.is_some());
        let ranges = result.unwrap().ranges;
        // definition + usage, NOT the comment
        assert_eq!(ranges.len(), 2);
    }

    #[test]
    fn test_linked_editing_skips_strings() {
        let source = "let name = 1;\nlet msg = \"name\";\nlet x = name;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 4); // cursor on "name"
        let result = handle_linked_editing_range(&state, &params);
        assert!(result.is_some());
        let ranges = result.unwrap().ranges;
        assert_eq!(ranges.len(), 2);
    }

    #[test]
    fn test_linked_editing_word_boundary() {
        let source = "let x = 1;\nlet xy = 2;\nlet z = x;";
        let (state, _) = analyze_document(source);
        let params = make_params(0, 4); // cursor on "x"
        let result = handle_linked_editing_range(&state, &params);
        assert!(result.is_some());
        let ranges = result.unwrap().ranges;
        // Only "x" occurrences, not "xy"
        assert_eq!(ranges.len(), 2);
    }
}
