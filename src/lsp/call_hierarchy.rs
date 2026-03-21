//! Call hierarchy provider for the MAGI LSP.
//!
//! Implements `prepareCallHierarchy`, `incomingCalls`, and `outgoingCalls`.
//! Walks the AST to discover function definitions, callers, and callees.

use super::analysis::{char_col_to_utf16, find_word_at_position, DocumentState};
use crate::syntax::ast::*;
use tower_lsp::lsp_types::*;

/// Handle a prepareCallHierarchy request.
///
/// If the cursor is on a function name (definition or call), returns a
/// `CallHierarchyItem` describing that function.
pub fn handle_prepare_call_hierarchy(
    state: &DocumentState,
    params: &CallHierarchyPrepareParams,
    uri: &Url,
) -> Option<Vec<CallHierarchyItem>> {
    let pos = params.text_document_position_params.position;
    let word = find_word_at_position(&state.source, pos.line, pos.character)?;

    // Check if it's a known function
    let func = state.functions.get(&word)?;

    let lsp_line = func.line.saturating_sub(1);
    let char_col = func.col.saturating_sub(1);
    let line_text = state.source.lines().nth(lsp_line as usize).unwrap_or("");
    let start_utf16 = char_col_to_utf16(line_text, char_col);
    let end_utf16 = char_col_to_utf16(line_text, char_col + word.chars().count() as u32);

    let selection_range = Range {
        start: Position { line: lsp_line, character: start_utf16 },
        end: Position { line: lsp_line, character: end_utf16 },
    };

    // For the full range, use the function's definition span if available from the AST
    let detail = if func.params.is_empty() {
        None
    } else {
        Some(format!("({})", func.params.join(", ")))
    };

    Some(vec![CallHierarchyItem {
        name: word.clone(),
        kind: SymbolKind::FUNCTION,
        tags: None,
        detail,
        uri: uri.clone(),
        range: selection_range,
        selection_range,
        data: None,
    }])
}

/// Handle an incomingCalls request.
///
/// Finds all functions that call the given function by walking the AST.
pub fn handle_incoming_calls(
    state: &DocumentState,
    params: &CallHierarchyIncomingCallsParams,
    uri: &Url,
) -> Vec<CallHierarchyIncomingCall> {
    let program = match &state.program {
        Some(p) => p,
        None => return Vec::new(),
    };

    let target_name = &params.item.name;
    let mut incoming = Vec::new();

    // Walk all function definitions to find which ones call the target
    for stmt in &program.statements {
        match &stmt.kind {
            StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
                let calls_in_body = find_calls_in_block(&fdef.body, target_name);
                if !calls_in_body.is_empty() {
                    let caller_line = fdef.span.start_line.saturating_sub(1);
                    let caller_col = fdef.span.start_col.saturating_sub(1);
                    let caller_line_text = state.source.lines().nth(caller_line as usize).unwrap_or("");
                    let caller_name_len = fdef.name.chars().count() as u32;

                    // Find the actual column of the function name
                    let name_col = find_name_in_line(caller_line_text, &fdef.name)
                        .unwrap_or(caller_col);
                    let name_start_utf16 = char_col_to_utf16(caller_line_text, name_col);
                    let name_end_utf16 = char_col_to_utf16(caller_line_text, name_col + caller_name_len);

                    let selection_range = Range {
                        start: Position { line: caller_line, character: name_start_utf16 },
                        end: Position { line: caller_line, character: name_end_utf16 },
                    };

                    // Convert call spans to LSP ranges
                    let from_ranges: Vec<Range> = calls_in_body.iter().map(|span| {
                        let call_line = span.start_line.saturating_sub(1);
                        let call_col = span.start_col.saturating_sub(1);
                        let call_line_text = state.source.lines().nth(call_line as usize).unwrap_or("");
                        let s = char_col_to_utf16(call_line_text, call_col);
                        let e = char_col_to_utf16(call_line_text, call_col + target_name.chars().count() as u32);
                        Range {
                            start: Position { line: call_line, character: s },
                            end: Position { line: call_line, character: e },
                        }
                    }).collect();

                    incoming.push(CallHierarchyIncomingCall {
                        from: CallHierarchyItem {
                            name: fdef.name.clone(),
                            kind: SymbolKind::FUNCTION,
                            tags: None,
                            detail: None,
                            uri: uri.clone(),
                            range: selection_range,
                            selection_range,
                            data: None,
                        },
                        from_ranges,
                    });
                }
            }
            _ => {}
        }
    }

    // Also check top-level code (not inside any function)
    let top_level_calls = find_calls_in_statements(&program.statements, target_name);
    if !top_level_calls.is_empty() {
        let from_ranges: Vec<Range> = top_level_calls.iter().map(|span| {
            let call_line = span.start_line.saturating_sub(1);
            let call_col = span.start_col.saturating_sub(1);
            let call_line_text = state.source.lines().nth(call_line as usize).unwrap_or("");
            let s = char_col_to_utf16(call_line_text, call_col);
            let e = char_col_to_utf16(call_line_text, call_col + target_name.chars().count() as u32);
            Range {
                start: Position { line: call_line, character: s },
                end: Position { line: call_line, character: e },
            }
        }).collect();

        incoming.push(CallHierarchyIncomingCall {
            from: CallHierarchyItem {
                name: "<module>".to_string(),
                kind: SymbolKind::MODULE,
                tags: None,
                detail: None,
                uri: uri.clone(),
                range: Range::default(),
                selection_range: Range::default(),
                data: None,
            },
            from_ranges,
        });
    }

    incoming
}

