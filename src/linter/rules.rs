//! Individual lint rule implementations.

use crate::syntax::ast::*;
use crate::syntax::errors::ErrorCode;
use crate::syntax::type_checker::AstDiagnostic;
use crate::eval::DiagnosticSeverity;

/// Convert a name to snake_case, handling acronyms correctly.
/// e.g. "HTTPServer" → "http_server", "myFunc" → "my_func"
fn to_snake_case(name: &str) -> String {
    use heck::ToSnakeCase;
    name.to_snake_case()
}

/// Check that a name uses snake_case (for functions and variables).
/// Also accepts SCREAMING_SNAKE_CASE (e.g., `MAX_SIZE`) for constants.
pub fn check_naming_snake_case(name: &str, span: Span) -> Option<AstDiagnostic> {
    // Skip names starting with _ (conventional suppression) or single-char names
    if name.starts_with('_') || name.len() <= 1 {
        return None;
    }
    // snake_case: only lowercase letters, digits, and underscores
    let is_snake = name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if is_snake {
        return None;
    }
    // SCREAMING_SNAKE_CASE: only uppercase letters, digits, and underscores (for constants)
    let is_screaming = name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if is_screaming {
        return None;
    }
    let code = ErrorCode::W200;
    let suggestion = to_snake_case(name);
    Some(AstDiagnostic {
        line: span.start_line,
        column: span.start_col,
        message: format!("'{}' should be snake_case", name),
        severity: DiagnosticSeverity::Warning,
        code: Some(code.to_string()),
        help: Some(code.help().to_string()),
        suggestion: Some(format!("Rename to `{}`", suggestion)),
    })
}

/// Check that a name uses PascalCase (for enums and structs).
pub fn check_naming_pascal_case(name: &str, span: Span) -> Option<AstDiagnostic> {
    if name.is_empty() || name.starts_with('_') {
        return None;
    }
    // PascalCase: starts with uppercase, no underscores
    let first_upper = name.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    let no_underscores = !name.contains('_');
    if first_upper && no_underscores {
        return None;
    }
    let code = ErrorCode::W201;
    Some(AstDiagnostic {
        line: span.start_line,
        column: span.start_col,
        message: format!("'{}' should be PascalCase", name),
        severity: DiagnosticSeverity::Warning,
        code: Some(code.to_string()),
        help: Some(code.help().to_string()),
        suggestion: None,
    })
}

/// Check for dead code after return/break/continue/throw in a block's statements.
/// Returns diagnostics for any statements that appear after a terminating statement.
pub fn check_dead_code_in_block(stmts: &[Statement]) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut terminated = false;

    for stmt in stmts {
        if terminated {
            let code = ErrorCode::W202;
            diagnostics.push(AstDiagnostic {
                line: stmt.span.start_line,
                column: stmt.span.start_col,
                message: "unreachable code".to_string(),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: None,
            });
            continue; // Report all unreachable statements, not just the first
        }

        match &stmt.kind {
            StatementKind::Return(_)
            | StatementKind::Break(_)
            | StatementKind::Continue
            | StatementKind::Throw(_) => {
                terminated = true;
            }
            // An expression-statement that is an if/else where both branches
            // terminate is also a terminator.
            StatementKind::ExprStatement(expr) => {
                if is_terminating_expr(expr) {
                    terminated = true;
                }
            }
            // A try/catch where both blocks terminate is a terminator.
            StatementKind::TryCatch { try_block, catch_block, .. } => {
                if is_terminating_block(try_block) && is_terminating_block(catch_block) {
                    terminated = true;
                }
            }
            _ => {}
        }
    }

    diagnostics
}

/// Check for constant conditions in if/while expressions.
/// For `while true`, suppress the warning if the body contains a `break` statement,
/// since `while true { ... break; }` is a common idiom for complex loop conditions.
pub fn check_constant_condition(condition: &Expression, loop_body: Option<&Block>) -> Option<AstDiagnostic> {
    if let ExpressionKind::Literal(Literal::Bool(val)) = &condition.kind {
        // Suppress W204 for `while true` with a break — it's an idiomatic pattern
        if *val {
            if let Some(body) = loop_body {
                if block_contains_break(body) {
                    return None;
                }
            }
        }
        let code = ErrorCode::W204;
        return Some(AstDiagnostic {
            line: condition.span.start_line,
            column: condition.span.start_col,
            message: format!("condition is always `{}`", val),
            severity: DiagnosticSeverity::Warning,
            code: Some(code.to_string()),
            help: Some(code.help().to_string()),
            suggestion: None,
        });
    }
    None
}

