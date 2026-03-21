//! Selection range provider for the MAGI LSP.
//!
//! Walks the AST to find the narrowest node containing the cursor position,
//! then builds a chain of parent nodes as nested `SelectionRange` entries.

use super::analysis::DocumentState;
use crate::syntax::ast::*;
use tower_lsp::lsp_types::{Position, Range, SelectionRange};

/// Handle a selection range request for the given cursor positions.
///
/// For each position, walks the AST to find nested spans containing
/// the cursor, from narrowest to widest, and returns a chain of
/// `SelectionRange` values.
pub fn handle_selection_ranges(
    state: &DocumentState,
    positions: &[Position],
) -> Vec<SelectionRange> {
    let program = match &state.program {
        Some(p) => p,
        None => {
            // No AST — return a trivial range for each position.
            return positions
                .iter()
                .map(|pos| SelectionRange {
                    range: Range::new(*pos, *pos),
                    parent: None,
                })
                .collect();
        }
    };

    positions
        .iter()
        .map(|pos| {
            let mut spans = Vec::new();

            // Collect all AST node spans that contain the cursor.
            // Positions in LSP are 0-based; AST spans are 1-based.
            let cursor_line = pos.line + 1;
            let cursor_col = pos.character + 1;

            // Program-level span
            if span_contains(&program.span, cursor_line, cursor_col) {
                spans.push(program.span);
            }

            for stmt in &program.statements {
                collect_statement_spans(stmt, cursor_line, cursor_col, &mut spans);
            }

            // Sort spans from widest to narrowest (by area).
            spans.sort_by(|a, b| {
                let a_size = span_size(a);
                let b_size = span_size(b);
                b_size.cmp(&a_size)
            });

            // Deduplicate identical spans.
            spans.dedup();

            // Build the chain from narrowest (innermost) to widest (outermost).
            // The result is a linked list: narrowest -> ... -> widest.
            let mut current: Option<SelectionRange> = None;

            for span in &spans {
                let range = span_to_range(span);
                current = Some(SelectionRange {
                    range,
                    parent: current.map(Box::new),
                });
            }

            // Reverse: we built widest-first, but need narrowest-first.
            // The chain is already in the right order since we iterate
            // from widest to narrowest, building inside-out.
            // Actually, we want the outermost link to be the narrowest span.
            // Let's rebuild correctly.
            build_selection_chain(&spans)
        })
        .collect()
}

/// Build a `SelectionRange` chain from spans ordered widest-to-narrowest.
/// The returned `SelectionRange` represents the narrowest span, with
/// its parent being the next-wider span, etc.
fn build_selection_chain(spans_widest_first: &[Span]) -> SelectionRange {
    if spans_widest_first.is_empty() {
        return SelectionRange {
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
            parent: None,
        };
    }

    // Start from the widest span (first in array) as the outermost parent.
    let mut chain = SelectionRange {
        range: span_to_range(&spans_widest_first[0]),
        parent: None,
    };

    // Wrap each subsequent (narrower) span around the current chain.
    for span in &spans_widest_first[1..] {
        chain = SelectionRange {
            range: span_to_range(span),
            parent: Some(Box::new(chain)),
        };
    }

    chain
}

/// Check if a 1-based span contains a 1-based (line, col) position.
fn span_contains(span: &Span, line: u32, col: u32) -> bool {
    if line < span.start_line || line > span.end_line {
        return false;
    }
    if line == span.start_line && col < span.start_col {
        return false;
    }
    if line == span.end_line && col > span.end_col {
        return false;
    }
    true
}

/// Compute a rough "size" of a span for sorting (wider spans get larger values).
fn span_size(span: &Span) -> u64 {
    let lines = (span.end_line.saturating_sub(span.start_line)) as u64;
    let cols = if span.start_line == span.end_line {
        span.end_col.saturating_sub(span.start_col) as u64
    } else {
        span.end_col as u64
    };
    lines * 10000 + cols
}

/// Convert a 1-based AST span to a 0-based LSP range.
fn span_to_range(span: &Span) -> Range {
    Range {
        start: Position {
            line: span.start_line.saturating_sub(1),
            character: span.start_col.saturating_sub(1),
        },
        end: Position {
            line: span.end_line.saturating_sub(1),
            character: span.end_col.saturating_sub(1),
        },
    }
}

