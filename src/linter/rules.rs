//! Individual lint rule implementations.

use crate::syntax::ast::*;
use crate::syntax::errors::ErrorCode;
use crate::syntax::type_checker::AstDiagnostic;
use crate::eval::DiagnosticSeverity;

/// Convert a name to snake_case, handling acronyms correctly.
/// e.g. "HTTPServer" → "http_server", "myFunc" → "my_func"
fn to_snake_case(name: &str) -> String {
    crate::util::to_snake_case(name)
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
        source_file: None,
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
        source_file: None,
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
                source_file: None,
            });
            continue; // Report all unreachable statements, not just the first
        }

        match &stmt.kind {
            StatementKind::Return(_)
            | StatementKind::Break { value: _, .. }
            | StatementKind::Continue { .. }
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
            source_file: None,
        });
    }
    None
}

/// Returns true if a block (or any of its nested non-loop branches) contains a `break`.
/// Does NOT recurse into nested loops since their breaks target the inner loop.
fn block_contains_break(block: &Block) -> bool {
    for stmt in &block.statements {
        match &stmt.kind {
            StatementKind::Break { value: _, .. } => return true,
            StatementKind::TryCatch { try_block, catch_block, .. } => {
                if block_contains_break(try_block) || block_contains_break(catch_block) {
                    return true;
                }
            }
            // Don't recurse into nested loops — their breaks target the inner loop.
            StatementKind::ForLoop { .. } | StatementKind::WhileLoop { .. } | StatementKind::DoWhileLoop { .. } | StatementKind::CStyleFor { .. } => {}
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
        ExpressionKind::Loop { body: _, .. } => false,
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
            source_file: None,
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
                source_file: None,
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
            StatementKind::ImportModule { path, .. } => Some(path.join(".")),
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
                    source_file: None,
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
        ExpressionKind::Loop { body: block, .. } => is_terminating_block(block) && !block_contains_break(block),
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
            | StatementKind::Break { value: _, .. }
            | StatementKind::Continue { .. }
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
                    source_file: None,
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
                    source_file: None,
                });
            }
        }
    }

    diagnostics
}

// W205: Self-comparison (comparing a value to itself)

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
            source_file: None,
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
        BinOp::In => "in",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::AndNot => "&^",
        BinOp::Pow => "**",
    }
}

// W209: Shadowed variable in same scope

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
                DestructurePattern::Array(elements) | DestructurePattern::Tuple(elements) => elements
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
            StatementKind::Use { path, alias, glob, .. } => {
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
                    source_file: None,
                });
            } else {
                seen.insert(name, span);
            }
        }
    }

    diagnostics
}

// W212: Return/break/continue/throw in finally block

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
        StatementKind::Break { value: _, .. } => {
            emit_w212(stmt.span, "break", diagnostics);
        }
        StatementKind::Continue { .. } => {
            emit_w212(stmt.span, "continue", diagnostics);
        }
        StatementKind::Throw(_) => {
            emit_w212(stmt.span, "throw", diagnostics);
        }
        // Inside a loop body within finally, break/continue target the loop itself.
        // Only return/throw still override the finally block.
        StatementKind::ForLoop { body, .. } | StatementKind::WhileLoop { body, .. } | StatementKind::DoWhileLoop { body, .. } | StatementKind::CStyleFor { body, .. } => {
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
        source_file: None,
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
            StatementKind::ForLoop { body, .. } | StatementKind::WhileLoop { body, .. } | StatementKind::DoWhileLoop { body, .. } | StatementKind::CStyleFor { body, .. } => {
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
        ExpressionKind::Loop { body: block, .. } => {
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
        ExpressionKind::Loop { body: block, .. } => {
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

// W216: Empty enum definition

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
            source_file: None,
        });
    }
    None
}

// W230: Self-assignment

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
                source_file: None,
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
                source_file: None,
            });
        }
    }
    None
}

// W231: Boolean literal in if-else return

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
                        source_file: None,
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
                        source_file: None,
                    });
                }
                _ => {}
            }
        }
    }
    None
}

// W215: Negated if condition with else branch

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
            source_file: None,
        });
    }
    None
}

// W233: Deeply nested code

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
        | StatementKind::WhileLoop { body, .. }
        | StatementKind::DoWhileLoop { body, .. } | StatementKind::CStyleFor { body, .. } => {
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
                    source_file: None,
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

// W229: Empty match arm body

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
                source_file: None,
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
                source_file: None,
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
                    source_file: None,
                });
            }
        }
    }
    diagnostics
}

