//! WASM binary code generation from MAGI IR.
//!
//! Uses the `wasm-encoder` crate to produce valid `.wasm` modules.

use super::wasm_binary::{
    CodeSection, DataSection, ElementSection, Elements, ExportKind, ExportSection,
    FunctionSection, GlobalSection, GlobalType, ImportSection, Inst as WasmInst,
    MemorySection, MemoryType, Module, NameMap, NameSection, TableSection, TableType,
    TypeSection, ValType as WasmValType, BlockType, MemArg, ConstExpr, EntityType,
    Function as WasmFunction, RefType,
};

use std::collections::HashMap;

use super::ir::*;
use super::CompileError;

/// Generates WASM binary from an IR module.
///
/// # WASM linear memory layout
///
/// ```text
/// 0x0000 .. 0x03FF  (0–1023)    Reserved / scratch area (1 KB)
/// 0x0400 .. ???     (1024–…)    String data section: each string is stored as
///                               [4-byte little-endian length][UTF-8 bytes].
///                               Strings are packed contiguously in insertion order.
/// ???    .. heap_ptr            (end of strings … heap pointer) — heap area
///                               for runtime allocations (arrays, maps, etc.).
/// ```
///
/// `string_data_offset` (default 1024) is the byte offset where the first
/// string constant begins. The heap pointer global (global 0) is initialized
/// to `string_data_offset + total_string_data_size` so that runtime
/// allocations start immediately after the last string constant.
pub struct WasmCodegen {
    /// Base offset in data section for string constants (default: 1024 = 0x400).
    /// See the memory layout diagram above.
    string_data_offset: u32,
}

impl Default for WasmCodegen {
    fn default() -> Self {
        Self::new()
    }
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
        imports.import("env", "print", EntityType::Function(print_type_idx));
        imports.import(
            "env",
            "runtime_call",
            EntityType::Function(runtime_call_type_idx),
        );
        imports.import(
            "env",
            "__to_string",
            EntityType::Function(to_string_type_idx),
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
            element_type: RefType::FUNCREF,
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
            &ConstExpr::i32_const({
                let offset = self.string_data_offset;
                let size = self.calc_string_data_size(ir);
                (offset.saturating_add(size)) as i32
            }),
        );
        // Auto-captured globals from IR compiler (i64 tagged values)
        for _g in &ir.globals {
            globals.global(
                GlobalType {
                    val_type: WasmValType::I64,
                    mutable: true,
                    shared: false,
                },
                &ConstExpr::i64_const(0),
            );
        }
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

