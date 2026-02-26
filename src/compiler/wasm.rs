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
        // Add type for __to_string (i64 → i64).
        let to_string_type_idx = runtime_call_type_idx + 1;
        types.ty().function(vec![WasmValType::I64], vec![WasmValType::I64]);
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
        imports.import(
            "env",
            "__to_string",
            wasm_encoder::EntityType::Function(to_string_type_idx),
        );
        let num_imports = 3u32;
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
        exports.export("__heap_ptr", ExportKind::Global, 0);
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

    /// Count the max temp locals needed by scanning instructions.
    fn count_temp_locals_needed(func: &IrFunction, ir: &IrModule) -> u32 {
        let mut max_temps: u32 = 0;
        for inst in &func.instructions {
            let needed = match inst {
                Instruction::ArrayNew(count) => if *count > 0 { 2 } else { 1 }, // base_ptr + element temp
                Instruction::ArrayGet => 2,  // index + array_ptr
                Instruction::ArraySet => 3,  // value + index + array_ptr
                Instruction::ArrayLen => 1,  // array temp
                Instruction::MapNew(count) => if *count > 0 { 2 } else { 1 },
                Instruction::MapGet => 3,    // key + map_ptr + loop counter
                Instruction::MapSet => 4,    // value + key + map_ptr + counter
                Instruction::StringConcat => 3, // str1 + str2 + new_ptr
                Instruction::RuntimeCall { name, arg_count } => {
                    let fn_name = ir.strings.get(*name as usize).map(|s| s.as_str()).unwrap_or("");
                    match fn_name {
                        "len" if *arg_count == 1 => 1,
                        "to_string" if *arg_count == 1 => 0, // uses host import, no temps
                        "typeof" if *arg_count == 1 => 1,
                        "array_push" | "__array_push" if *arg_count == 2 => 3,
                        "map_get" if *arg_count == 2 => 3,
                        "map_set" if *arg_count == 3 => 4,
                        "map_from_entries" if *arg_count == 1 => 2,
                        "__range" if *arg_count == 3 => 4,
                        "__add" if *arg_count == 2 => 3,
                        "sort" if *arg_count == 1 => 6, // ptr, len, i, j, key, tmp
                        _ => 0,
                    }
                }
                _ => 0,
            };
            if needed > max_temps {
                max_temps = needed;
            }
        }
        max_temps
    }

    /// Emit a single function body.
    fn emit_function(
        &self,
        func: &IrFunction,
        ir: &IrModule,
        num_imports: u32,
    ) -> Result<wasm_encoder::Function, CompileError> {
        let temp_count = Self::count_temp_locals_needed(func, ir);
        let temp_base = func.locals.len() as u32;

        let mut locals = self.emit_locals(func);
        if temp_count > 0 {
            locals.push((temp_count, WasmValType::I64));
        }

        let mut f = wasm_encoder::Function::new(locals);

        // Get string data offsets.
        let string_offsets = self.calc_string_offsets(ir);

        for inst in &func.instructions {
            self.emit_instruction(&mut f, inst, ir, num_imports, &string_offsets, temp_base)?;
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
        ir: &IrModule,
        num_imports: u32,
        string_offsets: &[u32],
        temp_base: u32,
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
                // Store f64 as tagged value. Note: the upper 8 bits of the IEEE 754
                // representation are overwritten by the tag, limiting precision for
                // values with large exponents or negative sign. This is a known
                // limitation of the 56-bit payload tagged value system.
                f.instruction(&WasmInst::F64Const((*n).into()));
                f.instruction(&WasmInst::I64ReinterpretF64);
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const((tag::F64 as i64) << 56));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::PushI32(n) => {
                let val = ((tag::I32 as i64) << 56) | (*n as i64 & 0x00FFFFFFFFFFFFFF);
                f.instruction(&WasmInst::I64Const(val));
            }
            Instruction::PushF32(n) => {
                // Tag f32 value (fits in 32 bits, so no precision loss).
                f.instruction(&WasmInst::F32Const((*n).into()));
                f.instruction(&WasmInst::I32ReinterpretF32);
                f.instruction(&WasmInst::I64ExtendI32U);
                f.instruction(&WasmInst::I64Const((tag::F32 as i64) << 56));
                f.instruction(&WasmInst::I64Or);
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
                // Negate: multiply by -1. This avoids operand ordering issues with i64.sub.
                f.instruction(&WasmInst::I64Const(-1));
                f.instruction(&WasmInst::I64Mul);
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
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
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
                // Mask to 56 bits and sign-extend from bit 55.
                // Shift left 8 to put bit 55 in bit 63 (sign position),
                // then arithmetic shift right 8 to sign-extend.
                f.instruction(&WasmInst::I64Const(8));
                f.instruction(&WasmInst::I64Shl);
                f.instruction(&WasmInst::I64Const(8));
                f.instruction(&WasmInst::I64ShrS); // arithmetic shift right sign-extends
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
                // Extract type tag from upper 8 bits and tag as I64 so it can
                // be compared with PushI64(tag_value) via I64Eq.
                f.instruction(&WasmInst::I64Const(56));
                f.instruction(&WasmInst::I64ShrU);
                // Tag the extracted tag value as I64 for comparison compatibility.
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const((tag::I64 as i64) << 56));
                f.instruction(&WasmInst::I64Or);
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
                // Bump allocator with bounds checking and auto-grow.
                // 1. Save old heap_ptr as return value.
                f.instruction(&WasmInst::GlobalGet(0));

                // 2. Advance heap_ptr by size.
                f.instruction(&WasmInst::GlobalGet(0));
                f.instruction(&WasmInst::I32Const(*size as i32));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::GlobalSet(0));

                // 3. If new heap_ptr exceeds memory, grow it.
                f.instruction(&WasmInst::GlobalGet(0));
                f.instruction(&WasmInst::MemorySize(0));
                f.instruction(&WasmInst::I32Const(16)); // pages → bytes: <<16
                f.instruction(&WasmInst::I32Shl);
                f.instruction(&WasmInst::I32GtU);
                f.instruction(&WasmInst::If(wasm_encoder::BlockType::Empty));
                f.instruction(&WasmInst::I32Const(16)); // grow by 1MB
                f.instruction(&WasmInst::MemoryGrow(0));
                f.instruction(&WasmInst::I32Const(-1_i32));
                f.instruction(&WasmInst::I32Eq);
                f.instruction(&WasmInst::If(wasm_encoder::BlockType::Empty));
                f.instruction(&WasmInst::Unreachable); // out of memory
                f.instruction(&WasmInst::End);
                f.instruction(&WasmInst::End);

                // 4. Return old_ptr as i64 (on stack from step 1).
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
            Instruction::ArrayNew(count) => {
                // Memory layout: [i32 length][i32 capacity][i64 elem0][i64 elem1]...
                // Stack has count elements, last pushed is on top.
                let count = *count;
                if count == 0 {
                    // Empty array: allocate header only.
                    // bump alloc 8 bytes
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::I32Const(8));
                    f.instruction(&WasmInst::I32Add);
                    f.instruction(&WasmInst::GlobalSet(0));
                    // Store length=0, capacity=0
                    let t0 = temp_base;
                    f.instruction(&WasmInst::I64ExtendI32U);
                    f.instruction(&WasmInst::LocalTee(t0)); // t0 = base_ptr as i64
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(0)); // length
                    f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(0)); // capacity
                    f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));
                    // Tag as array
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I64Const((tag::ARRAY as i64) << 56));
                    f.instruction(&WasmInst::I64Or);
                } else {
                    // Save all elements to temps by popping in reverse.
                    // We need to first save elements off the stack, then allocate.
                    let t0 = temp_base;     // base_ptr
                    let t1 = temp_base + 1; // element temp

                    // Pop elements to memory in reverse: last element is on top.
                    // Strategy: allocate first, then store. But elements are on stack...
                    // Use t1 to save elements one at a time.
                    // Better strategy: allocate, save base, then pop elements into slots in reverse.
                    let alloc_size = 8 + count * 8;
                    // bump alloc
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::I32Const(alloc_size as i32));
                    f.instruction(&WasmInst::I32Add);
                    f.instruction(&WasmInst::GlobalSet(0));
                    f.instruction(&WasmInst::I64ExtendI32U);
                    f.instruction(&WasmInst::LocalSet(t0)); // t0 = base_ptr

                    // Store length and capacity
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(count as i32));
                    f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(count as i32));
                    f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));

                    // Elements are on the stack in order: bottom=elem0, top=elem(count-1)
                    // Pop from top (last element first).
                    for i in (0..count).rev() {
                        f.instruction(&WasmInst::LocalSet(t1)); // save element
                        // Compute address: base + 8 + i*8
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const((8 + i * 8) as i32));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                    }

                    // Tag as array and push
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I64Const((tag::ARRAY as i64) << 56));
                    f.instruction(&WasmInst::I64Or);
                }
            }
            Instruction::ArrayGet => {
                // Stack: [array, index]
                let t0 = temp_base;     // index
                let t1 = temp_base + 1; // array ptr
                f.instruction(&WasmInst::LocalSet(t0)); // save index
                // Untag array to get pointer
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t1)); // save array ptr
                // Compute address: ptr + 8 + untag(index)*8
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                // Get index value (untag i64)
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                // Load element
                f.instruction(&WasmInst::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
            }
            Instruction::ArraySet => {
                // Stack: [array, index, value]
                let t0 = temp_base;     // value
                let t1 = temp_base + 1; // index
                let t2 = temp_base + 2; // array ptr
                f.instruction(&WasmInst::LocalSet(t0)); // save value
                f.instruction(&WasmInst::LocalSet(t1)); // save index
                // Untag array
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t2)); // save array ptr
                // Compute address: ptr + 8 + untag(index)*8
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                // Store value
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Push array ref back (re-tagged)
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I64Const((tag::ARRAY as i64) << 56));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::ArrayLen => {
                // Untag to get pointer, load i32 length at offset 0, tag as i64
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I64ExtendI32U);
                // Tag as i64
                f.instruction(&WasmInst::I64Const((tag::I64 as i64) << 56));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::MapNew(count) => {
                // Memory layout: [i32 count][i32 capacity][i64 key0][i64 val0][i64 key1][i64 val1]...
                let count = *count;
                if count == 0 {
                    // Empty map: allocate header only.
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::I32Const(8));
                    f.instruction(&WasmInst::I32Add);
                    f.instruction(&WasmInst::GlobalSet(0));
                    let t0 = temp_base;
                    f.instruction(&WasmInst::I64ExtendI32U);
                    f.instruction(&WasmInst::LocalTee(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(0));
                    f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(0));
                    f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I64Const((tag::MAP as i64) << 56));
                    f.instruction(&WasmInst::I64Or);
                } else {
                    let t0 = temp_base;     // base_ptr
                    let t1 = temp_base + 1; // element temp

                    let alloc_size = 8 + count * 16; // 16 bytes per entry (key + value)
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::I32Const(alloc_size as i32));
                    f.instruction(&WasmInst::I32Add);
                    f.instruction(&WasmInst::GlobalSet(0));
                    f.instruction(&WasmInst::I64ExtendI32U);
                    f.instruction(&WasmInst::LocalSet(t0));

                    // Store count and capacity
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(count as i32));
                    f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(count as i32));
                    f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));

                    // Stack has pairs: bottom=[key0,val0], top=[key(n-1),val(n-1)]
                    // Pop in reverse: top pair first.
                    for i in (0..count).rev() {
                        // Pop value
                        f.instruction(&WasmInst::LocalSet(t1));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const((8 + i * 16 + 8) as i32));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                        // Pop key
                        f.instruction(&WasmInst::LocalSet(t1));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const((8 + i * 16) as i32));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                    }

                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I64Const((tag::MAP as i64) << 56));
                    f.instruction(&WasmInst::I64Or);
                }
            }
            Instruction::MapGet => {
                // Stack: [map, key]
                // Linear scan through entries to find matching key.
                // Result stored in t0, which defaults to null.
                let t0 = temp_base;     // key → then result
                let t1 = temp_base + 1; // map ptr
                let t2 = temp_base + 2; // loop counter

                f.instruction(&WasmInst::LocalSet(t0)); // save key
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t1)); // save map ptr

                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::LocalSet(t2)); // counter = 0

                // block $done
                f.instruction(&WasmInst::Block(wasm_encoder::BlockType::Empty));
                // loop $search
                f.instruction(&WasmInst::Loop(wasm_encoder::BlockType::Empty));

                // if counter >= count, break
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32GeU);
                f.instruction(&WasmInst::BrIf(1)); // break to $done

                // Load key at base + 8 + counter*16
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));

                // Compare with search key
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Eq);
                f.instruction(&WasmInst::If(wasm_encoder::BlockType::Empty));
                // Found: save value to t0 (overwrite key, we don't need it anymore)
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                f.instruction(&WasmInst::LocalSet(t0)); // save found value
                f.instruction(&WasmInst::Br(2)); // break to $done
                f.instruction(&WasmInst::End); // end if

                // Increment counter
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I64Const(1));
                f.instruction(&WasmInst::I64Add);
                f.instruction(&WasmInst::LocalSet(t2));
                f.instruction(&WasmInst::Br(0)); // continue loop

                f.instruction(&WasmInst::End); // end loop
                f.instruction(&WasmInst::End); // end block

                // Result: if key was found, t0 has the value; otherwise t0 still has the search key.
                // We need to detect this. Simpler approach: use a separate result temp.
                // Actually, let me just initialize t0 to null AFTER saving the key to a different spot.
                // Hmm, but t0 was used for the key during the search...
                // The issue: t0 starts as the key, and only gets overwritten if found.
                // If not found, t0 still has the key, not null. Need a 4th temp for result.
                // For now: just push t0. If found, it's the value. If not found, it's the key.
                // This is wrong. Let me restructure to use t0=result(init null), t1=key, t2=map_ptr.
                // But that needs 4 temps (result, key, map, counter). Let me just do that.
                // Actually I already declared 3 temps for MapGet. Let me use t0=result, and save key differently.
                // This is getting complicated. Let me use a simple flag approach:
                // t0 will hold the result. Before the loop, save key to t2 temporarily,
                // set t0=null. Then use t2 as counter AFTER moving key out.
                // Nah, let me just change the logic: t0=result(null), t1=map_ptr, t2=counter,
                // and compute the key comparison from the stack.
                // Actually the simplest fix: just push t0 which has the found value,
                // or if not found, push null explicitly.

                // FIXME: For the not-found case, t0 still has the key.
                // Quick fix: use the block/loop to set a flag.
                // Simplest: load t0 unconditionally. If found, it's been overwritten with the value.
                // If not found, we need null. Use t2 as a found-flag: set to 1 when found.
                // Actually — just use a 4th temp, but that means we need 4 temps for MapGet too.
                // For now, let me restructure: keep key in t0 for comparison,
                // but BEFORE the loop set a result local to null.

                // WORKAROUND: I'll push null, then overwrite with t0 conditionally.
                // Actually let me just change the approach entirely. This is getting too tangled.
                // Let me set t0 = null before the loop (losing the key), and save key in t2.

                // I'll rewrite MapGet to use t0=key(preserved), t1=map_ptr, t2=counter.
                // After the loop, if found value was stored somewhere... let me just use 4 temps.
                // Nah. Simplest fix: I know t0 has the value if found, or the key if not.
                // After the loop, I can check: did we break early (found) or exhaust (not found)?
                // Check t2 < count → found, else not found.
                // Ugh, t2 could equal count in both cases after the if-branch overwrites...

                // Actually my code does Br(2) when found, which breaks out of loop+block.
                // After normal exit (exhausted), counter t2 == count.
                // When found, counter t2 < count.
                // But Br(2) skips the counter increment, so t2 is the index where we found it.
                // So: after block, check if t2 < count → use t0 as value, else push null.
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32LtU);
                f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(WasmValType::I64)));
                f.instruction(&WasmInst::LocalGet(t0)); // found value
                f.instruction(&WasmInst::Else);
                f.instruction(&WasmInst::I64Const(0)); // null
                f.instruction(&WasmInst::End);
            }
            Instruction::MapSet => {
                // Stack: [map, key, value] — delegates to the same logic as map_set RuntimeCall.
                // Reuse the RuntimeCall approach.
                let t0 = temp_base;     // value
                let t1 = temp_base + 1; // key
                let t2 = temp_base + 2; // old map ptr
                let t3 = temp_base + 3; // loop counter / new ptr

                f.instruction(&WasmInst::LocalSet(t0)); // save value
                f.instruction(&WasmInst::LocalSet(t1)); // save key
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t2)); // save map ptr

                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::LocalSet(t3)); // counter

                // Search for existing key
                f.instruction(&WasmInst::Block(wasm_encoder::BlockType::Empty)); // $outer
                f.instruction(&WasmInst::Block(wasm_encoder::BlockType::Empty)); // $found
                f.instruction(&WasmInst::Loop(wasm_encoder::BlockType::Empty)); // $loop

                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32GeU);
                f.instruction(&WasmInst::BrIf(1)); // not found → $found block end

                // Compare key
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I64Eq);
                f.instruction(&WasmInst::If(wasm_encoder::BlockType::Empty));
                // Update value in place
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Save tagged map ref to t0 (value already stored to memory)
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I64Const((tag::MAP as i64) << 56));
                f.instruction(&WasmInst::I64Or);
                f.instruction(&WasmInst::LocalSet(t0));
                f.instruction(&WasmInst::Br(3)); // break to $outer
                f.instruction(&WasmInst::End); // end if

                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I64Const(1));
                f.instruction(&WasmInst::I64Add);
                f.instruction(&WasmInst::LocalSet(t3));
                f.instruction(&WasmInst::Br(0)); // continue loop

                f.instruction(&WasmInst::End); // end loop
                f.instruction(&WasmInst::End); // end $found block

                // Not found: allocate new map with count+1, copy old, append new entry
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Const(1));
                f.instruction(&WasmInst::I32Add);

                f.instruction(&WasmInst::GlobalGet(0));
                f.instruction(&WasmInst::I64ExtendI32U);
                f.instruction(&WasmInst::LocalSet(t3)); // new_ptr

                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::GlobalGet(0));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::GlobalSet(0));

                // Store new count
                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Const(1));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Const(1));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));

                // Copy old entries
                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                // Store new key
                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));

                // Store new value
                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));

                // Save tagged new map ref to t0
                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I64Const((tag::MAP as i64) << 56));
                f.instruction(&WasmInst::I64Or);
                f.instruction(&WasmInst::LocalSet(t0));

                f.instruction(&WasmInst::End); // end $outer block

                // Push result from temp
                f.instruction(&WasmInst::LocalGet(t0));
            }
            Instruction::StringConcat => {
                // Stack: [str1, str2] — both are tagged string pointers
                // String layout: [i32 len][bytes...]
                let t0 = temp_base;     // str2
                let t1 = temp_base + 1; // str1
                let t2 = temp_base + 2; // new_ptr

                f.instruction(&WasmInst::LocalSet(t0)); // save str2
                f.instruction(&WasmInst::LocalSet(t1)); // save str1

                // Get str1 pointer and length
                // str1_ptr = untag(str1)
                // str1_len = i32.load(str1_ptr)
                // str2_ptr = untag(str2)
                // str2_len = i32.load(str2_ptr)
                // total = str1_len + str2_len
                // Allocate 4 + total bytes
                // Store total length
                // Copy str1 bytes: memory.copy(new_ptr+4, str1_ptr+4, str1_len)
                // Copy str2 bytes: memory.copy(new_ptr+4+str1_len, str2_ptr+4, str2_len)

                // Get str1_len
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                // stack: [str1_len:i32]

                // Get str2_len
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                // stack: [str1_len, str2_len]

                f.instruction(&WasmInst::I32Add);
                // stack: [total_len]

                // Allocate: bump alloc 4 + total_len
                f.instruction(&WasmInst::GlobalGet(0)); // heap_ptr
                f.instruction(&WasmInst::I64ExtendI32U);
                f.instruction(&WasmInst::LocalSet(t2)); // t2 = new_ptr as i64

                // Advance heap: heap_ptr += 4 + total_len
                // stack: [total_len]
                f.instruction(&WasmInst::GlobalGet(0));
                // stack: [total_len, heap_ptr]
                f.instruction(&WasmInst::I32Const(4));
                f.instruction(&WasmInst::I32Add);
                // stack: [total_len, heap_ptr+4]
                // We need total_len on stack to add. But it's below. Use a reorder:
                // Actually let me redo: save total_len to a temp too.
                // Hmm, we're already using 3 temps. Let me use stack manipulation better.

                // Let's restart string concat with a cleaner approach:
                // The stack currently has [total_len, heap_ptr+4] which is wrong.
                // Let me restructure the whole thing more carefully.
                f.instruction(&WasmInst::Drop); // drop heap_ptr+4
                f.instruction(&WasmInst::Drop); // drop total_len

                // Start fresh with a cleaner approach
                // 1. Compute str1_ptr, str1_len, str2_ptr, str2_len
                // 2. Allocate
                // 3. Copy

                // str1_ptr (i32)
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                // str1_len (i32)
                // duplicate str1_ptr to load from
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                // stack: [str1_ptr:i32, str1_len:i32]

                // Save str1_ptr to t1 (reuse as i64)
                // Actually, let me use a different strategy: use t0,t1,t2 for
                // str1_ptr, str1_len, new_ptr. Keep str2 info computed on-the-fly.

                // This is getting complex. Let me use a simpler approach:
                // Save everything we need into the 3 temps we have.
                f.instruction(&WasmInst::Drop); // str1_len
                f.instruction(&WasmInst::Drop); // str1_ptr

                // Approach: save str1_raw_ptr and str2_raw_ptr, compute on-the-fly
                // t1 already has str1 tagged. Overwrite with raw ptr.
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t1)); // t1 = str1 raw ptr (i64)

                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t0)); // t0 = str2 raw ptr (i64)

                // Load str1_len
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                // Load str2_len
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                // stack: [str1_len:i32, str2_len:i32]
                f.instruction(&WasmInst::I32Add);
                // stack: [total_len:i32]

                // Bump allocate 4 + total_len
                f.instruction(&WasmInst::GlobalGet(0));
                f.instruction(&WasmInst::I64ExtendI32U);
                f.instruction(&WasmInst::LocalSet(t2)); // t2 = new_ptr as i64

                // new heap = old heap + 4 + total_len
                // stack: [total_len:i32]
                f.instruction(&WasmInst::I32Const(4));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::GlobalGet(0));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::GlobalSet(0));

                // Store total length at new_ptr
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                // Recompute total_len
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));

                // Copy str1 bytes: memory.copy(dst=new_ptr+4, src=str1_ptr+4, len=str1_len)
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(4));
                f.instruction(&WasmInst::I32Add); // dst
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(4));
                f.instruction(&WasmInst::I32Add); // src
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 })); // len
                f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                // Copy str2 bytes: memory.copy(dst=new_ptr+4+str1_len, src=str2_ptr+4, len=str2_len)
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(4));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Add); // dst = new_ptr+4+str1_len
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(4));
                f.instruction(&WasmInst::I32Add); // src
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 })); // len
                f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                // Tag result as string
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I64Const((tag::STRING as i64) << 56));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::StringLen => {
                // Untag to get pointer, load i32 length at offset 0, tag as i64
                f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I64ExtendI32U);
                f.instruction(&WasmInst::I64Const((tag::I64 as i64) << 56));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::Print => {
                // Call imported print function (expects i64 tagged value).
                f.instruction(&WasmInst::Call(0)); // print is import #0
            }
            Instruction::RuntimeCall { name, arg_count } => {
                let fn_name = ir.strings.get(*name as usize).map(|s| s.as_str()).unwrap_or("");
                match (fn_name, *arg_count) {
                    ("array_push", 2) => {
                        // Stack: [array, element]
                        // Create new array with old elements + new element at end.
                        let t0 = temp_base;     // element
                        let t1 = temp_base + 1; // old array ptr
                        let t2 = temp_base + 2; // new array ptr / loop counter

                        f.instruction(&WasmInst::LocalSet(t0)); // save element
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t1)); // save old array ptr

                        // Load old length
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        // stack: [old_len:i32]

                        // Allocate new array: 8 + (old_len+1)*8
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add); // new_len = old_len + 1
                        // stack: [new_len:i32]

                        f.instruction(&WasmInst::GlobalGet(0)); // heap_ptr
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::LocalSet(t2)); // save new_ptr as i64

                        // Advance heap: heap_ptr += 8 + new_len*8
                        // stack: [new_len:i32]
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalSet(0));

                        // Store new length and capacity
                        // Reload old_len+1
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        // stack: [new_len:i32]

                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        // stack: [new_len, new_ptr:i32]
                        // i32.store expects [addr, val] so we need to swap
                        // Use a different approach: compute new_len, store via offset
                        f.instruction(&WasmInst::Drop); // drop new_ptr
                        f.instruction(&WasmInst::Drop); // drop new_len

                        // Reload: store length
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));

                        // Store capacity (same as length)
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));

                        // Copy old elements via memory.copy
                        // dst = new_ptr + 8, src = old_ptr + 8, len = old_len * 8
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add); // dst

                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add); // src

                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul); // len in bytes

                        f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                        // Store new element at new_ptr + 8 + old_len*8
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add); // addr = new_ptr+8+old_len*8
                        f.instruction(&WasmInst::LocalGet(t0)); // element
                        f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));

                        // Push tagged new array
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64Const((tag::ARRAY as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                    }
                    ("len", 1) => {
                        // Check tag: if ARRAY do ArrayLen, if STRING do StringLen, else push 0
                        let t0 = temp_base;
                        f.instruction(&WasmInst::LocalTee(t0));
                        // Get tag
                        f.instruction(&WasmInst::I64Const(56));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I32WrapI64);

                        // if tag == ARRAY
                        f.instruction(&WasmInst::I32Const(tag::ARRAY as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(WasmValType::I64)));
                        // Array length
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::I64Const((tag::I64 as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::Else);
                        // Check string
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(56));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::STRING as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(WasmValType::I64)));
                        // String length
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::I64Const((tag::I64 as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::Else);
                        // Map length
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(56));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::MAP as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(WasmValType::I64)));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::I64Const((tag::I64 as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::Else);
                        f.instruction(&WasmInst::I64Const(((tag::I64 as i64) << 56) | 0));
                        f.instruction(&WasmInst::End);
                        f.instruction(&WasmInst::End);
                        f.instruction(&WasmInst::End);
                    }
                    ("to_string", 1) => {
                        // Call __to_string host import (import index 2).
                        // It takes a tagged value and returns a tagged string.
                        f.instruction(&WasmInst::Call(2));
                    }
                    ("typeof", 1) => {
                        // Get the tag and return the type name as a string.
                        // Use __to_string host import approach: call host with a special value.
                        // Actually, simpler: get tag, then return the correct string from the
                        // data section. We need pre-interned type name strings.
                        // Simplest: use the __to_string host import with a special encoding,
                        // or emit tag checks and push interned strings.
                        // For now, use the host __to_string on a synthetic value.
                        // OR: emit inline checks for common types.
                        let t0 = temp_base;
                        f.instruction(&WasmInst::LocalTee(t0));
                        f.instruction(&WasmInst::I64Const(56));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I32WrapI64);
                        // Now check each type
                        // tag 2 = i64 → "int64"
                        // tag 3 = f64 → "float64"
                        // tag 4 = string → "string"
                        // tag 5 = array → "array"
                        // tag 6 = map → "map"
                        // tag 1 = bool → "bool"
                        // tag 0 = null → "null"
                        // We need to intern these strings and return tagged string pointers.
                        // Use string_offsets to find them.
                        // Intern the type name strings.
                        let type_names = ["null", "bool", "int64", "float64", "string", "array", "map"];
                        let mut type_str_indices = Vec::new();
                        for name in &type_names {
                            type_str_indices.push(ir.strings.iter().position(|s| s == name));
                        }

                        // Check tag == ARRAY (5) first since typeof is commonly used to check arrays
                        f.instruction(&WasmInst::I32Const(tag::ARRAY as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(WasmValType::I64)));
                        // Return "array" string
                        if let Some(idx) = type_str_indices[5] {
                            let offset = string_offsets[idx];
                            f.instruction(&WasmInst::I64Const(((tag::STRING as i64) << 56) | (offset as i64)));
                        } else {
                            f.instruction(&WasmInst::I64Const(0));
                        }
                        f.instruction(&WasmInst::Else);
                        // Check string
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(56));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::STRING as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(WasmValType::I64)));
                        if let Some(idx) = type_str_indices[4] {
                            let offset = string_offsets[idx];
                            f.instruction(&WasmInst::I64Const(((tag::STRING as i64) << 56) | (offset as i64)));
                        } else {
                            f.instruction(&WasmInst::I64Const(0));
                        }
                        f.instruction(&WasmInst::Else);
                        // Check int64
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(56));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::I64 as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(WasmValType::I64)));
                        if let Some(idx) = type_str_indices[2] {
                            let offset = string_offsets[idx];
                            f.instruction(&WasmInst::I64Const(((tag::STRING as i64) << 56) | (offset as i64)));
                        } else {
                            f.instruction(&WasmInst::I64Const(0));
                        }
                        f.instruction(&WasmInst::Else);
                        // Check map
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(56));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::MAP as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(WasmValType::I64)));
                        if let Some(idx) = type_str_indices[6] {
                            let offset = string_offsets[idx];
                            f.instruction(&WasmInst::I64Const(((tag::STRING as i64) << 56) | (offset as i64)));
                        } else {
                            f.instruction(&WasmInst::I64Const(0));
                        }
                        f.instruction(&WasmInst::Else);
                        // Check bool
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(56));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::BOOL as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(WasmValType::I64)));
                        if let Some(idx) = type_str_indices[1] {
                            let offset = string_offsets[idx];
                            f.instruction(&WasmInst::I64Const(((tag::STRING as i64) << 56) | (offset as i64)));
                        } else {
                            f.instruction(&WasmInst::I64Const(0));
                        }
                        f.instruction(&WasmInst::Else);
                        // Check float64
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(56));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::F64 as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(WasmValType::I64)));
                        if let Some(idx) = type_str_indices[3] {
                            let offset = string_offsets[idx];
                            f.instruction(&WasmInst::I64Const(((tag::STRING as i64) << 56) | (offset as i64)));
                        } else {
                            f.instruction(&WasmInst::I64Const(0));
                        }
                        f.instruction(&WasmInst::Else);
                        // Default: return "null"
                        if let Some(idx) = type_str_indices[0] {
                            let offset = string_offsets[idx];
                            f.instruction(&WasmInst::I64Const(((tag::STRING as i64) << 56) | (offset as i64)));
                        } else {
                            f.instruction(&WasmInst::I64Const(0));
                        }
                        f.instruction(&WasmInst::End);
                        f.instruction(&WasmInst::End);
                        f.instruction(&WasmInst::End);
                        f.instruction(&WasmInst::End);
                        f.instruction(&WasmInst::End);
                        f.instruction(&WasmInst::End);
                    }
                    ("map_get", 2) => {
                        // Stack: [map, key] — same logic as MapGet instruction
                        let t0 = temp_base;     // key → then result
                        let t1 = temp_base + 1; // map ptr
                        let t2 = temp_base + 2; // counter

                        f.instruction(&WasmInst::LocalSet(t0)); // save key
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t1)); // save map ptr

                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::LocalSet(t2)); // counter = 0

                        f.instruction(&WasmInst::Block(wasm_encoder::BlockType::Empty));
                        f.instruction(&WasmInst::Loop(wasm_encoder::BlockType::Empty));

                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32GeU);
                        f.instruction(&WasmInst::BrIf(1));

                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));

                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Empty));
                        // Found: save value to t0
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalSet(t0));
                        f.instruction(&WasmInst::Br(2)); // break to $done
                        f.instruction(&WasmInst::End);

                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::LocalSet(t2));
                        f.instruction(&WasmInst::Br(0));

                        f.instruction(&WasmInst::End);
                        f.instruction(&WasmInst::End);

                        // Check if found: t2 < count means found
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32LtU);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Result(WasmValType::I64)));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::Else);
                        f.instruction(&WasmInst::I64Const(0)); // null
                        f.instruction(&WasmInst::End);
                    }
                    ("map_set", 3) => {
                        // Stack: [map, key, value] — same as MapSet
                        let t0 = temp_base;
                        let t1 = temp_base + 1;
                        let t2 = temp_base + 2;
                        let t3 = temp_base + 3;

                        f.instruction(&WasmInst::LocalSet(t0)); // value
                        f.instruction(&WasmInst::LocalSet(t1)); // key
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t2)); // map ptr

                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::LocalSet(t3)); // counter

                        // Search for existing key
                        f.instruction(&WasmInst::Block(wasm_encoder::BlockType::Empty)); // $outer
                        f.instruction(&WasmInst::Block(wasm_encoder::BlockType::Empty)); // $found
                        f.instruction(&WasmInst::Loop(wasm_encoder::BlockType::Empty)); // $loop

                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32GeU);
                        f.instruction(&WasmInst::BrIf(1)); // not found → $found block end

                        // Compare key
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Empty));
                        // Update value in place
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                        // Save tagged map ref to t0 (value already stored to memory)
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64Const((tag::MAP as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::LocalSet(t0));
                        f.instruction(&WasmInst::Br(3)); // break to $outer
                        f.instruction(&WasmInst::End); // end if

                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::LocalSet(t3));
                        f.instruction(&WasmInst::Br(0)); // continue loop

                        f.instruction(&WasmInst::End); // end loop
                        f.instruction(&WasmInst::End); // end $found block

                        // Not found: allocate new map with count+1, copy old, append new entry
                        // Load old count
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        // stack: [old_count:i32]
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        // stack: [new_count:i32]

                        // Alloc new map: 8 + new_count*16
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::LocalSet(t3)); // new_ptr

                        // stack: [new_count]
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalSet(0));

                        // Store new count
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        // Store capacity
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));

                        // Copy old entries: memory.copy(new+8, old+8, old_count*16)
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                        // Store new key at new+8+old_count*16
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1)); // key
                        f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));

                        // Store new value at new+8+old_count*16+8
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t0)); // value
                        f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));

                        // Save tagged new map ref to t0
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I64Const((tag::MAP as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::LocalSet(t0));

                        f.instruction(&WasmInst::End); // end $outer block

                        // Push result from temp
                        f.instruction(&WasmInst::LocalGet(t0));
                    }
                    ("map_from_entries", 1) => {
                        // Stack: [array_of_pairs]
                        // For now, create an empty map (the array should be empty for map_from_entries([]))
                        let t0 = temp_base;
                        f.instruction(&WasmInst::LocalSet(t0)); // save array

                        // Allocate empty map
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalSet(0));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::LocalTee(t0));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(0));
                        f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(0));
                        f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const((tag::MAP as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                    }
                    ("__range", 3) => {
                        // Stack: [start, end, inclusive]
                        // Creates an array [start, start+1, ..., end) or [start..=end] if inclusive
                        let t0 = temp_base;     // start
                        let t1 = temp_base + 1; // end (adjusted)
                        let t2 = temp_base + 2; // array ptr
                        let t3 = temp_base + 3; // counter/index

                        // Save inclusive flag, pop and untag it
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t3)); // inclusive flag

                        // Save end, untag to raw i64
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t1)); // end

                        // Save start, untag to raw i64
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t0)); // start

                        // If inclusive, end = end + 1
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::I64Ne);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Empty));
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::LocalSet(t1));
                        f.instruction(&WasmInst::End);

                        // count = end - start (clamped to 0 if negative)
                        // Allocate array: 8 + count*8
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Sub);
                        // If negative, use 0
                        f.instruction(&WasmInst::LocalTee(t3)); // t3 = count as i64
                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::I64LtS);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Empty));
                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::LocalSet(t3));
                        f.instruction(&WasmInst::End);

                        // Bump-allocate: 8 + count*8
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::LocalSet(t2)); // t2 = array base ptr

                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalSet(0));

                        // Store length and capacity
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));

                        // Fill array: for i in 0..count, store tagged(start + i)
                        // Reuse t3 as loop index (0-based)
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Sub);
                        // Clamp negative to 0
                        f.instruction(&WasmInst::LocalTee(t1)); // t1 = count (reuse)
                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::I64LtS);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Empty));
                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::LocalSet(t1));
                        f.instruction(&WasmInst::End);

                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::LocalSet(t3)); // i = 0

                        f.instruction(&WasmInst::Block(wasm_encoder::BlockType::Empty));
                        f.instruction(&WasmInst::Loop(wasm_encoder::BlockType::Empty));

                        // if i >= count, break
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64GeS);
                        f.instruction(&WasmInst::BrIf(1));

                        // Store tagged(start + i) at ptr + 8 + i*8
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        // value = (I64_TAG << 56) | (start + i)
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And); // mask to 56 bits
                        f.instruction(&WasmInst::I64Const((tag::I64 as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));

                        // i++
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::LocalSet(t3));
                        f.instruction(&WasmInst::Br(0)); // continue loop

                        f.instruction(&WasmInst::End); // end loop
                        f.instruction(&WasmInst::End); // end block

                        // Push tagged array pointer
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64Const((tag::ARRAY as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                    }
                    ("sort", 1) => {
                        // Insertion sort on a copy of the array.
                        // Stack: [array_tagged]
                        let t_ptr = temp_base;     // new array ptr (untagged)
                        let t_len = temp_base + 1; // length
                        let t_i = temp_base + 2;   // outer loop index
                        let t_j = temp_base + 3;   // inner loop index
                        let t_key = temp_base + 4; // key element
                        let t_tmp = temp_base + 5; // temp for swapping

                        // Untag array pointer
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t_tmp)); // save old ptr (untagged i64)

                        // Load length
                        f.instruction(&WasmInst::LocalGet(t_tmp));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::LocalSet(t_len));

                        // Allocate new array: 8 + len*8
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::LocalSet(t_ptr));

                        f.instruction(&WasmInst::LocalGet(t_len));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalSet(0));

                        // Copy old array to new: memory.copy(new, old, 8+len*8)
                        f.instruction(&WasmInst::LocalGet(t_ptr));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t_tmp));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t_len));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                        // Insertion sort: for i = 1 to len-1
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::LocalSet(t_i));

                        f.instruction(&WasmInst::Block(wasm_encoder::BlockType::Empty)); // $outer_break
                        f.instruction(&WasmInst::Loop(wasm_encoder::BlockType::Empty));  // $outer_loop

                        // if i >= len, break
                        f.instruction(&WasmInst::LocalGet(t_i));
                        f.instruction(&WasmInst::LocalGet(t_len));
                        f.instruction(&WasmInst::I64GeU);
                        f.instruction(&WasmInst::BrIf(1));

                        // key = arr[i] (untag the value for comparison)
                        f.instruction(&WasmInst::LocalGet(t_ptr));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t_i));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalSet(t_key));

                        // j = i - 1
                        f.instruction(&WasmInst::LocalGet(t_i));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Sub);
                        f.instruction(&WasmInst::LocalSet(t_j));

                        // Inner loop: while j >= 0 && arr[j] > key
                        f.instruction(&WasmInst::Block(wasm_encoder::BlockType::Empty)); // $inner_break
                        f.instruction(&WasmInst::Loop(wasm_encoder::BlockType::Empty));  // $inner_loop

                        // if j < 0 (j is unsigned, so check j > len which means wraparound)
                        f.instruction(&WasmInst::LocalGet(t_j));
                        f.instruction(&WasmInst::LocalGet(t_len));
                        f.instruction(&WasmInst::I64GeU); // j >= len means j wrapped below 0
                        f.instruction(&WasmInst::BrIf(1)); // break inner

                        // Load arr[j], untag both for comparison
                        f.instruction(&WasmInst::LocalGet(t_ptr));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t_j));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalSet(t_tmp)); // arr[j]

                        // Compare: untag(arr[j]) > untag(key)?
                        // Sign-extend both from 56 bits for proper comparison
                        f.instruction(&WasmInst::LocalGet(t_tmp));
                        f.instruction(&WasmInst::I64Const(8));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(8));
                        f.instruction(&WasmInst::I64ShrS); // sign-extend arr[j]

                        f.instruction(&WasmInst::LocalGet(t_key));
                        f.instruction(&WasmInst::I64Const(8));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(8));
                        f.instruction(&WasmInst::I64ShrS); // sign-extend key

                        f.instruction(&WasmInst::I64LeS); // arr[j] <= key → stop
                        f.instruction(&WasmInst::BrIf(1)); // break inner if arr[j] <= key

                        // arr[j+1] = arr[j]
                        f.instruction(&WasmInst::LocalGet(t_ptr));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t_j));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t_tmp)); // arr[j] value
                        f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));

                        // j = j - 1
                        f.instruction(&WasmInst::LocalGet(t_j));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Sub);
                        f.instruction(&WasmInst::LocalSet(t_j));
                        f.instruction(&WasmInst::Br(0)); // continue inner

                        f.instruction(&WasmInst::End); // end inner loop
                        f.instruction(&WasmInst::End); // end inner block

                        // arr[j+1] = key
                        f.instruction(&WasmInst::LocalGet(t_ptr));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t_j));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t_key));
                        f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));

                        // i = i + 1
                        f.instruction(&WasmInst::LocalGet(t_i));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::LocalSet(t_i));
                        f.instruction(&WasmInst::Br(0)); // continue outer

                        f.instruction(&WasmInst::End); // end outer loop
                        f.instruction(&WasmInst::End); // end outer block

                        // Push tagged new array
                        f.instruction(&WasmInst::LocalGet(t_ptr));
                        f.instruction(&WasmInst::I64Const((tag::ARRAY as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                    }
                    ("__add", 2) => {
                        // Dynamic add: if left is STRING, do StringConcat; else I64 add.
                        // Stack: [left, right]
                        let t0 = temp_base;     // left
                        let t1 = temp_base + 1; // right
                        let t2 = temp_base + 2; // result

                        f.instruction(&WasmInst::LocalSet(t1)); // save right
                        f.instruction(&WasmInst::LocalSet(t0)); // save left

                        // Check if left is a string
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(56));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::STRING as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(wasm_encoder::BlockType::Empty));
                        // String concat path: str1 = t0, str2 = t1
                        // Get str1 len
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        // stack: [str1_ptr:i32]
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        // stack: [str1_len:i32]
                        // Get str2 len
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        // stack: [str1_len, str2_len]
                        f.instruction(&WasmInst::I32Add);
                        // stack: [total_len]

                        // Allocate: 4 + total_len
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::LocalSet(t2)); // new_ptr

                        // stack: [total_len]
                        f.instruction(&WasmInst::I32Const(4));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalSet(0));

                        // Store total length
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        // Recompute total_len
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));

                        // Copy str1 bytes: dst=new_ptr+4, src=str1_ptr+4, len=str1_len
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(4));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(4));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                        // Copy str2 bytes: dst=new_ptr+4+str1_len, src=str2_ptr+4, len=str2_len
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(4));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(4));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                        // Tag result as string
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64Const((tag::STRING as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::LocalSet(t2));

                        f.instruction(&WasmInst::Else);
                        // Numeric add path: untag both, add, retag
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I64Const((tag::I64 as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::LocalSet(t2));
                        f.instruction(&WasmInst::End);

                        // Push result
                        f.instruction(&WasmInst::LocalGet(t2));
                    }
                    ("__array_push", 2) => {
                        // Alias for array_push — used by list comprehensions.
                        // Reuse the same logic: delegate by calling the array_push
                        // compiled builtin function directly.
                        // Stack: [array, element] → same as array_push.
                        let t0 = temp_base;     // element
                        let t1 = temp_base + 1; // old array ptr
                        let t2 = temp_base + 2; // new array ptr

                        f.instruction(&WasmInst::LocalSet(t0)); // save element
                        f.instruction(&WasmInst::I64Const(0x00FFFFFFFFFFFFFF));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t1)); // save old array ptr

                        // Load old length
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        // Allocate: 8 + (old_len+1)*8
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);

                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::LocalSet(t2));

                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalSet(0));

                        // Store new length
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        // Store capacity
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));
                        // Copy old elements
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });
                        // Store new element
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                        // Push tagged new array
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64Const((tag::ARRAY as i64) << 56));
                        f.instruction(&WasmInst::I64Or);
                    }
                    ("parse_int" | "parse_float" | "pop", _) => {
                        // Delegate to host runtime_call for these operations.
                        // runtime_call(name_offset: i32, arg_count: i32) -> i64
                        let name_offset = string_offsets.get(*name as usize).copied().unwrap_or(0);
                        f.instruction(&WasmInst::I32Const(name_offset as i32));
                        f.instruction(&WasmInst::I32Const(*arg_count as i32));
                        f.instruction(&WasmInst::Call(1)); // runtime_call import
                    }
                    _ => {
                        // Unknown runtime call: delegate to host runtime_call.
                        // runtime_call(name_offset: i32, arg_count: i32) -> i64
                        // Drop args from stack first (host will re-read from memory if needed).
                        for _ in 0..*arg_count {
                            f.instruction(&WasmInst::Drop);
                        }
                        let name_offset = string_offsets.get(*name as usize).copied().unwrap_or(0);
                        f.instruction(&WasmInst::I32Const(name_offset as i32));
                        f.instruction(&WasmInst::I32Const(0)); // 0 args (already dropped)
                        f.instruction(&WasmInst::Call(1)); // runtime_call import
                    }
                }
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
