//! IR interpreter — executes MAGI IR directly.
//!
//! This is the bytecode VM backend. Instead of compiling IR to a separate
//! bytecode format, it interprets IR instructions directly on a stack machine.
//! This gives full language coverage since AST→IR (compile.rs) handles every
//! language construct.
//!
//! Architecture: AST → IR (compile.rs) → IrVm (this file)
//!               Same IR used by WASM and native backends.

use super::ir::*;
use crate::types::DataType;
use std::collections::HashMap;

/// A tagged runtime value using the same NaN-boxing scheme as the WASM backend.
#[derive(Debug, Clone, Copy)]
struct Val(i64);

impl Val {
    fn null() -> Self { Val(tag::encode(tag::NULL, 0)) }
    fn bool_(b: bool) -> Self { Val(tag::encode(tag::BOOL, if b { 1 } else { 0 })) }
    fn int(n: i64) -> Self { Val(tag::encode(tag::I64, n)) }
    fn float(f: f64) -> Self { Val(i64::from_ne_bytes(f.to_ne_bytes())) }
    fn string(idx: u32) -> Self { Val(tag::encode(tag::STRING, idx as i64)) }
    fn array(ptr: u32) -> Self { Val(tag::encode(tag::ARRAY, ptr as i64)) }
    fn map(ptr: u32) -> Self { Val(tag::encode(tag::MAP, ptr as i64)) }

    fn is_tagged(&self) -> bool { (self.0 & tag::NANBOX_MASK) == tag::NANBOX_SIG }
    fn tag(&self) -> u8 { if self.is_tagged() { ((self.0 >> tag::TAG_SHIFT) & 7) as u8 } else { 255 } }
    fn payload(&self) -> i64 { self.0 & tag::PAYLOAD_MASK }

    fn as_i64(&self) -> i64 {
        if self.is_tagged() && self.tag() == tag::I64 { self.payload() }
        else if !self.is_tagged() { f64::from_ne_bytes(self.0.to_ne_bytes()) as i64 }
        else { 0 }
    }
    fn as_f64(&self) -> f64 {
        if !self.is_tagged() { f64::from_ne_bytes(self.0.to_ne_bytes()) }
        else if self.tag() == tag::I64 { self.payload() as f64 }
        else { 0.0 }
    }
    fn as_bool(&self) -> bool {
        if self.is_tagged() {
            match self.tag() {
                tag::NULL => false,
                tag::BOOL => self.payload() != 0,
                tag::I64 => self.payload() != 0,
                _ => true,
            }
        } else {
            self.as_f64() != 0.0
        }
    }
    #[allow(dead_code)]
    fn to_data_type(&self, vm: &IrVm) -> DataType {
        if !self.is_tagged() {
            return DataType::Float64(self.as_f64());
        }
        match self.tag() {
            tag::NULL => DataType::Null,
            tag::BOOL => DataType::Bool(self.payload() != 0),
            tag::I64 => DataType::Int64(self.payload()),
            tag::STRING => {
                let idx = self.payload() as usize;
                DataType::String(vm.strings.get(idx).cloned().unwrap_or_default())
            }
            tag::ARRAY => {
                let ptr = self.payload() as usize;
                let arr = vm.heap_arrays.get(&ptr).cloned().unwrap_or_default();
                DataType::Array(arr.iter().map(|v| v.to_data_type(vm)).collect())
            }
            tag::MAP => {
                let ptr = self.payload() as usize;
                let entries = vm.heap_maps.get(&ptr).cloned().unwrap_or_default();
                let mut m = crate::util::OrderedMap::new();
                for (k, v) in &entries {
                    m.insert(k.clone(), v.to_data_type(vm));
                }
                DataType::Map(m)
            }
            _ => DataType::Null,
        }
    }
}

/// IR virtual machine — interprets IR instructions directly.
pub struct IrVm {
    /// Value stack.
    stack: Vec<Val>,
    /// Local variable slots per frame.
    locals: Vec<Vec<Val>>,
    /// Global variables.
    globals: Vec<Val>,
    /// String pool.
    strings: Vec<String>,
    /// Heap-allocated arrays: address → elements.
    heap_arrays: HashMap<usize, Vec<Val>>,
    /// Heap-allocated maps: address → entries.
    heap_maps: HashMap<usize, Vec<(String, Val)>>,
    /// Next heap address.
    next_heap: usize,
    /// Output lines.
    output: Vec<String>,
    /// Block/loop label stack for branch resolution.
    label_stack: Vec<LabelInfo>,
}