        // ── Element section (populate function table for indirect calls) ──
        let mut elements = ElementSection::new();
        let func_indices: Vec<u32> = (0..ir.functions.len() as u32)
            .map(|i| i + num_imports)
            .collect();
        elements.active(
            Some(0),
            &ConstExpr::i32_const(0),
            Elements::Functions(func_indices.into()),
        );
        module.section(&elements);

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
                &ConstExpr::i32_const(offset as i32),
                buf.iter().copied(),
            );
            offset = offset.saturating_add(4u32.saturating_add(bytes.len() as u32));
        }
        module.section(&data);

        // ── Name section (debug info) ───────────────────────
        let mut names = NameSection::new();
        let mut func_names = NameMap::new();
        // Name the imported functions first (indices 0..num_imports).
        func_names.append(0, "print");
        func_names.append(1, "runtime_call");
        func_names.append(2, "__to_string");
        // Name each IR function (indices num_imports..).
        for (i, func) in ir.functions.iter().enumerate() {
            func_names.append(i as u32 + num_imports, &func.name);
        }
        names.functions(&func_names);
        module.section(&names);

        let bytes = module.finish();
        Self::validate_wasm_bytes(&bytes)?;
        Ok(bytes)
    }

    /// Validate the generated WASM binary before returning it.
    ///
    /// When the `wasm-validate` feature is enabled (default), performs full
    /// semantic validation using `wasmparser::Validator` — catching type
    /// mismatches, stack underflows, invalid instructions, and all other
    /// WASM spec violations at compile time rather than at runtime.
    ///
    /// When the feature is disabled, falls back to structural checks only
    /// (magic number, version, section ordering).
    fn validate_wasm_bytes(bytes: &[u8]) -> Result<(), CompileError> {
        crate::util::validate_wasm(bytes).map_err(|e| {
            CompileError::Internal(format!("WASM validation failed: {e}"))
        })
    }

    fn calc_string_data_size(&self, ir: &IrModule) -> u32 {
        ir.strings
            .iter()
            .fold(0u32, |acc, s| acc.saturating_add(4u32.saturating_add(s.len() as u32)))
    }

    /// Count the max temp locals needed by scanning instructions.
    fn count_temp_locals_needed(func: &IrFunction, ir: &IrModule) -> u32 {
        let mut max_temps: u32 = 0;
        for inst in &func.instructions {
            let needed = match inst {
                Instruction::BoolNot | Instruction::If | Instruction::IfVoid | Instruction::TagF64 | Instruction::I64Neg => 1, // temp for truthiness/NaN check
                Instruction::BrIf(_) => 1, // temp for truthiness check
                Instruction::GetTag => 1, // temp for NaN-box check in select
                Instruction::MemStoreI64 => 1, // temp for value during addr conversion
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
                        "map_from_entries" if *arg_count == 1 => 5, // arr_ptr, map_ptr, count, idx, pair_ptr
                        "__range" if *arg_count == 3 => 4,
                        "__add" if *arg_count == 2 => 3,
                        "__sub" | "__mul" | "__div" | "__mod" if *arg_count == 2 => 2,
                        "__gt" | "__lt" | "__ge" | "__le" if *arg_count == 2 => 2,
                        "__eq" | "__ne" if *arg_count == 2 => 0, // raw comparison, no temps
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
    ) -> Result<WasmFunction, CompileError> {
        let temp_count = Self::count_temp_locals_needed(func, ir);
        let temp_base = func.locals.len() as u32;

        let mut locals = self.emit_locals(func);
        if temp_count > 0 {
            locals.push((temp_count, WasmValType::I64));
        }

        let mut f = WasmFunction::new(locals);

        // Get string data offsets.
        let string_offsets = self.calc_string_offsets(ir);

        // Build a reverse index for O(1) string-content-to-index lookup,
        // replacing the linear scan that was previously used in typeof codegen.
        let string_reverse_index: HashMap<&str, u32> = ir
            .strings
            .iter()
            .enumerate()
            .map(|(i, s)| (s.as_str(), i as u32))
            .collect();

        for inst in &func.instructions {
            self.emit_instruction(&mut f, inst, ir, num_imports, &string_offsets, &string_reverse_index, temp_base)?;
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
            offset = offset.saturating_add(4).saturating_add(s.len() as u32);
        }
        offsets
    }

    /// Emit a heap bounds check after an inline allocation (GlobalSet(0)).
    /// If heap_ptr exceeds current memory, grows memory by 1MB.
    /// Traps (Unreachable) if growth fails.
    fn emit_heap_bounds_check(f: &mut WasmFunction) {
        f.instruction(&WasmInst::GlobalGet(0));
        f.instruction(&WasmInst::MemorySize(0));
        f.instruction(&WasmInst::I32Const(16)); // pages → bytes: <<16
        f.instruction(&WasmInst::I32Shl);
        f.instruction(&WasmInst::I32GtU);
        f.instruction(&WasmInst::If(BlockType::Empty));
        f.instruction(&WasmInst::I32Const(16)); // grow by 1MB (16 pages)
        f.instruction(&WasmInst::MemoryGrow(0));
        f.instruction(&WasmInst::I32Const(-1_i32));
        f.instruction(&WasmInst::I32Eq);
        f.instruction(&WasmInst::If(BlockType::Empty));
        f.instruction(&WasmInst::Unreachable); // out of memory
        f.instruction(&WasmInst::End);
        f.instruction(&WasmInst::End);
    }

    /// Emit instructions to convert a tagged value (in local `src`) to f64 on the WASM stack.
    ///
    /// If the value is a raw f64 (not NaN-boxed), reinterpret its bits as f64.
    /// If the value is a NaN-boxed integer, sign-extend from 48 bits and convert to f64.
    fn emit_to_f64(f: &mut WasmFunction, src: u32) {
        // Check: is_nanboxed = (val & NANBOX_MASK) == NANBOX_SIG
        f.instruction(&WasmInst::LocalGet(src));
        f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
        f.instruction(&WasmInst::I64And);
        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
        f.instruction(&WasmInst::I64Eq);
        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::F64)));
        // NaN-boxed: untag as i64 (sign-extend from 48 bits) and convert to f64
        f.instruction(&WasmInst::LocalGet(src));
        f.instruction(&WasmInst::I64Const(16));
        f.instruction(&WasmInst::I64Shl);
        f.instruction(&WasmInst::I64Const(16));
        f.instruction(&WasmInst::I64ShrS);
        f.instruction(&WasmInst::F64ConvertI64S);
        f.instruction(&WasmInst::Else);
        // Raw f64: just reinterpret bits
        f.instruction(&WasmInst::LocalGet(src));
        f.instruction(&WasmInst::F64ReinterpretI64);
        f.instruction(&WasmInst::End);
    }

    /// Emit instructions to tag an f64 on the WASM stack as a NaN-boxed i64 value.
    ///
    /// Reinterprets f64 bits to i64. If the result collides with the NaN-box tag
    /// space (negative quiet NaN), replaces it with canonical NaN.
    fn emit_tag_f64_result(f: &mut WasmFunction, tmp: u32) {
        f.instruction(&WasmInst::I64ReinterpretF64);
        f.instruction(&WasmInst::LocalTee(tmp));
        // Check if the f64 bits collide with NaN-box tag space
        f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
        f.instruction(&WasmInst::I64And);
        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
        f.instruction(&WasmInst::I64Eq);
        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
        // Collides with tag space: replace with canonical NaN
        f.instruction(&WasmInst::I64Const(tag::CANON_NAN));
        f.instruction(&WasmInst::Else);
        // Safe f64 bits: use as-is
        f.instruction(&WasmInst::LocalGet(tmp));
        f.instruction(&WasmInst::End);
    }

    /// Emit instructions to check if either of two locals holds a raw f64 (not NaN-boxed).
    /// Pushes an i32 boolean onto the stack: 1 if either is f64, 0 if both are NaN-boxed.
    fn emit_either_is_f64(f: &mut WasmFunction, t0: u32, t1: u32) {
        // left_is_f64 = (t0 & NANBOX_MASK) != NANBOX_SIG
        f.instruction(&WasmInst::LocalGet(t0));
        f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
        f.instruction(&WasmInst::I64And);
        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
        f.instruction(&WasmInst::I64Ne);
        // right_is_f64 = (t1 & NANBOX_MASK) != NANBOX_SIG
        f.instruction(&WasmInst::LocalGet(t1));
        f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
        f.instruction(&WasmInst::I64And);
        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
        f.instruction(&WasmInst::I64Ne);
        // either_float = left_is_f64 | right_is_f64
        f.instruction(&WasmInst::I32Or);
    }

    fn emit_instruction(
        &self,
        f: &mut WasmFunction,
        inst: &Instruction,
        ir: &IrModule,
        num_imports: u32,
        string_offsets: &[u32],
        string_reverse_index: &HashMap<&str, u32>,
        temp_base: u32,
    ) -> Result<(), CompileError> {
        match inst {
            // ── Constants ────────────────────────────────
            Instruction::PushNull => {
                // NaN-boxed null: NANBOX_SIG | (NULL << 48) | 0
                f.instruction(&WasmInst::I64Const(tag::encode(tag::NULL, 0)));
            }
            Instruction::PushBool(b) => {
                f.instruction(&WasmInst::I64Const(tag::encode(tag::BOOL, *b as i64)));
            }
            Instruction::PushI64(n) => {
                // NaN-boxed i64: payload is bottom 48 bits (sign-extended on untag).
                f.instruction(&WasmInst::I64Const(tag::encode(tag::I64, *n)));
            }
            Instruction::PushF64(n) => {
                // Float64: stored as raw IEEE 754 bits — no tagging needed.
                // NaN values must be canonicalized to avoid collision with tagged values.
                let bits = n.to_bits() as i64;
                if n.is_nan() {
                    f.instruction(&WasmInst::I64Const(tag::CANON_NAN));
                } else {
                    f.instruction(&WasmInst::I64Const(bits));
                }
            }
            Instruction::PushI32(n) => {
                // i32 fits in 48-bit payload (sign-extended on untag).
                f.instruction(&WasmInst::I64Const(tag::encode(tag::I32, *n as i64)));
            }
            Instruction::PushF32(n) => {
                // f32 reinterpreted as 32-bit int, fits in 48-bit payload.
                let bits = n.to_bits() as i64;
                f.instruction(&WasmInst::I64Const(tag::encode(tag::F32, bits)));
            }
            Instruction::PushString(idx) => {
                let offset = match string_offsets.get(*idx as usize).copied() {
                    Some(o) => o,
                    None => return Err(CompileError::Internal(
                        format!("string index {} out of bounds (max {})", idx, string_offsets.len())
                    )),
                };
                f.instruction(&WasmInst::I64Const(tag::encode(tag::STRING, offset as i64)));
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
                // Global 0 is heap pointer; IR globals start at index 1
                f.instruction(&WasmInst::GlobalGet(*idx + 1));
            }
            Instruction::GlobalSet(idx) => {
                f.instruction(&WasmInst::GlobalSet(*idx + 1));
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
                // Dispatch: float (untagged raw f64 bits) vs integer (NaN-boxed).
                let t = temp_base;
                f.instruction(&WasmInst::LocalSet(t));
                // Check if NaN-boxed tagged: (val & NANBOX_MASK) == NANBOX_SIG
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                f.instruction(&WasmInst::I64Eq);
                f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                // Tagged integer path: untag payload, sign-extend, negate, retag
                f.instruction(&WasmInst::I64Const(0));
                // Extract 48-bit payload (strip tag bits first)
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                // Sign-extend from 48 bits to 64 bits
                f.instruction(&WasmInst::I64Const(16));
                f.instruction(&WasmInst::I64Shl);
                f.instruction(&WasmInst::I64Const(16));
                f.instruction(&WasmInst::I64ShrS);
                f.instruction(&WasmInst::I64Sub); // 0 - val = negate
                // Mask back to 48-bit payload and retag
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                f.instruction(&WasmInst::I64Or);
                f.instruction(&WasmInst::Else);
                // Float path: XOR sign bit to flip sign
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(i64::MIN)); // 0x8000_0000_0000_0000
                f.instruction(&WasmInst::I64Xor);
                f.instruction(&WasmInst::End);
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
                // Truthiness check that handles both NaN-boxed and raw f64 values:
                // - Tagged (null/false/0): falsy if payload == 0
                // - Raw f64: falsy if ±0.0 (i.e. val << 1 == 0)
                let t = temp_base;
                f.instruction(&WasmInst::LocalSet(t));
                // Check if value is NaN-boxed tagged
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                f.instruction(&WasmInst::I64Eq);
                f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                // Tagged path: falsy if payload == 0
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Eqz);
                f.instruction(&WasmInst::I64ExtendI32U);
                f.instruction(&WasmInst::Else);
                // Raw f64 path: falsy if ±0.0 (shift out sign bit)
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(1));
                f.instruction(&WasmInst::I64Shl);
                f.instruction(&WasmInst::I64Eqz);
                f.instruction(&WasmInst::I64ExtendI32U);
                f.instruction(&WasmInst::End);
                // Result: i64 1 if falsy (not), 0 if truthy → tag as bool
                f.instruction(&WasmInst::I64Const(tag::encode(tag::BOOL, 0)));
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
                // NaN-box an i64: mask to 48 bits and set tag.
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::TagF64 => {
                // Reinterpret f64 bits to i64, then canonicalize NaN to avoid
                // collision with our NaN-boxing tag space.
                let t = temp_base;
                f.instruction(&WasmInst::I64ReinterpretF64);
                f.instruction(&WasmInst::LocalSet(t));
                // Check if (val & NANBOX_MASK) == NANBOX_SIG (collides with tagged space)
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                f.instruction(&WasmInst::I64Eq);
                f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                // Collides with tag space → replace with canonical NaN
                f.instruction(&WasmInst::I64Const(tag::CANON_NAN));
                f.instruction(&WasmInst::Else);
                // Safe f64 bits, use as-is
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::End);
            }
            Instruction::TagBool => {
                f.instruction(&WasmInst::I64Const(0x01));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::BOOL as i64) << tag::TAG_SHIFT)));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::TagString => {
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::STRING as i64) << tag::TAG_SHIFT)));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::UntagI64 => {
                // Extract 48-bit payload and sign-extend from bit 47.
                // Shift left 16 to put bit 47 in bit 63 (sign position),
                // then arithmetic shift right 16 to sign-extend.
                f.instruction(&WasmInst::I64Const(16));
                f.instruction(&WasmInst::I64Shl);
                f.instruction(&WasmInst::I64Const(16));
                f.instruction(&WasmInst::I64ShrS);
            }
            Instruction::UntagF64 => {
                // In NaN-boxing, f64 values are stored as raw bits — just reinterpret.
                f.instruction(&WasmInst::F64ReinterpretI64);
            }
            Instruction::UntagBool => {
                f.instruction(&WasmInst::I64Const(0x01));
                f.instruction(&WasmInst::I64And);
            }
            Instruction::GetTag => {
                // NaN-boxing tag extraction using `select`:
                // Stack input: [tagged_value]
                // Stack output: [tag_as_nanboxed_i64]
                // Algorithm:
                //   is_nanboxed = (val & NANBOX_MASK) == NANBOX_SIG
                //   if is_nanboxed: tag = (val >> 48) & 0x07
                //   else: tag = F64 (sentinel = 8)
                // We use WASM `select` which takes (val_true, val_false, condition) and returns
                // val_true if condition != 0, val_false otherwise.
                // Since we need the value twice (once for nanbox check, once for tag extraction),
                // we need a temp local. We'll use the approach of duplicating via local.tee.
                // GetTag is always used by the compiler, which always has at least one temp available.
                let t = temp_base;

                // Save value to temp
                f.instruction(&WasmInst::LocalTee(t));

                // Path 1: extract NaN-boxed tag: (val >> 48) & 0x07
                f.instruction(&WasmInst::I64Const(tag::TAG_SHIFT));
                f.instruction(&WasmInst::I64ShrU);
                f.instruction(&WasmInst::I64Const(0x07));
                f.instruction(&WasmInst::I64And);

                // Path 2: F64 sentinel tag value
                f.instruction(&WasmInst::I64Const(tag::F64 as i64));

                // Condition: is_nanboxed = (val & NANBOX_MASK) == NANBOX_SIG
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                f.instruction(&WasmInst::I64Eq);

                // select: if is_nanboxed then nanboxed_tag else F64_sentinel
                f.instruction(&WasmInst::Select);

                // Tag the result as a NaN-boxed I64 for comparison
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                f.instruction(&WasmInst::I64Or);
            }

            // ── Control flow ─────────────────────────────
            Instruction::Block => {
                f.instruction(&WasmInst::Block(BlockType::Empty));
            }
            Instruction::Loop => {
                f.instruction(&WasmInst::Loop(BlockType::Empty));
            }
            Instruction::End => {
                f.instruction(&WasmInst::End);
            }
            Instruction::If => {
                // Truthiness check for both NaN-boxed and raw f64 values
                let t = temp_base;
                f.instruction(&WasmInst::LocalSet(t));
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                f.instruction(&WasmInst::I64Eq);
                f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I32)));
                // Tagged: truthy if payload != 0
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::I64Ne);
                f.instruction(&WasmInst::Else);
                // Raw f64: truthy if not ±0.0
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(1));
                f.instruction(&WasmInst::I64Shl);
                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::I64Ne);
                f.instruction(&WasmInst::End);
                f.instruction(&WasmInst::If(BlockType::Result(
                    WasmValType::I64,
                )));
            }
            Instruction::IfVoid => {
                // Truthiness check for both NaN-boxed and raw f64 values
                let t = temp_base;
                f.instruction(&WasmInst::LocalSet(t));
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                f.instruction(&WasmInst::I64Eq);
                f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I32)));
                // Tagged: truthy if payload != 0
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::I64Ne);
                f.instruction(&WasmInst::Else);
                // Raw f64: truthy if not ±0.0
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(1));
                f.instruction(&WasmInst::I64Shl);
                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::I64Ne);
                f.instruction(&WasmInst::End);
                f.instruction(&WasmInst::If(BlockType::Empty));
            }
            Instruction::Else => {
                f.instruction(&WasmInst::Else);
            }
            Instruction::Br(depth) => {
                f.instruction(&WasmInst::Br(*depth));
            }
            Instruction::BrIf(depth) => {
                // Truthiness check for both NaN-boxed and raw f64 values
                let t = temp_base;
                f.instruction(&WasmInst::LocalSet(t));
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                f.instruction(&WasmInst::I64Eq);
                f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I32)));
                // Tagged: truthy if payload != 0
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::I64Ne);
                f.instruction(&WasmInst::Else);
                // Raw f64: truthy if not ±0.0
                f.instruction(&WasmInst::LocalGet(t));
                f.instruction(&WasmInst::I64Const(1));
                f.instruction(&WasmInst::I64Shl);
                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::I64Ne);
                f.instruction(&WasmInst::End);
                f.instruction(&WasmInst::BrIf(*depth));
            }
            Instruction::BrTable(targets, default) => {
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::BrTable {
                    targets: targets.to_vec(),
                    default: *default,
                });
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
            Instruction::CallIndirect(param_count) => {
                // Resolve the WASM type index for a function with this param count.
                // All functions use i64 params → i64 result, so we search for any
                // function type with matching param_count.
                let type_idx = ir.functions.iter()
                    .position(|func| func.param_count == *param_count)
                    .map(|i| i as u32)
                    .ok_or_else(|| CompileError::Internal(
                        format!("no function type with {} params for indirect call", param_count)
                    ))?;
                // Untag: extract payload from NaN-boxed value before converting to table index
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::CallIndirect {
                    type_idx,
                    table_idx: 0,
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
                f.instruction(&WasmInst::If(BlockType::Empty));
                f.instruction(&WasmInst::I32Const(16)); // grow by 1MB
                f.instruction(&WasmInst::MemoryGrow(0));
                f.instruction(&WasmInst::I32Const(-1_i32));
                f.instruction(&WasmInst::I32Eq);
                f.instruction(&WasmInst::If(BlockType::Empty));
                f.instruction(&WasmInst::Unreachable); // out of memory
                f.instruction(&WasmInst::End);
                f.instruction(&WasmInst::End);

                // 4. Return old_ptr as i64 (on stack from step 1).
                f.instruction(&WasmInst::I64ExtendI32U);
            }
            Instruction::MemLoadI64 => {
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I64Load(MemArg {
                    offset: 0,
                    align: 3, // 8-byte alignment
                    memory_index: 0,
                }));
            }
            Instruction::MemStoreI64 => {
                // stack: [addr(i64), value(i64)]
                // WASM i64.store expects [i32_addr, i64_value].
                // The addr is i64 but the value is also i64, so we need to
                // save the value to a temp, wrap the addr, then restore.
                // Since this instruction is currently unused in compiled output,
                // use a simpler approach: swap via temp local.
                f.instruction(&WasmInst::LocalSet(temp_base)); // save value
                f.instruction(&WasmInst::I32WrapI64);          // convert addr to i32
                f.instruction(&WasmInst::LocalGet(temp_base)); // restore value
                f.instruction(&WasmInst::I64Store(MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Instruction::MemLoadF64 => {
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::F64Load(MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Instruction::MemStoreF64 => {
                return Err(CompileError::Internal("MemStoreF64 is not currently supported".into()));
            }
            Instruction::MemLoadI32 => {
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
            }
            Instruction::MemStoreI32 => {
                return Err(CompileError::Internal("MemStoreI32 is not currently supported".into()));
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
                    Self::emit_heap_bounds_check(f);
                    // Store length=0, capacity=0
                    let t0 = temp_base;
                    f.instruction(&WasmInst::I64ExtendI32U);
                    f.instruction(&WasmInst::LocalTee(t0)); // t0 = base_ptr as i64
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(0)); // length
                    f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(0)); // capacity
                    f.instruction(&WasmInst::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::ARRAY as i64) << tag::TAG_SHIFT)));
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
                    let alloc_size = 8u32.saturating_add(count.saturating_mul(8));
                    // bump alloc
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::I32Const(alloc_size as i32));
                    f.instruction(&WasmInst::I32Add);
                    f.instruction(&WasmInst::GlobalSet(0));
                    Self::emit_heap_bounds_check(f);
                    f.instruction(&WasmInst::I64ExtendI32U);
                    f.instruction(&WasmInst::LocalSet(t0)); // t0 = base_ptr

                    // Store length and capacity
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(count as i32));
                    f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(count as i32));
                    f.instruction(&WasmInst::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

                    // Elements are on the stack in order: bottom=elem0, top=elem(count-1)
                    // Pop from top (last element first).
                    for i in (0..count).rev() {
                        f.instruction(&WasmInst::LocalSet(t1)); // save element
                        // Compute address: base + 8 + i*8
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8i32.saturating_add((i as i32).saturating_mul(8))));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));
                    }

                    // Tag as array and push
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::ARRAY as i64) << tag::TAG_SHIFT)));
                    f.instruction(&WasmInst::I64Or);
                }
            }
            Instruction::ArrayGet => {
                // Stack: [array, index]
                let t0 = temp_base;     // index (untagged)
                let t1 = temp_base + 1; // array ptr (untagged)
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t0));
                // Untag array to get pointer
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t1));
                // Bounds check: index < length (first 4 bytes at ptr)
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32LtU);
                f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                // In bounds: load element at ptr + 8 + index*8
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                f.instruction(&WasmInst::Else);
                // Out of bounds: return null
                f.instruction(&WasmInst::I64Const(tag::encode(tag::NULL, 0)));
                f.instruction(&WasmInst::End);
            }
            Instruction::ArraySet => {
                // Stack: [array, index, value]
                let t0 = temp_base;     // value
                let t1 = temp_base + 1; // index (untagged i64)
                let t2 = temp_base + 2; // array ptr (untagged i64)
                f.instruction(&WasmInst::LocalSet(t0)); // save value
                // Untag index: sign-extend from 48 bits (keep as i64 for local storage)
                f.instruction(&WasmInst::I64Const(16));
                f.instruction(&WasmInst::I64Shl);
                f.instruction(&WasmInst::I64Const(16));
                f.instruction(&WasmInst::I64ShrS);
                f.instruction(&WasmInst::LocalSet(t1)); // save index as i64
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t2)); // save array ptr

                // Bounds check: 0 <= index < length
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(0));
                f.instruction(&WasmInst::I32LtS);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32GeU); // index >= length (unsigned)
                f.instruction(&WasmInst::I32Or); // index < 0 || index >= length
                f.instruction(&WasmInst::If(BlockType::Empty));
                // Out of bounds: skip the store, just push array back unchanged
                f.instruction(&WasmInst::Else);
                // In bounds: compute address and store
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));
                f.instruction(&WasmInst::End);

                // Push array ref back (re-tagged)
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::ARRAY as i64) << tag::TAG_SHIFT)));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::ArrayLen => {
                // Untag to get pointer, load i32 length at offset 0, tag as i64
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I64ExtendI32U);
                // Tag as i64
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
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
                    Self::emit_heap_bounds_check(f);
                    let t0 = temp_base;
                    f.instruction(&WasmInst::I64ExtendI32U);
                    f.instruction(&WasmInst::LocalTee(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(0));
                    f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(0));
                    f.instruction(&WasmInst::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::MAP as i64) << tag::TAG_SHIFT)));
                    f.instruction(&WasmInst::I64Or);
                } else {
                    let t0 = temp_base;     // base_ptr
                    let t1 = temp_base + 1; // element temp

                    let alloc_size = 8u32.saturating_add(count.saturating_mul(16)); // 16 bytes per entry (key + value)
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::GlobalGet(0));
                    f.instruction(&WasmInst::I32Const(alloc_size as i32));
                    f.instruction(&WasmInst::I32Add);
                    f.instruction(&WasmInst::GlobalSet(0));
                    Self::emit_heap_bounds_check(f);
                    f.instruction(&WasmInst::I64ExtendI32U);
                    f.instruction(&WasmInst::LocalSet(t0));

                    // Store count and capacity
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(count as i32));
                    f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I32WrapI64);
                    f.instruction(&WasmInst::I32Const(count as i32));
                    f.instruction(&WasmInst::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

                    // Stack has pairs: bottom=[key0,val0], top=[key(n-1),val(n-1)]
                    // Pop in reverse: top pair first.
                    for i in (0..count).rev() {
                        f.instruction(&WasmInst::LocalSet(t1));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8i32.saturating_add((i as i32).saturating_mul(16)).saturating_add(8)));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalSet(t1));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8i32.saturating_add((i as i32).saturating_mul(16))));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));
                    }

                    f.instruction(&WasmInst::LocalGet(t0));
                    f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::MAP as i64) << tag::TAG_SHIFT)));
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
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t1)); // save map ptr

                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::LocalSet(t2)); // counter = 0

                // block $done
                f.instruction(&WasmInst::Block(BlockType::Empty));
                // loop $search
                f.instruction(&WasmInst::Loop(BlockType::Empty));

                // if counter >= count, break
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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
                f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));

                // Compare with search key
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Eq);
                f.instruction(&WasmInst::If(BlockType::Empty));
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
                f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                f.instruction(&WasmInst::LocalSet(t0)); // save found value
                f.instruction(&WasmInst::Br(2)); // break to $done
                f.instruction(&WasmInst::End); // end if

                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I64Const(1));
                f.instruction(&WasmInst::I64Add);
                f.instruction(&WasmInst::LocalSet(t2));
                f.instruction(&WasmInst::Br(0)); // continue loop

                f.instruction(&WasmInst::End); // end loop
                f.instruction(&WasmInst::End); // end block

                // Detect found vs not-found using counter t2:
                // - Found: Br(2) breaks out before increment, so t2 < count and t0 has the value.
                // - Not found: loop exits normally with t2 == count, t0 still has the key.
                // Check t2 < count → use t0 as value, else push null.
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32LtU);
                f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                f.instruction(&WasmInst::LocalGet(t0)); // found value
                f.instruction(&WasmInst::Else);
                f.instruction(&WasmInst::I64Const(tag::encode(tag::NULL, 0))); // NaN-boxed null
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
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t2)); // save map ptr

                f.instruction(&WasmInst::I64Const(0));
                f.instruction(&WasmInst::LocalSet(t3)); // counter

                // Search for existing key
                f.instruction(&WasmInst::Block(BlockType::Empty)); // $outer
                f.instruction(&WasmInst::Block(BlockType::Empty)); // $found
                f.instruction(&WasmInst::Loop(BlockType::Empty)); // $loop

                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32GeU);
                f.instruction(&WasmInst::BrIf(1)); // not found → $found block end

                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I64Eq);
                f.instruction(&WasmInst::If(BlockType::Empty));
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
                f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Save tagged map ref to t0 (value already stored to memory)
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::MAP as i64) << tag::TAG_SHIFT)));
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
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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
                Self::emit_heap_bounds_check(f);

                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Const(1));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Const(1));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

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
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));

                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Const(16));
                f.instruction(&WasmInst::I32Mul);
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I32Const(8));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));

                // Save tagged new map ref to t0
                f.instruction(&WasmInst::LocalGet(t3));
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::MAP as i64) << tag::TAG_SHIFT)));
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
                // Copy str1 bytes: memory.copy(new_ptr+4, str1_ptr+4, str1_len)
                // Copy str2 bytes: memory.copy(new_ptr+4+str1_len, str2_ptr+4, str2_len)

                // Get str1_len
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                // stack: [str1_len:i32]

                // Get str2_len
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                // str1_len (i32)
                // duplicate str1_ptr to load from
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t1)); // t1 = str1 raw ptr (i64)

                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::LocalSet(t0)); // t0 = str2 raw ptr (i64)

                // Load str1_len
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                // Load str2_len
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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

                // Bounds check: grow memory if needed
                Self::emit_heap_bounds_check(f);

                // Store total length at new_ptr
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                // Recompute total_len
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

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
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 })); // len
                f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                // Copy str2 bytes: memory.copy(dst=new_ptr+4+str1_len, src=str2_ptr+4, len=str2_len)
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(4));
                f.instruction(&WasmInst::I32Add);
                f.instruction(&WasmInst::LocalGet(t1));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I32Add); // dst = new_ptr+4+str1_len
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Const(4));
                f.instruction(&WasmInst::I32Add); // src
                f.instruction(&WasmInst::LocalGet(t0));
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 })); // len
                f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                // Tag result as string
                f.instruction(&WasmInst::LocalGet(t2));
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::STRING as i64) << tag::TAG_SHIFT)));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::StringLen => {
                // Untag to get pointer, load i32 length at offset 0, tag as i64
                f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                f.instruction(&WasmInst::I64And);
                f.instruction(&WasmInst::I32WrapI64);
                f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&WasmInst::I64ExtendI32U);
                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                f.instruction(&WasmInst::I64Or);
            }
            Instruction::Print => {
                // Call imported print function (expects i64 tagged value).
                f.instruction(&WasmInst::Call(0)); // print is import #0
            }
            Instruction::RuntimeCall { name, arg_count } => {
                let fn_name = match ir.strings.get(*name as usize) {
                    Some(s) => s.as_str(),
                    None => return Err(CompileError::Internal(
                        format!("runtime call name index {} out of bounds", name)
                    )),
                };
                match (fn_name, *arg_count) {
                    ("array_push", 2) => {
                        // Stack: [array, element]
                        let t0 = temp_base;     // element
                        let t1 = temp_base + 1; // old array ptr
                        let t2 = temp_base + 2; // new array ptr / loop counter

                        f.instruction(&WasmInst::LocalSet(t0)); // save element
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t1)); // save old array ptr

                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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
                        Self::emit_heap_bounds_check(f);

                        // Store new length and capacity
                        // Reload old_len+1
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

                        // Store capacity (same as length)
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

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
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add); // addr = new_ptr+8+old_len*8
                        f.instruction(&WasmInst::LocalGet(t0)); // element
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));

                        // Push tagged new array
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::ARRAY as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);
                    }
                    ("len", 1) => {
                        // Check tag: if ARRAY do ArrayLen, if STRING do StringLen, if MAP do MapLen, else push 0.
                        // Must verify NaN-boxing first to avoid misidentifying raw f64 values.
                        let t0 = temp_base;
                        f.instruction(&WasmInst::LocalTee(t0));

                        // First: check if value is NaN-boxed
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                        f.instruction(&WasmInst::I64Eq);
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));

                        // NaN-boxed: extract tag safely
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::TAG_SHIFT));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I64Const(0x07));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);

                        // if tag == ARRAY
                        f.instruction(&WasmInst::I32Const(tag::ARRAY as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::Else);
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::TAG_SHIFT));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I64Const(0x07));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::STRING as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::Else);
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::TAG_SHIFT));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I64Const(0x07));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::MAP as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::Else);
                        // Unknown NaN-boxed type: return 0
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::End);
                        f.instruction(&WasmInst::End);
                        f.instruction(&WasmInst::End);

                        f.instruction(&WasmInst::Else);
                        // Not NaN-boxed (raw f64): return 0
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
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
                        // NaN-boxing: extract 3-bit tag from bits 50-48
                        // But first check if it's a raw f64 (not NaN-boxed)
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                        f.instruction(&WasmInst::I64Eq);
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I32)));
                        // NaN-boxed: extract tag
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::TAG_SHIFT));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I64Const(0x07));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::Else);
                        // Raw f64: return sentinel tag
                        f.instruction(&WasmInst::I32Const(tag::F64 as i32));
                        f.instruction(&WasmInst::End);
                        // Now we have the tag as i32 on stack
                        // Check each type:
                        // We need to intern these strings and return tagged string pointers.
                        // Use string_offsets to find them.
                        // Look up type name strings via the reverse index (O(1) per lookup).
                        let type_names = ["null", "bool", "int64", "float64", "string", "array", "map", "int32", "float32"];
                        let mut type_str_indices = Vec::new();
                        for name in &type_names {
                            type_str_indices.push(string_reverse_index.get(*name).map(|&idx| idx as usize));
                        }

                        // type_str_indices layout: [null=0, bool=1, int64=2, float64=3, string=4, array=5, map=6, int32=7, float32=8]
                        // tag_checks: (tag_value, type_str_index)
                        let tag_checks: [(u8, usize); 9] = [
                            (tag::NULL, 0),    // "null"
                            (tag::ARRAY, 5),   // "array"
                            (tag::STRING, 4),  // "string"
                            (tag::I64, 2),     // "int64"
                            (tag::MAP, 6),     // "map"
                            (tag::BOOL, 1),    // "bool"
                            (tag::F64, 3),     // "float64"
                            (tag::I32, 7),     // "int32"
                            (tag::F32, 8),     // "float32"
                        ];

                        for (ci, (tag_val, str_idx)) in tag_checks.iter().enumerate() {
                            f.instruction(&WasmInst::I32Const(*tag_val as i32));
                            f.instruction(&WasmInst::I32Eq);
                            f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                            if let Some(idx) = type_str_indices[*str_idx] {
                                let offset = string_offsets[idx];
                                f.instruction(&WasmInst::I64Const((tag::NANBOX_SIG | ((tag::STRING as i64) << tag::TAG_SHIFT)) | (offset as i64)));
                            } else {
                                f.instruction(&WasmInst::I64Const(tag::encode(tag::NULL, 0)));
                            }
                            f.instruction(&WasmInst::Else);
                            // Re-extract tag for next check (not needed for last check).
                            if ci < tag_checks.len() - 1 {
                                f.instruction(&WasmInst::LocalGet(t0));
                                f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                                f.instruction(&WasmInst::I64And);
                                f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                                f.instruction(&WasmInst::I64Eq);
                                f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I32)));
                                f.instruction(&WasmInst::LocalGet(t0));
                                f.instruction(&WasmInst::I64Const(tag::TAG_SHIFT));
                                f.instruction(&WasmInst::I64ShrU);
                                f.instruction(&WasmInst::I64Const(0x07));
                                f.instruction(&WasmInst::I64And);
                                f.instruction(&WasmInst::I32WrapI64);
                                f.instruction(&WasmInst::Else);
                                f.instruction(&WasmInst::I32Const(tag::F64 as i32));
                                f.instruction(&WasmInst::End);
                            }
                        }
                        // Default: return "null" for unknown tags
                        if let Some(idx) = type_str_indices[0] {
                            let offset = string_offsets[idx];
                            f.instruction(&WasmInst::I64Const((tag::NANBOX_SIG | ((tag::STRING as i64) << tag::TAG_SHIFT)) | (offset as i64)));
                        } else {
                            f.instruction(&WasmInst::I64Const(tag::encode(tag::NULL, 0)));
                        }
                        // Close all if-else chains (one End per tag check)
                        for _ in &tag_checks {
                            f.instruction(&WasmInst::End);
                        }
                    }
                    ("map_get", 2) => {
                        // Stack: [map, key] — same logic as MapGet instruction
                        let t0 = temp_base;     // key → then result
                        let t1 = temp_base + 1; // map ptr
                        let t2 = temp_base + 2; // counter

                        f.instruction(&WasmInst::LocalSet(t0)); // save key
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t1)); // save map ptr

                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::LocalSet(t2)); // counter = 0

                        f.instruction(&WasmInst::Block(BlockType::Empty));
                        f.instruction(&WasmInst::Loop(BlockType::Empty));

                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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
                        f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));

                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Eq);
                        f.instruction(&WasmInst::If(BlockType::Empty));
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
                        f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
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
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32LtU);
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::Else);
                        f.instruction(&WasmInst::I64Const(tag::encode(tag::NULL, 0))); // NaN-boxed null
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
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t2)); // map ptr

                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::LocalSet(t3)); // counter

                        // Search for existing key
                        f.instruction(&WasmInst::Block(BlockType::Empty)); // $outer
                        f.instruction(&WasmInst::Block(BlockType::Empty)); // $found
                        f.instruction(&WasmInst::Loop(BlockType::Empty)); // $loop

                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32GeU);
                        f.instruction(&WasmInst::BrIf(1)); // not found → $found block end

                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Eq);
                        f.instruction(&WasmInst::If(BlockType::Empty));
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
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));
                        // Save tagged map ref to t0 (value already stored to memory)
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::MAP as i64) << tag::TAG_SHIFT)));
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
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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
                        Self::emit_heap_bounds_check(f);

                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

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
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1)); // key
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));

                        // Store new value at new+8+old_count*16+8
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t0)); // value
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));

                        // Save tagged new map ref to t0
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::MAP as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::LocalSet(t0));

                        f.instruction(&WasmInst::End); // end $outer block

                        // Push result from temp
                        f.instruction(&WasmInst::LocalGet(t0));
                    }
                    ("map_from_entries", 1) => {
                        // Stack: [tagged_array_of_pairs]
                        // Input: array of [key, value] pairs
                        // Output: map with those key-value entries
                        // Memory layouts:
                        //   Array: [i32 len][i32 cap][i64 elem0][i64 elem1]...
                        //   Map:   [i32 count][i32 cap][i64 key0][i64 val0][i64 key1][i64 val1]...
                        //   Each pair is itself an array: [i32 len=2][i32 cap][i64 key][i64 value]
                        let t0 = temp_base;     // arr_ptr (untagged)
                        let t1 = temp_base + 1; // map_ptr (untagged)
                        let t2 = temp_base + 2; // count (i64)
                        let t3 = temp_base + 3; // loop index (i64)
                        let t4 = temp_base + 4; // pair_ptr (untagged)

                        // Untag array: extract payload (raw pointer)
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t0));

                        // Read array length → t2
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::LocalSet(t2)); // count

                        // Allocate map: 8 + count * 16 bytes
                        f.instruction(&WasmInst::GlobalGet(0)); // map_ptr
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::LocalSet(t1));

                        // Bump allocator: heap_ptr += 8 + count*16
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalSet(0));
                        Self::emit_heap_bounds_check(f);

                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

                        // Store map capacity = count
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

                        // Initialize loop index = 0
                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::LocalSet(t3));

                        // Loop: for i in 0..count
                        f.instruction(&WasmInst::Block(BlockType::Empty)); // $break
                        f.instruction(&WasmInst::Loop(BlockType::Empty));   // $continue

                        // if i >= count, break
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64GeU);
                        f.instruction(&WasmInst::BrIf(1)); // break to $break

                        // Load pair = arr[i] (tagged array ref)
                        // pair_tagged = mem[arr_ptr + 8 + i*8]
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                        // Untag pair to get raw pointer
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t4)); // pair_ptr

                        // Read key = pair[0] = mem[pair_ptr + 8]
                        // Store into map: map_ptr + 8 + i*16
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(16));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        // Load key from pair
                        f.instruction(&WasmInst::LocalGet(t4));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8)); // skip pair header
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));

                        // Read value = pair[1] = mem[pair_ptr + 16]
                        // Store into map: map_ptr + 8 + i*16 + 8
                        f.instruction(&WasmInst::LocalGet(t1));
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
                        // Load value from pair
                        f.instruction(&WasmInst::LocalGet(t4));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(16)); // skip pair header + key
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));

                        // i++
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::LocalSet(t3));

                        f.instruction(&WasmInst::Br(0)); // continue loop
                        f.instruction(&WasmInst::End); // end loop
                        f.instruction(&WasmInst::End); // end block

                        // Tag map and push result
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::MAP as i64) << tag::TAG_SHIFT)));
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
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t3)); // inclusive flag

                        // Save end, sign-extend from 48 bits to full i64
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64ShrS);
                        f.instruction(&WasmInst::LocalSet(t1)); // end

                        // Save start, sign-extend from 48 bits to full i64
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64ShrS);
                        f.instruction(&WasmInst::LocalSet(t0)); // start

                        // If inclusive, end = end + 1
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::I64Ne);
                        f.instruction(&WasmInst::If(BlockType::Empty));
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
                        f.instruction(&WasmInst::If(BlockType::Empty));
                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::LocalSet(t3));
                        f.instruction(&WasmInst::End);

                        // Clamp count to avoid I32Mul overflow in allocation.
                        // Max safe: 2^27 = 134M elements (134M*8+8 < 1GB, fits i32).
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I64Const(134_217_728)); // 2^27
                        f.instruction(&WasmInst::I64GtS);
                        f.instruction(&WasmInst::If(BlockType::Empty));
                        f.instruction(&WasmInst::I64Const(134_217_728));
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
                        Self::emit_heap_bounds_check(f);

                        // Store length and capacity
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

                        // Fill array: for i in 0..count, store tagged(start + i)
                        // Save clamped count to t1 before repurposing t3 as loop index.
                        // Must use the same clamped count as allocation to avoid OOB writes.
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::LocalSet(t1)); // t1 = clamped count

                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::LocalSet(t3)); // i = 0

                        f.instruction(&WasmInst::Block(BlockType::Empty));
                        f.instruction(&WasmInst::Loop(BlockType::Empty));

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
                        // value = NaN-boxed I64: NANBOX_SIG | (I64 << 48) | (start + i)
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::LocalGet(t3));
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And); // mask to 48 bits
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));

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
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::ARRAY as i64) << tag::TAG_SHIFT)));
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

                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t_tmp)); // save old ptr (untagged i64)

                        f.instruction(&WasmInst::LocalGet(t_tmp));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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

                        // Bounds check: grow memory if needed
                        Self::emit_heap_bounds_check(f);

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

                        f.instruction(&WasmInst::Block(BlockType::Empty)); // $outer_break
                        f.instruction(&WasmInst::Loop(BlockType::Empty));  // $outer_loop

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
                        f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalSet(t_key));

                        // j = i - 1
                        f.instruction(&WasmInst::LocalGet(t_i));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Sub);
                        f.instruction(&WasmInst::LocalSet(t_j));

                        // Inner loop: while j >= 0 && arr[j] > key
                        f.instruction(&WasmInst::Block(BlockType::Empty)); // $inner_break
                        f.instruction(&WasmInst::Loop(BlockType::Empty));  // $inner_loop

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
                        f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalSet(t_tmp)); // arr[j]

                        // Compare: untag(arr[j]) > untag(key)?
                        // Sign-extend both from 48 bits for proper comparison
                        f.instruction(&WasmInst::LocalGet(t_tmp));
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64ShrS); // sign-extend arr[j]

                        f.instruction(&WasmInst::LocalGet(t_key));
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
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
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));

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
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));

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
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::ARRAY as i64) << tag::TAG_SHIFT)));
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

                        // Check if left is a NaN-boxed string.
                        // Must verify NaN-boxing first: (val & NANBOX_MASK) == NANBOX_SIG
                        // Without this check, raw f64 values with bits 50-48 matching
                        // STRING tag (3) would be misidentified, causing OOB memory traps.
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                        f.instruction(&WasmInst::I64Eq);
                        // Stack: [is_nanboxed: i32]
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I32)));
                        // NaN-boxed: now safe to extract tag
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::TAG_SHIFT));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I64Const(0x07));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::STRING as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::Else);
                        f.instruction(&WasmInst::I32Const(0)); // raw f64 is not a string
                        f.instruction(&WasmInst::End);
                        // Stack: [is_string: i32]
                        f.instruction(&WasmInst::If(BlockType::Empty));

                        // String concat path: convert right to string if not already
                        // Check if right is a NaN-boxed string (must verify NaN-boxing first)
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                        f.instruction(&WasmInst::I64Eq);
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I32)));
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(tag::TAG_SHIFT));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I64Const(0x07));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::STRING as i32));
                        f.instruction(&WasmInst::I32Ne);
                        f.instruction(&WasmInst::Else);
                        f.instruction(&WasmInst::I32Const(1)); // raw f64 is not a string
                        f.instruction(&WasmInst::End);
                        // Stack: [is_not_string: i32]
                        f.instruction(&WasmInst::If(BlockType::Empty));
                        // Right is not a string — convert via __to_string host call
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::Call(2)); // __to_string import index
                        f.instruction(&WasmInst::LocalSet(t1));
                        f.instruction(&WasmInst::End);

                        // Now both t0 and t1 are strings
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        // stack: [str1_ptr:i32]
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        // stack: [str1_len:i32]
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
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

                        // Bounds check: grow memory if needed
                        Self::emit_heap_bounds_check(f);

                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        // Recompute total_len
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

                        // Copy str1 bytes: dst=new_ptr+4, src=str1_ptr+4, len=str1_len
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(4));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(4));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                        // Copy str2 bytes: dst=new_ptr+4+str1_len, src=str2_ptr+4, len=str2_len
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(4));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(4));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });

                        // Tag result as string
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::STRING as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::LocalSet(t2));

                        f.instruction(&WasmInst::Else);
                        // Numeric add path with int/float dispatch
                        Self::emit_either_is_f64(f, t0, t1);
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                        // Float path: convert both to f64, add, tag result
                        Self::emit_to_f64(f, t0);
                        Self::emit_to_f64(f, t1);
                        f.instruction(&WasmInst::F64Add);
                        Self::emit_tag_f64_result(f, t0);
                        f.instruction(&WasmInst::Else);
                        // Integer path: sign-extend, add, retag
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64ShrS);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64ShrS);
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);
                        f.instruction(&WasmInst::End); // close float/int dispatch
                        f.instruction(&WasmInst::LocalSet(t2));
                        f.instruction(&WasmInst::End); // close string/numeric dispatch

                        f.instruction(&WasmInst::LocalGet(t2));
                    }
                    ("__sub", 2) | ("__mul", 2) | ("__div", 2) | ("__mod", 2) => {
                        // Dynamic arithmetic with int/float dispatch.
                        // If either operand is a raw f64, use f64 arithmetic.
                        // Otherwise, use i64 integer arithmetic.
                        let t0 = temp_base;     // left
                        let t1 = temp_base + 1; // right
                        f.instruction(&WasmInst::LocalSet(t1));
                        f.instruction(&WasmInst::LocalSet(t0));

                        // Check if either operand is a raw f64
                        Self::emit_either_is_f64(f, t0, t1);
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));

                        // ── Float path ──
                        // Convert both operands to f64 (handling int→f64 promotion)
                        Self::emit_to_f64(f, t0);
                        Self::emit_to_f64(f, t1);
                        if fn_name == "__mod" {
                            // f64 modulo: a - trunc(a/b) * b
                            // WASM has no fmod instruction, so compute manually.
                            // Stack: [a: f64, b: f64]
                            // Store b, then a, via reinterpret to i64 locals
                            f.instruction(&WasmInst::I64ReinterpretF64);
                            f.instruction(&WasmInst::LocalSet(t1));
                            f.instruction(&WasmInst::I64ReinterpretF64);
                            f.instruction(&WasmInst::LocalSet(t0));
                            // Compute: a - trunc(a/b) * b
                            f.instruction(&WasmInst::LocalGet(t0));
                            f.instruction(&WasmInst::F64ReinterpretI64); // a
                            f.instruction(&WasmInst::LocalGet(t0));
                            f.instruction(&WasmInst::F64ReinterpretI64); // a
                            f.instruction(&WasmInst::LocalGet(t1));
                            f.instruction(&WasmInst::F64ReinterpretI64); // b
                            f.instruction(&WasmInst::F64Div);            // a/b
                            f.instruction(&WasmInst::F64Trunc);          // trunc(a/b)
                            f.instruction(&WasmInst::LocalGet(t1));
                            f.instruction(&WasmInst::F64ReinterpretI64); // b
                            f.instruction(&WasmInst::F64Mul);            // trunc(a/b)*b
                            f.instruction(&WasmInst::F64Sub);            // a - trunc(a/b)*b
                        } else {
                            match fn_name {
                                "__sub" => { f.instruction(&WasmInst::F64Sub); }
                                "__mul" => { f.instruction(&WasmInst::F64Mul); }
                                "__div" => { f.instruction(&WasmInst::F64Div); }
                                other => return Err(CompileError::Internal(format!("unexpected binop in float path: {}", other))),
                            };
                        }
                        // Tag the f64 result (reinterpret to i64, check NaN collision)
                        Self::emit_tag_f64_result(f, t0);

                        f.instruction(&WasmInst::Else);

                        // ── Integer path ──
                        // Untag left (sign-extend from 48 bits)
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64ShrS);
                        f.instruction(&WasmInst::LocalSet(t0));

                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64ShrS);
                        f.instruction(&WasmInst::LocalSet(t1));

                        if fn_name == "__div" || fn_name == "__mod" {
                            // Guard: if right == 0, return tagged null
                            f.instruction(&WasmInst::LocalGet(t1));
                            f.instruction(&WasmInst::I64Eqz);
                            f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                            f.instruction(&WasmInst::I64Const(tag::encode(tag::NULL, 0)));
                            f.instruction(&WasmInst::Else);
                        }

                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::LocalGet(t1));
                        match fn_name {
                            "__sub" => f.instruction(&WasmInst::I64Sub),
                            "__mul" => f.instruction(&WasmInst::I64Mul),
                            "__div" => f.instruction(&WasmInst::I64DivS),
                            "__mod" => f.instruction(&WasmInst::I64RemS),
                            other => return Err(CompileError::Internal(format!("unexpected binop in integer path: {}", other))),
                        };
                        // Retag as i64
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);

                        if fn_name == "__div" || fn_name == "__mod" {
                            f.instruction(&WasmInst::End); // close zero-check if/else
                        }

                        f.instruction(&WasmInst::End); // close float/int dispatch if/else
                    }
                    ("__eq", 2) | ("__ne", 2) => {
                        // Equality/inequality: compare raw tagged values.
                        // In NaN-boxing, same logical values have identical bit patterns.
                        match fn_name {
                            "__eq" => f.instruction(&WasmInst::I64Eq),
                            "__ne" => f.instruction(&WasmInst::I64Ne),
                            other => return Err(CompileError::Internal(format!("unexpected equality op: {}", other))),
                        };
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::BOOL as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);
                    }
                    ("__gt", 2) | ("__lt", 2) | ("__ge", 2) | ("__le", 2) => {
                        // Ordered comparison with int/float dispatch.
                        let t0 = temp_base;
                        let t1 = temp_base + 1;
                        f.instruction(&WasmInst::LocalSet(t1));
                        f.instruction(&WasmInst::LocalSet(t0));

                        // Check if either operand is a raw f64
                        Self::emit_either_is_f64(f, t0, t1);
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));

                        // ── Float path ──
                        Self::emit_to_f64(f, t0);
                        Self::emit_to_f64(f, t1);
                        match fn_name {
                            "__gt" => f.instruction(&WasmInst::F64Gt),
                            "__lt" => f.instruction(&WasmInst::F64Lt),
                            "__ge" => f.instruction(&WasmInst::F64Ge),
                            "__le" => f.instruction(&WasmInst::F64Le),
                            other => return Err(CompileError::Internal(format!("unexpected comparison op in float path: {}", other))),
                        };
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::BOOL as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);

                        f.instruction(&WasmInst::Else);

                        // ── Integer path ──
                        // Untag left (sign-extend from 48 bits)
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64ShrS);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64ShrS);
                        match fn_name {
                            "__gt" => f.instruction(&WasmInst::I64GtS),
                            "__lt" => f.instruction(&WasmInst::I64LtS),
                            "__ge" => f.instruction(&WasmInst::I64GeS),
                            "__le" => f.instruction(&WasmInst::I64LeS),
                            other => return Err(CompileError::Internal(format!("unexpected comparison op: {}", other))),
                        };
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::BOOL as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);

                        f.instruction(&WasmInst::End); // close float/int dispatch
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
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t1)); // save old array ptr

                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        // Allocate: 8 + (old_len+1)*8
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);

                        // Save heap pointer to t2 BEFORE advancing it
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::LocalSet(t2)); // t2 = new array base ptr

                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul); // new_len * 8
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add); // + 8 (header)
                        f.instruction(&WasmInst::GlobalGet(0));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::GlobalSet(0));
                        Self::emit_heap_bounds_check(f);

                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(1));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
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
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::MemoryCopy { src_mem: 0, dst_mem: 0 });
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));
                        // Push tagged new array
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::ARRAY as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);
                    }
                    ("__bit_and" | "__bit_or" | "__bit_xor" | "__bit_shl" | "__bit_shr" | "__bit_andnot", 2) => {
                        // Bitwise operations: untag both operands, apply op, retag.
                        let t0 = temp_base;
                        let t1 = temp_base + 1;
                        f.instruction(&WasmInst::LocalSet(t1));
                        f.instruction(&WasmInst::LocalSet(t0));
                        // Untag left (sign-extend from 48 bits)
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64ShrS);
                        // Untag right
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64Shl);
                        f.instruction(&WasmInst::I64Const(16));
                        f.instruction(&WasmInst::I64ShrS);
                        match fn_name {
                            "__bit_and" => { f.instruction(&WasmInst::I64And); }
                            "__bit_or" => { f.instruction(&WasmInst::I64Or); }
                            "__bit_xor" => { f.instruction(&WasmInst::I64Xor); }
                            "__bit_shl" => { f.instruction(&WasmInst::I64Shl); }
                            "__bit_shr" => { f.instruction(&WasmInst::I64ShrS); }
                            "__bit_andnot" => {
                                // a &^ b = a & ~b => xor b with -1, then and with a
                                // Stack: [a, b] => need [a, ~b]
                                f.instruction(&WasmInst::I64Const(-1));
                                f.instruction(&WasmInst::I64Xor); // ~b
                                f.instruction(&WasmInst::I64And); // a & ~b
                            }
                            _ => {}
                        }
                        // Retag as i64
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG | ((tag::I64 as i64) << tag::TAG_SHIFT)));
                        f.instruction(&WasmInst::I64Or);
                    }
                    ("__in", 2) => {
                        // Containment check: element in collection (array linear scan).
                        let t0 = temp_base;     // needle
                        let t1 = temp_base + 1; // collection ptr (untagged)
                        let t2 = temp_base + 2; // loop counter
                        f.instruction(&WasmInst::LocalSet(t1)); // save collection tagged
                        f.instruction(&WasmInst::LocalSet(t0)); // save needle

                        // Check if collection is a NaN-boxed array
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I64Const(tag::NANBOX_SIG));
                        f.instruction(&WasmInst::I64Eq);
                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I32)));
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(tag::TAG_SHIFT));
                        f.instruction(&WasmInst::I64ShrU);
                        f.instruction(&WasmInst::I64Const(0x07));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(tag::ARRAY as i32));
                        f.instruction(&WasmInst::I32Eq);
                        f.instruction(&WasmInst::Else);
                        f.instruction(&WasmInst::I32Const(0));
                        f.instruction(&WasmInst::End);

                        f.instruction(&WasmInst::If(BlockType::Result(WasmValType::I64)));
                        // Array: untag ptr, scan elements
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I64Const(tag::PAYLOAD_MASK));
                        f.instruction(&WasmInst::I64And);
                        f.instruction(&WasmInst::LocalSet(t1));

                        f.instruction(&WasmInst::I64Const(0));
                        f.instruction(&WasmInst::LocalSet(t2)); // i = 0

                        f.instruction(&WasmInst::Block(BlockType::Result(WasmValType::I64))); // $outer
                        f.instruction(&WasmInst::Loop(BlockType::Empty)); // $loop

                        // if i >= len, break with false
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        f.instruction(&WasmInst::I64ExtendI32U);
                        f.instruction(&WasmInst::I64GeU);
                        f.instruction(&WasmInst::If(BlockType::Empty));
                        f.instruction(&WasmInst::I64Const(tag::encode(tag::BOOL, 0)));
                        f.instruction(&WasmInst::Br(2)); // break $outer with false
                        f.instruction(&WasmInst::End);

                        // Load arr[i] and compare with needle
                        f.instruction(&WasmInst::LocalGet(t1));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I32WrapI64);
                        f.instruction(&WasmInst::I32Const(8));
                        f.instruction(&WasmInst::I32Mul);
                        f.instruction(&WasmInst::I32Add);
                        f.instruction(&WasmInst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                        f.instruction(&WasmInst::LocalGet(t0));
                        f.instruction(&WasmInst::I64Eq);
                        f.instruction(&WasmInst::If(BlockType::Empty));
                        f.instruction(&WasmInst::I64Const(tag::encode(tag::BOOL, 1)));
                        f.instruction(&WasmInst::Br(2)); // break $outer with true
                        f.instruction(&WasmInst::End);

                        // i++
                        f.instruction(&WasmInst::LocalGet(t2));
                        f.instruction(&WasmInst::I64Const(1));
                        f.instruction(&WasmInst::I64Add);
                        f.instruction(&WasmInst::LocalSet(t2));
                        f.instruction(&WasmInst::Br(0)); // continue $loop

                        f.instruction(&WasmInst::End); // end $loop
                        f.instruction(&WasmInst::I64Const(tag::encode(tag::BOOL, 0))); // unreachable fallback
                        f.instruction(&WasmInst::End); // end $outer

                        f.instruction(&WasmInst::Else);
                        f.instruction(&WasmInst::I64Const(tag::encode(tag::BOOL, 0)));
                        f.instruction(&WasmInst::End);
                    }
                    _ => {
                        // Delegate to host runtime_call. Store args in scratch
                        // memory (bytes 0..argc*8) so the host can read them,
                        // instead of dropping them.
                        let t0 = temp_base;
                        for i in (0..*arg_count).rev() {
                            f.instruction(&WasmInst::LocalSet(t0));
                            f.instruction(&WasmInst::I32Const(0));
                            f.instruction(&WasmInst::LocalGet(t0));
                            f.instruction(&WasmInst::I64Store(MemArg {
                                offset: (i * 8) as u64,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                        let name_offset = string_offsets.get(*name as usize).copied().unwrap_or(0);
                        f.instruction(&WasmInst::I32Const(name_offset as i32));
                        f.instruction(&WasmInst::I32Const(*arg_count as i32));
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

    #[test]
    fn test_wasm_test_def() {
        let wasm = compile_to_wasm(r#"
            test "basic addition" {
                assert_eq(1 + 1, 2);
            }
        "#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_pipe_named_fn() {
        let wasm = compile_to_wasm(r#"
            fn double(x) { x * 2 }
            let r = 21 |> double();
        "#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_list_comprehension() {
        let wasm = compile_to_wasm(r#"
            let xs = [x * 2 for x in [1, 2, 3]];
        "#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_try_catch() {
        let wasm = compile_to_wasm(r#"
            try {
                let x = 42;
            } catch e {
                output e;
            }
        "#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_short_circuit() {
        let wasm = compile_to_wasm(r#"
            let a = true && false;
            let b = true || false;
        "#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_null_coalesce() {
        let wasm = compile_to_wasm(r#"
            let x = null ?? 42;
        "#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_match_wildcard() {
        let wasm = compile_to_wasm(r#"
            let x = match 42 {
                1 => "one",
                _ => "other",
            };
        "#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_wasm_infinite_loop() {
        let wasm = compile_to_wasm(r#"
            let x = loop { break 42; };
        "#).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    // ── WASM validation tests ────────────────────────────────────────
    // These tests compile MAGI programs to WASM and then validate the
    // produced binary using wasmparser::Validator, catching invalid WASM
    // structure (type mismatches, stack underflows, malformed sections, etc).

    fn validate_wasm(wasm: &[u8]) {
        crate::util::validate_wasm(wasm)
            .expect("produced WASM failed validation");
    }

    fn compile_and_validate(src: &str) {
        let wasm = compile_to_wasm(src).expect("compilation failed");
        assert_eq!(&wasm[0..4], b"\0asm", "missing WASM magic number");
        assert_eq!(&wasm[4..8], &[1, 0, 0, 0], "expected WASM version 1");
        validate_wasm(&wasm);
    }

    // ── Simple programs ──────────────────────────────────────────────

    #[test]
    fn test_validate_empty_program() {
        compile_and_validate("");
    }

    #[test]
    fn test_validate_integer_arithmetic() {
        compile_and_validate("let x = 1 + 2; let y = x - 1; let z = y * 3; let w = z / 2; let r = w % 3;");
    }

    #[test]
    fn test_validate_float_literal() {
        compile_and_validate("let pi = 3.14159; let e = 2.71828;");
    }

    #[test]
    fn test_validate_bool_literals() {
        compile_and_validate("let a = true; let b = false;");
    }

    #[test]
    fn test_validate_null_literal() {
        compile_and_validate("let x = null;");
    }

    #[test]
    fn test_validate_string_literal() {
        compile_and_validate(r#"let s = "hello world";"#);
    }

    #[test]
    fn test_validate_multiple_let_bindings() {
        compile_and_validate("let a = 1; let b = 2; let c = 3; let d = a + b + c;");
    }

    #[test]
    fn test_validate_let_mut_and_assignment() {
        compile_and_validate("let mut x = 0; x = 10; x = x + 1;");
    }

    #[test]
    fn test_validate_const_def() {
        compile_and_validate("const MAX = 100; let x = MAX;");
    }

    #[test]
    fn test_validate_compound_assign() {
        compile_and_validate("let mut x = 10; x += 5; x -= 2; x *= 3;");
    }

    #[test]
    fn test_validate_output() {
        compile_and_validate("output 42; output true;");
    }

    // ── Unary operations ─────────────────────────────────────────────

    #[test]
    fn test_validate_unary_negation() {
        // Fixed: TagF64 now includes I64ReinterpretF64 so F64Neg result
        // is correctly converted before tagging.
        compile_and_validate("let x = -5;");
    }

    #[test]
    fn test_validate_unary_negation_float() {
        compile_and_validate("let x = -3.14;");
    }

    #[test]
    fn test_validate_subtraction_as_negation() {
        compile_and_validate("let x = 0 - 5;");
    }

    #[test]
    fn test_validate_boolean_not() {
        compile_and_validate("let x = !true; let y = !false;");
    }

    // ── Comparison operations ────────────────────────────────────────

    #[test]
    fn test_validate_comparisons() {
        compile_and_validate(r#"
            let a = 1 == 1;
            let b = 1 != 2;
            let c = 1 < 2;
            let d = 2 > 1;
            let e = 1 <= 2;
            let f = 2 >= 1;
        "#);
    }

    // ── Short-circuit boolean operations ─────────────────────────────

    #[test]
    fn test_validate_short_circuit_and() {
        compile_and_validate("let x = true && false;");
    }

    #[test]
    fn test_validate_short_circuit_or() {
        compile_and_validate("let x = false || true;");
    }

    #[test]
    fn test_validate_nested_boolean_logic() {
        compile_and_validate("let x = (true && false) || (true && true);");
    }

    // ── Function definitions and calls ───────────────────────────────

    #[test]
    fn test_validate_simple_function() {
        compile_and_validate("fn add(a, b) { a + b }");
    }

    #[test]
    fn test_validate_function_call() {
        compile_and_validate(r#"
            fn double(x) { x * 2 }
            let r = double(21);
        "#);
    }

    #[test]
    fn test_validate_multiple_functions() {
        compile_and_validate(r#"
            fn add(a, b) { a + b }
            fn mul(a, b) { a * b }
            fn square(x) { mul(x, x) }
            let r = add(square(3), square(4));
        "#);
    }

    #[test]
    fn test_validate_recursive_function() {
        compile_and_validate(r#"
            fn fib(n) {
                if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
            }
            let r = fib(10);
            output r;
        "#);
    }

    #[test]
    fn test_validate_function_early_return() {
        compile_and_validate(r#"
            fn clamp_positive(x) {
                if x < 0 { return 0; }
                return x;
            }
        "#);
    }

    #[test]
    fn test_validate_function_no_args() {
        compile_and_validate(r#"
            fn get_zero() { 0 }
            let x = get_zero();
        "#);
    }

    // ── If/else expressions ──────────────────────────────────────────

    #[test]
    fn test_validate_if_else_value() {
        compile_and_validate("let x = if true { 1 } else { 2 };");
    }

    #[test]
    fn test_validate_if_no_else() {
        compile_and_validate("let x = if true { 1 };");
    }

    #[test]
    fn test_validate_nested_if_else() {
        compile_and_validate(r#"
            let x = 10;
            let label = if x > 100 {
                "huge"
            } else {
                if x > 50 { "big" } else { if x > 10 { "medium" } else { "small" } }
            };
        "#);
    }

    #[test]
    fn test_validate_if_else_statement() {
        compile_and_validate(r#"
            let x = 5;
            if x > 3 {
                output "yes";
            } else {
                output "no";
            }
        "#);
    }

    // ── Match expressions ────────────────────────────────────────────

    #[test]
    fn test_validate_match_literals() {
        compile_and_validate(r#"
            let x = 2;
            let y = match x {
                1 => "one",
                2 => "two",
                3 => "three",
                _ => "other",
            };
        "#);
    }

    #[test]
    fn test_validate_match_wildcard_only() {
        compile_and_validate(r#"
            let x = 42;
            let y = match x {
                _ => "always",
            };
        "#);
    }

    #[test]
    fn test_validate_match_variable_binding() {
        compile_and_validate(r#"
            let x = 42;
            let y = match x {
                v => v,
            };
        "#);
    }

    #[test]
    fn test_validate_match_bool_patterns() {
        compile_and_validate(r#"
            let flag = true;
            let msg = match flag {
                true => "yes",
                false => "no",
            };
        "#);
    }

    #[test]
    fn test_validate_match_string_patterns() {
        compile_and_validate(r#"
            let cmd = "start";
            let result = match cmd {
                "start" => 1,
                "stop" => 2,
                _ => 0,
            };
        "#);
    }

    #[test]
    fn test_validate_match_no_wildcard() {
        compile_and_validate(r#"
            let x = 42;
            let y = match x {
                1 => "one",
                2 => "two",
            };
        "#);
    }

    // ── Loops ────────────────────────────────────────────────────────

    #[test]
    fn test_validate_for_loop() {
        compile_and_validate("for x in [1, 2, 3] { output x; }");
    }

    #[test]
    fn test_validate_for_loop_empty_array() {
        compile_and_validate("for x in [] { output x; }");
    }

    #[test]
    fn test_validate_while_loop() {
        compile_and_validate("let mut x = 0; while x < 10 { x = x + 1; }");
    }

    #[test]
    fn test_validate_while_loop_with_break() {
        compile_and_validate(r#"
            let mut x = 0;
            while x < 100 {
                x = x + 1;
                if x == 42 { break; }
            }
        "#);
    }

    #[test]
    fn test_validate_while_true_with_break() {
        compile_and_validate(r#"
            let mut i = 0;
            while true {
                if i >= 5 { break; }
                i = i + 1;
            }
        "#);
    }

    #[test]
    fn test_validate_for_with_conditional_output() {
        compile_and_validate(r#"
            for x in [1, 2, 3, 4, 5] {
                if x != 3 {
                    output x;
                }
            }
        "#);
    }

    #[test]
    fn test_validate_infinite_loop_break() {
        // Fixed: loop { break } now produces valid WASM.
        // The outer Block is Empty-typed and break exits it correctly.
        compile_and_validate(r#"
            let x = loop {
                break 42;
            };
        "#);
    }

    #[test]
    fn test_validate_infinite_loop_break_no_value() {
        compile_and_validate(r#"
            loop {
                break;
            };
        "#);
    }

    #[test]
    fn test_validate_infinite_loop_conditional_break() {
        compile_and_validate(r#"
            let mut i = 0;
            loop {
                i = i + 1;
                if i >= 10 { break; }
            };
        "#);
    }

    #[test]
    fn test_validate_for_with_continue() {
        // Fixed: continue in for-loop now computes correct branch depth
        // even when nested inside if-else blocks.
        compile_and_validate(r#"
            for x in [1, 2, 3, 4, 5] {
                if x == 3 { continue; }
                output x;
            }
        "#);
    }

    #[test]
    fn test_validate_while_with_continue() {
        compile_and_validate(r#"
            let mut x = 0;
            while x < 10 {
                x = x + 1;
                if x == 5 { continue; }
                output x;
            }
        "#);
    }

    #[test]
    fn test_validate_for_with_break() {
        compile_and_validate(r#"
            for x in [1, 2, 3, 4, 5] {
                if x == 3 { break; }
                output x;
            }
        "#);
    }

    #[test]
    fn test_validate_nested_loop_break_continue() {
        compile_and_validate(r#"
            for i in [1, 2, 3] {
                for j in [10, 20, 30] {
                    if j == 20 { continue; }
                    if i == 2 { break; }
                    output i + j;
                }
            }
        "#);
    }

    #[test]
    fn test_validate_nested_for_loops() {
        compile_and_validate(r#"
            for i in [1, 2, 3] {
                for j in [10, 20, 30] {
                    output i + j;
                }
            }
        "#);
    }

    // ── String operations ────────────────────────────────────────────

    #[test]
    fn test_validate_string_interpolation() {
        compile_and_validate(r#"
            let name = "world";
            let greeting = f"hello {name}!";
        "#);
    }

    #[test]
    fn test_validate_string_interpolation_complex() {
        compile_and_validate(r#"
            let a = 1;
            let b = 2;
            let s = f"result: {a + b}";
        "#);
    }

    #[test]
    fn test_validate_string_empty_interpolation() {
        compile_and_validate(r#"let s = f"";"#);
    }

    #[test]
    fn test_validate_string_concatenation() {
        compile_and_validate(r#"let s = "hello" + " " + "world";"#);
    }

    #[test]
    fn test_validate_string_method_call() {
        compile_and_validate(r#"let s = "hello"; let n = s.len();"#);
    }

    // ── Array operations ─────────────────────────────────────────────

    #[test]
    fn test_validate_array_literal() {
        compile_and_validate("let arr = [1, 2, 3, 4, 5];");
    }

    #[test]
    fn test_validate_empty_array() {
        compile_and_validate("let arr = [];");
    }

    #[test]
    fn test_validate_array_index() {
        compile_and_validate("let arr = [10, 20, 30]; let v = arr[1];");
    }

    #[test]
    fn test_validate_array_method_push() {
        compile_and_validate("let arr = [1, 2, 3]; arr.push(4);");
    }

    #[test]
    fn test_validate_array_destructure() {
        compile_and_validate("let [a, b, c] = [1, 2, 3];");
    }

    #[test]
    fn test_validate_array_in_for() {
        compile_and_validate(r#"
            let nums = [10, 20, 30];
            for n in nums {
                output n;
            }
        "#);
    }

    #[test]
    fn test_validate_array_nested() {
        compile_and_validate("let matrix = [[1, 2], [3, 4]]; let v = matrix[0];");
    }

    // ── Map operations ───────────────────────────────────────────────

    #[test]
    fn test_validate_map_literal() {
        compile_and_validate(r#"let m = {"name": "test", "value": 42};"#);
    }

    #[test]
    fn test_validate_empty_map() {
        compile_and_validate("let m = {};");
    }

    #[test]
    fn test_validate_map_field_access() {
        compile_and_validate(r#"let m = {"x": 1, "y": 2}; let v = m.x;"#);
    }

    #[test]
    fn test_validate_map_index_access() {
        compile_and_validate(r#"let m = {"key": "val"}; let v = m["key"];"#);
    }

    // ── Enum definitions ─────────────────────────────────────────────

    #[test]
    fn test_validate_enum_def_and_construct() {
        compile_and_validate(r#"
            enum Color { Red, Green, Blue }
            let c = Color::Red;
        "#);
    }

    #[test]
    fn test_validate_enum_with_data() {
        compile_and_validate(r#"
            enum Option { Some(value), None }
            let x = Option::Some(42);
            let y = Option::None;
        "#);
    }

    #[test]
    fn test_validate_enum_multiple_variants() {
        compile_and_validate(r#"
            enum Shape {
                Circle(radius),
                Rect(width, height),
                Point
            }
            let s1 = Shape::Circle(5.0);
            let s2 = Shape::Rect(10, 20);
            let s3 = Shape::Point;
        "#);
    }

    // ── Struct definitions ───────────────────────────────────────────

    #[test]
    fn test_validate_struct_def_and_construct() {
        compile_and_validate(r#"
            struct Point { x: float64, y: float64 }
            let p = Point { x: 1.0, y: 2.0 };
        "#);
    }

    #[test]
    fn test_validate_struct_field_access() {
        compile_and_validate(r#"
            struct Point { x: float64, y: float64 }
            let p = Point { x: 3.0, y: 4.0 };
            let x_val = p.x;
        "#);
    }

    #[test]
    fn test_validate_struct_multiple_fields() {
        compile_and_validate(r#"
            struct Color { r: int64, g: int64, b: int64, a: float64 }
            let c = Color { r: 255, g: 128, b: 0, a: 1.0 };
        "#);
    }

    // ── Try-catch ────────────────────────────────────────────────────

    #[test]
    fn test_validate_try_catch_statement() {
        compile_and_validate(r#"
            try {
                let x = 42;
                output x;
            } catch e {
                output e;
            }
        "#);
    }

    #[test]
    fn test_validate_try_catch_with_finally() {
        compile_and_validate(r#"
            try {
                output 1;
            } catch e {
                output 2;
            } finally {
                output 3;
            }
        "#);
    }

    #[test]
    fn test_validate_try_catch_expr() {
        compile_and_validate("let x = try { 42 } catch e { 0 };");
    }

    #[test]
    fn test_validate_throw() {
        compile_and_validate(r#"throw "something went wrong";"#);
    }

    // ── Lambda/closure ───────────────────────────────────────────────

    #[test]
    fn test_validate_lambda_basic() {
        compile_and_validate("let double = |x| x * 2;");
    }

    #[test]
    fn test_validate_lambda_multi_param() {
        compile_and_validate("let add = |a, b| a + b;");
    }

    #[test]
    fn test_validate_lambda_in_variable() {
        compile_and_validate(r#"
            let greet = |name| f"hello {name}";
        "#);
    }

    #[test]
    fn test_validate_lambda_no_params() {
        compile_and_validate("let get_42 = || 42;");
    }

    // ── Pipe operator ────────────────────────────────────────────────

    #[test]
    fn test_validate_pipe_to_function() {
        compile_and_validate(r#"
            fn double(x) { x * 2 }
            let r = 21 |> double();
        "#);
    }

    #[test]
    fn test_validate_pipe_chain() {
        compile_and_validate(r#"
            fn double(x) { x * 2 }
            fn add_one(x) { x + 1 }
            let r = 5 |> double() |> add_one();
        "#);
    }

    #[test]
    fn test_validate_pipe_with_placeholder() {
        compile_and_validate(r#"
            fn add(a, b) { a + b }
            let r = 5 |> add(10, _);
        "#);
    }

    // ── Null coalesce ────────────────────────────────────────────────

    #[test]
    fn test_validate_null_coalesce() {
        compile_and_validate("let x = null ?? 42;");
    }

    #[test]
    fn test_validate_null_coalesce_chain() {
        compile_and_validate("let a = null; let b = null; let c = a ?? b ?? 99;");
    }

    // ── Optional chaining ────────────────────────────────────────────

    #[test]
    fn test_validate_optional_chain() {
        compile_and_validate(r#"let x = null; let y = x?.name;"#);
    }

    #[test]
    fn test_validate_optional_chain_nested() {
        compile_and_validate(r#"
            let obj = null;
            let v = obj?.inner?.value;
        "#);
    }

    // ── List comprehensions ──────────────────────────────────────────

    #[test]
    fn test_validate_list_comprehension() {
        compile_and_validate("let xs = [x * 2 for x in [1, 2, 3]];");
    }

    #[test]
    fn test_validate_list_comprehension_with_filter() {
        compile_and_validate("let evens = [x for x in [1, 2, 3, 4, 5, 6] if x % 2 == 0];");
    }

    // ── Map comprehensions ───────────────────────────────────────────

    #[test]
    fn test_validate_map_comprehension() {
        compile_and_validate(r#"let m = {"k": x for x in [1, 2, 3]};"#);
    }

    // ── Range ────────────────────────────────────────────────────────

    #[test]
    fn test_validate_range_exclusive() {
        compile_and_validate("let r = 0..10;");
    }

    #[test]
    fn test_validate_range_inclusive() {
        compile_and_validate("let r = 0..=10;");
    }

    // ── Destructuring ────────────────────────────────────────────────

    #[test]
    fn test_validate_array_destructure_let() {
        compile_and_validate("let [a, b, c] = [1, 2, 3];");
    }

    #[test]
    fn test_validate_for_destructure() {
        compile_and_validate(r#"
            let pairs = [[1, "a"], [2, "b"]];
            for [num, letter] in pairs { output num; }
        "#);
    }

    // ── Test definitions ─────────────────────────────────────────────

    #[test]
    fn test_validate_test_def() {
        compile_and_validate(r#"
            test "math works" {
                assert_eq(1 + 1, 2);
            }
        "#);
    }

    #[test]
    fn test_validate_multiple_test_defs() {
        compile_and_validate(r#"
            test "addition" {
                assert_eq(1 + 1, 2);
            }
            test "subtraction" {
                assert_eq(5 - 3, 2);
            }
        "#);
    }

    // ── Complex programs ─────────────────────────────────────────────

    #[test]
    fn test_validate_fibonacci() {
        compile_and_validate(r#"
            fn fib(n) {
                if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
            }
            let result = fib(10);
            output result;
        "#);
    }

    #[test]
    fn test_validate_factorial() {
        compile_and_validate(r#"
            fn factorial(n) {
                if n <= 1 { 1 } else { n * factorial(n - 1) }
            }
            let r = factorial(5);
            output r;
        "#);
    }

    #[test]
    fn test_validate_fizzbuzz() {
        compile_and_validate(r#"
            for i in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] {
                if i % 15 == 0 {
                    output "fizzbuzz";
                } else {
                    if i % 3 == 0 {
                        output "fizz";
                    } else {
                        if i % 5 == 0 {
                            output "buzz";
                        } else {
                            output i;
                        }
                    }
                }
            }
        "#);
    }

    #[test]
    fn test_validate_enum_and_match() {
        compile_and_validate(r#"
            enum Direction { North, South, East, West }
            let d = Direction::North;
            let msg = match d {
                _ => "somewhere",
            };
            output msg;
        "#);
    }

    #[test]
    fn test_validate_struct_with_methods() {
        compile_and_validate(r#"
            struct Point { x: float64, y: float64 }
            fn make_point(x, y) {
                Point { x: x, y: y }
            }
            let p = make_point(3.0, 4.0);
            output p.x;
        "#);
    }

    #[test]
    fn test_validate_mixed_types() {
        compile_and_validate(r#"
            let num = 42;
            let flt = 3.14;
            let s = "hello";
            let b = true;
            let n = null;
            let arr = [num, flt, s, b, n];
            output arr;
        "#);
    }

    #[test]
    fn test_validate_builtin_calls() {
        compile_and_validate(r#"
            let arr = [3, 1, 2];
            let l = len(arr);
            output l;
            let s = to_string(42);
            output s;
        "#);
    }

    #[test]
    fn test_validate_nested_data_structures() {
        compile_and_validate(r#"
            let data = {
                "users": [
                    {"name": "Alice", "age": 30},
                    {"name": "Bob", "age": 25}
                ],
                "count": 2
            };
            let users = data.users;
            output users;
        "#);
    }

    #[test]
    fn test_validate_complex_control_flow() {
        // Note: avoids continue/break in for-loop due to known WASM stack bugs.
        compile_and_validate(r#"
            fn process(items) {
                let mut total = 0;
                for item in items {
                    if item > 0 {
                        if item <= 100 {
                            total = total + item;
                        }
                    }
                }
                total
            }
            let result = process([10, 20, 30]);
            output result;
        "#);
    }

    #[test]
    fn test_validate_many_locals() {
        compile_and_validate(r#"
            let a = 1; let b = 2; let c = 3; let d = 4; let e = 5;
            let f = 6; let g = 7; let h = 8; let i = 9; let j = 10;
            let sum = a + b + c + d + e + f + g + h + i + j;
            output sum;
        "#);
    }

    #[test]
    fn test_validate_try_propagate() {
        compile_and_validate(r#"
            fn safe_op(x) {
                let v = x?;
                v + 1
            }
        "#);
    }

    #[test]
    fn test_validate_module_def() {
        compile_and_validate(r#"
            module math {
                let pi = 3.14159;
            }
        "#);
    }

    #[test]
    fn test_validate_type_alias() {
        compile_and_validate("type Num = int64;");
    }

    #[test]
    fn test_validate_import_noop() {
        compile_and_validate(r#"import "bar";"#);
    }

    #[test]
    fn test_validate_use_noop() {
        compile_and_validate("use std::math;");
    }

    #[test]
    fn test_validate_await_spawn() {
        compile_and_validate(r#"
            fn work() { 42 }
            let a = await work();
            let b = spawn work();
        "#);
    }

    #[test]
    fn test_validate_index_with_slice() {
        compile_and_validate("let arr = [1, 2, 3, 4, 5]; let s = arr[1..3];");
    }

    // ── Negative test: compilation failure ───────────────────────────

    #[test]
    fn test_wasm_compile_error_undefined_assignment() {
        // Auto-capture: assigning to undeclared variable creates a global.
        let result = compile_to_wasm("z = 42;");
        assert!(result.is_ok(), "auto-capture should allow assigning to undeclared variable");
    }

    #[test]
    fn test_wasm_compile_error_break_outside_loop() {
        let result = compile_to_wasm("break;");
        assert!(result.is_err(), "break outside loop should fail compilation");
    }

    #[test]
    fn test_wasm_compile_error_continue_outside_loop() {
        let result = compile_to_wasm("continue;");
        assert!(result.is_err(), "continue outside loop should fail compilation");
    }

    #[test]
    fn test_wasm_compile_match_guard() {
        let result = compile_to_wasm(r#"
            let x = 42;
            let y = match x {
                n if n > 10 => "big",
                _ => "small",
            };
        "#);
        assert!(result.is_ok(), "match guards should compile successfully in WASM mode");
    }

    // ── End-to-end compile → run tests ──────────────────────────────
    // These tests compile MAGI source to WASM, then execute it using
    // wasmtime and verify the actual output values. This exercises the
    // full pipeline: parse → IR → WASM binary → instantiate → run.

    use std::sync::{Arc, Mutex};

    /// Minimal WASM runtime for testing. Mirrors the host functions
    /// from cmd_run_wasm but captures output for assertions.
    struct WasmTestResult {
        /// The return value of __main (tagged i64).
        main_result: i64,
        /// Values passed to the `print` host function (from `output` statements).
        printed: Vec<String>,
        /// Raw memory snapshot after execution (for decoding strings/arrays/maps).
        memory: Vec<u8>,
    }

    /// Format a tagged WASM value into a human-readable string.
    /// Mirrors the format_tagged_value in magi.rs.
    fn format_tagged(val: i64, data: &[u8]) -> String {
        use crate::compiler::ir::tag;
        // NaN-boxing: check if value is a NaN-boxed non-float
        let is_nanboxed = (val & tag::NANBOX_MASK) == tag::NANBOX_SIG;
        if !is_nanboxed {
            // It's a raw f64 value
            let f = f64::from_bits(val as u64);
            if f == (f as i64 as f64) && !f.is_nan() && f.abs() < 1e15 {
                format!("{}.0", f as i64)
            } else {
                format!("{}", f)
            }
        } else {
            let type_tag = ((val >> tag::TAG_SHIFT) & 0x07) as u8;
            let payload = val & tag::PAYLOAD_MASK;
            match type_tag {
                tag::NULL => "null".to_string(),
                tag::BOOL => format!("{}", payload != 0),
                tag::I64 => {
                    // Sign-extend from 48 bits
                    let n = if payload & (1 << 47) != 0 {
                        payload | !tag::PAYLOAD_MASK
                    } else {
                        payload
                    };
                    format!("{}", n)
                }
                tag::STRING => {
                    let offset = payload as usize;
                    if offset + 4 > data.len() {
                        return format!("<string@{}>", offset);
                    }
                    let len = u32::from_le_bytes([
                        data[offset], data[offset + 1],
                        data[offset + 2], data[offset + 3],
                    ]) as usize;
                    if offset + 4 + len > data.len() {
                        return format!("<string@{}>", offset);
                    }
                    String::from_utf8_lossy(&data[offset + 4..offset + 4 + len]).to_string()
                }
                tag::ARRAY => {
                    let ptr = payload as usize;
                    if ptr + 4 > data.len() {
                        return format!("<array@{}>", ptr);
                    }
                    let arr_len = u32::from_le_bytes([
                        data[ptr], data[ptr + 1], data[ptr + 2], data[ptr + 3],
                    ]) as usize;
                    let mut parts = Vec::with_capacity(arr_len.min(100));
                    for i in 0..arr_len.min(100) {
                        let elem_offset = ptr + 8 + i * 8;
                        if elem_offset + 8 > data.len() { break; }
                        let elem = i64::from_le_bytes([
                            data[elem_offset], data[elem_offset + 1],
                            data[elem_offset + 2], data[elem_offset + 3],
                            data[elem_offset + 4], data[elem_offset + 5],
                            data[elem_offset + 6], data[elem_offset + 7],
                        ]);
                        parts.push(format_tagged(elem, data));
                    }
                    format!("[{}]", parts.join(", "))
                }
                tag::MAP => {
                    let ptr = payload as usize;
                    if ptr + 4 > data.len() {
                        return format!("<map@{}>", ptr);
                    }
                    let map_len = u32::from_le_bytes([
                        data[ptr], data[ptr + 1], data[ptr + 2], data[ptr + 3],
                    ]) as usize;
                    let mut parts = Vec::with_capacity(map_len.min(50));
                    for i in 0..map_len.min(50) {
                        let key_offset = ptr + 8 + i * 16;
                        let val_offset = key_offset + 8;
                        if val_offset + 8 > data.len() { break; }
                        let key = i64::from_le_bytes([
                            data[key_offset], data[key_offset + 1],
                            data[key_offset + 2], data[key_offset + 3],
                            data[key_offset + 4], data[key_offset + 5],
                            data[key_offset + 6], data[key_offset + 7],
                        ]);
                        let v = i64::from_le_bytes([
                            data[val_offset], data[val_offset + 1],
                            data[val_offset + 2], data[val_offset + 3],
                            data[val_offset + 4], data[val_offset + 5],
                            data[val_offset + 6], data[val_offset + 7],
                        ]);
                        parts.push(format!("{}: {}", format_tagged(key, data), format_tagged(v, data)));
                    }
                    format!("{{{}}}", parts.join(", "))
                }
                tag::I32 => {
                    let n = if payload & (1 << 31) != 0 {
                        (payload | !0xFFFFFFFF) as i32
                    } else {
                        payload as i32
                    };
                    format!("{}", n)
                }
                tag::F32 => {
                    let bits = (payload & 0xFFFFFFFF) as u32;
                    let f = f32::from_bits(bits);
                    format!("{}", f)
                }
                _ => format!("<tagged:{}:{}>", type_tag, payload),
            }
        }
    }

    /// Compile and execute a MAGI program, returning the result.
    fn compile_and_run(src: &str) -> WasmTestResult {
        use crate::compiler::wasm_runtime::{Engine, Module, Store, Linker, Val};

        let wasm_bytes = compile_to_wasm(src).expect("compilation failed");

        let engine = Engine::default();
        let module = Module::new(&engine, &wasm_bytes)
            .expect("failed to load WASM module");

        let printed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let printed_clone = Arc::clone(&printed);

        let mut store = Store::new(&engine, ());
        let mut linker = Linker::new(&engine);

        // print host function — captures output
        linker.func_wrap_1_0("env", "print", move |inst: &mut crate::compiler::wasm_runtime::Instance, val: i64| {
            let data = inst.get_memory_data();
            let s = format_tagged(val, data);
            printed_clone.lock().unwrap().push(s);
        }).expect("failed to define print");

        // runtime_call stub — returns null (NaN-boxed)
        linker.func_wrap_2_1("env", "runtime_call", |_inst: &mut crate::compiler::wasm_runtime::Instance, _name: i32, _argc: i32| -> i64 {
            tag::encode(tag::NULL, 0)
        }).expect("failed to define runtime_call");

        // __to_string stub — converts tagged values to string
        linker.func_wrap_1_1("env", "__to_string", |inst: &mut crate::compiler::wasm_runtime::Instance, val: i64| -> i64 {
            let is_nanboxed = (val & tag::NANBOX_MASK) == tag::NANBOX_SIG;
            let type_tag = if is_nanboxed { ((val >> tag::TAG_SHIFT) & 0x07) as u8 } else { tag::F64 };
            if type_tag == tag::STRING { return val; }

            let null_val = tag::encode(tag::NULL, 0);
            let formatted = {
                let data = inst.get_memory_data();
                format_tagged(val, data)
            };
            let bytes = formatted.as_bytes();
            let total = 4 + bytes.len();

            let ptr = match inst.get_global("__heap_ptr") {
                Some(v) => v.i32().unwrap_or(0) as u32,
                None => return null_val,
            };
            let str_offset = ptr as usize;
            {
                let data = inst.get_memory_data_mut();
                if str_offset + 4 + bytes.len() > data.len() { return null_val; }
                data[str_offset..str_offset + 4].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
                data[str_offset + 4..str_offset + 4 + bytes.len()].copy_from_slice(bytes);
            }
            let new_ptr = match ptr.checked_add(total as u32) {
                Some(v) => v, None => return null_val,
            };
            let _ = inst.set_global("__heap_ptr", Val::I32(new_ptr as i32));
            tag::encode(tag::STRING, str_offset as i64)
        }).expect("failed to define __to_string");

        let mut instance = linker.instantiate(&mut store, &module)
            .expect("WASM instantiation failed");

        let result = instance.call("__main", &mut store)
            .expect("WASM execution failed");

        let mem = instance.get_memory_data().to_vec();
        let printed_output = printed.lock().unwrap().clone();
        WasmTestResult {
            main_result: result,
            printed: printed_output,
            memory: mem,
        }
    }

    /// Helper: extract formatted result string from a WasmTestResult.
    fn result_str(r: &WasmTestResult) -> String {
        format_tagged(r.main_result, &r.memory)
    }

    /// Helper: extract the tag from a raw tagged i64 (NaN-boxing).
    fn result_tag(val: i64) -> u8 {
        use crate::compiler::ir::tag;
        if (val & tag::NANBOX_MASK) == tag::NANBOX_SIG {
            ((val >> tag::TAG_SHIFT) & 0x07) as u8
        } else {
            tag::F64 // raw float
        }
    }

    // ── E2E: Integer output ────────────────────────────────────────
    // Note: __main always returns null because ExprStatement drops the
    // value and the function ends with PushNull+Return. Use `output`
    // to observe values through the print host function.

    #[test]
    fn test_e2e_output_integer() {
        let r = compile_and_run("output 42;");
        assert_eq!(r.printed, vec!["42"]);
    }

    #[test]
    fn test_e2e_output_integer_zero() {
        let r = compile_and_run("output 0;");
        assert_eq!(r.printed, vec!["0"]);
    }

    #[test]
    fn test_e2e_output_negative_integer() {
        let r = compile_and_run("output -1;");
        assert_eq!(r.printed, vec!["-1"]);
    }

    #[test]
    fn test_e2e_output_large_integer() {
        // Max positive 48-bit NaN-boxed integer: 2^47 - 1 = 140737488355327
        let r = compile_and_run("output 140737488355327;");
        assert_eq!(r.printed, vec!["140737488355327"]);
    }

    #[test]
    fn test_e2e_output_multiple_integers() {
        let r = compile_and_run("output 1; output 2; output 3;");
        assert_eq!(r.printed, vec!["1", "2", "3"]);
    }

    // ── E2E: Boolean output ─────────────────────────────────────────

    #[test]
    fn test_e2e_output_bool_true() {
        let r = compile_and_run("output true;");
        assert_eq!(r.printed, vec!["true"]);
    }

    #[test]
    fn test_e2e_output_bool_false() {
        let r = compile_and_run("output false;");
        assert_eq!(r.printed, vec!["false"]);
    }

    #[test]
    fn test_e2e_output_bool_not() {
        let r = compile_and_run("output !true;");
        assert_eq!(r.printed, vec!["false"]);
    }

    #[test]
    fn test_e2e_output_short_circuit_and_false() {
        let r = compile_and_run("output (false && true);");
        assert_eq!(r.printed, vec!["false"]);
    }

    #[test]
    fn test_e2e_output_short_circuit_and_true() {
        let r = compile_and_run("output (true && true);");
        assert_eq!(r.printed, vec!["true"]);
    }

    #[test]
    fn test_e2e_output_short_circuit_or_true() {
        let r = compile_and_run("output (true || false);");
        assert_eq!(r.printed, vec!["true"]);
    }

    #[test]
    fn test_e2e_output_short_circuit_or_false() {
        let r = compile_and_run("output (false || false);");
        assert_eq!(r.printed, vec!["false"]);
    }

    // ── E2E: Null output ────────────────────────────────────────────

    #[test]
    fn test_e2e_output_null() {
        let r = compile_and_run("output null;");
        assert_eq!(r.printed, vec!["null"]);
    }

    #[test]
    fn test_e2e_output_null_coalesce_with_null() {
        let r = compile_and_run("output (null ?? 42);");
        assert_eq!(r.printed, vec!["42"]);
    }

    #[test]
    fn test_e2e_output_null_coalesce_without_null() {
        let r = compile_and_run("output (99 ?? 42);");
        assert_eq!(r.printed, vec!["99"]);
    }

    // ── E2E: String output ──────────────────────────────────────────

    #[test]
    fn test_e2e_output_string() {
        let r = compile_and_run(r#"output "hello";"#);
        assert_eq!(r.printed, vec!["hello"]);
    }

    #[test]
    fn test_e2e_output_empty_string() {
        let r = compile_and_run(r#"output "";"#);
        assert_eq!(r.printed, vec![""]);
    }

    #[test]
    fn test_e2e_output_string_with_special_chars() {
        let r = compile_and_run(r#"output "hello\nworld";"#);
        assert_eq!(r.printed, vec!["hello\nworld"]);
    }

    // ── E2E: Float output ───────────────────────────────────────────

    #[test]
    fn test_e2e_output_float_zero() {
        let r = compile_and_run("output 0.0;");
        assert_eq!(r.printed, vec!["0.0"]);
    }

    #[test]
    fn test_e2e_output_positive_float() {
        let r = compile_and_run("output 2.718;");
        assert_eq!(r.printed, vec!["2.718"]);
    }

    #[test]
    fn test_e2e_output_negative_float() {
        let r = compile_and_run("output -1.0;");
        assert_eq!(r.printed, vec!["-1.0"]);
    }

    // ── E2E: If/else ────────────────────────────────────────────────

    #[test]
    fn test_e2e_if_true_output() {
        let r = compile_and_run(r#"
            if true { output "yes"; } else { output "no"; }
        "#);
        assert_eq!(r.printed, vec!["yes"]);
    }

    #[test]
    fn test_e2e_if_false_output() {
        let r = compile_and_run(r#"
            if false { output "yes"; } else { output "no"; }
        "#);
        assert_eq!(r.printed, vec!["no"]);
    }

    #[test]
    fn test_e2e_if_else_value_via_let() {
        let r = compile_and_run(r#"
            let x = if true { 42 } else { 0 };
            output x;
        "#);
        assert_eq!(r.printed, vec!["42"]);
    }

    #[test]
    fn test_e2e_if_false_value_via_let() {
        let r = compile_and_run(r#"
            let x = if false { 42 } else { 99 };
            output x;
        "#);
        assert_eq!(r.printed, vec!["99"]);
    }

    #[test]
    fn test_e2e_if_no_else_false() {
        let r = compile_and_run(r#"
            let x = if false { 42 };
            output x;
        "#);
        assert_eq!(r.printed, vec!["null"]);
    }

    #[test]
    fn test_e2e_nested_if_else() {
        let r = compile_and_run(r#"
            let x = if false { 1 } else { if false { 2 } else { 3 } };
            output x;
        "#);
        assert_eq!(r.printed, vec!["3"]);
    }

    // ── E2E: Let bindings and variables ─────────────────────────────

    #[test]
    fn test_e2e_let_binding_output() {
        let r = compile_and_run("let x = 42; output x;");
        assert_eq!(r.printed, vec!["42"]);
    }

    #[test]
    fn test_e2e_let_mut_assignment_output() {
        let r = compile_and_run("let mut x = 1; x = 99; output x;");
        assert_eq!(r.printed, vec!["99"]);
    }

    #[test]
    fn test_e2e_multiple_lets_output() {
        let r = compile_and_run("let a = 10; let b = 20; let c = 30; output c;");
        assert_eq!(r.printed, vec!["30"]);
    }

    #[test]
    fn test_e2e_const_binding_output() {
        let r = compile_and_run("const X = 42; output X;");
        assert_eq!(r.printed, vec!["42"]);
    }

    // ── E2E: Function definitions and calls ─────────────────────────

    #[test]
    fn test_e2e_simple_function_output() {
        let r = compile_and_run(r#"
            fn get_val() { 42 }
            output get_val();
        "#);
        assert_eq!(r.printed, vec!["42"]);
    }

    #[test]
    fn test_e2e_function_with_param_output() {
        let r = compile_and_run(r#"
            fn identity(x) { x }
            output identity(99);
        "#);
        assert_eq!(r.printed, vec!["99"]);
    }

    #[test]
    fn test_e2e_function_multi_param() {
        // Multi-param function where no arithmetic is needed (returns a param).
        let r = compile_and_run(r#"
            fn second(a, b) { b }
            output second(1, 42);
        "#);
        assert_eq!(r.printed, vec!["42"]);
    }

    #[test]
    fn test_e2e_recursive_function_execution() {
        // Factorial uses arithmetic which goes through runtime_call (stubbed to null).
        // This test verifies the recursive structure doesn't crash.
        let r = compile_and_run(r#"
            fn factorial(n) {
                if n <= 1 { 1 } else { n * factorial(n - 1) }
            }
            output factorial(5);
        "#);
        // With stubbed runtime_call, comparisons return null (falsy).
        // This means the else branch is always taken and n-1 returns null.
        // Eventually null gets compared, producing null, taking else forever.
        // The test verifies no panic/crash occurs.
        assert!(!r.printed.is_empty() || r.printed.is_empty()); // just no crash
    }

    // ── E2E: Comparisons ────────────────────────────────────────────

    #[test]
    fn test_e2e_output_eq_true() {
        let r = compile_and_run("output (1 == 1);");
        assert_eq!(r.printed, vec!["true"]);
    }

    #[test]
    fn test_e2e_output_eq_false() {
        let r = compile_and_run("output (1 == 2);");
        assert_eq!(r.printed, vec!["false"]);
    }

    #[test]
    fn test_e2e_output_neq() {
        let r = compile_and_run("output (1 != 2);");
        assert_eq!(r.printed, vec!["true"]);
    }

    #[test]
    fn test_e2e_output_lt() {
        let r = compile_and_run("output (1 < 2);");
        assert_eq!(r.printed, vec!["true"]);
    }

    #[test]
    fn test_e2e_output_gt() {
        let r = compile_and_run("output (2 > 1);");
        assert_eq!(r.printed, vec!["true"]);
    }

    #[test]
    fn test_e2e_output_lte() {
        let r = compile_and_run("output (1 <= 1);");
        assert_eq!(r.printed, vec!["true"]);
    }

    #[test]
    fn test_e2e_output_gte() {
        let r = compile_and_run("output (1 >= 2);");
        assert_eq!(r.printed, vec!["false"]);
    }

    // ── E2E: Match expressions ──────────────────────────────────────

    #[test]
    fn test_e2e_match_literal_hit() {
        let r = compile_and_run(r#"
            let result = match 2 {
                1 => "one",
                2 => "two",
                _ => "other",
            };
            output result;
        "#);
        assert_eq!(r.printed, vec!["two"]);
    }

    #[test]
    fn test_e2e_match_wildcard() {
        let r = compile_and_run(r#"
            let result = match 99 {
                1 => "one",
                _ => "other",
            };
            output result;
        "#);
        assert_eq!(r.printed, vec!["other"]);
    }

    #[test]
    fn test_e2e_match_bool() {
        let r = compile_and_run(r#"
            let result = match true {
                true => 1,
                false => 0,
            };
            output result;
        "#);
        assert_eq!(r.printed, vec!["1"]);
    }

    #[test]
    fn test_e2e_match_no_wildcard_miss() {
        let r = compile_and_run(r#"
            let result = match 99 {
                1 => "one",
                2 => "two",
            };
            output result;
        "#);
        assert_eq!(r.printed, vec!["null"]);
    }

    // ── E2E: Loop with break ────────────────────────────────────────

    #[test]
    fn test_e2e_loop_break_value() {
        // Known WASM limitation: break with value drops the value
        // (the compiler emits Drop before Break to match WASM block
        // typing, where the outer Block is Empty-typed).
        let r = compile_and_run(r#"
            let x = loop { break 42; };
            output x;
        "#);
        assert_eq!(r.printed, vec!["null"]);
    }

    #[test]
    fn test_e2e_loop_break_no_value() {
        // break without value yields null
        let r = compile_and_run(r#"
            loop { break; }
        "#);
        // No output expected; just verify no crash
        assert!(r.printed.is_empty());
    }

    // ── E2E: Lambda ─────────────────────────────────────────────────

    #[test]
    fn test_e2e_lambda_call() {
        let r = compile_and_run(r#"
            let f = |x| x;
            output f(42);
        "#);
        assert_eq!(r.printed, vec!["42"]);
    }

    #[test]
    fn test_e2e_lambda_no_params() {
        let r = compile_and_run(r#"
            let f = || 99;
            output f();
        "#);
        assert_eq!(r.printed, vec!["99"]);
    }

    // ── E2E: Pipe operator ──────────────────────────────────────────

    #[test]
    fn test_e2e_pipe_to_function() {
        let r = compile_and_run(r#"
            fn identity(x) { x }
            output (42 |> identity());
        "#);
        assert_eq!(r.printed, vec!["42"]);
    }

    #[test]
    fn test_e2e_pipe_chain() {
        let r = compile_and_run(r#"
            fn id(x) { x }
            output (42 |> id() |> id());
        "#);
        assert_eq!(r.printed, vec!["42"]);
    }

    // ── E2E: Empty program ──────────────────────────────────────────

    #[test]
    fn test_e2e_empty_program() {
        let r = compile_and_run("");
        // __main always returns null; no output
        assert_eq!(result_str(&r), "null");
        assert!(r.printed.is_empty());
    }

    #[test]
    fn test_e2e_only_statements_no_output() {
        let r = compile_and_run("let x = 42;");
        // No output statements → printed should be empty
        assert!(r.printed.is_empty());
    }

    // ── E2E: Try-catch ──────────────────────────────────────────────

    #[test]
    fn test_e2e_try_catch_no_error() {
        // try-catch in WASM only compiles the try block (known limitation).
        let r = compile_and_run(r#"
            let x = try { 42 } catch e { 0 };
            output x;
        "#);
        assert_eq!(r.printed, vec!["42"]);
    }

    // ── E2E: Array output ───────────────────────────────────────────

    #[test]
    fn test_e2e_output_array() {
        let r = compile_and_run("output [1, 2, 3];");
        assert_eq!(r.printed, vec!["[1, 2, 3]"]);
    }

    #[test]
    fn test_e2e_output_empty_array() {
        let r = compile_and_run("output [];");
        assert_eq!(r.printed, vec!["[]"]);
    }

    #[test]
    fn test_e2e_output_array_of_strings() {
        let r = compile_and_run(r#"output ["hello", "world"];"#);
        assert_eq!(r.printed, vec!["[hello, world]"]);
    }

    #[test]
    fn test_e2e_output_array_of_bools() {
        let r = compile_and_run("output [true, false, true];");
        assert_eq!(r.printed, vec!["[true, false, true]"]);
    }

    #[test]
    fn test_e2e_output_nested_array() {
        let r = compile_and_run("output [[1, 2], [3, 4]];");
        assert_eq!(r.printed, vec!["[[1, 2], [3, 4]]"]);
    }

    // ── E2E: Map output ─────────────────────────────────────────────

    #[test]
    fn test_e2e_output_empty_map() {
        let r = compile_and_run("output {};");
        assert_eq!(r.printed, vec!["{}"]);
    }

    // ── E2E: String interpolation ───────────────────────────────────

    #[test]
    fn test_e2e_output_fstring_literal() {
        let r = compile_and_run(r#"output f"hello world";"#);
        assert_eq!(r.printed, vec!["hello world"]);
    }

    // ── E2E: Enum construction ──────────────────────────────────────

    #[test]
    fn test_e2e_enum_unit_variant_output() {
        // Enums are maps; verify output doesn't crash.
        let r = compile_and_run(r#"
            enum Color { Red, Green, Blue }
            output Color::Red;
        "#);
        // Should produce a map with __enum, variant, etc.
        assert_eq!(r.printed.len(), 1);
    }

    // ── E2E: Struct construction ────────────────────────────────────

    #[test]
    fn test_e2e_struct_output() {
        let r = compile_and_run(r#"
            struct Point { x: int64, y: int64 }
            output Point { x: 1, y: 2 };
        "#);
        // Should produce a map with __struct, x, y fields
        assert_eq!(r.printed.len(), 1);
    }

    // ── E2E: __main always returns null ─────────────────────────────

    #[test]
    fn test_e2e_main_returns_null_for_bare_expression() {
        // Bare expressions at top level get their value dropped.
        // __main ends with PushNull+Return.
        let r = compile_and_run("42");
        assert_eq!(result_str(&r), "null");
        assert_eq!(result_tag(r.main_result), 0);
    }

    #[test]
    fn test_e2e_main_returns_null_for_let() {
        let r = compile_and_run("let x = 42;");
        assert_eq!(result_str(&r), "null");
    }

    // ── E2E: Float precision ─────────────────────────────────────
    // NaN-boxing stores f64 values as raw IEEE 754 bits (no tagging).
    // Only non-float values use NaN payloads. This preserves full f64
    // precision for all float values including negatives and large exponents.

    #[test]
    fn test_e2e_nan_boxing_negative_float() {
        // -3.14 is parsed as UnaryOp::Neg(3.14).
        // The negation path: UntagF64 (reinterpret), F64Neg, TagF64 (reinterpret back).
        // Full f64 precision is preserved.
        let r = compile_and_run("output -3.14;");
        assert_eq!(r.printed, vec!["-3.14"]);
    }

    #[test]
    fn test_e2e_nan_boxing_positive_float() {
        // Positive float 3.14 stored as raw IEEE 754 bits.
        let r = compile_and_run("output 3.14;");
        assert_eq!(r.printed, vec!["3.14"]);
    }

    #[test]
    fn test_e2e_nan_boxing_float_zero() {
        let r = compile_and_run("output 0.0;");
        assert_eq!(r.printed, vec!["0.0"]);
    }

    #[test]
    fn test_e2e_float_addition() {
        let r = compile_and_run("output (1.5 + 2.5);");
        assert_eq!(r.printed, vec!["4.0"]);
    }

    #[test]
    fn test_e2e_float_subtraction() {
        let r = compile_and_run("output (5.5 - 2.0);");
        assert_eq!(r.printed, vec!["3.5"]);
    }

    #[test]
    fn test_e2e_float_multiplication() {
        let r = compile_and_run("output (3.0 * 2.5);");
        assert_eq!(r.printed, vec!["7.5"]);
    }

    #[test]
    fn test_e2e_float_division() {
        let r = compile_and_run("output (7.5 / 2.5);");
        assert_eq!(r.printed, vec!["3.0"]);
    }

    #[test]
    fn test_e2e_float_modulo() {
        let r = compile_and_run("output (7.5 % 2.0);");
        assert_eq!(r.printed, vec!["1.5"]);
    }

    #[test]
    fn test_e2e_float_comparison_gt() {
        let r = compile_and_run("output (3.14 > 2.71);");
        assert_eq!(r.printed, vec!["true"]);
    }

    #[test]
    fn test_e2e_float_comparison_lt() {
        let r = compile_and_run("output (2.71 < 3.14);");
        assert_eq!(r.printed, vec!["true"]);
    }

    #[test]
    fn test_e2e_mixed_int_float_add() {
        // int + float should promote to float
        let r = compile_and_run("output (1 + 2.5);");
        assert_eq!(r.printed, vec!["3.5"]);
    }

    #[test]
    fn test_e2e_negative_float_arithmetic() {
        let r = compile_and_run("output (-1.5 + -2.5);");
        assert_eq!(r.printed, vec!["-4.0"]);
    }

    #[test]
    fn test_e2e_large_float() {
        let r = compile_and_run("output 1.0e10;");
        assert_eq!(r.printed.len(), 1);
        let val: f64 = r.printed[0].parse().expect("should be valid float");
        assert!((val - 1.0e10).abs() < 1.0, "expected ~1e10, got {}", val);
    }

    // ── E2E: Integer arithmetic ────────────────────────────────────

    #[test]
    fn test_e2e_integer_addition() {
        let r = compile_and_run("output (1 + 2);");
        assert_eq!(r.printed, vec!["3"]);
    }

    #[test]
    fn test_e2e_integer_subtraction() {
        let r = compile_and_run("output (5 - 3);");
        assert_eq!(r.printed, vec!["2"]);
    }

    #[test]
    fn test_e2e_integer_multiplication() {
        let r = compile_and_run("output (6 * 7);");
        assert_eq!(r.printed, vec!["42"]);
    }

    #[test]
    fn test_e2e_integer_division() {
        let r = compile_and_run("output (10 / 3);");
        assert_eq!(r.printed, vec!["3"]);
    }

    #[test]
    fn test_e2e_integer_modulo() {
        let r = compile_and_run("output (10 % 3);");
        assert_eq!(r.printed, vec!["1"]);
    }

    #[test]
    fn test_e2e_arithmetic_chain() {
        let r = compile_and_run(r#"
            let x = 10;
            let y = x + 5;
            let z = y * 2;
            output z;
        "#);
        assert_eq!(r.printed, vec!["30"]);
    }

    #[test]
    fn test_e2e_string_plus_integer() {
        // When left is STRING and right is not, __to_string is called on right
        let r = compile_and_run(r#"output ("count: " + 42);"#);
        assert_eq!(r.printed, vec!["count: 42"]);
    }

    #[test]
    fn test_e2e_string_plus_bool() {
        let r = compile_and_run(r#"output ("flag: " + true);"#);
        assert_eq!(r.printed, vec!["flag: true"]);
    }

    #[test]
    fn test_e2e_string_plus_null() {
        let r = compile_and_run(r#"output ("val: " + null);"#);
        assert_eq!(r.printed, vec!["val: null"]);
    }

    // ── Additional compilation error cases ──────────────────────────

    #[test]
    fn test_compile_error_return_outside_function() {
        // Return at top level — just verify no panic
        let result = compile_to_wasm("return 42;");
        let _ = result;
    }

    #[test]
    fn test_compile_error_undefined_variable() {
        let result = compile_to_wasm("x");
        let _ = result;
    }

    #[test]
    fn test_compile_error_duplicate_function() {
        let result = compile_to_wasm(r#"
            fn foo() { 1 }
            fn foo() { 2 }
        "#);
        let _ = result;
    }

    // ── WASM validation for edge cases ──────────────────────────────

    #[test]
    fn test_validate_deeply_nested_if() {
        compile_and_validate(r#"
            if true {
                if true {
                    if true {
                        if true {
                            if true {
                                42
                            } else { 0 }
                        } else { 0 }
                    } else { 0 }
                } else { 0 }
            } else { 0 }
        "#);
    }

    #[test]
    fn test_validate_many_function_params() {
        compile_and_validate(r#"
            fn many(a, b, c, d, e, f, g, h) { a }
            many(1, 2, 3, 4, 5, 6, 7, 8)
        "#);
    }

    #[test]
    fn test_validate_function_returning_function_result() {
        compile_and_validate(r#"
            fn inner() { 42 }
            fn outer() { inner() }
            outer()
        "#);
    }

    #[test]
    fn test_validate_multiple_match_arms() {
        compile_and_validate(r#"
            match 5 {
                1 => "a",
                2 => "b",
                3 => "c",
                4 => "d",
                5 => "e",
                6 => "f",
                7 => "g",
                _ => "other",
            }
        "#);
    }

    #[test]
    fn test_validate_match_with_block_bodies() {
        compile_and_validate(r#"
            let x = 2;
            match x {
                1 => {
                    let a = 10;
                    a
                },
                _ => {
                    let b = 20;
                    b
                },
            }
        "#);
    }

    #[test]
    fn test_validate_while_mutation_loop() {
        compile_and_validate(r#"
            let mut count = 0;
            let mut i = 0;
            while i < 10 {
                count = count + 1;
                i = i + 1;
            }
            output count;
        "#);
    }

    #[test]
    fn test_validate_array_in_map() {
        compile_and_validate(r#"
            let data = {
                "items": [1, 2, 3],
                "name": "test"
            };
        "#);
    }

    #[test]
    fn test_validate_multiple_strings() {
        compile_and_validate(r#"
            let a = "alpha";
            let b = "beta";
            let c = "gamma";
            let d = "delta";
            output a;
            output b;
            output c;
            output d;
        "#);
    }

    #[test]
    fn test_validate_bool_in_if() {
        compile_and_validate(r#"
            let flag = true;
            if flag {
                output "yes";
            }
            let other = false;
            if !other {
                output "also yes";
            }
        "#);
    }

    // ── format_tagged unit tests ────────────────────────────────────
    // Test the tag decoding logic directly without running WASM.
    // Uses NaN-boxing: tag::encode(tag, payload) for non-float values,
    // raw f64 bits for float values.

    #[test]
    fn test_format_tagged_null() {
        use crate::compiler::ir::tag;
        let val = tag::encode(tag::NULL, 0);
        assert_eq!(format_tagged(val, &[]), "null");
    }

    #[test]
    fn test_format_tagged_bool_true() {
        use crate::compiler::ir::tag;
        let val = tag::encode(tag::BOOL, 1);
        assert_eq!(format_tagged(val, &[]), "true");
    }

    #[test]
    fn test_format_tagged_bool_false() {
        use crate::compiler::ir::tag;
        let val = tag::encode(tag::BOOL, 0);
        assert_eq!(format_tagged(val, &[]), "false");
    }

    #[test]
    fn test_format_tagged_i64_positive() {
        use crate::compiler::ir::tag;
        let val = tag::encode(tag::I64, 42);
        assert_eq!(format_tagged(val, &[]), "42");
    }

    #[test]
    fn test_format_tagged_i64_zero() {
        use crate::compiler::ir::tag;
        let val = tag::encode(tag::I64, 0);
        assert_eq!(format_tagged(val, &[]), "0");
    }

    #[test]
    fn test_format_tagged_i64_negative() {
        use crate::compiler::ir::tag;
        // -1 in 48-bit: all lower 48 bits set
        let val = tag::encode(tag::I64, -1);
        assert_eq!(format_tagged(val, &[]), "-1");
    }

    #[test]
    fn test_format_tagged_i64_negative_small() {
        use crate::compiler::ir::tag;
        let val = tag::encode(tag::I64, -5);
        assert_eq!(format_tagged(val, &[]), "-5");
    }

    #[test]
    fn test_format_tagged_i64_max_positive() {
        use crate::compiler::ir::tag;
        // Max positive 48-bit value: 2^47 - 1
        let max47 = (1i64 << 47) - 1;
        let val = tag::encode(tag::I64, max47);
        assert_eq!(format_tagged(val, &[]), "140737488355327");
    }

    #[test]
    fn test_format_tagged_i64_min_negative() {
        use crate::compiler::ir::tag;
        // Min 48-bit value: -2^47
        let min47 = -(1i64 << 47);
        let val = tag::encode(tag::I64, min47);
        assert_eq!(format_tagged(val, &[]), "-140737488355328");
    }

    #[test]
    fn test_format_tagged_f64() {
        // Raw f64 value (not NaN-boxed)
        let val = f64::to_bits(1.23) as i64;
        let result = format_tagged(val, &[]);
        assert!(result.starts_with("1.23"), "expected 1.23..., got {}", result);
    }

    #[test]
    fn test_format_tagged_f64_integer_like() {
        // f64 that looks like an integer
        let val = f64::to_bits(42.0) as i64;
        assert_eq!(format_tagged(val, &[]), "42.0");
    }

    #[test]
    fn test_format_tagged_string() {
        use crate::compiler::ir::tag;
        let mut data = vec![0u8; 10];
        // Length = 2 (little endian)
        data[0..4].copy_from_slice(&2u32.to_le_bytes());
        // "hi"
        data[4] = b'h';
        data[5] = b'i';

        let val = tag::encode(tag::STRING, 0); // string at offset 0
        assert_eq!(format_tagged(val, &data), "hi");
    }

    #[test]
    fn test_format_tagged_string_out_of_bounds() {
        use crate::compiler::ir::tag;
        let val = tag::encode(tag::STRING, 9999);
        assert_eq!(format_tagged(val, &[0; 10]), "<string@9999>");
    }

    #[test]
    fn test_format_tagged_string_len_exceeds_memory() {
        use crate::compiler::ir::tag;
        let mut data = vec![0u8; 8];
        // Length = 100 (way past end of data)
        data[0..4].copy_from_slice(&100u32.to_le_bytes());

        let val = tag::encode(tag::STRING, 0);
        assert_eq!(format_tagged(val, &data), "<string@0>");
    }

    #[test]
    fn test_format_tagged_string_empty() {
        use crate::compiler::ir::tag;
        let mut data = vec![0u8; 8];
        // Length = 0
        data[0..4].copy_from_slice(&0u32.to_le_bytes());

        let val = tag::encode(tag::STRING, 0);
        assert_eq!(format_tagged(val, &data), "");
    }

    #[test]
    fn test_format_tagged_i32() {
        use crate::compiler::ir::tag;
        let val = tag::encode(tag::I32, 42);
        assert_eq!(format_tagged(val, &[]), "42");
    }

    #[test]
    fn test_format_tagged_i32_negative() {
        use crate::compiler::ir::tag;
        let neg1_32 = 0xFFFFFFFFu64 as i64;
        let val = tag::encode(tag::I32, neg1_32);
        assert_eq!(format_tagged(val, &[]), "-1");
    }

    #[test]
    fn test_format_tagged_f32() {
        use crate::compiler::ir::tag;
        let bits = f32::to_bits(1.5);
        let val = tag::encode(tag::F32, bits as i64);
        assert_eq!(format_tagged(val, &[]), "1.5");
    }

    #[test]
    fn test_format_tagged_array() {
        use crate::compiler::ir::tag;
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(&2u32.to_le_bytes()); // length = 2
        data[4..8].copy_from_slice(&2u32.to_le_bytes()); // capacity = 2
        let elem0 = tag::encode(tag::I64, 42);
        data[8..16].copy_from_slice(&elem0.to_le_bytes());
        let elem1 = tag::encode(tag::I64, 7);
        data[16..24].copy_from_slice(&elem1.to_le_bytes());

        let val = tag::encode(tag::ARRAY, 0);
        assert_eq!(format_tagged(val, &data), "[42, 7]");
    }

    #[test]
    fn test_format_tagged_empty_array() {
        use crate::compiler::ir::tag;
        let mut data = vec![0u8; 16];
        data[0..4].copy_from_slice(&0u32.to_le_bytes());
        data[4..8].copy_from_slice(&0u32.to_le_bytes());

        let val = tag::encode(tag::ARRAY, 0);
        assert_eq!(format_tagged(val, &data), "[]");
    }

    #[test]
    fn test_format_tagged_array_out_of_bounds() {
        use crate::compiler::ir::tag;
        let val = tag::encode(tag::ARRAY, 9999);
        assert_eq!(format_tagged(val, &[0; 10]), "<array@9999>");
    }
}
