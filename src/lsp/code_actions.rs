//! Code action provider for the MAGI LSP.
//!
//! Provides quick-fix code actions for common diagnostics:
//! - **Import suggestion (E201)**: When a function name matches a known std module
//!   operation, suggest adding the appropriate `use std::module::*;` import.
//! - **Snake case fix (W200)**: When a naming convention warning fires, offer a
//!   code action to rename the identifier to snake_case.

use super::analysis::DocumentState;
use crate::syntax::interpreter::{std_module_ops, STD_MODULE_NAMES};
use crate::util::to_snake_case;
use std::collections::HashMap;
use tower_lsp::lsp_types::*;

/// Handle a code action request.
///
/// For each diagnostic in the requested range, checks whether a quick fix is
/// available and returns the corresponding `CodeActionOrCommand` items.
pub fn handle_code_actions(
    state: &DocumentState,
    params: &CodeActionParams,
    uri: &Url,
) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();

    for diag in &params.context.diagnostics {
        let code_str = match &diag.code {
            Some(NumberOrString::String(s)) => s.as_str(),
            _ => continue,
        };

        match code_str {
            "E201" => {
                if let Some(action) = import_suggestion_action(state, diag, uri) {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
            }
            "W200" => {
                if let Some(action) = snake_case_fix_action(state, diag, uri) {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
            }
            _ => {}
        }
    }

    // Always offer "Organize Imports" if there are use statements to sort
    if let Some(action) = organize_imports_action(state, uri) {
        actions.push(CodeActionOrCommand::CodeAction(action));
    }

    actions
}

/// Find which std module(s) export a given function name.
fn find_modules_for_function(name: &str) -> Vec<&'static str> {
    let mut modules = Vec::new();
    for &module in STD_MODULE_NAMES {
        let ops = std_module_ops(module);
        if ops.contains(&name) {
            modules.push(module);
        }
    }
    modules
}

/// Extract the function name from an E201 diagnostic message.
///
/// Expected format: "Undefined function or operation 'foo_bar'"
fn extract_function_name(message: &str) -> Option<&str> {
    let start = message.find('\'')?;
    let rest = &message[start + 1..];
    let end = rest.find('\'')?;
    Some(&rest[..end])
}

/// Build a code action that inserts a `use std::module::*;` import for an
/// undefined function that exists in a standard library module.
fn import_suggestion_action(
    state: &DocumentState,
    diag: &Diagnostic,
    uri: &Url,
) -> Option<CodeAction> {
    let func_name = extract_function_name(&diag.message)?;
    let modules = find_modules_for_function(func_name);
    if modules.is_empty() {
        return None;
    }

    // Use the first matching module (most common case: one match).
    let module = modules[0];

    // Determine where to insert the import. Place it at the top of the file,
    // after any existing `use` statements on consecutive lines starting from
    // line 0.
    let insert_line = find_import_insert_line(&state.source);
    let import_text = format!("use std::{}::*;\n", module);

    let mut changes = HashMap::new();
    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range: Range {
                start: Position {
                    line: insert_line,
                    character: 0,
                },
                end: Position {
                    line: insert_line,
                    character: 0,
                },
            },
            new_text: import_text,
        }],
    );

    Some(CodeAction {
        title: format!("Import `{}` from std::{}", func_name, module),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        is_preferred: Some(true),
        ..Default::default()
    })
}

/// Find the line number where a new import should be inserted.
///
/// Scans from the top of the file for consecutive `use` lines and returns the
/// line after the last one. If there are no imports, returns 0.
fn find_import_insert_line(source: &str) -> u32 {
    let mut last_use_line: Option<u32> = None;
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("use ") {
            last_use_line = Some(idx as u32);
        } else if !trimmed.is_empty() && last_use_line.is_some() {
            // Hit a non-empty, non-use line after seeing use lines — stop.
            break;
        } else if !trimmed.is_empty() && last_use_line.is_none() {
            // Hit code before any use line — insert at top.
            break;
        }
    }
    match last_use_line {
        Some(line) => line + 1,
        None => 0,
    }
}

