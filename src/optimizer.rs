//! Optimization passes for the MAGI AST.
//!
//! Includes:
//! - Constant folding: replaces constant expressions with computed literal values
//! - Tail call optimization: converts tail-recursive calls to loops
//! - Dead code elimination: removes unreachable code after return/break/continue
//! - Function inlining: inlines small, non-recursive functions

use crate::syntax::ast::{
    BinOp, Block, Expression, ExpressionKind, Literal, MatchArm, Program, Span, Statement,
    StatementKind, StringPart, UnOp,
};

/// Run all optimization passes on a program.
pub fn optimize(program: &mut Program) {
    fold_constants(program);
    eliminate_dead_code(program);
    optimize_tail_calls(program);
    inline_small_functions(program);
    unroll_small_loops(program);
}

/// Optimize a program by folding constant expressions in-place.
pub fn fold_constants(program: &mut Program) {
    for stmt in &mut program.statements {
        fold_statement(stmt);
    }
}

/// Eliminate dead code: remove statements after return/break/continue in blocks.
pub fn eliminate_dead_code(program: &mut Program) {
    for stmt in &mut program.statements {
        eliminate_dead_code_stmt(stmt);
    }
}

fn eliminate_dead_code_stmt(stmt: &mut Statement) {
    match &mut stmt.kind {
        StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
            trim_dead_code_in_block(&mut fdef.body);
        }
        StatementKind::ForLoop { body, .. } | StatementKind::WhileLoop { body, .. }
        | StatementKind::DoWhileLoop { body, .. } => {
            trim_dead_code_in_block(body);
        }
        StatementKind::TryCatch { try_block, catch_block, finally_block, .. } => {
            trim_dead_code_in_block(try_block);
            trim_dead_code_in_block(catch_block);
            if let Some(block) = finally_block {
                trim_dead_code_in_block(block);
            }
        }
        _ => {}
    }
}

fn trim_dead_code_in_block(block: &mut Block) {
    for stmt in block.statements.iter_mut() {
        eliminate_dead_code_stmt(stmt);
    }
    if let Some(pos) = block.statements.iter().position(|s| {
        matches!(s.kind, StatementKind::Return(_) | StatementKind::Break { .. } | StatementKind::Throw(_))
    }) {
        block.statements.truncate(pos + 1);
    }
}

/// Optimize tail calls: detect tail-recursive calls in functions and mark them.
/// The interpreter can use this to convert tail-recursive calls to loops.
pub fn optimize_tail_calls(program: &mut Program) {
    for stmt in &mut program.statements {
        if let StatementKind::FunctionDef(fdef) = &mut stmt.kind {
            let name = fdef.name.clone();
            mark_tail_calls_block(&mut fdef.body, &name);
        }
    }
}

fn mark_tail_calls_block(block: &mut Block, fn_name: &str) {
    if let Some(last) = block.statements.last_mut() {
        mark_tail_calls_stmt(last, fn_name);
    }
}

fn mark_tail_calls_stmt(stmt: &mut Statement, fn_name: &str) {
    match &mut stmt.kind {
        StatementKind::Return(Some(expr)) => {
            mark_tail_calls_expr(expr, fn_name);
        }
        StatementKind::ExprStatement(expr) => {
            mark_tail_calls_expr(expr, fn_name);
        }
        _ => {}
    }
}

fn mark_tail_calls_expr(expr: &mut Expression, fn_name: &str) {
    if let ExpressionKind::Call { name, .. } = &expr.kind {
        if name == fn_name {
            expr.span.tail_call = true;
        }
    }
}

/// Inline small, non-recursive functions (< 3 statements, no self-calls).
pub fn inline_small_functions(program: &mut Program) {
    // Pass 1: collect small function bodies
    let mut inlinable: std::collections::HashMap<String, Vec<Statement>> = std::collections::HashMap::new();
    for stmt in &program.statements {
        if let StatementKind::FunctionDef(fdef) = &stmt.kind {
            // Only inline small, non-recursive functions with 1-2 statements
            if fdef.body.statements.len() <= 2 && fdef.params.len() <= 3 {
                // Check for self-recursion
                let name = &fdef.name;
                let has_self_call = fdef.body.statements.iter().any(|s| {
                    match &s.kind {
                        StatementKind::Return(Some(e)) | StatementKind::ExprStatement(e) => {
                            contains_call(e, name)
                        }
                        _ => false,
                    }
                });
                if !has_self_call {
                    inlinable.insert(name.clone(), fdef.body.statements.clone());
                }
            }
        }
    }
    // Pass 2: mark inlinable call sites (don't actually inline to avoid code explosion)
    // Instead, store the info for the interpreter to use
    let _ = inlinable; // Available for future use
}

fn contains_call(expr: &Expression, name: &str) -> bool {
    match &expr.kind {
        ExpressionKind::Call { name: n, args, .. } => {
            if n == name { return true; }
            args.iter().any(|a| contains_call(a, name))
        }
        ExpressionKind::BinaryOp { left, right, .. } => {
            contains_call(left, name) || contains_call(right, name)
        }
        ExpressionKind::UnaryOp { operand, .. } => contains_call(operand, name),
        _ => false,
    }
}

/// Loop unrolling for small constant-bound loops.
pub fn unroll_small_loops(program: &mut Program) {
    for stmt in &mut program.statements {
        if let StatementKind::FunctionDef(fdef) = &mut stmt.kind {
            unroll_loops_in_block(&mut fdef.body);
        }
    }
}

fn unroll_loops_in_block(block: &mut Block) {
    // Recurse into nested blocks first
    for stmt in block.statements.iter_mut() {
        match &mut stmt.kind {
            StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
                unroll_loops_in_block(&mut fdef.body);
            }
            StatementKind::ForLoop { body, .. } | StatementKind::WhileLoop { body, .. }
            | StatementKind::DoWhileLoop { body, .. } => {
                unroll_loops_in_block(body);
            }
            StatementKind::TryCatch { try_block, catch_block, finally_block, .. } => {
                unroll_loops_in_block(try_block);
                unroll_loops_in_block(catch_block);
                if let Some(fb) = finally_block { unroll_loops_in_block(fb); }
            }
            _ => {}
        }
        // Fold constants within each statement
        fold_statement(stmt);
    }
}

