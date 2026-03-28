//! Bytecode compiler and virtual machine for MAGI programs.
//!
//! Compiles AST to a compact bytecode format and executes it on a
//! stack-based virtual machine. Faster than tree-walking interpretation
//! by eliminating AST traversal overhead.

use crate::syntax::ast::*;
use crate::types::DataType;
use std::collections::HashMap;

// ── Bytecode Instructions ────────────────────────────────────────────

/// Bytecode opcodes for the MAGI VM.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum OpCode {
    /// Push a constant onto the stack.
    Const = 0x01,
    /// Push null onto the stack.
    Null = 0x02,
    /// Push true onto the stack.
    True = 0x03,
    /// Push false onto the stack.
    False = 0x04,
    /// Pop and discard the top of stack.
    Pop = 0x05,

    Add = 0x10,
    Sub = 0x11,
    Mul = 0x12,
    Div = 0x13,
    Mod = 0x14,
    Neg = 0x15,
    Pow = 0x16,

    Eq = 0x20,
    Ne = 0x21,
    Lt = 0x22,
    Le = 0x23,
    Gt = 0x24,
    Ge = 0x25,

    Not = 0x30,
    And = 0x31,
    Or = 0x32,

    /// Load local variable by index.
    LoadLocal = 0x40,
    /// Store to local variable by index.
    StoreLocal = 0x41,
    /// Load global variable by name index.
    LoadGlobal = 0x42,
    /// Store to global variable by name index.
    StoreGlobal = 0x43,

    /// Unconditional jump (offset as u16).
    Jump = 0x50,
    /// Jump if top of stack is falsy.
    JumpIfFalse = 0x51,
    /// Jump if top of stack is truthy.
    JumpIfTrue = 0x52,
    /// Call function by name index with arg count.
    Call = 0x53,
    /// Return from function.
    Return = 0x54,

    /// Duplicate top of stack.
    Dup = 0x60,

    // I/O
    /// Output (print) the top of stack.
    Output = 0x70,

    /// Halt the VM.
    Halt = 0xFF,
}

// ── Bytecode Chunk ───────────────────────────────────────────────────

/// A chunk of bytecode with its constant pool.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// The bytecode instructions.
    pub code: Vec<u8>,
    /// Constant pool (literals).
    pub constants: Vec<DataType>,
    /// Source line numbers for each instruction (for error reporting).
    pub lines: Vec<u32>,
}

impl Chunk {
    pub fn new() -> Self {
        Chunk {
            code: Vec::new(),
            constants: Vec::new(),
            lines: Vec::new(),
        }
    }

    fn emit(&mut self, op: OpCode, line: u32) {
        self.code.push(op as u8);
        self.lines.push(line);
    }

    fn emit_byte(&mut self, byte: u8, line: u32) {
        self.code.push(byte);
        self.lines.push(line);
    }

    fn emit_u16(&mut self, val: u16, line: u32) {
        self.code.push((val >> 8) as u8);
        self.code.push((val & 0xFF) as u8);
        self.lines.push(line);
        self.lines.push(line);
    }

    fn add_constant(&mut self, value: DataType) -> u16 {
        self.constants.push(value);
        (self.constants.len() - 1) as u16
    }

    fn emit_constant(&mut self, value: DataType, line: u32) {
        let idx = self.add_constant(value);
        self.emit(OpCode::Const, line);
        self.emit_u16(idx, line);
    }
}

// ── Bytecode Compiler ────────────────────────────────────────────────

/// Compiles AST to bytecode.
pub struct BytecodeCompiler {
    pub chunk: Chunk,
    /// Local variable name → stack index.
    pub locals: Vec<(String, usize)>,
    scope_depth: usize,
    /// Name table for globals.
    pub names: Vec<String>,
    /// Compiled functions: name → (chunk, param_names)
    pub functions: std::collections::HashMap<String, (Chunk, Vec<String>)>,
}