// W238: Unused variables

/// Collect all names referenced in an expression (variable reads).
fn collect_referenced_names(expr: &Expression, names: &mut std::collections::HashSet<String>) {
    match &expr.kind {
        ExpressionKind::Variable(name) => { names.insert(name.clone()); }
        ExpressionKind::BinaryOp { left, right, .. } => {
            collect_referenced_names(left, names);
            collect_referenced_names(right, names);
        }
        ExpressionKind::UnaryOp { operand, .. } => {
            collect_referenced_names(operand, names);
        }
        ExpressionKind::Call { name, args, kwargs, .. } => {
            names.insert(name.clone());
            for a in args { collect_referenced_names(a, names); }
            for (_, a) in kwargs { collect_referenced_names(a, names); }
        }
        ExpressionKind::MethodCall { object, args, kwargs, .. } => {
            collect_referenced_names(object, names);
            for a in args { collect_referenced_names(a, names); }
            for (_, a) in kwargs { collect_referenced_names(a, names); }
        }
        ExpressionKind::FieldAccess { object, .. } => {
            collect_referenced_names(object, names);
        }
        ExpressionKind::Index { object, index } => {
            collect_referenced_names(object, names);
            collect_referenced_names(index, names);
        }
        ExpressionKind::IfElse { condition, then_block, else_block } => {
            collect_referenced_names(condition, names);
            collect_referenced_names_block(then_block, names);
            if let Some(eb) = else_block { collect_referenced_names_block(eb, names); }
        }
        ExpressionKind::Block(block) => {
            collect_referenced_names_block(block, names);
        }
        ExpressionKind::Match { value, arms } => {
            collect_referenced_names(value, names);
            for arm in arms {
                collect_referenced_names_block(&arm.body, names);
                if let Some(g) = &arm.guard { collect_referenced_names(g, names); }
            }
        }
        ExpressionKind::Lambda { body, .. } => {
            collect_referenced_names(body, names);
        }
        ExpressionKind::Pipe { left, right } => {
            collect_referenced_names(left, names);
            collect_referenced_names(right, names);
        }
        ExpressionKind::Range { start, end, .. } => {
            collect_referenced_names(start, names);
            collect_referenced_names(end, names);
        }
        ExpressionKind::NullCoalesce { left, right } => {
            collect_referenced_names(left, names);
            collect_referenced_names(right, names);
        }
        ExpressionKind::OptionalChain { object, .. } => {
            collect_referenced_names(object, names);
        }
        ExpressionKind::Spread(inner) | ExpressionKind::Await(inner) | ExpressionKind::Spawn(inner) | ExpressionKind::TryPropagate(inner) => {
            collect_referenced_names(inner, names);
        }
        ExpressionKind::StringInterpolation { parts } => {
            for part in parts {
                if let StringPart::Expr(e) = part { collect_referenced_names(e, names); }
            }
        }
        ExpressionKind::Literal(Literal::Array(elems)) => {
            for e in elems { collect_referenced_names(e, names); }
        }
        ExpressionKind::Literal(Literal::Map(entries)) => {
            for (_, v) in entries { collect_referenced_names(v, names); }
        }
        ExpressionKind::ListComprehension { expr: inner, iterable, condition, .. } => {
            collect_referenced_names(inner, names);
            collect_referenced_names(iterable, names);
            if let Some(c) = condition { collect_referenced_names(c, names); }
        }
        ExpressionKind::MapComprehension { key_expr, value_expr, iterable, condition, .. } => {
            collect_referenced_names(key_expr, names);
            collect_referenced_names(value_expr, names);
            collect_referenced_names(iterable, names);
            if let Some(c) = condition { collect_referenced_names(c, names); }
        }
        ExpressionKind::EnumConstruct { args, .. } => {
            for a in args { collect_referenced_names(a, names); }
        }
        ExpressionKind::StructConstruct { fields, .. } => {
            for (_, v) in fields { collect_referenced_names(v, names); }
        }
        ExpressionKind::Loop { body, .. } => {
            collect_referenced_names_block(body, names);
        }
        ExpressionKind::TryCatchExpr { try_block, catch_block, finally_block, .. } => {
            collect_referenced_names_block(try_block, names);
            collect_referenced_names_block(catch_block, names);
            if let Some(fb) = finally_block { collect_referenced_names_block(fb, names); }
        }
        ExpressionKind::TupleLiteral(exprs) => {
            for e in exprs { collect_referenced_names(e, names); }
        }
        _ => {}
    }
}