/// Collect all spans from a statement and its children that contain the cursor.
fn collect_statement_spans(stmt: &Statement, line: u32, col: u32, spans: &mut Vec<Span>) {
    if !span_contains(&stmt.span, line, col) {
        return;
    }
    spans.push(stmt.span);

    match &stmt.kind {
        StatementKind::Let { value, .. }
        | StatementKind::LetMut { value, .. }
        | StatementKind::ConstDef { value, .. } => {
            collect_expr_spans(value, line, col, spans);
        }
        StatementKind::LetDestructure { value, .. } => {
            collect_expr_spans(value, line, col, spans);
        }
        StatementKind::Assignment { value, .. }
        | StatementKind::Output(value)
        | StatementKind::ExprStatement(value)
        | StatementKind::Throw(value) => {
            collect_expr_spans(value, line, col, spans);
        }
        StatementKind::CompoundAssign { value, .. } => {
            collect_expr_spans(value, line, col, spans);
        }
        StatementKind::FieldAssignment { object, value, .. } => {
            collect_expr_spans(object, line, col, spans);
            collect_expr_spans(value, line, col, spans);
        }
        StatementKind::IndexAssignment { object, index, value } => {
            collect_expr_spans(object, line, col, spans);
            collect_expr_spans(index, line, col, spans);
            collect_expr_spans(value, line, col, spans);
        }
        StatementKind::ForLoop { iterable, body, .. } => {
            collect_expr_spans(iterable, line, col, spans);
            collect_block_spans(body, line, col, spans);
        }
        StatementKind::WhileLoop { condition, body, .. } => {
            collect_expr_spans(condition, line, col, spans);
            collect_block_spans(body, line, col, spans);
        }
        StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
            collect_block_spans(&fdef.body, line, col, spans);
        }
        StatementKind::Return(Some(expr)) | StatementKind::Break { label: None, value: Some(expr) } => {
            collect_expr_spans(expr, line, col, spans);
        }
        StatementKind::TryCatch {
            try_block,
            catch_block,
            finally_block,
            ..
        } => {
            collect_block_spans(try_block, line, col, spans);
            collect_block_spans(catch_block, line, col, spans);
            if let Some(fb) = finally_block {
                collect_block_spans(fb, line, col, spans);
            }
        }
        StatementKind::TestDef { body, .. } => {
            collect_block_spans(body, line, col, spans);
        }
        StatementKind::ModuleDef { body, .. } => {
            collect_block_spans(body, line, col, spans);
        }
        _ => {}
    }
}

/// Collect all spans from an expression and its children that contain the cursor.
fn collect_expr_spans(expr: &Expression, line: u32, col: u32, spans: &mut Vec<Span>) {
    if !span_contains(&expr.span, line, col) {
        return;
    }
    spans.push(expr.span);

    match &expr.kind {
        ExpressionKind::BinaryOp { left, right, .. } => {
            collect_expr_spans(left, line, col, spans);
            collect_expr_spans(right, line, col, spans);
        }
        ExpressionKind::UnaryOp { operand, .. } => {
            collect_expr_spans(operand, line, col, spans);
        }
        ExpressionKind::Call { args, .. } => {
            for arg in args {
                collect_expr_spans(arg, line, col, spans);
            }
        }
        ExpressionKind::MethodCall { object, args, .. } => {
            collect_expr_spans(object, line, col, spans);
            for arg in args {
                collect_expr_spans(arg, line, col, spans);
            }
        }
        ExpressionKind::Pipe { left, right } => {
            collect_expr_spans(left, line, col, spans);
            collect_expr_spans(right, line, col, spans);
        }
        ExpressionKind::IfElse {
            condition,
            then_block,
            else_block,
        } => {
            collect_expr_spans(condition, line, col, spans);
            collect_block_spans(then_block, line, col, spans);
            if let Some(eb) = else_block {
                collect_block_spans(eb, line, col, spans);
            }
        }
        ExpressionKind::Block(block) => {
            collect_block_spans(block, line, col, spans);
        }
        ExpressionKind::Index { object, index } => {
            collect_expr_spans(object, line, col, spans);
            collect_expr_spans(index, line, col, spans);
        }
        ExpressionKind::FieldAccess { object, .. } => {
            collect_expr_spans(object, line, col, spans);
        }
        ExpressionKind::Range { start, end, .. } => {
            collect_expr_spans(start, line, col, spans);
            collect_expr_spans(end, line, col, spans);
        }
        ExpressionKind::Await(inner) | ExpressionKind::Spawn(inner) => {
            collect_expr_spans(inner, line, col, spans);
        }
        ExpressionKind::Lambda { body, .. } => {
            collect_expr_spans(body, line, col, spans);
        }
        ExpressionKind::Match { value, arms } => {
            collect_expr_spans(value, line, col, spans);
            for arm in arms {
                collect_block_spans(&arm.body, line, col, spans);
            }
        }
        ExpressionKind::NullCoalesce { left, right } => {
            collect_expr_spans(left, line, col, spans);
            collect_expr_spans(right, line, col, spans);
        }
        ExpressionKind::OptionalChain { object, .. } => {
            collect_expr_spans(object, line, col, spans);
        }
        ExpressionKind::Spread(inner) | ExpressionKind::TryPropagate(inner) => {
            collect_expr_spans(inner, line, col, spans);
        }
        ExpressionKind::Loop { body: block, .. } => {
            collect_block_spans(block, line, col, spans);
        }
        ExpressionKind::TryCatchExpr {
            try_block,
            catch_block,
            finally_block,
            ..
        } => {
            collect_block_spans(try_block, line, col, spans);
            collect_block_spans(catch_block, line, col, spans);
            if let Some(fb) = finally_block {
                collect_block_spans(fb, line, col, spans);
            }
        }
        ExpressionKind::ListComprehension {
            expr, iterable, condition, ..
        } => {
            collect_expr_spans(expr, line, col, spans);
            collect_expr_spans(iterable, line, col, spans);
            if let Some(cond) = condition {
                collect_expr_spans(cond, line, col, spans);
            }
        }
        ExpressionKind::MapComprehension {
            key_expr,
            value_expr,
            iterable,
            condition,
            ..
        } => {
            collect_expr_spans(key_expr, line, col, spans);
            collect_expr_spans(value_expr, line, col, spans);
            collect_expr_spans(iterable, line, col, spans);
            if let Some(cond) = condition {
                collect_expr_spans(cond, line, col, spans);
            }
        }
        ExpressionKind::EnumConstruct { args, .. } => {
            for arg in args {
                collect_expr_spans(arg, line, col, spans);
            }
        }
        ExpressionKind::StructConstruct { fields, .. } => {
            for (_, field_expr) in fields {
                collect_expr_spans(field_expr, line, col, spans);
            }
        }
        ExpressionKind::StringInterpolation { parts } => {
            for part in parts {
                if let StringPart::Expr(e) = part {
                    collect_expr_spans(e, line, col, spans);
                }
            }
        }
        ExpressionKind::Literal(Literal::Array(elems)) => {
            for elem in elems {
                collect_expr_spans(elem, line, col, spans);
            }
        }
        ExpressionKind::Literal(Literal::Map(entries)) => {
            for (_k, v) in entries {
                collect_expr_spans(v, line, col, spans);
            }
        }
        _ => {}
    }
}