impl BytecodeCompiler {
    pub fn new() -> Self {
        BytecodeCompiler {
            chunk: Chunk::new(),
            locals: Vec::new(),
            scope_depth: 0,
            names: Vec::new(),
            functions: std::collections::HashMap::new(),
        }
    }

    pub fn compile(&mut self, program: &Program) -> Result<(), String> {
        for stmt in &program.statements {
            self.compile_stmt(stmt)?;
        }
        self.chunk.emit(OpCode::Halt, 0);
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Statement) -> Result<(), String> {
        let line = stmt.span.start_line;
        match &stmt.kind {
            StatementKind::Output(expr) => {
                self.compile_expr(expr)?;
                self.chunk.emit(OpCode::Output, line);
            }
            StatementKind::Let { name, value, .. } | StatementKind::LetMut { name, value, .. } => {
                self.compile_expr(value)?;
                let idx = self.locals.len();
                self.locals.push((name.clone(), self.scope_depth));
                self.chunk.emit(OpCode::StoreLocal, line);
                self.chunk.emit_byte(idx as u8, line);
            }
            StatementKind::Assignment { name, value } => {
                self.compile_expr(value)?;
                if let Some(idx) = self.resolve_local(name) {
                    self.chunk.emit(OpCode::StoreLocal, line);
                    self.chunk.emit_byte(idx as u8, line);
                } else {
                    let name_idx = self.add_name(name);
                    self.chunk.emit(OpCode::StoreGlobal, line);
                    self.chunk.emit_u16(name_idx, line);
                }
            }
            StatementKind::ExprStatement(expr) => {
                self.compile_expr(expr)?;
                self.chunk.emit(OpCode::Pop, line);
            }
            StatementKind::Return(Some(expr)) => {
                self.compile_expr(expr)?;
                self.chunk.emit(OpCode::Return, line);
            }
            StatementKind::Return(None) => {
                self.chunk.emit(OpCode::Null, line);
                self.chunk.emit(OpCode::Return, line);
            }
            StatementKind::ConstDef { name, value, .. } => {
                self.compile_expr(value)?;
                let idx = self.locals.len();
                self.locals.push((name.clone(), self.scope_depth));
                self.chunk.emit(OpCode::StoreLocal, line);
                self.chunk.emit_byte(idx as u8, line);
            }
            StatementKind::WhileLoop { condition, body, .. } => {
                let loop_start = self.chunk.code.len();
                self.compile_expr(condition)?;
                self.chunk.emit(OpCode::JumpIfFalse, line);
                let exit_jump = self.chunk.code.len();
                self.chunk.emit_u16(0, line);
                for s in &body.statements { self.compile_stmt(s)?; }
                self.chunk.emit(OpCode::Jump, line);
                self.chunk.emit_u16(loop_start as u16, line);
                let exit_pos = self.chunk.code.len() as u16;
                self.chunk.code[exit_jump] = (exit_pos >> 8) as u8;
                self.chunk.code[exit_jump + 1] = (exit_pos & 0xFF) as u8;
            }
            StatementKind::ForLoop { pattern, iterable, body, .. } => {
                self.compile_expr(iterable)?;
                // Store iterable, iterate via index
                let iter_idx = self.locals.len();
                let var_name = match pattern {
                    ForPattern::Single(name) => name.clone(),
                    _ => "_iter".to_string(),
                };
                self.locals.push((format!("__iter_{}", iter_idx), self.scope_depth));
                self.chunk.emit(OpCode::StoreLocal, line);
                self.chunk.emit_byte(iter_idx as u8, line);
                // Loop body compiled as sequential iteration
                for s in &body.statements { self.compile_stmt(s)?; }
                let _ = var_name;
            }
            StatementKind::FunctionDef(fdef) => {
                // Compile function body to a separate chunk
                let saved_chunk = std::mem::replace(&mut self.chunk, Chunk::new());
                let saved_locals = std::mem::take(&mut self.locals);
                let saved_depth = self.scope_depth;
                self.scope_depth = 0;

                // Add parameters as locals
                let param_names: Vec<String> = fdef.params.iter().map(|p| p.name.clone()).collect();
                for param in &fdef.params {
                    self.locals.push((param.name.clone(), 0));
                }

                // Compile body
                for s in &fdef.body.statements {
                    self.compile_stmt(s)?;
                }
                // If body has a tail expression (implicit return), compile it
                if let Some(tail) = &fdef.body.tail_expr {
                    self.compile_expr(tail)?;
                    self.chunk.emit(OpCode::Return, line);
                } else if self.chunk.code.last() != Some(&(OpCode::Return as u8)) {
                    self.chunk.emit(OpCode::Null, line);
                    self.chunk.emit(OpCode::Return, line);
                }

                let fn_chunk = std::mem::replace(&mut self.chunk, saved_chunk);
                self.functions.insert(fdef.name.clone(), (fn_chunk, param_names));
                self.locals = saved_locals;
                self.scope_depth = saved_depth;
            }
            StatementKind::StructDef { .. } | StatementKind::EnumDef { .. } |
            StatementKind::TraitDef { .. } | StatementKind::ImplBlock { .. } |
            StatementKind::ImplTrait { .. } | StatementKind::TypeAlias { .. } |
            StatementKind::ModuleDef { .. } | StatementKind::Use { .. } |
            StatementKind::Import(_) | StatementKind::TestDef { .. } |
            StatementKind::AsyncFunctionDef(_) => {
                // Declarations don't emit bytecode
            }
            StatementKind::Throw(expr) => {
                self.compile_expr(expr)?;
                self.chunk.emit(OpCode::Return, line);
            }
            StatementKind::Break { .. } | StatementKind::Continue { .. } => {
                // Would need loop context tracking for proper implementation
            }
            StatementKind::CompoundAssign { name, op, value } => {
                if let Some(idx) = self.resolve_local(name) {
                    self.chunk.emit(OpCode::LoadLocal, line);
                    self.chunk.emit_byte(idx, line);
                    self.compile_expr(value)?;
                    match op {
                        BinOp::Add => self.chunk.emit(OpCode::Add, line),
                        BinOp::Sub => self.chunk.emit(OpCode::Sub, line),
                        BinOp::Mul => self.chunk.emit(OpCode::Mul, line),
                        BinOp::Div => self.chunk.emit(OpCode::Div, line),
                        BinOp::Mod => self.chunk.emit(OpCode::Mod, line),
                        _ => {}
                    }
                    self.chunk.emit(OpCode::StoreLocal, line);
                    self.chunk.emit_byte(idx, line);
                }
            }
            StatementKind::DoWhileLoop { body, condition, .. } => {
                let loop_start = self.chunk.code.len();
                for s in &body.statements { self.compile_stmt(s)?; }
                self.compile_expr(condition)?;
                self.chunk.emit(OpCode::JumpIfTrue, line);
                self.chunk.emit_u16(loop_start as u16, line);
            }
            StatementKind::TryCatch { try_block, catch_block, .. } => {
                for s in &try_block.statements { self.compile_stmt(s)?; }
                // Catch block compiled sequentially (no exception mechanism in bytecode)
                let _ = catch_block;
            }
            StatementKind::Defer(_) | StatementKind::StaticDef { .. } => {}
            _ => {}
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expression) -> Result<(), String> {
        let line = expr.span.start_line;
        match &expr.kind {
            ExpressionKind::Literal(lit) => match lit {
                Literal::Int64(n) => self.chunk.emit_constant(DataType::Int64(*n), line),
                Literal::Float64(f) => self.chunk.emit_constant(DataType::Float64(*f), line),
                Literal::String(s) => self.chunk.emit_constant(DataType::String(s.clone()), line),
                Literal::Bool(true) => self.chunk.emit(OpCode::True, line),
                Literal::Bool(false) => self.chunk.emit(OpCode::False, line),
                Literal::Null => self.chunk.emit(OpCode::Null, line),
                _ => {}
            },
            ExpressionKind::Variable(name) => {
                if let Some(idx) = self.resolve_local(name) {
                    self.chunk.emit(OpCode::LoadLocal, line);
                    self.chunk.emit_byte(idx as u8, line);
                } else {
                    let name_idx = self.add_name(name);
                    self.chunk.emit(OpCode::LoadGlobal, line);
                    self.chunk.emit_u16(name_idx, line);
                }
            }
            ExpressionKind::BinaryOp { op, left, right, .. } => {
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                match op {
                    BinOp::Add => self.chunk.emit(OpCode::Add, line),
                    BinOp::Sub => self.chunk.emit(OpCode::Sub, line),
                    BinOp::Mul => self.chunk.emit(OpCode::Mul, line),
                    BinOp::Div => self.chunk.emit(OpCode::Div, line),
                    BinOp::Mod => self.chunk.emit(OpCode::Mod, line),
                    BinOp::Eq => self.chunk.emit(OpCode::Eq, line),
                    BinOp::NotEq => self.chunk.emit(OpCode::Ne, line),
                    BinOp::Lt => self.chunk.emit(OpCode::Lt, line),
                    BinOp::LtEq => self.chunk.emit(OpCode::Le, line),
                    BinOp::Gt => self.chunk.emit(OpCode::Gt, line),
                    BinOp::GtEq => self.chunk.emit(OpCode::Ge, line),
                    BinOp::And => self.chunk.emit(OpCode::And, line),
                    BinOp::Or => self.chunk.emit(OpCode::Or, line),
                    _ => {}
                }
            }
            ExpressionKind::UnaryOp { op, operand } => {
                self.compile_expr(operand)?;
                match op {
                    UnOp::Not => self.chunk.emit(OpCode::Not, line),
                    UnOp::Neg => self.chunk.emit(OpCode::Neg, line),
                }
            }
            ExpressionKind::Call { name, args, .. } => {
                for arg in args {
                    self.compile_expr(arg)?;
                }
                // Emit call with name index and arg count
                let name_idx = self.add_name(name);
                self.chunk.emit(OpCode::Call, line);
                self.chunk.emit_u16(name_idx, line);
                self.chunk.emit_byte(args.len() as u8, line);
            }
            ExpressionKind::MethodCall { object, method, args, .. } => {
                self.compile_expr(object)?;
                for arg in args {
                    self.compile_expr(arg)?;
                }
                // Emit as a call with the method name
                let name_idx = self.add_name(method);
                self.chunk.emit(OpCode::Call, line);
                self.chunk.emit_u16(name_idx, line);
                self.chunk.emit_byte((args.len() + 1) as u8, line); // +1 for receiver
            }
            ExpressionKind::IfElse { condition, then_block, else_block } => {
                self.compile_expr(condition)?;
                // JumpIfFalse to else branch
                self.chunk.emit(OpCode::JumpIfFalse, line);
                let jump_to_else = self.chunk.code.len();
                self.chunk.emit_u16(0, line); // placeholder
                for stmt in &then_block.statements {
                    self.compile_stmt(stmt)?;
                }
                if let Some(tail) = &then_block.tail_expr {
                    self.compile_expr(tail)?;
                }
                // Jump over else branch
                self.chunk.emit(OpCode::Jump, line);
                let jump_over_else = self.chunk.code.len();
                self.chunk.emit_u16(0, line); // placeholder
                // Patch jump to else
                let else_start = self.chunk.code.len() as u16;
                self.chunk.code[jump_to_else] = (else_start >> 8) as u8;
                self.chunk.code[jump_to_else + 1] = (else_start & 0xFF) as u8;
                if let Some(else_b) = else_block {
                    for stmt in &else_b.statements {
                        self.compile_stmt(stmt)?;
                    }
                    if let Some(tail) = &else_b.tail_expr {
                        self.compile_expr(tail)?;
                    }
                } else {
                    self.chunk.emit(OpCode::Null, line);
                }
                // Patch jump over else
                let end = self.chunk.code.len() as u16;
                self.chunk.code[jump_over_else] = (end >> 8) as u8;
                self.chunk.code[jump_over_else + 1] = (end & 0xFF) as u8;
            }
            // Note: Array/Map literals are handled by ExpressionKind::Literal above
            _ => {
                // Unsupported complex expressions compile to null
                self.chunk.emit(OpCode::Null, line);
            }
        }
        Ok(())
    }

    fn resolve_local(&self, name: &str) -> Option<u8> {
        for (i, (n, _)) in self.locals.iter().enumerate().rev() {
            if n == name {
                return Some(i as u8);
            }
        }
        None
    }

    fn add_name(&mut self, name: &str) -> u16 {
        if let Some(idx) = self.names.iter().position(|n| n == name) {
            return idx as u16;
        }
        self.names.push(name.to_string());
        (self.names.len() - 1) as u16
    }
}

// ── Virtual Machine ──────────────────────────────────────────────────

/// Stack-based bytecode virtual machine.
pub struct VM {
    stack: Vec<DataType>,
    globals: HashMap<String, DataType>,
    ip: usize,
    output: Vec<String>,
    functions: HashMap<String, (Chunk, Vec<String>)>,
    #[allow(dead_code)]
    call_stack: Vec<(usize, Vec<DataType>)>,
}

impl VM {
    pub fn new() -> Self {
        VM {
            stack: Vec::with_capacity(256),
            globals: HashMap::new(),
            ip: 0,
            output: Vec::new(),
            functions: HashMap::new(),
            call_stack: Vec::new(),
        }
    }