/// Handle an outgoingCalls request.
///
/// For the given function, finds all functions it calls by walking its body.
pub fn handle_outgoing_calls(
    state: &DocumentState,
    params: &CallHierarchyOutgoingCallsParams,
    uri: &Url,
) -> Vec<CallHierarchyOutgoingCall> {
    let program = match &state.program {
        Some(p) => p,
        None => return Vec::new(),
    };

    let func_name = &params.item.name;

    // Find the function definition in the AST
    let fdef = program.statements.iter().find_map(|stmt| {
        match &stmt.kind {
            StatementKind::FunctionDef(f) | StatementKind::AsyncFunctionDef(f) => {
                if f.name == *func_name { Some(f) } else { None }
            }
            _ => None,
        }
    });

    let fdef = match fdef {
        Some(f) => f,
        None => return Vec::new(),
    };

    // Collect all unique function calls in the body
    let mut call_map: std::collections::HashMap<String, Vec<Span>> = std::collections::HashMap::new();
    collect_calls_in_block(&fdef.body, &mut call_map);

    let mut outgoing = Vec::new();
    for (callee_name, call_spans) in &call_map {
        // Try to find the callee definition
        let (callee_range, callee_kind) = if let Some(callee_func) = state.functions.get(callee_name) {
            let line = callee_func.line.saturating_sub(1);
            let col = callee_func.col.saturating_sub(1);
            let line_text = state.source.lines().nth(line as usize).unwrap_or("");
            let s = char_col_to_utf16(line_text, col);
            let e = char_col_to_utf16(line_text, col + callee_name.chars().count() as u32);
            (Range {
                start: Position { line, character: s },
                end: Position { line, character: e },
            }, SymbolKind::FUNCTION)
        } else {
            // Unknown function (possibly a built-in)
            (Range::default(), SymbolKind::FUNCTION)
        };

        let from_ranges: Vec<Range> = call_spans.iter().map(|span| {
            let call_line = span.start_line.saturating_sub(1);
            let call_col = span.start_col.saturating_sub(1);
            let call_line_text = state.source.lines().nth(call_line as usize).unwrap_or("");
            let s = char_col_to_utf16(call_line_text, call_col);
            let e = char_col_to_utf16(call_line_text, call_col + callee_name.chars().count() as u32);
            Range {
                start: Position { line: call_line, character: s },
                end: Position { line: call_line, character: e },
            }
        }).collect();

        outgoing.push(CallHierarchyOutgoingCall {
            to: CallHierarchyItem {
                name: callee_name.clone(),
                kind: callee_kind,
                tags: None,
                detail: None,
                uri: uri.clone(),
                range: callee_range,
                selection_range: callee_range,
                data: None,
            },
            from_ranges,
        });
    }

    outgoing
}

