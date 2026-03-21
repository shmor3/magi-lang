//! Rename provider for the MAGI LSP.

use super::analysis::{char_col_to_utf16, find_word_at_position, is_ident_char, DocumentState};
use std::collections::HashMap;
use tower_lsp::lsp_types::*;

/// Handle a rename request.
///
/// Finds all occurrences of the identifier under the cursor and returns a
/// `WorkspaceEdit` that replaces each one with the new name. Only renames
/// identifiers, skipping occurrences inside string literals and comments.
pub fn handle_rename(
    state: &DocumentState,
    params: &RenameParams,
    uri: &Url,
) -> Option<WorkspaceEdit> {
    let pos = params.text_document_position.position;
    let word = find_word_at_position(&state.source, pos.line, pos.character)?;

    let edits = find_identifier_edits(&state.source, &word, &params.new_name);

    if edits.is_empty() {
        return None;
    }

    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);

    Some(WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    })
}

/// Find all occurrences of an identifier in source and return `TextEdit`s
/// that replace each with `new_name`.
///
/// Only matches whole identifiers (word-boundary aware), skipping occurrences
/// inside string literals and comments.
fn find_identifier_edits(source: &str, name: &str, new_name: &str) -> Vec<TextEdit> {
    let mut edits = Vec::new();

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

                edits.push(TextEdit {
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
                    new_text: new_name.to_string(),
                });
            }

            search_start = abs_byte_offset + name.len().max(1);
        }
    }

    edits
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

    fn test_uri() -> Url {
        Url::parse("file:///test.magi").unwrap()
    }

    fn make_rename_params(line: u32, character: u32, new_name: &str) -> RenameParams {
        RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: test_uri() },
                position: Position { line, character },
            },
            new_name: new_name.to_string(),
            work_done_progress_params: Default::default(),
        }
    }

    #[test]
    fn test_rename_variable() {
        let source = "let x = 5;\nlet y = x + 1;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_rename_params(0, 4, "count"); // rename "x" to "count"
        let result = handle_rename(&state, &params, &uri);
        assert!(result.is_some());
        let edit = result.unwrap();
        let changes = edit.changes.unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits.len(), 2); // "x" in definition + usage
        for e in edits {
            assert_eq!(e.new_text, "count");
        }
    }

    #[test]
    fn test_rename_function() {
        let source = "fn greet() { null }\ngreet()\ngreet()";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_rename_params(0, 3, "hello"); // rename "greet" to "hello"
        let result = handle_rename(&state, &params, &uri);
        assert!(result.is_some());
        let edit = result.unwrap();
        let changes = edit.changes.unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits.len(), 3); // definition + 2 calls
    }

    #[test]
    fn test_rename_skips_strings() {
        let source = "let name = 1;\nlet msg = \"name\";\nlet x = name;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_rename_params(0, 4, "id"); // rename "name" to "id"
        let result = handle_rename(&state, &params, &uri);
        assert!(result.is_some());
        let edit = result.unwrap();
        let changes = edit.changes.unwrap();
        let edits = changes.get(&uri).unwrap();
        // definition + usage, NOT the string
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn test_rename_skips_comments() {
        let source = "let foo = 1;\n// foo is great\nlet bar = foo;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_rename_params(0, 4, "baz");
        let result = handle_rename(&state, &params, &uri);
        assert!(result.is_some());
        let edit = result.unwrap();
        let changes = edit.changes.unwrap();
        let edits = changes.get(&uri).unwrap();
        // definition + usage, NOT the comment
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn test_rename_whitespace_returns_none() {
        let source = "let x = 5;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_rename_params(0, 6, "y"); // cursor on space between "=" and "5"
        let result = handle_rename(&state, &params, &uri);
        assert!(result.is_none());
    }

    #[test]
    fn test_rename_respects_word_boundaries() {
        let source = "let x = 1;\nlet xy = 2;\nlet z = x;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let params = make_rename_params(0, 4, "a");
        let result = handle_rename(&state, &params, &uri);
        assert!(result.is_some());
        let edit = result.unwrap();
        let changes = edit.changes.unwrap();
        let edits = changes.get(&uri).unwrap();
        // Only "x" in "let x = 1" and "let z = x", not "xy"
        assert_eq!(edits.len(), 2);
    }
}