/// Returns true if a block (or any of its nested non-loop branches) contains a `break`.
/// Does NOT recurse into nested loops since their breaks target the inner loop.
fn block_contains_break(block: &Block) -> bool {
    for stmt in &block.statements {
        match &stmt.kind {
            StatementKind::Break(_) => return true,
            StatementKind::TryCatch { try_block, catch_block, .. } => {
                if block_contains_break(try_block) || block_contains_break(catch_block) {
                    return true;
                }
            }
            // Don't recurse into nested loops — their breaks target the inner loop.
            StatementKind::ForLoop { .. } | StatementKind::WhileLoop { .. } => {}
            StatementKind::ExprStatement(expr) => {
                if expr_contains_break(expr) {
                    return true;
                }
            }
            _ => {}
        }
    }
    if let Some(tail) = &block.tail_expr {
        if expr_contains_break(tail) {
            return true;
        }
    }
    false
}

/// Returns true if an expression (non-loop) contains a break statement.
fn expr_contains_break(expr: &Expression) -> bool {
    match &expr.kind {
        ExpressionKind::IfElse { then_block, else_block, .. } => {
            if block_contains_break(then_block) { return true; }
            if let Some(eb) = else_block {
                if block_contains_break(eb) { return true; }
            }
            false
        }
        ExpressionKind::Block(block) => block_contains_break(block),
        ExpressionKind::Match { arms, .. } => {
            arms.iter().any(|arm| block_contains_break(&arm.body))
        }
        // Don't recurse into loop expressions — their breaks are for the inner loop.
        ExpressionKind::Loop(_) => false,
        ExpressionKind::TryCatchExpr { try_block, catch_block, .. } => {
            block_contains_break(try_block) || block_contains_break(catch_block)
        }
        _ => false,
    }
}

/// Check for empty block bodies (excluding blocks that have a tail expression).
pub fn check_empty_block(block: &Block, context: &str, span: Span) -> Option<AstDiagnostic> {
    if block.statements.is_empty() && block.tail_expr.is_none() {
        let code = ErrorCode::W206;
        return Some(AstDiagnostic {
            line: span.start_line,
            column: span.start_col,
            message: format!("empty {} body", context),
            severity: DiagnosticSeverity::Warning,
            code: Some(code.to_string()),
            help: Some(code.help().to_string()),
            suggestion: None,
        });
    }
    None
}

/// Check for unreachable match arms after a wildcard or unguarded variable pattern.
pub fn check_unreachable_arms(arms: &[MatchArm]) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut seen_catch_all = false;

    for arm in arms {
        if seen_catch_all {
            let code = ErrorCode::W207;
            diagnostics.push(AstDiagnostic {
                line: arm.span.start_line,
                column: arm.span.start_col,
                message: "unreachable match arm after wildcard".to_string(),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: None,
            });
            continue;
        }

        // A wildcard, unguarded variable, or Or-pattern containing a catch-all
        if arm.guard.is_none() && is_catch_all_pattern(&arm.pattern) {
            seen_catch_all = true;
        }
    }

    diagnostics
}

/// Check for duplicate imports in a list of statements.
pub fn check_duplicate_imports(stmts: &[Statement]) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for stmt in stmts {
        let path: Option<String> = match &stmt.kind {
            StatementKind::Import(path) => Some(path.clone()),
            StatementKind::Use { path, .. } => Some(path.join("::")),
            _ => None,
        };
        if let Some(path) = path {
            if !seen.insert(path.clone()) {
                let code = ErrorCode::W208;
                diagnostics.push(AstDiagnostic {
                    line: stmt.span.start_line,
                    column: stmt.span.start_col,
                    message: format!("duplicate import: \"{}\"", path),
                    severity: DiagnosticSeverity::Warning,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: None,
                });
            }
        }
    }

    diagnostics
}