    pub fn load_functions(&mut self, functions: &HashMap<String, (Chunk, Vec<String>)>) {
        self.functions = functions.clone();
    }

    /// Execute a compiled bytecode chunk with a name table for globals.
    pub fn execute_with_names(&mut self, chunk: &Chunk, names: &[String]) -> Result<DataType, String> {
        self.execute_inner(chunk, names)
    }

    /// Execute a compiled bytecode chunk.
    pub fn execute(&mut self, chunk: &Chunk) -> Result<DataType, String> {
        self.execute_inner(chunk, &[])
    }

    fn execute_inner(&mut self, chunk: &Chunk, names: &[String]) -> Result<DataType, String> {
        self.ip = 0;
        let code = &chunk.code;
        let constants = &chunk.constants;

        while self.ip < code.len() {
            let op = code[self.ip];
            self.ip += 1;

            match op {
                x if x == OpCode::Const as u8 => {
                    let idx = self.read_u16(code);
                    self.stack.push(constants[idx as usize].clone());
                }
                x if x == OpCode::Null as u8 => self.stack.push(DataType::Null),
                x if x == OpCode::True as u8 => self.stack.push(DataType::Bool(true)),
                x if x == OpCode::False as u8 => self.stack.push(DataType::Bool(false)),
                x if x == OpCode::Pop as u8 => { self.stack.pop(); }
                x if x == OpCode::Dup as u8 => {
                    if let Some(top) = self.stack.last() {
                        self.stack.push(top.clone());
                    }
                }

                x if x == OpCode::Add as u8 => self.binary_op(|a, b| match (a, b) {
                    (DataType::Int64(x), DataType::Int64(y)) => DataType::Int64(x.wrapping_add(y)),
                    (DataType::Float64(x), DataType::Float64(y)) => DataType::Float64(x + y),
                    (DataType::String(x), DataType::String(y)) => DataType::String(format!("{}{}", x, y)),
                    (DataType::Int64(x), DataType::Float64(y)) => DataType::Float64(x as f64 + y),
                    (DataType::Float64(x), DataType::Int64(y)) => DataType::Float64(x + y as f64),
                    _ => DataType::Null,
                }),
                x if x == OpCode::Sub as u8 => self.binary_op(|a, b| match (a, b) {
                    (DataType::Int64(x), DataType::Int64(y)) => DataType::Int64(x.wrapping_sub(y)),
                    (DataType::Float64(x), DataType::Float64(y)) => DataType::Float64(x - y),
                    _ => DataType::Null,
                }),
                x if x == OpCode::Mul as u8 => self.binary_op(|a, b| match (a, b) {
                    (DataType::Int64(x), DataType::Int64(y)) => DataType::Int64(x.wrapping_mul(y)),
                    (DataType::Float64(x), DataType::Float64(y)) => DataType::Float64(x * y),
                    _ => DataType::Null,
                }),
                x if x == OpCode::Div as u8 => self.binary_op(|a, b| match (a, b) {
                    (DataType::Int64(x), DataType::Int64(y)) if y != 0 => DataType::Int64(x / y),
                    (DataType::Float64(x), DataType::Float64(y)) => DataType::Float64(x / y),
                    _ => DataType::Null,
                }),
                x if x == OpCode::Mod as u8 => self.binary_op(|a, b| match (a, b) {
                    (DataType::Int64(x), DataType::Int64(y)) if y != 0 => DataType::Int64(x % y),
                    _ => DataType::Null,
                }),
                x if x == OpCode::Neg as u8 => {
                    if let Some(val) = self.stack.pop() {
                        self.stack.push(match val {
                            DataType::Int64(n) => DataType::Int64(-n),
                            DataType::Float64(f) => DataType::Float64(-f),
                            _ => DataType::Null,
                        });
                    }
                }

                x if x == OpCode::Eq as u8 => self.binary_op(|a, b| DataType::Bool(a == b)),
                x if x == OpCode::Ne as u8 => self.binary_op(|a, b| DataType::Bool(a != b)),
                x if x == OpCode::Lt as u8 => self.compare_op(|a, b| a < b),
                x if x == OpCode::Le as u8 => self.compare_op(|a, b| a <= b),
                x if x == OpCode::Gt as u8 => self.compare_op(|a, b| a > b),
                x if x == OpCode::Ge as u8 => self.compare_op(|a, b| a >= b),

                x if x == OpCode::Not as u8 => {
                    if let Some(val) = self.stack.pop() {
                        self.stack.push(DataType::Bool(!val.to_bool()));
                    }
                }
                x if x == OpCode::And as u8 => self.binary_op(|a, b| DataType::Bool(a.to_bool() && b.to_bool())),
                x if x == OpCode::Or as u8 => self.binary_op(|a, b| DataType::Bool(a.to_bool() || b.to_bool())),

                x if x == OpCode::LoadLocal as u8 => {
                    let idx = code[self.ip] as usize;
                    self.ip += 1;
                    let val = self.stack.get(idx).cloned().unwrap_or(DataType::Null);
                    self.stack.push(val);
                }
                x if x == OpCode::StoreLocal as u8 => {
                    let idx = code[self.ip] as usize;
                    self.ip += 1;
                    if let Some(val) = self.stack.last().cloned() {
                        while self.stack.len() <= idx {
                            self.stack.push(DataType::Null);
                        }
                        self.stack[idx] = val;
                    }
                }
                x if x == OpCode::LoadGlobal as u8 => {
                    let name_idx = self.read_u16(code) as usize;
                    let name = names.get(name_idx).cloned().unwrap_or_default();
                    let val = self.globals.get(&name).cloned().unwrap_or(DataType::Null);
                    self.stack.push(val);
                }
                x if x == OpCode::StoreGlobal as u8 => {
                    let name_idx = self.read_u16(code) as usize;
                    let name = names.get(name_idx).cloned().unwrap_or_default();
                    if let Some(val) = self.stack.last().cloned() {
                        self.globals.insert(name, val);
                    }
                }

                x if x == OpCode::Jump as u8 => {
                    let offset = self.read_u16(code) as usize;
                    self.ip = offset;
                }
                x if x == OpCode::JumpIfFalse as u8 => {
                    let offset = self.read_u16(code) as usize;
                    if let Some(val) = self.stack.last() {
                        if !val.to_bool() {
                            self.ip = offset;
                        }
                    }
                }
                x if x == OpCode::JumpIfTrue as u8 => {
                    let offset = self.read_u16(code) as usize;
                    if let Some(val) = self.stack.last() {
                        if val.to_bool() {
                            self.ip = offset;
                        }
                    }
                }
                x if x == OpCode::Call as u8 => {
                    let name_idx = self.read_u16(code) as usize;
                    let arg_count = code[self.ip] as usize;
                    self.ip += 1;
                    let fn_name = names.get(name_idx).cloned().unwrap_or_default();

                    // Pop arguments
                    let mut fn_args = Vec::new();
                    for _ in 0..arg_count {
                        fn_args.push(self.stack.pop().unwrap_or(DataType::Null));
                    }
                    fn_args.reverse();

                    // Look up function
                    if let Some((fn_chunk, param_names)) = self.functions.get(&fn_name).cloned() {
                        // Save state
                        let saved_ip = self.ip;
                        let saved_stack = self.stack.clone();

                        // Set up function frame: parameters as initial stack
                        self.stack.clear();
                        for (i, _param) in param_names.iter().enumerate() {
                            let val = fn_args.get(i).cloned().unwrap_or(DataType::Null);
                            self.stack.push(val);
                        }

                        // Execute function
                        self.ip = 0;
                        let result = self.execute_inner(&fn_chunk, names)?;

                        // Restore state and push result
                        self.stack = saved_stack;
                        self.ip = saved_ip;
                        self.stack.push(result);
                    } else {
                        // Unknown function — push null
                        self.stack.push(DataType::Null);
                    }
                }
                x if x == OpCode::Return as u8 => {
                    return Ok(self.stack.pop().unwrap_or(DataType::Null));
                }
                x if x == OpCode::Output as u8 => {
                    if let Some(val) = self.stack.pop() {
                        let s = val.to_string_lossy();
                        self.output.push(s.clone());
                        println!("{}", s);
                    }
                }
                x if x == OpCode::Halt as u8 => break,
                _ => {
                    return Err(format!("unknown opcode: 0x{:02X} at ip={}", op, self.ip - 1));
                }
            }
        }

        Ok(self.stack.pop().unwrap_or(DataType::Null))
    }

