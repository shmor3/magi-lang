//! Folding range provider for the MAGI LSP.
//!
//! Walks the AST to find foldable constructs (functions, structs, enums,
//! modules, control flow blocks, match expressions, try/catch) and returns
//! `FoldingRange` entries for each.

use super::analysis::DocumentState;
use crate::syntax::ast::*;
use super::types::{FoldingRange, FoldingRangeKind};

/// Compute folding ranges for a document from its parsed AST.
pub fn handle_folding_ranges(state: &DocumentState) -> Vec<FoldingRange> {
    let program = match &state.program {
        Some(p) => p,
        None => return Vec::new(),
    };

    let mut ranges = Vec::new();
    for stmt in &program.statements {
        collect_statement_folding_ranges(stmt, &mut ranges);
    }

    // Sort by start_line for deterministic output.
    ranges.sort_by_key(|r| (r.start_line, r.end_line));
    ranges
}

/// Push a folding range if the construct spans multiple lines.
fn push_range(ranges: &mut Vec<FoldingRange>, span: &Span, kind: FoldingRangeKind) {
    // Spans are 1-based; LSP folding ranges are 0-based.
    let start_line = span.start_line.saturating_sub(1);
    let end_line = span.end_line.saturating_sub(1);

    // Only fold if the construct spans at least two lines.
    if end_line > start_line {
        ranges.push(FoldingRange {
            start_line,
            start_character: None,
            end_line,
            end_character: None,
            kind: Some(kind),
            collapsed_text: None,
        });
    }
}

/// Collect folding ranges from a single statement.
fn collect_statement_folding_ranges(stmt: &Statement, ranges: &mut Vec<FoldingRange>) {
    match &stmt.kind {
        StatementKind::FunctionDef(func_def) | StatementKind::AsyncFunctionDef(func_def) => {
            push_range(ranges, &func_def.span, FoldingRangeKind::REGION);
            collect_block_folding_ranges(&func_def.body, ranges);
        }

        StatementKind::StructDef { .. } => {
            push_range(ranges, &stmt.span, FoldingRangeKind::REGION);
        }

        StatementKind::EnumDef { .. } => {
            push_range(ranges, &stmt.span, FoldingRangeKind::REGION);
        }

        StatementKind::ModuleDef { body, .. } => {
            push_range(ranges, &stmt.span, FoldingRangeKind::REGION);
            collect_block_folding_ranges(body, ranges);
        }

        StatementKind::ForLoop { body, iterable, .. } => {
            push_range(ranges, &stmt.span, FoldingRangeKind::REGION);
            collect_expression_folding_ranges(iterable, ranges);
            collect_block_folding_ranges(body, ranges);
        }

        StatementKind::WhileLoop { condition, body, .. } => {
            push_range(ranges, &stmt.span, FoldingRangeKind::REGION);
            collect_expression_folding_ranges(condition, ranges);
            collect_block_folding_ranges(body, ranges);
        }

        StatementKind::DoWhileLoop { body, condition, .. } | StatementKind::CStyleFor { body, condition, .. } => {
            push_range(ranges, &stmt.span, FoldingRangeKind::REGION);
            collect_block_folding_ranges(body, ranges);
            collect_expression_folding_ranges(condition, ranges);
        }

        StatementKind::Defer(expr) => {
            collect_expression_folding_ranges(expr, ranges);
        }

        StatementKind::TryCatch {
            try_block,
            catch_block,
            finally_block,
            ..
        } => {
            push_range(ranges, &stmt.span, FoldingRangeKind::REGION);
            collect_block_folding_ranges(try_block, ranges);
            collect_block_folding_ranges(catch_block, ranges);
            if let Some(fb) = finally_block {
                collect_block_folding_ranges(fb, ranges);
            }
        }

        StatementKind::TestDef { body, .. } => {
            push_range(ranges, &stmt.span, FoldingRangeKind::REGION);
            collect_block_folding_ranges(body, ranges);
        }

        StatementKind::Let { value, .. }
        | StatementKind::LetMut { value, .. }
        | StatementKind::Assignment { value, .. }
        | StatementKind::ConstDef { value, .. }
        | StatementKind::StaticDef { value, .. }
        | StatementKind::CompoundAssign { value, .. } => {
            collect_expression_folding_ranges(value, ranges);
        }

        StatementKind::LetDestructure { value, .. } => {
            collect_expression_folding_ranges(value, ranges);
        }

        StatementKind::ExprStatement(expr)
        | StatementKind::Output(expr)
        | StatementKind::Throw(expr) => {
            collect_expression_folding_ranges(expr, ranges);
        }

        StatementKind::Return(Some(expr)) => {
            collect_expression_folding_ranges(expr, ranges);
        }

        StatementKind::Break { value: Some(expr), .. } => {
            collect_expression_folding_ranges(expr, ranges);
        }

        // Leaf statements and other kinds with no foldable children.
        _ => {}
    }
}