#[derive(Debug)]
enum LabelInfo {
    Block(#[allow(dead_code)] usize),
    Loop(usize),
}

impl IrVm {
    pub fn new() -> Self {
        IrVm {
            stack: Vec::with_capacity(1024),
            locals: Vec::new(),
            globals: Vec::new(),
            strings: Vec::new(),
            heap_arrays: HashMap::new(),
            heap_maps: HashMap::new(),
            next_heap: 1,
            output: Vec::new(),
            label_stack: Vec::new(),
        }
    }

    /// Execute an IR module. Returns the output lines.
    pub fn execute(&mut self, module: &IrModule) -> Result<Vec<String>, String> {
        self.strings = module.strings.clone();

        // Initialize globals
        self.globals = vec![Val::null(); module.globals.len()];
        for (i, g) in module.globals.iter().enumerate() {
            if !g.init.is_empty() {
                let val = self.eval_const_expr(&g.init)?;
                self.globals[i] = val;
            }
        }

        // Find __main function
        let main_idx = module.functions.iter().position(|f| f.name == "__main")
            .ok_or_else(|| "no __main function found".to_string())?;

        self.execute_function(module, main_idx, &[])?;
        Ok(self.output.clone())
    }

    fn execute_function(&mut self, module: &IrModule, fn_idx: usize, args: &[Val]) -> Result<Val, String> {
        let func = &module.functions[fn_idx];

        // Set up locals: params + declared locals
        let mut frame_locals = vec![Val::null(); func.locals.len()];
        for (i, arg) in args.iter().enumerate() {
            if i < frame_locals.len() {
                frame_locals[i] = *arg;
            }
        }
        self.locals.push(frame_locals);

        // Save label stack — function calls must not corrupt the caller's labels
        let saved_labels = std::mem::take(&mut self.label_stack);

        let result = self.execute_instructions(module, &func.instructions, fn_idx);

        // Restore label stack
        self.label_stack = saved_labels;
        self.locals.pop();
        result
    }