/// Collect spans from a block that contain the cursor.
fn collect_block_spans(block: &Block, line: u32, col: u32, spans: &mut Vec<Span>) {
    if !span_contains(&block.span, line, col) {
        return;
    }
    spans.push(block.span);

    for stmt in &block.statements {
        collect_statement_spans(stmt, line, col, spans);
    }
    if let Some(tail) = &block.tail_expr {
        collect_expr_spans(tail, line, col, spans);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze_document;

    #[test]
    fn test_selection_range_simple() {
        let source = "let x = 42";
        let (state, _) = analyze_document(source);
        let ranges = handle_selection_ranges(&state, &[Position::new(0, 4)]);
        assert_eq!(ranges.len(), 1);
        // Should have at least one range (the statement or program)
        let r = &ranges[0];
        assert!(r.range.start.line == 0);
    }

    #[test]
    fn test_selection_range_nested_function() {
        let source = "fn foo() {\n    let x = 1 + 2\n    x\n}";
        let (state, _) = analyze_document(source);
        // Cursor on "1" inside the function body
        let ranges = handle_selection_ranges(&state, &[Position::new(1, 12)]);
        assert_eq!(ranges.len(), 1);
        let r = &ranges[0];
        // The narrowest range should be contained within the function
        // and there should be a parent chain
        assert!(r.parent.is_some());
    }

    #[test]
    fn test_selection_range_no_ast() {
        let source = "";
        let (state, _) = analyze_document(source);
        let ranges = handle_selection_ranges(&state, &[Position::new(0, 0)]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].range.start, Position::new(0, 0));
    }

    #[test]
    fn test_selection_range_multiple_positions() {
        let source = "let a = 1\nlet b = 2";
        let (state, _) = analyze_document(source);
        let ranges = handle_selection_ranges(
            &state,
            &[Position::new(0, 0), Position::new(1, 0)],
        );
        assert_eq!(ranges.len(), 2);
    }

    #[test]
    fn test_span_contains() {
        let span = Span::new(1, 1, 3, 10);
        assert!(span_contains(&span, 1, 1));
        assert!(span_contains(&span, 2, 5));
        assert!(span_contains(&span, 3, 10));
        assert!(!span_contains(&span, 0, 1));
        assert!(!span_contains(&span, 4, 1));
        assert!(!span_contains(&span, 1, 0));
        assert!(!span_contains(&span, 3, 11));
    }

    #[test]
    fn test_span_to_range() {
        let span = Span::new(1, 1, 3, 10);
        let range = span_to_range(&span);
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
        assert_eq!(range.end.line, 2);
        assert_eq!(range.end.character, 9);
    }
}
