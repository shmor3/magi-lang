//! MagiVM — the virtual machine that executes compiled .magc bytecode.
//!
//! Stack-based VM with call frames, garbage collection, and runtime library.

use super::classfile::{ClassFile, Constant, Function};
use super::gc::GarbageCollector;
use crate::types::DataType;
use std::collections::HashMap;

const MAX_STACK: usize = 65536;


pub struct MagiVM {
    stack: Vec<DataType>,
    frames: Vec<CallFrame>,
    globals: HashMap<String, DataType>,
    gc: GarbageCollector,
    output: Vec<String>,
    halted: bool,
}

struct CallFrame {
    function: Function,
    ip: usize,
    base: usize, // stack base for this frame's locals
    constants: Vec<Constant>,
}

// Opcodes for the MagiVM (superset of the basic bytecode compiler opcodes)
mod op {
    pub const NOP: u8 = 0x00;
    pub const CONST: u8 = 0x01;
    pub const NULL: u8 = 0x02;
    pub const TRUE: u8 = 0x03;
    pub const FALSE: u8 = 0x04;
    pub const POP: u8 = 0x05;
    pub const DUP: u8 = 0x06;

    pub const ADD: u8 = 0x10;
    pub const SUB: u8 = 0x11;
    pub const MUL: u8 = 0x12;
    pub const DIV: u8 = 0x13;
    pub const MOD: u8 = 0x14;
    pub const NEG: u8 = 0x15;
    pub const _POW: u8 = 0x16;

    pub const EQ: u8 = 0x20;
    pub const NE: u8 = 0x21;
    pub const LT: u8 = 0x22;
    pub const LE: u8 = 0x23;
    pub const GT: u8 = 0x24;
    pub const GE: u8 = 0x25;

    pub const NOT: u8 = 0x30;
    pub const AND: u8 = 0x31;
    pub const OR: u8 = 0x32;

    pub const LOAD_LOCAL: u8 = 0x40;
    pub const STORE_LOCAL: u8 = 0x41;
    pub const LOAD_GLOBAL: u8 = 0x42;
    pub const STORE_GLOBAL: u8 = 0x43;

    pub const JUMP: u8 = 0x50;
    pub const JUMP_IF_FALSE: u8 = 0x51;
    pub const JUMP_IF_TRUE: u8 = 0x52;
    pub const CALL: u8 = 0x53;
    pub const RETURN: u8 = 0x54;

    pub const OUTPUT: u8 = 0x70;

    pub const NEW_ARRAY: u8 = 0x80;
    pub const NEW_MAP: u8 = 0x81;
    pub const INDEX_GET: u8 = 0x82;
    pub const _INDEX_SET: u8 = 0x83;
    pub const _FIELD_GET: u8 = 0x84;
    pub const _FIELD_SET: u8 = 0x85;
    pub const _ARRAY_PUSH: u8 = 0x86;
    pub const LENGTH: u8 = 0x87;

    pub const CONCAT: u8 = 0x90;
    pub const TO_STRING: u8 = 0x91;

    pub const GC_ALLOC: u8 = 0xA0;
    pub const GC_READ: u8 = 0xA1;
    pub const GC_WRITE: u8 = 0xA2;

    pub const HALT: u8 = 0xFF;
}

impl MagiVM {
    pub fn new() -> Self {
        MagiVM {
            stack: Vec::with_capacity(256),
            frames: Vec::new(),
            globals: HashMap::new(),
            gc: GarbageCollector::new(),
            output: Vec::new(),
            halted: false,
        }
    }

    pub fn execute(&mut self, classfile: &ClassFile) -> Result<DataType, String> {
        if classfile.functions.is_empty() {
            return Ok(DataType::Null);
        }
        let entry = classfile.entry as usize;
        if entry >= classfile.functions.len() {
            return Err("invalid entry point".into());
        }

        let func = classfile.functions[entry].clone();
        self.frames.push(CallFrame {
            function: func,
            ip: 0,
            base: 0,
            constants: classfile.constants.clone(),
        });

        // Ensure stack has space for locals
        let locals_needed = classfile.functions[entry].locals as usize;
        for _ in 0..locals_needed {
            self.stack.push(DataType::Null);
        }

        self.run()
    }