/// Collect referenced names from a block.
fn collect_referenced_names_block(block: &Block, names: &mut std::collections::HashSet<String>) {
    for stmt in &block.statements {
        collect_referenced_names_stmt(stmt, names);
    }
    if let Some(tail) = &block.tail_expr {
        collect_referenced_names(tail, names);
    }
}

/// Collect referenced names from a statement.
fn collect_referenced_names_stmt(stmt: &Statement, names: &mut std::collections::HashSet<String>) {
    match &stmt.kind {
        StatementKind::Let { value, .. }
        | StatementKind::LetMut { value, .. }
        | StatementKind::LetDestructure { value, .. }
        | StatementKind::ConstDef { value, .. } => {
            collect_referenced_names(value, names);
        }
        StatementKind::Assignment { name, value } => {
            names.insert(name.clone());
            collect_referenced_names(value, names);
        }
        StatementKind::CompoundAssign { name, value, .. } => {
            names.insert(name.clone());
            collect_referenced_names(value, names);
        }
        StatementKind::FieldAssignment { object, value, .. } => {
            collect_referenced_names(object, names);
            collect_referenced_names(value, names);
        }
        StatementKind::IndexAssignment { object, index, value } => {
            collect_referenced_names(object, names);
            collect_referenced_names(index, names);
            collect_referenced_names(value, names);
        }
        StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
            collect_referenced_names_block(&fdef.body, names);
        }
        StatementKind::ForLoop { iterable, body, .. } => {
            collect_referenced_names(iterable, names);
            collect_referenced_names_block(body, names);
        }
        StatementKind::WhileLoop { condition, body, .. } => {
            collect_referenced_names(condition, names);
            collect_referenced_names_block(body, names);
        }
        StatementKind::DoWhileLoop { condition, body, .. } | StatementKind::CStyleFor { condition, body, .. } => {
            collect_referenced_names(condition, names);
            collect_referenced_names_block(body, names);
        }
        StatementKind::Output(expr)
        | StatementKind::ExprStatement(expr)
        | StatementKind::Defer(expr)
        | StatementKind::Throw(expr) => {
            collect_referenced_names(expr, names);
        }
        StatementKind::Return(Some(expr)) | StatementKind::Break { value: Some(expr), .. } => {
            collect_referenced_names(expr, names);
        }
        StatementKind::TryCatch { try_block, catch_block, finally_block, .. } => {
            collect_referenced_names_block(try_block, names);
            collect_referenced_names_block(catch_block, names);
            if let Some(fb) = finally_block { collect_referenced_names_block(fb, names); }
        }
        StatementKind::ModuleDef { body, .. } => {
            collect_referenced_names_block(body, names);
        }
        StatementKind::TestDef { body, .. } => {
            collect_referenced_names_block(body, names);
        }
        _ => {}
    }
}

/// W238: Check for unused variables in a block (let bindings never referenced later).
pub fn check_unused_variables(stmts: &[Statement]) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();

    // Collect all names referenced in subsequent statements for each let binding
    for (i, stmt) in stmts.iter().enumerate() {
        let bound_names: Vec<(String, Span)> = match &stmt.kind {
            StatementKind::Let { name, .. } | StatementKind::LetMut { name, .. } => {
                vec![(name.clone(), stmt.span)]
            }
            _ => continue,
        };

        for (name, span) in bound_names {
            // Skip _ prefixed names (conventional suppression)
            if name.starts_with('_') { continue; }

            // Collect all names referenced in the rest of the block
            let mut referenced = std::collections::HashSet::new();
            for later_stmt in &stmts[i + 1..] {
                collect_referenced_names_stmt(later_stmt, &mut referenced);
            }

            if !referenced.contains(&name) {
                let code = ErrorCode::W238;
                diagnostics.push(AstDiagnostic {
                    line: span.start_line,
                    column: span.start_col,
                    message: format!("unused variable: `{}`", name),
                    severity: DiagnosticSeverity::Warning,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: Some(format!("Prefix with underscore `_{}` to suppress, or remove", name)),
                    source_file: None,
                });
            }
        }
    }

    diagnostics
}

// W239: Unnecessary mut