// =============================================================================
// AST walking helpers
// =============================================================================

/// Find all calls to `target_name` in a block, returning their spans.
fn find_calls_in_block(block: &Block, target_name: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    for stmt in &block.statements {
        find_calls_in_statement(stmt, target_name, &mut spans);
    }
    if let Some(ref tail) = block.tail_expr {
        find_calls_in_expr(tail, target_name, &mut spans);
    }
    spans
}

/// Find calls to `target_name` in top-level statements (excluding function bodies).
fn find_calls_in_statements(stmts: &[Statement], target_name: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    for stmt in stmts {
        // Skip function definitions (those are handled separately as callers)
        match &stmt.kind {
            StatementKind::FunctionDef(_) | StatementKind::AsyncFunctionDef(_) => continue,
            _ => {}
        }
        find_calls_in_statement(stmt, target_name, &mut spans);
    }
    spans
}

fn find_calls_in_statement(stmt: &Statement, target_name: &str, spans: &mut Vec<Span>) {
    match &stmt.kind {
        StatementKind::Let { value, .. }
        | StatementKind::LetMut { value, .. }
        | StatementKind::ConstDef { value, .. }
        | StatementKind::Assignment { value, .. }
        | StatementKind::Output(value)
        | StatementKind::ExprStatement(value)
        | StatementKind::Return(Some(value))
        | StatementKind::Throw(value)
        | StatementKind::Defer(value) => {
            find_calls_in_expr(value, target_name, spans);
        }
        StatementKind::LetDestructure { value, .. } => {
            find_calls_in_expr(value, target_name, spans);
        }
        StatementKind::CompoundAssign { value, .. } => {
            find_calls_in_expr(value, target_name, spans);
        }
        StatementKind::FieldAssignment { object, value, .. } => {
            find_calls_in_expr(object, target_name, spans);
            find_calls_in_expr(value, target_name, spans);
        }
        StatementKind::IndexAssignment { object, index, value } => {
            find_calls_in_expr(object, target_name, spans);
            find_calls_in_expr(index, target_name, spans);
            find_calls_in_expr(value, target_name, spans);
        }
        StatementKind::ForLoop { iterable, body, .. } => {
            find_calls_in_expr(iterable, target_name, spans);
            spans.extend(find_calls_in_block(body, target_name));
        }
        StatementKind::WhileLoop { condition, body, .. } => {
            find_calls_in_expr(condition, target_name, spans);
            spans.extend(find_calls_in_block(body, target_name));
        }
        StatementKind::DoWhileLoop { body, condition, .. } => {
            spans.extend(find_calls_in_block(body, target_name));
            find_calls_in_expr(condition, target_name, spans);
        }
        StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
            spans.extend(find_calls_in_block(&fdef.body, target_name));
        }
        StatementKind::TryCatch { try_block, catch_block, finally_block, .. } => {
            spans.extend(find_calls_in_block(try_block, target_name));
            spans.extend(find_calls_in_block(catch_block, target_name));
            if let Some(fb) = finally_block {
                spans.extend(find_calls_in_block(fb, target_name));
            }
        }
        StatementKind::CStyleFor { init, condition, update, body } => {
            find_calls_in_statement(init, target_name, spans);
            find_calls_in_expr(condition, target_name, spans);
            find_calls_in_statement(update, target_name, spans);
            spans.extend(find_calls_in_block(body, target_name));
        }
        StatementKind::ImplBlock { methods, .. } => {
            for method in methods {
                spans.extend(find_calls_in_block(&method.body, target_name));
            }
        }
        StatementKind::ImplTrait { methods, .. } => {
            for method in methods {
                spans.extend(find_calls_in_block(&method.body, target_name));
            }
        }
        StatementKind::TestDef { body, .. } | StatementKind::ModuleDef { body, .. } => {
            spans.extend(find_calls_in_block(body, target_name));
        }
        StatementKind::TupleAssignment { value, .. } => {
            find_calls_in_expr(value, target_name, spans);
        }
        StatementKind::Return(None)
        | StatementKind::Break { .. }
        | StatementKind::Continue { .. }
        | StatementKind::Import(_)
        | StatementKind::Use { .. }
        | StatementKind::TypeAlias { .. }
        | StatementKind::EnumDef { .. }
        | StatementKind::StructDef { .. }
        | StatementKind::TraitDef { .. } => {}
    }
}

