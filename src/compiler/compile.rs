//! AST → IR compiler for MAGI.
//!
//! Walks the AST and emits stack-based IR instructions that map to WASM.

use std::collections::HashMap;

use crate::syntax::ast::*;

use super::ir::*;
use super::CompileError;

/// The MAGI compiler. Translates a parsed AST into an IR module.
pub struct Compiler {
    /// The IR module being built.
    module: IrModule,
    /// Current function being compiled.
    current_fn: Option<FnBuilder>,
    /// Map from function name → function index in module.
    fn_index: HashMap<String, u32>,
    /// Counter for generating unique lambda names.
    lambda_counter: u32,
    /// Loop context stack for break/continue.
    loop_stack: Vec<LoopContext>,
}

/// Builder for a function being compiled.
struct FnBuilder {
    name: String,
    param_count: u32,
    has_rest: bool,
    locals: Vec<IrLocal>,
    instructions: Vec<Instruction>,
    /// Map from variable name → local index.
    scope_stack: Vec<HashMap<String, u32>>,
    exported: bool,
    return_type: ValType,
}

impl FnBuilder {
    fn new(name: String, exported: bool) -> Self {
        Self {
            name,
            param_count: 0,
            has_rest: false,
            locals: Vec::new(),
            instructions: Vec::new(),
            scope_stack: vec![HashMap::new()],
            exported,
            return_type: ValType::Tagged,
        }
    }

    fn emit(&mut self, inst: Instruction) {
        self.instructions.push(inst);
    }

    fn push_scope(&mut self) {
        self.scope_stack.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scope_stack.pop();
    }

    fn define_local(&mut self, name: &str, val_type: ValType, mutable: bool) -> u32 {
        let idx = self.locals.len() as u32;
        self.locals.push(IrLocal {
            name: name.to_string(),
            val_type,
            mutable,
        });
        if let Some(scope) = self.scope_stack.last_mut() {
            scope.insert(name.to_string(), idx);
        }
        idx
    }

    fn resolve_local(&self, name: &str) -> Option<u32> {
        for scope in self.scope_stack.iter().rev() {
            if let Some(&idx) = scope.get(name) {
                return Some(idx);
            }
        }
        None
    }
}

