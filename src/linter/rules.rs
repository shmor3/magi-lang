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
    if name.starts_with('_') || name.chars().count() <= 1 {
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
            // A try/catch where both blocks terminate, or finally terminates, is a terminator.
            StatementKind::TryCatch { try_block, catch_block, finally_block, .. } => {
                if (is_terminating_block(try_block) && is_terminating_block(catch_block))
                    || finally_block.as_ref().is_some_and(is_terminating_block)
                {
                    terminated = true;
                }
            }
            // `while true { return/throw; }` is a terminator (condition is always true).
            // But if the body contains a `break`, code after the loop IS reachable.
            StatementKind::WhileLoop { condition, body, .. } => {
                if matches!(&condition.kind, ExpressionKind::Literal(Literal::Bool(true)))
                    && is_terminating_block(body)
                    && !block_contains_break(body)
                {
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
        // A loop always either runs forever or exits via break/return/throw.
        // Code after it is unreachable only if the body terminates without break
        // (break would exit the loop, making subsequent code reachable).
        ExpressionKind::Loop(block) => is_terminating_block(block) && !block_contains_break(block),
        // A try/catch expression where both blocks terminate, or finally terminates, is a terminator.
        ExpressionKind::TryCatchExpr { try_block, catch_block, finally_block, .. } => {
            (is_terminating_block(try_block) && is_terminating_block(catch_block))
                || finally_block.as_ref().is_some_and(is_terminating_block)
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
            StatementKind::TryCatch { try_block, catch_block, finally_block, .. } => {
                if (is_terminating_block(try_block) && is_terminating_block(catch_block))
                    || finally_block.as_ref().is_some_and(is_terminating_block)
                {
                    return true;
                }
            }
            // `while true { return/throw; }` always terminates, unless it contains break.
            StatementKind::WhileLoop { condition, body, .. } => {
                if matches!(&condition.kind, ExpressionKind::Literal(Literal::Bool(true)))
                    && is_terminating_block(body)
                    && !block_contains_break(body)
                {
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

/// Check if a pattern contains any enum patterns (including inside Or-patterns).
fn contains_enum_pattern(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::EnumPattern { .. } => true,
        Pattern::Or(alternatives) => alternatives.iter().any(contains_enum_pattern),
        _ => false,
    }
}

/// Recursively collect enum variant names from a pattern, including inside Or-patterns.
fn collect_enum_variants(
    pattern: &Pattern,
    map: &mut std::collections::HashMap<String, std::collections::HashSet<String>>,
) {
    match pattern {
        Pattern::EnumPattern { enum_name, variant, .. } => {
            map.entry(enum_name.clone()).or_default().insert(variant.clone());
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
    match_span: Span,
) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();

    // Check if any arm is a catch-all
    for arm in arms {
        if is_catch_all_pattern(&arm.pattern) && arm.guard.is_none() {
            return diagnostics;
        }
    }

    // Collect enum names referenced in arms
    let mut enum_variants_used: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();

    let mut has_non_enum_arm = false;
    for arm in arms {
        // Only count unguarded arms as covering a variant — guarded arms may not match
        if arm.guard.is_none() {
            collect_enum_variants(&arm.pattern, &mut enum_variants_used);
            if !contains_enum_pattern(&arm.pattern) && !is_catch_all_pattern(&arm.pattern) {
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

    // If all arms were guarded (no unguarded enum coverage), check for guarded enum patterns
    if enum_variants_used.is_empty() {
        let mut guarded_enums: std::collections::HashMap<String, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        for arm in arms {
            if arm.guard.is_some() {
                collect_enum_variants(&arm.pattern, &mut guarded_enums);
            }
        }
        for enum_name in guarded_enums.keys() {
            if let Some((_, all_variants)) = enum_defs.iter().find(|(name, _)| name == enum_name) {
                let code = ErrorCode::W203;
                let all_names: Vec<&str> = all_variants.iter().map(|s| s.as_str()).collect();
                diagnostics.push(AstDiagnostic {
                    line: match_span.start_line,
                    column: match_span.start_col,
                    message: format!(
                        "non-exhaustive match: all arms are guarded, no variant of {} is definitively covered",
                        enum_name,
                    ),
                    severity: DiagnosticSeverity::Warning,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: Some(format!("add a wildcard arm `_ => ...` or unguarded variant arms for {}", all_names.join(", "))),
                });
            }
        }
    }

    // For each enum referenced, check if all its variants are covered
    for (enum_name, used_variants) in &enum_variants_used {
        if let Some((_, all_variants)) = enum_defs.iter().find(|(name, _)| name == enum_name) {
            let missing: Vec<&String> = all_variants
                .iter()
                .filter(|v| !used_variants.contains(*v))
                .collect();

            if !missing.is_empty() {
                let code = ErrorCode::W203;
                let missing_names: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
                diagnostics.push(AstDiagnostic {
                    line: match_span.start_line,
                    column: match_span.start_col,
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
// W205: Self-comparison (comparing a value to itself)
// =============================================================================

/// Check if a binary comparison compares an expression to itself.
/// e.g., `x == x`, `y != y`, `a > a`, `a < a`, `a >= a`, `a <= a`
pub fn check_self_comparison(op: &BinOp, left: &Expression, right: &Expression, span: Span) -> Option<AstDiagnostic> {
    // Only check comparison operators
    match op {
        BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {}
        _ => return None,
    }
    if exprs_structurally_equal(left, right) {
        let code = ErrorCode::W205;
        Some(AstDiagnostic {
            line: span.start_line,
            column: span.start_col,
            message: format!("comparing a value to itself (`{}`)", op_symbol(op)),
            severity: DiagnosticSeverity::Warning,
            code: Some(code.to_string()),
            help: Some(code.help().to_string()),
            suggestion: Some("Did you mean to compare to a different value? Use `x.is_nan()` to check for NaN.".to_string()),
        })
    } else {
        None
    }
}

/// Check if two expressions are structurally identical (simple cases only).
fn exprs_structurally_equal(a: &Expression, b: &Expression) -> bool {
    match (&a.kind, &b.kind) {
        (ExpressionKind::Variable(va), ExpressionKind::Variable(vb)) => va == vb,
        (
            ExpressionKind::FieldAccess { object: oa, field: fa },
            ExpressionKind::FieldAccess { object: ob, field: fb },
        ) => fa == fb && exprs_structurally_equal(oa, ob),
        (
            ExpressionKind::Index { object: oa, index: ia },
            ExpressionKind::Index { object: ob, index: ib },
        ) => exprs_structurally_equal(oa, ob) && exprs_structurally_equal(ia, ib),
        (ExpressionKind::Literal(la), ExpressionKind::Literal(lb)) => literals_equal(la, lb),
        _ => false,
    }
}

/// Check if two literals are equal.
fn literals_equal(a: &Literal, b: &Literal) -> bool {
    match (a, b) {
        (Literal::Int64(ia), Literal::Int64(ib)) => ia == ib,
        (Literal::Float64(fa), Literal::Float64(fb)) => fa == fb,
        (Literal::String(sa), Literal::String(sb)) => sa == sb,
        (Literal::Bool(ba), Literal::Bool(bb)) => ba == bb,
        (Literal::Null, Literal::Null) => true,
        _ => false,
    }
}

fn op_symbol(op: &BinOp) -> &'static str {
    match op {
        BinOp::Eq => "==",
        BinOp::NotEq => "!=",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::LtEq => "<=",
        BinOp::GtEq => ">=",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
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
            StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
                vec![(fdef.name.clone(), fdef.span)]
            }
            StatementKind::EnumDef { name, .. }
            | StatementKind::StructDef { name, .. }
            | StatementKind::TypeAlias { name, .. }
            | StatementKind::ModuleDef { name, .. } => {
                vec![(name.clone(), stmt.span)]
            }
            StatementKind::Use { path, alias, glob } => {
                if *glob {
                    vec![]
                } else if let Some(alias_name) = alias {
                    vec![(alias_name.clone(), stmt.span)]
                } else if let Some(last) = path.last() {
                    vec![(last.clone(), stmt.span)]
                } else {
                    vec![]
                }
            }
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
                        "Use a different name or remove the earlier declaration of `{}`",
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
pub fn check_control_flow_in_finally(finally_block: &Block) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();
    find_control_flow_in_block(finally_block, &mut diagnostics);
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
        StatementKind::ExprStatement(expr)
        | StatementKind::Output(expr) => {
            find_control_flow_in_expr(expr, diagnostics);
        }
        StatementKind::Let { value, .. }
        | StatementKind::LetMut { value, .. }
        | StatementKind::LetDestructure { value, .. }
        | StatementKind::ConstDef { value, .. }
        | StatementKind::Assignment { value, .. }
        | StatementKind::CompoundAssign { value, .. } => {
            find_control_flow_in_expr(value, diagnostics);
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
            StatementKind::TryCatch {
                try_block,
                catch_block,
                finally_block,
                ..
            } => {
                find_return_throw_in_block(try_block, diagnostics);
                find_return_throw_in_block(catch_block, diagnostics);
                if let Some(fb) = finally_block {
                    find_return_throw_in_block(fb, diagnostics);
                }
            }
            StatementKind::ExprStatement(expr)
            | StatementKind::Output(expr) => {
                find_return_throw_in_expr(expr, diagnostics);
            }
            StatementKind::Let { value, .. }
            | StatementKind::LetMut { value, .. }
            | StatementKind::LetDestructure { value, .. }
            | StatementKind::ConstDef { value, .. }
            | StatementKind::Assignment { value, .. }
            | StatementKind::CompoundAssign { value, .. } => {
                find_return_throw_in_expr(value, diagnostics);
            }
            StatementKind::FunctionDef(_) | StatementKind::AsyncFunctionDef(_) => {}
            _ => {}
        }
    }
    if let Some(tail) = &block.tail_expr {
        find_return_throw_in_expr(tail, diagnostics);
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
        ExpressionKind::TryCatchExpr {
            try_block,
            catch_block,
            finally_block,
            ..
        } => {
            find_return_throw_in_block(try_block, diagnostics);
            find_return_throw_in_block(catch_block, diagnostics);
            if let Some(fb) = finally_block {
                find_return_throw_in_block(fb, diagnostics);
            }
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

// =============================================================================
// W216: Empty enum definition
// =============================================================================

/// W216: Check for empty enum definitions (enum with zero variants).
pub fn check_empty_enum(variants: &[EnumVariant], name: &str, span: Span) -> Option<AstDiagnostic> {
    if variants.is_empty() {
        let code = ErrorCode::W216;
        return Some(AstDiagnostic {
            line: span.start_line,
            column: span.start_col,
            message: format!("empty enum '{}' has no variants", name),
            severity: DiagnosticSeverity::Warning,
            code: Some(code.to_string()),
            help: Some(code.help().to_string()),
            suggestion: None,
        });
    }
    None
}

// =============================================================================
// W230: Self-assignment
// =============================================================================

/// W230: Check `let x = x` patterns (self-assignment in let binding).
pub fn check_self_assignment_let(name: &str, value: &Expression, span: Span) -> Option<AstDiagnostic> {
    if let ExpressionKind::Variable(v) = &value.kind {
        if v == name {
            let code = ErrorCode::W230;
            return Some(AstDiagnostic {
                line: span.start_line,
                column: span.start_col,
                message: format!("self-assignment: `{} = {}`", name, name),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: None,
            });
        }
    }
    None
}

/// W230: Check `x = x` patterns (self-assignment in assignment statement).
pub fn check_self_assignment(name: &str, value: &Expression, span: Span) -> Option<AstDiagnostic> {
    if let ExpressionKind::Variable(v) = &value.kind {
        if v == name {
            let code = ErrorCode::W230;
            return Some(AstDiagnostic {
                line: span.start_line,
                column: span.start_col,
                message: format!("self-assignment: `{} = {}`", name, name),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: None,
            });
        }
    }
    None
}

// =============================================================================
// W231: Boolean literal in if-else return
// =============================================================================

/// W231: Check for `if cond { true } else { false }` patterns.
pub fn check_boolean_if_else(expr: &Expression) -> Option<AstDiagnostic> {
    if let ExpressionKind::IfElse { then_block, else_block: Some(else_block), .. } = &expr.kind {
        let then_bool = then_block.tail_expr.as_ref().and_then(|e| match &e.kind {
            ExpressionKind::Literal(Literal::Bool(b)) => Some(*b),
            _ => None,
        });
        let else_bool = else_block.tail_expr.as_ref().and_then(|e| match &e.kind {
            ExpressionKind::Literal(Literal::Bool(b)) => Some(*b),
            _ => None,
        });
        if then_block.statements.is_empty() && else_block.statements.is_empty() {
            match (then_bool, else_bool) {
                (Some(true), Some(false)) => {
                    let code = ErrorCode::W231;
                    return Some(AstDiagnostic {
                        line: expr.span.start_line,
                        column: expr.span.start_col,
                        message: "`if cond { true } else { false }` can be simplified to `cond`".to_string(),
                        severity: DiagnosticSeverity::Warning,
                        code: Some(code.to_string()),
                        help: Some(code.help().to_string()),
                        suggestion: Some("Replace with the condition directly".to_string()),
                    });
                }
                (Some(false), Some(true)) => {
                    let code = ErrorCode::W231;
                    return Some(AstDiagnostic {
                        line: expr.span.start_line,
                        column: expr.span.start_col,
                        message: "`if cond { false } else { true }` can be simplified to `!cond`".to_string(),
                        severity: DiagnosticSeverity::Warning,
                        code: Some(code.to_string()),
                        help: Some(code.help().to_string()),
                        suggestion: Some("Replace with `!cond`".to_string()),
                    });
                }
                _ => {}
            }
        }
    }
    None
}

// =============================================================================
// W215: Negated if condition with else branch
// =============================================================================

/// W215: Check for `if !cond { a } else { b }` which should be `if cond { b } else { a }`.
pub fn check_negated_if_else(condition: &Expression, else_block: Option<&Block>, span: Span) -> Option<AstDiagnostic> {
    if else_block.is_none() {
        return None; // Only applies when both branches exist
    }
    if let ExpressionKind::UnaryOp { op: UnOp::Not, .. } = &condition.kind {
        let code = ErrorCode::W215;
        return Some(AstDiagnostic {
            line: span.start_line,
            column: span.start_col,
            message: "negated condition in `if !cond { ... } else { ... }`".to_string(),
            severity: DiagnosticSeverity::Warning,
            code: Some(code.to_string()),
            help: Some(code.help().to_string()),
            suggestion: Some("Swap the branches and remove the `!`".to_string()),
        });
    }
    None
}

// =============================================================================
// W233: Deeply nested code
// =============================================================================

/// W233: Check for deeply nested code blocks.
pub fn check_deep_nesting(stmts: &[Statement], max_depth: usize) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();
    for stmt in stmts {
        check_nesting_depth_stmt(stmt, 0, max_depth, &mut diagnostics);
    }
    diagnostics
}

fn check_nesting_depth_stmt(stmt: &Statement, depth: usize, max_depth: usize, diagnostics: &mut Vec<AstDiagnostic>) {
    match &stmt.kind {
        StatementKind::ForLoop { body, .. }
        | StatementKind::WhileLoop { body, .. } => {
            let new_depth = depth + 1;
            if new_depth > max_depth {
                let code = ErrorCode::W233;
                diagnostics.push(AstDiagnostic {
                    line: stmt.span.start_line,
                    column: stmt.span.start_col,
                    message: format!("code nested {} levels deep", new_depth),
                    severity: DiagnosticSeverity::Warning,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: None,
                });
            }
            for s in &body.statements {
                check_nesting_depth_stmt(s, new_depth, max_depth, diagnostics);
            }
        }
        StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
            for s in &fdef.body.statements {
                check_nesting_depth_stmt(s, depth, max_depth, diagnostics);
            }
        }
        StatementKind::TryCatch { try_block, catch_block, finally_block, .. } => {
            for s in &try_block.statements {
                check_nesting_depth_stmt(s, depth + 1, max_depth, diagnostics);
            }
            for s in &catch_block.statements {
                check_nesting_depth_stmt(s, depth + 1, max_depth, diagnostics);
            }
            if let Some(fb) = finally_block {
                for s in &fb.statements {
                    check_nesting_depth_stmt(s, depth + 1, max_depth, diagnostics);
                }
            }
        }
        StatementKind::ExprStatement(expr) => {
            check_nesting_depth_expr(expr, depth, max_depth, diagnostics);
        }
        _ => {}
    }
}

// =============================================================================
// W229: Empty match arm body
// =============================================================================

/// W229: Check for match arms that have empty bodies.
pub fn check_empty_match_arms(arms: &[MatchArm]) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();
    for arm in arms {
        if arm.body.statements.is_empty() && arm.body.tail_expr.is_none() {
            let code = ErrorCode::W229;
            diagnostics.push(AstDiagnostic {
                line: arm.span.start_line,
                column: arm.span.start_col,
                message: "empty match arm body".to_string(),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: Some("Add an expression or use `null` explicitly".to_string()),
            });
        }
    }
    diagnostics
}

fn check_nesting_depth_expr(expr: &Expression, depth: usize, max_depth: usize, diagnostics: &mut Vec<AstDiagnostic>) {
    if let ExpressionKind::IfElse { then_block, else_block, .. } = &expr.kind {
        let new_depth = depth + 1;
        if new_depth > max_depth {
            let code = ErrorCode::W233;
            diagnostics.push(AstDiagnostic {
                line: expr.span.start_line,
                column: expr.span.start_col,
                message: format!("code nested {} levels deep", new_depth),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: None,
            });
        }
        for s in &then_block.statements {
            check_nesting_depth_stmt(s, new_depth, max_depth, diagnostics);
        }
        if let Some(eb) = else_block {
            for s in &eb.statements {
                check_nesting_depth_stmt(s, new_depth, max_depth, diagnostics);
            }
        }
    }
}

/// W236: Check for TODO/FIXME comments in source code (#151).
pub fn check_todo_comments(source: &str) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();
    for (line_idx, line) in source.lines().enumerate() {
        if let Some(comment_start) = line.find("//") {
            let comment = &line[comment_start..];
            let upper = comment.to_uppercase();
            if upper.contains("TODO") || upper.contains("FIXME") || upper.contains("HACK") || upper.contains("XXX") {
                let code = ErrorCode::W236;
                diagnostics.push(AstDiagnostic {
                    line: (line_idx + 1) as u32,
                    column: (comment_start + 1) as u32,
                    message: format!("found comment marker in: {}", comment.trim()),
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

/// W237: Check for magic numbers in expressions (#152).
/// Only flags integer literals > 1 and < -1 that appear directly in comparisons
/// or arithmetic (not in let/const bindings or array/map literals).
pub fn check_magic_number(expr: &Expression, in_binding: bool) -> Option<AstDiagnostic> {
    if in_binding { return None; }
    match &expr.kind {
        ExpressionKind::BinaryOp { op, left, right, .. } => {
            // Check operands of comparisons and arithmetic for bare integer literals
            let check_side = |side: &Expression| -> Option<AstDiagnostic> {
                if let ExpressionKind::Literal(Literal::Int64(n)) = &side.kind {
                    let n = *n;
                    if n > 1 || n < -1 {
                        let code = ErrorCode::W237;
                        return Some(AstDiagnostic {
                            line: side.span.start_line,
                            column: side.span.start_col,
                            message: format!("magic number: {}", n),
                            severity: DiagnosticSeverity::Warning,
                            code: Some(code.to_string()),
                            help: Some(code.help().to_string()),
                            suggestion: Some(format!("Extract {} into a named constant", n)),
                        });
                    }
                }
                None
            };
            use crate::syntax::ast::BinOp::*;
            match op {
                Gt | Lt | GtEq | LtEq | Eq | NotEq => {
                    check_side(left).or_else(|| check_side(right))
                }
                _ => None,
            }
        }
        _ => None,
    }
}