fn find_calls_in_expr(expr: &Expression, target_name: &str, spans: &mut Vec<Span>) {
    match &expr.kind {
        ExpressionKind::Call { name, args, kwargs } => {
            if name == target_name {
                spans.push(expr.span);
            }
            for arg in args {
                find_calls_in_expr(arg, target_name, spans);
            }
            for (_, v) in kwargs {
                find_calls_in_expr(v, target_name, spans);
            }
        }
        ExpressionKind::MethodCall { object, args, kwargs, .. } => {
            find_calls_in_expr(object, target_name, spans);
            for arg in args {
                find_calls_in_expr(arg, target_name, spans);
            }
            for (_, v) in kwargs {
                find_calls_in_expr(v, target_name, spans);
            }
        }
        ExpressionKind::BinaryOp { left, right, .. } => {
            find_calls_in_expr(left, target_name, spans);
            find_calls_in_expr(right, target_name, spans);
        }
        ExpressionKind::UnaryOp { operand, .. } => {
            find_calls_in_expr(operand, target_name, spans);
        }
        ExpressionKind::Pipe { left, right } => {
            find_calls_in_expr(left, target_name, spans);
            find_calls_in_expr(right, target_name, spans);
        }
        ExpressionKind::IfElse { condition, then_block, else_block } => {
            find_calls_in_expr(condition, target_name, spans);
            spans.extend(find_calls_in_block(then_block, target_name));
            if let Some(eb) = else_block {
                spans.extend(find_calls_in_block(eb, target_name));
            }
        }
        ExpressionKind::Block(block) => {
            spans.extend(find_calls_in_block(block, target_name));
        }
        ExpressionKind::Index { object, index } => {
            find_calls_in_expr(object, target_name, spans);
            find_calls_in_expr(index, target_name, spans);
        }
        ExpressionKind::FieldAccess { object, .. } | ExpressionKind::OptionalChain { object, .. } => {
            find_calls_in_expr(object, target_name, spans);
        }
        ExpressionKind::Range { start, end, .. } => {
            find_calls_in_expr(start, target_name, spans);
            find_calls_in_expr(end, target_name, spans);
        }
        ExpressionKind::Await(inner) | ExpressionKind::Spawn(inner)
        | ExpressionKind::Spread(inner) | ExpressionKind::TryPropagate(inner) => {
            find_calls_in_expr(inner, target_name, spans);
        }
        ExpressionKind::Lambda { body, .. } => {
            find_calls_in_expr(body, target_name, spans);
        }
        ExpressionKind::Match { value, arms } => {
            find_calls_in_expr(value, target_name, spans);
            for arm in arms {
                spans.extend(find_calls_in_block(&arm.body, target_name));
                if let Some(ref guard) = arm.guard {
                    find_calls_in_expr(guard, target_name, spans);
                }
            }
        }
        ExpressionKind::NullCoalesce { left, right } => {
            find_calls_in_expr(left, target_name, spans);
            find_calls_in_expr(right, target_name, spans);
        }
        ExpressionKind::StringInterpolation { parts } => {
            for part in parts {
                if let StringPart::Expr(e) = part {
                    find_calls_in_expr(e, target_name, spans);
                }
            }
        }
        ExpressionKind::Loop { body, .. } => {
            spans.extend(find_calls_in_block(body, target_name));
        }
        ExpressionKind::TryCatchExpr { try_block, catch_block, finally_block, .. } => {
            spans.extend(find_calls_in_block(try_block, target_name));
            spans.extend(find_calls_in_block(catch_block, target_name));
            if let Some(fb) = finally_block {
                spans.extend(find_calls_in_block(fb, target_name));
            }
        }
        ExpressionKind::ListComprehension { expr, iterable, condition, .. } => {
            find_calls_in_expr(expr, target_name, spans);
            find_calls_in_expr(iterable, target_name, spans);
            if let Some(c) = condition {
                find_calls_in_expr(c, target_name, spans);
            }
        }
        ExpressionKind::MapComprehension { key_expr, value_expr, iterable, condition, .. } => {
            find_calls_in_expr(key_expr, target_name, spans);
            find_calls_in_expr(value_expr, target_name, spans);
            find_calls_in_expr(iterable, target_name, spans);
            if let Some(c) = condition {
                find_calls_in_expr(c, target_name, spans);
            }
        }
        ExpressionKind::EnumConstruct { args, .. } => {
            for arg in args {
                find_calls_in_expr(arg, target_name, spans);
            }
        }
        ExpressionKind::StructConstruct { fields, .. } => {
            for (_, v) in fields {
                find_calls_in_expr(v, target_name, spans);
            }
        }
        ExpressionKind::Literal(lit) => {
            match lit {
                Literal::Array(elements) => {
                    for e in elements {
                        find_calls_in_expr(e, target_name, spans);
                    }
                }
                Literal::Map(entries) => {
                    for (_k, v) in entries {
                        find_calls_in_expr(v, target_name, spans);
                    }
                }
                _ => {}
            }
        }
        ExpressionKind::Variable(_) | ExpressionKind::Placeholder => {}
    }
}