/// W239: Check for `let mut x` where `x` is never reassigned in subsequent statements.
pub fn check_unnecessary_mut(stmts: &[Statement]) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();

    for (i, stmt) in stmts.iter().enumerate() {
        let (name, span) = match &stmt.kind {
            StatementKind::LetMut { name, .. } => (name.clone(), stmt.span),
            _ => continue,
        };

        // Skip _ prefixed names
        if name.starts_with('_') { continue; }

        // Check if any subsequent statement reassigns this variable
        let mut is_reassigned = false;
        for later_stmt in &stmts[i + 1..] {
            if stmt_reassigns_name(later_stmt, &name) {
                is_reassigned = true;
                break;
            }
        }

        if !is_reassigned {
            let code = ErrorCode::W239;
            diagnostics.push(AstDiagnostic {
                line: span.start_line,
                column: span.start_col,
                message: format!("variable `{}` is declared as `mut` but never reassigned", name),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: Some(format!("Use `let {}` instead of `let mut {}`", name, name)),
                source_file: None,
            });
        }
    }

    diagnostics
}

/// Check if a statement reassigns a given variable name.
fn stmt_reassigns_name(stmt: &Statement, name: &str) -> bool {
    match &stmt.kind {
        StatementKind::Assignment { name: n, .. } if n == name => true,
        StatementKind::CompoundAssign { name: n, .. } if n == name => true,
        // Check inside nested blocks (e.g., x could be reassigned inside a loop)
        StatementKind::ForLoop { body, .. }
        | StatementKind::WhileLoop { body, .. }
        | StatementKind::DoWhileLoop { body, .. }
        | StatementKind::CStyleFor { body, .. } => {
            body.statements.iter().any(|s| stmt_reassigns_name(s, name))
        }
        StatementKind::TryCatch { try_block, catch_block, finally_block, .. } => {
            try_block.statements.iter().any(|s| stmt_reassigns_name(s, name))
                || catch_block.statements.iter().any(|s| stmt_reassigns_name(s, name))
                || finally_block.as_ref().is_some_and(|fb| fb.statements.iter().any(|s| stmt_reassigns_name(s, name)))
        }
        StatementKind::ExprStatement(expr) => expr_reassigns_name(expr, name),
        _ => false,
    }
}

/// Check if an expression contains a reassignment of the given name.
fn expr_reassigns_name(expr: &Expression, name: &str) -> bool {
    match &expr.kind {
        ExpressionKind::IfElse { then_block, else_block, .. } => {
            then_block.statements.iter().any(|s| stmt_reassigns_name(s, name))
                || else_block.as_ref().is_some_and(|eb| eb.statements.iter().any(|s| stmt_reassigns_name(s, name)))
        }
        ExpressionKind::Block(block) => {
            block.statements.iter().any(|s| stmt_reassigns_name(s, name))
        }
        ExpressionKind::Match { arms, .. } => {
            arms.iter().any(|arm| arm.body.statements.iter().any(|s| stmt_reassigns_name(s, name)))
        }
        ExpressionKind::Loop { body, .. } => {
            body.statements.iter().any(|s| stmt_reassigns_name(s, name))
        }
        _ => false,
    }
}

// W240: Needless return

/// W240: Check for `return expr` as the last statement in a block where the
/// expression alone would suffice as an implicit return.
pub fn check_needless_return(stmts: &[Statement], span: Span) -> Option<AstDiagnostic> {
    if let Some(last) = stmts.last() {
        if let StatementKind::Return(Some(_)) = &last.kind {
            let code = ErrorCode::W240;
            return Some(AstDiagnostic {
                line: last.span.start_line,
                column: last.span.start_col,
                message: "needless `return` as last statement".to_string(),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: Some("Remove the `return` keyword; the expression alone is the implicit return value".to_string()),
                source_file: None,
            });
        }
    }
    // Suppress unused parameter warning
    let _ = span;
    None
}

// W241: Comparison to bool literal

