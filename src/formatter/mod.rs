//! AST pretty-printer / code formatter for the MAGI language.
//!
//! Takes a parsed `Program` and produces formatted source code.
//! Comments are lost (the parser discards them), which is acceptable for v1.

use crate::syntax::ast::*;

/// Returns true if a statement is a "definition" (function, enum, struct, etc.)
/// that should be separated by blank lines in formatted output.
fn is_definition(stmt: &Statement) -> bool {
    matches!(
        &stmt.kind,
        StatementKind::FunctionDef(_)
            | StatementKind::AsyncFunctionDef(_)
            | StatementKind::EnumDef { .. }
            | StatementKind::StructDef { .. }
            | StatementKind::ModuleDef { .. }
            | StatementKind::TestDef { .. }
            | StatementKind::ConstDef { .. }
            | StatementKind::TypeAlias { .. }
    )
}

/// Re-escape a string's contents so that control characters are represented
/// as their escape sequences (e.g., newline → `\n`). This ensures the
/// formatter produces valid, parseable string literals.
fn escape_string_contents(s: &str) -> String {
    // Fast path: if the string is all ASCII and contains no characters that
    // need escaping, return it as-is without allocating.
    if s.is_ascii() && !s.bytes().any(|b| b == b'"' || b == b'\\' || b < 0x20 || b == 0x7f) {
        return s.to_string();
    }

    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() => {
                if (c as u32) < 0x80 {
                    out.push_str(&format!("\\x{:02x}", c as u32));
                } else {
                    out.push_str(&format!("\\u{{{:04x}}}", c as u32));
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// Configuration for the formatter.
#[derive(Debug, Clone)]
pub struct FormatConfig {
    /// Number of spaces per indentation level (default: 4).
    pub indent_width: usize,
    /// Maximum line width before wrapping (default: 100).
    pub max_width: usize,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            indent_width: 4,
            max_width: 100,
        }
    }
}

/// Format a parsed program into a source string.
pub fn format_program(program: &Program, config: &FormatConfig) -> String {
    let mut f = Formatter::new(config);
    f.fmt_program(program);
    // Ensure trailing newline
    let output = f.output.trim_end().to_string();
    if output.is_empty() {
        String::new()
    } else {
        format!("{}\n", output)
    }
}

struct Formatter<'a> {
    config: &'a FormatConfig,
    output: String,
    indent: usize,
    at_line_start: bool,
    depth: usize,
}

const MAX_FORMAT_DEPTH: usize = 128;

impl<'a> Formatter<'a> {
    fn new(config: &'a FormatConfig) -> Self {
        Self {
            config,
            output: String::new(),
            indent: 0,
            at_line_start: true,
            depth: 0,
        }
    }

    fn write(&mut self, s: &str) {
        if self.at_line_start && !s.is_empty() {
            let indent_str = " ".repeat(self.indent * self.config.indent_width);
            self.output.push_str(&indent_str);
            self.at_line_start = false;
        }
        self.output.push_str(s);
    }

    fn newline(&mut self) {
        self.output.push('\n');
        self.at_line_start = true;
    }

    fn indent(&mut self) {
        self.indent += 1;
    }

    fn dedent(&mut self) {
        self.indent = self.indent.saturating_sub(1);
    }

    /// Estimate the display width of an expression when formatted on a single line.
    /// Inherits the current depth to prevent exponential blowup on nested collections.
    fn expr_len(&self, expr: &Expression) -> usize {
        if self.depth >= MAX_FORMAT_DEPTH {
            return self.config.max_width; // force multi-line at extreme depth
        }
        let mut f = Formatter::new(self.config);
        f.depth = self.depth + 1; // inherit depth to bound recursion
        f.fmt_expression(expr);
        f.output.chars().count()
    }

    /// Check if a block is "short" enough to inline.
    fn is_short_block(&self, block: &Block) -> bool {
        if !block.statements.is_empty() {
            return false;
        }
        match &block.tail_expr {
            Some(expr) => self.expr_len(expr) < self.config.max_width / 2,
            None => true,
        }
    }

    fn fmt_program(&mut self, program: &Program) {
        let mut prev_was_def = false;

        for (i, stmt) in program.statements.iter().enumerate() {
            let is_def = is_definition(stmt);

            // Blank line before definitions (but not the very first statement)
            if i > 0 && (is_def || prev_was_def) {
                self.newline();
            }

            self.fmt_statement(stmt);
            self.newline();

            prev_was_def = is_def;
        }
    }

    fn fmt_statement(&mut self, stmt: &Statement) {
        match &stmt.kind {
            StatementKind::Import(path) => {
                self.write(&format!("import \"{}\";", escape_string_contents(path)));
            }
            StatementKind::Let {
                name,
                type_annotation,
                value,
            } => {
                self.write("let ");
                self.write(name);
                if let Some(ty) = type_annotation {
                    self.write(": ");
                    self.write(ty);
                }
                self.write(" = ");
                self.fmt_expression(value);
                self.write(";");
            }
            StatementKind::LetMut {
                name,
                type_annotation,
                value,
            } => {
                self.write("let mut ");
                self.write(name);
                if let Some(ty) = type_annotation {
                    self.write(": ");
                    self.write(ty);
                }
                self.write(" = ");
                self.fmt_expression(value);
                self.write(";");
            }
            StatementKind::LetDestructure {
                pattern,
                mutable,
                value,
            } => {
                if *mutable {
                    self.write("let mut ");
                } else {
                    self.write("let ");
                }
                self.fmt_destructure_pattern(pattern);
                self.write(" = ");
                self.fmt_expression(value);
                self.write(";");
            }
            StatementKind::Assignment { name, value } => {
                self.write(name);
                self.write(" = ");
                self.fmt_expression(value);
                self.write(";");
            }
            StatementKind::CompoundAssign { name, op, value } => {
                self.write(name);
                // Parser only produces CompoundAssign with Add/Sub/Mul/Div/Mod
                let op_str = match op {
                    BinOp::Add => "+=",
                    BinOp::Sub => "-=",
                    BinOp::Mul => "*=",
                    BinOp::Div => "/=",
                    _ => "%=",  // Mod (only remaining valid operator)
                };
                self.write(&format!(" {} ", op_str));
                self.fmt_expression(value);
                self.write(";");
            }
            StatementKind::FieldAssignment { object, field, value } => {
                self.fmt_expression(object);
                self.write(&format!(".{} = ", field));
                self.fmt_expression(value);
                self.write(";");
            }
            StatementKind::IndexAssignment { object, index, value } => {
                self.fmt_expression(object);
                self.write("[");
                self.fmt_expression(index);
                self.write("] = ");
                self.fmt_expression(value);
                self.write(";");
            }
            StatementKind::ForLoop {
                pattern,
                iterable,
                body,
            } => {
                self.write("for ");
                self.fmt_for_pattern(pattern);
                self.write(" in ");
                self.fmt_expression(iterable);
                self.write(" ");
                self.fmt_block(body);
            }
            StatementKind::WhileLoop { condition, body } => {
                self.write("while ");
                self.fmt_expression(condition);
                self.write(" ");
                self.fmt_block(body);
            }
            StatementKind::Output(expr) => {
                self.write("output ");
                self.fmt_expression(expr);
                self.write(";");
            }
            StatementKind::ExprStatement(expr) => {
                self.fmt_expression(expr);
                // Skip semicolons for block-ending expressions
                match &expr.kind {
                    ExpressionKind::IfElse { .. }
                    | ExpressionKind::Match { .. }
                    | ExpressionKind::Loop { .. }
                    | ExpressionKind::Block(_)
                    | ExpressionKind::TryCatchExpr { .. } => {}
                    _ => self.write(";"),
                }
            }
            StatementKind::FunctionDef(fdef) => {
                self.write("fn ");
                self.fmt_function_def(fdef);
            }
            StatementKind::AsyncFunctionDef(fdef) => {
                self.write("async fn ");
                self.fmt_function_def(fdef);
            }
            StatementKind::Break(None) => {
                self.write("break;");
            }
            StatementKind::Break(Some(expr)) => {
                self.write("break ");
                self.fmt_expression(expr);
                self.write(";");
            }
            StatementKind::Continue => {
                self.write("continue;");
            }
            StatementKind::Return(None) => {
                self.write("return;");
            }
            StatementKind::Return(Some(expr)) => {
                self.write("return ");
                self.fmt_expression(expr);
                self.write(";");
            }
            StatementKind::TryCatch {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                self.write("try ");
                self.fmt_block(try_block);
                self.write(" catch ");
                if let Some(var) = catch_var {
                    self.write(var);
                    self.write(" ");
                }
                self.fmt_block(catch_block);
                if let Some(fb) = finally_block {
                    self.write(" finally ");
                    self.fmt_block(fb);
                }
            }
            StatementKind::Throw(expr) => {
                self.write("throw ");
                self.fmt_expression(expr);
                self.write(";");
            }
            StatementKind::ConstDef {
                name,
                type_annotation,
                value,
            } => {
                self.write("const ");
                self.write(name);
                if let Some(ty) = type_annotation {
                    self.write(": ");
                    self.write(ty);
                }
                self.write(" = ");
                self.fmt_expression(value);
                self.write(";");
            }
            StatementKind::TypeAlias { name, target } => {
                self.write(&format!("type {} = {};", name, target));
            }
            StatementKind::ModuleDef { name, body } => {
                self.write("mod ");
                self.write(name);
                self.write(" ");
                self.fmt_block(body);
            }
            StatementKind::Use { path, alias, glob } => {
                self.write("use ");
                self.write(&path.join("::"));
                if *glob {
                    self.write("::*");
                }
                if let Some(alias) = alias {
                    self.write(" as ");
                    self.write(alias);
                }
                self.write(";");
            }
            StatementKind::TestDef { name, body } => {
                self.write(&format!("test \"{}\" ", escape_string_contents(name)));
                self.fmt_block(body);
            }
            StatementKind::EnumDef { name, variants } => {
                self.write("enum ");
                self.write(name);
                self.write(" {");
                self.newline();
                self.indent();
                for (i, variant) in variants.iter().enumerate() {
                    self.write(&variant.name);
                    if !variant.fields.is_empty() {
                        self.write("(");
                        self.write(&variant.fields.join(", "));
                        self.write(")");
                    }
                    if i < variants.len() - 1 {
                        self.write(",");
                    }
                    self.newline();
                }
                self.dedent();
                self.write("}");
            }
            StatementKind::StructDef { name, fields } => {
                self.write("struct ");
                self.write(name);
                self.write(" {");
                self.newline();
                self.indent();
                for (i, field) in fields.iter().enumerate() {
                    self.write(&field.name);
                    if let Some(ty) = &field.type_annotation {
                        self.write(": ");
                        self.write(ty);
                    }
                    if i < fields.len() - 1 {
                        self.write(",");
                    }
                    self.newline();
                }
                self.dedent();
                self.write("}");
            }
        }
    }