    fn execute_instructions(&mut self, module: &IrModule, instructions: &[Instruction], _fn_idx: usize) -> Result<Val, String> {
        let mut ip = 0;
        let mut step_count: u64 = 0;
        const MAX_STEPS: u64 = 100_000_000;
        while ip < instructions.len() {
            step_count += 1;
            if step_count > MAX_STEPS {
                return Err("execution limit exceeded (100M instructions)".into());
            }
            match &instructions[ip] {
                // Constants
                Instruction::PushNull => self.stack.push(Val::null()),
                Instruction::PushBool(b) => self.stack.push(Val::bool_(*b)),
                Instruction::PushI64(n) => self.stack.push(Val::int(*n)),
                Instruction::PushF64(f) => self.stack.push(Val::float(*f)),
                Instruction::PushI32(n) => self.stack.push(Val::int(*n as i64)),
                Instruction::PushF32(f) => self.stack.push(Val::float(*f as f64)),
                Instruction::PushString(idx) => self.stack.push(Val::string(*idx)),

                // Locals & Globals
                Instruction::LocalGet(idx) => {
                    let val = self.locals.last().and_then(|l| l.get(*idx as usize)).copied().unwrap_or(Val::null());
                    self.stack.push(val);
                }
                Instruction::LocalSet(idx) => {
                    let val = self.stack.pop().unwrap_or(Val::null());
                    if let Some(frame) = self.locals.last_mut() {
                        while frame.len() <= *idx as usize { frame.push(Val::null()); }
                        frame[*idx as usize] = val;
                    }
                }
                Instruction::LocalTee(idx) => {
                    let val = self.stack.last().copied().unwrap_or(Val::null());
                    if let Some(frame) = self.locals.last_mut() {
                        while frame.len() <= *idx as usize { frame.push(Val::null()); }
                        frame[*idx as usize] = val;
                    }
                }
                Instruction::GlobalGet(idx) => {
                    let val = self.globals.get(*idx as usize).copied().unwrap_or(Val::null());
                    self.stack.push(val);
                }
                Instruction::GlobalSet(idx) => {
                    let val = self.stack.pop().unwrap_or(Val::null());
                    while self.globals.len() <= *idx as usize { self.globals.push(Val::null()); }
                    self.globals[*idx as usize] = val;
                }

                // i64 arithmetic
                Instruction::I64Add => { let b = self.pop_i64(); let a = self.pop_i64(); self.stack.push(Val::int(a.wrapping_add(b))); }
                Instruction::I64Sub => { let b = self.pop_i64(); let a = self.pop_i64(); self.stack.push(Val::int(a.wrapping_sub(b))); }
                Instruction::I64Mul => { let b = self.pop_i64(); let a = self.pop_i64(); self.stack.push(Val::int(a.wrapping_mul(b))); }
                Instruction::I64Div => { let b = self.pop_i64(); let a = self.pop_i64(); self.stack.push(Val::int(if b != 0 { a / b } else { 0 })); }
                Instruction::I64Rem => { let b = self.pop_i64(); let a = self.pop_i64(); self.stack.push(Val::int(if b != 0 { a % b } else { 0 })); }
                Instruction::I64Neg => { let a = self.pop_i64(); self.stack.push(Val::int(-a)); }

                // f64 arithmetic
                Instruction::F64Add => { let b = self.pop_f64(); let a = self.pop_f64(); self.stack.push(Val::float(a + b)); }
                Instruction::F64Sub => { let b = self.pop_f64(); let a = self.pop_f64(); self.stack.push(Val::float(a - b)); }
                Instruction::F64Mul => { let b = self.pop_f64(); let a = self.pop_f64(); self.stack.push(Val::float(a * b)); }
                Instruction::F64Div => { let b = self.pop_f64(); let a = self.pop_f64(); self.stack.push(Val::float(a / b)); }
                Instruction::F64Neg => { let a = self.pop_f64(); self.stack.push(Val::float(-a)); }
                Instruction::F64Sqrt => { let a = self.pop_f64(); self.stack.push(Val::float(a.sqrt())); }
                Instruction::F64Floor => { let a = self.pop_f64(); self.stack.push(Val::float(a.floor())); }
                Instruction::F64Ceil => { let a = self.pop_f64(); self.stack.push(Val::float(a.ceil())); }
                Instruction::F64Abs => { let a = self.pop_f64(); self.stack.push(Val::float(a.abs())); }

                // i64 comparisons
                Instruction::I64Eq => { let b = self.pop_i64(); let a = self.pop_i64(); self.stack.push(Val::int(if a == b { 1 } else { 0 })); }
                Instruction::I64Ne => { let b = self.pop_i64(); let a = self.pop_i64(); self.stack.push(Val::int(if a != b { 1 } else { 0 })); }
                Instruction::I64Lt => { let b = self.pop_i64(); let a = self.pop_i64(); self.stack.push(Val::int(if a < b { 1 } else { 0 })); }
                Instruction::I64Gt => { let b = self.pop_i64(); let a = self.pop_i64(); self.stack.push(Val::int(if a > b { 1 } else { 0 })); }
                Instruction::I64Le => { let b = self.pop_i64(); let a = self.pop_i64(); self.stack.push(Val::int(if a <= b { 1 } else { 0 })); }
                Instruction::I64Ge => { let b = self.pop_i64(); let a = self.pop_i64(); self.stack.push(Val::int(if a >= b { 1 } else { 0 })); }

                // f64 comparisons
                Instruction::F64Eq => { let b = self.pop_f64(); let a = self.pop_f64(); self.stack.push(Val::int(if a == b { 1 } else { 0 })); }
                Instruction::F64Ne => { let b = self.pop_f64(); let a = self.pop_f64(); self.stack.push(Val::int(if a != b { 1 } else { 0 })); }
                Instruction::F64Lt => { let b = self.pop_f64(); let a = self.pop_f64(); self.stack.push(Val::int(if a < b { 1 } else { 0 })); }
                Instruction::F64Gt => { let b = self.pop_f64(); let a = self.pop_f64(); self.stack.push(Val::int(if a > b { 1 } else { 0 })); }
                Instruction::F64Le => { let b = self.pop_f64(); let a = self.pop_f64(); self.stack.push(Val::int(if a <= b { 1 } else { 0 })); }
                Instruction::F64Ge => { let b = self.pop_f64(); let a = self.pop_f64(); self.stack.push(Val::int(if a >= b { 1 } else { 0 })); }

                // Logical
                Instruction::BoolNot => { let a = self.stack.pop().unwrap_or(Val::null()); self.stack.push(Val::bool_(!a.as_bool())); }

                // Conversions
                Instruction::I64ToF64 => { let a = self.pop_i64(); self.stack.push(Val::float(a as f64)); }
                Instruction::F64ToI64 => { let a = self.pop_f64(); self.stack.push(Val::int(a as i64)); }

                // Tagged value operations
                Instruction::TagI64 => { /* value already tagged by Val::int */ }
                Instruction::TagF64 => { /* value already f64 bits */ }
                Instruction::TagBool => { /* already tagged */ }
                Instruction::TagString => { /* already tagged */ }
                Instruction::UntagI64 => { let v = self.stack.pop().unwrap_or(Val::null()); self.stack.push(Val::int(v.as_i64())); }
                Instruction::UntagF64 => { let v = self.stack.pop().unwrap_or(Val::null()); self.stack.push(Val::float(v.as_f64())); }
                Instruction::UntagBool => { let v = self.stack.pop().unwrap_or(Val::null()); self.stack.push(Val::bool_(v.as_bool())); }
                Instruction::GetTag => { let v = self.stack.pop().unwrap_or(Val::null()); self.stack.push(Val::int(v.tag() as i64)); }

                // Control flow
                Instruction::Block => { self.label_stack.push(LabelInfo::Block(ip)); }
                Instruction::Loop => { self.label_stack.push(LabelInfo::Loop(ip)); }
                Instruction::End => { self.label_stack.pop(); }
                Instruction::Br(depth) => {
                    let target = self.resolve_branch(*depth);
                    match target {
                        Some(BranchTarget::Forward(_)) => { ip = self.find_end(instructions, ip, *depth); }
                        Some(BranchTarget::Backward(loop_ip)) => { ip = loop_ip + 1; continue; }
                        None => {}
                    }
                }
                Instruction::BrIf(depth) => {
                    let cond = self.stack.pop().unwrap_or(Val::null());
                    if cond.as_bool() {
                        let target = self.resolve_branch(*depth);
                        match target {
                            Some(BranchTarget::Forward(_)) => { ip = self.find_end(instructions, ip, *depth); }
                            Some(BranchTarget::Backward(loop_ip)) => { ip = loop_ip + 1; continue; }
                            None => {}
                        }
                    }
                }
                Instruction::BrTable(targets, default) => {
                    let idx = self.pop_i64() as usize;
                    let depth = if idx < targets.len() { targets[idx] } else { *default };
                    let target = self.resolve_branch(depth);
                    match target {
                        Some(BranchTarget::Forward(_)) => { ip = self.find_end(instructions, ip, depth); }
                        Some(BranchTarget::Backward(loop_ip)) => { ip = loop_ip + 1; continue; }
                        None => {}
                    }
                }
                Instruction::If => {
                    let cond = self.stack.pop().unwrap_or(Val::null());
                    self.label_stack.push(LabelInfo::Block(ip));
                    if !cond.as_bool() {
                        ip = self.skip_to_else_or_end(instructions, ip);
                        continue;
                    }
                }
                Instruction::IfVoid => {
                    let cond = self.stack.pop().unwrap_or(Val::null());
                    self.label_stack.push(LabelInfo::Block(ip));
                    if !cond.as_bool() {
                        ip = self.skip_to_else_or_end(instructions, ip);
                        continue;
                    }
                }
                Instruction::Else => {
                    // We reached Else from the then-branch — skip to End
                    ip = self.skip_to_end(instructions, ip);
                    continue;
                }
                Instruction::Return => {
                    return Ok(self.stack.pop().unwrap_or(Val::null()));
                }
                Instruction::Unreachable => {
                    return Err("unreachable executed".into());
                }
                Instruction::Nop => {}
                Instruction::Drop => { self.stack.pop(); }

                // Function calls
                Instruction::Call(fn_idx) => {
                    let func = module.functions.get(*fn_idx as usize)
                        .ok_or_else(|| format!("function index {} out of bounds", fn_idx))?;
                    let arg_count = func.param_count as usize;
                    let mut args = Vec::new();
                    for _ in 0..arg_count {
                        args.push(self.stack.pop().unwrap_or(Val::null()));
                    }
                    args.reverse();
                    let result = self.execute_function(module, *fn_idx as usize, &args)?;
                    self.stack.push(result);
                }
                Instruction::CallIndirect(_) => {
                    // Indirect calls via function table — pop index from stack
                    let fn_idx = self.pop_i64() as usize;
                    if fn_idx < module.functions.len() {
                        let func = &module.functions[fn_idx];
                        let arg_count = func.param_count as usize;
                        let mut args = Vec::new();
                        for _ in 0..arg_count { args.push(self.stack.pop().unwrap_or(Val::null())); }
                        args.reverse();
                        let result = self.execute_function(module, fn_idx, &args)?;
                        self.stack.push(result);
                    } else {
                        self.stack.push(Val::null());
                    }
                }

                // Memory operations
                Instruction::HeapAlloc(_size) => {
                    let addr = self.next_heap;
                    self.next_heap += 1;
                    self.stack.push(Val::int(addr as i64));
                }
                Instruction::MemLoadI64 | Instruction::MemLoadF64 | Instruction::MemLoadI32 => {
                    self.stack.push(Val::null()); // Memory ops need heap backing
                }
                Instruction::MemStoreI64 | Instruction::MemStoreF64 | Instruction::MemStoreI32 => {
                    self.stack.pop(); self.stack.pop(); // addr, val
                }

                // Runtime support
                Instruction::ArrayNew(count) => {
                    let mut elems = Vec::new();
                    for _ in 0..*count { elems.push(self.stack.pop().unwrap_or(Val::null())); }
                    elems.reverse();
                    let addr = self.next_heap;
                    self.next_heap += 1;
                    self.heap_arrays.insert(addr, elems);
                    self.stack.push(Val::array(addr as u32));
                }
                Instruction::ArrayGet => {
                    let idx = self.pop_i64() as usize;
                    let arr_val = self.stack.pop().unwrap_or(Val::null());
                    let ptr = arr_val.payload() as usize;
                    let val = self.heap_arrays.get(&ptr).and_then(|a| a.get(idx)).copied().unwrap_or(Val::null());
                    self.stack.push(val);
                }
                Instruction::ArraySet => {
                    let val = self.stack.pop().unwrap_or(Val::null());
                    let idx = self.pop_i64() as usize;
                    let arr_val = self.stack.pop().unwrap_or(Val::null());
                    let ptr = arr_val.payload() as usize;
                    if let Some(arr) = self.heap_arrays.get_mut(&ptr) {
                        while arr.len() <= idx { arr.push(Val::null()); }
                        arr[idx] = val;
                    }
                }
                Instruction::ArrayLen => {
                    let arr_val = self.stack.pop().unwrap_or(Val::null());
                    let ptr = arr_val.payload() as usize;
                    let len = self.heap_arrays.get(&ptr).map(|a| a.len()).unwrap_or(0);
                    self.stack.push(Val::int(len as i64));
                }
                Instruction::MapNew(count) => {
                    let mut entries = Vec::new();
                    for _ in 0..*count {
                        let val = self.stack.pop().unwrap_or(Val::null());
                        let key_val = self.stack.pop().unwrap_or(Val::null());
                        let key = if key_val.is_tagged() && key_val.tag() == tag::STRING {
                            self.strings.get(key_val.payload() as usize).cloned().unwrap_or_default()
                        } else {
                            format!("{}", key_val.as_i64())
                        };
                        entries.push((key, val));
                    }
                    entries.reverse();
                    let addr = self.next_heap;
                    self.next_heap += 1;
                    self.heap_maps.insert(addr, entries);
                    self.stack.push(Val::map(addr as u32));
                }
                Instruction::MapGet => {
                    let key_val = self.stack.pop().unwrap_or(Val::null());
                    let map_val = self.stack.pop().unwrap_or(Val::null());
                    let ptr = map_val.payload() as usize;
                    let key = if key_val.is_tagged() && key_val.tag() == tag::STRING {
                        self.strings.get(key_val.payload() as usize).cloned().unwrap_or_default()
                    } else {
                        format!("{}", key_val.as_i64())
                    };
                    let val = self.heap_maps.get(&ptr)
                        .and_then(|m| m.iter().find(|(k, _)| k == &key))
                        .map(|(_, v)| *v)
                        .unwrap_or(Val::null());
                    self.stack.push(val);
                }
                Instruction::MapSet => {
                    let val = self.stack.pop().unwrap_or(Val::null());
                    let key_val = self.stack.pop().unwrap_or(Val::null());
                    let map_val = self.stack.pop().unwrap_or(Val::null());
                    let ptr = map_val.payload() as usize;
                    let key = if key_val.is_tagged() && key_val.tag() == tag::STRING {
                        self.strings.get(key_val.payload() as usize).cloned().unwrap_or_default()
                    } else {
                        format!("{}", key_val.as_i64())
                    };
                    if let Some(m) = self.heap_maps.get_mut(&ptr) {
                        if let Some(entry) = m.iter_mut().find(|(k, _)| k == &key) {
                            entry.1 = val;
                        } else {
                            m.push((key, val));
                        }
                    }
                }
                Instruction::StringConcat => {
                    let b_val = self.stack.pop().unwrap_or(Val::null());
                    let a_val = self.stack.pop().unwrap_or(Val::null());
                    let a = self.val_to_string(&a_val);
                    let b = self.val_to_string(&b_val);
                    let result = format!("{}{}", a, b);
                    let idx = self.intern_string(&result);
                    self.stack.push(Val::string(idx));
                }
                Instruction::StringLen => {
                    let s_val = self.stack.pop().unwrap_or(Val::null());
                    let s = self.val_to_string(&s_val);
                    self.stack.push(Val::int(s.len() as i64));
                }
                Instruction::Print => {
                    let val = self.stack.pop().unwrap_or(Val::null());
                    let s = self.val_to_display_string(&val);
                    self.output.push(s.clone());
                }
                Instruction::RuntimeCall { name, arg_count } => {
                    let fn_name = self.strings.get(*name as usize).cloned().unwrap_or_default();
                    let mut args = Vec::new();
                    for _ in 0..*arg_count { args.push(self.stack.pop().unwrap_or(Val::null())); }
                    args.reverse();
                    let result = self.dispatch_runtime_call(&fn_name, &args);
                    self.stack.push(result);
                }
            }
            ip += 1;
        }
        Ok(self.stack.pop().unwrap_or(Val::null()))
    }

