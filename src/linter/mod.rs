//! AST-level linter for the MAGI language.
//!
//! Provides lint passes that analyze a parsed AST and produce diagnostics
//! for style issues, dead code, non-exhaustive matches, and more.

pub mod rules;

use crate::syntax::ast::*;
use crate::syntax::type_checker::AstDiagnostic;

/// Configuration for the linter.
#[derive(Debug, Clone, Default)]
pub struct LintConfig {
    /// Rule codes to disable (e.g., ["W200", "W201"]).
    pub disabled_rules: Vec<String>,
}

/// Result of a lint pass.
#[derive(Debug, Clone)]
pub struct LintResult {
    pub diagnostics: Vec<AstDiagnostic>,
}

/// Lint a parsed program, returning all diagnostics.
pub fn lint(program: &Program, config: &LintConfig) -> LintResult {
    let mut ctx = LintContext::new(config);
    ctx.check_program(program);
    LintResult {
        diagnostics: ctx.diagnostics,
    }
}

/// Internal lint context that walks the AST and accumulates diagnostics.
struct LintContext<'a> {
    config: &'a LintConfig,
    diagnostics: Vec<AstDiagnostic>,
    /// Known enum definitions: (name, [variant_names])
    enum_defs: Vec<(String, Vec<String>)>,
}

impl<'a> LintContext<'a> {
    fn new(config: &'a LintConfig) -> Self {
        Self {
            config,
            diagnostics: Vec::new(),
            enum_defs: Vec::new(),
        }
    }

    fn emit(&mut self, diag: AstDiagnostic) {
        if let Some(ref code) = diag.code {
            if self.config.disabled_rules.contains(code) {
                return;
            }
        }
        self.diagnostics.push(diag);
    }

    fn emit_all(&mut self, diags: Vec<AstDiagnostic>) {
        for d in diags {
            self.emit(d);
        }
    }

    fn check_program(&mut self, program: &Program) {
        // First pass: collect enum definitions for exhaustiveness checks
        for stmt in &program.statements {
            if let StatementKind::EnumDef { name, variants } = &stmt.kind {
                let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
                self.enum_defs.push((name.clone(), variant_names));
            }
        }

        // Second pass: lint all statements
        for stmt in &program.statements {
            self.check_statement(stmt);
        }

        // Check dead code at program level
        let diags = rules::check_dead_code_in_block(&program.statements);
        self.emit_all(diags);

        // Check duplicate imports
        let diags = rules::check_duplicate_imports(&program.statements);
        self.emit_all(diags);

        // Check same-scope shadowing at program level
        let diags = rules::check_same_scope_shadowing(&program.statements);
        self.emit_all(diags);
    }