/// Returns true if an expression always terminates (return/break/continue/throw in all paths).
fn is_terminating_expr(expr: &Expression) -> bool {
    match &expr.kind {
        ExpressionKind::IfElse { then_block, else_block: Some(else_block), .. } => {
            is_terminating_block(then_block) && is_terminating_block(else_block)
        }
        ExpressionKind::Block(block) => is_terminating_block(block),
        // A match where every arm terminates is itself terminating,
        // but only if there's a catch-all (wildcard/variable) to guarantee coverage.
        ExpressionKind::Match { arms, .. } => {
            let has_catch_all = arms.iter().any(|a| a.guard.is_none() && is_catch_all_pattern(&a.pattern));
            has_catch_all && arms.iter().all(|a| is_terminating_block(&a.body))
        }
        // A loop always either runs forever or exits via break/return/throw,
        // so code after it is unreachable if the loop body terminates unconditionally.
        ExpressionKind::Loop(block) => is_terminating_block(block),
        // A try/catch expression where both blocks terminate is a terminator.
        ExpressionKind::TryCatchExpr { try_block, catch_block, .. } => {
            is_terminating_block(try_block) && is_terminating_block(catch_block)
        }
        _ => false,
    }
}

/// Returns true if a block always terminates.
fn is_terminating_block(block: &Block) -> bool {
    for stmt in &block.statements {
        match &stmt.kind {
            StatementKind::Return(_)
            | StatementKind::Break(_)
            | StatementKind::Continue
            | StatementKind::Throw(_) => return true,
            StatementKind::ExprStatement(expr) => {
                if is_terminating_expr(expr) {
                    return true;
                }
            }
            _ => {}
        }
    }
    // Check tail expression
    if let Some(tail) = &block.tail_expr {
        return is_terminating_expr(tail);
    }
    false
}

/// Returns true if a pattern is a catch-all (matches everything).
fn is_catch_all_pattern(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::Wildcard => true,
        Pattern::Variable(_) => true,
        Pattern::Or(alternatives) => alternatives.iter().any(is_catch_all_pattern),
        _ => false,
    }
}

/// Recursively collect enum variant names from a pattern, including inside Or-patterns.
fn collect_enum_variants(
    pattern: &Pattern,
    map: &mut std::collections::HashMap<String, Vec<String>>,
) {
    match pattern {
        Pattern::EnumPattern { enum_name, variant, .. } => {
            map.entry(enum_name.clone()).or_default().push(variant.clone());
        }
        Pattern::Or(alternatives) => {
            for alt in alternatives {
                collect_enum_variants(alt, map);
            }
        }
        _ => {}
    }
}

/// Check match exhaustiveness for enum patterns.
/// If all arms use EnumPattern for the same enum and there's no wildcard/variable catch-all,
/// check that all variants of that enum are covered.
pub fn check_match_exhaustiveness(
    arms: &[MatchArm],
    enum_defs: &[(String, Vec<String>)],
) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();

    // Check if any arm is a catch-all
    for arm in arms {
        if is_catch_all_pattern(&arm.pattern) && arm.guard.is_none() {
            return diagnostics;
        }
    }

    // Collect enum names referenced in arms
    let mut enum_variants_used: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    let mut has_non_enum_arm = false;
    for arm in arms {
        // Only count unguarded arms as covering a variant — guarded arms may not match
        if arm.guard.is_none() {
            let before = enum_variants_used.values().map(|v| v.len()).sum::<usize>();
            collect_enum_variants(&arm.pattern, &mut enum_variants_used);
            let after = enum_variants_used.values().map(|v| v.len()).sum::<usize>();
            if before == after && !is_catch_all_pattern(&arm.pattern) {
                // This arm is not an enum pattern and not a catch-all — mixed match
                has_non_enum_arm = true;
            }
        }
    }

    // If arms mix enum patterns with non-enum patterns (literals, structs, etc.),
    // we can't reliably check exhaustiveness — skip to avoid false positives.
    if has_non_enum_arm {
        return diagnostics;
    }

    // For each enum referenced, check if all its variants are covered
    for (enum_name, used_variants) in &enum_variants_used {
        if let Some((_, all_variants)) = enum_defs.iter().find(|(name, _)| name == enum_name) {
            let missing: Vec<&String> = all_variants
                .iter()
                .filter(|v| !used_variants.contains(v))
                .collect();

            if !missing.is_empty() {
                let code = ErrorCode::W203;
                let missing_names: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
                // Use the span of the first arm as a reasonable location
                let span = arms.first().map_or(Span::default(), |a| a.span);
                diagnostics.push(AstDiagnostic {
                    line: span.start_line,
                    column: span.start_col,
                    message: format!(
                        "non-exhaustive match: missing variant(s) {}::{{{}}}",
                        enum_name,
                        missing_names.join(", ")
                    ),
                    severity: DiagnosticSeverity::Warning,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: None,
                });
            }
        }
    }

    diagnostics
}