    fn pop_i64(&mut self) -> i64 { self.stack.pop().unwrap_or(Val::null()).as_i64() }
    fn pop_f64(&mut self) -> f64 { self.stack.pop().unwrap_or(Val::null()).as_f64() }

    fn val_to_string(&self, val: &Val) -> String {
        if val.is_tagged() {
            match val.tag() {
                tag::STRING => self.strings.get(val.payload() as usize).cloned().unwrap_or_default(),
                tag::I64 => format!("{}", val.payload() as i64),
                tag::BOOL => if val.payload() != 0 { "true".into() } else { "false".into() },
                tag::NULL => "null".into(),
                _ => format!("<val:{}>", val.0),
            }
        } else {
            let f = val.as_f64();
            if f == (f as i64) as f64 && f.abs() < 1e15 { format!("{}", f as i64) } else { format!("{}", f) }
        }
    }

    fn val_to_display_string(&self, val: &Val) -> String {
        self.val_to_string(val)
    }

    fn intern_string(&mut self, s: &str) -> u32 {
        if let Some(idx) = self.strings.iter().position(|existing| existing == s) {
            return idx as u32;
        }
        let idx = self.strings.len() as u32;
        self.strings.push(s.to_string());
        idx
    }

    fn eval_const_expr(&mut self, instructions: &[Instruction]) -> Result<Val, String> {
        for inst in instructions {
            match inst {
                Instruction::PushI64(n) => return Ok(Val::int(*n)),
                Instruction::PushF64(f) => return Ok(Val::float(*f)),
                Instruction::PushNull => return Ok(Val::null()),
                Instruction::PushBool(b) => return Ok(Val::bool_(*b)),
                _ => {}
            }
        }
        Ok(Val::null())
    }

