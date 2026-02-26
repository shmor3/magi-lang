//! AST pretty-printer / code formatter for the MAGI language.
//!
//! Takes a parsed `Program` and produces formatted source code.
//! Comments are lost (the parser discards them), which is acceptable for v1.

use crate::syntax::ast::*;

/// Re-escape a string's contents so that control characters are represented
/// as their escape sequences (e.g., newline → `\n`). This ensures the
/// formatter produces valid, parseable string literals.
fn escape_string_contents(s: &str) -> String {
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
                // Escape other control chars as \xHH
                for b in c.to_string().bytes() {
                    out.push_str(&format!("\\x{:02x}", b));
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
}

impl<'a> Formatter<'a> {
    fn new(config: &'a FormatConfig) -> Self {
        Self {
            config,
            output: String::new(),
            indent: 0,
            at_line_start: true,
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

    /// Estimate the length of an expression when formatted on a single line.
    fn expr_len(&self, expr: &Expression) -> usize {
        let mut f = Formatter::new(self.config);
        f.fmt_expression(expr);
        f.output.len()
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
            let is_def = matches!(
                &stmt.kind,
                StatementKind::FunctionDef(_)
                    | StatementKind::AsyncFunctionDef(_)
                    | StatementKind::EnumDef { .. }
                    | StatementKind::StructDef { .. }
            );

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
                self.write(&format!("import \"{}\";", path));
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
                self.write(&format!(" {}= ", op));
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
                self.write(";");
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
                self.write(&format!("test \"{}\" ", name));
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
        if self.is_short_block(block) && block.statements.is_empty() {
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

        for stmt in &block.statements {
            self.fmt_statement(stmt);
            self.newline();
        }

        if let Some(tail) = &block.tail_expr {
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
                self.write(&op.to_string());
                self.fmt_expression_prec(operand, 7); // High precedence for unary
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
                self.fmt_expression_prec(object, 9);
                self.write(".");
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
                    self.write(" else ");
                    self.fmt_block(eb);
                }
            }
            ExpressionKind::Block(block) => {
                self.fmt_block(block);
            }
            ExpressionKind::Index { object, index } => {
                self.fmt_expression_prec(object, 9);
                self.write("[");
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
                self.fmt_expression(start);
                if *inclusive {
                    self.write("..=");
                } else {
                    self.write("..");
                }
                self.fmt_expression(end);
                if needs_parens {
                    self.write(")");
                }
            }
            ExpressionKind::Await(inner) => {
                self.write("await ");
                self.fmt_expression(inner);
            }
            ExpressionKind::Spawn(inner) => {
                self.write("spawn ");
                self.fmt_expression(inner);
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
                            // Escape special characters in the literal.
                            // Braces must be doubled so they round-trip through the parser.
                            let escaped = escape_string_contents(s);
                            let escaped = escaped.replace('{', "{{").replace('}', "}}");
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
                self.fmt_expression(left);
                self.write(" ?? ");
                self.fmt_expression(right);
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
            } => {
                self.write("try ");
                self.fmt_block(try_block);
                self.write(" catch ");
                if let Some(var) = catch_var {
                    self.write(var);
                    self.write(" ");
                }
                self.fmt_block(catch_block);
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
            if i < total - 1 {
                self.write(", ");
            }
        }
        for (i, (name, val)) in kwargs.iter().enumerate() {
            self.write(name);
            self.write(": ");
            self.fmt_expression(val);
            if i < kwargs.len() - 1 {
                self.write(", ");
            }
        }
    }

    fn fmt_literal(&mut self, lit: &Literal) {
        match lit {
            Literal::Int64(n) => self.write(&n.to_string()),
            Literal::Float64(f) => {
                let s = f.to_string();
                // Ensure there's a decimal point for readability
                if !s.contains('.') {
                    self.write(&format!("{}.0", s));
                } else {
                    self.write(&s);
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
                        let escaped_key = key.replace('\\', "\\\\").replace('"', "\\\"");
                        self.write(&format!("\"{}\": ", escaped_key));
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
                        let escaped_key = key.replace('\\', "\\\\").replace('"', "\\\"");
                        self.write(&format!("\"{}\": ", escaped_key));
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
}
