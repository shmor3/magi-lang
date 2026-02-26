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
                }
                if let Some(d) = rules::check_empty_block(&fdef.body, "function", fdef.span) {
                    self.emit(d);
                }
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
                }
                if let Some(d) = rules::check_empty_block(&fdef.body, "function", fdef.span) {
                    self.emit(d);
                }
                self.check_block(&fdef.body);
            }
            StatementKind::EnumDef { name, variants } => {
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
                match pattern {
                    ForPattern::Single(name) => {
                        if let Some(d) = rules::check_naming_snake_case(name, stmt.span) {
                            self.emit(d);
                        }
                    }
                    ForPattern::ArrayDestructure(elements) => {
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
                    ForPattern::MapDestructure(entries) => {
                        for (key, alias) in entries {
                            let name = alias.as_deref().unwrap_or(key.as_str());
                            if let Some(d) = rules::check_naming_snake_case(name, stmt.span) {
                                self.emit(d);
                            }
                        }
                    }
                }
                if let Some(d) = rules::check_empty_block(body, "for-loop", stmt.span) {
                    self.emit(d);
                }
                self.check_expression(iterable);
                self.check_block(body);
            }
            StatementKind::WhileLoop { condition, body } => {
                if let Some(d) = rules::check_constant_condition(condition) {
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
                catch_block,
                finally_block,
                ..
            } => {
                self.check_block(try_block);
                // Don't emit W206 for empty catch blocks — intentional error suppression is idiomatic
                self.check_block(catch_block);
                if let Some(fb) = finally_block {
                    self.check_block(fb);
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

        for stmt in &block.statements {
            self.check_statement(stmt);
        }

        if let Some(tail) = &block.tail_expr {
            self.check_expression(tail);
        }
    }

    fn check_expression(&mut self, expr: &Expression) {
        match &expr.kind {
            ExpressionKind::IfElse {
                condition,
                then_block,
                else_block,
            } => {
                if let Some(d) = rules::check_constant_condition(condition) {
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
                catch_block,
                ..
            } => {
                self.check_block(try_block);
                self.check_block(catch_block);
            }
            ExpressionKind::ListComprehension {
                expr: inner,
                iterable,
                condition,
                ..
            } => {
                self.check_expression(inner);
                self.check_expression(iterable);
                if let Some(cond) = condition {
                    self.check_expression(cond);
                }
            }
            ExpressionKind::MapComprehension {
                key_expr,
                value_expr,
                iterable,
                condition,
                ..
            } => {
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
}