    fn dispatch_runtime_call(&mut self, name: &str, args: &[Val]) -> Val {
        let a = args.first().copied().unwrap_or(Val::null());
        let b = args.get(1).copied().unwrap_or(Val::null());

        match name {
            // Arithmetic — dynamic dispatch based on operand types
            "__add" => self.runtime_arith(a, b, |x, y| x.wrapping_add(y), |x, y| x + y, true),
            "__sub" => self.runtime_arith(a, b, |x, y| x.wrapping_sub(y), |x, y| x - y, false),
            "__mul" => self.runtime_arith(a, b, |x, y| x.wrapping_mul(y), |x, y| x * y, false),
            "__div" => self.runtime_arith(a, b, |x, y| if y != 0 { x / y } else { 0 }, |x, y| x / y, false),
            "__mod" | "__rem" => self.runtime_arith(a, b, |x, y| if y != 0 { x % y } else { 0 }, |x, y| x % y, false),
            "__pow" => {
                let base = a.as_f64();
                let exp = b.as_f64();
                Val::float(base.powf(exp))
            }
            "__neg" => {
                if a.is_tagged() && a.tag() == tag::I64 { Val::int(-a.as_i64()) }
                else { Val::float(-a.as_f64()) }
            }

            // Comparison
            "__eq" => Val::bool_(a.0 == b.0),
            "__ne" => Val::bool_(a.0 != b.0),
            "__lt" => self.runtime_cmp(a, b, |x, y| x < y, |x, y| x < y),
            "__gt" => self.runtime_cmp(a, b, |x, y| x > y, |x, y| x > y),
            "__le" => self.runtime_cmp(a, b, |x, y| x <= y, |x, y| x <= y),
            "__ge" => self.runtime_cmp(a, b, |x, y| x >= y, |x, y| x >= y),

            // Logical
            "__and" => Val::bool_(a.as_bool() && b.as_bool()),
            "__or" => Val::bool_(a.as_bool() || b.as_bool()),
            "__not" => Val::bool_(!a.as_bool()),

            // Bitwise
            "__bit_and" => Val::int(a.as_i64() & b.as_i64()),
            "__bit_or" => Val::int(a.as_i64() | b.as_i64()),
            "__bit_xor" => Val::int(a.as_i64() ^ b.as_i64()),
            "__shl" => Val::int(a.as_i64().wrapping_shl(b.as_i64() as u32)),
            "__shr" => Val::int(a.as_i64().wrapping_shr(b.as_i64() as u32)),
            "__bit_not" => Val::int(!a.as_i64()),

            // Range
            "__range" => {
                let start = a.as_i64();
                let end = b.as_i64();
                let inclusive = args.get(2).map(|v| v.as_bool()).unwrap_or(false);
                let mut elems = Vec::new();
                if inclusive {
                    for i in start..=end { elems.push(Val::int(i)); }
                } else {
                    for i in start..end { elems.push(Val::int(i)); }
                }
                let addr = self.next_heap;
                self.next_heap += 1;
                self.heap_arrays.insert(addr, elems);
                Val::array(addr as u32)
            }

            // String/collection operations
            "len" => {
                if a.is_tagged() && a.tag() == tag::ARRAY {
                    Val::int(self.heap_arrays.get(&(a.payload() as usize)).map(|v| v.len()).unwrap_or(0) as i64)
                } else if a.is_tagged() && a.tag() == tag::STRING {
                    Val::int(self.strings.get(a.payload() as usize).map(|s| s.len()).unwrap_or(0) as i64)
                } else { Val::int(0) }
            }
            "to_string" | "string" => {
                let s = self.val_to_string(&a);
                let idx = self.intern_string(&s);
                Val::string(idx)
            }
            "typeof" | "type_of" => {
                let name = if !a.is_tagged() { "float" }
                else { match a.tag() { tag::NULL => "null", tag::BOOL => "bool", tag::I64 => "int", tag::STRING => "string", tag::ARRAY => "array", tag::MAP => "map", _ => "unknown" } };
                let idx = self.intern_string(name);
                Val::string(idx)
            }
            "push" | "array_push" => {
                let ptr = a.payload() as usize;
                if let Some(arr) = self.heap_arrays.get_mut(&ptr) { arr.push(b); }
                a
            }
            "pop" | "array_pop" => {
                let ptr = a.payload() as usize;
                self.heap_arrays.get_mut(&ptr).and_then(|arr| arr.pop()).unwrap_or(Val::null())
            }
            "map_get" => {
                let ptr = a.payload() as usize;
                let key = self.val_to_string(&b);
                self.heap_maps.get(&ptr)
                    .and_then(|m| m.iter().find(|(k, _)| k == &key))
                    .map(|(_, v)| *v)
                    .unwrap_or(Val::null())
            }
            "map_set" => {
                let ptr = a.payload() as usize;
                let key = self.val_to_string(&b);
                let val = args.get(2).copied().unwrap_or(Val::null());
                if let Some(m) = self.heap_maps.get_mut(&ptr) {
                    if let Some(entry) = m.iter_mut().find(|(k, _)| k == &key) { entry.1 = val; }
                    else { m.push((key, val)); }
                }
                Val::null()
            }
            "parse_int" => {
                let s = self.val_to_string(&a);
                Val::int(s.parse::<i64>().unwrap_or(0))
            }
            "parse_float" => {
                let s = self.val_to_string(&a);
                Val::float(s.parse::<f64>().unwrap_or(0.0))
            }
            _ => Val::null(),
        }
    }