    fn check_statement(&mut self, stmt: &Statement) {
        match &stmt.kind {
            StatementKind::Let { name, value, .. } => {
                if let Some(d) = rules::check_naming_snake_case(name, stmt.span) {
                    self.emit(d);
                }
                self.check_expression(value);
            }
            StatementKind::LetMut { name, value, .. } => {
                if let Some(d) = rules::check_naming_snake_case(name, stmt.span) {
                    self.emit(d);
                }
                self.check_expression(value);
            }
            StatementKind::LetDestructure { pattern, value, .. } => {
                match pattern {
                    DestructurePattern::Array(elements) => {
                        for elem in elements {
                            let name = match elem {
                                DestructureElement::Name(n) => n,
                                DestructureElement::Rest(n) => n,
                            };
                            if let Some(d) = rules::check_naming_snake_case(name, stmt.span) {
                                self.emit(d);
                            }
                        }
                    }
                    DestructurePattern::Map(entries) => {
                        for (key, alias) in entries {
                            let name = alias.as_deref().unwrap_or(key.as_str());
                            if let Some(d) = rules::check_naming_snake_case(name, stmt.span) {
                                self.emit(d);
                            }
                        }
                    }
                }
                self.check_expression(value);
            }
            StatementKind::Assignment { value, .. } => {
                self.check_expression(value);
            }
            StatementKind::CompoundAssign { value, .. } => {
                self.check_expression(value);
            }
            StatementKind::FunctionDef(fdef) => {
                if let Some(d) = rules::check_naming_snake_case(&fdef.name, fdef.span) {
                    self.emit(d);
                }
                for param in &fdef.params {
                    if let Some(d) = rules::check_naming_snake_case(&param.name, param.span) {
                        self.emit(d);
                    }
                    if let Some(ref default) = param.default {
                        self.check_expression(default);
                    }
                }
                if let Some(d) = rules::check_empty_block(&fdef.body, "function", fdef.span) {
                    self.emit(d);
                }
                // W211: unused function parameters
                let diags = rules::check_unused_params(fdef);
                self.emit_all(diags);
                self.check_block(&fdef.body);
            }
            StatementKind::AsyncFunctionDef(fdef) => {
                if let Some(d) = rules::check_naming_snake_case(&fdef.name, fdef.span) {
                    self.emit(d);
                }
                for param in &fdef.params {
                    if let Some(d) = rules::check_naming_snake_case(&param.name, param.span) {
                        self.emit(d);
                    }
                    if let Some(ref default) = param.default {
                        self.check_expression(default);
                    }
                }
                if let Some(d) = rules::check_empty_block(&fdef.body, "function", fdef.span) {
                    self.emit(d);
                }
                // W211: unused function parameters
                let diags = rules::check_unused_params(fdef);
                self.emit_all(diags);
                self.check_block(&fdef.body);
            }
            StatementKind::EnumDef { name, variants } => {
                // Collect for exhaustiveness checks (handles enums in nested scopes)
                let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
                if !self.enum_defs.iter().any(|(n, _)| n == name) {
                    self.enum_defs.push((name.clone(), variant_names));
                }
                if let Some(d) = rules::check_naming_pascal_case(name, stmt.span) {
                    self.emit(d);
                }
                for variant in variants {
                    if let Some(d) = rules::check_naming_pascal_case(&variant.name, variant.span) {
                        self.emit(d);
                    }
                }
            }
            StatementKind::StructDef { name, .. } => {
                if let Some(d) = rules::check_naming_pascal_case(name, stmt.span) {
                    self.emit(d);
                }
            }
            StatementKind::ForLoop { pattern, iterable, body } => {
                self.check_for_pattern(pattern, stmt.span);
                if let Some(d) = rules::check_empty_block(body, "for-loop", stmt.span) {
                    self.emit(d);
                }
                self.check_expression(iterable);
                self.check_block(body);
            }
            StatementKind::WhileLoop { condition, body } => {
                if let Some(d) = rules::check_constant_condition(condition, Some(body)) {
                    self.emit(d);
                }
                if let Some(d) = rules::check_empty_block(body, "while-loop", stmt.span) {
                    self.emit(d);
                }
                self.check_expression(condition);
                self.check_block(body);
            }
            StatementKind::Output(expr) => {
                self.check_expression(expr);
            }
            StatementKind::ExprStatement(expr) => {
                self.check_expression(expr);
            }
            StatementKind::Return(Some(expr)) => {
                self.check_expression(expr);
            }
            StatementKind::Break(Some(expr)) => {
                self.check_expression(expr);
            }
            StatementKind::Throw(expr) => {
                self.check_expression(expr);
            }
            StatementKind::TryCatch {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                if let Some(var) = catch_var {
                    if let Some(d) = rules::check_naming_snake_case(var, stmt.span) {
                        self.emit(d);
                    }
                }
                self.check_block(try_block);
                // Don't emit W206 for empty catch blocks — intentional error suppression is idiomatic
                self.check_block(catch_block);
                if let Some(fb) = finally_block {
                    // W212: return/break/continue/throw in finally
                    let diags = rules::check_return_in_finally(fb, stmt.span);
                    self.emit_all(diags);
                    self.check_block(fb);
                }
            }
            StatementKind::TypeAlias { name, .. } => {
                if let Some(d) = rules::check_naming_pascal_case(name, stmt.span) {
                    self.emit(d);
                }
            }
            StatementKind::ConstDef { name, value, .. } => {
                if let Some(d) = rules::check_naming_snake_case(name, stmt.span) {
                    self.emit(d);
                }
                self.check_expression(value);
            }
            StatementKind::ModuleDef { body, .. } => {
                self.check_block(body);
            }
            StatementKind::TestDef { body, .. } => {
                self.check_block(body);
            }
            _ => {}
        }
    }

    fn check_block(&mut self, block: &Block) {
        // Check dead code within this block
        let diags = rules::check_dead_code_in_block(&block.statements);
        self.emit_all(diags);

        // Check same-scope shadowing within this block
        let diags = rules::check_same_scope_shadowing(&block.statements);
        self.emit_all(diags);

        for stmt in &block.statements {
            self.check_statement(stmt);
        }

        if let Some(tail) = &block.tail_expr {
            self.check_expression(tail);
        }
    }