    fn run(&mut self) -> Result<DataType, String> {
        let max_steps = 100_000_000u64;
        let mut steps = 0u64;

        while !self.halted && !self.frames.is_empty() {
            steps += 1;
            if steps > max_steps {
                return Err("execution limit exceeded".into());
            }

            let frame = self.frames.last_mut().unwrap();
            if frame.ip >= frame.function.code.len() {
                // Implicit return
                let result = self.stack.pop().unwrap_or(DataType::Null);
                self.frames.pop();
                if !self.frames.is_empty() {
                    self.stack.push(result);
                } else {
                    return Ok(result);
                }
                continue;
            }

            let opcode = frame.function.code[frame.ip];
            frame.ip += 1;

            match opcode {
                op::NOP => {}
                op::HALT => { self.halted = true; }

                op::CONST => {
                    let idx = self.read_u16() as usize;
                    let frame = self.frames.last().unwrap();
                    let val = if idx < frame.constants.len() {
                        frame.constants[idx].to_datatype()
                    } else { DataType::Null };
                    self.stack.push(val);
                }
                op::NULL => self.stack.push(DataType::Null),
                op::TRUE => self.stack.push(DataType::Bool(true)),
                op::FALSE => self.stack.push(DataType::Bool(false)),
                op::POP => { self.stack.pop(); }
                op::DUP => {
                    if let Some(v) = self.stack.last().cloned() { self.stack.push(v); }
                }

                // Arithmetic
                op::ADD => self.binary(|a, b| match (a, b) {
                    (DataType::Int64(x), DataType::Int64(y)) => DataType::Int64(x.wrapping_add(y)),
                    (DataType::Float64(x), DataType::Float64(y)) => DataType::Float64(x + y),
                    (DataType::Int64(x), DataType::Float64(y)) => DataType::Float64(x as f64 + y),
                    (DataType::Float64(x), DataType::Int64(y)) => DataType::Float64(x + y as f64),
                    (DataType::String(x), DataType::String(y)) => DataType::String(format!("{}{}", x, y)),
                    (DataType::String(x), b) => DataType::String(format!("{}{}", x, b.to_string_lossy())),
                    (a, DataType::String(y)) => DataType::String(format!("{}{}", a.to_string_lossy(), y)),
                    _ => DataType::Null,
                }),
                op::SUB => self.binary(|a, b| match (a, b) {
                    (DataType::Int64(x), DataType::Int64(y)) => DataType::Int64(x.wrapping_sub(y)),
                    (DataType::Float64(x), DataType::Float64(y)) => DataType::Float64(x - y),
                    _ => DataType::Null,
                }),
                op::MUL => self.binary(|a, b| match (a, b) {
                    (DataType::Int64(x), DataType::Int64(y)) => DataType::Int64(x.wrapping_mul(y)),
                    (DataType::Float64(x), DataType::Float64(y)) => DataType::Float64(x * y),
                    (DataType::String(s), DataType::Int64(n)) => DataType::String(s.repeat(n.max(0) as usize)),
                    _ => DataType::Null,
                }),
                op::DIV => self.binary(|a, b| match (a, b) {
                    (DataType::Int64(x), DataType::Int64(y)) if y != 0 => DataType::Int64(x / y),
                    (DataType::Float64(x), DataType::Float64(y)) => DataType::Float64(x / y),
                    _ => DataType::Null,
                }),
                op::MOD => self.binary(|a, b| match (a, b) {
                    (DataType::Int64(x), DataType::Int64(y)) if y != 0 => DataType::Int64(x % y),
                    _ => DataType::Null,
                }),
                op::NEG => {
                    if let Some(v) = self.stack.pop() {
                        self.stack.push(match v {
                            DataType::Int64(n) => DataType::Int64(-n),
                            DataType::Float64(f) => DataType::Float64(-f),
                            _ => DataType::Null,
                        });
                    }
                }

                // Comparison
                op::EQ => self.binary(|a, b| DataType::Bool(a == b)),
                op::NE => self.binary(|a, b| DataType::Bool(a != b)),
                op::LT => self.cmp(|a, b| a < b),
                op::LE => self.cmp(|a, b| a <= b),
                op::GT => self.cmp(|a, b| a > b),
                op::GE => self.cmp(|a, b| a >= b),

                // Logic
                op::NOT => {
                    if let Some(v) = self.stack.pop() {
                        self.stack.push(DataType::Bool(!v.to_bool()));
                    }
                }
                op::AND => self.binary(|a, b| if !a.to_bool() { a } else { b }),
                op::OR => self.binary(|a, b| if a.to_bool() { a } else { b }),

                // Locals
                op::LOAD_LOCAL => {
                    let idx = self.read_u8() as usize;
                    let frame = self.frames.last().unwrap();
                    let abs = frame.base + idx;
                    let val = self.stack.get(abs).cloned().unwrap_or(DataType::Null);
                    self.stack.push(val);
                }
                op::STORE_LOCAL => {
                    let idx = self.read_u8() as usize;
                    let frame = self.frames.last().unwrap();
                    let abs = frame.base + idx;
                    if let Some(val) = self.stack.last().cloned() {
                        while self.stack.len() <= abs {
                            self.stack.push(DataType::Null);
                        }
                        self.stack[abs] = val;
                    }
                }

                // Globals
                op::LOAD_GLOBAL => {
                    let idx = self.read_u16() as usize;
                    let name = self.get_constant_string(idx);
                    let val = self.globals.get(&name).cloned().unwrap_or(DataType::Null);
                    self.stack.push(val);
                }
                op::STORE_GLOBAL => {
                    let idx = self.read_u16() as usize;
                    let name = self.get_constant_string(idx);
                    if let Some(val) = self.stack.last().cloned() {
                        self.globals.insert(name, val);
                    }
                }

                // Control flow
                op::JUMP => {
                    let target = self.read_u16() as usize;
                    self.frames.last_mut().unwrap().ip = target;
                }
                op::JUMP_IF_FALSE => {
                    let target = self.read_u16() as usize;
                    if let Some(v) = self.stack.last() {
                        if !v.to_bool() {
                            self.frames.last_mut().unwrap().ip = target;
                        }
                    }
                }
                op::JUMP_IF_TRUE => {
                    let target = self.read_u16() as usize;
                    if let Some(v) = self.stack.last() {
                        if v.to_bool() {
                            self.frames.last_mut().unwrap().ip = target;
                        }
                    }
                }
                op::CALL => {
                    let _func_idx = self.read_u16();
                    let _arg_count = self.read_u8();
                    // Function calls would push a new frame
                    // For now, simple implementation
                }
                op::RETURN => {
                    let result = self.stack.pop().unwrap_or(DataType::Null);
                    let frame = self.frames.pop().unwrap();
                    // Clean up stack to frame base
                    self.stack.truncate(frame.base);
                    if !self.frames.is_empty() {
                        self.stack.push(result);
                    } else {
                        return Ok(result);
                    }
                }

                // I/O
                op::OUTPUT => {
                    if let Some(val) = self.stack.pop() {
                        let s = val.to_string_lossy();
                        self.output.push(s.clone());
                        println!("{}", s);
                    }
                }

                // Collections
                op::NEW_ARRAY => {
                    let count = self.read_u16() as usize;
                    let mut arr = Vec::with_capacity(count);
                    for _ in 0..count {
                        arr.push(self.stack.pop().unwrap_or(DataType::Null));
                    }
                    arr.reverse();
                    self.stack.push(DataType::Array(arr));
                }
                op::NEW_MAP => {
                    let count = self.read_u16() as usize;
                    let mut map = crate::util::OrderedMap::new();
                    for _ in 0..count {
                        let val = self.stack.pop().unwrap_or(DataType::Null);
                        let key = self.stack.pop().unwrap_or(DataType::Null).to_string_lossy();
                        map.insert(key, val);
                    }
                    self.stack.push(DataType::Map(map));
                }
                op::INDEX_GET => {
                    let idx = self.stack.pop().unwrap_or(DataType::Null);
                    let obj = self.stack.pop().unwrap_or(DataType::Null);
                    let result = match (&obj, &idx) {
                        (DataType::Array(a), DataType::Int64(i)) => {
                            let i = if *i < 0 { (a.len() as i64 + i).max(0) as usize } else { *i as usize };
                            a.get(i).cloned().unwrap_or(DataType::Null)
                        }
                        (DataType::Map(m), DataType::String(k)) => m.get(k).cloned().unwrap_or(DataType::Null),
                        (DataType::String(s), DataType::Int64(i)) => {
                            s.chars().nth(*i as usize).map(|c| DataType::String(c.to_string())).unwrap_or(DataType::Null)
                        }
                        _ => DataType::Null,
                    };
                    self.stack.push(result);
                }
                op::LENGTH => {
                    if let Some(v) = self.stack.pop() {
                        let len = match &v {
                            DataType::Array(a) => a.len() as i64,
                            DataType::Map(m) => m.len() as i64,
                            DataType::String(s) => s.chars().count() as i64,
                            DataType::Bytes(b) => b.len() as i64,
                            _ => 0,
                        };
                        self.stack.push(DataType::Int64(len));
                    }
                }
                op::CONCAT => {
                    let b = self.stack.pop().unwrap_or(DataType::Null).to_string_lossy();
                    let a = self.stack.pop().unwrap_or(DataType::Null).to_string_lossy();
                    self.stack.push(DataType::String(format!("{}{}", a, b)));
                }
                op::TO_STRING => {
                    if let Some(v) = self.stack.pop() {
                        self.stack.push(DataType::String(v.to_string_lossy()));
                    }
                }

                // GC operations
                op::GC_ALLOC => {
                    let val = self.stack.pop().unwrap_or(DataType::Null);
                    let id = self.gc.alloc(val);
                    self.stack.push(DataType::Int64(id as i64));
                }
                op::GC_READ => {
                    let id = self.stack.pop().and_then(|v| v.to_i64()).unwrap_or(0) as u64;
                    let val = self.gc.read(id).cloned().unwrap_or(DataType::Null);
                    self.stack.push(val);
                }
                op::GC_WRITE => {
                    let val = self.stack.pop().unwrap_or(DataType::Null);
                    let id = self.stack.pop().and_then(|v| v.to_i64()).unwrap_or(0) as u64;
                    self.gc.write(id, val);
                }

                _ => {
                    return Err(format!("unknown opcode: 0x{:02X}", opcode));
                }
            }

            if self.stack.len() > MAX_STACK {
                return Err("stack overflow".into());
            }
        }

        Ok(self.stack.pop().unwrap_or(DataType::Null))
    }