    fn runtime_arith(&mut self, a: Val, b: Val, int_op: impl Fn(i64, i64) -> i64, float_op: impl Fn(f64, f64) -> f64, concat_strings: bool) -> Val {
        let a_tag = a.tag();
        let b_tag = b.tag();
        // Both integers
        if a.is_tagged() && a_tag == tag::I64 && b.is_tagged() && b_tag == tag::I64 {
            return Val::int(int_op(a.as_i64(), b.as_i64()));
        }
        // String concat for __add
        if concat_strings && a.is_tagged() && a_tag == tag::STRING && b.is_tagged() && b_tag == tag::STRING {
            let sa = self.strings.get(a.payload() as usize).cloned().unwrap_or_default();
            let sb = self.strings.get(b.payload() as usize).cloned().unwrap_or_default();
            let concatenated = format!("{}{}", sa, sb);
            let idx = self.strings.len();
            self.strings.push(concatenated);
            return Val::string(idx as u32);
        }
        // Float arithmetic
        Val::float(float_op(a.as_f64(), b.as_f64()))
    }

    fn runtime_cmp(&self, a: Val, b: Val, int_cmp: impl Fn(i64, i64) -> bool, float_cmp: impl Fn(f64, f64) -> bool) -> Val {
        if a.is_tagged() && a.tag() == tag::I64 && b.is_tagged() && b.tag() == tag::I64 {
            Val::bool_(int_cmp(a.as_i64(), b.as_i64()))
        } else {
            Val::bool_(float_cmp(a.as_f64(), b.as_f64()))
        }
    }