// =============================================================================
// W209: Shadowed variable in same scope
// =============================================================================

/// Check for variables declared more than once in the same block scope.
/// Only checks within a flat block (not nested scopes). Skips _-prefixed names.
pub fn check_same_scope_shadowing(stmts: &[Statement]) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut seen: std::collections::HashMap<String, Span> = std::collections::HashMap::new();

    for stmt in stmts {
        let names_and_spans: Vec<(String, Span)> = match &stmt.kind {
            StatementKind::Let { name, .. } | StatementKind::LetMut { name, .. } => {
                vec![(name.clone(), stmt.span)]
            }
            StatementKind::ConstDef { name, .. } => {
                vec![(name.clone(), stmt.span)]
            }
            StatementKind::LetDestructure { pattern, .. } => match pattern {
                DestructurePattern::Array(elements) => elements
                    .iter()
                    .map(|elem| {
                        let name = match elem {
                            DestructureElement::Name(n) => n,
                            DestructureElement::Rest(n) => n,
                        };
                        (name.clone(), stmt.span)
                    })
                    .collect(),
                DestructurePattern::Map(entries) => entries
                    .iter()
                    .map(|(key, alias)| {
                        let name = alias.as_deref().unwrap_or(key.as_str());
                        (name.to_string(), stmt.span)
                    })
                    .collect(),
            },
            _ => vec![],
        };

        for (name, span) in names_and_spans {
            if name.starts_with('_') {
                continue;
            }
            if let Some(prev_span) = seen.get(&name) {
                let code = ErrorCode::W209;
                diagnostics.push(AstDiagnostic {
                    line: span.start_line,
                    column: span.start_col,
                    message: format!(
                        "'{}' shadows a previous binding in the same scope (line {})",
                        name, prev_span.start_line
                    ),
                    severity: DiagnosticSeverity::Warning,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: Some(format!(
                        "Use a different name or remove the earlier `let {}`",
                        name
                    )),
                });
            } else {
                seen.insert(name, span);
            }
        }
    }

    diagnostics
}

// =============================================================================
// W212: Return/break/continue/throw in finally block
// =============================================================================

/// Check if a finally block contains return/break/continue/throw statements,
/// which override the try/catch result and are almost always bugs.
pub fn check_return_in_finally(finally_block: &Block, span: Span) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();
    find_control_flow_in_block(finally_block, &mut diagnostics);
    let _ = span;
    diagnostics
}

/// Recursively find return/break/continue/throw in a block (for W212).
fn find_control_flow_in_block(block: &Block, diagnostics: &mut Vec<AstDiagnostic>) {
    for stmt in &block.statements {
        find_control_flow_in_stmt(stmt, diagnostics);
    }
    if let Some(tail) = &block.tail_expr {
        find_control_flow_in_expr(tail, diagnostics);
    }
}

