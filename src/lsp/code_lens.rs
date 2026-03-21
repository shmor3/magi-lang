//! Code lens provider for the MAGI LSP.
//!
//! Provides "Run Test" code lens above `test "name" { ... }` blocks.

use super::analysis::DocumentState;
use crate::syntax::ast::StatementKind;
use tower_lsp::lsp_types::*;

/// Handle a code lens request.
///
/// Scans the AST for `TestDef` nodes and returns a `CodeLens` with a
/// "Run Test" title for each one.
pub fn handle_code_lens(state: &DocumentState) -> Vec<CodeLens> {
    let program = match &state.program {
        Some(p) => p,
        None => return Vec::new(),
    };

    let mut lenses = Vec::new();

    for stmt in &program.statements {
        if let StatementKind::TestDef { name, .. } = &stmt.kind {
            // Spans are 1-based; LSP positions are 0-based.
            let line = stmt.span.start_line.saturating_sub(1);
            let character = stmt.span.start_col.saturating_sub(1);

            let range = Range {
                start: Position { line, character },
                end: Position { line, character },
            };

            lenses.push(CodeLens {
                range,
                command: Some(Command {
                    title: "Run Test".to_string(),
                    command: "magi.runTest".to_string(),
                    arguments: Some(vec![serde_json::Value::String(name.clone())]),
                }),
                data: None,
            });
        }
    }

    lenses
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze_document;

    #[test]
    fn test_code_lens_single_test() {
        let source = "test \"addition\" {\n    assert(1 + 1 == 2)\n}";
        let (state, _) = analyze_document(source);
        let lenses = handle_code_lens(&state);
        assert_eq!(lenses.len(), 1);
        let lens = &lenses[0];
        assert_eq!(lens.command.as_ref().unwrap().title, "Run Test");
        assert_eq!(lens.range.start.line, 0);
    }

    #[test]
    fn test_code_lens_multiple_tests() {
        let source = "test \"a\" {\n    assert(true)\n}\ntest \"b\" {\n    assert(true)\n}";
        let (state, _) = analyze_document(source);
        let lenses = handle_code_lens(&state);
        assert_eq!(lenses.len(), 2);
        // Verify test names are passed as arguments
        let names: Vec<&str> = lenses
            .iter()
            .map(|l| {
                l.command.as_ref().unwrap().arguments.as_ref().unwrap()[0]
                    .as_str()
                    .unwrap()
            })
            .collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
    }

    #[test]
    fn test_code_lens_no_tests() {
        let source = "let x = 5;\nfn foo() { null }";
        let (state, _) = analyze_document(source);
        let lenses = handle_code_lens(&state);
        assert!(lenses.is_empty());
    }

    #[test]
    fn test_code_lens_empty_document() {
        let source = "";
        let (state, _) = analyze_document(source);
        let lenses = handle_code_lens(&state);
        assert!(lenses.is_empty());
    }

    #[test]
    fn test_code_lens_parse_error() {
        let source = "test {";
        let (state, _) = analyze_document(source);
        let lenses = handle_code_lens(&state);
        // Parse error — no valid AST, no lenses
        assert!(lenses.is_empty());
    }

    #[test]
    fn test_code_lens_command_arguments() {
        let source = "test \"my test\" {\n    assert(true)\n}";
        let (state, _) = analyze_document(source);
        let lenses = handle_code_lens(&state);
        assert_eq!(lenses.len(), 1);
        let cmd = lenses[0].command.as_ref().unwrap();
        assert_eq!(cmd.command, "magi.runTest");
        assert_eq!(
            cmd.arguments.as_ref().unwrap()[0],
            serde_json::Value::String("my test".to_string())
        );
    }
}