    fn check_for_pattern(&mut self, pattern: &ForPattern, span: Span) {
        match pattern {
            ForPattern::Single(name) => {
                if let Some(d) = rules::check_naming_snake_case(name, span) {
                    self.emit(d);
                }
            }
            ForPattern::ArrayDestructure(elements) => {
                for elem in elements {
                    let name = match elem {
                        DestructureElement::Name(n) => n,
                        DestructureElement::Rest(n) => n,
                    };
                    if let Some(d) = rules::check_naming_snake_case(name, span) {
                        self.emit(d);
                    }
                }
            }
            ForPattern::MapDestructure(entries) => {
                for (key, alias) in entries {
                    let name = alias.as_deref().unwrap_or(key.as_str());
                    if let Some(d) = rules::check_naming_snake_case(name, span) {
                        self.emit(d);
                    }
                }
            }
        }
    }

    fn check_expression(&mut self, expr: &Expression) {
        match &expr.kind {
            ExpressionKind::IfElse {
                condition,
                then_block,
                else_block,
            } => {
                if let Some(d) = rules::check_constant_condition(condition, None) {
                    self.emit(d);
                }
                if let Some(d) = rules::check_empty_block(then_block, "if", expr.span) {
                    self.emit(d);
                }
                if let Some(eb) = else_block {
                    if let Some(d) = rules::check_empty_block(eb, "else", expr.span) {
                        self.emit(d);
                    }
                }
                self.check_expression(condition);
                self.check_block(then_block);
                if let Some(eb) = else_block {
                    self.check_block(eb);
                }
            }
            ExpressionKind::Match { value, arms } => {
                self.check_expression(value);
                for arm in arms {
                    self.check_block(&arm.body);
                    if let Some(guard) = &arm.guard {
                        self.check_expression(guard);
                    }
                }

                // Check for unreachable arms
                let diags = rules::check_unreachable_arms(arms);
                self.emit_all(diags);

                // Check exhaustiveness
                let diags = rules::check_match_exhaustiveness(arms, &self.enum_defs);
                self.emit_all(diags);
            }
            ExpressionKind::Block(block) => {
                if let Some(d) = rules::check_empty_block(block, "block", expr.span) {
                    self.emit(d);
                }
                self.check_block(block);
            }
            ExpressionKind::BinaryOp { left, right, .. } => {
                self.check_expression(left);
                self.check_expression(right);
            }
            ExpressionKind::UnaryOp { operand, .. } => {
                self.check_expression(operand);
            }
            ExpressionKind::Call { args, kwargs, .. } => {
                for arg in args {
                    self.check_expression(arg);
                }
                for (_, arg) in kwargs {
                    self.check_expression(arg);
                }
            }
            ExpressionKind::MethodCall { object, args, kwargs, .. } => {
                self.check_expression(object);
                for arg in args {
                    self.check_expression(arg);
                }
                for (_, arg) in kwargs {
                    self.check_expression(arg);
                }
            }
            ExpressionKind::Pipe { left, right } => {
                self.check_expression(left);
                self.check_expression(right);
            }
            ExpressionKind::Index { object, index } => {
                self.check_expression(object);
                self.check_expression(index);
            }
            ExpressionKind::FieldAccess { object, .. } => {
                self.check_expression(object);
            }
            ExpressionKind::Lambda { params, body } => {
                for param in params {
                    if let Some(d) = rules::check_naming_snake_case(&param.name, param.span) {
                        self.emit(d);
                    }
                    if let Some(ref default) = param.default {
                        self.check_expression(default);
                    }
                }
                self.check_expression(body);
            }
            ExpressionKind::Loop(block) => {
                if let Some(d) = rules::check_empty_block(block, "loop", expr.span) {
                    self.emit(d);
                }
                self.check_block(block);
            }
            ExpressionKind::TryCatchExpr {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                if let Some(var) = catch_var {
                    if let Some(d) = rules::check_naming_snake_case(var, expr.span) {
                        self.emit(d);
                    }
                }
                self.check_block(try_block);
                self.check_block(catch_block);
                if let Some(finally) = finally_block {
                    // W212: return/break/continue/throw in finally
                    let diags = rules::check_return_in_finally(finally, expr.span);
                    self.emit_all(diags);
                    self.check_block(finally);
                }
            }
            ExpressionKind::ListComprehension {
                expr: inner,
                pattern,
                iterable,
                condition,
            } => {
                self.check_for_pattern(pattern, expr.span);
                self.check_expression(inner);
                self.check_expression(iterable);
                if let Some(cond) = condition {
                    self.check_expression(cond);
                }
            }
            ExpressionKind::MapComprehension {
                key_expr,
                value_expr,
                pattern,
                iterable,
                condition,
            } => {
                self.check_for_pattern(pattern, expr.span);
                self.check_expression(key_expr);
                self.check_expression(value_expr);
                self.check_expression(iterable);
                if let Some(cond) = condition {
                    self.check_expression(cond);
                }
            }
            ExpressionKind::Literal(Literal::Array(elems)) => {
                for e in elems {
                    self.check_expression(e);
                }
            }
            ExpressionKind::Literal(Literal::Map(entries)) => {
                for (_, e) in entries {
                    self.check_expression(e);
                }
            }
            ExpressionKind::StringInterpolation { parts } => {
                for part in parts {
                    if let StringPart::Expr(e) = part {
                        self.check_expression(e);
                    }
                }
            }
            ExpressionKind::Range { start, end, .. } => {
                self.check_expression(start);
                self.check_expression(end);
            }
            ExpressionKind::NullCoalesce { left, right } => {
                self.check_expression(left);
                self.check_expression(right);
            }
            ExpressionKind::OptionalChain { object, .. } => {
                self.check_expression(object);
            }
            ExpressionKind::Spread(inner) => {
                self.check_expression(inner);
            }
            ExpressionKind::Await(inner) => {
                self.check_expression(inner);
            }
            ExpressionKind::Spawn(inner) => {
                self.check_expression(inner);
            }
            ExpressionKind::TryPropagate(inner) => {
                self.check_expression(inner);
            }
            ExpressionKind::EnumConstruct { args, .. } => {
                for arg in args {
                    self.check_expression(arg);
                }
            }
            ExpressionKind::StructConstruct { fields, .. } => {
                for (_, val) in fields {
                    self.check_expression(val);
                }
            }
            _ => {}
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parser::parse_v2;

    fn lint_source(source: &str) -> Vec<AstDiagnostic> {
        let program = parse_v2(source).expect("parse failed");
        let config = LintConfig::default();
        lint(&program, &config).diagnostics
    }

    fn lint_codes(source: &str) -> Vec<String> {
        lint_source(source)
            .iter()
            .filter_map(|d| d.code.clone())
            .collect()
    }

    // W200: snake_case for variables/functions
    #[test]
    fn test_w200_variable_naming() {
        let codes = lint_codes("let myVar = 5;");
        assert!(codes.contains(&"W200".to_string()), "expected W200, got {:?}", codes);
    }

    #[test]
    fn test_w200_function_naming() {
        let codes = lint_codes("fn myFunc() { 1 }");
        assert!(codes.contains(&"W200".to_string()), "expected W200, got {:?}", codes);
    }

    #[test]
    fn test_w200_snake_case_ok() {
        let codes = lint_codes("let my_var = 5;\nfn my_func() { 1 }");
        assert!(!codes.contains(&"W200".to_string()), "should not warn on snake_case: {:?}", codes);
    }

    #[test]
    fn test_w200_underscore_suppression() {
        let codes = lint_codes("let _ignored = 5;");
        assert!(!codes.contains(&"W200".to_string()), "_ prefix should suppress: {:?}", codes);
    }

    // W201: PascalCase for enums/structs
    #[test]
    fn test_w201_enum_naming() {
        let codes = lint_codes("enum my_color { Red, Green }");
        assert!(codes.contains(&"W201".to_string()), "expected W201, got {:?}", codes);
    }

    #[test]
    fn test_w201_struct_naming() {
        let codes = lint_codes("struct my_point { x: int64 }");
        assert!(codes.contains(&"W201".to_string()), "expected W201, got {:?}", codes);
    }

    #[test]
    fn test_w201_pascal_case_ok() {
        let codes = lint_codes("enum Color { Red, Green }\nstruct Point { x: int64 }");
        assert!(!codes.contains(&"W201".to_string()), "should not warn on PascalCase: {:?}", codes);
    }

    // W202: dead code after return/break/continue/throw
    #[test]
    fn test_w202_dead_code_after_return() {
        let codes = lint_codes("fn foo() {\n  return 1;\n  let x = 2;\n}");
        assert!(codes.contains(&"W202".to_string()), "expected W202, got {:?}", codes);
    }

    #[test]
    fn test_w202_no_dead_code() {
        let codes = lint_codes("fn foo() {\n  let x = 1;\n  return x;\n}");
        assert!(!codes.contains(&"W202".to_string()), "should not warn: {:?}", codes);
    }

    // W204: constant condition
    #[test]
    fn test_w204_if_true() {
        let codes = lint_codes("if true { 1 }");
        assert!(codes.contains(&"W204".to_string()), "expected W204, got {:?}", codes);
    }

    #[test]
    fn test_w204_if_false() {
        let codes = lint_codes("if false { 1 }");
        assert!(codes.contains(&"W204".to_string()), "expected W204, got {:?}", codes);
    }

    #[test]
    fn test_w204_normal_condition() {
        let codes = lint_codes("let x = true;\nif x { 1 }");
        assert!(!codes.contains(&"W204".to_string()), "should not warn: {:?}", codes);
    }

    // W206: empty block
    #[test]
    fn test_w206_empty_block() {
        let codes = lint_codes("fn foo() {}");
        assert!(codes.contains(&"W206".to_string()), "expected W206, got {:?}", codes);
    }

    // W207: unreachable match arm
    #[test]
    fn test_w207_unreachable_after_wildcard() {
        let codes = lint_codes("let x = 1;\nmatch x {\n  _ => 0,\n  1 => 1,\n}");
        assert!(codes.contains(&"W207".to_string()), "expected W207, got {:?}", codes);
    }

    #[test]
    fn test_w207_no_unreachable() {
        let codes = lint_codes("let x = 1;\nmatch x {\n  1 => 1,\n  _ => 0,\n}");
        assert!(!codes.contains(&"W207".to_string()), "should not warn: {:?}", codes);
    }

    // W200: snake_case suggestion
    #[test]
    fn test_w200_suggestion() {
        let diags = lint_source("let myVar = 5;");
        let w200 = diags.iter().find(|d| d.code.as_deref() == Some("W200")).unwrap();
        assert!(w200.suggestion.as_ref().unwrap().contains("my_var"), "expected snake_case suggestion, got: {:?}", w200.suggestion);
    }

    // W208: duplicate imports
    #[test]
    fn test_w208_duplicate_import() {
        let codes = lint_codes("import \"std\";\nimport \"std\";");
        assert!(codes.contains(&"W208".to_string()), "expected W208, got {:?}", codes);
    }

    #[test]
    fn test_w208_no_duplicate() {
        let codes = lint_codes("import \"std\";\nimport \"io\";");
        assert!(!codes.contains(&"W208".to_string()), "should not warn: {:?}", codes);
    }

    #[test]
    fn test_w208_duplicate_use() {
        let codes = lint_codes("use std::math;\nuse std::math;");
        assert!(codes.contains(&"W208".to_string()), "expected W208 for duplicate use, got {:?}", codes);
    }

    // Config: disabled rules
    #[test]
    fn test_disabled_rule() {
        let program = parse_v2("let myVar = 5;").expect("parse failed");
        let config = LintConfig {
            disabled_rules: vec!["W200".to_string()],
        };
        let result = lint(&program, &config);
        let codes: Vec<String> = result.diagnostics.iter().filter_map(|d| d.code.clone()).collect();
        assert!(!codes.contains(&"W200".to_string()), "W200 should be disabled: {:?}", codes);
    }

    // W202: dead code after terminating if/else
    #[test]
    fn test_w202_dead_code_after_terminating_if_else() {
        let codes = lint_codes(r#"
fn foo(x) {
    if x > 0 { return 1; } else { return 2; }
    let y = 3;
}
"#);
        assert!(codes.contains(&"W202".to_string()), "expected W202 for dead code after terminating if/else, got {:?}", codes);
    }

    #[test]
    fn test_w202_no_dead_code_if_else_one_branch() {
        // Only one branch terminates — the code after is reachable
        let codes = lint_codes(r#"
fn foo(x) {
    if x > 0 { return 1; } else { let z = 2; }
    let y = 3;
}
"#);
        assert!(!codes.contains(&"W202".to_string()), "should not warn when only one branch returns: {:?}", codes);
    }

    // ── W202: dead code after terminating match/loop/try-catch ──

    #[test]
    fn test_w202_dead_code_after_terminating_match() {
        let codes = lint_codes(r#"
fn foo(x) {
    match x {
        1 => { return 1; }
        _ => { return 0; }
    }
    let y = 3;
}
"#);
        assert!(codes.contains(&"W202".to_string()),
            "expected W202 for dead code after all-arms-return match, got {:?}", codes);
    }

    #[test]
    fn test_w202_no_dead_code_match_partial_return() {
        // Only one arm returns — code after is reachable
        let codes = lint_codes(r#"
fn foo(x) {
    match x {
        1 => { return 1; }
        _ => { let z = 0; }
    }
    let y = 3;
}
"#);
        assert!(!codes.contains(&"W202".to_string()),
            "should not warn when only one match arm returns: {:?}", codes);
    }

    #[test]
    fn test_w202_dead_code_after_loop() {
        let codes = lint_codes(r#"
fn foo() {
    loop { return 1; }
    let x = 2;
}
"#);
        assert!(codes.contains(&"W202".to_string()),
            "expected W202 for dead code after loop-with-return, got {:?}", codes);
    }

    #[test]
    fn test_w202_dead_code_after_try_catch_both_return() {
        let codes = lint_codes(r#"
fn foo() {
    try { return 1; } catch e { return 2; }
    let x = 3;
}
"#);
        assert!(codes.contains(&"W202".to_string()),
            "expected W202 for dead code after try-catch-both-return, got {:?}", codes);
    }

    #[test]
    fn test_w202_no_dead_code_try_catch_one_return() {
        // Only try block returns — code after is reachable
        let codes = lint_codes(r#"
fn foo() {
    try { return 1; } catch e { let x = 2; }
    let y = 3;
}
"#);
        assert!(!codes.contains(&"W202".to_string()),
            "should not warn when only try block returns: {:?}", codes);
    }

    // ── W200: comprehension pattern and catch variable naming ──

    #[test]
    fn test_w200_comprehension_pattern() {
        let codes = lint_codes("let items = [1, 2, 3];\nlet result = [x for myItem in items];");
        assert!(codes.contains(&"W200".to_string()),
            "expected W200 for non-snake_case comprehension var, got {:?}", codes);
    }

    #[test]
    fn test_w200_map_comprehension_pattern() {
        let codes = lint_codes("let items = [1, 2, 3];\nlet result = {\"k\": v for myItem in items};");
        assert!(codes.contains(&"W200".to_string()),
            "expected W200 for non-snake_case map comprehension var, got {:?}", codes);
    }

    #[test]
    fn test_w200_catch_var_naming() {
        let codes = lint_codes("try { 1 } catch myError { 2 }");
        assert!(codes.contains(&"W200".to_string()),
            "expected W200 for non-snake_case catch var, got {:?}", codes);
    }

    #[test]
    fn test_w200_catch_var_snake_case_ok() {
        let codes = lint_codes("try { 1 } catch my_error { 2 }");
        assert!(!codes.contains(&"W200".to_string()),
            "should not warn on snake_case catch var: {:?}", codes);
    }

    // ── W204: constant condition — while true with break suppression ──

    #[test]
    fn test_w204_while_true_with_break_suppressed() {
        // while true { break; } is idiomatic — should NOT warn W204
        let codes = lint_codes("while true { break; }");
        assert!(!codes.contains(&"W204".to_string()),
            "should not warn W204 for while true with break: {:?}", codes);
    }

    #[test]
    fn test_w204_while_true_no_break_warns() {
        // while true { output 1; } has no break — should warn
        let codes = lint_codes("while true { output 1; }");
        assert!(codes.contains(&"W204".to_string()),
            "expected W204 for while true without break: {:?}", codes);
    }

    #[test]
    fn test_w204_while_false_warns() {
        let codes = lint_codes("while false { output 1; }");
        assert!(codes.contains(&"W204".to_string()),
            "expected W204 for while false: {:?}", codes);
    }

    #[test]
    fn test_w204_while_true_nested_break_suppressed() {
        // break inside an if within the while body still targets the while loop
        let codes = lint_codes(r#"
while true {
    let x = 1;
    if x > 0 { break; }
}
"#);
        assert!(!codes.contains(&"W204".to_string()),
            "should not warn W204 for while true with conditional break: {:?}", codes);
    }

    #[test]
    fn test_w204_while_true_break_in_nested_loop_warns() {
        // break inside an inner loop does NOT break the outer while
        let codes = lint_codes(r#"
while true {
    for x in [1, 2, 3] {
        break;
    }
}
"#);
        assert!(codes.contains(&"W204".to_string()),
            "expected W204: break in inner loop does not target outer while: {:?}", codes);
    }

    // ── W209: Shadowed variable in same scope ──

    #[test]
    fn test_w209_same_scope_shadowing() {
        let codes = lint_codes("let x = 1;\nlet x = 2;");
        assert!(codes.contains(&"W209".to_string()),
            "expected W209 for same-scope shadowing, got {:?}", codes);
    }

    #[test]
    fn test_w209_no_warning_different_names() {
        let codes = lint_codes("let x = 1;\nlet y = 2;");
        assert!(!codes.contains(&"W209".to_string()),
            "should not warn when names are different: {:?}", codes);
    }

    #[test]
    fn test_w209_nested_scope_no_warning() {
        // Shadowing in nested scope is intentional and common
        let codes = lint_codes(r#"
let x = 1;
fn foo() {
    let x = 2;
    x
}
"#);
        // W209 should NOT fire for the nested `let x = 2` since it's in a different scope
        let w209_count = codes.iter().filter(|c| c.as_str() == "W209").count();
        assert_eq!(w209_count, 0,
            "should not warn for nested scope shadowing: {:?}", codes);
    }

    #[test]
    fn test_w209_underscore_prefix_suppressed() {
        let codes = lint_codes("let _x = 1;\nlet _x = 2;");
        assert!(!codes.contains(&"W209".to_string()),
            "_ prefix should suppress W209: {:?}", codes);
    }

    #[test]
    fn test_w209_let_mut_shadowing() {
        let codes = lint_codes("let mut x = 1;\nlet x = 2;");
        assert!(codes.contains(&"W209".to_string()),
            "expected W209 for let mut followed by let, got {:?}", codes);
    }

    #[test]
    fn test_w209_const_shadowing() {
        let codes = lint_codes("const X = 1;\nconst X = 2;");
        assert!(codes.contains(&"W209".to_string()),
            "expected W209 for const shadowing, got {:?}", codes);
    }

    #[test]
    fn test_w209_inside_function_body() {
        let codes = lint_codes(r#"
fn foo() {
    let x = 1;
    let x = 2;
    x
}
"#);
        assert!(codes.contains(&"W209".to_string()),
            "expected W209 inside function body, got {:?}", codes);
    }

    #[test]
    fn test_w209_suggestion_message() {
        let diags = lint_source("let x = 1;\nlet x = 2;");
        let w209 = diags.iter().find(|d| d.code.as_deref() == Some("W209")).unwrap();
        assert!(w209.message.contains("shadows"), "message should mention shadowing: {}", w209.message);
        assert!(w209.suggestion.is_some(), "should have a suggestion");
    }

    // ── W211: Unused function parameter ──

    #[test]
    fn test_w211_unused_param() {
        let codes = lint_codes("fn foo(x, y) { x }");
        assert!(codes.contains(&"W211".to_string()),
            "expected W211 for unused parameter y, got {:?}", codes);
    }

    #[test]
    fn test_w211_all_params_used() {
        let codes = lint_codes("fn foo(x, y) { x + y }");
        assert!(!codes.contains(&"W211".to_string()),
            "should not warn when all params used: {:?}", codes);
    }

    #[test]
    fn test_w211_underscore_prefix_suppressed() {
        let codes = lint_codes("fn foo(x, _y) { x }");
        assert!(!codes.contains(&"W211".to_string()),
            "_ prefix should suppress W211: {:?}", codes);
    }

    #[test]
    fn test_w211_param_used_in_nested_block() {
        let codes = lint_codes(r#"
fn foo(x) {
    if true {
        x + 1
    } else {
        0
    }
}
"#);
        assert!(!codes.contains(&"W211".to_string()),
            "should not warn when param used in nested block: {:?}", codes);
    }

    #[test]
    fn test_w211_param_used_in_method_call() {
        let codes = lint_codes("fn foo(arr) { arr.len() }");
        assert!(!codes.contains(&"W211".to_string()),
            "should not warn when param used as method receiver: {:?}", codes);
    }

    #[test]
    fn test_w211_param_used_in_string_interpolation() {
        let codes = lint_codes(r#"fn foo(name) { f"hello {name}" }"#);
        assert!(!codes.contains(&"W211".to_string()),
            "should not warn when param used in f-string: {:?}", codes);
    }

    #[test]
    fn test_w211_multiple_unused_params() {
        let diags = lint_source("fn foo(a, b, c) { 42 }");
        let w211_count = diags.iter().filter(|d| d.code.as_deref() == Some("W211")).count();
        assert_eq!(w211_count, 3, "expected W211 for all 3 params, got {}", w211_count);
    }

    #[test]
    fn test_w211_suggestion_message() {
        let diags = lint_source("fn foo(x, y) { x }");
        let w211 = diags.iter().find(|d| d.code.as_deref() == Some("W211")).unwrap();
        assert!(w211.message.contains("y"), "message should mention the param name: {}", w211.message);
        assert!(w211.suggestion.as_ref().unwrap().contains("_y"), "suggestion should suggest _y: {:?}", w211.suggestion);
    }

    #[test]
    fn test_w211_async_function() {
        let codes = lint_codes("async fn foo(x, y) { x }");
        assert!(codes.contains(&"W211".to_string()),
            "expected W211 for unused async param y, got {:?}", codes);
    }

    #[test]
    fn test_w211_param_used_in_lambda() {
        // Parameter used inside a lambda within the function body
        let codes = lint_codes(r#"
fn foo(x) {
    let f = |y| x + y;
    f(1)
}
"#);
        assert!(!codes.contains(&"W211".to_string()),
            "should not warn when param used in nested lambda: {:?}", codes);
    }

    // ── W212: Return in finally block ──

    #[test]
    fn test_w212_return_in_finally() {
        let codes = lint_codes(r#"
fn foo() {
    try { 1 } catch e { 2 } finally { return 3; }
}
"#);
        assert!(codes.contains(&"W212".to_string()),
            "expected W212 for return in finally, got {:?}", codes);
    }

    #[test]
    fn test_w212_throw_in_finally() {
        let codes = lint_codes(r#"
fn foo() {
    try { 1 } catch e { 2 } finally { throw "error"; }
}
"#);
        assert!(codes.contains(&"W212".to_string()),
            "expected W212 for throw in finally, got {:?}", codes);
    }

    #[test]
    fn test_w212_break_in_finally() {
        let codes = lint_codes(r#"
while true {
    try { 1 } catch e { 2 } finally { break; }
}
"#);
        assert!(codes.contains(&"W212".to_string()),
            "expected W212 for break in finally, got {:?}", codes);
    }

    #[test]
    fn test_w212_no_warning_clean_finally() {
        let codes = lint_codes(r#"
fn foo() {
    try { 1 } catch e { 2 } finally { output "cleanup"; }
}
"#);
        assert!(!codes.contains(&"W212".to_string()),
            "should not warn for clean finally block: {:?}", codes);
    }

    #[test]
    fn test_w212_return_in_finally_expr_form() {
        // Expression form of try/catch/finally
        let codes = lint_codes(r#"
fn foo() {
    let result = try { 1 } catch e { 2 } finally { return 3; };
    result
}
"#);
        assert!(codes.contains(&"W212".to_string()),
            "expected W212 for return in finally (expr form), got {:?}", codes);
    }

    #[test]
    fn test_w212_break_in_loop_inside_finally_no_warning() {
        // break inside a loop within finally targets the loop, not the finally block
        let codes = lint_codes(r#"
fn foo() {
    try { 1 } catch e { 2 } finally {
        for x in [1, 2, 3] {
            break;
        }
    }
}
"#);
        assert!(!codes.contains(&"W212".to_string()),
            "break in loop inside finally should not warn: {:?}", codes);
    }

    #[test]
    fn test_w212_return_in_loop_inside_finally_warns() {
        // return inside a loop within finally still overrides the try/catch
        let codes = lint_codes(r#"
fn foo() {
    try { 1 } catch e { 2 } finally {
        for x in [1, 2, 3] {
            return 99;
        }
    }
}
"#);
        assert!(codes.contains(&"W212".to_string()),
            "return in loop inside finally should still warn: {:?}", codes);
    }

    #[test]
    fn test_w212_return_in_fn_inside_finally_no_warning() {
        // return inside a nested function within finally is for that function, not the finally
        let codes = lint_codes(r#"
fn foo() {
    try { 1 } catch e { 2 } finally {
        fn helper() { return 42; }
        helper();
    }
}
"#);
        assert!(!codes.contains(&"W212".to_string()),
            "return in nested fn inside finally should not warn: {:?}", codes);
    }

    #[test]
    fn test_w212_return_in_if_inside_finally_warns() {
        let codes = lint_codes(r#"
fn foo(x) {
    try { 1 } catch e { 2 } finally {
        if x > 0 { return 99; }
    }
}
"#);
        assert!(codes.contains(&"W212".to_string()),
            "return in if-block inside finally should warn: {:?}", codes);
    }

}