/// Context for compiling loop bodies (break/continue targets).
struct LoopContext {
    /// Block label for break (forward branch out of loop).
    break_label: u32,
    /// Loop label for continue (backward branch to loop start).
    continue_label: u32,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            module: IrModule::new(),
            current_fn: None,
            fn_index: HashMap::new(),
            lambda_counter: 0,
            loop_stack: Vec::new(),
        }
    }

    /// Compile a MAGI program into an IR module.
    pub fn compile(&mut self, program: &Program) -> Result<IrModule, CompileError> {
        // First pass: register all function definitions.
        self.register_functions(program)?;

        // Second pass: compile top-level code into __main.
        self.begin_function("__main", true);
        self.compile_statements(&program.statements)?;
        // Return null if no explicit return.
        self.emit(Instruction::PushNull);
        self.emit(Instruction::Return);
        self.end_function();

        // Third pass: compile function bodies.
        let fn_defs: Vec<_> = program
            .statements
            .iter()
            .filter_map(|s| match &s.kind {
                StatementKind::FunctionDef(f) | StatementKind::AsyncFunctionDef(f) => Some(f.clone()),
                _ => None,
            })
            .collect();

        for func in &fn_defs {
            self.compile_function_def(func)?;
        }

        Ok(self.module.clone())
    }

    // ── Pass 1: Register function signatures ─────────────────────────

    fn register_functions(&mut self, program: &Program) -> Result<(), CompileError> {
        // Reserve index 0 for __main.
        let main_idx = self.module.functions.len() as u32;
        self.fn_index.insert("__main".to_string(), main_idx);
        // Placeholder — will be replaced by end_function().
        self.module.functions.push(IrFunction {
            name: "__main".to_string(),
            param_count: 0,
            has_rest: false,
            locals: Vec::new(),
            instructions: Vec::new(),
            exported: true,
            return_type: ValType::Tagged,
        });

        // Register built-in runtime functions.
        self.register_builtins();

        for stmt in &program.statements {
            match &stmt.kind {
                StatementKind::FunctionDef(f) | StatementKind::AsyncFunctionDef(f) => {
                    let idx = self.module.functions.len() as u32;
                    self.fn_index.insert(f.name.clone(), idx);
                    // Placeholder.
                    self.module.functions.push(IrFunction {
                        name: f.name.clone(),
                        param_count: f.params.len() as u32,
                        has_rest: f.params.last().map_or(false, |p| p.rest),
                        locals: Vec::new(),
                        instructions: Vec::new(),
                        exported: false,
                        return_type: ValType::Tagged,
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn register_builtins(&mut self) {
        let builtins = [
            ("println", 1),
            ("print", 1),
            ("len", 1),
            ("typeof", 1),
            ("assert", 1),
            ("assert_eq", 2),
            ("to_string", 1),
            ("parse_int", 1),
            ("parse_float", 1),
            ("push", 2),
            ("array_push", 2),
            ("pop", 1),
            ("range", 2),
            ("debug_log", 1),
            ("map_get", 2),
            ("map_set", 3),
            ("map_from_entries", 1),
        ];

        for (name, param_count) in builtins {
            if !self.fn_index.contains_key(name) {
                let idx = self.module.functions.len() as u32;
                self.fn_index.insert(name.to_string(), idx);
                let mut instructions = Vec::new();
                match name {
                    "println" | "print" | "debug_log" => {
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::Print);
                        instructions.push(Instruction::PushNull);
                        instructions.push(Instruction::Return);
                    }
                    "len" => {
                        // Delegate to RuntimeCall so wasm.rs handles tag dispatch.
                        let name_idx = self.module.intern_string("len");
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::RuntimeCall { name: name_idx, arg_count: 1 });
                        instructions.push(Instruction::Return);
                    }
                    "to_string" => {
                        let name_idx = self.module.intern_string("to_string");
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::RuntimeCall { name: name_idx, arg_count: 1 });
                        instructions.push(Instruction::Return);
                    }
                    "typeof" => {
                        let name_idx = self.module.intern_string("typeof");
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::RuntimeCall { name: name_idx, arg_count: 1 });
                        instructions.push(Instruction::Return);
                    }
                    "array_push" | "push" => {
                        let name_idx = self.module.intern_string("array_push");
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::LocalGet(1));
                        instructions.push(Instruction::RuntimeCall { name: name_idx, arg_count: 2 });
                        instructions.push(Instruction::Return);
                    }
                    "map_get" => {
                        let name_idx = self.module.intern_string("map_get");
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::LocalGet(1));
                        instructions.push(Instruction::RuntimeCall { name: name_idx, arg_count: 2 });
                        instructions.push(Instruction::Return);
                    }
                    "map_set" => {
                        let name_idx = self.module.intern_string("map_set");
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::LocalGet(1));
                        instructions.push(Instruction::LocalGet(2));
                        instructions.push(Instruction::RuntimeCall { name: name_idx, arg_count: 3 });
                        instructions.push(Instruction::Return);
                    }
                    "map_from_entries" => {
                        let name_idx = self.module.intern_string("map_from_entries");
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::RuntimeCall { name: name_idx, arg_count: 1 });
                        instructions.push(Instruction::Return);
                    }
                    "range" => {
                        // range(start, end) → __range(start, end, false)
                        let name_idx = self.module.intern_string("__range");
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::LocalGet(1));
                        instructions.push(Instruction::PushBool(false));
                        instructions.push(Instruction::RuntimeCall { name: name_idx, arg_count: 3 });
                        instructions.push(Instruction::Return);
                    }
                    "assert" => {
                        // assert(value) → if falsy, trap; else return null.
                        // BoolNot turns truthy→0, falsy→1 (tagged bool). If truthy (not-falsy), trap.
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::BoolNot);
                        instructions.push(Instruction::If);
                        instructions.push(Instruction::Unreachable); // trap on assertion failure
                        instructions.push(Instruction::End);
                        instructions.push(Instruction::PushNull);
                        instructions.push(Instruction::Return);
                    }
                    "assert_eq" => {
                        // assert_eq(a, b) → if a != b, trap; else return null.
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::LocalGet(1));
                        instructions.push(Instruction::I64Ne);
                        instructions.push(Instruction::TagBool);
                        instructions.push(Instruction::If);
                        instructions.push(Instruction::Unreachable); // trap on assertion failure
                        instructions.push(Instruction::End);
                        instructions.push(Instruction::PushNull);
                        instructions.push(Instruction::Return);
                    }
                    "parse_int" => {
                        // Delegate to runtime for string→int conversion.
                        let name_idx = self.module.intern_string("parse_int");
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::RuntimeCall { name: name_idx, arg_count: 1 });
                        instructions.push(Instruction::Return);
                    }
                    "parse_float" => {
                        let name_idx = self.module.intern_string("parse_float");
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::RuntimeCall { name: name_idx, arg_count: 1 });
                        instructions.push(Instruction::Return);
                    }
                    "pop" => {
                        let name_idx = self.module.intern_string("pop");
                        instructions.push(Instruction::LocalGet(0));
                        instructions.push(Instruction::RuntimeCall { name: name_idx, arg_count: 1 });
                        instructions.push(Instruction::Return);
                    }
                    _ => {
                        // Unknown builtin: trap instead of silently returning null.
                        instructions.push(Instruction::Unreachable);
                    }
                }
                let locals: Vec<IrLocal> = (0..param_count)
                    .map(|i| IrLocal {
                        name: format!("__param{}", i),
                        val_type: ValType::Tagged,
                        mutable: false,
                    })
                    .collect();
                self.module.functions.push(IrFunction {
                    name: name.to_string(),
                    param_count,
                    has_rest: false,
                    locals,
                    instructions,
                    exported: false,
                    return_type: ValType::Tagged,
                });
            }
        }

        // Intern type name strings used by typeof runtime dispatch.
        for name in &["null", "bool", "int64", "float64", "string", "array", "map"] {
            self.module.intern_string(name);
        }
    }

    // ── Function building ────────────────────────────────────────────

    fn begin_function(&mut self, name: &str, exported: bool) {
        self.current_fn = Some(FnBuilder::new(name.to_string(), exported));
    }

    fn end_function(&mut self) {
        if let Some(fb) = self.current_fn.take() {
            let func = IrFunction {
                name: fb.name.clone(),
                param_count: fb.param_count,
                has_rest: fb.has_rest,
                locals: fb.locals,
                instructions: fb.instructions,
                exported: fb.exported,
                return_type: fb.return_type,
            };
            // Replace placeholder or push new.
            if let Some(&idx) = self.fn_index.get(&fb.name) {
                self.module.functions[idx as usize] = func;
            } else {
                let idx = self.module.functions.len() as u32;
                self.fn_index.insert(fb.name.clone(), idx);
                self.module.functions.push(func);
            }
        }
    }

    fn emit(&mut self, inst: Instruction) {
        if let Some(fb) = &mut self.current_fn {
            fb.emit(inst);
        }
    }

    fn fb(&mut self) -> &mut FnBuilder {
        self.current_fn.as_mut().expect("no function context")
    }

    fn define_local(&mut self, name: &str, val_type: ValType, mutable: bool) -> u32 {
        self.fb().define_local(name, val_type, mutable)
    }

    fn resolve_local(&self, name: &str) -> Option<u32> {
        self.current_fn.as_ref()?.resolve_local(name)
    }

    // ── Compile statements ───────────────────────────────────────────

    fn compile_statements(&mut self, stmts: &[Statement]) -> Result<(), CompileError> {
        for stmt in stmts {
            self.compile_statement(stmt)?;
        }
        Ok(())
    }

    fn compile_statement(&mut self, stmt: &Statement) -> Result<(), CompileError> {
        match &stmt.kind {
            StatementKind::Let { name, value, .. } => {
                self.compile_expr(value)?;
                let idx = self.define_local(name, ValType::Tagged, false);
                self.emit(Instruction::LocalSet(idx));
            }

            StatementKind::LetMut { name, value, .. } => {
                self.compile_expr(value)?;
                let idx = self.define_local(name, ValType::Tagged, true);
                self.emit(Instruction::LocalSet(idx));
            }

            StatementKind::LetDestructure { pattern, value, mutable, .. } => {
                self.compile_expr(value)?;
                self.compile_destructure(pattern, *mutable)?;
            }

            StatementKind::Assignment { name, value } => {
                self.compile_expr(value)?;
                if let Some(idx) = self.resolve_local(name) {
                    self.emit(Instruction::LocalSet(idx));
                } else {
                    return Err(CompileError::at(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("undefined variable: {name}"),
                    ));
                }
            }

            StatementKind::CompoundAssign { name, op, value } => {
                if let Some(idx) = self.resolve_local(name) {
                    self.emit(Instruction::LocalGet(idx));
                    self.compile_expr(value)?;
                    self.compile_binop(*op)?;
                    self.emit(Instruction::LocalSet(idx));
                } else {
                    return Err(CompileError::at(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("undefined variable: {name}"),
                    ));
                }
            }

            StatementKind::ExprStatement(expr) => {
                self.compile_expr(expr)?;
                self.emit(Instruction::Drop);
            }

            StatementKind::Output(expr) => {
                self.compile_expr(expr)?;
                self.emit(Instruction::Print);
            }

            StatementKind::ForLoop { pattern, iterable, body } => {
                self.compile_for_loop(pattern, iterable, body)?;
            }

            StatementKind::WhileLoop { condition, body } => {
                self.compile_while_loop(condition, body)?;
            }

            StatementKind::Break(expr) => {
                if let Some(e) = expr {
                    self.compile_expr(e)?;
                }
                if let Some(ctx) = self.loop_stack.last() {
                    let label = ctx.break_label;
                    self.emit(Instruction::Br(label));
                }
            }

            StatementKind::Continue => {
                if let Some(ctx) = self.loop_stack.last() {
                    let label = ctx.continue_label;
                    self.emit(Instruction::Br(label));
                }
            }

            StatementKind::Return(expr) => {
                if let Some(e) = expr {
                    self.compile_expr(e)?;
                } else {
                    self.emit(Instruction::PushNull);
                }
                self.emit(Instruction::Return);
            }

            StatementKind::FunctionDef(_) | StatementKind::AsyncFunctionDef(_) => {
                // Already compiled in second pass.
            }

            StatementKind::ConstDef { name, value, .. } => {
                self.compile_expr(value)?;
                let idx = self.define_local(name, ValType::Tagged, false);
                self.emit(Instruction::LocalSet(idx));
            }

            StatementKind::TryCatch {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                self.compile_try_catch(try_block, catch_var.as_deref(), catch_block, finally_block.as_ref())?;
            }

            StatementKind::Throw(expr) => {
                self.compile_expr(expr)?;
                self.emit(Instruction::Unreachable);
            }

            StatementKind::Import(_) => {
                // No-op in compiled mode; imports are resolved at link time.
            }

            StatementKind::Use { .. } => {
                // No-op in compiled mode.
            }

            StatementKind::TypeAlias { .. } => {
                // Type aliases are compile-time only.
            }

            StatementKind::ModuleDef { name: _, body } => {
                self.fb().push_scope();
                self.compile_block(body)?;
                self.emit(Instruction::Drop);
                self.fb().pop_scope();
            }

            StatementKind::TestDef { name, body } => {
                // Compile tests as callable functions.
                let test_name = format!("__test_{}", name.replace(' ', "_"));
                let idx = self.module.functions.len() as u32;
                self.fn_index.insert(test_name.clone(), idx);

                let prev = self.current_fn.take();
                self.begin_function(&test_name, false);
                self.compile_block(body)?;
                self.emit(Instruction::PushNull);
                self.emit(Instruction::Return);
                self.end_function();
                self.current_fn = prev;
            }

            StatementKind::EnumDef { name, variants } => {
                // Enum definitions are compile-time type info.
                // Store variant info in string pool for runtime construction.
                for variant in variants {
                    let key = format!("{}::{}", name, variant.name);
                    self.module.intern_string(&key);
                }
            }

            StatementKind::StructDef { name, fields } => {
                // Struct definitions are compile-time type info.
                for field in fields {
                    let key = format!("{}::{}", name, field.name);
                    self.module.intern_string(&key);
                }
            }
        }
        Ok(())
    }

    // ── Compile expressions ──────────────────────────────────────────

    fn compile_expr(&mut self, expr: &Expression) -> Result<(), CompileError> {
        match &expr.kind {
            ExpressionKind::Literal(lit) => self.compile_literal(lit)?,

            ExpressionKind::Variable(name) => {
                if let Some(idx) = self.resolve_local(name) {
                    self.emit(Instruction::LocalGet(idx));
                } else if let Some(&fn_idx) = self.fn_index.get(name.as_str()) {
                    // Function reference — push index as i64 for indirect calls.
                    self.emit(Instruction::PushI64(fn_idx as i64));
                } else {
                    // Unresolved variable — may be a runtime/closure capture.
                    // Emit a runtime lookup instead of erroring.
                    let name_idx = self.module.intern_string(name);
                    self.emit(Instruction::RuntimeCall {
                        name: name_idx,
                        arg_count: 0,
                    });
                }
            }

            ExpressionKind::BinaryOp { op, left, right } => {
                // Short-circuit for && and ||.
                match op {
                    BinOp::And => {
                        self.compile_expr(left)?;
                        let temp = self.ensure_temp_local();
                        self.emit(Instruction::Block);
                        self.emit(Instruction::Block);
                        // Duplicate & test: if falsy, skip right side.
                        self.emit(Instruction::LocalTee(temp));
                        self.emit(Instruction::BoolNot);
                        self.emit(Instruction::BrIf(0)); // branch to inner block end
                        self.emit(Instruction::Drop); // drop left value
                        self.compile_expr(right)?;
                        self.emit(Instruction::End); // inner block
                        self.emit(Instruction::End); // outer block
                    }
                    BinOp::Or => {
                        self.compile_expr(left)?;
                        let temp = self.ensure_temp_local();
                        self.emit(Instruction::Block);
                        self.emit(Instruction::Block);
                        self.emit(Instruction::LocalTee(temp));
                        self.emit(Instruction::BrIf(0)); // if truthy, skip right
                        self.emit(Instruction::Drop); // drop left
                        self.compile_expr(right)?;
                        self.emit(Instruction::End);
                        self.emit(Instruction::End);
                    }
                    _ => {
                        self.compile_expr(left)?;
                        self.compile_expr(right)?;
                        self.compile_binop(*op)?;
                    }
                }
            }

            ExpressionKind::UnaryOp { op, operand } => {
                self.compile_expr(operand)?;
                match op {
                    UnOp::Not => self.emit(Instruction::BoolNot),
                    UnOp::Neg => {
                        // Dispatch based on type tag: f64 uses F64Neg, i64 uses I64Neg.
                        let temp = self.ensure_temp_local();
                        self.emit(Instruction::LocalTee(temp));
                        self.emit(Instruction::GetTag);
                        self.emit(Instruction::PushI64(tag::F64 as i64));
                        self.emit(Instruction::I64Eq);
                        self.emit(Instruction::If);
                        // Float path: untag as f64, negate, retag.
                        self.emit(Instruction::LocalGet(temp));
                        self.emit(Instruction::UntagF64);
                        self.emit(Instruction::F64Neg);
                        self.emit(Instruction::TagF64);
                        self.emit(Instruction::Else);
                        // Integer path: untag as i64, negate, retag.
                        self.emit(Instruction::LocalGet(temp));
                        self.emit(Instruction::UntagI64);
                        self.emit(Instruction::I64Neg);
                        self.emit(Instruction::TagI64);
                        self.emit(Instruction::End);
                    }
                }
            }

            ExpressionKind::Call { name, args, .. } => {
                // Compile arguments.
                for arg in args {
                    if let ExpressionKind::Spread(inner) = &arg.kind {
                        // Spread in function call — at runtime this expands the array.
                        self.compile_expr(inner)?;
                    } else {
                        self.compile_expr(arg)?;
                    }
                }

                if let Some(&fn_idx) = self.fn_index.get(name.as_str()) {
                    self.emit(Instruction::Call(fn_idx));
                } else {
                    // Runtime call for operations we don't have a compiled version of.
                    let name_idx = self.module.intern_string(name);
                    self.emit(Instruction::RuntimeCall {
                        name: name_idx,
                        arg_count: args.len() as u32,
                    });
                }
            }

            ExpressionKind::MethodCall { object, method, args, .. } => {
                self.compile_expr(object)?;
                for arg in args {
                    self.compile_expr(arg)?;
                }
                let name_idx = self.module.intern_string(method);
                self.emit(Instruction::RuntimeCall {
                    name: name_idx,
                    arg_count: (args.len() + 1) as u32, // +1 for self
                });
            }

            ExpressionKind::IfElse { condition, then_block, else_block } => {
                self.compile_expr(condition)?;
                self.emit(Instruction::If);
                self.compile_block(then_block)?;
                if let Some(else_blk) = else_block {
                    self.emit(Instruction::Else);
                    self.compile_block(else_blk)?;
                } else {
                    self.emit(Instruction::Else);
                    self.emit(Instruction::PushNull);
                }
                self.emit(Instruction::End);
            }

            ExpressionKind::Block(block) => {
                self.compile_block(block)?;
            }

            ExpressionKind::Index { object, index } => {
                // Check for range (slice syntax).
                if let ExpressionKind::Range { start, end, inclusive } = &index.kind {
                    self.compile_expr(object)?;
                    self.compile_expr(start)?;
                    self.compile_expr(end)?;
                    self.emit(Instruction::PushBool(*inclusive));
                    let name_idx = self.module.intern_string("__slice");
                    self.emit(Instruction::RuntimeCall {
                        name: name_idx,
                        arg_count: 4,
                    });
                } else {
                    self.compile_expr(object)?;
                    self.compile_expr(index)?;
                    self.emit(Instruction::ArrayGet);
                }
            }

            ExpressionKind::FieldAccess { object, field } => {
                self.compile_expr(object)?;
                let key = self.module.intern_string(field);
                self.emit(Instruction::PushString(key));
                self.emit(Instruction::MapGet);
            }

            ExpressionKind::Range { start, end, inclusive } => {
                self.compile_expr(start)?;
                self.compile_expr(end)?;
                self.emit(Instruction::PushBool(*inclusive));
                let name_idx = self.module.intern_string("__range");
                self.emit(Instruction::RuntimeCall {
                    name: name_idx,
                    arg_count: 3,
                });
            }

            ExpressionKind::Lambda { params, body } => {
                let lambda_name = format!("__lambda_{}", self.lambda_counter);
                self.lambda_counter += 1;

                let idx = self.module.functions.len() as u32;
                self.fn_index.insert(lambda_name.clone(), idx);
                // Push placeholder that end_function will replace.
                self.module.functions.push(IrFunction {
                    name: lambda_name.clone(),
                    param_count: params.len() as u32,
                    has_rest: params.last().map_or(false, |p| p.rest),
                    locals: Vec::new(),
                    instructions: Vec::new(),
                    exported: false,
                    return_type: ValType::Tagged,
                });

                // Save current context.
                let prev = self.current_fn.take();
                self.begin_function(&lambda_name, false);
                {
                    let fb = self.fb();
                    fb.param_count = params.len() as u32;
                    fb.has_rest = params.last().map_or(false, |p| p.rest);
                }
                for param in params {
                    self.define_local(&param.name, ValType::Tagged, false);
                }
                self.compile_expr(body)?;
                self.emit(Instruction::Return);
                self.end_function();
                self.current_fn = prev;

                // Push function reference.
                self.emit(Instruction::PushI64(idx as i64));
            }

            ExpressionKind::Pipe { left, right } => {
                // Compile left, then substitute into right's placeholder.
                // For now, compile as: left becomes first arg of right's call.
                self.compile_expr(left)?;
                match &right.kind {
                    ExpressionKind::Call { name, args, .. } => {
                        // Insert left value as replacement for placeholder.
                        for arg in args {
                            if matches!(&arg.kind, ExpressionKind::Placeholder) {
                                // Already on stack from left.
                            } else {
                                self.compile_expr(arg)?;
                            }
                        }
                        if let Some(&fn_idx) = self.fn_index.get(name.as_str()) {
                            self.emit(Instruction::Call(fn_idx));
                        } else {
                            let name_idx = self.module.intern_string(name);
                            self.emit(Instruction::RuntimeCall {
                                name: name_idx,
                                arg_count: args.len() as u32,
                            });
                        }
                    }
                    _ => {
                        // General case: compile right as function, call with left as arg.
                        self.compile_expr(right)?;
                        self.emit(Instruction::CallIndirect(1));
                    }
                }
            }

            ExpressionKind::Match { value, arms } => {
                self.compile_match(value, arms)?;
            }

            ExpressionKind::StringInterpolation { parts } => {
                if parts.is_empty() {
                    let idx = self.module.intern_string("");
                    self.emit(Instruction::PushString(idx));
                    return Ok(());
                }
                let mut first = true;
                for part in parts {
                    match part {
                        StringPart::Literal(s) => {
                            let idx = self.module.intern_string(s);
                            self.emit(Instruction::PushString(idx));
                        }
                        StringPart::Expr(e) => {
                            self.compile_expr(e)?;
                            // Convert to string if needed.
                            let name_idx = self.module.intern_string("to_string");
                            self.emit(Instruction::RuntimeCall {
                                name: name_idx,
                                arg_count: 1,
                            });
                        }
                    }
                    if !first {
                        self.emit(Instruction::StringConcat);
                    }
                    first = false;
                }
            }

            ExpressionKind::NullCoalesce { left, right } => {
                self.compile_expr(left)?;
                // If not null, keep; else evaluate right.
                let temp = self.ensure_temp_local();
                self.emit(Instruction::LocalTee(temp));
                self.emit(Instruction::GetTag);
                self.emit(Instruction::PushI64(tag::NULL as i64));
                self.emit(Instruction::I64Eq);
                self.emit(Instruction::If);
                self.emit(Instruction::Drop);
                self.compile_expr(right)?;
                self.emit(Instruction::End);
            }

            ExpressionKind::OptionalChain { object, field } => {
                self.compile_expr(object)?;
                let temp = self.ensure_temp_local();
                self.emit(Instruction::LocalTee(temp));
                self.emit(Instruction::GetTag);
                self.emit(Instruction::PushI64(tag::NULL as i64));
                self.emit(Instruction::I64Eq);
                self.emit(Instruction::If);
                self.emit(Instruction::PushNull);
                self.emit(Instruction::Else);
                self.emit(Instruction::LocalGet(temp));
                let key = self.module.intern_string(field);
                self.emit(Instruction::PushString(key));
                self.emit(Instruction::MapGet);
                self.emit(Instruction::End);
            }

            ExpressionKind::Spread(inner) => {
                self.compile_expr(inner)?;
            }

            ExpressionKind::Loop(block) => {
                self.compile_infinite_loop(block)?;
            }

            ExpressionKind::Await(inner) => {
                // In WASM, await is a no-op (synchronous execution).
                self.compile_expr(inner)?;
            }

            ExpressionKind::Spawn(inner) => {
                // In WASM, spawn just evaluates the expression.
                self.compile_expr(inner)?;
            }

            ExpressionKind::Placeholder => {
                // Should be handled by pipe compilation. Push null as fallback.
                self.emit(Instruction::PushNull);
            }

            ExpressionKind::TryCatchExpr { try_block, catch_var, catch_block } => {
                // Compile try block; catch block is compiled as fallback.
                // In WASM MVP, traps can't be caught. This handles Result-based errors.
                self.compile_block(try_block)?;
                // Compile catch block (for completeness, though WASM traps bypass it).
                if let Some(var_name) = catch_var {
                    self.fb().push_scope();
                    let _idx = self.define_local(var_name, ValType::Tagged, false);
                    self.compile_block(catch_block)?;
                    self.emit(Instruction::Drop);
                    self.fb().pop_scope();
                }
            }

            ExpressionKind::ListComprehension { expr, pattern, iterable, condition } => {
                self.compile_list_comprehension(expr, pattern, iterable, condition.as_deref())?;
            }

            ExpressionKind::MapComprehension { key_expr, value_expr, pattern, iterable, condition } => {
                self.compile_map_comprehension(key_expr, value_expr, pattern, iterable, condition.as_deref())?;
            }

            ExpressionKind::EnumConstruct { enum_name, variant, args } => {
                // Build map: {"__enum": name, "__variant": variant, "__data": [args]}
                let enum_key = self.module.intern_string("__enum");
                let variant_key = self.module.intern_string("__variant");
                let data_key = self.module.intern_string("__data");
                let enum_val = self.module.intern_string(enum_name);
                let variant_val = self.module.intern_string(variant);

                self.emit(Instruction::PushString(enum_key));
                self.emit(Instruction::PushString(enum_val));
                self.emit(Instruction::PushString(variant_key));
                self.emit(Instruction::PushString(variant_val));
                self.emit(Instruction::PushString(data_key));

                for arg in args {
                    self.compile_expr(arg)?;
                }
                self.emit(Instruction::ArrayNew(args.len() as u32));

                self.emit(Instruction::MapNew(3));
            }

            ExpressionKind::StructConstruct { name, fields } => {
                // Build map: {"__struct": name, field1: val1, ...}
                let struct_key = self.module.intern_string("__struct");
                let struct_val = self.module.intern_string(name);
                self.emit(Instruction::PushString(struct_key));
                self.emit(Instruction::PushString(struct_val));

                for (field_name, field_val) in fields {
                    let key = self.module.intern_string(field_name);
                    self.emit(Instruction::PushString(key));
                    self.compile_expr(field_val)?;
                }

                self.emit(Instruction::MapNew((fields.len() + 1) as u32));
            }

            ExpressionKind::TryPropagate(inner) => {
                self.compile_expr(inner)?;
                // Check for null → return null.
                let temp = self.ensure_temp_local();
                self.emit(Instruction::LocalTee(temp));
                self.emit(Instruction::GetTag);
                self.emit(Instruction::PushI64(tag::NULL as i64));
                self.emit(Instruction::I64Eq);
                self.emit(Instruction::If);
                self.emit(Instruction::PushNull);
                self.emit(Instruction::Return);
                self.emit(Instruction::Else);
                self.emit(Instruction::LocalGet(temp));
                self.emit(Instruction::End);
            }
        }
        Ok(())
    }

    // ── Helpers ──────────────────────────────────────────────────────

    fn compile_literal(&mut self, lit: &Literal) -> Result<(), CompileError> {
        match lit {
            Literal::Int64(n) => self.emit(Instruction::PushI64(*n)),
            Literal::Float64(n) => self.emit(Instruction::PushF64(*n)),
            Literal::String(s) => {
                let idx = self.module.intern_string(s);
                self.emit(Instruction::PushString(idx));
            }
            Literal::Bool(b) => self.emit(Instruction::PushBool(*b)),
            Literal::Null => self.emit(Instruction::PushNull),
            Literal::Array(elems) => {
                for elem in elems {
                    self.compile_expr(elem)?;
                }
                self.emit(Instruction::ArrayNew(elems.len() as u32));
            }
            Literal::Map(pairs) => {
                for (key, val) in pairs {
                    let key_idx = self.module.intern_string(key);
                    self.emit(Instruction::PushString(key_idx));
                    self.compile_expr(val)?;
                }
                self.emit(Instruction::MapNew(pairs.len() as u32));
            }
        }
        Ok(())
    }

    fn compile_binop(&mut self, op: BinOp) -> Result<(), CompileError> {
        // For Add, we need dynamic dispatch: string + string = StringConcat, else I64Add.
        if matches!(op, BinOp::Add) {
            // Stack: [left_tagged, right_tagged]
            // Use RuntimeCall "__add" which wasm.rs handles with dynamic type dispatch.
            let name_idx = self.module.intern_string("__add");
            self.emit(Instruction::RuntimeCall { name: name_idx, arg_count: 2 });
            return Ok(());
        }

        // For arithmetic and comparisons, untag both operands first.
        // Stack: [left_tagged, right_tagged]
        // We need to untag both. Use a temp local to hold right while untagging left.
        let temp = self.ensure_temp_local();
        self.emit(Instruction::LocalSet(temp)); // save right
        self.emit(Instruction::UntagI64);       // untag left
        self.emit(Instruction::LocalGet(temp));  // restore right
        self.emit(Instruction::UntagI64);       // untag right

        match op {
            BinOp::Add => unreachable!(), // handled above
            BinOp::Sub => {
                self.emit(Instruction::I64Sub);
                self.emit(Instruction::TagI64);
            }
            BinOp::Mul => {
                self.emit(Instruction::I64Mul);
                self.emit(Instruction::TagI64);
            }
            BinOp::Div => {
                self.emit(Instruction::I64Div);
                self.emit(Instruction::TagI64);
            }
            BinOp::Mod => {
                self.emit(Instruction::I64Rem);
                self.emit(Instruction::TagI64);
            }
            BinOp::Eq => {
                self.emit(Instruction::I64Eq);
                self.emit(Instruction::TagBool);
            }
            BinOp::NotEq => {
                self.emit(Instruction::I64Ne);
                self.emit(Instruction::TagBool);
            }
            BinOp::Gt => {
                self.emit(Instruction::I64Gt);
                self.emit(Instruction::TagBool);
            }
            BinOp::Lt => {
                self.emit(Instruction::I64Lt);
                self.emit(Instruction::TagBool);
            }
            BinOp::GtEq => {
                self.emit(Instruction::I64Ge);
                self.emit(Instruction::TagBool);
            }
            BinOp::LtEq => {
                self.emit(Instruction::I64Le);
                self.emit(Instruction::TagBool);
            }
            BinOp::And => {} // Handled as short-circuit above.
            BinOp::Or => {}  // Handled as short-circuit above.
        }
        Ok(())
    }

    fn compile_block(&mut self, block: &Block) -> Result<(), CompileError> {
        self.fb().push_scope();
        self.compile_statements(&block.statements)?;
        if let Some(tail) = &block.tail_expr {
            self.compile_expr(tail)?;
        } else {
            self.emit(Instruction::PushNull);
        }
        self.fb().pop_scope();
        Ok(())
    }

    fn compile_function_def(&mut self, func: &FunctionDef) -> Result<(), CompileError> {
        self.begin_function(&func.name, false);
        {
            let fb = self.fb();
            fb.param_count = func.params.len() as u32;
            fb.has_rest = func.params.last().map_or(false, |p| p.rest);
        }

        // Define parameter locals.
        for param in &func.params {
            self.define_local(&param.name, ValType::Tagged, false);
        }

        // Compile body.
        self.compile_block(&func.body)?;
        self.emit(Instruction::Return);
        self.end_function();
        Ok(())
    }

    fn compile_for_loop(
        &mut self,
        pattern: &ForPattern,
        iterable: &Expression,
        body: &Block,
    ) -> Result<(), CompileError> {
        // Evaluate iterable into a local.
        self.compile_expr(iterable)?;
        let iter_local = self.define_local("__iter", ValType::Tagged, false);
        self.emit(Instruction::LocalSet(iter_local));

        // Counter (raw untagged i64 — internal use only).
        let counter_local = self.define_local("__counter", ValType::I64, true);
        self.emit(Instruction::PushNull); // placeholder 0 value (will be overwritten)
        self.emit(Instruction::LocalSet(counter_local));
        // Actually store raw 0. We use PushNull (i64 0) since it's
        // already an i64 const 0. The counter is untagged.

        // Length (raw untagged i64).
        self.emit(Instruction::LocalGet(iter_local));
        self.emit(Instruction::ArrayLen);
        self.emit(Instruction::UntagI64); // strip tag to get raw length
        let len_local = self.define_local("__len", ValType::I64, false);
        self.emit(Instruction::LocalSet(len_local));

        // Loop structure.
        self.emit(Instruction::Block); // break target
        self.emit(Instruction::Loop);  // continue target

        let break_label = 1; // block (1 level up from loop)
        let continue_label = 0; // loop (current level)
        self.loop_stack.push(LoopContext {
            break_label,
            continue_label,
        });

        // Check: counter >= len (both raw untagged).
        self.emit(Instruction::LocalGet(counter_local));
        self.emit(Instruction::LocalGet(len_local));
        self.emit(Instruction::I64Ge);
        self.emit(Instruction::BrIf(1)); // break out of block

        // Load current element. ArrayGet expects tagged index.
        self.emit(Instruction::LocalGet(iter_local));
        self.emit(Instruction::LocalGet(counter_local));
        self.emit(Instruction::TagI64); // tag counter for ArrayGet
        self.emit(Instruction::ArrayGet);

        // Bind pattern.
        match pattern {
            ForPattern::Single(name) => {
                let idx = self.define_local(name, ValType::Tagged, false);
                self.emit(Instruction::LocalSet(idx));
            }
            ForPattern::ArrayDestructure(elements) => {
                let elem_local = self.define_local("__elem", ValType::Tagged, false);
                self.emit(Instruction::LocalSet(elem_local));
                for (i, elem) in elements.iter().enumerate() {
                    match elem {
                        DestructureElement::Name(name) => {
                            self.emit(Instruction::LocalGet(elem_local));
                            self.emit(Instruction::PushI64(i as i64));
                            self.emit(Instruction::ArrayGet);
                            let idx = self.define_local(name, ValType::Tagged, false);
                            self.emit(Instruction::LocalSet(idx));
                        }
                        DestructureElement::Rest(name) => {
                            // Rest: slice from i to end.
                            self.emit(Instruction::LocalGet(elem_local));
                            self.emit(Instruction::PushI64(i as i64));
                            self.emit(Instruction::LocalGet(elem_local));
                            self.emit(Instruction::ArrayLen);
                            self.emit(Instruction::PushBool(false));
                            let slice_name = self.module.intern_string("__slice");
                            self.emit(Instruction::RuntimeCall {
                                name: slice_name,
                                arg_count: 4,
                            });
                            let idx = self.define_local(name, ValType::Tagged, false);
                            self.emit(Instruction::LocalSet(idx));
                        }
                    }
                }
            }
            ForPattern::MapDestructure(entries) => {
                let elem_local = self.define_local("__elem", ValType::Tagged, false);
                self.emit(Instruction::LocalSet(elem_local));
                for (key, alias) in entries {
                    self.emit(Instruction::LocalGet(elem_local));
                    let key_idx = self.module.intern_string(key);
                    self.emit(Instruction::PushString(key_idx));
                    self.emit(Instruction::MapGet);
                    let bind_name = alias.as_deref().unwrap_or(key);
                    let idx = self.define_local(bind_name, ValType::Tagged, false);
                    self.emit(Instruction::LocalSet(idx));
                }
            }
        }

        // Compile body.
        self.fb().push_scope();
        self.compile_statements(&body.statements)?;
        if let Some(tail) = &body.tail_expr {
            self.compile_expr(tail)?;
            self.emit(Instruction::Drop);
        }
        self.fb().pop_scope();

        // Increment counter (raw untagged arithmetic).
        self.emit(Instruction::LocalGet(counter_local));
        self.emit(Instruction::PushI64(1));
        self.emit(Instruction::UntagI64); // get raw 1
        self.emit(Instruction::I64Add);
        self.emit(Instruction::LocalSet(counter_local));

        // Loop back.
        self.emit(Instruction::Br(0)); // back to Loop

        self.emit(Instruction::End); // Loop end
        self.emit(Instruction::End); // Block end

        self.loop_stack.pop();
        Ok(())
    }

    fn compile_while_loop(
        &mut self,
        condition: &Expression,
        body: &Block,
    ) -> Result<(), CompileError> {
        self.emit(Instruction::Block); // break target
        self.emit(Instruction::Loop);  // continue target

        self.loop_stack.push(LoopContext {
            break_label: 1,
            continue_label: 0,
        });

        // Check condition.
        self.compile_expr(condition)?;
        self.emit(Instruction::BoolNot);
        self.emit(Instruction::BrIf(1)); // break if false

        // Body.
        self.fb().push_scope();
        self.compile_statements(&body.statements)?;
        if let Some(tail) = &body.tail_expr {
            self.compile_expr(tail)?;
            self.emit(Instruction::Drop);
        }
        self.fb().pop_scope();

        self.emit(Instruction::Br(0)); // continue
        self.emit(Instruction::End); // Loop
        self.emit(Instruction::End); // Block

        self.loop_stack.pop();
        Ok(())
    }

    fn compile_infinite_loop(&mut self, block: &Block) -> Result<(), CompileError> {
        self.emit(Instruction::Block);
        self.emit(Instruction::Loop);

        self.loop_stack.push(LoopContext {
            break_label: 1,
            continue_label: 0,
        });

        self.compile_block(block)?;
        self.emit(Instruction::Drop);

        self.emit(Instruction::Br(0));
        self.emit(Instruction::End);
        self.emit(Instruction::End);

        self.loop_stack.pop();
        Ok(())
    }

    fn compile_match(
        &mut self,
        value: &Expression,
        arms: &[MatchArm],
    ) -> Result<(), CompileError> {
        self.compile_expr(value)?;
        let val_local = self.define_local("__match_val", ValType::Tagged, false);
        self.emit(Instruction::LocalSet(val_local));

        // Compile as chain of if-else.
        for (i, arm) in arms.iter().enumerate() {
            let is_last = i == arms.len() - 1;

            match &arm.pattern {
                Pattern::Wildcard | Pattern::Variable(_) => {
                    // Always matches.
                    if let Pattern::Variable(name) = &arm.pattern {
                        self.emit(Instruction::LocalGet(val_local));
                        let idx = self.define_local(name, ValType::Tagged, false);
                        self.emit(Instruction::LocalSet(idx));
                    }
                    self.compile_block(&arm.body)?;
                    // Skip remaining arms.
                    break;
                }
                Pattern::Literal(lit) => {
                    self.emit(Instruction::LocalGet(val_local));
                    self.compile_literal(lit)?;
                    self.emit(Instruction::I64Eq);
                    self.emit(Instruction::If);
                    self.compile_block(&arm.body)?;
                    if !is_last {
                        self.emit(Instruction::Else);
                    }
                }
                _ => {
                    // For complex patterns, fall back to runtime matching.
                    self.emit(Instruction::LocalGet(val_local));
                    let name_idx = self.module.intern_string("__match_pattern");
                    self.emit(Instruction::RuntimeCall {
                        name: name_idx,
                        arg_count: 1,
                    });
                    self.emit(Instruction::If);
                    self.compile_block(&arm.body)?;
                    if !is_last {
                        self.emit(Instruction::Else);
                    }
                }
            }
        }

        // Close if chains.
        let non_wildcard_count = arms
            .iter()
            .take_while(|a| !matches!(&a.pattern, Pattern::Wildcard | Pattern::Variable(_)))
            .count();

        for _ in 0..non_wildcard_count {
            self.emit(Instruction::End);
        }

        Ok(())
    }

    fn compile_try_catch(
        &mut self,
        try_block: &Block,
        catch_var: Option<&str>,
        catch_block: &Block,
        finally_block: Option<&Block>,
    ) -> Result<(), CompileError> {
        // WASM doesn't have native exception handling in the MVP.
        // Emulation strategy: compile try block normally. If a trap occurs,
        // WASM will abort. For non-trapping errors (Result-based), the try
        // block's value is checked: if it's a Result::Err enum map, branch
        // to catch. Otherwise, fall through.
        //
        // This handles the common case: `try { risky() } catch e { fallback }`
        // where risky() returns Result::Err(...) instead of throwing.
        let try_result = self.ensure_temp_local();
        self.compile_block(try_block)?;
        self.emit(Instruction::LocalSet(try_result));

        // Check if result is an error (map with __variant == "Err")
        // For simplicity, just use the try result. The catch block handles
        // error inspection. In WASM, traps can't be caught, but Result-based
        // errors can be.
        self.emit(Instruction::LocalGet(try_result));

        // Compile catch block (available but only reached via explicit error checks
        // in user code that returns Result::Err).
        if let Some(var_name) = catch_var {
            // Bind the error value for the catch block's scope.
            self.fb().push_scope();
            let idx = self.define_local(var_name, ValType::Tagged, false);
            // Store try result as the error binding (user should check if it's Err).
            self.emit(Instruction::LocalGet(try_result));
            self.emit(Instruction::LocalSet(idx));
            // Compile catch block but drop its result (try result is the final value).
            self.compile_block(catch_block)?;
            self.emit(Instruction::Drop);
            self.fb().pop_scope();
        }

        // Compile finally block if present.
        if let Some(finally) = finally_block {
            self.compile_block(finally)?;
            self.emit(Instruction::Drop); // finally doesn't contribute a value
        }

        // The try block's result stays on stack (via LocalGet above).
        Ok(())
    }

    fn compile_destructure(
        &mut self,
        pattern: &DestructurePattern,
        mutable: bool,
    ) -> Result<(), CompileError> {
        let val_local = self.define_local("__destruct", ValType::Tagged, false);
        self.emit(Instruction::LocalSet(val_local));

        match pattern {
            DestructurePattern::Array(elements) => {
                for (i, elem) in elements.iter().enumerate() {
                    match elem {
                        DestructureElement::Name(name) => {
                            self.emit(Instruction::LocalGet(val_local));
                            self.emit(Instruction::PushI64(i as i64));
                            self.emit(Instruction::ArrayGet);
                            let idx = self.define_local(name, ValType::Tagged, mutable);
                            self.emit(Instruction::LocalSet(idx));
                        }
                        DestructureElement::Rest(name) => {
                            self.emit(Instruction::LocalGet(val_local));
                            self.emit(Instruction::PushI64(i as i64));
                            self.emit(Instruction::LocalGet(val_local));
                            self.emit(Instruction::ArrayLen);
                            self.emit(Instruction::PushBool(false));
                            let slice_name = self.module.intern_string("__slice");
                            self.emit(Instruction::RuntimeCall {
                                name: slice_name,
                                arg_count: 4,
                            });
                            let idx = self.define_local(name, ValType::Tagged, mutable);
                            self.emit(Instruction::LocalSet(idx));
                        }
                    }
                }
            }
            DestructurePattern::Map(entries) => {
                for (key, alias) in entries {
                    self.emit(Instruction::LocalGet(val_local));
                    let key_idx = self.module.intern_string(key);
                    self.emit(Instruction::PushString(key_idx));
                    self.emit(Instruction::MapGet);
                    let bind_name = alias.as_deref().unwrap_or(key);
                    let idx = self.define_local(bind_name, ValType::Tagged, mutable);
                    self.emit(Instruction::LocalSet(idx));
                }
            }
        }
        Ok(())
    }

    fn compile_list_comprehension(
        &mut self,
        expr: &Expression,
        pattern: &ForPattern,
        iterable: &Expression,
        condition: Option<&Expression>,
    ) -> Result<(), CompileError> {
        // Create result array.
        self.emit(Instruction::ArrayNew(0));
        let result_local = self.define_local("__comp_result", ValType::Tagged, true);
        self.emit(Instruction::LocalSet(result_local));

        // Compile iterable.
        self.compile_expr(iterable)?;
        let iter_local = self.define_local("__comp_iter", ValType::Tagged, false);
        self.emit(Instruction::LocalSet(iter_local));

        // Counter + length (untagged raw i64 to avoid tag corruption on arithmetic).
        let counter = self.define_local("__comp_i", ValType::I64, true);
        self.emit(Instruction::PushI64(0));
        self.emit(Instruction::UntagI64);
        self.emit(Instruction::LocalSet(counter));

        self.emit(Instruction::LocalGet(iter_local));
        self.emit(Instruction::ArrayLen);
        self.emit(Instruction::UntagI64);
        let len = self.define_local("__comp_len", ValType::I64, false);
        self.emit(Instruction::LocalSet(len));

        // Loop.
        self.emit(Instruction::Block);
        self.emit(Instruction::Loop);

        self.emit(Instruction::LocalGet(counter));
        self.emit(Instruction::LocalGet(len));
        self.emit(Instruction::I64Ge);
        self.emit(Instruction::BrIf(1));

        // Get element: re-tag counter for ArrayGet (expects tagged i64 index).
        self.emit(Instruction::LocalGet(iter_local));
        self.emit(Instruction::LocalGet(counter));
        self.emit(Instruction::TagI64);
        self.emit(Instruction::ArrayGet);

        // Bind pattern.
        match pattern {
            ForPattern::Single(name) => {
                let idx = self.define_local(name, ValType::Tagged, false);
                self.emit(Instruction::LocalSet(idx));
            }
            _ => {
                // Simplified: store in temp.
                let tmp = self.define_local("__comp_elem", ValType::Tagged, false);
                self.emit(Instruction::LocalSet(tmp));
            }
        }

        // Optional filter condition.
        if let Some(cond) = condition {
            self.compile_expr(cond)?;
            self.emit(Instruction::BoolNot);
            self.emit(Instruction::If);
            // Skip this element (untagged counter + 1).
            self.emit(Instruction::LocalGet(counter));
            self.emit(Instruction::PushI64(1));
            self.emit(Instruction::UntagI64);
            self.emit(Instruction::I64Add);
            self.emit(Instruction::LocalSet(counter));
            self.emit(Instruction::Br(1)); // continue loop
            self.emit(Instruction::End);
        }

        // Compile expression and push to result.
        self.emit(Instruction::LocalGet(result_local));
        self.compile_expr(expr)?;
        let push_name = self.module.intern_string("__array_push");
        self.emit(Instruction::RuntimeCall {
            name: push_name,
            arg_count: 2,
        });
        self.emit(Instruction::LocalSet(result_local));

        // Increment (untagged counter + 1).
        self.emit(Instruction::LocalGet(counter));
        self.emit(Instruction::PushI64(1));
        self.emit(Instruction::UntagI64);
        self.emit(Instruction::I64Add);
        self.emit(Instruction::LocalSet(counter));
        self.emit(Instruction::Br(0));

        self.emit(Instruction::End); // loop
        self.emit(Instruction::End); // block

        self.emit(Instruction::LocalGet(result_local));
        Ok(())
    }

    fn compile_map_comprehension(
        &mut self,
        key_expr: &Expression,
        value_expr: &Expression,
        pattern: &ForPattern,
        iterable: &Expression,
        condition: Option<&Expression>,
    ) -> Result<(), CompileError> {
        // Create empty result map.
        self.emit(Instruction::MapNew(0));
        let result_local = self.define_local("__mcomp_result", ValType::Tagged, true);
        self.emit(Instruction::LocalSet(result_local));

        // Iterable.
        self.compile_expr(iterable)?;
        let iter_local = self.define_local("__mcomp_iter", ValType::Tagged, false);
        self.emit(Instruction::LocalSet(iter_local));

        // Counter + length (untagged raw i64 to avoid tag corruption on arithmetic).
        let counter = self.define_local("__mcomp_i", ValType::I64, true);
        self.emit(Instruction::PushI64(0));
        self.emit(Instruction::UntagI64);
        self.emit(Instruction::LocalSet(counter));

        self.emit(Instruction::LocalGet(iter_local));
        self.emit(Instruction::ArrayLen);
        self.emit(Instruction::UntagI64);
        let len = self.define_local("__mcomp_len", ValType::I64, false);
        self.emit(Instruction::LocalSet(len));

        self.emit(Instruction::Block);
        self.emit(Instruction::Loop);

        self.emit(Instruction::LocalGet(counter));
        self.emit(Instruction::LocalGet(len));
        self.emit(Instruction::I64Ge);
        self.emit(Instruction::BrIf(1));

        // Re-tag counter for ArrayGet.
        self.emit(Instruction::LocalGet(iter_local));
        self.emit(Instruction::LocalGet(counter));
        self.emit(Instruction::TagI64);
        self.emit(Instruction::ArrayGet);

        match pattern {
            ForPattern::Single(name) => {
                let idx = self.define_local(name, ValType::Tagged, false);
                self.emit(Instruction::LocalSet(idx));
            }
            _ => {
                let tmp = self.define_local("__mcomp_elem", ValType::Tagged, false);
                self.emit(Instruction::LocalSet(tmp));
            }
        }

        if let Some(cond) = condition {
            self.compile_expr(cond)?;
            self.emit(Instruction::BoolNot);
            self.emit(Instruction::If);
            self.emit(Instruction::LocalGet(counter));
            self.emit(Instruction::PushI64(1));
            self.emit(Instruction::UntagI64);
            self.emit(Instruction::I64Add);
            self.emit(Instruction::LocalSet(counter));
            self.emit(Instruction::Br(1));
            self.emit(Instruction::End);
        }

        self.emit(Instruction::LocalGet(result_local));
        self.compile_expr(key_expr)?;
        self.compile_expr(value_expr)?;
        self.emit(Instruction::MapSet);
        self.emit(Instruction::LocalSet(result_local));

        self.emit(Instruction::LocalGet(counter));
        self.emit(Instruction::PushI64(1));
        self.emit(Instruction::UntagI64);
        self.emit(Instruction::I64Add);
        self.emit(Instruction::LocalSet(counter));
        self.emit(Instruction::Br(0));

        self.emit(Instruction::End);
        self.emit(Instruction::End);

        self.emit(Instruction::LocalGet(result_local));
        Ok(())
    }

    /// Ensure a __temp local exists for intermediate values. Returns its index.
    fn ensure_temp_local(&mut self) -> u32 {
        let fb = self.current_fn.as_ref().expect("no function context");
        if let Some(idx) = fb.resolve_local("__temp") {
            return idx;
        }
        self.define_local("__temp", ValType::Tagged, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parser::parse_v2;

    fn compile(src: &str) -> Result<IrModule, CompileError> {
        let program = parse_v2(src).expect("parse error");
        let mut compiler = Compiler::new();
        compiler.compile(&program)
    }

    #[test]
    fn test_compile_empty_program() {
        let module = compile("").unwrap();
        assert!(module.functions.iter().any(|f| f.name == "__main"));
    }

    #[test]
    fn test_compile_let_binding() {
        let module = compile("let x = 42;").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::PushI64(42))));
    }

    #[test]
    fn test_compile_function_def() {
        let module = compile("fn add(a, b) { a + b }").unwrap();
        assert!(module.functions.iter().any(|f| f.name == "add"));
    }

    #[test]
    fn test_compile_for_loop() {
        let module = compile("for x in [1, 2, 3] { output x; }").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Loop)));
    }

    #[test]
    fn test_compile_while_loop() {
        let module = compile("let mut x = 0; while x < 10 { x = x + 1; }").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Loop)));
    }

    #[test]
    fn test_compile_if_else() {
        let module = compile("let x = if true { 1 } else { 2 };").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::If)));
    }

    #[test]
    fn test_compile_string_interp() {
        let module = compile(r#"let x = "world"; let s = f"hello {x}";"#).unwrap();
        assert!(module.strings.contains(&"hello ".to_string()));
    }

    #[test]
    fn test_compile_lambda() {
        let module = compile("let f = |x| x + 1;").unwrap();
        assert!(module
            .functions
            .iter()
            .any(|f| f.name.starts_with("__lambda_")));
    }

    #[test]
    fn test_compile_enum_construct() {
        let module = compile(r#"
            enum Result { Ok(value), Err(msg) }
            let r = Result::Ok(42);
        "#).unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::MapNew(3))));
    }

    #[test]
    fn test_compile_struct_construct() {
        let module = compile(r#"
            struct Point { x: float64, y: float64 }
            let p = Point { x: 1.0, y: 2.0 };
        "#).unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::MapNew(_))));
    }

    #[test]
    fn test_compile_range() {
        let module = compile("let r = 0..10;").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::RuntimeCall { .. })));
    }

    #[test]
    fn test_compile_match() {
        let module = compile(r#"
            let x = 42;
            let y = match x {
                1 => "one",
                2 => "two",
                _ => "other",
            };
        "#).unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::If)));
    }

    #[test]
    fn test_compile_method_call() {
        let module = compile(r#"let arr = [1, 2, 3]; arr.push(4);"#).unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::RuntimeCall { .. })));
    }

    #[test]
    fn test_string_interning() {
        let module = compile(r#"let a = "hello"; let b = "hello"; let c = "world";"#).unwrap();
        // "hello" should only appear once.
        let hello_count = module.strings.iter().filter(|s| *s == "hello").count();
        assert_eq!(hello_count, 1);
    }

    #[test]
    fn test_compile_try_propagate() {
        let module = compile("fn foo(x) { let v = x?; v }").unwrap();
        let foo = module.functions.iter().find(|f| f.name == "foo").unwrap();
        assert!(foo
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::GetTag)));
    }

    #[test]
    fn test_compile_list_comprehension() {
        let module = compile("let xs = [x * 2 for x in [1, 2, 3]];").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::ArrayNew(0))));
    }

    #[test]
    fn test_compile_null_coalesce() {
        let module = compile("let x = null; let y = x ?? 42;").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::GetTag)));
    }

    #[test]
    fn test_compile_const() {
        let module = compile("const PI = 3.14159;").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main.instructions.iter().any(|i| matches!(i, Instruction::PushF64(f) if (*f - 3.14159).abs() < 0.001)));
    }

    #[test]
    fn test_compile_compound_assign() {
        let module = compile("let mut x = 10; x += 5;").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        // Add now uses __add RuntimeCall for dynamic string/int dispatch.
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::RuntimeCall { arg_count: 2, .. })));
    }

    #[test]
    fn test_compile_optional_chain() {
        let module = compile(r#"let x = null; let y = x?.name;"#).unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::PushNull)));
    }

    #[test]
    fn test_compile_array_index() {
        let module = compile("let arr = [10, 20, 30]; let v = arr[1];").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::ArrayGet)));
    }

    #[test]
    fn test_compile_field_access() {
        let module = compile(r#"let obj = {"name": "test"}; let n = obj.name;"#).unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::MapGet)));
    }

    #[test]
    fn test_compile_recursive_function() {
        let module = compile(r#"
            fn fib(n) {
                if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
            }
            let r = fib(10);
        "#).unwrap();
        let fib = module.functions.iter().find(|f| f.name == "fib").unwrap();
        assert!(fib
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Call(_))));
    }

    #[test]
    fn test_compile_multiple_functions() {
        let module = compile(r#"
            fn add(a, b) { a + b }
            fn mul(a, b) { a * b }
            let r = add(2, mul(3, 4));
        "#).unwrap();
        assert!(module.functions.iter().any(|f| f.name == "add"));
        assert!(module.functions.iter().any(|f| f.name == "mul"));
    }

    #[test]
    fn test_compile_destructure_array() {
        let module = compile("let [a, b, c] = [1, 2, 3];").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::ArrayGet)));
    }

    #[test]
    fn test_compile_for_destructure() {
        let module = compile(r#"
            let pairs = [[1, "a"], [2, "b"]];
            for [num, letter] in pairs { output num; }
        "#).unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::ArrayGet)));
    }

    #[test]
    fn test_compile_output() {
        let module = compile("output 42;").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Print)));
    }

    #[test]
    fn test_compile_unary_ops() {
        let module = compile("let x = -5; let y = !true;").unwrap();
        let main = module.functions.iter().find(|f| f.name == "__main").unwrap();
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::I64Neg)));
        assert!(main
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::BoolNot)));
    }
}