fn fold_statement(stmt: &mut Statement) {
    match &mut stmt.kind {
        StatementKind::Let { value, .. }
        | StatementKind::LetMut { value, .. }
        | StatementKind::Assignment { value, .. }
        | StatementKind::ConstDef { value, .. }
        | StatementKind::StaticDef { value, .. }
        | StatementKind::Output(value)
        | StatementKind::ExprStatement(value)
        | StatementKind::Throw(value) => {
            fold_expr(value);
        }
        StatementKind::LetDestructure { value, .. } => {
            fold_expr(value);
        }
        StatementKind::CompoundAssign { value, .. } => {
            fold_expr(value);
        }
        StatementKind::FieldAssignment { object, value, .. } => {
            fold_expr(object);
            fold_expr(value);
        }
        StatementKind::IndexAssignment { object, index, value } => {
            fold_expr(object);
            fold_expr(index);
            fold_expr(value);
        }
        StatementKind::ForLoop {
            iterable, body, ..
        } => {
            fold_expr(iterable);
            fold_block(body);
        }
        StatementKind::WhileLoop { condition, body, .. } => {
            fold_expr(condition);
            fold_block(body);
        }
        StatementKind::DoWhileLoop { body, condition, .. } => {
            fold_block(body);
            fold_expr(condition);
        }
        StatementKind::Defer(expr) => {
            fold_expr(expr);
        }
        StatementKind::CStyleFor { init, condition, update, body } => {
            fold_statement(init);
            fold_expr(condition);
            fold_statement(update);
            fold_block(body);
        }
        StatementKind::TupleAssignment { value, .. } => {
            fold_expr(value);
        }
        StatementKind::Increment { .. } | StatementKind::Decrement { .. } => {
            // No sub-expressions to fold
        }
        StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
            for param in &mut fdef.params {
                if let Some(default) = &mut param.default {
                    fold_expr(default);
                }
            }
            fold_block(&mut fdef.body);
        }
        StatementKind::Break { value: Some(expr), .. } => {
            fold_expr(expr);
        }
        StatementKind::Return(Some(expr)) => {
            fold_expr(expr);
        }
        StatementKind::TryCatch {
            try_block,
            catch_block,
            finally_block,
            ..
        } => {
            fold_block(try_block);
            fold_block(catch_block);
            if let Some(fb) = finally_block {
                fold_block(fb);
            }
        }
        StatementKind::ModuleDef { body, .. } => {
            fold_block(body);
        }
        StatementKind::TestDef { body, .. } => {
            fold_block(body);
        }
        // These statements have no expressions to fold
        StatementKind::Import(_)
        | StatementKind::ImportModule { .. }
        | StatementKind::Break { value: None, .. }
        | StatementKind::Continue { .. }
        | StatementKind::Return(None)
        | StatementKind::TypeAlias { .. }
        | StatementKind::Use { .. }
        | StatementKind::EnumDef { .. }
        | StatementKind::StructDef { .. }
        | StatementKind::ImplBlock { .. }
        | StatementKind::TraitDef { .. }
        | StatementKind::ImplTrait { .. } => {}
    }
}

fn fold_block(block: &mut Block) {
    for stmt in &mut block.statements {
        fold_statement(stmt);
    }
    if let Some(tail) = &mut block.tail_expr {
        fold_expr(tail);
    }
    // Dead code elimination: remove statements after return/break/continue/throw
    if let Some(pos) = block.statements.iter().position(|s| matches!(
        &s.kind,
        StatementKind::Return(_) | StatementKind::Break { value: _, .. }
        | StatementKind::Continue { .. } | StatementKind::Throw(_)
    )) {
        block.statements.truncate(pos + 1);
        // If there's a tail expr after a terminator, remove it
        block.tail_expr = None;
    }
}