/// W241: Check for `x == true`, `x == false`, `true == x`, `false == x`.
pub fn check_bool_comparison(op: &BinOp, left: &Expression, right: &Expression, span: Span) -> Option<AstDiagnostic> {
    // Only check == and !=
    match op {
        BinOp::Eq | BinOp::NotEq => {}
        _ => return None,
    }

    let left_is_bool = matches!(&left.kind, ExpressionKind::Literal(Literal::Bool(_)));
    let right_is_bool = matches!(&right.kind, ExpressionKind::Literal(Literal::Bool(_)));

    if left_is_bool || right_is_bool {
        let code = ErrorCode::W241;
        let suggestion = if *op == BinOp::Eq {
            if right_is_bool {
                match &right.kind {
                    ExpressionKind::Literal(Literal::Bool(true)) => "Use the value directly instead of `x == true`".to_string(),
                    ExpressionKind::Literal(Literal::Bool(false)) => "Use `!x` instead of `x == false`".to_string(),
                    _ => unreachable!(),
                }
            } else {
                match &left.kind {
                    ExpressionKind::Literal(Literal::Bool(true)) => "Use the value directly instead of `true == x`".to_string(),
                    ExpressionKind::Literal(Literal::Bool(false)) => "Use `!x` instead of `false == x`".to_string(),
                    _ => unreachable!(),
                }
            }
        } else {
            "Use `!x` or the value directly instead of comparing to a boolean literal".to_string()
        };

        return Some(AstDiagnostic {
            line: span.start_line,
            column: span.start_col,
            message: "comparison to boolean literal".to_string(),
            severity: DiagnosticSeverity::Warning,
            code: Some(code.to_string()),
            help: Some(code.help().to_string()),
            suggestion: Some(suggestion),
            source_file: None,
        });
    }

    None
}

// W242: Collapsible if

/// W242: Check for `if a { if b { ... } }` that could be `if a && b { ... }`.
/// Only flags when:
/// - The outer `if` has no `else` branch
/// - The outer then-block has exactly one statement, which is an expression-statement
///   containing another `if` with no `else` branch
pub fn check_collapsible_if(
    then_block: &Block,
    else_block: &Option<Block>,
    span: Span,
) -> Option<AstDiagnostic> {
    // Only applies when outer if has no else
    if else_block.is_some() { return None; }
    // Only applies when then_block has no tail expr
    if then_block.tail_expr.is_some() { return None; }

    if then_block.statements.len() != 1 { return None; }

    let inner_stmt = &then_block.statements[0];
    if let StatementKind::ExprStatement(inner_expr) = &inner_stmt.kind {
        if let ExpressionKind::IfElse { else_block: inner_else, .. } = &inner_expr.kind {
            if inner_else.is_none() {
                let code = ErrorCode::W242;
                return Some(AstDiagnostic {
                    line: span.start_line,
                    column: span.start_col,
                    message: "this `if` can be collapsed into the outer `if` with `&&`".to_string(),
                    severity: DiagnosticSeverity::Warning,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: Some("Combine into `if outer_cond && inner_cond { ... }`".to_string()),
                    source_file: None,
                });
            }
        }
    }

    None
}

/// W243: Check for too many function parameters (> 7).
pub fn check_too_many_params(fdef: &FunctionDef) -> Option<AstDiagnostic> {
    const MAX_PARAMS: usize = 7;
    if fdef.params.len() > MAX_PARAMS {
        let code = ErrorCode::W243;
        return Some(AstDiagnostic {
            line: fdef.span.start_line,
            column: fdef.span.start_col,
            message: format!("function '{}' has {} parameters (max {})", fdef.name, fdef.params.len(), MAX_PARAMS),
            severity: DiagnosticSeverity::Warning,
            code: Some(code.to_string()),
            help: Some(code.help().to_string()),
            suggestion: Some("Consider using a struct to group parameters".to_string()),
            source_file: None,
        });
    }
    None
}

/// W244: Check for TODO/FIXME/HACK/XXX comments in code.
/// This is used by the linter to find items that need attention.
pub fn check_todo_comment(stmt: &Statement, source: &str) -> Vec<AstDiagnostic> {
    let mut results = Vec::new();
    let line = stmt.span.start_line as usize;
    if let Some(source_line) = source.lines().nth(line.saturating_sub(1)) {
        let upper = source_line.to_uppercase();
        for marker in &["TODO", "FIXME", "HACK", "XXX"] {
            if upper.contains(marker) {
                let code = ErrorCode::W244;
                results.push(AstDiagnostic {
                    line: stmt.span.start_line,
                    column: stmt.span.start_col,
                    message: format!("{} comment found", marker),
                    severity: DiagnosticSeverity::Info,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: None,
                    source_file: None,
                });
            }
        }
    }
    results
}