/// Extract the identifier name from a W200 diagnostic message.
///
/// Expected format: "'myFunc' should be snake_case"
fn extract_naming_identifier(message: &str) -> Option<&str> {
    let start = message.find('\'')?;
    let rest = &message[start + 1..];
    let end = rest.find('\'')?;
    Some(&rest[..end])
}

/// Build a code action that renames an identifier to snake_case.
fn snake_case_fix_action(
    state: &DocumentState,
    diag: &Diagnostic,
    uri: &Url,
) -> Option<CodeAction> {
    let ident = extract_naming_identifier(&diag.message)?;
    let snake = to_snake_case(ident);

    if snake == ident || snake.is_empty() {
        return None;
    }

    // Find all occurrences of the identifier in the source and replace them.
    let edits = find_identifier_edits_for_rename(&state.source, ident, &snake);
    if edits.is_empty() {
        return None;
    }

    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);

    Some(CodeAction {
        title: format!("Rename `{}` to `{}`", ident, snake),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        is_preferred: Some(true),
        ..Default::default()
    })
}

/// Find all word-boundary occurrences of `name` in `source` and return
/// `TextEdit`s replacing each with `new_name`.
fn find_identifier_edits_for_rename(source: &str, name: &str, new_name: &str) -> Vec<TextEdit> {
    use super::analysis::{char_col_to_utf16, is_ident_char};

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

            if before_ok && after_ok {
                // Convert byte offsets to char columns, then to UTF-16
                let start_char = line_text[..abs_byte_offset].chars().count() as u32;
                let end_char = start_char + name.chars().count() as u32;

                let start_utf16 = char_col_to_utf16(line_text, start_char);
                let end_utf16 = char_col_to_utf16(line_text, end_char);

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

/// Build a code action that sorts all `use` statements alphabetically.
///
/// Returns `None` if there are fewer than 2 `use` statements or they are
/// already sorted.
fn organize_imports_action(
    state: &DocumentState,
    uri: &Url,
) -> Option<CodeAction> {
    // Collect all use statement lines with their text
    let mut use_lines: Vec<(u32, String)> = Vec::new();

    for (idx, line) in state.source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("use ") {
            use_lines.push((idx as u32, line.to_string()));
        }
    }

    if use_lines.len() < 2 {
        return None;
    }

    // Check if already sorted
    let sorted_texts: Vec<String> = {
        let mut texts: Vec<String> = use_lines.iter().map(|(_, t)| t.clone()).collect();
        texts.sort_by(|a, b| {
            let a_trimmed = a.trim();
            let b_trimmed = b.trim();
            a_trimmed.cmp(b_trimmed)
        });
        texts
    };

    let current_texts: Vec<&String> = use_lines.iter().map(|(_, t)| t).collect();
    let is_sorted = current_texts.iter().zip(sorted_texts.iter()).all(|(a, b)| *a == b);

    if is_sorted {
        return None;
    }

    // Build text edits to replace each use line with the sorted version
    let mut edits = Vec::new();
    for (i, (line_num, _)) in use_lines.iter().enumerate() {
        let original_line = state.source.lines().nth(*line_num as usize).unwrap_or("");
        let original_len: u32 = original_line.chars().map(|c| c.len_utf16() as u32).sum();

        edits.push(TextEdit {
            range: Range {
                start: Position { line: *line_num, character: 0 },
                end: Position { line: *line_num, character: original_len },
            },
            new_text: sorted_texts[i].clone(),
        });
    }

    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);

    Some(CodeAction {
        title: "Organize imports".to_string(),
        kind: Some(CodeActionKind::SOURCE_ORGANIZE_IMPORTS),
        diagnostics: None,
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        is_preferred: Some(false),
        ..Default::default()
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Helper utilities
    // =========================================================================

    fn make_e201_diagnostic(func_name: &str, line: u32, col: u32) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position {
                    line,
                    character: col,
                },
                end: Position {
                    line,
                    character: col + func_name.len() as u32,
                },
            },
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String("E201".to_string())),
            source: Some("magi".to_string()),
            message: format!("Undefined function or operation '{}'", func_name),
            ..Default::default()
        }
    }

    fn make_w200_diagnostic(ident: &str, line: u32, col: u32) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position {
                    line,
                    character: col,
                },
                end: Position {
                    line,
                    character: col + ident.len() as u32,
                },
            },
            severity: Some(DiagnosticSeverity::WARNING),
            code: Some(NumberOrString::String("W200".to_string())),
            source: Some("magi".to_string()),
            message: format!("'{}' should be snake_case", ident),
            ..Default::default()
        }
    }

    fn make_state(source: &str) -> DocumentState {
        DocumentState {
            source: source.to_string(),
            program: None,
            functions: HashMap::new(),
            variables: HashMap::new(),
            enums: HashMap::new(),
            structs: HashMap::new(),
        }
    }

    fn make_params(diagnostics: Vec<Diagnostic>) -> (CodeActionParams, Url) {
        let uri = Url::parse("file:///test.magi").unwrap();
        let range = if let Some(d) = diagnostics.first() {
            d.range
        } else {
            Range::default()
        };
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range,
            context: CodeActionContext {
                diagnostics,
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        (params, uri)
    }

    // =========================================================================
    // Import suggestion tests
    // =========================================================================

    #[test]
    fn test_import_suggestion_for_known_std_function() {
        let source = "let x = sqrt(4.0);";
        let state = make_state(source);
        let diag = make_e201_diagnostic("sqrt", 0, 8);
        let (params, uri) = make_params(vec![diag]);

        let actions = handle_code_actions(&state, &params, &uri);
        assert_eq!(actions.len(), 1);
        if let CodeActionOrCommand::CodeAction(action) = &actions[0] {
            assert!(action.title.contains("sqrt"));
            assert!(action.title.contains("std::math"));
            assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
            // Verify the edit inserts the import
            let edit = action.edit.as_ref().unwrap();
            let changes = edit.changes.as_ref().unwrap();
            let edits = changes.get(&uri).unwrap();
            assert_eq!(edits.len(), 1);
            assert_eq!(edits[0].new_text, "use std::math::*;\n");
        } else {
            panic!("expected CodeAction");
        }
    }

    #[test]
    fn test_import_suggestion_after_existing_imports() {
        let source = "use std::str;\nlet x = array_push([], 1);";
        let state = make_state(source);
        let diag = make_e201_diagnostic("array_push", 1, 8);
        let (params, uri) = make_params(vec![diag]);

        let actions = handle_code_actions(&state, &params, &uri);
        assert_eq!(actions.len(), 1);
        if let CodeActionOrCommand::CodeAction(action) = &actions[0] {
            let edit = action.edit.as_ref().unwrap();
            let changes = edit.changes.as_ref().unwrap();
            let edits = changes.get(&uri).unwrap();
            // Should insert after the existing `use` line (line 1)
            assert_eq!(edits[0].range.start.line, 1);
            assert_eq!(edits[0].new_text, "use std::array::*;\n");
        } else {
            panic!("expected CodeAction");
        }
    }

    #[test]
    fn test_no_import_suggestion_for_unknown_function() {
        let source = "let x = totally_made_up_fn(42);";
        let state = make_state(source);
        let diag = make_e201_diagnostic("totally_made_up_fn", 0, 8);
        let (params, uri) = make_params(vec![diag]);

        let actions = handle_code_actions(&state, &params, &uri);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_import_suggestion_str_module() {
        let source = "let x = concat(\"a\", \"b\");";
        let state = make_state(source);
        let diag = make_e201_diagnostic("concat", 0, 8);
        let (params, uri) = make_params(vec![diag]);

        let actions = handle_code_actions(&state, &params, &uri);
        assert_eq!(actions.len(), 1);
        if let CodeActionOrCommand::CodeAction(action) = &actions[0] {
            assert!(action.title.contains("std::str"));
        } else {
            panic!("expected CodeAction");
        }
    }

    // =========================================================================
    // Snake case fix tests
    // =========================================================================

    #[test]
    fn test_snake_case_fix_action() {
        let source = "let myVar = 42;";
        let state = make_state(source);
        let diag = make_w200_diagnostic("myVar", 0, 4);
        let (params, uri) = make_params(vec![diag]);

        let actions = handle_code_actions(&state, &params, &uri);
        assert_eq!(actions.len(), 1);
        if let CodeActionOrCommand::CodeAction(action) = &actions[0] {
            assert!(action.title.contains("my_var"));
            assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
            let edit = action.edit.as_ref().unwrap();
            let changes = edit.changes.as_ref().unwrap();
            let edits = changes.get(&uri).unwrap();
            assert_eq!(edits.len(), 1);
            assert_eq!(edits[0].new_text, "my_var");
        } else {
            panic!("expected CodeAction");
        }
    }

    #[test]
    fn test_snake_case_fix_renames_all_occurrences() {
        let source = "let myVar = 42;\nlet y = myVar + 1;";
        let state = make_state(source);
        let diag = make_w200_diagnostic("myVar", 0, 4);
        let (params, uri) = make_params(vec![diag]);

        let actions = handle_code_actions(&state, &params, &uri);
        assert_eq!(actions.len(), 1);
        if let CodeActionOrCommand::CodeAction(action) = &actions[0] {
            let edit = action.edit.as_ref().unwrap();
            let changes = edit.changes.as_ref().unwrap();
            let edits = changes.get(&uri).unwrap();
            // Should rename both occurrences
            assert_eq!(edits.len(), 2);
            assert!(edits.iter().all(|e| e.new_text == "my_var"));
        } else {
            panic!("expected CodeAction");
        }
    }

    #[test]
    fn test_snake_case_fix_camel_case() {
        let source = "fn httpServer() {}";
        let state = make_state(source);
        let diag = make_w200_diagnostic("httpServer", 0, 3);
        let (params, uri) = make_params(vec![diag]);

        let actions = handle_code_actions(&state, &params, &uri);
        assert_eq!(actions.len(), 1);
        if let CodeActionOrCommand::CodeAction(action) = &actions[0] {
            assert!(action.title.contains("http_server"));
        } else {
            panic!("expected CodeAction");
        }
    }

    // =========================================================================
    // Multiple diagnostics
    // =========================================================================

    #[test]
    fn test_multiple_diagnostics_produce_multiple_actions() {
        let source = "let myVar = sqrt(4.0);";
        let state = make_state(source);
        let diag1 = make_w200_diagnostic("myVar", 0, 4);
        let diag2 = make_e201_diagnostic("sqrt", 0, 12);
        let (params, uri) = make_params(vec![diag1, diag2]);

        let actions = handle_code_actions(&state, &params, &uri);
        assert_eq!(actions.len(), 2);
    }

    #[test]
    fn test_no_actions_for_unrelated_diagnostic() {
        let source = "let x = 42;";
        let state = make_state(source);
        let diag = Diagnostic {
            range: Range::default(),
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String("E100".to_string())),
            source: Some("magi".to_string()),
            message: "undefined variable 'y'".to_string(),
            ..Default::default()
        };
        let (params, uri) = make_params(vec![diag]);

        let actions = handle_code_actions(&state, &params, &uri);
        assert!(actions.is_empty());
    }

    // =========================================================================
    // Helper function tests
    // =========================================================================

    #[test]
    fn test_extract_function_name() {
        assert_eq!(
            extract_function_name("Undefined function or operation 'sqrt'"),
            Some("sqrt")
        );
        assert_eq!(
            extract_function_name("Undefined function or operation 'array_push'"),
            Some("array_push")
        );
        assert_eq!(extract_function_name("no quotes here"), None);
    }

    #[test]
    fn test_extract_naming_identifier() {
        assert_eq!(
            extract_naming_identifier("'myVar' should be snake_case"),
            Some("myVar")
        );
        assert_eq!(extract_naming_identifier("no quotes"), None);
    }

    #[test]
    fn test_find_modules_for_function() {
        let modules = find_modules_for_function("sqrt");
        assert!(modules.contains(&"math"));

        let modules = find_modules_for_function("concat");
        assert!(modules.contains(&"str"));

        let modules = find_modules_for_function("totally_unknown_fn_xyz");
        assert!(modules.is_empty());
    }

    #[test]
    fn test_find_import_insert_line_no_imports() {
        assert_eq!(find_import_insert_line("let x = 1;"), 0);
    }

    #[test]
    fn test_find_import_insert_line_with_imports() {
        assert_eq!(
            find_import_insert_line("use std::math;\nuse std::str;\nlet x = 1;"),
            2
        );
    }

    #[test]
    fn test_find_import_insert_line_empty_source() {
        assert_eq!(find_import_insert_line(""), 0);
    }

    #[test]
    fn test_find_identifier_edits_word_boundary() {
        let edits = find_identifier_edits_for_rename("let myVar = myVarExtra;", "myVar", "my_var");
        // Should match only "myVar", not "myVarExtra"
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "my_var");
    }

    #[test]
    fn test_import_insert_respects_blank_line_gap() {
        let source = "use std::math;\n\nlet x = 1;";
        // The blank line separates the import block from code; insert after imports
        assert_eq!(find_import_insert_line(source), 1);
    }

    // =========================================================================
    // Organize imports tests
    // =========================================================================

    #[test]
    fn test_organize_imports_sorts_alphabetically() {
        let source = "use std::str::*;\nuse std::math::*;\nlet x = 1;";
        let state = make_state(source);
        let uri = Url::parse("file:///test.magi").unwrap();

        let action = organize_imports_action(&state, &uri);
        assert!(action.is_some());
        let action = action.unwrap();
        assert_eq!(action.title, "Organize imports");
        assert_eq!(action.kind, Some(CodeActionKind::SOURCE_ORGANIZE_IMPORTS));

        let edit = action.edit.unwrap();
        let changes = edit.changes.unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].new_text, "use std::math::*;");
        assert_eq!(edits[1].new_text, "use std::str::*;");
    }

    #[test]
    fn test_organize_imports_already_sorted() {
        let source = "use std::math::*;\nuse std::str::*;\nlet x = 1;";
        let state = make_state(source);
        let uri = Url::parse("file:///test.magi").unwrap();

        let action = organize_imports_action(&state, &uri);
        assert!(action.is_none(), "already sorted imports should not produce an action");
    }

    #[test]
    fn test_organize_imports_single_import() {
        let source = "use std::math::*;\nlet x = 1;";
        let state = make_state(source);
        let uri = Url::parse("file:///test.magi").unwrap();

        let action = organize_imports_action(&state, &uri);
        assert!(action.is_none(), "single import should not produce an action");
    }

    #[test]
    fn test_organize_imports_no_imports() {
        let source = "let x = 1;";
        let state = make_state(source);
        let uri = Url::parse("file:///test.magi").unwrap();

        let action = organize_imports_action(&state, &uri);
        assert!(action.is_none());
    }

    #[test]
    fn test_organize_imports_in_code_actions() {
        let source = "use std::str::*;\nuse std::math::*;\nlet x = 1;";
        let state = make_state(source);
        let (params, uri) = make_params(vec![]);

        let actions = handle_code_actions(&state, &params, &uri);
        // Should contain the organize imports action
        let organize = actions.iter().find(|a| {
            if let CodeActionOrCommand::CodeAction(ca) = a {
                ca.kind == Some(CodeActionKind::SOURCE_ORGANIZE_IMPORTS)
            } else {
                false
            }
        });
        assert!(organize.is_some());
    }
}