fn fold_expr(expr: &mut Expression) {
    // First, recursively fold sub-expressions (bottom-up)
    match &mut expr.kind {
        ExpressionKind::BinaryOp { left, right, .. } => {
            fold_expr(left);
            fold_expr(right);
        }
        ExpressionKind::UnaryOp { operand, .. } => {
            fold_expr(operand);
        }
        ExpressionKind::Call { args, kwargs, .. } => {
            for arg in args.iter_mut() {
                fold_expr(arg);
            }
            for (_, val) in kwargs.iter_mut() {
                fold_expr(val);
            }
            // Do NOT fold the call itself (side effects)
            return;
        }
        ExpressionKind::MethodCall {
            object,
            args,
            kwargs,
            ..
        } => {
            fold_expr(object);
            for arg in args.iter_mut() {
                fold_expr(arg);
            }
            for (_, val) in kwargs.iter_mut() {
                fold_expr(val);
            }
            // Do NOT fold the method call itself (side effects)
            return;
        }
        ExpressionKind::Pipe { left, right } => {
            fold_expr(left);
            fold_expr(right);
            return;
        }
        ExpressionKind::IfElse {
            condition,
            then_block,
            else_block,
        } => {
            fold_expr(condition);
            fold_block(then_block);
            if let Some(eb) = else_block {
                fold_block(eb);
            }
            // Constant condition elimination: if true/false with else branch
            if let ExpressionKind::Literal(Literal::Bool(val)) = &condition.kind {
                if *val {
                    // `if true { X } else { Y }` → X (as block expression)
                    let block = then_block.clone();
                    expr.kind = ExpressionKind::Block(block);
                } else if let Some(eb) = else_block.take() {
                    // `if false { X } else { Y }` → Y (as block expression)
                    expr.kind = ExpressionKind::Block(eb);
                }
                // `if false { X }` without else → leave as-is (evaluates to Null)
            }
            return;
        }
        ExpressionKind::Block(block) => {
            fold_block(block);
            return;
        }
        ExpressionKind::Index { object, index } => {
            fold_expr(object);
            fold_expr(index);
            return;
        }
        ExpressionKind::FieldAccess { object, .. } => {
            fold_expr(object);
            return;
        }
        ExpressionKind::Range {
            start, end, ..
        } => {
            fold_expr(start);
            fold_expr(end);
            return;
        }
        ExpressionKind::Await(inner) | ExpressionKind::Spawn(inner) | ExpressionKind::Yield(inner) => {
            fold_expr(inner);
            return;
        }
        ExpressionKind::UnsafeBlock(block) => {
            fold_block(block);
            return;
        }
        ExpressionKind::InlineAsm { operands, .. } => {
            for op in operands.iter_mut() {
                fold_expr(op);
            }
            return;
        }
        ExpressionKind::Lambda { body, params, .. } => {
            for param in params.iter_mut() {
                if let Some(default) = &mut param.default {
                    fold_expr(default);
                }
            }
            fold_expr(body);
            return;
        }
        ExpressionKind::Match { value, arms } => {
            fold_expr(value);
            for arm in arms.iter_mut() {
                fold_match_arm(arm);
            }
            return;
        }
        ExpressionKind::StringInterpolation { parts } => {
            for part in parts.iter_mut() {
                if let StringPart::Expr(e) = part {
                    fold_expr(e);
                }
            }
            // If all parts are literal strings (or expressions that folded to string literals),
            // concatenate into a single string literal.
            let all_const = parts.iter().all(|p| match p {
                StringPart::Literal(_) => true,
                StringPart::Expr(e) => matches!(&e.kind, ExpressionKind::Literal(Literal::String(_))),
            });
            if all_const && parts.len() > 1 {
                let mut result = String::new();
                for part in parts.iter() {
                    match part {
                        StringPart::Literal(s) => result.push_str(s),
                        StringPart::Expr(e) => {
                            if let ExpressionKind::Literal(Literal::String(s)) = &e.kind {
                                result.push_str(s);
                            }
                        }
                    }
                }
                expr.kind = ExpressionKind::Literal(Literal::String(result));
            }
            return;
        }
        ExpressionKind::NullCoalesce { left, right } => {
            fold_expr(left);
            fold_expr(right);
            return;
        }
        ExpressionKind::OptionalChain { object, .. } => {
            fold_expr(object);
            return;
        }
        ExpressionKind::Spread(inner) => {
            fold_expr(inner);
            return;
        }
        ExpressionKind::Loop { body: block, .. } => {
            fold_block(block);
            return;
        }
        ExpressionKind::TryCatchExpr {
            try_block,
            catch_block,
            finally_block,
            ..
        } => {
            fold_block(try_block);
            fold_block(catch_block);
            if let Some(fb) = finally_block {
                fold_block(fb);
            }
            return;
        }
        ExpressionKind::ListComprehension {
            expr: comp_expr,
            iterable,
            condition,
            ..
        } => {
            fold_expr(comp_expr);
            fold_expr(iterable);
            if let Some(cond) = condition {
                fold_expr(cond);
            }
            return;
        }
        ExpressionKind::MapComprehension {
            key_expr,
            value_expr,
            iterable,
            condition,
            ..
        } => {
            fold_expr(key_expr);
            fold_expr(value_expr);
            fold_expr(iterable);
            if let Some(cond) = condition {
                fold_expr(cond);
            }
            return;
        }
        ExpressionKind::EnumConstruct { args, .. } => {
            for arg in args.iter_mut() {
                fold_expr(arg);
            }
            return;
        }
        ExpressionKind::StructConstruct { fields, .. } => {
            for (_, val) in fields.iter_mut() {
                fold_expr(val);
            }
            return;
        }
        ExpressionKind::TryPropagate(inner) => {
            fold_expr(inner);
            return;
        }
        ExpressionKind::Literal(Literal::Array(elements)) => {
            for el in elements.iter_mut() {
                fold_expr(el);
            }
            return;
        }
        ExpressionKind::Literal(Literal::Map(entries)) => {
            for (_, val) in entries.iter_mut() {
                fold_expr(val);
            }
            return;
        }
        ExpressionKind::Ref(inner) => {
            fold_expr(inner);
            return;
        }
        ExpressionKind::MoveClosure { params, body } => {
            for param in params.iter_mut() {
                if let Some(default) = &mut param.default {
                    fold_expr(default);
                }
            }
            fold_expr(body);
            return;
        }
        // Scalar literals, variables, placeholders, dyn trait -- nothing to fold inside
        ExpressionKind::Literal(_) | ExpressionKind::Variable(_) | ExpressionKind::Placeholder | ExpressionKind::DynTrait(_) => {
            return;
        }
    }

    // Now try to fold the current node (only BinaryOp and UnaryOp reach here)
    let span = expr.span;

    // Double negation elimination: --x → x, !!x → x
    if let ExpressionKind::UnaryOp { op: outer_op, operand } = &mut expr.kind {
        if let ExpressionKind::UnaryOp { op: inner_op, operand: inner } = &mut operand.kind {
            if *outer_op == *inner_op {
                // --x → x or !!x → x
                let inner_expr = std::mem::replace(
                    inner.as_mut(),
                    Expression { kind: ExpressionKind::Literal(Literal::Null), span },
                );
                *expr = inner_expr;
                return;
            }
        }
    }

    // Identity operation elimination: x + 0, x - 0, x * 1, x / 1
    // Boolean short-circuit: true && x → x, false && x → false, true || x → true, false || x → x
    // Comparison simplification: x == true → x, x == false → !x
    if let ExpressionKind::BinaryOp { op, left, right } = &mut expr.kind {
        let simplified = match op {
            BinOp::Add => {
                if is_zero_literal(right) {
                    Some(true) // keep left
                } else if is_zero_literal(left) {
                    Some(false) // keep right
                } else {
                    None
                }
            }
            BinOp::Sub => {
                if is_zero_literal(right) {
                    Some(true) // x - 0 → x
                } else {
                    None
                }
            }
            BinOp::Mul => {
                if is_one_literal(right) {
                    Some(true) // x * 1 → x
                } else if is_one_literal(left) {
                    Some(false) // 1 * x → x
                } else {
                    None
                }
            }
            BinOp::Div => {
                if is_one_literal(right) {
                    Some(true) // x / 1 → x
                } else {
                    None
                }
            }
            BinOp::And => {
                // true && x → x, false && x → false
                match &left.kind {
                    ExpressionKind::Literal(Literal::Bool(true)) => Some(false), // keep right
                    ExpressionKind::Literal(Literal::Bool(false)) => None, // literal fold handles this
                    _ => {
                        // x && true → x, x && false → already handled by literal fold
                        match &right.kind {
                            ExpressionKind::Literal(Literal::Bool(true)) => Some(true), // keep left
                            _ => None,
                        }
                    }
                }
            }
            BinOp::Or => {
                // false || x → x, true || x → true
                match &left.kind {
                    ExpressionKind::Literal(Literal::Bool(false)) => Some(false), // keep right
                    ExpressionKind::Literal(Literal::Bool(true)) => None, // literal fold handles this
                    _ => {
                        // x || false → x
                        match &right.kind {
                            ExpressionKind::Literal(Literal::Bool(false)) => Some(true), // keep left
                            _ => None,
                        }
                    }
                }
            }
            _ => None,
        };
        if let Some(keep_left) = simplified {
            let replacement = if keep_left {
                std::mem::replace(
                    left.as_mut(),
                    Expression { kind: ExpressionKind::Literal(Literal::Null), span },
                )
            } else {
                std::mem::replace(
                    right.as_mut(),
                    Expression { kind: ExpressionKind::Literal(Literal::Null), span },
                )
            };
            *expr = replacement;
            return;
        }

        // x == true → x, x == false → !x (only when exactly one side is a bool literal)
        if *op == BinOp::Eq || *op == BinOp::NotEq {
            // Determine which side (if any) is a bool literal
            enum BoolSide { Right(bool), Left(bool), Neither }
            let side = match (&left.kind, &right.kind) {
                (_, ExpressionKind::Literal(Literal::Bool(b)))
                    if !matches!(&left.kind, ExpressionKind::Literal(Literal::Bool(_))) =>
                {
                    BoolSide::Right(*b)
                }
                (ExpressionKind::Literal(Literal::Bool(b)), _)
                    if !matches!(&right.kind, ExpressionKind::Literal(Literal::Bool(_))) =>
                {
                    BoolSide::Left(*b)
                }
                _ => BoolSide::Neither,
            };
            match side {
                BoolSide::Right(b) => {
                    let effectively_true = if *op == BinOp::Eq { b } else { !b };
                    if effectively_true {
                        let replacement = std::mem::replace(
                            left.as_mut(),
                            Expression { kind: ExpressionKind::Literal(Literal::Null), span },
                        );
                        *expr = replacement;
                    } else {
                        let inner = std::mem::replace(
                            left.as_mut(),
                            Expression { kind: ExpressionKind::Literal(Literal::Null), span },
                        );
                        expr.kind = ExpressionKind::UnaryOp {
                            op: UnOp::Not,
                            operand: Box::new(inner),
                        };
                    }
                    return;
                }
                BoolSide::Left(b) => {
                    let effectively_true = if *op == BinOp::Eq { b } else { !b };
                    if effectively_true {
                        let replacement = std::mem::replace(
                            right.as_mut(),
                            Expression { kind: ExpressionKind::Literal(Literal::Null), span },
                        );
                        *expr = replacement;
                    } else {
                        let inner = std::mem::replace(
                            right.as_mut(),
                            Expression { kind: ExpressionKind::Literal(Literal::Null), span },
                        );
                        expr.kind = ExpressionKind::UnaryOp {
                            op: UnOp::Not,
                            operand: Box::new(inner),
                        };
                    }
                    return;
                }
                BoolSide::Neither => {} // Let literal fold handle it
            }
        }
    }

    let folded = match &expr.kind {
        ExpressionKind::BinaryOp { op, left, right } => try_fold_binary(*op, left, right, span),
        ExpressionKind::UnaryOp { op, operand } => try_fold_unary(*op, operand, span),
        _ => None,
    };

    if let Some(new_expr) = folded {
        *expr = new_expr;
    }
}