    fn read_u8(&mut self) -> u8 {
        let frame = self.frames.last_mut().unwrap();
        let v = frame.function.code.get(frame.ip).copied().unwrap_or(0);
        frame.ip += 1;
        v
    }

    fn read_u16(&mut self) -> u16 {
        let frame = self.frames.last_mut().unwrap();
        let hi = frame.function.code.get(frame.ip).copied().unwrap_or(0) as u16;
        let lo = frame.function.code.get(frame.ip + 1).copied().unwrap_or(0) as u16;
        frame.ip += 2;
        (hi << 8) | lo
    }

    fn get_constant_string(&self, idx: usize) -> String {
        let frame = self.frames.last().unwrap();
        match frame.constants.get(idx) {
            Some(Constant::String(s)) => s.clone(),
            _ => format!("const_{}", idx),
        }
    }

    fn binary<F: Fn(DataType, DataType) -> DataType>(&mut self, f: F) {
        let b = self.stack.pop().unwrap_or(DataType::Null);
        let a = self.stack.pop().unwrap_or(DataType::Null);
        self.stack.push(f(a, b));
    }

    fn cmp<F: Fn(f64, f64) -> bool>(&mut self, f: F) {
        let b = self.stack.pop().unwrap_or(DataType::Null);
        let a = self.stack.pop().unwrap_or(DataType::Null);
        let result = match (a.to_f64(), b.to_f64()) {
            (Some(x), Some(y)) => f(x, y),
            _ => false,
        };
        self.stack.push(DataType::Bool(result));
    }