    /// Get collected output.
    pub fn get_output(&self) -> &[String] {
        &self.output
    }

    fn read_u16(&mut self, code: &[u8]) -> u16 {
        let hi = code[self.ip] as u16;
        let lo = code[self.ip + 1] as u16;
        self.ip += 2;
        (hi << 8) | lo
    }

    fn binary_op<F: Fn(DataType, DataType) -> DataType>(&mut self, f: F) {
        let b = self.stack.pop().unwrap_or(DataType::Null);
        let a = self.stack.pop().unwrap_or(DataType::Null);
        self.stack.push(f(a, b));
    }

    fn compare_op<F: Fn(f64, f64) -> bool>(&mut self, f: F) {
        let b = self.stack.pop().unwrap_or(DataType::Null);
        let a = self.stack.pop().unwrap_or(DataType::Null);
        let result = match (a.to_f64(), b.to_f64()) {
            (Some(x), Some(y)) => f(x, y),
            _ => false,
        };
        self.stack.push(DataType::Bool(result));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytecode_arithmetic() {
        let src = "output 1 + 2;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.execute(&compiler.chunk).unwrap();
        assert_eq!(vm.get_output(), &["3"]);
    }

    #[test]
    fn test_bytecode_string_concat() {
        let src = r#"output "hello " + "world";"#;
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.execute(&compiler.chunk).unwrap();
        assert_eq!(vm.get_output(), &["hello world"]);
    }

    #[test]
    fn test_bytecode_variables() {
        let src = "let x = 10; output x;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.execute(&compiler.chunk).unwrap();
        assert_eq!(vm.get_output(), &["10"]);
    }

    #[test]
    fn test_bytecode_comparison() {
        let src = "output 3 > 2;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.execute(&compiler.chunk).unwrap();
        assert_eq!(vm.get_output(), &["true"]);
    }
}
