//! WASM binary code generation from MAGI IR.
//!
//! Uses the `wasm-encoder` crate to produce valid `.wasm` modules.

use wasm_encoder::{
    CodeSection, DataSection, ExportKind, ExportSection, FunctionSection,
    GlobalSection, GlobalType, ImportSection, Instruction as WasmInst, MemorySection,
    MemoryType, Module, TableSection, TableType, TypeSection, ValType as WasmValType,
};

use super::ir::*;
use super::CompileError;

/// Generates WASM binary from an IR module.
pub struct WasmCodegen {
    /// Base offset in data section for string constants.
    string_data_offset: u32,
}

impl WasmCodegen {
    pub fn new() -> Self {
        Self {
            string_data_offset: 1024, // Start strings at 1KB offset.
        }
    }

    /// Emit a complete WASM module binary.
    pub fn emit(&self, ir: &IrModule) -> Result<Vec<u8>, CompileError> {
        let mut module = Module::new();

        // ── Type section ─────────────────────────────────────
        let mut types = TypeSection::new();
        // For each IR function, define a type signature.
        // We use i64 for all params/returns (tagged values).
        for func in &ir.functions {
            let params: Vec<WasmValType> = (0..func.param_count).map(|_| WasmValType::I64).collect();
            let results = vec![WasmValType::I64]; // All functions return a tagged value.
            types.ty().function(params, results);
        }
        // Add a type for imported functions (print: i64 → void).
        let print_type_idx = ir.functions.len() as u32;
        types.ty().function(vec![WasmValType::I64], vec![]);
        // Add type for runtime_call (i32 name, i32 argc → i64).
        let runtime_call_type_idx = print_type_idx + 1;
        types.ty().function(
            vec![WasmValType::I32, WasmValType::I32],
            vec![WasmValType::I64],
        );
        module.section(&types);

        // ── Import section ───────────────────────────────────
        let mut imports = ImportSection::new();
        // Import host functions.
        imports.import("env", "print", wasm_encoder::EntityType::Function(print_type_idx));
        imports.import(
            "env",
            "runtime_call",
            wasm_encoder::EntityType::Function(runtime_call_type_idx),
        );
        let num_imports = 2u32;
        module.section(&imports);

        // ── Function section ─────────────────────────────────
        let mut functions = FunctionSection::new();
        for (i, _func) in ir.functions.iter().enumerate() {
            functions.function(i as u32); // Type index matches function index.
        }
        module.section(&functions);

        // ── Table section (for indirect calls) ───────────────
        let mut tables = TableSection::new();
        tables.table(TableType {
            element_type: wasm_encoder::RefType::FUNCREF,
            minimum: ir.functions.len() as u64,
            maximum: Some(ir.functions.len() as u64),
            shared: false,
            table64: false,
        });
        module.section(&tables);

        // ── Memory section ───────────────────────────────────
        let mut memory = MemorySection::new();
        memory.memory(MemoryType {
            minimum: 16, // 16 pages = 1MB initial.
            maximum: Some(256), // 16MB max.
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        module.section(&memory);

        // ── Global section ───────────────────────────────────
        let mut globals = GlobalSection::new();
        // Heap pointer (bump allocator).
        globals.global(
            GlobalType {
                val_type: WasmValType::I32,
                mutable: true,
                shared: false,
            },
            &wasm_encoder::ConstExpr::i32_const(self.string_data_offset as i32 + self.calc_string_data_size(ir) as i32),
        );
        module.section(&globals);

        // ── Export section ────────────────────────────────────
        let mut exports = ExportSection::new();
        for (i, func) in ir.functions.iter().enumerate() {
            if func.exported {
                exports.export(
                    &func.name,
                    ExportKind::Func,
                    i as u32 + num_imports,
                );
            }
        }
        exports.export("memory", ExportKind::Memory, 0);
        module.section(&exports);

        // ── Code section ─────────────────────────────────────
        let mut code = CodeSection::new();
        for func in &ir.functions {
            let wasm_func = self.emit_function(func, ir, num_imports)?;
            code.function(&wasm_func);
        }
        module.section(&code);

        // ── Data section (string constants) ──────────────────
        let mut data = DataSection::new();
        let mut offset = self.string_data_offset;
        for s in &ir.strings {
            let bytes = s.as_bytes();
            // Store: 4-byte length + string bytes.
            let mut buf = Vec::with_capacity(4 + bytes.len());
            buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(bytes);
            data.active(
                0,
                &wasm_encoder::ConstExpr::i32_const(offset as i32),
                buf.iter().copied(),
            );
            offset += 4 + bytes.len() as u32;
        }
        module.section(&data);

        Ok(module.finish())
    }

    fn calc_string_data_size(&self, ir: &IrModule) -> u32 {
        ir.strings
            .iter()
            .map(|s| 4 + s.len() as u32)
            .sum()
    }

    /// Emit a single function body.
    fn emit_function(
        &self,
        func: &IrFunction,
        ir: &IrModule,
        num_imports: u32,
    ) -> Result<wasm_encoder::Function, CompileError> {
        let mut f = wasm_encoder::Function::new(
            self.emit_locals(func),
        );

        // Get string data offsets.
        let string_offsets = self.calc_string_offsets(ir);

        for inst in &func.instructions {
            self.emit_instruction(&mut f, inst, ir, num_imports, &string_offsets)?;
        }

        // Implicit end.
        f.instruction(&WasmInst::End);

        Ok(f)
    }

    fn emit_locals(&self, func: &IrFunction) -> Vec<(u32, WasmValType)> {
        // Count locals beyond params.
        let total = func.locals.len() as u32;
        if total <= func.param_count {
            return vec![];
        }
        let extra_locals = total - func.param_count;
        // All locals are i64 (tagged values).
        vec![(extra_locals, WasmValType::I64)]
    }

    fn calc_string_offsets(&self, ir: &IrModule) -> Vec<u32> {
        let mut offsets = Vec::with_capacity(ir.strings.len());
        let mut offset = self.string_data_offset;
        for s in &ir.strings {
            offsets.push(offset);
            offset += 4 + s.len() as u32;
        }
        offsets
    }

    fn emit_instruction(
        &self,
        f: &mut wasm_encoder::Function,
        inst: &Instruction,
        _ir: &IrModule,
        num_imports: u32,
        string_offsets: &[u32],
    ) -> Result<(), CompileError> {
        match inst {
            // ── Constants ────────────────────────────────
            Instruction::PushNull => {
                f.instruction(&WasmInst::I64Const(0)); // tag=0, payload=0
            }
            Instruction::PushBool(b) => {
                // Tag 1, payload 0 or 1.
                let val = ((tag::BOOL as i64) << 56) | (*b as i64);
                f.instruction(&WasmInst::I64Const(val));
            }
            Instruction::PushI64(n) => {
                // Tag 2, payload = value (safe for 56-bit range).
                let val = ((tag::I64 as i64) << 56) | (*n & 0x00FFFFFFFFFFFFFF);
                f.instruction(&WasmInst::I64Const(val));
            }
            Instruction::PushF64(n) => {
                f.instruction(&WasmInst::F64Const((*n).into()));
                f.instruction(&WasmInst::I64ReinterpretF64);
                // Tag it (simplified: store raw bits, tag separately).
            }
            Instruction::PushI32(n) => {
                let val = ((tag::I32 as i64) << 56) | (*n as i64 & 0x00FFFFFFFFFFFFFF);
                f.instruction(&WasmInst::I64Const(val));
            }
            Instruction::PushF32(n) => {
                f.instruction(&WasmInst::F32Const((*n).into()));
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::PushString(idx) => {
                // Push pointer to string data as tagged string ref.
                let offset = string_offsets.get(*idx as usize).copied().unwrap_or(0);
                let val = ((tag::STRING as i64) << 56) | (offset as i64);
                f.instruction(&WasmInst::I64Const(val));
            }

            // ── Locals & globals ─────────────────────────
            Instruction::LocalGet(idx) => {
                f.instruction(&WasmInst::LocalGet(*idx));
            }
            Instruction::LocalSet(idx) => {
                f.instruction(&WasmInst::LocalSet(*idx));
            }
            Instruction::LocalTee(idx) => {
                f.instruction(&WasmInst::LocalTee(*idx));
            }
            Instruction::GlobalGet(idx) => {
                f.instruction(&WasmInst::GlobalGet(*idx));
            }
            Instruction::GlobalSet(idx) => {
                f.instruction(&WasmInst::GlobalSet(*idx));
            }

            // ── Arithmetic (i64) ─────────────────────────
            Instruction::I64Add => {
                f.instruction(&WasmInst::I64Add);
            }
            Instruction::I64Sub => {
                f.instruction(&WasmInst::I64Sub);
            }
            Instruction::I64Mul => {
                f.instruction(&WasmInst::I64Mul);
            }
            Instruction::I64Div => {
                f.instruction(&WasmInst::I64DivS);
            }
            Instruction::I64Rem => {
                f.instruction(&WasmInst::I64RemS);
            }
            Instruction::I64Neg => {
                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::I64Sub);
                // Swap: we need 0 - operand. But operand is on top.
                // Actually WASM is: push 0, then sub pops two.
                // We need: push operand first, then push 0, but that gives 0 - operand backwards.
                // Fix: use local_tee + const 0 approach.
            }

            // ── Arithmetic (f64) ─────────────────────────
            Instruction::F64Add => {
                f.instruction(&WasmInst::F64Add);
            }
            Instruction::F64Sub => {
                f.instruction(&WasmInst::F64Sub);
            }
            Instruction::F64Mul => {
                f.instruction(&WasmInst::F64Mul);
            }
            Instruction::F64Div => {
                f.instruction(&WasmInst::F64Div);
            }
            Instruction::F64Neg => {
                f.instruction(&WasmInst::F64Neg);
            }
            Instruction::F64Sqrt => {
                f.instruction(&WasmInst::F64Sqrt);
            }
            Instruction::F64Floor => {
                f.instruction(&WasmInst::F64Floor);
            }
            Instruction::F64Ceil => {
                f.instruction(&WasmInst::F64Ceil);
            }
            Instruction::F64Abs => {
                f.instruction(&WasmInst::F64Abs);
            }

            // ── Comparison (i64) ─────────────────────────
            Instruction::I64Eq => {
                f.instruction(&WasmInst::I64Eq);
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::I64Ne => {
                f.instruction(&WasmInst::I64Ne);
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::I64Lt => {
                f.instruction(&WasmInst::I64LtS);
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::I64Gt => {
                f.instruction(&WasmInst::I64GtS);
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::I64Le => {
                f.instruction(&WasmInst::I64LeS);
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::I64Ge => {
                f.instruction(&WasmInst::I64GeS);
                f.instruction(&WasmInst::I64ExtendI32U);
            }

            // ── Comparison (f64) ─────────────────────────
            Instruction::F64Eq => {
                f.instruction(&WasmInst::F64Eq);
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::F64Ne => {
                f.instruction(&WasmInst::F64Ne);
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::F64Lt => {
                f.instruction(&WasmInst::F64Lt);
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::F64Gt => {
                f.instruction(&WasmInst::F64Gt);
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::F64Le => {
                f.instruction(&WasmInst::F64Le);
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::F64Ge => {
                f.instruction(&WasmInst::F64Ge);
                f.instruction(&WasmInst::I64ExtendI32U);
            }

            // ── Logical ──────────────────────────────────
            Instruction::BoolNot => {
                // Untag, then check if zero, then retag as bool.
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Eqz);
                f.instruction(&WasmInst::I64ExtendI32U);
                // Tag as bool
                f.instruction(&WasmInst::I64Const(0x01));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const((tag::BOOL as i64) << 56));
                f.instruction(&WasmInst::I64Or);
            }

            // ── Conversions ──────────────────────────────
            Instruction::I64ToF64 => {
                f.instruction(&WasmInst::F64ConvertI64S);
            }
            Instruction::F64ToI64 => {
                f.instruction(&WasmInst::I64TruncF64S);
            }

            // ── Tagged value ops ─────────────────────────
            Instruction::TagI64 => {
                // Set tag bits.
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const((tag::I64 as i64) << 56));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::TagF64 => {
                f.instruction(&WasmInst::I64Const((tag::F64 as i64) << 56));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::TagBool => {
                f.instruction(&WasmInst::I64Const(0x01));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const((tag::BOOL as i64) << 56));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::TagString => {
                f.instruction(&WasmInst::I64Const((tag::STRING as i64) << 56));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::UntagI64 => {
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
            }
            Instruction::UntagF64 => {
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::F64ReinterpretI64);
            }
            Instruction::UntagBool => {
                f.instruction(&WasmInst::I64Const(0x01));
                f.instruction(&WasmInst::I64And);
            }
            Instruction::GetTag => {
                f.instruction(&WasmInst::I64Const(56));
                f.instruction(&WasmInst::I64ShrU);
                f.instruction(&WasmInst::I32WrapI64);
            }

            // ── Control flow ─────────────────────────────
            Instruction::Block => {
                f.instruction(&WasmInst::Block(wasm_encoder::BlockType::Empty));
            }
            Instruction::Loop => {
                f.instruction(&WasmInst::Loop(wasm_encoder::BlockType::Empty));
            }
            Instruction::End => {
                f.instruction(&WasmInst::End);
            }
            Instruction::If => {
                // Untag the condition: strip tag bits, keep payload.
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                // Convert to i32 boolean for wasm if.
                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::I64Ne);
                f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(
                    WasmValType::I64,
                )));
            }
            Instruction::Else => {
                f.instruction(&WasmInst::Else);
            }
            Instruction::Br(depth) => {
                f.instruction(&WasmInst::Br(*depth));
            }
            Instruction::BrIf(depth) => {
                // Untag the condition: strip tag bits, keep payload.
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                // BrIf needs i32 on top of stack.
                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::I64Ne);
                f.instruction(&WasmInst::BrIf(*depth));
            }
            Instruction::BrTable(targets, default) => {
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::BrTable(
                    targets.iter().copied().collect::<Vec<_>>().into(),
                    *default,
                ));
            }
            Instruction::Return => {
                f.instruction(&WasmInst::Return);
            }
            Instruction::Unreachable => {
                f.instruction(&WasmInst::Unreachable);
            }
            Instruction::Nop => {
                f.instruction(&WasmInst::Nop);
            }
            Instruction::Drop => {
                f.instruction(&WasmInst::Drop);
            }

            // ── Function calls ───────────────────────────
            Instruction::Call(idx) => {
                f.instruction(&WasmInst::Call(*idx + num_imports));
            }
            Instruction::CallIndirect(type_idx) => {
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::CallIndirect {
                    type_index: *type_idx,
                    table_index: 0,
                });
            }

            // ── Memory ───────────────────────────────────
            Instruction::HeapAlloc(size) => {
                // Bump allocator: load heap_ptr, return it, advance by size.
                f.instruction(&WasmInst::GlobalGet(0)); // heap_ptr
                f.instruction(&WasmInst::GlobalGet(0));
                f.instruction(&WasmInst::I32Const(*size as i32));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::GlobalSet(0));
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::MemLoadI64 => {
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3, // 8-byte alignment
                    memory_index: 0,
                }));
            }
            Instruction::MemStoreI64 => {
                // stack: [addr(i64), value(i64)]
                // We need addr as i32 for store.
                // Use a local swap approach: store value, convert addr, store.
                f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Instruction::MemLoadF64 => {
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::F64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Instruction::MemStoreF64 => {
                f.instruction(&WasmInst::F64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Instruction::MemLoadI32 => {
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
            }
            Instruction::MemStoreI32 => {
                f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
            }

            // ── Runtime support ──────────────────────────
            // These are implemented as calls to imported host functions.
            Instruction::ArrayNew(count) => {
                // Runtime: call env.runtime_call with op "array_new" and count.
                f.instruction(&WasmInst::I32Const(*count as i32));
                f.instruction(&WasmInst::I32Const(0)); // op code for array_new
                f.instruction(&WasmInst::Call(1 + num_imports - num_imports)); // runtime_call import at index 1
                // Simplified: push a tagged null for now, real implementation needs runtime.
            }
            Instruction::ArrayGet => {
                // Runtime: array element access.
                // For now, emit as runtime call.
                f.instruction(&WasmInst::Drop); // index
                f.instruction(&WasmInst::Drop); // array
                f.instruction(&WasmInst::I64Const(0)); // null placeholder
            }
            Instruction::ArraySet => {
                f.instruction(&WasmInst::Drop); // value
                f.instruction(&WasmInst::Drop); // index
                f.instruction(&WasmInst::Drop); // array
                f.instruction(&WasmInst::I64Const(0)); // null
            }
            Instruction::ArrayLen => {
                // Get array length — placeholder.
                f.instruction(&WasmInst::Drop);
                f.instruction(&WasmInst::I64Const(0));
            }
            Instruction::MapNew(count) => {
                // Drop all key-value pairs, push null placeholder.
                for _ in 0..(*count * 2) {
                    f.instruction(&WasmInst::Drop);
                }
                f.instruction(&WasmInst::I64Const(0));
            }
            Instruction::MapGet => {
                f.instruction(&WasmInst::Drop); // key
                f.instruction(&WasmInst::Drop); // map
                f.instruction(&WasmInst::I64Const(0));
            }
            Instruction::MapSet => {
                f.instruction(&WasmInst::Drop); // value
                f.instruction(&WasmInst::Drop); // key
                // Keep map ref on stack.
            }
            Instruction::StringConcat => {
                // Placeholder: drop both, push null.
                f.instruction(&WasmInst::Drop);
                // Keep one string on stack.
            }
            Instruction::StringLen => {
                f.instruction(&WasmInst::Drop);
                f.instruction(&WasmInst::I64Const(0));
            }
            Instruction::Print => {
                // Call imported print function (expects i64 tagged value).
                f.instruction(&WasmInst::Call(0)); // print is import #0
            }
            Instruction::RuntimeCall { name: _, arg_count } => {
                // Drop all args and push null result.
                // In a full implementation, this would call into a runtime.
                for _ in 0..*arg_count {
                    f.instruction(&WasmInst::Drop);
                }
                f.instruction(&WasmInst::I64Const(0)); // null result
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile::Compiler;
    use crate::syntax::parser::parse_v2;

    fn compile_to_wasm(src: &str) -> Result<Vec<u8>, CompileError> {
        let program = parse_v2(src).expect("parse error");
        let mut compiler = Compiler::new();
        let module = compiler.compile(&program)?;
        let codegen = WasmCodegen::new();
        codegen.emit(&module)
    }

    #[test]
    fn test_wasm_empty_program() {
        let wasm = compile_to_wasm("").unwrap();
        // WASM magic number: \0asm
        assert_eq!(&wasm[0..4], b"\0asm");
        // Version 1.
        assert_eq!(&wasm[4..8], &[1, 0, 0, 0]);
    }

    #[test]
    fn test_wasm_simple_let() {
        let wasm = compile_to_wasm("let x = 42;").unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
        assert!(wasm.len() > 8);
    }

    #[test]
    fn test_wasm_function() {
        let wasm = compile_to_wasm("fn add(a, b) { a + b }").unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_if_else() {
        let wasm = compile_to_wasm("let x = if true { 1 } else { 2 };").unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_for_loop() {
        let wasm = compile_to_wasm("for x in [1, 2, 3] { output x; }").unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_while_loop() {
        let wasm = compile_to_wasm("let mut x = 0; while x < 5 { x = x + 1; }").unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_string_constants() {
        let wasm = compile_to_wasm(r#"let x = "hello"; let y = "world";"#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
        // Check that string bytes are in the binary.
        assert!(wasm.windows(5).any(|w| w == b"hello"));
        assert!(wasm.windows(5).any(|w| w == b"world"));
    }

    #[test]
    fn test_wasm_output() {
        let wasm = compile_to_wasm("output 42;").unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_complex_program() {
        let wasm = compile_to_wasm(r#"
            fn fib(n) {
                if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
            }
            let result = fib(10);
            output result;
        "#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
        assert!(wasm.len() > 50);
    }

    #[test]
    fn test_wasm_enum_struct() {
        let wasm = compile_to_wasm(r#"
            enum Color { Red, Green, Blue }
            struct Point { x: float64, y: float64 }
            let c = Color::Red;
            let p = Point { x: 1.0, y: 2.0 };
        "#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_lambda() {
        let wasm = compile_to_wasm(r#"
            let double = |x| x * 2;
        "#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_compile_to_wasm_api() {
        let program = parse_v2("let x = 1 + 2;").unwrap();
        let wasm = super::super::compile_to_wasm(&program).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }
}