/// Check a statement for control flow that would override try/catch in a finally block.
fn find_control_flow_in_stmt(stmt: &Statement, diagnostics: &mut Vec<AstDiagnostic>) {
    match &stmt.kind {
        StatementKind::Return(_) => {
            emit_w212(stmt.span, "return", diagnostics);
        }
        StatementKind::Break(_) => {
            emit_w212(stmt.span, "break", diagnostics);
        }
        StatementKind::Continue => {
            emit_w212(stmt.span, "continue", diagnostics);
        }
        StatementKind::Throw(_) => {
            emit_w212(stmt.span, "throw", diagnostics);
        }
        // Inside a loop body within finally, break/continue target the loop itself.
        // Only return/throw still override the finally block.
        StatementKind::ForLoop { body, .. } | StatementKind::WhileLoop { body, .. } => {
            find_return_throw_in_block(body, diagnostics);
        }
        StatementKind::TryCatch {
            try_block,
            catch_block,
            finally_block,
            ..
        } => {
            find_control_flow_in_block(try_block, diagnostics);
            find_control_flow_in_block(catch_block, diagnostics);
            if let Some(fb) = finally_block {
                find_control_flow_in_block(fb, diagnostics);
            }
        }
        StatementKind::ExprStatement(expr) => {
            find_control_flow_in_expr(expr, diagnostics);
        }
        StatementKind::FunctionDef(_) | StatementKind::AsyncFunctionDef(_) => {}
        _ => {}
    }
}

fn emit_w212(span: Span, keyword: &str, diagnostics: &mut Vec<AstDiagnostic>) {
    let code = ErrorCode::W212;
    diagnostics.push(AstDiagnostic {
        line: span.start_line,
        column: span.start_col,
        message: format!(
            "`{}` in `finally` block overrides try/catch result",
            keyword
        ),
        severity: DiagnosticSeverity::Warning,
        code: Some(code.to_string()),
        help: Some(code.help().to_string()),
        suggestion: None,
    });
}

/// Inside a loop body within a finally block, only return/throw are problematic.
/// break/continue target the loop itself, not the finally block.
fn find_return_throw_in_block(block: &Block, diagnostics: &mut Vec<AstDiagnostic>) {
    for stmt in &block.statements {
        match &stmt.kind {
            StatementKind::Return(_) => {
                emit_w212(stmt.span, "return", diagnostics);
            }
            StatementKind::Throw(_) => {
                emit_w212(stmt.span, "throw", diagnostics);
            }
            StatementKind::ForLoop { body, .. } | StatementKind::WhileLoop { body, .. } => {
                find_return_throw_in_block(body, diagnostics);
            }
            StatementKind::ExprStatement(expr) => {
                find_return_throw_in_expr(expr, diagnostics);
            }
            StatementKind::FunctionDef(_) | StatementKind::AsyncFunctionDef(_) => {}
            _ => {}
        }
    }
}

/// Inside a loop body, only look for return/throw (not break/continue).
fn find_return_throw_in_expr(expr: &Expression, diagnostics: &mut Vec<AstDiagnostic>) {
    match &expr.kind {
        ExpressionKind::IfElse {
            then_block,
            else_block,
            ..
        } => {
            find_return_throw_in_block(then_block, diagnostics);
            if let Some(eb) = else_block {
                find_return_throw_in_block(eb, diagnostics);
            }
        }
        ExpressionKind::Block(block) => {
            find_return_throw_in_block(block, diagnostics);
        }
        ExpressionKind::Match { arms, .. } => {
            for arm in arms {
                find_return_throw_in_block(&arm.body, diagnostics);
            }
        }
        ExpressionKind::Loop(block) => {
            find_return_throw_in_block(block, diagnostics);
        }
        _ => {}
    }
}

/// Check an expression for control flow statements (for W212 finally block check).
fn find_control_flow_in_expr(expr: &Expression, diagnostics: &mut Vec<AstDiagnostic>) {
    match &expr.kind {
        ExpressionKind::IfElse {
            then_block,
            else_block,
            ..
        } => {
            find_control_flow_in_block(then_block, diagnostics);
            if let Some(eb) = else_block {
                find_control_flow_in_block(eb, diagnostics);
            }
        }
        ExpressionKind::Block(block) => {
            find_control_flow_in_block(block, diagnostics);
        }
        ExpressionKind::Match { arms, .. } => {
            for arm in arms {
                find_control_flow_in_block(&arm.body, diagnostics);
            }
        }
        ExpressionKind::Loop(block) => {
            find_return_throw_in_block(block, diagnostics);
        }
        ExpressionKind::TryCatchExpr {
            try_block,
            catch_block,
            finally_block,
            ..
        } => {
            find_control_flow_in_block(try_block, diagnostics);
            find_control_flow_in_block(catch_block, diagnostics);
            if let Some(fb) = finally_block {
                find_control_flow_in_block(fb, diagnostics);
            }
        }
        _ => {}
    }
}
