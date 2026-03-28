//! Find-references provider for the MAGI LSP.

use super::analysis::{char_col_to_utf16, find_word_at_position, is_ident_char, DocumentState};
use super::types::*;

/// Handle a find-references request.
///
/// Finds all occurrences of the identifier under the cursor in the document
/// using word-boundary-aware text matching.
pub fn handle_references(
    state: &DocumentState,
    params: &ReferenceParams,
    uri: &Url,
) -> Option<Vec<Location>> {
    let pos = params.text_document_position.position;
    let word = find_word_at_position(&state.source, pos.line, pos.character)?;

    let locations = find_identifier_occurrences(&state.source, &word, uri);

    if locations.is_empty() {
        None
    } else {
        Some(locations)
    }
}

/// Find all occurrences of an identifier in a source string, returning LSP locations.
///
/// Only matches whole identifiers (word-boundary aware), skipping occurrences
/// inside string literals and comments.
fn find_identifier_occurrences(source: &str, name: &str, uri: &Url) -> Vec<Location> {
    let mut locations = Vec::new();

    for (line_idx, line_text) in source.lines().enumerate() {
        let lsp_line = line_idx as u32;

        // Find all word-boundary matches on this line, skipping strings and comments
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

                locations.push(Location {
                    uri: uri.clone(),
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
                });
            }

            // Advance past this match
            search_start = abs_byte_offset + name.len().max(1);
        }
    }

    locations
}

/// Check if a byte offset in a line falls inside a string literal or comment.
///
/// Uses a simple single-pass state machine that tracks whether we are inside
/// a single-quoted string, double-quoted string, or a `//` line comment.
fn is_in_string_or_comment(line: &str, target_byte_offset: usize) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && i < target_byte_offset {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                // Rest of line is a comment
                return true;
            }
            b'"' => {
                // Enter double-quoted string
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2; // skip escape
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
                // Enter single-quoted string
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2; // skip escape
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

    fn test_uri() -> Url {
        Url::parse("file:///test.magi").unwrap()
    }

    fn make_reference_params(line: u32, character: u32) -> ReferenceParams {
        ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: test_uri() },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: ReferenceContext {
                include_declaration: true,
            },
        }
    }

    #[test]
    fn test_find_variable_references() {
        let source = "let x = 5;\nlet y = x + 1;\nlet z = x;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_reference_params(0, 4); // cursor on "x"
        let result = handle_references(&state, &params, &uri);
        assert!(result.is_some());
        let locs = result.unwrap();
        assert_eq!(locs.len(), 3); // definition + 2 usages
    }

    #[test]
    fn test_find_function_references() {
        let source = "fn greet() { null }\ngreet()\ngreet()";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_reference_params(1, 2); // cursor on "greet" call
        let result = handle_references(&state, &params, &uri);
        assert!(result.is_some());
        let locs = result.unwrap();
        assert_eq!(locs.len(), 3); // definition + 2 calls
    }

    #[test]
    fn test_no_references_for_whitespace() {
        let source = "let x = 5;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_reference_params(0, 6); // cursor on space between "=" and "5"
        let result = handle_references(&state, &params, &uri);
        // Whitespace should not match any identifier
        assert!(result.is_none());
    }

    #[test]
    fn test_references_skip_comments() {
        let source = "let foo = 1;\n// foo is great\nlet bar = foo;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_reference_params(0, 4); // cursor on "foo"
        let result = handle_references(&state, &params, &uri);
        assert!(result.is_some());
        let locs = result.unwrap();
        // definition + usage in bar, NOT the comment
        assert_eq!(locs.len(), 2);
    }

    #[test]
    fn test_references_skip_strings() {
        let source = "let name = 1;\nlet msg = \"name is cool\";\nlet x = name;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_reference_params(0, 4); // cursor on "name"
        let result = handle_references(&state, &params, &uri);
        assert!(result.is_some());
        let locs = result.unwrap();
        // definition + usage, NOT the string
        assert_eq!(locs.len(), 2);
    }

    #[test]
    fn test_references_word_boundary() {
        let source = "let x = 1;\nlet xy = 2;\nlet ax = 3;\nlet z = x;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_reference_params(0, 4); // cursor on "x"
        let result = handle_references(&state, &params, &uri);
        assert!(result.is_some());
        let locs = result.unwrap();
        // Only "x" in "let x = 1" and "let z = x", not "xy" or "ax"
        assert_eq!(locs.len(), 2);
    }

    #[test]
    fn test_is_in_string_or_comment() {
        assert!(!is_in_string_or_comment("let x = 5;", 4));
        assert!(is_in_string_or_comment("// let x = 5;", 7));
        assert!(is_in_string_or_comment("let x = \"hello\";", 10));
        assert!(!is_in_string_or_comment("let x = \"hello\";", 4));
    }
}