/// Collect folding ranges from a block's statements and tail expression.
fn collect_block_folding_ranges(block: &Block, ranges: &mut Vec<FoldingRange>) {
    for stmt in &block.statements {
        collect_statement_folding_ranges(stmt, ranges);
    }
    if let Some(tail) = &block.tail_expr {
        collect_expression_folding_ranges(tail, ranges);
    }
}

/// Collect folding ranges from expressions that may contain foldable constructs.
fn collect_expression_folding_ranges(expr: &Expression, ranges: &mut Vec<FoldingRange>) {
    match &expr.kind {
        ExpressionKind::IfElse {
            condition,
            then_block,
            else_block,
        } => {
            push_range(ranges, &expr.span, FoldingRangeKind::REGION);
            collect_expression_folding_ranges(condition, ranges);
            collect_block_folding_ranges(then_block, ranges);
            if let Some(eb) = else_block {
                collect_block_folding_ranges(eb, ranges);
            }
        }

        ExpressionKind::Match { value, arms } => {
            push_range(ranges, &expr.span, FoldingRangeKind::REGION);
            collect_expression_folding_ranges(value, ranges);
            for arm in arms {
                // Each arm body is a Block; fold it if multiline.
                push_range(ranges, &arm.span, FoldingRangeKind::REGION);
                collect_block_folding_ranges(&arm.body, ranges);
                if let Some(guard) = &arm.guard {
                    collect_expression_folding_ranges(guard, ranges);
                }
            }
        }

        ExpressionKind::Loop { body, .. } => {
            push_range(ranges, &expr.span, FoldingRangeKind::REGION);
            collect_block_folding_ranges(body, ranges);
        }

        ExpressionKind::TryCatchExpr {
            try_block,
            catch_block,
            finally_block,
            ..
        } => {
            push_range(ranges, &expr.span, FoldingRangeKind::REGION);
            collect_block_folding_ranges(try_block, ranges);
            collect_block_folding_ranges(catch_block, ranges);
            if let Some(fb) = finally_block {
                collect_block_folding_ranges(fb, ranges);
            }
        }

        ExpressionKind::Block(block) => {
            push_range(ranges, &expr.span, FoldingRangeKind::REGION);
            collect_block_folding_ranges(block, ranges);
        }

        ExpressionKind::Lambda { body, params: _, .. } => {
            push_range(ranges, &expr.span, FoldingRangeKind::REGION);
            collect_expression_folding_ranges(body, ranges);
        }

        ExpressionKind::BinaryOp { left, right, .. }
        | ExpressionKind::Pipe { left, right }
        | ExpressionKind::NullCoalesce { left, right } => {
            collect_expression_folding_ranges(left, ranges);
            collect_expression_folding_ranges(right, ranges);
        }

        ExpressionKind::UnaryOp { operand, .. }
        | ExpressionKind::Await(operand)
        | ExpressionKind::Spawn(operand)
        | ExpressionKind::Spread(operand)
        | ExpressionKind::TryPropagate(operand)
        | ExpressionKind::Yield(operand) => {
            collect_expression_folding_ranges(operand, ranges);
        }

        ExpressionKind::UnsafeBlock(block) => {
            push_range(ranges, &expr.span, FoldingRangeKind::REGION);
            collect_block_folding_ranges(block, ranges);
        }

        ExpressionKind::InlineAsm { operands, .. } => {
            for op in operands {
                collect_expression_folding_ranges(op, ranges);
            }
        }

        ExpressionKind::Call { args, .. } => {
            for arg in args {
                collect_expression_folding_ranges(arg, ranges);
            }
        }

        ExpressionKind::MethodCall { object, args, .. } => {
            collect_expression_folding_ranges(object, ranges);
            for arg in args {
                collect_expression_folding_ranges(arg, ranges);
            }
        }

        ExpressionKind::Index { object, index } => {
            collect_expression_folding_ranges(object, ranges);
            collect_expression_folding_ranges(index, ranges);
        }

        ExpressionKind::FieldAccess { object, .. }
        | ExpressionKind::OptionalChain { object, .. } => {
            collect_expression_folding_ranges(object, ranges);
        }

        ExpressionKind::Range { start, end, .. } => {
            collect_expression_folding_ranges(start, ranges);
            collect_expression_folding_ranges(end, ranges);
        }

        ExpressionKind::ListComprehension {
            expr: inner,
            iterable,
            condition,
            ..
        } => {
            collect_expression_folding_ranges(inner, ranges);
            collect_expression_folding_ranges(iterable, ranges);
            if let Some(cond) = condition {
                collect_expression_folding_ranges(cond, ranges);
            }
        }

        ExpressionKind::MapComprehension {
            key_expr,
            value_expr,
            iterable,
            condition,
            ..
        } => {
            collect_expression_folding_ranges(key_expr, ranges);
            collect_expression_folding_ranges(value_expr, ranges);
            collect_expression_folding_ranges(iterable, ranges);
            if let Some(cond) = condition {
                collect_expression_folding_ranges(cond, ranges);
            }
        }

        ExpressionKind::EnumConstruct { args, .. } => {
            for arg in args {
                collect_expression_folding_ranges(arg, ranges);
            }
        }

        ExpressionKind::StructConstruct { fields, .. } => {
            for (_, field_expr) in fields {
                collect_expression_folding_ranges(field_expr, ranges);
            }
        }

        ExpressionKind::StringInterpolation { parts } => {
            for part in parts {
                if let StringPart::Expr(e) = part {
                    collect_expression_folding_ranges(e, ranges);
                }
            }
        }

        ExpressionKind::Literal(Literal::Array(elems)) => {
            for elem in elems {
                collect_expression_folding_ranges(elem, ranges);
            }
        }

        ExpressionKind::Literal(Literal::Map(entries)) => {
            for (_, val) in entries {
                collect_expression_folding_ranges(val, ranges);
            }
        }

        ExpressionKind::Ref(inner) | ExpressionKind::MoveClosure { body: inner, .. } => {
            collect_expression_folding_ranges(inner, ranges);
        }

        ExpressionKind::TupleLiteral(exprs) => {
            for e in exprs {
                collect_expression_folding_ranges(e, ranges);
            }
        }

        // Leaf expressions: no foldable children.
        ExpressionKind::Literal(_)
        | ExpressionKind::Variable(_)
        | ExpressionKind::Placeholder
        | ExpressionKind::DynTrait(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze_document;

    fn folding_ranges_for(source: &str) -> Vec<FoldingRange> {
        let (state, _) = analyze_document(source);
        handle_folding_ranges(&state)
    }

    #[test]
    fn test_function_def_folding() {
        let source = "fn foo() {\n    let x = 1;\n    x\n}";
        let ranges = folding_ranges_for(source);
        assert!(
            !ranges.is_empty(),
            "expected at least one folding range for a multiline function"
        );
        let r = &ranges[0];
        assert_eq!(r.start_line, 0);
        assert_eq!(r.end_line, 3);
        assert_eq!(r.kind, Some(FoldingRangeKind::REGION));
    }

    #[test]
    fn test_async_function_def_folding() {
        let source = "async fn fetch() {\n    let x = 1;\n    x\n}";
        let ranges = folding_ranges_for(source);
        assert!(
            !ranges.is_empty(),
            "expected folding range for async function"
        );
        assert_eq!(ranges[0].start_line, 0);
    }

    #[test]
    fn test_struct_def_folding() {
        let source = "struct Point {\n    x: float64,\n    y: float64,\n}";
        let ranges = folding_ranges_for(source);
        assert!(
            !ranges.is_empty(),
            "expected folding range for multiline struct"
        );
        assert_eq!(ranges[0].start_line, 0);
    }

    #[test]
    fn test_enum_def_folding() {
        let source = "enum Color {\n    Red,\n    Green,\n    Blue,\n}";
        let ranges = folding_ranges_for(source);
        assert!(
            !ranges.is_empty(),
            "expected folding range for multiline enum"
        );
        assert_eq!(ranges[0].start_line, 0);
    }

    #[test]
    fn test_module_def_folding() {
        let source = "mod math {\n    fn add(a, b) {\n        a + b\n    }\n}";
        let ranges = folding_ranges_for(source);
        // Should have a range for the module and the inner function.
        assert!(
            ranges.len() >= 2,
            "expected folding ranges for module and inner function, got {}",
            ranges.len()
        );
    }

    #[test]
    fn test_if_else_folding() {
        let source = "let x = if true {\n    1\n} else {\n    2\n};";
        let ranges = folding_ranges_for(source);
        assert!(
            !ranges.is_empty(),
            "expected folding range for if/else expression"
        );
    }

    #[test]
    fn test_for_loop_folding() {
        let source = "for i in 0..10 {\n    let x = i;\n}";
        let ranges = folding_ranges_for(source);
        assert!(
            !ranges.is_empty(),
            "expected folding range for for loop"
        );
        assert_eq!(ranges[0].start_line, 0);
    }

    #[test]
    fn test_while_loop_folding() {
        let source = "let mut i = 0;\nwhile i < 10 {\n    i += 1;\n}";
        let ranges = folding_ranges_for(source);
        assert!(
            !ranges.is_empty(),
            "expected folding range for while loop"
        );
    }

    #[test]
    fn test_loop_expression_folding() {
        let source = "let x = loop {\n    break 42;\n};";
        let ranges = folding_ranges_for(source);
        assert!(
            !ranges.is_empty(),
            "expected folding range for loop expression"
        );
    }

    #[test]
    fn test_match_expression_folding() {
        let source = "let x = match 1 {\n    1 => { \"one\" }\n    _ => { \"other\" }\n};";
        let ranges = folding_ranges_for(source);
        assert!(
            !ranges.is_empty(),
            "expected folding range for match expression"
        );
    }

    #[test]
    fn test_try_catch_statement_folding() {
        let source = "try {\n    let x = 1;\n} catch err {\n    let y = 2;\n}";
        let ranges = folding_ranges_for(source);
        assert!(
            !ranges.is_empty(),
            "expected folding range for try/catch"
        );
    }

    #[test]
    fn test_single_line_no_folding() {
        let source = "let x = 5;";
        let ranges = folding_ranges_for(source);
        assert!(
            ranges.is_empty(),
            "single-line statements should not produce folding ranges"
        );
    }

    #[test]
    fn test_single_line_function_no_folding() {
        let source = "fn id(x) { x }";
        let ranges = folding_ranges_for(source);
        // A single-line function should not produce a folding range.
        assert!(
            ranges.is_empty(),
            "single-line function should not produce a folding range"
        );
    }

    #[test]
    fn test_empty_program_no_folding() {
        let ranges = folding_ranges_for("");
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_no_program_no_folding() {
        // Unparseable source: no program produced.
        let state = DocumentState {
            source: "fn {{{".to_string(),
            program: None,
            functions: Default::default(),
            variables: Default::default(),
            enums: Default::default(),
            structs: Default::default(),
        };
        let ranges = handle_folding_ranges(&state);
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_nested_functions_multiple_ranges() {
        let source = "fn outer() {\n    fn inner() {\n        null\n    }\n    null\n}";
        let ranges = folding_ranges_for(source);
        assert!(
            ranges.len() >= 2,
            "expected at least 2 folding ranges for nested functions, got {}",
            ranges.len()
        );
    }

    #[test]
    fn test_test_def_folding() {
        let source = "test \"my test\" {\n    assert(true);\n}";
        let ranges = folding_ranges_for(source);
        assert!(
            !ranges.is_empty(),
            "expected folding range for test definition"
        );
    }

    #[test]
    fn test_ranges_are_sorted() {
        let source = "fn b() {\n    null\n}\nfn a() {\n    null\n}";
        let ranges = folding_ranges_for(source);
        for window in ranges.windows(2) {
            assert!(
                (window[0].start_line, window[0].end_line)
                    <= (window[1].start_line, window[1].end_line),
                "folding ranges should be sorted by start_line"
            );
        }
    }
}