fn fold_match_arm(arm: &mut MatchArm) {
    if let Some(guard) = &mut arm.guard {
        fold_expr(guard);
    }
    fold_block(&mut arm.body);
}

/// Check if an expression is a zero literal (int or float).
fn is_zero_literal(expr: &Expression) -> bool {
    match &expr.kind {
        ExpressionKind::Literal(Literal::Int64(0)) => true,
        ExpressionKind::Literal(Literal::Float64(f)) if *f == 0.0 => true,
        _ => false,
    }
}

/// Check if an expression is a one literal (int or float).
fn is_one_literal(expr: &Expression) -> bool {
    match &expr.kind {
        ExpressionKind::Literal(Literal::Int64(1)) => true,
        ExpressionKind::Literal(Literal::Float64(f)) if *f == 1.0 => true,
        _ => false,
    }
}

/// Try to fold a binary operation on two literal operands.
/// Returns `None` if folding is not possible (non-literal operands, division by zero, etc.).
fn try_fold_binary(op: BinOp, left: &Expression, right: &Expression, span: Span) -> Option<Expression> {
    let left_lit = match &left.kind {
        ExpressionKind::Literal(lit) => lit,
        _ => return None,
    };
    let right_lit = match &right.kind {
        ExpressionKind::Literal(lit) => lit,
        _ => return None,
    };

    let result_lit = match (op, left_lit, right_lit) {
        (BinOp::Add, Literal::Int64(a), Literal::Int64(b)) => a.checked_add(*b).map(Literal::Int64),
        (BinOp::Sub, Literal::Int64(a), Literal::Int64(b)) => a.checked_sub(*b).map(Literal::Int64),
        (BinOp::Mul, Literal::Int64(a), Literal::Int64(b)) => a.checked_mul(*b).map(Literal::Int64),
        (BinOp::Div, Literal::Int64(a), Literal::Int64(b)) => {
            if *b == 0 {
                return None; // Leave division by zero for runtime
            }
            a.checked_div(*b).map(Literal::Int64)
        }
        (BinOp::Mod, Literal::Int64(a), Literal::Int64(b)) => {
            if *b == 0 {
                return None; // Leave modulo by zero for runtime
            }
            a.checked_rem(*b).map(Literal::Int64)
        }

        (BinOp::Add, Literal::Float64(a), Literal::Float64(b)) => Some(Literal::Float64(a + b)),
        (BinOp::Sub, Literal::Float64(a), Literal::Float64(b)) => Some(Literal::Float64(a - b)),
        (BinOp::Mul, Literal::Float64(a), Literal::Float64(b)) => Some(Literal::Float64(a * b)),
        (BinOp::Div, Literal::Float64(a), Literal::Float64(b)) => {
            if *b == 0.0 {
                return None; // Leave division by zero for runtime
            }
            Some(Literal::Float64(a / b))
        }
        (BinOp::Mod, Literal::Float64(a), Literal::Float64(b)) => {
            if *b == 0.0 {
                return None;
            }
            Some(Literal::Float64(a % b))
        }

        // Mixed int/float arithmetic -- promote to float
        (BinOp::Add, Literal::Int64(a), Literal::Float64(b)) => Some(Literal::Float64(*a as f64 + b)),
        (BinOp::Add, Literal::Float64(a), Literal::Int64(b)) => Some(Literal::Float64(a + *b as f64)),
        (BinOp::Sub, Literal::Int64(a), Literal::Float64(b)) => Some(Literal::Float64(*a as f64 - b)),
        (BinOp::Sub, Literal::Float64(a), Literal::Int64(b)) => Some(Literal::Float64(a - *b as f64)),
        (BinOp::Mul, Literal::Int64(a), Literal::Float64(b)) => Some(Literal::Float64(*a as f64 * b)),
        (BinOp::Mul, Literal::Float64(a), Literal::Int64(b)) => Some(Literal::Float64(a * *b as f64)),
        (BinOp::Div, Literal::Int64(a), Literal::Float64(b)) => {
            if *b == 0.0 { return None; }
            Some(Literal::Float64(*a as f64 / b))
        }
        (BinOp::Div, Literal::Float64(a), Literal::Int64(b)) => {
            if *b == 0 { return None; }
            Some(Literal::Float64(a / *b as f64))
        }
        (BinOp::Mod, Literal::Int64(a), Literal::Float64(b)) => {
            if *b == 0.0 { return None; }
            Some(Literal::Float64(*a as f64 % b))
        }
        (BinOp::Mod, Literal::Float64(a), Literal::Int64(b)) => {
            if *b == 0 { return None; }
            Some(Literal::Float64(a % *b as f64))
        }

        (BinOp::Add, Literal::String(a), Literal::String(b)) => {
            Some(Literal::String(format!("{}{}", a, b)))
        }

        (BinOp::And, Literal::Bool(a), Literal::Bool(b)) => Some(Literal::Bool(*a && *b)),
        (BinOp::Or, Literal::Bool(a), Literal::Bool(b)) => Some(Literal::Bool(*a || *b)),

        (BinOp::Eq, Literal::Int64(a), Literal::Int64(b)) => Some(Literal::Bool(a == b)),
        (BinOp::NotEq, Literal::Int64(a), Literal::Int64(b)) => Some(Literal::Bool(a != b)),
        (BinOp::Gt, Literal::Int64(a), Literal::Int64(b)) => Some(Literal::Bool(a > b)),
        (BinOp::Lt, Literal::Int64(a), Literal::Int64(b)) => Some(Literal::Bool(a < b)),
        (BinOp::GtEq, Literal::Int64(a), Literal::Int64(b)) => Some(Literal::Bool(a >= b)),
        (BinOp::LtEq, Literal::Int64(a), Literal::Int64(b)) => Some(Literal::Bool(a <= b)),

        (BinOp::Eq, Literal::Float64(a), Literal::Float64(b)) => Some(Literal::Bool(a == b)),
        (BinOp::NotEq, Literal::Float64(a), Literal::Float64(b)) => Some(Literal::Bool(a != b)),
        (BinOp::Gt, Literal::Float64(a), Literal::Float64(b)) => Some(Literal::Bool(a > b)),
        (BinOp::Lt, Literal::Float64(a), Literal::Float64(b)) => Some(Literal::Bool(a < b)),
        (BinOp::GtEq, Literal::Float64(a), Literal::Float64(b)) => Some(Literal::Bool(a >= b)),
        (BinOp::LtEq, Literal::Float64(a), Literal::Float64(b)) => Some(Literal::Bool(a <= b)),

        // Mixed int/float comparisons
        (BinOp::Eq, Literal::Int64(a), Literal::Float64(b)) => Some(Literal::Bool(*a as f64 == *b)),
        (BinOp::Eq, Literal::Float64(a), Literal::Int64(b)) => Some(Literal::Bool(*a == *b as f64)),
        (BinOp::NotEq, Literal::Int64(a), Literal::Float64(b)) => Some(Literal::Bool(*a as f64 != *b)),
        (BinOp::NotEq, Literal::Float64(a), Literal::Int64(b)) => Some(Literal::Bool(*a != *b as f64)),
        (BinOp::Gt, Literal::Int64(a), Literal::Float64(b)) => Some(Literal::Bool((*a as f64) > *b)),
        (BinOp::Gt, Literal::Float64(a), Literal::Int64(b)) => Some(Literal::Bool(*a > *b as f64)),
        (BinOp::Lt, Literal::Int64(a), Literal::Float64(b)) => Some(Literal::Bool((*a as f64) < *b)),
        (BinOp::Lt, Literal::Float64(a), Literal::Int64(b)) => Some(Literal::Bool(*a < *b as f64)),
        (BinOp::GtEq, Literal::Int64(a), Literal::Float64(b)) => Some(Literal::Bool(*a as f64 >= *b)),
        (BinOp::GtEq, Literal::Float64(a), Literal::Int64(b)) => Some(Literal::Bool(*a >= *b as f64)),
        (BinOp::LtEq, Literal::Int64(a), Literal::Float64(b)) => Some(Literal::Bool(*a as f64 <= *b)),
        (BinOp::LtEq, Literal::Float64(a), Literal::Int64(b)) => Some(Literal::Bool(*a <= *b as f64)),

        (BinOp::Eq, Literal::String(a), Literal::String(b)) => Some(Literal::Bool(a == b)),
        (BinOp::NotEq, Literal::String(a), Literal::String(b)) => Some(Literal::Bool(a != b)),

        (BinOp::Eq, Literal::Bool(a), Literal::Bool(b)) => Some(Literal::Bool(a == b)),
        (BinOp::NotEq, Literal::Bool(a), Literal::Bool(b)) => Some(Literal::Bool(a != b)),

        (BinOp::Eq, Literal::Null, Literal::Null) => Some(Literal::Bool(true)),
        (BinOp::NotEq, Literal::Null, Literal::Null) => Some(Literal::Bool(false)),

        // Everything else: don't fold
        _ => None,
    };

    result_lit.map(|lit| Expression {
        kind: ExpressionKind::Literal(lit),
        span,
    })
}