    fn fmt_function_def(&mut self, fdef: &FunctionDef) {
        self.write(&fdef.name);
        self.write("(");
        for (i, param) in fdef.params.iter().enumerate() {
            if param.rest {
                self.write("...");
            }
            self.write(&param.name);
            if let Some(ty) = &param.type_annotation {
                self.write(": ");
                self.write(ty);
            }
            if let Some(default) = &param.default {
                self.write(" = ");
                self.fmt_expression(default);
            }
            if i < fdef.params.len() - 1 {
                self.write(", ");
            }
        }
        self.write(")");
        if let Some(ret) = &fdef.return_type {
            self.write(" -> ");
            self.write(ret);
        }
        self.write(" ");
        self.fmt_block(&fdef.body);
    }

    fn fmt_block(&mut self, block: &Block) {
        // Try inline for short blocks
        if self.is_short_block(block) {
            if let Some(tail) = &block.tail_expr {
                self.write("{ ");
                self.fmt_expression(tail);
                self.write(" }");
                return;
            }
            self.write("{}");
            return;
        }

        self.write("{");
        self.newline();
        self.indent();

        let mut prev_was_def = false;
        for (i, stmt) in block.statements.iter().enumerate() {
            let is_def = is_definition(stmt);

            if i > 0 && (is_def || prev_was_def) {
                self.newline();
            }

            self.fmt_statement(stmt);
            self.newline();

            prev_was_def = is_def;
        }

        if let Some(tail) = &block.tail_expr {
            if prev_was_def {
                self.newline();
            }
            self.fmt_expression(tail);
            self.newline();
        }

        self.dedent();
        self.write("}");
    }

    fn fmt_expression(&mut self, expr: &Expression) {
        self.fmt_expression_prec(expr, 0);
    }