/// Collect all function calls in a block, grouped by callee name.
fn collect_calls_in_block(block: &Block, call_map: &mut std::collections::HashMap<String, Vec<Span>>) {
    for stmt in &block.statements {
        collect_calls_in_statement(stmt, call_map);
    }
    if let Some(ref tail) = block.tail_expr {
        collect_calls_in_expr(tail, call_map);
    }
}

fn collect_calls_in_statement(stmt: &Statement, call_map: &mut std::collections::HashMap<String, Vec<Span>>) {
    match &stmt.kind {
        StatementKind::Let { value, .. }
        | StatementKind::LetMut { value, .. }
        | StatementKind::ConstDef { value, .. }
        | StatementKind::Assignment { value, .. }
        | StatementKind::Output(value)
        | StatementKind::ExprStatement(value)
        | StatementKind::Return(Some(value))
        | StatementKind::Throw(value)
        | StatementKind::Defer(value) => {
            collect_calls_in_expr(value, call_map);
        }
        StatementKind::LetDestructure { value, .. } => {
            collect_calls_in_expr(value, call_map);
        }
        StatementKind::CompoundAssign { value, .. } => {
            collect_calls_in_expr(value, call_map);
        }
        StatementKind::FieldAssignment { object, value, .. } => {
            collect_calls_in_expr(object, call_map);
            collect_calls_in_expr(value, call_map);
        }
        StatementKind::IndexAssignment { object, index, value } => {
            collect_calls_in_expr(object, call_map);
            collect_calls_in_expr(index, call_map);
            collect_calls_in_expr(value, call_map);
        }
        StatementKind::ForLoop { iterable, body, .. } => {
            collect_calls_in_expr(iterable, call_map);
            collect_calls_in_block(body, call_map);
        }
        StatementKind::WhileLoop { condition, body, .. } => {
            collect_calls_in_expr(condition, call_map);
            collect_calls_in_block(body, call_map);
        }
        StatementKind::DoWhileLoop { body, condition, .. } => {
            collect_calls_in_block(body, call_map);
            collect_calls_in_expr(condition, call_map);
        }
        StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
            collect_calls_in_block(&fdef.body, call_map);
        }
        StatementKind::TryCatch { try_block, catch_block, finally_block, .. } => {
            collect_calls_in_block(try_block, call_map);
            collect_calls_in_block(catch_block, call_map);
            if let Some(fb) = finally_block {
                collect_calls_in_block(fb, call_map);
            }
        }
        StatementKind::CStyleFor { init, condition, update, body } => {
            collect_calls_in_statement(init, call_map);
            collect_calls_in_expr(condition, call_map);
            collect_calls_in_statement(update, call_map);
            collect_calls_in_block(body, call_map);
        }
        StatementKind::TestDef { body, .. } | StatementKind::ModuleDef { body, .. } => {
            collect_calls_in_block(body, call_map);
        }
        _ => {}
    }
}