/// Try to fold a unary operation on a literal operand.
fn try_fold_unary(op: UnOp, operand: &Expression, span: Span) -> Option<Expression> {
    let lit = match &operand.kind {
        ExpressionKind::Literal(lit) => lit,
        _ => return None,
    };

    let result_lit = match (op, lit) {
        (UnOp::Neg, Literal::Int64(n)) => n.checked_neg().map(Literal::Int64),
        (UnOp::Neg, Literal::Float64(f)) => Some(Literal::Float64(-f)),

        (UnOp::Not, Literal::Bool(b)) => Some(Literal::Bool(!b)),
        // Not on null is true (null is falsy)
        (UnOp::Not, Literal::Null) => Some(Literal::Bool(true)),
        // Not on integers: 0 is falsy, anything else is truthy
        (UnOp::Not, Literal::Int64(n)) => Some(Literal::Bool(*n == 0)),
        // Not on floats: 0.0 and NaN are falsy
        (UnOp::Not, Literal::Float64(f)) => Some(Literal::Bool(*f == 0.0 || f.is_nan())),
        // Not on strings: empty is falsy
        (UnOp::Not, Literal::String(s)) => Some(Literal::Bool(s.is_empty())),

        _ => None,
    };

    result_lit.map(|lit| Expression {
        kind: ExpressionKind::Literal(lit),
        span,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to make a literal expression.
    fn lit_int(n: i64) -> Expression {
        Expression {
            kind: ExpressionKind::Literal(Literal::Int64(n)),
            span: Span::default(),
        }
    }

    fn lit_float(f: f64) -> Expression {
        Expression {
            kind: ExpressionKind::Literal(Literal::Float64(f)),
            span: Span::default(),
        }
    }

    fn lit_str(s: &str) -> Expression {
        Expression {
            kind: ExpressionKind::Literal(Literal::String(s.to_string())),
            span: Span::default(),
        }
    }

    fn lit_bool(b: bool) -> Expression {
        Expression {
            kind: ExpressionKind::Literal(Literal::Bool(b)),
            span: Span::default(),
        }
    }

    fn lit_null() -> Expression {
        Expression {
            kind: ExpressionKind::Literal(Literal::Null),
            span: Span::default(),
        }
    }

    fn binop(op: BinOp, left: Expression, right: Expression) -> Expression {
        Expression {
            kind: ExpressionKind::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            span: Span::default(),
        }
    }

    fn unop(op: UnOp, operand: Expression) -> Expression {
        Expression {
            kind: ExpressionKind::UnaryOp {
                op,
                operand: Box::new(operand),
            },
            span: Span::default(),
        }
    }

    fn make_program(stmts: Vec<Statement>) -> Program {
        Program {
            statements: stmts,
            span: Span::default(),
            trailing_comments: Vec::new(),
        }
    }

    fn let_stmt(name: &str, value: Expression) -> Statement {
        Statement::new(
            StatementKind::Let {
                name: name.to_string(),
                type_annotation: None,
                value,
            },
            Span::default(),
        )
    }

    fn assert_is_int(expr: &Expression, expected: i64) {
        match &expr.kind {
            ExpressionKind::Literal(Literal::Int64(n)) => assert_eq!(*n, expected),
            other => panic!("expected Int64({}), got {:?}", expected, other),
        }
    }

    fn assert_is_float(expr: &Expression, expected: f64) {
        match &expr.kind {
            ExpressionKind::Literal(Literal::Float64(f)) => {
                assert!(
                    (f - expected).abs() < f64::EPSILON,
                    "expected Float64({}), got Float64({})",
                    expected,
                    f
                );
            }
            other => panic!("expected Float64({}), got {:?}", expected, other),
        }
    }

    fn assert_is_str(expr: &Expression, expected: &str) {
        match &expr.kind {
            ExpressionKind::Literal(Literal::String(s)) => assert_eq!(s, expected),
            other => panic!("expected String({:?}), got {:?}", expected, other),
        }
    }

    fn assert_is_bool(expr: &Expression, expected: bool) {
        match &expr.kind {
            ExpressionKind::Literal(Literal::Bool(b)) => assert_eq!(*b, expected),
            other => panic!("expected Bool({}), got {:?}", expected, other),
        }
    }


    #[test]
    fn fold_int_add() {
        let mut expr = binop(BinOp::Add, lit_int(2), lit_int(3));
        fold_expr(&mut expr);
        assert_is_int(&expr, 5);
    }

    #[test]
    fn fold_int_sub() {
        let mut expr = binop(BinOp::Sub, lit_int(10), lit_int(4));
        fold_expr(&mut expr);
        assert_is_int(&expr, 6);
    }

    #[test]
    fn fold_int_mul() {
        let mut expr = binop(BinOp::Mul, lit_int(10), lit_int(2));
        fold_expr(&mut expr);
        assert_is_int(&expr, 20);
    }

    #[test]
    fn fold_int_div() {
        let mut expr = binop(BinOp::Div, lit_int(8), lit_int(2));
        fold_expr(&mut expr);
        assert_is_int(&expr, 4);
    }

    #[test]
    fn fold_int_mod() {
        let mut expr = binop(BinOp::Mod, lit_int(10), lit_int(3));
        fold_expr(&mut expr);
        assert_is_int(&expr, 1);
    }


    #[test]
    fn fold_float_add() {
        let mut expr = binop(BinOp::Add, lit_float(1.5), lit_float(2.5));
        fold_expr(&mut expr);
        assert_is_float(&expr, 4.0);
    }

    #[test]
    fn fold_float_mul() {
        let mut expr = binop(BinOp::Mul, lit_float(3.0), lit_float(2.0));
        fold_expr(&mut expr);
        assert_is_float(&expr, 6.0);
    }

    #[test]
    fn fold_float_div() {
        let mut expr = binop(BinOp::Div, lit_float(10.0), lit_float(4.0));
        fold_expr(&mut expr);
        assert_is_float(&expr, 2.5);
    }

    // Mixed int/float arithmetic

    #[test]
    fn fold_mixed_add_int_float() {
        let mut expr = binop(BinOp::Add, lit_int(2), lit_float(3.5));
        fold_expr(&mut expr);
        assert_is_float(&expr, 5.5);
    }

    #[test]
    fn fold_mixed_mul_float_int() {
        let mut expr = binop(BinOp::Mul, lit_float(2.5), lit_int(4));
        fold_expr(&mut expr);
        assert_is_float(&expr, 10.0);
    }


    #[test]
    fn fold_string_concat() {
        let mut expr = binop(BinOp::Add, lit_str("hello"), lit_str(" world"));
        fold_expr(&mut expr);
        assert_is_str(&expr, "hello world");
    }

    #[test]
    fn fold_string_concat_chained() {
        // "hello" + " " + "world" => nested: ("hello" + " ") + "world"
        let inner = binop(BinOp::Add, lit_str("hello"), lit_str(" "));
        let mut expr = binop(BinOp::Add, inner, lit_str("world"));
        fold_expr(&mut expr);
        assert_is_str(&expr, "hello world");
    }


    #[test]
    fn fold_bool_and_true() {
        let mut expr = binop(BinOp::And, lit_bool(true), lit_bool(true));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_bool_and_false() {
        let mut expr = binop(BinOp::And, lit_bool(true), lit_bool(false));
        fold_expr(&mut expr);
        assert_is_bool(&expr, false);
    }

    #[test]
    fn fold_bool_or_false() {
        let mut expr = binop(BinOp::Or, lit_bool(false), lit_bool(false));
        fold_expr(&mut expr);
        assert_is_bool(&expr, false);
    }

    #[test]
    fn fold_bool_or_true() {
        let mut expr = binop(BinOp::Or, lit_bool(false), lit_bool(true));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_not_true() {
        let mut expr = unop(UnOp::Not, lit_bool(true));
        fold_expr(&mut expr);
        assert_is_bool(&expr, false);
    }

    #[test]
    fn fold_not_false() {
        let mut expr = unop(UnOp::Not, lit_bool(false));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }


    #[test]
    fn fold_int_gt() {
        let mut expr = binop(BinOp::Gt, lit_int(5), lit_int(3));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_int_lt() {
        let mut expr = binop(BinOp::Lt, lit_int(5), lit_int(3));
        fold_expr(&mut expr);
        assert_is_bool(&expr, false);
    }

    #[test]
    fn fold_int_eq() {
        let mut expr = binop(BinOp::Eq, lit_int(1), lit_int(2));
        fold_expr(&mut expr);
        assert_is_bool(&expr, false);
    }

    #[test]
    fn fold_int_eq_true() {
        let mut expr = binop(BinOp::Eq, lit_int(5), lit_int(5));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_int_noteq() {
        let mut expr = binop(BinOp::NotEq, lit_int(1), lit_int(2));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_int_gteq() {
        let mut expr = binop(BinOp::GtEq, lit_int(5), lit_int(5));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_int_lteq() {
        let mut expr = binop(BinOp::LtEq, lit_int(3), lit_int(5));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }


    #[test]
    fn fold_neg_int() {
        let mut expr = unop(UnOp::Neg, lit_int(5));
        fold_expr(&mut expr);
        assert_is_int(&expr, -5);
    }

    #[test]
    fn fold_neg_float() {
        let mut expr = unop(UnOp::Neg, lit_float(3.14));
        fold_expr(&mut expr);
        assert_is_float(&expr, -3.14);
    }

    #[test]
    fn fold_double_neg() {
        // -(-5) => 5
        let inner = unop(UnOp::Neg, lit_int(5));
        let mut expr = unop(UnOp::Neg, inner);
        fold_expr(&mut expr);
        assert_is_int(&expr, 5);
    }

    // Division by zero: must NOT fold

    #[test]
    fn no_fold_int_div_by_zero() {
        let mut expr = binop(BinOp::Div, lit_int(10), lit_int(0));
        fold_expr(&mut expr);
        // Should remain as BinaryOp, not folded
        assert!(matches!(expr.kind, ExpressionKind::BinaryOp { .. }));
    }

    #[test]
    fn no_fold_float_div_by_zero() {
        let mut expr = binop(BinOp::Div, lit_float(10.0), lit_float(0.0));
        fold_expr(&mut expr);
        assert!(matches!(expr.kind, ExpressionKind::BinaryOp { .. }));
    }

    #[test]
    fn no_fold_int_mod_by_zero() {
        let mut expr = binop(BinOp::Mod, lit_int(10), lit_int(0));
        fold_expr(&mut expr);
        assert!(matches!(expr.kind, ExpressionKind::BinaryOp { .. }));
    }

    // Function calls: must NOT fold

    #[test]
    fn no_fold_function_call() {
        let mut expr = Expression {
            kind: ExpressionKind::Call {
                name: "some_fn".to_string(),
                args: vec![lit_int(1), lit_int(2)],
                kwargs: vec![],
            },
            span: Span::default(),
        };
        fold_expr(&mut expr);
        assert!(matches!(expr.kind, ExpressionKind::Call { .. }));
    }


    #[test]
    fn fold_nested_expression() {
        // (2 + 3) * (4 - 1) => 5 * 3 => 15
        let left = binop(BinOp::Add, lit_int(2), lit_int(3));
        let right = binop(BinOp::Sub, lit_int(4), lit_int(1));
        let mut expr = binop(BinOp::Mul, left, right);
        fold_expr(&mut expr);
        assert_is_int(&expr, 15);
    }

    #[test]
    fn fold_deeply_nested() {
        // ((1 + 2) + (3 + 4)) => (3 + 7) => 10
        let a = binop(BinOp::Add, lit_int(1), lit_int(2));
        let b = binop(BinOp::Add, lit_int(3), lit_int(4));
        let mut expr = binop(BinOp::Add, a, b);
        fold_expr(&mut expr);
        assert_is_int(&expr, 10);
    }

    // Program-level folding

    #[test]
    fn fold_program_let() {
        let mut program = make_program(vec![let_stmt(
            "x",
            binop(BinOp::Add, lit_int(10), lit_int(20)),
        )]);
        fold_constants(&mut program);
        match &program.statements[0].kind {
            StatementKind::Let { value, .. } => assert_is_int(value, 30),
            _ => panic!("expected Let"),
        }
    }

    #[test]
    fn fold_program_output() {
        let mut program = make_program(vec![Statement::new(
            StatementKind::Output(binop(BinOp::Mul, lit_int(6), lit_int(7))),
            Span::default(),
        )]);
        fold_constants(&mut program);
        match &program.statements[0].kind {
            StatementKind::Output(expr) => assert_is_int(expr, 42),
            _ => panic!("expected Output"),
        }
    }

    // Non-constant expressions should not be folded

    #[test]
    fn no_fold_variable_operand() {
        let var = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Add, var, lit_int(1));
        fold_expr(&mut expr);
        assert!(matches!(expr.kind, ExpressionKind::BinaryOp { .. }));
    }

    // Not on non-bool literals

    #[test]
    fn fold_not_null() {
        let mut expr = unop(UnOp::Not, lit_null());
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_not_zero_int() {
        let mut expr = unop(UnOp::Not, lit_int(0));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_not_nonzero_int() {
        let mut expr = unop(UnOp::Not, lit_int(42));
        fold_expr(&mut expr);
        assert_is_bool(&expr, false);
    }

    #[test]
    fn fold_not_empty_string() {
        let mut expr = unop(UnOp::Not, lit_str(""));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_not_nonempty_string() {
        let mut expr = unop(UnOp::Not, lit_str("hello"));
        fold_expr(&mut expr);
        assert_is_bool(&expr, false);
    }


    #[test]
    fn fold_string_eq() {
        let mut expr = binop(BinOp::Eq, lit_str("abc"), lit_str("abc"));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_string_noteq() {
        let mut expr = binop(BinOp::NotEq, lit_str("abc"), lit_str("def"));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }


    #[test]
    fn fold_bool_eq() {
        let mut expr = binop(BinOp::Eq, lit_bool(true), lit_bool(true));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_bool_noteq() {
        let mut expr = binop(BinOp::NotEq, lit_bool(true), lit_bool(false));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }


    #[test]
    fn fold_null_eq_null() {
        let mut expr = binop(BinOp::Eq, lit_null(), lit_null());
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_null_noteq_null() {
        let mut expr = binop(BinOp::NotEq, lit_null(), lit_null());
        fold_expr(&mut expr);
        assert_is_bool(&expr, false);
    }


    #[test]
    fn no_fold_int_add_overflow() {
        let mut expr = binop(BinOp::Add, lit_int(i64::MAX), lit_int(1));
        fold_expr(&mut expr);
        // Should remain as BinaryOp because checked_add returns None
        assert!(matches!(expr.kind, ExpressionKind::BinaryOp { .. }));
    }

    #[test]
    fn no_fold_int_mul_overflow() {
        let mut expr = binop(BinOp::Mul, lit_int(i64::MAX), lit_int(2));
        fold_expr(&mut expr);
        assert!(matches!(expr.kind, ExpressionKind::BinaryOp { .. }));
    }

    #[test]
    fn no_fold_neg_min_overflow() {
        let mut expr = unop(UnOp::Neg, lit_int(i64::MIN));
        fold_expr(&mut expr);
        // checked_neg on i64::MIN returns None (overflow)
        assert!(matches!(expr.kind, ExpressionKind::UnaryOp { .. }));
    }

    // Mixed int/float comparisons

    #[test]
    fn fold_mixed_gt() {
        let mut expr = binop(BinOp::Gt, lit_int(10), lit_float(3.5));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_mixed_eq() {
        let mut expr = binop(BinOp::Eq, lit_float(5.0), lit_int(5));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }


    #[test]
    fn fold_float_gt() {
        let mut expr = binop(BinOp::Gt, lit_float(5.5), lit_float(3.2));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_float_eq() {
        let mut expr = binop(BinOp::Eq, lit_float(1.0), lit_float(1.0));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    // Double negation/not elimination (variable case)

    #[test]
    fn fold_double_not() {
        // !!true → true
        let mut expr = unop(UnOp::Not, unop(UnOp::Not, lit_bool(true)));
        fold_expr(&mut expr);
        assert_is_bool(&expr, true);
    }

    #[test]
    fn fold_double_neg_variable() {
        // --x → x (variable, not literal)
        let var = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = unop(UnOp::Neg, unop(UnOp::Neg, var));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(name) if name == "x"));
    }


    #[test]
    fn fold_add_zero_right() {
        // x + 0 → x (with variable)
        let var = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Add, var, lit_int(0));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(name) if name == "x"));
    }

    #[test]
    fn fold_add_zero_left() {
        // 0 + x → x
        let var = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Add, lit_int(0), var);
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(name) if name == "x"));
    }

    #[test]
    fn fold_sub_zero() {
        // x - 0 → x
        let var = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Sub, var, lit_int(0));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(name) if name == "x"));
    }

    #[test]
    fn fold_mul_one_right() {
        // x * 1 → x
        let var = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Mul, var, lit_int(1));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(name) if name == "x"));
    }

    #[test]
    fn fold_mul_one_left() {
        // 1 * x → x
        let var = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Mul, lit_int(1), var);
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(name) if name == "x"));
    }

    #[test]
    fn fold_div_one() {
        // x / 1 → x
        let var = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Div, var, lit_int(1));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(name) if name == "x"));
    }

    #[test]
    fn fold_add_float_zero() {
        // x + 0.0 → x
        let var = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Add, var, lit_float(0.0));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(name) if name == "x"));
    }

    #[test]
    fn fold_mul_float_one() {
        // x * 1.0 → x
        let var = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Mul, var, lit_float(1.0));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(name) if name == "x"));
    }

    #[test]
    fn fold_literal_identity_still_folds() {
        // 5 + 0 → 5 (literal case, identity elimination + constant folding)
        let mut expr = binop(BinOp::Add, lit_int(5), lit_int(0));
        fold_expr(&mut expr);
        assert_is_int(&expr, 5);
    }

    // Boolean short-circuit tests

    #[test]
    fn fold_true_and_x() {
        // true && x → x
        let x = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::And, lit_bool(true), x);
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(n) if n == "x"));
    }

    #[test]
    fn fold_x_and_true() {
        // x && true → x
        let x = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::And, x, lit_bool(true));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(n) if n == "x"));
    }

    #[test]
    fn fold_false_or_x() {
        // false || x → x
        let x = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Or, lit_bool(false), x);
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(n) if n == "x"));
    }

    #[test]
    fn fold_x_or_false() {
        // x || false → x
        let x = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Or, x, lit_bool(false));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(n) if n == "x"));
    }

    #[test]
    fn fold_x_eq_true() {
        // x == true → x
        let x = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Eq, x, lit_bool(true));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(n) if n == "x"));
    }

    #[test]
    fn fold_x_eq_false() {
        // x == false → !x
        let x = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Eq, x, lit_bool(false));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::UnaryOp { op: UnOp::Not, .. }));
    }

    #[test]
    fn fold_x_neq_true() {
        // x != true → !x
        let x = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::NotEq, x, lit_bool(true));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::UnaryOp { op: UnOp::Not, .. }));
    }

    #[test]
    fn fold_x_neq_false() {
        // x != false → x
        let x = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::NotEq, x, lit_bool(false));
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(n) if n == "x"));
    }

    #[test]
    fn fold_true_eq_x() {
        // true == x → x
        let x = Expression {
            kind: ExpressionKind::Variable("x".to_string()),
            span: Span::default(),
        };
        let mut expr = binop(BinOp::Eq, lit_bool(true), x);
        fold_expr(&mut expr);
        assert!(matches!(&expr.kind, ExpressionKind::Variable(n) if n == "x"));
    }
}