    fn resolve_branch(&self, depth: u32) -> Option<BranchTarget> {
        let idx = self.label_stack.len().checked_sub(1 + depth as usize)?;
        match &self.label_stack[idx] {
            LabelInfo::Block(_) => Some(BranchTarget::Forward(0)),
            LabelInfo::Loop(start) => Some(BranchTarget::Backward(*start)),
        }
    }

    fn find_end(&self, instructions: &[Instruction], from: usize, depth: u32) -> usize {
        let mut nested = 0u32;
        let mut target_depth = depth;
        let mut ip = from + 1;
        while ip < instructions.len() {
            match &instructions[ip] {
                Instruction::Block | Instruction::Loop | Instruction::If | Instruction::IfVoid => nested += 1,
                Instruction::End => {
                    if nested == 0 {
                        if target_depth == 0 { return ip; }
                        target_depth -= 1;
                    } else {
                        nested -= 1;
                    }
                }
                _ => {}
            }
            ip += 1;
        }
        instructions.len()
    }

    fn skip_to_else_or_end(&self, instructions: &[Instruction], from: usize) -> usize {
        let mut nested = 0u32;
        let mut ip = from + 1;
        while ip < instructions.len() {
            match &instructions[ip] {
                Instruction::Block | Instruction::Loop | Instruction::If | Instruction::IfVoid => nested += 1,
                Instruction::Else if nested == 0 => return ip + 1,
                Instruction::End => {
                    if nested == 0 { return ip; }
                    nested -= 1;
                }
                _ => {}
            }
            ip += 1;
        }
        instructions.len()
    }