fn collect_calls_in_expr(expr: &Expression, call_map: &mut std::collections::HashMap<String, Vec<Span>>) {
    match &expr.kind {
        ExpressionKind::Call { name, args, kwargs } => {
            call_map.entry(name.clone()).or_default().push(expr.span);
            for arg in args {
                collect_calls_in_expr(arg, call_map);
            }
            for (_, v) in kwargs {
                collect_calls_in_expr(v, call_map);
            }
        }
        ExpressionKind::MethodCall { object, args, kwargs, .. } => {
            collect_calls_in_expr(object, call_map);
            for arg in args {
                collect_calls_in_expr(arg, call_map);
            }
            for (_, v) in kwargs {
                collect_calls_in_expr(v, call_map);
            }
        }
        ExpressionKind::BinaryOp { left, right, .. } => {
            collect_calls_in_expr(left, call_map);
            collect_calls_in_expr(right, call_map);
        }
        ExpressionKind::UnaryOp { operand, .. } => {
            collect_calls_in_expr(operand, call_map);
        }
        ExpressionKind::Pipe { left, right } => {
            collect_calls_in_expr(left, call_map);
            collect_calls_in_expr(right, call_map);
        }
        ExpressionKind::IfElse { condition, then_block, else_block } => {
            collect_calls_in_expr(condition, call_map);
            collect_calls_in_block(then_block, call_map);
            if let Some(eb) = else_block {
                collect_calls_in_block(eb, call_map);
            }
        }
        ExpressionKind::Block(block) => {
            collect_calls_in_block(block, call_map);
        }
        ExpressionKind::Index { object, index } => {
            collect_calls_in_expr(object, call_map);
            collect_calls_in_expr(index, call_map);
        }
        ExpressionKind::FieldAccess { object, .. } | ExpressionKind::OptionalChain { object, .. } => {
            collect_calls_in_expr(object, call_map);
        }
        ExpressionKind::Range { start, end, .. } => {
            collect_calls_in_expr(start, call_map);
            collect_calls_in_expr(end, call_map);
        }
        ExpressionKind::Await(inner) | ExpressionKind::Spawn(inner)
        | ExpressionKind::Spread(inner) | ExpressionKind::TryPropagate(inner) => {
            collect_calls_in_expr(inner, call_map);
        }
        ExpressionKind::Lambda { body, .. } => {
            collect_calls_in_expr(body, call_map);
        }
        ExpressionKind::Match { value, arms } => {
            collect_calls_in_expr(value, call_map);
            for arm in arms {
                collect_calls_in_block(&arm.body, call_map);
                if let Some(ref guard) = arm.guard {
                    collect_calls_in_expr(guard, call_map);
                }
            }
        }
        ExpressionKind::NullCoalesce { left, right } => {
            collect_calls_in_expr(left, call_map);
            collect_calls_in_expr(right, call_map);
        }
        ExpressionKind::StringInterpolation { parts } => {
            for part in parts {
                if let StringPart::Expr(e) = part {
                    collect_calls_in_expr(e, call_map);
                }
            }
        }
        ExpressionKind::Loop { body, .. } => {
            collect_calls_in_block(body, call_map);
        }
        ExpressionKind::TryCatchExpr { try_block, catch_block, finally_block, .. } => {
            collect_calls_in_block(try_block, call_map);
            collect_calls_in_block(catch_block, call_map);
            if let Some(fb) = finally_block {
                collect_calls_in_block(fb, call_map);
            }
        }
        ExpressionKind::ListComprehension { expr, iterable, condition, .. } => {
            collect_calls_in_expr(expr, call_map);
            collect_calls_in_expr(iterable, call_map);
            if let Some(c) = condition {
                collect_calls_in_expr(c, call_map);
            }
        }
        ExpressionKind::MapComprehension { key_expr, value_expr, iterable, condition, .. } => {
            collect_calls_in_expr(key_expr, call_map);
            collect_calls_in_expr(value_expr, call_map);
            collect_calls_in_expr(iterable, call_map);
            if let Some(c) = condition {
                collect_calls_in_expr(c, call_map);
            }
        }
        ExpressionKind::EnumConstruct { args, .. } => {
            for arg in args {
                collect_calls_in_expr(arg, call_map);
            }
        }
        ExpressionKind::StructConstruct { fields, .. } => {
            for (_, v) in fields {
                collect_calls_in_expr(v, call_map);
            }
        }
        ExpressionKind::Literal(lit) => {
            match lit {
                Literal::Array(elements) => {
                    for e in elements {
                        collect_calls_in_expr(e, call_map);
                    }
                }
                Literal::Map(entries) => {
                    for (_k, v) in entries {
                        collect_calls_in_expr(v, call_map);
                    }
                }
                _ => {}
            }
        }
        ExpressionKind::Variable(_) | ExpressionKind::Placeholder => {}
    }
}