    pub fn get_output(&self) -> &[String] {
        &self.output
    }

    pub fn gc_stats(&self) -> super::gc::GcStats {
        self.gc.stats()
    }
}

/// Compile a MAGI source string to a ClassFile, then execute on the VM.
pub fn compile_and_run(source: &str) -> Result<DataType, String> {
    use crate::compiler::bytecode::BytecodeCompiler;

    let program = crate::syntax::parser::parse_v2(source)
        .map_err(|e| format!("parse error: {}", e.message))?;

    let mut compiler = BytecodeCompiler::new();
    compiler.compile(&program)?;

    // Build ClassFile from bytecode compiler output
    let mut cf = ClassFile::new();
    for c in compiler.chunk.constants.iter() {
        let constant = match c {
            DataType::Int64(n) => Constant::Int(*n),
            DataType::Float64(f) => Constant::Float(*f),
            DataType::String(s) => Constant::String(s.clone()),
            DataType::Bool(b) => Constant::Bool(*b),
            DataType::Null => Constant::Null,
            _ => Constant::String(c.to_string_lossy()),
        };
        cf.constants.push(constant);
    }
    cf.add_function(Function {
        name: "main".into(),
        arity: 0,
        locals: compiler.locals.len() as u16,
        code: compiler.chunk.code,
        line_table: compiler.chunk.lines.iter().enumerate()
            .map(|(i, &line)| (i as u32, line))
            .collect(),
    });

    let mut vm = MagiVM::new();
    vm.execute(&cf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_arithmetic() {
        let result = compile_and_run("output 1 + 2;").unwrap();
        // Output goes to stdout; result is the last value
    }

    #[test]
    fn test_vm_variables() {
        compile_and_run("let x = 10; let y = 20; output x + y;").unwrap();
    }

    #[test]
    fn test_vm_string_concat() {
        compile_and_run(r#"output "hello " + "world";"#).unwrap();
    }

    #[test]
    fn test_vm_comparison() {
        compile_and_run("output 3 > 2;").unwrap();
    }

    #[test]
    fn test_vm_while_loop() {
        compile_and_run("let mut x = 0; while x < 5 { x += 1; } output x;").unwrap();
    }

    #[test]
    fn test_classfile_execute() {
        let mut cf = ClassFile::new();
        let c_42 = cf.add_constant(Constant::Int(42));
        cf.add_function(Function {
            name: "main".into(),
            arity: 0,
            locals: 0,
            code: vec![
                op::CONST, (c_42 >> 8) as u8, c_42 as u8,
                op::OUTPUT,
                op::HALT,
            ],
            line_table: vec![],
        });

        let mut vm = MagiVM::new();
        vm.execute(&cf).unwrap();
        assert_eq!(vm.get_output(), &["42"]);
    }
}