/// W246: Check for empty match body (match with no arms).
pub fn check_empty_match(expr: &Expression) -> Option<AstDiagnostic> {
    if let ExpressionKind::Match { arms, .. } = &expr.kind {
        if arms.is_empty() {
            let code = ErrorCode::W246;
            return Some(AstDiagnostic {
                line: expr.span.start_line,
                column: expr.span.start_col,
                message: "match expression has no arms".to_string(),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: Some("Add match arms or use a default `_ =>` case".to_string()),
                source_file: None,
            });
        }
    }
    None
}

/// W247: Check for unused imports in a program.
pub fn check_unused_imports(program: &Program) -> Vec<AstDiagnostic> {
    let mut results = Vec::new();
    let mut imported_names: Vec<(String, Span)> = Vec::new();
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Pass 1: collect imports
    for stmt in &program.statements {
        if let StatementKind::Use { path, .. } = &stmt.kind {
            for segment in path {
                imported_names.push((segment.clone(), stmt.span));
            }
        }
    }

    // Pass 2: collect all used identifiers (simplified)
    fn collect_used(expr: &Expression, used: &mut std::collections::HashSet<String>) {
        match &expr.kind {
            ExpressionKind::Variable(name) => { used.insert(name.clone()); }
            ExpressionKind::Call { name, args, .. } => {
                used.insert(name.clone());
                for a in args { collect_used(a, used); }
            }
            ExpressionKind::MethodCall { object, args, .. } => {
                collect_used(object, used);
                for a in args { collect_used(a, used); }
            }
            ExpressionKind::BinaryOp { left, right, .. } => {
                collect_used(left, used);
                collect_used(right, used);
            }
            _ => {}
        }
    }

    for stmt in &program.statements {
        match &stmt.kind {
            StatementKind::ExprStatement(e) | StatementKind::Output(e) | StatementKind::Throw(e) => {
                collect_used(e, &mut used_names);
            }
            StatementKind::Let { value, .. } | StatementKind::LetMut { value, .. } => {
                collect_used(value, &mut used_names);
            }
            _ => {}
        }
    }

    for (name, span) in &imported_names {
        if name != "*" && !used_names.contains(name) {
            let code = ErrorCode::W247;
            results.push(AstDiagnostic {
                line: span.start_line,
                column: span.start_col,
                message: format!("unused import: '{}'", name),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: Some(format!("Remove the unused import '{}'", name)),
                source_file: None,
            });
        }
    }
    results
}

/// W248: Check for unused functions (defined but never called).
pub fn check_unused_functions(program: &Program) -> Vec<AstDiagnostic> {
    let mut results = Vec::new();
    let mut defined: Vec<(String, Span)> = Vec::new();
    let mut called: std::collections::HashSet<String> = std::collections::HashSet::new();

    for stmt in &program.statements {
        match &stmt.kind {
            StatementKind::FunctionDef(f) => {
                if !f.name.starts_with('_') && f.name != "main" {
                    defined.push((f.name.clone(), f.span));
                }
            }
            _ => {}
        }
    }

    fn collect_calls(expr: &Expression, calls: &mut std::collections::HashSet<String>) {
        match &expr.kind {
            ExpressionKind::Call { name, args, .. } => {
                calls.insert(name.clone());
                for a in args { collect_calls(a, calls); }
            }
            ExpressionKind::Variable(name) => { calls.insert(name.clone()); }
            ExpressionKind::BinaryOp { left, right, .. } => {
                collect_calls(left, calls);
                collect_calls(right, calls);
            }
            ExpressionKind::MethodCall { object, args, .. } => {
                collect_calls(object, calls);
                for a in args { collect_calls(a, calls); }
            }
            _ => {}
        }
    }

    for stmt in &program.statements {
        match &stmt.kind {
            StatementKind::ExprStatement(e) | StatementKind::Output(e) => { collect_calls(e, &mut called); }
            StatementKind::Let { value, .. } | StatementKind::LetMut { value, .. } => { collect_calls(value, &mut called); }
            StatementKind::FunctionDef(f) => {
                for s in &f.body.statements {
                    if let StatementKind::ExprStatement(e) | StatementKind::Output(e) = &s.kind {
                        collect_calls(e, &mut called);
                    }
                }
            }
            _ => {}
        }
    }

    for (name, span) in &defined {
        if !called.contains(name) {
            let code = ErrorCode::W248;
            results.push(AstDiagnostic {
                line: span.start_line,
                column: span.start_col,
                message: format!("function '{}' is defined but never called", name),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: Some(format!("Remove '{}' or prefix with `_`", name)),
                source_file: None,
            });
        }
    }
    results
}