/// Find the 0-based char column of a name in a line text.
fn find_name_in_line(line_text: &str, name: &str) -> Option<u32> {
    let mut start = 0;
    while let Some(offset) = line_text[start..].find(name) {
        let abs = start + offset;
        let after = abs + name.len();
        let before_ok = abs == 0
            || !line_text[..abs].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_');
        let after_ok = after >= line_text.len()
            || !line_text[after..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_');
        if before_ok && after_ok {
            return Some(line_text[..abs].chars().count() as u32);
        }
        start = abs + name.len().max(1);
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

    fn make_prepare_params(line: u32, character: u32) -> CallHierarchyPrepareParams {
        CallHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: test_uri() },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        }
    }

    #[test]
    fn test_prepare_call_hierarchy_on_function() {
        let source = "fn greet() { null }\ngreet()";
        let (state, _) = analyze_document(source);
        let params = make_prepare_params(0, 3); // cursor on "greet" definition
        let result = handle_prepare_call_hierarchy(&state, &params, &test_uri());
        assert!(result.is_some());
        let items = result.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "greet");
        assert_eq!(items[0].kind, SymbolKind::FUNCTION);
    }

    #[test]
    fn test_prepare_call_hierarchy_unknown() {
        let source = "let x = 5;";
        let (state, _) = analyze_document(source);
        let params = make_prepare_params(0, 4); // cursor on "x" (a variable, not a function)
        let result = handle_prepare_call_hierarchy(&state, &params, &test_uri());
        assert!(result.is_none());
    }

    #[test]
    fn test_outgoing_calls() {
        let source = "fn helper() { null }\nfn main() { helper() }";
        let (state, _) = analyze_document(source);
        let item = CallHierarchyItem {
            name: "main".to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            detail: None,
            uri: test_uri(),
            range: Range::default(),
            selection_range: Range::default(),
            data: None,
        };
        let params = CallHierarchyOutgoingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let result = handle_outgoing_calls(&state, &params, &test_uri());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].to.name, "helper");
    }

    #[test]
    fn test_incoming_calls() {
        let source = "fn helper() { null }\nfn main() { helper() }\nfn other() { helper() }";
        let (state, _) = analyze_document(source);
        let item = CallHierarchyItem {
            name: "helper".to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            detail: None,
            uri: test_uri(),
            range: Range::default(),
            selection_range: Range::default(),
            data: None,
        };
        let params = CallHierarchyIncomingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let result = handle_incoming_calls(&state, &params, &test_uri());
        // main and other both call helper
        assert_eq!(result.len(), 2);
        let names: Vec<&str> = result.iter().map(|c| c.from.name.as_str()).collect();
        assert!(names.contains(&"main"));
        assert!(names.contains(&"other"));
    }

    #[test]
    fn test_incoming_calls_from_top_level() {
        let source = "fn helper() { null }\nhelper()";
        let (state, _) = analyze_document(source);
        let item = CallHierarchyItem {
            name: "helper".to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            detail: None,
            uri: test_uri(),
            range: Range::default(),
            selection_range: Range::default(),
            data: None,
        };
        let params = CallHierarchyIncomingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let result = handle_incoming_calls(&state, &params, &test_uri());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].from.name, "<module>");
    }

    #[test]
    fn test_outgoing_calls_none_for_unknown_function() {
        let source = "fn greet() { null }";
        let (state, _) = analyze_document(source);
        let item = CallHierarchyItem {
            name: "nonexistent".to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            detail: None,
            uri: test_uri(),
            range: Range::default(),
            selection_range: Range::default(),
            data: None,
        };
        let params = CallHierarchyOutgoingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let result = handle_outgoing_calls(&state, &params, &test_uri());
        assert!(result.is_empty());
    }
}
