//! Inlay hints provider for the MAGI LSP.
//!
//! Shows type annotations for variable bindings.

use super::analysis::DocumentState;
use tower_lsp::lsp_types::*;

/// Handle an inlay hints request for a given range of the document.
pub fn handle_inlay_hints(state: &DocumentState, range: &Range) -> Vec<InlayHint> {
    let mut hints = Vec::new();

    // Add variable type hints for `let` bindings without explicit type annotations
    for var in state.variables.values() {
        let line = var.line;

        // Skip if outside requested range
        if line < range.start.line || line > range.end.line {
            continue;
        }

        // Skip variables that already have type annotations
        if var.type_annotation.is_some() {
            continue;
        }

        // Skip type aliases and module imports
        if var.is_type_alias {
            continue;
        }

        // For untyped let bindings, show a placeholder hint
        // (full type inference would require running the type checker)
        let position = Position {
            line,
            character: var.col + var.name.len() as u32,
        };
        hints.push(InlayHint {
            position,
            label: InlayHintLabel::String(": _".to_string()),
            kind: Some(InlayHintKind::TYPE),
            text_edits: None,
            tooltip: None,
            padding_left: None,
            padding_right: Some(true),
            data: None,
        });
    }

    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze_document;

    fn full_range() -> Range {
        Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 1000,
                character: 0,
            },
        }
    }

    #[test]
    fn test_inlay_hints_type_annotation() {
        let source = "let x = 42";
        let (state, _) = analyze_document(source);
        let hints = handle_inlay_hints(&state, &full_range());
        let type_hints: Vec<_> = hints
            .iter()
            .filter(|h| h.kind == Some(InlayHintKind::TYPE))
            .collect();
        assert!(
            !type_hints.is_empty(),
            "should show type hint for let x = 42"
        );
    }

    #[test]
    fn test_inlay_hints_skip_annotated() {
        let source = "let x: int64 = 42";
        let (state, _) = analyze_document(source);
        let hints = handle_inlay_hints(&state, &full_range());
        let type_hints: Vec<_> = hints
            .iter()
            .filter(|h| h.kind == Some(InlayHintKind::TYPE))
            .collect();
        assert!(
            type_hints.is_empty(),
            "should not show hints for annotated variables"
        );
    }

    #[test]
    fn test_inlay_hints_range_filtering() {
        let source = "let a = 1\nlet b = 2\nlet c = 3";
        let (state, _) = analyze_document(source);
        // Only request hints for line 1
        let range = Range {
            start: Position {
                line: 1,
                character: 0,
            },
            end: Position {
                line: 1,
                character: 100,
            },
        };
        let hints = handle_inlay_hints(&state, &range);
        for hint in &hints {
            assert_eq!(hint.position.line, 1);
        }
    }
}