/// W249: Check for single-arm match (should be if-let).
pub fn check_single_match(expr: &Expression) -> Option<AstDiagnostic> {
    if let ExpressionKind::Match { arms, .. } = &expr.kind {
        if arms.len() == 1 {
            let code = ErrorCode::W249;
            return Some(AstDiagnostic {
                line: expr.span.start_line,
                column: expr.span.start_col,
                message: "match with single arm".to_string(),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: Some("Consider using `if let` instead of `match` with one arm".to_string()),
                source_file: None,
            });
        }
    }
    None
}

/// W250: Check for high cognitive complexity in a function.
pub fn check_cognitive_complexity(fdef: &FunctionDef) -> Option<AstDiagnostic> {
    const MAX_COMPLEXITY: usize = 25;
    fn count_complexity(stmts: &[Statement], depth: usize) -> usize {
        let mut score = 0;
        for stmt in stmts {
            match &stmt.kind {
                StatementKind::ForLoop { body, .. } | StatementKind::WhileLoop { body, .. }
                | StatementKind::DoWhileLoop { body, .. } => {
                    score += 1 + depth;
                    score += count_complexity(&body.statements, depth + 1);
                }
                StatementKind::TryCatch { try_block, catch_block, finally_block, .. } => {
                    score += 1;
                    score += count_complexity(&try_block.statements, depth + 1);
                    score += count_complexity(&catch_block.statements, depth + 1);
                    if let Some(fb) = finally_block { score += count_complexity(&fb.statements, depth + 1); }
                }
                StatementKind::Return(_) | StatementKind::Break { .. } | StatementKind::Continue { .. } => {
                    score += 1;
                }
                _ => {}
            }
        }
        score
    }
    let complexity = count_complexity(&fdef.body.statements, 1);
    if complexity > MAX_COMPLEXITY {
        let code = ErrorCode::W250;
        return Some(AstDiagnostic {
            line: fdef.span.start_line,
            column: fdef.span.start_col,
            message: format!("function '{}' has cognitive complexity {} (max {})", fdef.name, complexity, MAX_COMPLEXITY),
            severity: DiagnosticSeverity::Warning,
            code: Some(code.to_string()),
            help: Some(code.help().to_string()),
            suggestion: Some("Break this function into smaller helper functions".to_string()),
            source_file: None,
        });
    }
    None
}

/// W251: Check for overly long function bodies (> 100 lines).
pub fn check_function_length(fdef: &FunctionDef) -> Option<AstDiagnostic> {
    const MAX_LINES: u32 = 100;
    let body_lines = fdef.span.end_line.saturating_sub(fdef.span.start_line);
    if body_lines > MAX_LINES {
        let code = ErrorCode::W251;
        return Some(AstDiagnostic {
            line: fdef.span.start_line,
            column: fdef.span.start_col,
            message: format!("function '{}' is {} lines long (max {})", fdef.name, body_lines, MAX_LINES),
            severity: DiagnosticSeverity::Warning,
            code: Some(code.to_string()),
            help: Some(code.help().to_string()),
            suggestion: Some("Extract logic into helper functions".to_string()),
            source_file: None,
        });
    }
    None
}

/// W252: Check for manual map/filter reimplementation.
/// Detects patterns like: `let result = []; for x in arr { result.push(f(x)); }`.
pub fn check_manual_map(stmt: &Statement) -> Option<AstDiagnostic> {
    if let StatementKind::ForLoop { body, .. } = &stmt.kind {
        // Look for push calls inside the loop body
        for s in &body.statements {
            if let StatementKind::ExprStatement(expr) = &s.kind {
                if let ExpressionKind::MethodCall { method, .. } = &expr.kind {
                    if method == "push" {
                        let code = ErrorCode::W252;
                        return Some(AstDiagnostic {
                            line: stmt.span.start_line,
                            column: stmt.span.start_col,
                            message: "loop with push() may be replaceable with .map()".to_string(),
                            severity: DiagnosticSeverity::Info,
                            code: Some(code.to_string()),
                            help: Some(code.help().to_string()),
                            suggestion: Some("Consider using `.map()` or `.filter()` instead".to_string()),
                            source_file: None,
                        });
                    }
                }
            }
        }
    }
    None
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
                            source_file: None,
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
