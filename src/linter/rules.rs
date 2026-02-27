//! Individual lint rule implementations.

use crate::syntax::ast::*;
use crate::syntax::errors::ErrorCode;
use crate::syntax::type_checker::AstDiagnostic;
use crate::eval::DiagnosticSeverity;

/// Convert a name to snake_case, handling acronyms correctly.
/// e.g. "HTTPServer" → "http_server", "myFunc" → "my_func"
fn to_snake_case(name: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let prev_upper = i > 0 && chars[i - 1].is_ascii_uppercase();
            let next_lower = i + 1 < chars.len() && chars[i + 1].is_ascii_lowercase();
            // Insert underscore before:
            // 1. A capital that follows a lowercase (camelCase boundary)
            // 2. A capital in an acronym run that precedes a lowercase (e.g. the S in HTTPServer)
            if i > 0 && (!prev_upper || next_lower) {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
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
    let first_upper = name.chars().next().map_or(false, |c| c.is_ascii_uppercase());
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
            _ => {}
        }
    }

    diagnostics
}

/// Check for constant conditions in if/while expressions.
pub fn check_constant_condition(condition: &Expression) -> Option<AstDiagnostic> {
    if let ExpressionKind::Literal(Literal::Bool(val)) = &condition.kind {
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