    /// Flatten a chain of Pipe expressions into a list of stages (left-to-right).
    fn collect_pipe_stages<'b>(&self, expr: &'b Expression, stages: &mut Vec<&'b Expression>) {
        match &expr.kind {
            ExpressionKind::Pipe { left, right } => {
                self.collect_pipe_stages(left, stages);
                self.collect_pipe_stages(right, stages);
            }
            _ => stages.push(expr),
        }
    }

    fn fmt_expression_prec(&mut self, expr: &Expression, parent_prec: u8) {
        if self.depth >= MAX_FORMAT_DEPTH {
            self.write("/* ... */");
            return;
        }
        self.depth += 1;
        self.fmt_expression_prec_inner(expr, parent_prec);
        self.depth -= 1;
    }

    fn fmt_expression_prec_inner(&mut self, expr: &Expression, parent_prec: u8) {
        match &expr.kind {
            ExpressionKind::Literal(lit) => self.fmt_literal(lit),
            ExpressionKind::Variable(name) => self.write(name),
            ExpressionKind::BinaryOp { op, left, right } => {
                let prec = op.precedence();
                let needs_parens = prec < parent_prec;
                if needs_parens {
                    self.write("(");
                }
                self.fmt_expression_prec(left, prec);
                self.write(&format!(" {} ", op));
                // Right side needs higher precedence to avoid ambiguity with left-assoc ops
                self.fmt_expression_prec(right, prec + 1);
                if needs_parens {
                    self.write(")");
                }
            }
            ExpressionKind::UnaryOp { op, operand } => {
                let needs_parens = parent_prec >= 8; // parenthesize when used as postfix object
                if needs_parens {
                    self.write("(");
                }
                self.write(&op.to_string());
                self.fmt_expression_prec(operand, 7); // High precedence for unary
                if needs_parens {
                    self.write(")");
                }
            }
            ExpressionKind::Call {
                name,
                args,
                kwargs,
            } => {
                self.write(name);
                self.write("(");
                self.fmt_args(args, kwargs);
                self.write(")");
            }
            ExpressionKind::MethodCall {
                object,
                method,
                args,
                kwargs,
            } => {
                // Check if object is an OptionalChain marker (empty field = ?.method() syntax)
                let is_optional_method = matches!(
                    &object.kind,
                    ExpressionKind::OptionalChain { field, .. } if field.is_empty()
                );
                if is_optional_method {
                    // Format as obj?.method() — the OptionalChain already outputs "?."
                    self.fmt_expression_prec(object, 9);
                } else {
                    self.fmt_expression_prec(object, 9);
                    self.write(".");
                }
                self.write(method);
                self.write("(");
                self.fmt_args(args, kwargs);
                self.write(")");
            }
            ExpressionKind::Pipe { .. } => {
                let needs_parens = parent_prec > 0;
                if needs_parens {
                    self.write("(");
                }
                // Flatten the pipe chain to avoid non-idempotent nested indentation
                let mut stages = Vec::new();
                self.collect_pipe_stages(expr, &mut stages);
                self.fmt_expression(stages[0]);
                self.indent();
                for stage in &stages[1..] {
                    self.newline();
                    self.write("|> ");
                    self.fmt_expression(stage);
                }
                self.dedent();
                if needs_parens {
                    self.write(")");
                }
            }
            ExpressionKind::IfElse {
                condition,
                then_block,
                else_block,
            } => {
                self.write("if ");
                self.fmt_expression(condition);
                self.write(" ");
                self.fmt_block(then_block);
                if let Some(eb) = else_block {
                    // Detect else-if chain: else block has no statements and
                    // its tail_expr is another IfElse
                    if eb.statements.is_empty() {
                        if let Some(tail) = &eb.tail_expr {
                            if matches!(tail.kind, ExpressionKind::IfElse { .. }) {
                                self.write(" else ");
                                self.fmt_expression(tail);
                                return;
                            }
                        }
                    }
                    self.write(" else ");
                    self.fmt_block(eb);
                }
            }
            ExpressionKind::Block(block) => {
                self.fmt_block(block);
            }
            ExpressionKind::Index { object, index } => {
                // Detect optional index: obj?[idx] — the parser wraps obj in
                // OptionalChain { field: "" } as a marker for ?[ syntax.
                let is_optional_index = matches!(
                    &object.kind,
                    ExpressionKind::OptionalChain { field, .. } if field.is_empty()
                );
                if is_optional_index {
                    // Extract the inner object from the OptionalChain marker
                    if let ExpressionKind::OptionalChain { object: inner, .. } = &object.kind {
                        self.fmt_expression_prec(inner, 9);
                        self.write("?[");
                    }
                } else {
                    self.fmt_expression_prec(object, 9);
                    self.write("[");
                }
                self.fmt_expression(index);
                self.write("]");
            }
            ExpressionKind::FieldAccess { object, field } => {
                self.fmt_expression_prec(object, 9);
                self.write(".");
                self.write(field);
            }
            ExpressionKind::Placeholder => {
                self.write("_");
            }
            ExpressionKind::Range {
                start,
                end,
                inclusive,
            } => {
                let needs_parens = parent_prec > 0;
                if needs_parens {
                    self.write("(");
                }
                // Range children are parsed by parse_binary_expr(0), so they cannot
                // contain Pipe, NullCoalesce, Range, or Lambda without parens.
                // Pass prec=1 to force parens on those sub-binary expression types.
                self.fmt_expression_prec(start, 1);
                if *inclusive {
                    self.write("..=");
                } else {
                    self.write("..");
                }
                self.fmt_expression_prec(end, 1);
                if needs_parens {
                    self.write(")");
                }
            }
            ExpressionKind::Await(inner) => {
                let needs_parens = parent_prec >= 8;
                if needs_parens {
                    self.write("(");
                }
                self.write("await ");
                self.fmt_expression_prec(inner, 7);
                if needs_parens {
                    self.write(")");
                }
            }
            ExpressionKind::Spawn(inner) => {
                let needs_parens = parent_prec >= 8;
                if needs_parens {
                    self.write("(");
                }
                self.write("spawn ");
                self.fmt_expression_prec(inner, 7);
                if needs_parens {
                    self.write(")");
                }
            }
            ExpressionKind::Lambda { params, body } => {
                let needs_parens = parent_prec > 0;
                if needs_parens {
                    self.write("(");
                }
                self.write("|");
                for (i, param) in params.iter().enumerate() {
                    if param.rest {
                        self.write("...");
                    }
                    self.write(&param.name);
                    if let Some(ty) = &param.type_annotation {
                        self.write(": ");
                        self.write(ty);
                    }
                    if let Some(default) = &param.default {
                        self.write(" = ");
                        self.fmt_expression(default);
                    }
                    if i < params.len() - 1 {
                        self.write(", ");
                    }
                }
                self.write("| ");
                self.fmt_expression(body);
                if needs_parens {
                    self.write(")");
                }
            }
            ExpressionKind::Match { value, arms } => {
                self.write("match ");
                self.fmt_expression(value);
                self.write(" {");
                self.newline();
                self.indent();
                for (i, arm) in arms.iter().enumerate() {
                    self.fmt_pattern(&arm.pattern);
                    if let Some(guard) = &arm.guard {
                        self.write(" if ");
                        self.fmt_expression(guard);
                    }
                    self.write(" => ");
                    // If the body is a short block with just a tail expression, inline it
                    if self.is_short_block(&arm.body) && arm.body.statements.is_empty() {
                        if let Some(tail) = &arm.body.tail_expr {
                            self.fmt_expression(tail);
                        } else {
                            self.write("{}");
                        }
                    } else {
                        self.fmt_block(&arm.body);
                    }
                    if i < arms.len() - 1 {
                        self.write(",");
                    }
                    self.newline();
                }
                self.dedent();
                self.write("}");
            }
            ExpressionKind::StringInterpolation { parts } => {
                self.write("f\"");
                for part in parts {
                    match part {
                        StringPart::Literal(s) => {
                            // Convert sentinel chars back to escaped braces before escaping.
                            // The lexer uses \u{FFF0}/\u{FFF1} as sentinels for \{/\} in f-strings.
                            let s = s.replace('\u{FFF0}', "{").replace('\u{FFF1}', "}");
                            // Escape special characters in the literal.
                            // Braces must use \{ and \} to round-trip through the parser.
                            let escaped = escape_string_contents(&s);
                            let escaped = escaped.replace('{', "\\{").replace('}', "\\}");
                            self.write(&escaped);
                        }
                        StringPart::Expr(e) => {
                            self.write("{");
                            self.fmt_expression(e);
                            self.write("}");
                        }
                    }
                }
                self.write("\"");
            }
            ExpressionKind::NullCoalesce { left, right } => {
                // NullCoalesce has very low precedence (below Or)
                let needs_parens = parent_prec > 0;
                if needs_parens {
                    self.write("(");
                }
                // NullCoalesce children are parsed by parse_range_expr, so they cannot
                // contain Pipe or Lambda without parens. Pass prec=1 to force parens
                // on sub-binary expression types (Pipe, Lambda, Range, NullCoalesce).
                self.fmt_expression_prec(left, 1);
                self.write(" ?? ");
                self.fmt_expression_prec(right, 1);
                if needs_parens {
                    self.write(")");
                }
            }
            ExpressionKind::OptionalChain { object, field } => {
                self.fmt_expression_prec(object, 8); // High precedence for postfix ?.
                self.write("?.");
                self.write(field);
            }
            ExpressionKind::Spread(inner) => {
                self.write("...");
                self.fmt_expression(inner);
            }
            ExpressionKind::Loop(block) => {
                self.write("loop ");
                self.fmt_block(block);
            }
            ExpressionKind::TryCatchExpr {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                self.write("try ");
                self.fmt_block(try_block);
                self.write(" catch ");
                if let Some(var) = catch_var {
                    self.write(var);
                    self.write(" ");
                }
                self.fmt_block(catch_block);
                if let Some(finally) = finally_block {
                    self.write(" finally ");
                    self.fmt_block(finally);
                }
            }
            ExpressionKind::ListComprehension {
                expr: inner,
                pattern,
                iterable,
                condition,
            } => {
                self.write("[");
                self.fmt_expression(inner);
                self.write(" for ");
                self.fmt_for_pattern(pattern);
                self.write(" in ");
                self.fmt_expression(iterable);
                if let Some(cond) = condition {
                    self.write(" if ");
                    self.fmt_expression(cond);
                }
                self.write("]");
            }
            ExpressionKind::MapComprehension {
                key_expr,
                value_expr,
                pattern,
                iterable,
                condition,
            } => {
                self.write("{");
                self.fmt_expression(key_expr);
                self.write(": ");
                self.fmt_expression(value_expr);
                self.write(" for ");
                self.fmt_for_pattern(pattern);
                self.write(" in ");
                self.fmt_expression(iterable);
                if let Some(cond) = condition {
                    self.write(" if ");
                    self.fmt_expression(cond);
                }
                self.write("}");
            }
            ExpressionKind::EnumConstruct {
                enum_name,
                variant,
                args,
            } => {
                self.write(enum_name);
                self.write("::");
                self.write(variant);
                if !args.is_empty() {
                    let inline_len: usize = args.iter().map(|e| self.expr_len(e) + 2).sum();
                    let prefix_len = enum_name.len() + 2 + variant.len();
                    if inline_len + prefix_len < self.config.max_width / 2 {
                        self.write("(");
                        for (i, arg) in args.iter().enumerate() {
                            self.fmt_expression(arg);
                            if i < args.len() - 1 {
                                self.write(", ");
                            }
                        }
                        self.write(")");
                    } else {
                        self.write("(");
                        self.newline();
                        self.indent();
                        for (i, arg) in args.iter().enumerate() {
                            self.fmt_expression(arg);
                            if i < args.len() - 1 {
                                self.write(",");
                            }
                            self.newline();
                        }
                        self.dedent();
                        self.write(")");
                    }
                }
            }
            ExpressionKind::StructConstruct { name, fields } => {
                self.write(name);
                if fields.is_empty() {
                    self.write(" {}");
                    return;
                }
                let inline_len: usize = fields
                    .iter()
                    .map(|(k, v)| k.len() + 4 + self.expr_len(v))
                    .sum();
                if inline_len + name.len() < self.config.max_width / 2 {
                    self.write(" { ");
                    for (i, (fname, val)) in fields.iter().enumerate() {
                        self.write(fname);
                        self.write(": ");
                        self.fmt_expression(val);
                        if i < fields.len() - 1 {
                            self.write(", ");
                        }
                    }
                    self.write(" }");
                } else {
                    self.write(" {");
                    self.newline();
                    self.indent();
                    for (i, (fname, val)) in fields.iter().enumerate() {
                        self.write(fname);
                        self.write(": ");
                        self.fmt_expression(val);
                        if i < fields.len() - 1 {
                            self.write(",");
                        }
                        self.newline();
                    }
                    self.dedent();
                    self.write("}");
                }
            }
            ExpressionKind::TryPropagate(inner) => {
                self.fmt_expression_prec(inner, 8); // Very high precedence for postfix ?
                self.write("?");
            }
        }
    }

    fn fmt_args(&mut self, args: &[Expression], kwargs: &[(String, Expression)]) {
        let total = args.len() + kwargs.len();
        for (i, arg) in args.iter().enumerate() {
            self.fmt_expression(arg);
            if i + 1 < total {
                self.write(", ");
            }
        }
        for (i, (name, val)) in kwargs.iter().enumerate() {
            self.write(name);
            self.write("=");
            self.fmt_expression(val);
            if args.len() + i + 1 < total {
                self.write(", ");
            }
        }
    }

    fn fmt_literal(&mut self, lit: &Literal) {
        match lit {
            Literal::Int64(n) => {
                if *n == i64::MIN {
                    // i64::MIN cannot be parsed as a negated literal (9223372036854775808 overflows i64)
                    self.write("(-9223372036854775807 - 1)");
                } else {
                    self.write(&n.to_string());
                }
            }
            Literal::Float64(f) => {
                if f.is_nan() {
                    self.write("0.0 / 0.0");
                } else if f.is_infinite() {
                    if *f > 0.0 {
                        self.write("1.0 / 0.0");
                    } else {
                        self.write("-1.0 / 0.0");
                    }
                } else {
                    let s = f.to_string();
                    if s.contains('e') || s.contains('E') {
                        // Scientific notation: format with explicit decimal to ensure parsability
                        self.write(&format!("{:e}", f));
                    } else if !s.contains('.') {
                        self.write(&format!("{}.0", s));
                    } else {
                        self.write(&s);
                    }
                }
            }
            Literal::String(s) => {
                self.write(&format!("\"{}\"", escape_string_contents(s)));
            }
            Literal::Bool(b) => self.write(if *b { "true" } else { "false" }),
            Literal::Null => self.write("null"),
            Literal::Array(elems) => {
                // Estimate inline length
                let inline_len: usize = elems.iter().map(|e| self.expr_len(e) + 2).sum();
                if inline_len < self.config.max_width / 2 || elems.is_empty() {
                    self.write("[");
                    for (i, elem) in elems.iter().enumerate() {
                        self.fmt_expression(elem);
                        if i < elems.len() - 1 {
                            self.write(", ");
                        }
                    }
                    self.write("]");
                } else {
                    self.write("[");
                    self.newline();
                    self.indent();
                    for (i, elem) in elems.iter().enumerate() {
                        self.fmt_expression(elem);
                        if i < elems.len() - 1 {
                            self.write(",");
                        }
                        self.newline();
                    }
                    self.dedent();
                    self.write("]");
                }
            }
            Literal::Map(entries) => {
                if entries.is_empty() {
                    // Unreachable from parsed code: the parser never produces an empty
                    // Map literal (it would be parsed as an empty block instead).
                    // Kept as a defensive fallback for programmatically-constructed ASTs.
                    self.write("{}");
                    return;
                }
                let inline_len: usize = entries
                    .iter()
                    .map(|(k, v)| k.len() + 4 + self.expr_len(v))
                    .sum();
                if inline_len < self.config.max_width / 2 {
                    self.write("{");
                    for (i, (key, val)) in entries.iter().enumerate() {
                        self.write(&format!("\"{}\": ", escape_string_contents(key)));
                        self.fmt_expression(val);
                        if i < entries.len() - 1 {
                            self.write(", ");
                        }
                    }
                    self.write("}");
                } else {
                    self.write("{");
                    self.newline();
                    self.indent();
                    for (i, (key, val)) in entries.iter().enumerate() {
                        self.write(&format!("\"{}\": ", escape_string_contents(key)));
                        self.fmt_expression(val);
                        if i < entries.len() - 1 {
                            self.write(",");
                        }
                        self.newline();
                    }
                    self.dedent();
                    self.write("}");
                }
            }
        }
    }

    fn fmt_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Literal(lit) => self.fmt_literal(lit),
            Pattern::Variable(name) => self.write(name),
            Pattern::Wildcard => self.write("_"),
            Pattern::Array(pats) => {
                self.write("[");
                for (i, p) in pats.iter().enumerate() {
                    self.fmt_pattern(p);
                    if i < pats.len() - 1 {
                        self.write(", ");
                    }
                }
                self.write("]");
            }
            Pattern::Map(entries) => {
                self.write("{");
                for (i, (key, pat)) in entries.iter().enumerate() {
                    self.write(key);
                    self.write(": ");
                    self.fmt_pattern(pat);
                    if i < entries.len() - 1 {
                        self.write(", ");
                    }
                }
                self.write("}");
            }
            Pattern::Or(pats) => {
                for (i, p) in pats.iter().enumerate() {
                    self.fmt_pattern(p);
                    if i < pats.len() - 1 {
                        self.write(" | ");
                    }
                }
            }
            Pattern::Rest(Some(name)) => {
                self.write("...");
                self.write(name);
            }
            Pattern::Rest(None) => {
                self.write("...");
            }
            Pattern::EnumPattern {
                enum_name,
                variant,
                bindings,
            } => {
                self.write(enum_name);
                self.write("::");
                self.write(variant);
                if !bindings.is_empty() {
                    self.write("(");
                    for (i, b) in bindings.iter().enumerate() {
                        self.fmt_pattern(b);
                        if i < bindings.len() - 1 {
                            self.write(", ");
                        }
                    }
                    self.write(")");
                }
            }
            Pattern::TypePattern { name, type_name } => {
                self.write(name);
                self.write(": ");
                self.write(type_name);
            }
            Pattern::RangePattern {
                start,
                end,
                inclusive,
            } => {
                self.fmt_expression(start);
                if *inclusive {
                    self.write("..=");
                } else {
                    self.write("..");
                }
                self.fmt_expression(end);
            }
        }
    }

    fn fmt_for_pattern(&mut self, pattern: &ForPattern) {
        match pattern {
            ForPattern::Single(name) => self.write(name),
            ForPattern::ArrayDestructure(elems) => {
                self.write("[");
                for (i, elem) in elems.iter().enumerate() {
                    match elem {
                        DestructureElement::Name(name) => self.write(name),
                        DestructureElement::Rest(name) => {
                            self.write("...");
                            self.write(name);
                        }
                    }
                    if i < elems.len() - 1 {
                        self.write(", ");
                    }
                }
                self.write("]");
            }
            ForPattern::MapDestructure(entries) => {
                self.write("{");
                for (i, (key, alias)) in entries.iter().enumerate() {
                    self.write(key);
                    if let Some(alias) = alias {
                        self.write(": ");
                        self.write(alias);
                    }
                    if i < entries.len() - 1 {
                        self.write(", ");
                    }
                }
                self.write("}");
            }
        }
    }

    fn fmt_destructure_pattern(&mut self, pattern: &DestructurePattern) {
        match pattern {
            DestructurePattern::Array(elems) => {
                self.write("[");
                for (i, elem) in elems.iter().enumerate() {
                    match elem {
                        DestructureElement::Name(name) => self.write(name),
                        DestructureElement::Rest(name) => {
                            self.write("...");
                            self.write(name);
                        }
                    }
                    if i < elems.len() - 1 {
                        self.write(", ");
                    }
                }
                self.write("]");
            }
            DestructurePattern::Map(entries) => {
                self.write("{");
                for (i, (key, alias)) in entries.iter().enumerate() {
                    self.write(key);
                    if let Some(alias) = alias {
                        self.write(": ");
                        self.write(alias);
                    }
                    if i < entries.len() - 1 {
                        self.write(", ");
                    }
                }
                self.write("}");
            }
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

    fn format_source(source: &str) -> String {
        let program = parse_v2(source).expect("parse failed");
        let config = FormatConfig::default();
        format_program(&program, &config)
    }

    /// Round-trip idempotency: format(format(src)) == format(src)
    fn assert_idempotent(source: &str) {
        let first = format_source(source);
        let second = format_source(&first);
        assert_eq!(first, second, "not idempotent:\nfirst:\n{}\nsecond:\n{}", first, second);
    }

    #[test]
    fn test_let_statement() {
        let result = format_source("let   x   =   5 ;");
        assert!(result.contains("let x = 5;"), "got: {}", result);
    }

    #[test]
    fn test_let_mut_statement() {
        let result = format_source("let   mut  x   =   10 ;");
        assert!(result.contains("let mut x = 10;"), "got: {}", result);
    }

    #[test]
    fn test_function_def() {
        let result = format_source("fn add(a, b) { a + b }");
        assert!(result.contains("fn add(a, b)"), "got: {}", result);
        assert_idempotent("fn add(a, b) { a + b }");
    }

    #[test]
    fn test_if_else() {
        let result = format_source("if true { 1 } else { 2 }");
        assert!(result.contains("if true { 1 } else { 2 }"), "got: {}", result);
    }

    #[test]
    fn test_for_loop() {
        let result = format_source("for x in items { output x; }");
        assert!(result.contains("for x in items"), "got: {}", result);
    }

    #[test]
    fn test_while_loop() {
        let result = format_source("while x < 10 { x = x + 1; }");
        assert!(result.contains("while x < 10"), "got: {}", result);
    }

    #[test]
    fn test_match_expression() {
        let source = r#"
let x = 1;
match x {
    1 => "one",
    2 => "two",
    _ => "other",
}
"#;
        let result = format_source(source);
        assert!(result.contains("match x {"), "got: {}", result);
        assert!(result.contains("1 => \"one\""), "got: {}", result);
        assert_idempotent(source);
    }

    #[test]
    fn test_enum_def() {
        let result = format_source("enum Color { Red, Green, Blue }");
        assert!(result.contains("enum Color {"), "got: {}", result);
        assert!(result.contains("Red"), "got: {}", result);
    }

    #[test]
    fn test_struct_def() {
        let result = format_source("struct Point { x: float64, y: float64 }");
        assert!(result.contains("struct Point {"), "got: {}", result);
        assert!(result.contains("x: float64"), "got: {}", result);
    }

    #[test]
    fn test_string_interpolation() {
        let result = format_source(r#"output f"hello {name}";"#);
        assert!(result.contains("f\"hello {name}\""), "got: {}", result);
    }

    #[test]
    fn test_binary_op_precedence() {
        let result = format_source("let x = 1 + 2 * 3;");
        assert!(result.contains("1 + 2 * 3"), "got: {}", result);
    }

    #[test]
    fn test_lambda() {
        let result = format_source("let f = |x| x * 2;");
        assert!(result.contains("|x| x * 2"), "got: {}", result);
    }

    #[test]
    fn test_range_expression() {
        let result = format_source("let r = 1..10;");
        assert!(result.contains("1..10"), "got: {}", result);
    }

    #[test]
    fn test_inclusive_range() {
        let result = format_source("let r = 1..=10;");
        assert!(result.contains("1..=10"), "got: {}", result);
    }

    #[test]
    fn test_list_comprehension() {
        let result = format_source("[x * x for x in 1..=10 if x > 3]");
        assert!(result.contains("[x * x for x in 1..=10 if x > 3]"), "got: {}", result);
    }

    #[test]
    fn test_null_coalesce() {
        let result = format_source("let x = a ?? b;");
        assert!(result.contains("a ?? b"), "got: {}", result);
    }

    #[test]
    fn test_optional_chain() {
        let result = format_source("let x = obj?.field;");
        assert!(result.contains("obj?.field"), "got: {}", result);
    }

    #[test]
    fn test_blank_lines_between_defs() {
        let source = "fn a() { 1 }\nfn b() { 2 }";
        let result = format_source(source);
        assert!(result.contains("\n\nfn b"), "should have blank line between fns: {}", result);
    }

    #[test]
    fn test_idempotent_showcase_snippet() {
        let source = r#"
struct Point {
    x: float64,
    y: float64
}

enum Shape {
    Circle(radius),
    Rectangle(width, height)
}

fn area(shape) {
    match shape {
        Shape::Circle(r) => 3.14159 * r * r,
        Shape::Rectangle(w, h) => w * h,
        _ => 0.0,
    }
}

let origin = Point { x: 0.0, y: 0.0 };
output f"Point: {origin}";
"#;
        assert_idempotent(source);
    }

    #[test]
    fn test_short_block_respects_max_width() {
        let source = "fn f() { some_long_variable_name + another_long_variable_name }";
        // With narrow max_width, the block should be multiline
        let program = parse_v2(source).expect("parse failed");
        let narrow_config = FormatConfig { indent_width: 4, max_width: 40 };
        let result = format_program(&program, &narrow_config);
        assert!(result.contains('\n'), "expected multiline with narrow max_width: {}", result);

        // With wide max_width, the block should inline
        let wide_config = FormatConfig { indent_width: 4, max_width: 200 };
        let result = format_program(&program, &wide_config);
        assert!(result.contains("{ some_long_variable_name + another_long_variable_name }"), "expected inline with wide max_width: {}", result);
    }

    #[test]
    fn test_idempotent_control_flow() {
        let source = r#"
let mut count = 0;
while count < 5 {
    count = count + 1;
}

for n in 1..=10 {
    if n % 2 == 0 {
        continue;
    }
    output n;
}
"#;
        assert_idempotent(source);
    }

    #[test]
    fn test_optional_chain_method_call() {
        let source = "let x = null;\nlet y = x?.foo();\nlet z = x?.bar.baz();";
        let formatted = format_source(source);
        assert!(formatted.contains("x?.foo()"), "expected x?.foo(), got: {}", formatted);
        assert!(formatted.contains("x?.bar.baz()"), "expected x?.bar.baz(), got: {}", formatted);
        assert_idempotent(source);
    }

    // =====================================================================
    // Comprehensive idempotency test suite
    //
    // For every test case, we verify:
    //   format(parse(source)) can be re-parsed, AND
    //   format(parse(format(parse(source)))) == format(parse(source))
    //
    // This is the core idempotency invariant for the formatter.
    // =====================================================================

    /// Helper: assert that a batch of labeled test cases are all idempotent.
    /// Collects all failures before panicking so you see everything at once.
    fn assert_batch_idempotent(cases: &[(&str, &str)]) {
        let mut failures = Vec::new();
        for (label, source) in cases {
            let parsed = match parse_v2(source) {
                Ok(p) => p,
                Err(e) => {
                    failures.push(format!("{}: parse1 failed: {}", label, e));
                    continue;
                }
            };
            let config = FormatConfig::default();
            let first = format_program(&parsed, &config);

            let parsed2 = match parse_v2(&first) {
                Ok(p) => p,
                Err(e) => {
                    failures.push(format!(
                        "{}: parse2 failed on formatted output: {}\nformatted:\n{}",
                        label, e, first
                    ));
                    continue;
                }
            };
            let second = format_program(&parsed2, &config);

            if first != second {
                failures.push(format!(
                    "{}: NOT IDEMPOTENT\n  first:  {:?}\n  second: {:?}",
                    label, first, second
                ));
            }
        }

        if !failures.is_empty() {
            panic!(
                "{} idempotency failure(s):\n{}",
                failures.len(),
                failures.join("\n\n")
            );
        }
    }

    // -----------------------------------------------------------------
    // Edge case: empty program
    // -----------------------------------------------------------------

    #[test]
    fn test_idempotent_empty_program() {
        let result = format_source("");
        assert_eq!(result, "", "empty program should produce empty string");
        // Second pass
        let program2 = parse_v2(&result).expect("parse empty");
        let result2 = format_program(&program2, &FormatConfig::default());
        assert_eq!(result, result2);
    }

    // -----------------------------------------------------------------
    // Edge case: program with only comments (comments are lost)
    // -----------------------------------------------------------------

    #[test]
    fn test_idempotent_comments_only() {
        let source = "// just a comment\n// another comment\n";
        let result = format_source(source);
        // Comments are stripped by the parser, so output is empty
        assert_eq!(result, "", "comments-only should produce empty string");
        let result2 = format_source(&result);
        assert_eq!(result, result2);
    }

    // -----------------------------------------------------------------
    // Edge case: very long lines
    // -----------------------------------------------------------------

    #[test]
    fn test_idempotent_very_long_line() {
        // A line that exceeds max_width — should still be idempotent
        let long_name = "x".repeat(200);
        let source = format!("let {} = 1;", long_name);
        assert_idempotent(&source);
    }

    // -----------------------------------------------------------------
    // Edge case: deeply nested expressions
    // -----------------------------------------------------------------

    #[test]
    fn test_idempotent_deeply_nested() {
        // Nested if-else (5 levels)
        let source = "if a { if b { if c { if d { if e { 1 } else { 2 } } else { 3 } } else { 4 } } else { 5 } }";
        assert_idempotent(source);

        // Nested binary ops
        let source = "let x = ((((a + b) * c) - d) / e) % f;";
        assert_idempotent(source);

        // Nested method chains
        let source = "let x = a.b().c().d().e().f();";
        assert_idempotent(source);

        // Nested blocks
        let source = "let x = { let a = { let b = { let c = 1; c }; b }; a };";
        assert_idempotent(source);
    }

    // -----------------------------------------------------------------
    // All statement types
    // -----------------------------------------------------------------

    #[test]
    fn test_idempotent_all_statement_types() {
        assert_batch_idempotent(&[
            // Import
            ("import", "import \"plugin\";"),
            // Let
            ("let", "let x = 5;"),
            ("let_typed", "let x: int64 = 42;"),
            // LetMut
            ("let_mut", "let mut x = 10;"),
            ("let_mut_typed", "let mut x: int64 = 0;"),
            // LetDestructure (array)
            ("let_destr_array", "let [a, b] = pair;"),
            ("let_destr_rest", "let [first, ...rest] = items;"),
            ("let_mut_destr", "let mut [a, b] = pair;"),
            // LetDestructure (map)
            ("let_destr_map", "let {x, y} = point;"),
            ("let_destr_map_alias", "let {x: ax, y: ay} = point;"),
            // Assignment
            ("assign", "x = 42;"),
            // CompoundAssign
            ("compound_add", "x += 1;"),
            ("compound_sub", "x -= 1;"),
            ("compound_mul", "x *= 2;"),
            ("compound_div", "x /= 2;"),
            ("compound_mod", "x %= 3;"),
            // ForLoop
            ("for_simple", "for x in items {}"),
            ("for_array_destr", "for [a, b] in pairs {}"),
            ("for_map_destr", "for {k, v} in entries {}"),
            ("for_with_body", "for x in 0..10 {\n    output x;\n}"),
            // WhileLoop
            ("while", "while x < 10 {\n    x += 1;\n}"),
            // Output
            ("output", "output 42;"),
            // ExprStatement
            ("expr_stmt_call", "foo();"),
            ("expr_stmt_if", "if true { foo(); }"),
            ("expr_stmt_match", "match x {\n    _ => 0\n}"),
            ("expr_stmt_loop", "loop {\n    break;\n}"),
            ("expr_stmt_block", "{\n    foo();\n}"),
            ("expr_stmt_try", "try {\n    risky();\n} catch e {\n    handle(e);\n}"),
            // FunctionDef
            ("fn_simple", "fn f() { 1 }"),
            ("fn_params", "fn f(x, y) { x + y }"),
            ("fn_typed", "fn f(x: int64) -> int64 { x }"),
            ("fn_default", "fn f(x, y = 10) { x + y }"),
            ("fn_rest", "fn f(x, ...args) { x }"),
            ("fn_multiline", "fn f(x) {\n    let y = x + 1;\n    y * 2\n}"),
            // AsyncFunctionDef
            ("async_fn", "async fn fetch() { 42 }"),
            // Break
            ("break", "loop {\n    break;\n}"),
            ("break_val", "loop {\n    break 42;\n}"),
            // Continue
            ("continue", "for x in items {\n    continue;\n}"),
            // Return
            ("return", "fn f() { return; }"),
            ("return_val", "fn f() { return 42; }"),
            // TryCatch (statement)
            ("try_catch", "try {\n    risky();\n} catch e {\n    handle(e);\n}"),
            ("try_catch_no_var", "try {\n    risky();\n} catch {\n    handle();\n}"),
            ("try_catch_finally", "try {\n    risky();\n} catch e {\n    handle(e);\n} finally {\n    cleanup();\n}"),
            // Throw
            ("throw", "throw \"error\";"),
            // ConstDef
            ("const", "const PI = 3.14;"),
            ("const_typed", "const PI: float64 = 3.14;"),
            // TypeAlias
            ("type_alias", "type Num = int64;"),
            // ModuleDef
            ("module", "mod math {\n    fn add(a, b) { a + b }\n}"),
            // Use
            ("use_simple", "use std::io;"),
            ("use_glob", "use std::io::*;"),
            ("use_alias", "use std::io as sio;"),
            // TestDef
            ("test", "test \"my test\" {\n    assert(true);\n}"),
            // EnumDef
            ("enum_simple", "enum Color {\n    Red,\n    Green,\n    Blue\n}"),
            ("enum_fields", "enum Shape {\n    Circle(radius),\n    Rect(w, h)\n}"),
            // StructDef
            ("struct", "struct Point {\n    x: float64,\n    y: float64\n}"),
        ]);
    }

    // -----------------------------------------------------------------
    // All expression types
    // -----------------------------------------------------------------

    #[test]
    fn test_idempotent_all_expression_types() {
        assert_batch_idempotent(&[
            // Literal: Int64
            ("int", "let x = 42;"),
            ("int_neg", "let x = -42;"),
            ("int_zero", "let x = 0;"),
            // Literal: Float64
            ("float", "let x = 3.14;"),
            ("float_zero", "let x = 0.0;"),
            ("float_integer_like", "let x = 1.0;"),
            ("float_neg", "let x = -1.5;"),
            ("float_large", "let x = 100000000000000000000.0;"),
            ("float_small", "let x = 0.000000001;"),
            // Literal: String
            ("string", r#"let s = "hello";"#),
            ("string_empty", r#"let s = "";"#),
            ("string_escapes", r#"let s = "tab\there\nnewline\r\0null";"#),
            ("string_quotes", r#"let s = "say \"hello\"";"#),
            ("string_backslash", r#"let s = "back\\slash";"#),
            // Literal: Bool
            ("bool_true", "let x = true;"),
            ("bool_false", "let x = false;"),
            // Literal: Null
            ("null", "let x = null;"),
            // Literal: Array
            ("array_empty", "let a = [];"),
            ("array", "let a = [1, 2, 3];"),
            // Literal: Map
            ("map", r#"let m = {"a": 1, "b": 2};"#),
            // Variable
            ("var", "let x = y;"),
            // BinaryOp (all operators)
            ("add", "let x = a + b;"),
            ("sub", "let x = a - b;"),
            ("mul", "let x = a * b;"),
            ("div", "let x = a / b;"),
            ("modulo", "let x = a % b;"),
            ("eq", "let x = a == b;"),
            ("neq", "let x = a != b;"),
            ("gt", "let x = a > b;"),
            ("lt", "let x = a < b;"),
            ("gte", "let x = a >= b;"),
            ("lte", "let x = a <= b;"),
            ("and", "let x = a && b;"),
            ("or", "let x = a || b;"),
            // UnaryOp
            ("neg", "let x = -y;"),
            ("not", "let x = !y;"),
            // Call
            ("call", "foo();"),
            ("call_args", "foo(1, 2, 3);"),
            ("call_kwargs", "let x = f(1, name=2);"),
            ("call_multi_kwargs", "let x = f(1, 2, mode=\"fast\", verbose=true);"),
            // MethodCall
            ("method", "let x = arr.push(5);"),
            ("method_chain", "let x = arr.map(|x| x * 2).filter(|x| x > 3);"),
            ("method_kwargs", "let x = obj.f(1, name=2);"),
            // Pipe
            ("pipe", "let x = a\n    |> b(_);"),
            ("pipe_chain", "let x = a\n    |> b(_)\n    |> c(_);"),
            // IfElse
            ("if_only", "if true { 1 }"),
            ("if_else", "if true { 1 } else { 2 }"),
            ("if_else_if", "if a { 1 } else if b { 2 } else { 3 }"),
            // Block
            ("block", "{\n    let a = 1;\n    a + 2\n}"),
            // Index
            ("index", "let x = arr[0];"),
            // FieldAccess
            ("field", "let x = obj.field;"),
            // Placeholder
            ("placeholder_in_pipe", "let x = a\n    |> f(_);"),
            // Range
            ("range", "let r = 0..10;"),
            ("range_inclusive", "let r = 0..=10;"),
            // Await
            ("await", "let x = await fetch();"),
            // Spawn
            ("spawn", "let x = spawn task();"),
            // Lambda
            ("lambda", "let f = |x| x * 2;"),
            ("lambda_typed", "let f = |x: int64| x * 2;"),
            ("lambda_default", "let f = |x, y = 10| x + y;"),
            ("lambda_no_params", "let f = || 42;"),
            ("lambda_block", "let f = |x| {\n    let y = x + 1;\n    y * 2\n};"),
            ("lambda_nested", "let f = |x| (|y| x + y);"),
            // Match
            ("match", "match x {\n    1 => true,\n    _ => false\n}"),
            ("match_guard", "match x {\n    n if n > 0 => true,\n    _ => false\n}"),
            ("match_or", "match x {\n    1 | 2 | 3 => true,\n    _ => false\n}"),
            ("match_enum", "match x {\n    Result::Ok(v) => v,\n    Result::Err(e) => 0\n}"),
            ("match_type", "match x {\n    n: int64 => n + 1,\n    _ => 0\n}"),
            ("match_range", "match x {\n    0..10 => true,\n    _ => false\n}"),
            ("match_range_incl", "match x {\n    0..=10 => true,\n    _ => false\n}"),
            ("match_array", "match x {\n    [first, ...rest] => first,\n    _ => 0\n}"),
            ("match_block_body", "match x {\n    1 => {\n        let y = 2;\n        y + 1\n    },\n    _ => 0\n}"),
            // StringInterpolation
            ("fstring", "let s = f\"hello {name}\";"),
            ("fstring_multi", "let s = f\"a={a} b={b} c={c}\";"),
            ("fstring_escaped", "let s = f\"braces \\{here\\}\";"),
            // NullCoalesce
            ("null_coalesce", "let x = a ?? b;"),
            ("null_coalesce_chain", "let x = a ?? b ?? c;"),
            // OptionalChain
            ("opt_chain", "let x = obj?.field;"),
            ("opt_chain_method", "let x = obj?.method();"),
            ("opt_chain_index", "let x = arr?[0];"),
            // Spread
            ("spread", "let arr = [...other, 1, 2];"),
            // Loop
            ("loop", "loop {\n    break 42;\n}"),
            // TryCatchExpr
            ("try_catch_expr", "let x = try { risky() } catch e { 0 };"),
            ("try_catch_expr_finally", "let x = try {\n    risky();\n    1\n} catch e {\n    0\n} finally {\n    cleanup();\n};"),
            // ListComprehension
            ("list_comp", "[x * 2 for x in 0..10]"),
            ("list_comp_if", "[x * 2 for x in 0..10 if x > 3]"),
            // MapComprehension
            ("map_comp", r#"{"k": v for k in keys}"#),
            // EnumConstruct
            ("enum_construct", "let c = Color::Red;"),
            ("enum_construct_args", "let r = Result::Ok(42);"),
            // StructConstruct
            ("struct_construct", "let p = Point { x: 1.0, y: 2.0 };"),
            ("struct_empty", "let p = Empty {};"),
            // TryPropagate
            ("try_prop", "let x = foo()?;"),
        ]);
    }

    // -----------------------------------------------------------------
    // Precedence and parenthesization
    // -----------------------------------------------------------------

    #[test]
    fn test_idempotent_precedence() {
        assert_batch_idempotent(&[
            // Parentheses that should be preserved
            ("parens_needed", "let x = (1 + 2) * 3;"),
            ("parens_or_and", "let x = (a || b) && c;"),
            // Parentheses that should be dropped (not needed)
            ("no_parens_mul_add", "let x = 1 + 2 * 3;"),
            ("no_parens_and_or", "let x = a && b || c && d;"),
            // Left-associativity
            ("left_assoc_add", "let x = 1 + 2 + 3;"),
            ("left_assoc_mul", "let x = 1 * 2 * 3;"),
            ("left_assoc_sub", "let x = 1 - 2 - 3;"),
            // Mixed precedence
            ("mixed_1", "let x = a + b * c - d / e;"),
            ("mixed_2", "let x = a == b && c != d;"),
            ("mixed_3", "let x = a > b && c <= d;"),
            // Negation of complex expression
            ("neg_paren", "let x = -(a + b);"),
            ("neg_neg", "let x = -(-y);"),
            ("not_not", "let x = !!y;"),
            // Unary in postfix context
            ("neg_method", "let x = (-a).abs();"),
            ("not_field", "let x = (!a).val;"),
            // Pipe/Range/NullCoalesce/Lambda parenthesized in postfix
            ("lambda_in_call", "let x = items.map((|x| x * 2));"),
            // Pipe inside NullCoalesce (round 132 fix)
            ("pipe_in_null_coalesce", "let x = a ?? (b\n    |> c);"),
            // NullCoalesce inside Range (round 132 fix)
            ("null_coalesce_in_range", "let x = (a ?? b)..(a ?? c);"),
            // Lambda inside Range (round 132 fix)
            ("lambda_in_range", "let x = (|x| x)..(|y| y);"),
            // Range inside NullCoalesce — slightly over-parenthesized but correct
            ("range_in_null_coalesce", "let x = (0..10) ?? (1..5);"),
        ]);
    }

    #[test]
    fn test_optional_index_formatting() {
        // Round 132: optional index expr?[idx] should not become expr?.[idx]
        let source = "let x = arr?[0];";
        let result = format_source(source);
        assert!(result.contains("arr?[0]"), "got: {}", result);
        assert!(!result.contains("?."), "should not contain ?. for optional index, got: {}", result);
        assert_idempotent(source);
    }

    // -----------------------------------------------------------------
    // Whitespace and blank lines
    // -----------------------------------------------------------------

    #[test]
    fn test_idempotent_blank_lines() {
        assert_batch_idempotent(&[
            // Blank lines between function defs
            ("fn_fn", "fn a() { 1 }\n\nfn b() { 2 }"),
            // Blank line between const and fn
            ("const_fn", "const X = 1;\n\nfn f() { X }"),
            // Blank line between enum and struct
            ("enum_struct", "enum A {\n    X\n}\n\nstruct B {\n    y: int64\n}"),
            // Let then fn
            ("let_fn", "let x = 1;\n\nfn f() { x }"),
            // Multiple fns
            ("three_fns", "fn a() { 1 }\n\nfn b() { 2 }\n\nfn c() { 3 }"),
        ]);
    }

    // -----------------------------------------------------------------
    // Multiline formatting (arrays, maps, structs that exceed width)
    // -----------------------------------------------------------------

    #[test]
    fn test_idempotent_multiline_formatting() {
        assert_batch_idempotent(&[
            ("long_array", "let arr = [\"aaaaaaaaaaaaaa\", \"bbbbbbbbbbbbbb\", \"cccccccccccccc\", \"dddddddddddddd\", \"eeeeeeeeeeeeee\"];"),
            ("long_map", r#"let m = {"aaaaaaaaaaaaaa": 1, "bbbbbbbbbbbbbb": 2, "cccccccccccccc": 3, "dddddddddddddd": 4};"#),
            ("long_struct", "let p = Point { field_one: some_long_name, field_two: another_long_name, field_three: yet_another };"),
            ("long_enum_args", "let r = Result::Ok(some_very_long_variable_name_that_makes_line_long);"),
        ]);
    }

    // -----------------------------------------------------------------
    // NaN and Infinity (special float formatting)
    // -----------------------------------------------------------------

    #[test]
    fn test_idempotent_special_floats() {
        // NaN: formatted as 0.0 / 0.0 (a binary expression, not a literal)
        assert_idempotent("let x = 0.0 / 0.0;");
        // Positive infinity
        assert_idempotent("let x = 1.0 / 0.0;");
        // Negative infinity
        assert_idempotent("let x = -1.0 / 0.0;");
    }

    // -----------------------------------------------------------------
    // Complete realistic program
    // -----------------------------------------------------------------

    #[test]
    fn test_idempotent_realistic_program() {
        let source = r#"
use std::math;

const MAX_SIZE: int64 = 100;

type Distance = float64;

enum Shape {
    Circle(radius),
    Rectangle(width, height),
    Triangle(a, b, c)
}

struct Point {
    x: float64,
    y: float64
}

fn distance(p1, p2) {
    let dx = p1.x - p2.x;
    let dy = p1.y - p2.y;
    sqrt(dx * dx + dy * dy)
}

fn area(shape) {
    match shape {
        Shape::Circle(r) => 3.14159 * r * r,
        Shape::Rectangle(w, h) => w * h,
        Shape::Triangle(a, b, c) => {
            let s = (a + b + c) / 2.0;
            sqrt(s * (s - a) * (s - b) * (s - c))
        },
        _ => 0.0
    }
}

async fn fetch_data() {
    let result = await get_url("https://example.com");
    result ?? "default"
}

fn process(items) {
    let filtered = [x * 2 for x in items if x > 0];
    for [idx, val] in filtered.enumerate() {
        if val > MAX_SIZE {
            output f"Item {idx}: too large ({val})";
            continue;
        }
        output f"Item {idx}: {val}";
    }
    filtered
}

test "area calculation" {
    let c = Shape::Circle(5.0);
    let a = area(c);
    assert(a > 78.0);
    assert(a < 79.0);
}

fn main() {
    let origin = Point { x: 0.0, y: 0.0 };
    let p = Point { x: 3.0, y: 4.0 };
    let d = distance(origin, p);
    output f"Distance: {d}";

    let mut count = 0;
    while count < 10 {
        count += 1;
    }

    let result = try {
        let data = fetch_data()?;
        process(data)
    } catch e {
        output f"Error: {e}";
        []
    };
    output result;
}
"#;
        assert_idempotent(source);
    }

    // -----------------------------------------------------------------
    // Kwargs formatting (regression test for `:` vs `=` bug)
    // -----------------------------------------------------------------

    #[test]
    fn test_kwargs_use_equals_sign() {
        let source = r#"let x = f(1, mode="fast");"#;
        let formatted = format_source(source);
        assert!(
            formatted.contains("mode=\"fast\""),
            "kwargs should use = not :, got: {}",
            formatted
        );
        assert_idempotent(source);

        // Method call kwargs
        let source2 = r#"let x = obj.f(1, mode="fast");"#;
        let formatted2 = format_source(source2);
        assert!(
            formatted2.contains("mode=\"fast\""),
            "method kwargs should use =, got: {}",
            formatted2
        );
        assert_idempotent(source2);
    }

    // -----------------------------------------------------------------
    // Blank lines between definitions inside blocks
    // -----------------------------------------------------------------

    #[test]
    fn test_blank_lines_in_module_body() {
        let source = "mod utils {\n    fn a() { 1 }\n    fn b() { 2 }\n}";
        let result = format_source(source);
        assert!(
            result.contains("fn a() { 1 }\n\n    fn b() { 2 }"),
            "should have blank line between fns in module body: {}",
            result
        );
        assert_idempotent(&result);
    }

    #[test]
    fn test_blank_lines_in_block_mixed_defs_and_stmts() {
        let source = "mod m {\n    let x = 1;\n    fn f() { x }\n    let y = 2;\n}";
        let result = format_source(source);
        // Should have blank line before fn (def after non-def) and after fn (non-def after def)
        assert!(
            result.contains("let x = 1;\n\n    fn f() { x }\n\n    let y = 2;"),
            "should have blank lines around fn in block: {}",
            result
        );
        assert_idempotent(&result);
    }

    #[test]
    fn test_blank_line_before_tail_expr_after_def() {
        let source = "fn outer() {\n    fn inner() { 1 }\n    inner()\n}";
        let result = format_source(source);
        assert!(
            result.contains("fn inner() { 1 }\n\n    inner()"),
            "should have blank line between fn def and tail expr: {}",
            result
        );
        assert_idempotent(&result);
    }

    // -----------------------------------------------------------------
    // Edge cases: empty/single/nested arrays
    // -----------------------------------------------------------------

    #[test]
    fn test_empty_array() {
        let result = format_source("let a = [];");
        assert!(result.contains("let a = [];"), "got: {}", result);
        assert_idempotent("let a = [];");
    }

    #[test]
    fn test_single_element_array() {
        let result = format_source("let a = [1];");
        assert!(result.contains("let a = [1];"), "got: {}", result);
        assert_idempotent("let a = [1];");
    }

    #[test]
    fn test_nested_arrays() {
        let result = format_source("let a = [[1, 2], [3, 4]];");
        assert!(result.contains("[[1, 2], [3, 4]]"), "got: {}", result);
        assert_idempotent("let a = [[1, 2], [3, 4]];");
    }

    // -----------------------------------------------------------------
    // TryPropagate
    // -----------------------------------------------------------------

    #[test]
    fn test_try_propagate_formatting() {
        assert_batch_idempotent(&[
            ("try_prop_call", "let x = foo()?;"),
            ("try_prop_method", "let x = obj.method()?;"),
            ("try_prop_chain", "let x = foo()?.bar();"),
            ("try_prop_field", "let x = foo()?.field;"),
        ]);
    }

    // -----------------------------------------------------------------
    // Inclusive ranges
    // -----------------------------------------------------------------

    #[test]
    fn test_inclusive_range_formatting() {
        assert_batch_idempotent(&[
            ("incl_range", "let r = 0..=10;"),
            ("incl_range_for", "for x in 0..=10 {}"),
            ("incl_range_match", "match x {\n    0..=10 => true,\n    _ => false\n}"),
        ]);
    }

    // -----------------------------------------------------------------
    // Default and rest parameters
    // -----------------------------------------------------------------

    #[test]
    fn test_default_and_rest_params() {
        assert_batch_idempotent(&[
            ("default_param", "fn f(x, y = 10) { x + y }"),
            ("default_typed", "fn f(x: int64, y: int64 = 10) -> int64 { x + y }"),
            ("rest_param", "fn f(x, ...args) { x }"),
            ("rest_only", "fn f(...args) { args }"),
            ("default_lambda", "let f = |x, y = 10| x + y;"),
            ("rest_lambda", "let f = |x, ...rest| x;"),
        ]);
    }

    // -----------------------------------------------------------------
    // Async function definitions
    // -----------------------------------------------------------------

    #[test]
    fn test_async_fn_formatting() {
        assert_batch_idempotent(&[
            ("async_fn_simple", "async fn fetch() { 42 }"),
            ("async_fn_typed", "async fn fetch(url: string) -> string { url }"),
            ("async_fn_body", "async fn fetch() {\n    let x = await get();\n    x\n}"),
        ]);
    }

    // -----------------------------------------------------------------
    // Test definitions
    // -----------------------------------------------------------------

    #[test]
    fn test_test_def_formatting() {
        assert_batch_idempotent(&[
            ("test_simple", "test \"basic\" {\n    assert(true);\n}"),
            ("test_complex", "test \"complex test\" {\n    let x = 42;\n    assert(x == 42);\n}"),
        ]);
    }

    // -----------------------------------------------------------------
    // Module definitions with complex bodies
    // -----------------------------------------------------------------

    #[test]
    fn test_module_def_formatting() {
        let source = r#"
mod math {
    const PI = 3.14159;

    fn add(a, b) { a + b }

    fn multiply(a, b) { a * b }
}
"#;
        assert_idempotent(source);
    }

    // -----------------------------------------------------------------
    // Keyword arguments
    // -----------------------------------------------------------------

    #[test]
    fn test_kwargs_formatting() {
        assert_batch_idempotent(&[
            ("kwargs_only", "let x = f(name=1);"),
            ("kwargs_multi", "let x = f(a=1, b=2, c=3);"),
            ("kwargs_mixed", "let x = f(1, 2, name=3);"),
            ("method_kwargs", "let x = obj.f(1, name=2);"),
        ]);
    }

    // -----------------------------------------------------------------
    // Spread operator
    // -----------------------------------------------------------------

    #[test]
    fn test_spread_formatting() {
        assert_batch_idempotent(&[
            ("spread_array", "let a = [...other, 1, 2];"),
            ("spread_multi", "let a = [...x, ...y, ...z];"),
        ]);
    }

    // -----------------------------------------------------------------
    // Comprehensions with destructuring
    // -----------------------------------------------------------------

    #[test]
    fn test_comprehension_destructure() {
        assert_batch_idempotent(&[
            ("list_comp_destr", "[a + b for [a, b] in pairs]"),
            ("list_comp_map_destr", "[v for {k, v} in entries]"),
        ]);
    }

    // -----------------------------------------------------------------
    // Enum/struct construction edge cases
    // -----------------------------------------------------------------

    #[test]
    fn test_enum_struct_construct() {
        assert_batch_idempotent(&[
            ("enum_no_args", "let x = Color::Red;"),
            ("enum_one_arg", "let x = Result::Ok(42);"),
            ("enum_multi_args", "let x = Shape::Rect(10, 20);"),
            ("struct_empty", "let x = Empty {};"),
            ("struct_single", "let x = Point { x: 1.0 };"),
            ("struct_multi", "let x = Point { x: 1.0, y: 2.0 };"),
        ]);
    }

    // -----------------------------------------------------------------
    // Loop expression
    // -----------------------------------------------------------------

    #[test]
    fn test_loop_expression() {
        assert_batch_idempotent(&[
            ("loop_break", "loop {\n    break;\n}"),
            ("loop_break_val", "let x = loop {\n    break 42;\n};"),
            ("loop_complex", "let x = loop {\n    let v = next();\n    if v > 10 {\n        break v;\n    }\n};"),
        ]);
    }

    // -----------------------------------------------------------------
    // Await and spawn in various positions
    // -----------------------------------------------------------------

    #[test]
    fn test_await_spawn_positions() {
        assert_batch_idempotent(&[
            ("await_call", "let x = await fetch();"),
            ("await_method", "let x = (await fetch()).body;"),
            ("spawn_call", "let x = spawn task();"),
            ("spawn_block", "let x = spawn {\n    let y = 1;\n    y + 2\n};"),
        ]);
    }
}