    fn skip_to_end(&self, instructions: &[Instruction], from: usize) -> usize {
        let mut nested = 0u32;
        let mut ip = from + 1;
        while ip < instructions.len() {
            match &instructions[ip] {
                Instruction::Block | Instruction::Loop | Instruction::If | Instruction::IfVoid => nested += 1,
                Instruction::End => {
                    if nested == 0 { return ip; }
                    nested -= 1;
                }
                _ => {}
            }
            ip += 1;
        }
        instructions.len()
    }
}

enum BranchTarget {
    Forward(#[allow(dead_code)] usize),
    Backward(usize),
}

/// Execute a MAGI program via the IR VM pipeline.
pub fn run_ir(source: &str) -> Result<Vec<String>, String> {
    let program = crate::syntax::parser::parse_v2(source)
        .map_err(|e| format!("parse error: {}", e.message))?;
    let mut compiler = super::Compiler::new();
    let module = compiler.compile(&program)
        .map_err(|e| format!("{}", e))?;
    let mut vm = IrVm::new();
    vm.execute(&module)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ir_vm_arithmetic() {
        let output = run_ir("output 1 + 2;").unwrap();
        assert_eq!(output, vec!["3"]);
    }

    #[test]
    fn test_ir_vm_variables() {
        let output = run_ir("let x = 10; let y = 20; output x + y;").unwrap();
        assert_eq!(output, vec!["30"]);
    }

    #[test]
    fn test_ir_vm_string() {
        let output = run_ir(r#"output "hello";"#).unwrap();
        assert_eq!(output, vec!["hello"]);
    }

    #[test]
    fn test_ir_vm_if_else() {
        let output = run_ir("let x = 5; if x > 3 { output 1; } else { output 0; }").unwrap();
        assert_eq!(output, vec!["1"]);
    }

    #[test]
    fn test_ir_vm_function() {
        let output = run_ir("fn add(a, b) { a + b } output add(3, 4);").unwrap();
        assert_eq!(output, vec!["7"]);
    }

    #[test]
    fn test_ir_vm_while_loop() {
        let output = run_ir("let mut i = 0; while i < 5 { i = i + 1; } output i;").unwrap();
        assert_eq!(output, vec!["5"]);
    }
}
