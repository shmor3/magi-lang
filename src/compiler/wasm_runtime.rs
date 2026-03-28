//! Own WASM interpreter — replaces the `wasmtime` crate.
//!
//! A stack-machine WASM interpreter that supports the instruction subset
//! emitted by our compiler (~80 opcodes). Provides Engine/Store/Linker/Instance
//! API compatible with how wasmtime is used in the codebase.

use std::collections::HashMap;

// ── Values ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum Val {
    I32(i32),
    I64(i64),
    F64(f64),
}

impl Val {
    pub fn i32(&self) -> Option<i32> { match self { Val::I32(v) => Some(*v), _ => None } }
    pub fn i64(&self) -> Option<i64> { match self { Val::I64(v) => Some(*v), _ => None } }
    pub fn unwrap_i64(&self) -> i64 {
        match self {
            Val::I64(v) => *v,
            Val::I32(v) => *v as i64,
            Val::F64(v) => v.to_bits() as i64, // preserve bit pattern for NaN-boxing
        }
    }
}

// ── Decoded module ──────────────────────────────────────────────────────

#[derive(Clone)]
struct FuncType {
    params: Vec<u8>,  // 0x7F=i32, 0x7E=i64, 0x7D=f32, 0x7C=f64
    results: Vec<u8>,
}

#[derive(Clone)]
struct WasmImport {
    module: String,
    name: String,
    kind: ImportKind,
}

#[derive(Clone)]
enum ImportKind {
    Func(u32), // type index
}

#[derive(Clone)]
struct WasmExport {
    name: String,
    kind: u8,  // 0=func, 1=table, 2=mem, 3=global
    index: u32,
}

#[derive(Clone)]
struct WasmGlobal {
    val_type: u8,
    init_expr: Vec<u8>,
}

struct DecodedModule {
    types: Vec<FuncType>,
    imports: Vec<WasmImport>,
    func_type_indices: Vec<u32>,
    tables: Vec<(u32, Option<u32>)>, // min, max
    memories: Vec<(u32, Option<u32>)>,
    globals: Vec<WasmGlobal>,
    exports: Vec<WasmExport>,
    elements: Vec<(u32, Vec<u8>, Vec<u32>)>, // table, offset_expr, func_indices
    code: Vec<Vec<u8>>,  // raw function bodies
    data_segments: Vec<(u32, Vec<u8>, Vec<u8>)>, // mem, offset_expr, data
}

fn decode_module(bytes: &[u8]) -> Result<DecodedModule, String> {
    if bytes.len() < 8 || &bytes[0..4] != b"\0asm" {
        return Err("invalid WASM magic".into());
    }
    let mut mod_ = DecodedModule {
        types: Vec::new(), imports: Vec::new(), func_type_indices: Vec::new(),
        tables: Vec::new(), memories: Vec::new(), globals: Vec::new(),
        exports: Vec::new(), elements: Vec::new(), code: Vec::new(),
        data_segments: Vec::new(),
    };
    let mut pos = 8;
    while pos < bytes.len() {
        let section_id = bytes[pos]; pos += 1;
        let (sec_len, consumed) = read_leb128_u32(&bytes[pos..])?;
        pos += consumed;
        let sec_end = pos + sec_len as usize;
        if sec_end > bytes.len() {
            return Err(format!("truncated section {} (need {} bytes, have {})", section_id, sec_len, bytes.len() - pos));
        }
        let sec_data = &bytes[pos..sec_end];
        match section_id {
            1 => mod_.types = decode_type_section(sec_data)?,
            2 => mod_.imports = decode_import_section(sec_data)?,
            3 => mod_.func_type_indices = decode_function_section(sec_data)?,
            4 => mod_.tables = decode_table_section(sec_data)?,
            5 => mod_.memories = decode_memory_section(sec_data)?,
            6 => mod_.globals = decode_global_section(sec_data)?,
            7 => mod_.exports = decode_export_section(sec_data)?,
            9 => mod_.elements = decode_element_section(sec_data)?,
            10 => mod_.code = decode_code_section(sec_data)?,
            11 => mod_.data_segments = decode_data_section(sec_data)?,
            _ => {} // skip custom sections
        }
        pos = sec_end;
    }
    Ok(mod_)
}

fn read_leb128_u32(data: &[u8]) -> Result<(u32, usize), String> {
    let mut result: u32 = 0; let mut shift = 0;
    for (i, &b) in data.iter().enumerate() {
        result |= ((b & 0x7f) as u32) << shift;
        if b & 0x80 == 0 { return Ok((result, i + 1)); }
        shift += 7;
        if shift >= 35 { return Err("LEB128 overflow".into()); }
    }
    Err("unterminated LEB128".into())
}

fn read_leb128_i32(data: &[u8]) -> Result<(i32, usize), String> {
    let mut result: i32 = 0; let mut shift = 0;
    for (i, &b) in data.iter().enumerate() {
        result |= ((b & 0x7f) as i32) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            if shift < 32 && b & 0x40 != 0 { result |= !0 << shift; }
            return Ok((result, i + 1));
        }
        if shift >= 35 { return Err("LEB128 overflow".into()); }
    }
    Err("unterminated LEB128".into())
}

fn read_leb128_i64(data: &[u8]) -> Result<(i64, usize), String> {
    let mut result: i64 = 0; let mut shift = 0;
    for (i, &b) in data.iter().enumerate() {
        result |= ((b & 0x7f) as i64) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            if shift < 64 && b & 0x40 != 0 { result |= !0i64 << shift; }
            return Ok((result, i + 1));
        }
        if shift >= 70 { return Err("LEB128 overflow".into()); }
    }
    Err("unterminated LEB128".into())
}

fn read_name(data: &[u8], pos: &mut usize) -> Result<String, String> {
    let (len, c) = read_leb128_u32(&data[*pos..])?; *pos += c;
    let end = *pos + len as usize;
    if end > data.len() { return Err("truncated name".into()); }
    let s = std::str::from_utf8(&data[*pos..end]).map_err(|_| "invalid UTF-8 name")?;
    *pos = end;
    Ok(s.to_string())
}

fn decode_type_section(data: &[u8]) -> Result<Vec<FuncType>, String> {
    let mut pos = 0;
    let (count, c) = read_leb128_u32(&data[pos..])?; pos += c;
    let mut types = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if data[pos] != 0x60 { return Err("expected functype 0x60".into()); }
        pos += 1;
        let (pc, c) = read_leb128_u32(&data[pos..])?; pos += c;
        let params: Vec<u8> = data[pos..pos + pc as usize].to_vec(); pos += pc as usize;
        let (rc, c) = read_leb128_u32(&data[pos..])?; pos += c;
        let results: Vec<u8> = data[pos..pos + rc as usize].to_vec(); pos += rc as usize;
        types.push(FuncType { params, results });
    }
    Ok(types)
}

fn decode_import_section(data: &[u8]) -> Result<Vec<WasmImport>, String> {
    let mut pos = 0;
    let (count, c) = read_leb128_u32(&data[pos..])?; pos += c;
    let mut imports = Vec::new();
    for _ in 0..count {
        let module = read_name(data, &mut pos)?;
        let name = read_name(data, &mut pos)?;
        let kind_byte = data[pos]; pos += 1;
        let kind = match kind_byte {
            0x00 => { let (idx, c) = read_leb128_u32(&data[pos..])?; pos += c; ImportKind::Func(idx) }
            _ => return Err(format!("unsupported import kind: {}", kind_byte)),
        };
        imports.push(WasmImport { module, name, kind });
    }
    Ok(imports)
}

fn decode_function_section(data: &[u8]) -> Result<Vec<u32>, String> {
    let mut pos = 0;
    let (count, c) = read_leb128_u32(&data[pos..])?; pos += c;
    let mut indices = Vec::new();
    for _ in 0..count {
        let (idx, c) = read_leb128_u32(&data[pos..])?; pos += c;
        indices.push(idx);
    }
    Ok(indices)
}

fn decode_table_section(data: &[u8]) -> Result<Vec<(u32, Option<u32>)>, String> {
    let mut pos = 0;
    let (count, c) = read_leb128_u32(&data[pos..])?; pos += c;
    let mut tables = Vec::new();
    for _ in 0..count {
        pos += 1; // elem type
        let (min, max, consumed) = decode_limits(&data[pos..])?; pos += consumed;
        tables.push((min, max));
    }
    Ok(tables)
}

fn decode_memory_section(data: &[u8]) -> Result<Vec<(u32, Option<u32>)>, String> {
    let mut pos = 0;
    let (count, c) = read_leb128_u32(&data[pos..])?; pos += c;
    let mut mems = Vec::new();
    for _ in 0..count {
        let (min, max, consumed) = decode_limits(&data[pos..])?; pos += consumed;
        mems.push((min, max));
    }
    Ok(mems)
}

fn decode_limits(data: &[u8]) -> Result<(u32, Option<u32>, usize), String> {
    let flags = data[0]; let mut pos = 1;
    let (min, c) = read_leb128_u32(&data[pos..])?; pos += c;
    let max = if flags & 1 != 0 {
        let (m, c) = read_leb128_u32(&data[pos..])?; pos += c; Some(m)
    } else { None };
    Ok((min, max, pos))
}

fn decode_global_section(data: &[u8]) -> Result<Vec<WasmGlobal>, String> {
    let mut pos = 0;
    let (count, c) = read_leb128_u32(&data[pos..])?; pos += c;
    let mut globals = Vec::new();
    for _ in 0..count {
        let vt = data[pos]; pos += 1;
        pos += 1; // skip mutability byte
        let expr_start = pos;
        skip_const_expr(data, &mut pos);
        globals.push(WasmGlobal { val_type: vt, init_expr: data[expr_start..pos].to_vec() });
    }
    Ok(globals)
}

fn decode_export_section(data: &[u8]) -> Result<Vec<WasmExport>, String> {
    let mut pos = 0;
    let (count, c) = read_leb128_u32(&data[pos..])?; pos += c;
    let mut exports = Vec::new();
    for _ in 0..count {
        let name = read_name(data, &mut pos)?;
        let kind = data[pos]; pos += 1;
        let (index, c) = read_leb128_u32(&data[pos..])?; pos += c;
        exports.push(WasmExport { name, kind, index });
    }
    Ok(exports)
}

fn decode_element_section(data: &[u8]) -> Result<Vec<(u32, Vec<u8>, Vec<u32>)>, String> {
    let mut pos = 0;
    let (count, c) = read_leb128_u32(&data[pos..])?; pos += c;
    let mut elems = Vec::new();
    for _ in 0..count {
        let flags = data[pos]; pos += 1;
        let _ = flags; // simplified: assume active, table 0
        let expr_start = pos;
        skip_const_expr(data, &mut pos);
        let offset_expr = data[expr_start..pos].to_vec();
        let (fc, c) = read_leb128_u32(&data[pos..])?; pos += c;
        let mut indices = Vec::new();
        for _ in 0..fc {
            let (idx, c) = read_leb128_u32(&data[pos..])?; pos += c;
            indices.push(idx);
        }
        elems.push((0, offset_expr, indices));
    }
    Ok(elems)
}

fn decode_code_section(data: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let mut pos = 0;
    let (count, c) = read_leb128_u32(&data[pos..])?; pos += c;
    let mut bodies = Vec::new();
    for _ in 0..count {
        let (size, c) = read_leb128_u32(&data[pos..])?; pos += c;
        bodies.push(data[pos..pos + size as usize].to_vec());
        pos += size as usize;
    }
    Ok(bodies)
}

fn decode_data_section(data: &[u8]) -> Result<Vec<(u32, Vec<u8>, Vec<u8>)>, String> {
    let mut pos = 0;
    let (count, c) = read_leb128_u32(&data[pos..])?; pos += c;
    let mut segs = Vec::new();
    for _ in 0..count {
        let flags = data[pos]; pos += 1;
        let _ = flags; // assume active, memory 0
        let expr_start = pos;
        skip_const_expr(data, &mut pos);
        let offset_expr = data[expr_start..pos].to_vec();
        let (size, c) = read_leb128_u32(&data[pos..])?; pos += c;
        let seg_data = data[pos..pos + size as usize].to_vec();
        pos += size as usize;
        segs.push((0, offset_expr, seg_data));
    }
    Ok(segs)
}

fn eval_const_expr(expr: &[u8]) -> i64 {
    if expr.is_empty() { return 0; }
    match expr[0] {
        0x41 => { // i32.const
            let (v, _) = read_leb128_i32(&expr[1..]).unwrap_or((0, 0));
            v as i64
        }
        0x42 => { // i64.const
            let (v, _) = read_leb128_i64(&expr[1..]).unwrap_or((0, 0));
            v
        }
        _ => 0,
    }
}

/// Skip past a const expression properly, accounting for LEB128 operands.
fn skip_const_expr(data: &[u8], pos: &mut usize) {
    if *pos >= data.len() { return; }
    match data[*pos] {
        0x41 => { *pos += 1; let (_, c) = read_leb128_i32(&data[*pos..]).unwrap_or((0, 1)); *pos += c; }
        0x42 => { *pos += 1; let (_, c) = read_leb128_i64(&data[*pos..]).unwrap_or((0, 1)); *pos += c; }
        0x43 => { *pos += 5; } // f32.const
        0x44 => { *pos += 9; } // f64.const
        0x23 => { *pos += 1; let (_, c) = read_leb128_u32(&data[*pos..]).unwrap_or((0, 1)); *pos += c; } // global.get
        _ => { // unknown — scan for 0x0B as fallback
            while *pos < data.len() && data[*pos] != 0x0B { *pos += 1; }
        }
    }
    if *pos < data.len() && data[*pos] == 0x0B { *pos += 1; } // skip end
}

// ── Host functions ──────────────────────────────────────────────────────

type HostFn = Box<dyn Fn(&mut Instance, &[Val]) -> Result<Vec<Val>, String> + Send + Sync>;
type HostFnMap = HashMap<(String, String), HostFn>;

// ── Engine / Module / Store / Linker / Instance ─────────────────────────

pub struct Engine;
impl Engine { pub fn default() -> Self { Engine } }

pub struct Module { decoded: DecodedModule }
impl Module {
    pub fn new(_engine: &Engine, bytes: &[u8]) -> Result<Module, String> {
        Ok(Module { decoded: decode_module(bytes)? })
    }
}

pub struct Store;
impl Store {
    pub fn new(_engine: &Engine, _data: ()) -> Self { Store }
}

pub struct Linker {
    host_fns: std::sync::Arc<HostFnMap>,
}

impl Linker {
    pub fn new(_engine: &Engine) -> Self {
        Linker { host_fns: std::sync::Arc::new(HashMap::new()) }
    }

    fn host_fns_mut(&mut self) -> &mut HostFnMap {
        std::sync::Arc::get_mut(&mut self.host_fns)
            .expect("cannot modify host_fns after instantiation")
    }

    /// Register a host function: (i64) -> ()
    pub fn func_wrap_1_0(
        &mut self, module: &str, name: &str,
        f: impl Fn(&mut Instance, i64) + Send + Sync + 'static,
    ) -> Result<(), String> {
        let f = Box::new(move |inst: &mut Instance, args: &[Val]| {
            f(inst, args.first().map(|v| v.unwrap_i64()).unwrap_or(0));
            Ok(Vec::new())
        });
        self.host_fns_mut().insert((module.to_string(), name.to_string()), f);
        Ok(())
    }

    /// Register a host function: (i32, i32) -> i64
    pub fn func_wrap_2_1(
        &mut self, module: &str, name: &str,
        f: impl Fn(&mut Instance, i32, i32) -> i64 + Send + Sync + 'static,
    ) -> Result<(), String> {
        let f = Box::new(move |inst: &mut Instance, args: &[Val]| {
            let a = args.first().and_then(|v| v.i32()).unwrap_or(0);
            let b = args.get(1).and_then(|v| v.i32()).unwrap_or(0);
            Ok(vec![Val::I64(f(inst, a, b))])
        });
        self.host_fns_mut().insert((module.to_string(), name.to_string()), f);
        Ok(())
    }

    /// Register a host function: (i64) -> i64
    pub fn func_wrap_1_1(
        &mut self, module: &str, name: &str,
        f: impl Fn(&mut Instance, i64) -> i64 + Send + Sync + 'static,
    ) -> Result<(), String> {
        let f = Box::new(move |inst: &mut Instance, args: &[Val]| {
            let a = args.first().map(|v| v.unwrap_i64()).unwrap_or(0);
            Ok(vec![Val::I64(f(inst, a))])
        });
        self.host_fns_mut().insert((module.to_string(), name.to_string()), f);
        Ok(())
    }

    pub fn instantiate(&self, _store: &mut Store, module: &Module) -> Result<Instance, String> {
        let dec = &module.decoded;
        let num_imports = dec.imports.len() as u32;

        let mem_pages = dec.memories.first().map(|(min, _)| *min).unwrap_or(1);
        let mut memory = vec![0u8; mem_pages as usize * 65536];

        let mut globals: Vec<Val> = Vec::new();
        for g in &dec.globals {
            let val = eval_const_expr(&g.init_expr);
            globals.push(match g.val_type {
                0x7F => Val::I32(val as i32),
                0x7E => Val::I64(val),
                0x7C => Val::F64(f64::from_bits(val as u64)),
                _ => Val::I32(val as i32),
            });
        }

        // Data segments → memory
        for (_, offset_expr, data) in &dec.data_segments {
            let offset = eval_const_expr(offset_expr) as usize;
            if offset + data.len() <= memory.len() {
                memory[offset..offset + data.len()].copy_from_slice(data);
            }
        }

        let table_size = dec.tables.first().map(|(min, _)| *min as usize).unwrap_or(0);
        let mut func_table = vec![u32::MAX; table_size];
        for (_, offset_expr, indices) in &dec.elements {
            let offset = eval_const_expr(offset_expr) as usize;
            for (i, &idx) in indices.iter().enumerate() {
                if offset + i < func_table.len() {
                    func_table[offset + i] = idx;
                }
            }
        }

        let mut import_keys: Vec<(String, String)> = Vec::new();
        for imp in &dec.imports {
            import_keys.push((imp.module.clone(), imp.name.clone()));
        }

        let mut export_map = HashMap::new();
        for exp in &dec.exports {
            export_map.insert(exp.name.clone(), (exp.kind, exp.index));
        }

        // Collect host function refs
        let mut host_fn_keys: Vec<Option<(String, String)>> = Vec::new();
        for key in &import_keys {
            if self.host_fns.contains_key(key) {
                host_fn_keys.push(Some(key.clone()));
            } else {
                host_fn_keys.push(None);
            }
        }

        // Build import type indices
        let import_type_indices: Vec<u32> = dec.imports.iter().map(|imp| {
            match &imp.kind { ImportKind::Func(idx) => *idx }
        }).collect();

        Ok(Instance {
            types: dec.types.clone(),
            code: dec.code.clone(),
            func_type_indices: dec.func_type_indices.clone(),
            import_type_indices,
            num_imports,
            memory,
            globals,
            func_table,
            export_map,
            host_fn_keys,
            host_fns: std::sync::Arc::clone(&self.host_fns),
            call_depth: 0,
        })
    }
}

pub struct Instance {
    types: Vec<FuncType>,
    code: Vec<Vec<u8>>,
    func_type_indices: Vec<u32>,
    import_type_indices: Vec<u32>,
    num_imports: u32,
    pub memory: Vec<u8>,
    pub globals: Vec<Val>,
    func_table: Vec<u32>,
    export_map: HashMap<String, (u8, u32)>,
    host_fn_keys: Vec<Option<(String, String)>>,
    host_fns: std::sync::Arc<HostFnMap>,
    call_depth: u32,
}

impl Instance {
    pub fn get_memory_data(&self) -> &[u8] { &self.memory }
    pub fn get_memory_data_mut(&mut self) -> &mut [u8] { &mut self.memory }

    pub fn get_global(&self, name: &str) -> Option<Val> {
        self.export_map.get(name).and_then(|(kind, idx)| {
            if *kind == 3 { self.globals.get(*idx as usize).copied() } else { None }
        })
    }

    pub fn set_global(&mut self, name: &str, val: Val) -> Result<(), String> {
        if let Some((kind, idx)) = self.export_map.get(name) {
            if *kind == 3 {
                if let Some(g) = self.globals.get_mut(*idx as usize) { *g = val; return Ok(()); }
            }
        }
        Err(format!("global '{}' not found", name))
    }

    pub fn call(&mut self, name: &str, _store: &mut Store) -> Result<i64, String> {
        let (kind, idx) = self.export_map.get(name)
            .ok_or_else(|| format!("export '{}' not found", name))?;
        if *kind != 0 { return Err("not a function export".into()); }
        let func_idx = *idx;
        self.call_func(func_idx)
    }

    fn call_func(&mut self, func_idx: u32) -> Result<i64, String> {
        if func_idx < self.num_imports {
            return Err(format!("cannot directly call import {}", func_idx));
        }
        if self.call_depth > 1000 {
            return Err("WASM call stack overflow (depth > 1000)".into());
        }
        self.call_depth += 1;
        let code_idx = (func_idx - self.num_imports) as usize;
        if code_idx >= self.code.len() {
            self.call_depth -= 1;
            return Err(format!("function index {} out of range", func_idx));
        }
        let type_idx = self.func_type_indices[code_idx] as usize;
        let func_type = self.types[type_idx].clone();
        let body = self.code[code_idx].clone();
        let result = execute_function(self, &body, &func_type, &[]);
        self.call_depth -= 1;
        result
    }

    fn call_func_with_args(&mut self, func_idx: u32, args: &[Val]) -> Result<i64, String> {
        if self.call_depth > 1000 {
            return Err("WASM call stack overflow (depth > 1000)".into());
        }
        self.call_depth += 1;
        let result = self.call_func_with_args_inner(func_idx, args);
        self.call_depth -= 1;
        result
    }

    fn call_func_with_args_inner(&mut self, func_idx: u32, args: &[Val]) -> Result<i64, String> {
        if func_idx < self.num_imports {
            // Call host function — clone Arc to avoid borrowing self immutably
            let key = self.host_fn_keys.get(func_idx as usize)
                .and_then(|k| k.clone())
                .ok_or_else(|| format!("import {} not linked", func_idx))?;
            let host_fns = std::sync::Arc::clone(&self.host_fns);
            let f = host_fns.get(&key).ok_or_else(|| format!("host fn {:?} not found", key))?;
            let result = f(self, args)?;
            return Ok(result.first().map(|v| v.unwrap_i64()).unwrap_or(0));
        }
        let code_idx = (func_idx - self.num_imports) as usize;
        let type_idx = self.func_type_indices[code_idx] as usize;
        let func_type = self.types[type_idx].clone();
        let body = self.code[code_idx].clone();
        execute_function(self, &body, &func_type, args)
    }
}

// ── Execution engine ────────────────────────────────────────────────────

fn execute_function(inst: &mut Instance, body: &[u8], func_type: &FuncType, args: &[Val]) -> Result<i64, String> {
    let mut pos = 0;
    let (local_count, c) = read_leb128_u32(&body[pos..])?; pos += c;
    let mut locals: Vec<Val> = Vec::new();
    // First: function parameters
    for (i, &_pt) in func_type.params.iter().enumerate() {
        locals.push(args.get(i).copied().unwrap_or(Val::I64(0)));
    }
    // Then: declared locals
    for _ in 0..local_count {
        let (count, c) = read_leb128_u32(&body[pos..])?; pos += c;
        let vt = body[pos]; pos += 1;
        for _ in 0..count {
            locals.push(match vt { 0x7C => Val::F64(0.0), _ => Val::I64(0) });
        }
    }

    let mut stack: Vec<Val> = Vec::with_capacity(256);
    let mut block_stack: Vec<(usize, usize, bool)> = Vec::new(); // (pc_end, stack_depth, is_loop_start_pc)
    let code = &body[pos..];

    exec_block(inst, code, &mut locals, &mut stack, &mut block_stack)
}

fn exec_block(
    inst: &mut Instance,
    code: &[u8],
    locals: &mut Vec<Val>,
    stack: &mut Vec<Val>,
    block_stack: &mut Vec<(usize, usize, bool)>,
) -> Result<i64, String> {
    let mut pc = 0usize;
    let mut iter_count = 0u64;
    let max_iters = 100_000_000u64;

    while pc < code.len() {
        iter_count += 1;
        if iter_count > max_iters { return Err("WASM execution exceeded iteration limit".into()); }

        let op = code[pc]; pc += 1;
        match op {
            0x00 => return Err("unreachable executed".into()), // unreachable
            0x01 => {} // nop
            0x02 => { // block
                pc += 1; // skip block type
                let end_pc = find_end(code, pc);
                block_stack.push((end_pc, stack.len(), false));
            }
            0x03 => { // loop
                pc += 1; // skip block type
                let end_pc = find_end(code, pc);
                block_stack.push((end_pc, stack.len(), true));
                block_stack.last_mut().unwrap().0 = pc; // loop target is start of loop
            }
            0x04 => { // if
                pc += 1; // skip block type
                let cond = stack.pop().unwrap_or(Val::I32(0));
                let end_pc = find_end(code, pc);
                let else_pc = find_else(code, pc);
                let is_true = match cond { Val::I32(v) => v != 0, Val::I64(v) => v != 0, Val::F64(v) => v != 0.0 };
                if is_true {
                    block_stack.push((end_pc, stack.len(), false));
                    // execute then branch (pc is already at start)
                } else if let Some(ep) = else_pc {
                    block_stack.push((end_pc, stack.len(), false));
                    pc = ep;
                } else {
                    pc = end_pc;
                }
            }
            0x05 => { // else — skip to end of block and pop it
                if let Some((end_pc, _, _)) = block_stack.pop() {
                    pc = end_pc;
                }
            }
            0x0B => { // end
                if block_stack.pop().is_none() {
                    let result = stack.last().map(|v| v.unwrap_i64()).unwrap_or(0);
                    return Ok(result);
                }
            }
            0x0C => { // br
                let (depth, c) = read_leb128_u32(&code[pc..])?; pc += c;
                do_branch(block_stack, depth as usize, &mut pc);
            }
            0x0D => { // br_if
                let (depth, c) = read_leb128_u32(&code[pc..])?; pc += c;
                let cond = stack.pop().unwrap_or(Val::I32(0));
                let is_true = match cond { Val::I32(v) => v != 0, Val::I64(v) => v != 0, _ => false };
                if is_true { do_branch(block_stack, depth as usize, &mut pc); }
            }
            0x0E => { // br_table
                let (count, c) = read_leb128_u32(&code[pc..])?; pc += c;
                let mut targets = Vec::new();
                for _ in 0..count {
                    let (t, c) = read_leb128_u32(&code[pc..])?; pc += c;
                    targets.push(t);
                }
                let (default, c) = read_leb128_u32(&code[pc..])?; pc += c;
                let idx = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                let depth = if idx < targets.len() { targets[idx] } else { default };
                do_branch(block_stack, depth as usize, &mut pc);
            }
            0x0F => { // return
                let result = stack.last().map(|v| v.unwrap_i64()).unwrap_or(0);
                return Ok(result);
            }
            0x10 => { // call
                let (func_idx, c) = read_leb128_u32(&code[pc..])?; pc += c;
                let ft_idx = if (func_idx as usize) < inst.num_imports as usize {
                    // import — type from import section
                    inst.import_type_indices.get(func_idx as usize).copied().unwrap_or(0)
                } else {
                    let code_idx = (func_idx - inst.num_imports) as usize;
                    inst.func_type_indices.get(code_idx).copied().unwrap_or(0)
                };
                let param_count = inst.types.get(ft_idx as usize).map(|t| t.params.len()).unwrap_or(0);
                let mut args = Vec::new();
                for _ in 0..param_count {
                    args.push(stack.pop().unwrap_or(Val::I64(0)));
                }
                args.reverse();
                let result = inst.call_func_with_args(func_idx, &args)?;
                let has_result = inst.types.get(ft_idx as usize).map(|t| !t.results.is_empty()).unwrap_or(false);
                if has_result { stack.push(Val::I64(result)); }
            }
            0x11 => { // call_indirect
                let (type_idx, c) = read_leb128_u32(&code[pc..])?; pc += c;
                let (_table_idx, c) = read_leb128_u32(&code[pc..])?; pc += c;
                let table_entry = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                let func_idx = inst.func_table.get(table_entry).copied().unwrap_or(u32::MAX);
                if func_idx == u32::MAX { return Err("indirect call to null table entry".into()); }
                let param_count = inst.types.get(type_idx as usize).map(|t| t.params.len()).unwrap_or(0);
                let mut args = Vec::new();
                for _ in 0..param_count { args.push(stack.pop().unwrap_or(Val::I64(0))); }
                args.reverse();
                let result = inst.call_func_with_args(func_idx, &args)?;
                let has_result = inst.types.get(type_idx as usize).map(|t| !t.results.is_empty()).unwrap_or(false);
                if has_result { stack.push(Val::I64(result)); }
            }
            0x1A => { stack.pop(); } // drop
            0x1B => { // select
                let c = stack.pop().unwrap_or(Val::I32(0));
                let b = stack.pop().unwrap_or(Val::I64(0));
                let a = stack.pop().unwrap_or(Val::I64(0));
                let cond = match c { Val::I32(v) => v != 0, Val::I64(v) => v != 0, _ => false };
                stack.push(if cond { a } else { b });
            }
            0x20 => { let (i, c) = read_leb128_u32(&code[pc..])?; pc += c; stack.push(locals.get(i as usize).copied().unwrap_or(Val::I64(0))); }
            0x21 => { let (i, c) = read_leb128_u32(&code[pc..])?; pc += c; let v = stack.pop().unwrap_or(Val::I64(0)); if let Some(l) = locals.get_mut(i as usize) { *l = v; } }
            0x22 => { let (i, c) = read_leb128_u32(&code[pc..])?; pc += c; let v = stack.last().copied().unwrap_or(Val::I64(0)); if let Some(l) = locals.get_mut(i as usize) { *l = v; } }
            0x23 => { let (i, c) = read_leb128_u32(&code[pc..])?; pc += c; stack.push(inst.globals.get(i as usize).copied().unwrap_or(Val::I32(0))); }
            0x24 => { let (i, c) = read_leb128_u32(&code[pc..])?; pc += c; let v = stack.pop().unwrap_or(Val::I32(0)); if let Some(g) = inst.globals.get_mut(i as usize) { *g = v; } }
            // Memory loads/stores
            0x28 => { let (_, o, c) = read_memarg(&code[pc..])?; pc += c; let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize + o; let val = read_u32_le(&inst.memory, addr); stack.push(Val::I32(val as i32)); }
            0x29 => { let (_, o, c) = read_memarg(&code[pc..])?; pc += c; let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize + o; let val = read_u64_le(&inst.memory, addr); stack.push(Val::I64(val as i64)); }
            0x2B => { let (_, o, c) = read_memarg(&code[pc..])?; pc += c; let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize + o; let val = read_u64_le(&inst.memory, addr); stack.push(Val::F64(f64::from_bits(val))); }
            0x36 => { let (_, o, c) = read_memarg(&code[pc..])?; pc += c; let val = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize + o; write_u32_le(&mut inst.memory, addr, val as u32); }
            0x37 => { let (_, o, c) = read_memarg(&code[pc..])?; pc += c; let val = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize + o; write_u64_le(&mut inst.memory, addr, val as u64); }
            0x3F => { let (_idx, c) = read_leb128_u32(&code[pc..])?; pc += c; stack.push(Val::I32((inst.memory.len() / 65536) as i32)); }
            0x40 => {
                let (_idx, c) = read_leb128_u32(&code[pc..])?; pc += c;
                let pages = stack.pop().and_then(|v| v.i32()).unwrap_or(0);
                if pages < 0 || pages > 65536 {
                    stack.push(Val::I32(-1)); // grow failed
                } else {
                    let old = (inst.memory.len() / 65536) as i32;
                    inst.memory.resize(inst.memory.len() + pages as usize * 65536, 0);
                    stack.push(Val::I32(old));
                }
            }
            0x41 => { let (v, c) = read_leb128_i32(&code[pc..])?; pc += c; stack.push(Val::I32(v)); }
            0x42 => { let (v, c) = read_leb128_i64(&code[pc..])?; pc += c; stack.push(Val::I64(v)); }
            0x44 => { let bits = u64::from_le_bytes([code[pc],code[pc+1],code[pc+2],code[pc+3],code[pc+4],code[pc+5],code[pc+6],code[pc+7]]); pc += 8; stack.push(Val::F64(f64::from_bits(bits))); }
            // i32 ops
            0x45 => { let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if a == 0 { 1 } else { 0 })); }
            0x46 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if a == b { 1 } else { 0 })); }
            0x47 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if a != b { 1 } else { 0 })); }
            0x48 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if a < b { 1 } else { 0 })); }
            0x49 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if (a as u32) < (b as u32) { 1 } else { 0 })); }
            0x4B => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if (a as u32) > (b as u32) { 1 } else { 0 })); }
            0x4E => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if (a as u32) >= (b as u32) { 1 } else { 0 })); }
            0x6A => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a.wrapping_add(b))); }
            0x6C => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a.wrapping_mul(b))); }
            0x72 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a | b)); }
            0x74 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a.wrapping_shl(b as u32))); }
            0xA7 => { let v = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(v as i32)); }
            // i64 ops
            0x50 => { let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(if a == 0 { 1 } else { 0 })); }
            0x51 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(if a == b { 1 } else { 0 })); }
            0x52 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(if a != b { 1 } else { 0 })); }
            0x53 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(if a < b { 1 } else { 0 })); }
            0x55 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(if a > b { 1 } else { 0 })); }
            0x57 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(if a <= b { 1 } else { 0 })); }
            0x59 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(if a >= b { 1 } else { 0 })); }
            0x5A => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(if (a as u64) >= (b as u64) { 1 } else { 0 })); }
            0x7C => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a.wrapping_add(b))); }
            0x7D => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a.wrapping_sub(b))); }
            0x7E => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a.wrapping_mul(b))); }
            0x7F => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); if b == 0 { return Err("division by zero".into()); } if a == i64::MIN && b == -1 { return Err("integer overflow in division".into()); } stack.push(Val::I64(a.wrapping_div(b))); }
            0x81 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); if b == 0 { return Err("division by zero".into()); } if a == i64::MIN && b == -1 { stack.push(Val::I64(0)); } else { stack.push(Val::I64(a.wrapping_rem(b))); } }
            0x83 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a & b)); }
            0x84 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a | b)); }
            0x86 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a.wrapping_shl(b as u32))); }
            0x87 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a.wrapping_shr(b as u32))); }
            0x88 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(((a as u64).wrapping_shr(b as u32)) as i64)); }
            0xAD => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I64(v as u32 as i64)); }
            // f64 ops
            0x61 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::I32(if a == b { 1 } else { 0 })); }
            0x62 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::I32(if a != b { 1 } else { 0 })); }
            0x63 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::I32(if a < b { 1 } else { 0 })); }
            0x64 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::I32(if a > b { 1 } else { 0 })); }
            0x65 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::I32(if a <= b { 1 } else { 0 })); }
            0x66 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::I32(if a >= b { 1 } else { 0 })); }
            0x99 => { let v = pop_f64(stack); stack.push(Val::F64(v.abs())); }
            0x9A => { let v = pop_f64(stack); stack.push(Val::F64(-v)); }
            0x9B => { let v = pop_f64(stack); stack.push(Val::F64(v.ceil())); }
            0x9C => { let v = pop_f64(stack); stack.push(Val::F64(v.floor())); }
            0x9D => { let v = pop_f64(stack); stack.push(Val::F64(v.trunc())); }
            0x9F => { let v = pop_f64(stack); stack.push(Val::F64(v.sqrt())); }
            0xA0 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::F64(a + b)); }
            0xA1 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::F64(a - b)); }
            0xA2 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::F64(a * b)); }
            0xA3 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::F64(a / b)); }
            0xB0 => { let v = pop_f64(stack); stack.push(Val::I64(v as i64)); }
            0xB9 => { let v = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::F64(v as f64)); }
            0xBD => { let v = pop_f64(stack); stack.push(Val::I64(v.to_bits() as i64)); }
            0xBF => { let v = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::F64(f64::from_bits(v as u64))); }

            // i32 arithmetic (missing)
            0x6B => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a.wrapping_sub(b))); } // i32.sub
            0x6D => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a.wrapping_sub(b))); } // i32.sub (alt)
            0x6E => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); if b == 0 { return Err("i32 division by zero".into()); } stack.push(Val::I32(a.wrapping_div(b))); } // i32.div_s
            0x6F => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); if b == 0 { return Err("i32 division by zero".into()); } stack.push(Val::I32(a.wrapping_rem(b))); } // i32.rem_s
            0x70 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); if b == 0 { return Err("i32 division by zero".into()); } stack.push(Val::I32(((a as u32).wrapping_rem(b as u32)) as i32)); } // i32.rem_u
            0x71 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a & b)); } // i32.and
            0x73 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a ^ b)); } // i32.xor
            0x75 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a.wrapping_shr(b as u32))); } // i32.shr_s
            0x76 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(((a as u32).wrapping_shr(b as u32)) as i32)); } // i32.shr_u
            0x77 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a.rotate_left(b as u32))); } // i32.rotl
            0x78 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a.rotate_right(b as u32))); } // i32.rotr
            0x67 => { let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a.leading_zeros() as i32)); } // i32.clz
            0x68 => { let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a.trailing_zeros() as i32)); } // i32.ctz
            0x69 => { let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(a.count_ones() as i32)); } // i32.popcnt
            0x4A => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if a > b { 1 } else { 0 })); } // i32.gt_s
            0x4C => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if a <= b { 1 } else { 0 })); } // i32.le_s
            0x4D => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if (a as u32) <= (b as u32) { 1 } else { 0 })); } // i32.le_u
            0x4F => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if a >= b { 1 } else { 0 })); } // i32.ge_s

            // i64 missing
            0x54 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(if (a as u64) < (b as u64) { 1 } else { 0 })); } // i64.lt_u
            0x56 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(if (a as u64) > (b as u64) { 1 } else { 0 })); } // i64.gt_u
            0x58 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(if (a as u64) <= (b as u64) { 1 } else { 0 })); } // i64.le_u
            0x85 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a ^ b)); } // i64.xor
            0x89 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a.rotate_left(b as u32))); } // i64.rotl
            0x8A => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a.rotate_right(b as u32))); } // i64.rotr
            0x79 => { let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a.leading_zeros() as i64)); } // i64.clz
            0x7A => { let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a.trailing_zeros() as i64)); } // i64.ctz
            0x7B => { let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I64(a.count_ones() as i64)); } // i64.popcnt
            0x80 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); if b == 0 { return Err("i64 division by zero".into()); } stack.push(Val::I64(((a as u64).wrapping_div(b as u64)) as i64)); } // i64.div_u
            0x82 => { let b = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); let a = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); if b == 0 { return Err("i64 division by zero".into()); } stack.push(Val::I64(((a as u64).wrapping_rem(b as u64)) as i64)); } // i64.rem_u

            // f32 ops
            0x43 => { let v = f32::from_le_bytes([code[pc], code[pc+1], code[pc+2], code[pc+3]]); pc += 4; stack.push(Val::I32(v.to_bits() as i32)); } // f32.const
            0x92 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32((f32::from_bits(a as u32) + f32::from_bits(b as u32)).to_bits() as i32)); } // f32.add
            0x93 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32((f32::from_bits(a as u32) - f32::from_bits(b as u32)).to_bits() as i32)); } // f32.sub
            0x94 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32((f32::from_bits(a as u32) * f32::from_bits(b as u32)).to_bits() as i32)); } // f32.mul
            0x95 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32((f32::from_bits(a as u32) / f32::from_bits(b as u32)).to_bits() as i32)); } // f32.div
            0x8B => { let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(f32::from_bits(a as u32).abs().to_bits() as i32)); } // f32.abs
            0x8C => { let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32((-f32::from_bits(a as u32)).to_bits() as i32)); } // f32.neg
            0x91 => { let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(f32::from_bits(a as u32).sqrt().to_bits() as i32)); } // f32.sqrt
            0x5B => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if f32::from_bits(a as u32) == f32::from_bits(b as u32) { 1 } else { 0 })); } // f32.eq
            0x5C => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if f32::from_bits(a as u32) != f32::from_bits(b as u32) { 1 } else { 0 })); } // f32.ne
            0x5D => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if f32::from_bits(a as u32) < f32::from_bits(b as u32) { 1 } else { 0 })); } // f32.lt
            0x5E => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if f32::from_bits(a as u32) > f32::from_bits(b as u32) { 1 } else { 0 })); } // f32.gt

            0x5F => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if f32::from_bits(a as u32) <= f32::from_bits(b as u32) { 1 } else { 0 })); } // f32.le
            0x60 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(if f32::from_bits(a as u32) >= f32::from_bits(b as u32) { 1 } else { 0 })); } // f32.ge
            0x8D => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(f32::from_bits(v as u32).ceil().to_bits() as i32)); } // f32.ceil
            0x8E => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(f32::from_bits(v as u32).floor().to_bits() as i32)); } // f32.floor
            0x8F => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(f32::from_bits(v as u32).trunc().to_bits() as i32)); } // f32.trunc
            0x90 => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(f32::from_bits(v as u32).round().to_bits() as i32)); } // f32.nearest
            0x96 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(f32::from_bits(a as u32).min(f32::from_bits(b as u32)).to_bits() as i32)); } // f32.min
            0x97 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(f32::from_bits(a as u32).max(f32::from_bits(b as u32)).to_bits() as i32)); } // f32.max
            0x98 => { let b = stack.pop().and_then(|v| v.i32()).unwrap_or(0); let a = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(f32::from_bits(a as u32).copysign(f32::from_bits(b as u32)).to_bits() as i32)); } // f32.copysign

            0x9E => { let v = pop_f64(stack); stack.push(Val::F64(v.round())); } // f64.nearest
            0xA4 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::F64(a.min(b))); } // f64.min
            0xA5 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::F64(a.max(b))); } // f64.max
            0xA6 => { let b = pop_f64(stack); let a = pop_f64(stack); stack.push(Val::F64(a.copysign(b))); } // f64.copysign

            // Type conversions
            0xA8 => { let v = pop_f64(stack); stack.push(Val::I32(v as i32)); } // i32.trunc_f64_s
            0xA9 => { let v = pop_f64(stack); stack.push(Val::I32(v as u32 as i32)); } // i32.trunc_f64_u
            0xAA => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(f32::from_bits(v as u32) as i32)); } // i32.trunc_f32_s
            0xAB => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(f32::from_bits(v as u32) as u32 as i32)); } // i32.trunc_f32_u
            0xAC => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I64(v as i64)); } // i64.extend_i32_s
            0xAE => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I64((f32::from_bits(v as u32)) as i64)); } // i64.trunc_f32_s
            0xAF => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I64(f32::from_bits(v as u32) as u64 as i64)); } // i64.trunc_f32_u
            0xB1 => { let v = pop_f64(stack); stack.push(Val::I64(v as u64 as i64)); } // i64.trunc_f64_u
            0xB2 => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32((v as f32).to_bits() as i32)); } // f32.convert_i32_s
            0xB3 => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(((v as u32) as f32).to_bits() as i32)); } // f32.convert_i32_u
            0xB4 => { let v = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32((v as f32).to_bits() as i32)); } // f32.convert_i64_s
            0xB5 => { let v = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::I32(((v as u64) as f32).to_bits() as i32)); } // f32.convert_i64_u
            0xB6 => { let v = pop_f64(stack); stack.push(Val::I32((v as f32).to_bits() as i32)); } // f32.demote_f64
            0xB7 => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::F64(v as f64)); } // f64.convert_i32_s
            0xB8 => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::F64((v as u32) as f64)); } // f64.convert_i32_u
            0xBA => { let v = stack.pop().map(|v| v.unwrap_i64()).unwrap_or(0); stack.push(Val::F64((v as u64) as f64)); } // f64.convert_i64_u
            0xBB => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::F64(f32::from_bits(v as u32) as f64)); } // f64.promote_f32
            0xBC => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(v)); } // i32.reinterpret_f32
            0xBE => { let v = stack.pop().and_then(|v| v.i32()).unwrap_or(0); stack.push(Val::I32(v)); } // f32.reinterpret_i32

            // f32.load (0x2A)
            0x2A => {
                let (_align, _offset, c) = read_memarg(&code[pc..])?; pc += c;
                let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                if addr + 4 <= inst.memory.len() {
                    let v = f32::from_le_bytes([inst.memory[addr], inst.memory[addr+1], inst.memory[addr+2], inst.memory[addr+3]]);
                    stack.push(Val::I32(v.to_bits() as i32));
                } else { stack.push(Val::I32(0)); }
            }
            // f32.store (0x38)
            0x38 => {
                let (_align, _offset, c) = read_memarg(&code[pc..])?; pc += c;
                let val = stack.pop().and_then(|v| v.i32()).unwrap_or(0);
                let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                if addr + 4 <= inst.memory.len() {
                    let bytes = (val as u32).to_le_bytes();
                    inst.memory[addr..addr+4].copy_from_slice(&bytes);
                }
            }
            // i32.load8_s (0x2C)
            0x2C => {
                let (_align, _offset, c) = read_memarg(&code[pc..])?; pc += c;
                let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                if addr < inst.memory.len() { stack.push(Val::I32(inst.memory[addr] as i8 as i32)); }
                else { stack.push(Val::I32(0)); }
            }
            // i32.load8_u (0x2D)
            0x2D => {
                let (_align, _offset, c) = read_memarg(&code[pc..])?; pc += c;
                let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                if addr < inst.memory.len() { stack.push(Val::I32(inst.memory[addr] as i32)); }
                else { stack.push(Val::I32(0)); }
            }
            // i32.load16_s (0x2E)
            0x2E => {
                let (_align, _offset, c) = read_memarg(&code[pc..])?; pc += c;
                let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                if addr + 2 <= inst.memory.len() { stack.push(Val::I32(i16::from_le_bytes([inst.memory[addr], inst.memory[addr+1]]) as i32)); }
                else { stack.push(Val::I32(0)); }
            }
            // i32.store8 (0x3A)
            0x3A => {
                let (_align, _offset, c) = read_memarg(&code[pc..])?; pc += c;
                let val = stack.pop().and_then(|v| v.i32()).unwrap_or(0);
                let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                if addr < inst.memory.len() { inst.memory[addr] = val as u8; }
            }
            // i32.store16 (0x3B)
            0x3B => {
                let (_align, _offset, c) = read_memarg(&code[pc..])?; pc += c;
                let val = stack.pop().and_then(|v| v.i32()).unwrap_or(0);
                let addr = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                if addr + 2 <= inst.memory.len() { let bytes = (val as u16).to_le_bytes(); inst.memory[addr..addr+2].copy_from_slice(&bytes); }
            }

            // memory.copy (0xFC prefix)
            0xFC => {
                let (sub_op, c) = read_leb128_u32(&code[pc..])?; pc += c;
                if sub_op == 10 { // memory.copy
                    let (_dst_mem, c) = read_leb128_u32(&code[pc..])?; pc += c;
                    let (_src_mem, c) = read_leb128_u32(&code[pc..])?; pc += c;
                    let n = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                    let src = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                    let dst = stack.pop().and_then(|v| v.i32()).unwrap_or(0) as usize;
                    if src + n <= inst.memory.len() && dst + n <= inst.memory.len() {
                        let tmp: Vec<u8> = inst.memory[src..src+n].to_vec();
                        inst.memory[dst..dst+n].copy_from_slice(&tmp);
                    }
                }
            }
            other => return Err(format!("unsupported WASM opcode: 0x{:02X} at pc={}", other, pc - 1)),
        }
    }
    stack.last().map(|v| v.unwrap_i64()).ok_or_else(|| "empty stack at function end".into())
}

fn pop_f64(stack: &mut Vec<Val>) -> f64 {
    match stack.pop() {
        Some(Val::F64(f)) => f,
        Some(Val::I64(i)) => f64::from_bits(i as u64),
        Some(Val::I32(i)) => i as f64,
        None => 0.0,
    }
}

fn read_memarg(data: &[u8]) -> Result<(u32, usize, usize), String> {
    let (align, c1) = read_leb128_u32(data)?;
    let (offset, c2) = read_leb128_u32(&data[c1..])?;
    Ok((align, offset as usize, c1 + c2))
}

fn read_u32_le(mem: &[u8], addr: usize) -> u32 {
    if addr + 4 > mem.len() { return 0; }
    u32::from_le_bytes([mem[addr], mem[addr+1], mem[addr+2], mem[addr+3]])
}

fn read_u64_le(mem: &[u8], addr: usize) -> u64 {
    if addr + 8 > mem.len() { return 0; }
    u64::from_le_bytes([mem[addr], mem[addr+1], mem[addr+2], mem[addr+3], mem[addr+4], mem[addr+5], mem[addr+6], mem[addr+7]])
}

fn write_u32_le(mem: &mut [u8], addr: usize, val: u32) {
    if addr + 4 <= mem.len() { mem[addr..addr+4].copy_from_slice(&val.to_le_bytes()); }
}

fn write_u64_le(mem: &mut [u8], addr: usize, val: u64) {
    if addr + 8 <= mem.len() { mem[addr..addr+8].copy_from_slice(&val.to_le_bytes()); }
}

/// Find the matching End opcode for a block/if/loop.
fn find_end(code: &[u8], start: usize) -> usize {
    let mut depth = 1;
    let mut pc = start;
    while pc < code.len() {
        match code[pc] {
            0x02 | 0x03 | 0x04 => { depth += 1; pc += 2; } // block/loop/if + blocktype
            0x0B => { depth -= 1; if depth == 0 { return pc + 1; } pc += 1; }
            0x05 => { if depth == 1 { /* else at our level — skip */ } pc += 1; }
            0x0C | 0x0D => { pc += 1; let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; }
            0x0E => { // br_table
                pc += 1;
                let (count, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c;
                for _ in 0..=count { let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; }
            }
            0x10 | 0x20 | 0x21 | 0x22 | 0x23 | 0x24 | 0x3F | 0x40 => {
                pc += 1; let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c;
            }
            0x11 => { pc += 1; let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; let (_, c2) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c2; }
            0x28 | 0x29 | 0x2B | 0x36 | 0x37 => { pc += 1; let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; let (_, c2) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c2; }
            0x41 => { pc += 1; let (_, c) = read_leb128_i32(&code[pc..]).unwrap_or((0, 1)); pc += c; }
            0x42 => { pc += 1; let (_, c) = read_leb128_i64(&code[pc..]).unwrap_or((0, 1)); pc += c; }
            0x44 => { pc += 9; } // f64.const
            0xFC => { pc += 1; let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; let (_, c2) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c2; let (_, c3) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c3; }
            _ => { pc += 1; }
        }
    }
    code.len()
}

/// Find the Else opcode matching a block at the current depth.
fn find_else(code: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1;
    let mut pc = start;
    while pc < code.len() {
        match code[pc] {
            0x02 | 0x03 | 0x04 => { depth += 1; pc += 2; }
            0x0B => { depth -= 1; if depth == 0 { return None; } pc += 1; }
            0x05 => { if depth == 1 { return Some(pc + 1); } pc += 1; }
            0x0C | 0x0D => { pc += 1; let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; }
            0x0E => { pc += 1; let (count, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; for _ in 0..=count { let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; } }
            0x10 | 0x20 | 0x21 | 0x22 | 0x23 | 0x24 | 0x3F | 0x40 => { pc += 1; let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; }
            0x11 => { pc += 1; let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; let (_, c2) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c2; }
            0x28 | 0x29 | 0x2B | 0x36 | 0x37 => { pc += 1; let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; let (_, c2) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c2; }
            0x41 => { pc += 1; let (_, c) = read_leb128_i32(&code[pc..]).unwrap_or((0, 1)); pc += c; }
            0x42 => { pc += 1; let (_, c) = read_leb128_i64(&code[pc..]).unwrap_or((0, 1)); pc += c; }
            0x44 => { pc += 9; }
            0xFC => { pc += 1; let (_, c) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c; let (_, c2) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c2; let (_, c3) = read_leb128_u32(&code[pc..]).unwrap_or((0, 1)); pc += c3; }
            _ => { pc += 1; }
        }
    }
    None
}

fn do_branch(block_stack: &mut Vec<(usize, usize, bool)>, depth: usize, pc: &mut usize) {
    if depth >= block_stack.len() { return; }
    let target_idx = block_stack.len() - 1 - depth;
    let (target_pc, _, is_loop) = block_stack[target_idx];
    // Pop blocks above the target
    while block_stack.len() > target_idx + 1 { block_stack.pop(); }
    if is_loop {
        // Loop: branch back to loop start (target_pc is start of loop body)
        *pc = target_pc;
    } else {
        // Block: branch to end
        block_stack.pop(); // pop the target block itself
        *pc = target_pc;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::wasm_binary::{
        self as wb, BlockType, CodeSection, ConstExpr, DataSection, ElementSection, TypeSection,
        EntityType, ExportKind, ExportSection, FunctionSection, GlobalSection,
        GlobalType, ImportSection, Inst, MemArg, MemorySection, MemoryType, NameMap,
        NameSection, TableSection, ValType, Function,
    };

    fn build_simple_module(body_fn: impl FnOnce(&mut Function)) -> Vec<u8> {
        let mut module = wb::Module::new();

        let mut types = TypeSection::new();
        types.ty().function(vec![], vec![ValType::I64]);
        module.section(&types);

        let mut functions = FunctionSection::new();
        functions.function(0);
        module.section(&functions);

        let mut memory = MemorySection::new();
        memory.memory(MemoryType { minimum: 1, maximum: Some(16), memory64: false, shared: false, page_size_log2: None });
        module.section(&memory);

        let mut exports = ExportSection::new();
        exports.export("main", ExportKind::Func, 0);
        exports.export("memory", ExportKind::Memory, 0);
        module.section(&exports);

        let mut code = CodeSection::new();
        let mut func = Function::new(vec![]);
        body_fn(&mut func);
        code.function(&func);
        module.section(&code);

        module.finish()
    }

    fn run_module(wasm: &[u8]) -> i64 {
        let engine = Engine::default();
        let module = super::Module::new(&engine, wasm).unwrap();
        let mut store = Store::new(&engine, ());
        let linker = Linker::new(&engine);
        let mut instance = linker.instantiate(&mut store, &module).unwrap();
        instance.call("main", &mut store).unwrap()
    }

    #[test]
    fn runtime_i64_const() {
        let wasm = build_simple_module(|f| {
            f.instruction(&Inst::I64Const(42));
            f.instruction(&Inst::End);
        });
        assert_eq!(run_module(&wasm), 42);
    }

    #[test]
    fn runtime_i64_add() {
        let wasm = build_simple_module(|f| {
            f.instruction(&Inst::I64Const(10));
            f.instruction(&Inst::I64Const(32));
            f.instruction(&Inst::I64Add);
            f.instruction(&Inst::End);
        });
        assert_eq!(run_module(&wasm), 42);
    }

    #[test]
    fn runtime_i64_sub() {
        let wasm = build_simple_module(|f| {
            f.instruction(&Inst::I64Const(100));
            f.instruction(&Inst::I64Const(58));
            f.instruction(&Inst::I64Sub);
            f.instruction(&Inst::End);
        });
        assert_eq!(run_module(&wasm), 42);
    }

    #[test]
    fn runtime_i64_mul() {
        let wasm = build_simple_module(|f| {
            f.instruction(&Inst::I64Const(6));
            f.instruction(&Inst::I64Const(7));
            f.instruction(&Inst::I64Mul);
            f.instruction(&Inst::End);
        });
        assert_eq!(run_module(&wasm), 42);
    }

    #[test]
    fn runtime_i64_div() {
        let wasm = build_simple_module(|f| {
            f.instruction(&Inst::I64Const(84));
            f.instruction(&Inst::I64Const(2));
            f.instruction(&Inst::I64DivS);
            f.instruction(&Inst::End);
        });
        assert_eq!(run_module(&wasm), 42);
    }

    #[test]
    fn runtime_local_get_set() {
        let mut module = wb::Module::new();

        let mut types = TypeSection::new();
        types.ty().function(vec![], vec![ValType::I64]);
        module.section(&types);

        let mut functions = FunctionSection::new();
        functions.function(0);
        module.section(&functions);

        let mut exports = ExportSection::new();
        exports.export("main", ExportKind::Func, 0);
        module.section(&exports);

        let mut code = CodeSection::new();
        let mut func = Function::new(vec![(1, ValType::I64)]); // 1 local
        func.instruction(&Inst::I64Const(42));
        func.instruction(&Inst::LocalSet(0));
        func.instruction(&Inst::LocalGet(0));
        func.instruction(&Inst::End);
        code.function(&func);
        module.section(&code);

        let wasm = module.finish();
        assert_eq!(run_module(&wasm), 42);
    }

    #[test]
    fn runtime_if_true() {
        let wasm = build_simple_module(|f| {
            f.instruction(&Inst::I32Const(1)); // true
            f.instruction(&Inst::If(BlockType::Result(ValType::I64)));
            f.instruction(&Inst::I64Const(42));
            f.instruction(&Inst::Else);
            f.instruction(&Inst::I64Const(0));
            f.instruction(&Inst::End);
            f.instruction(&Inst::End);
        });
        assert_eq!(run_module(&wasm), 42);
    }

    #[test]
    fn runtime_if_false() {
        let wasm = build_simple_module(|f| {
            f.instruction(&Inst::I32Const(0)); // false
            f.instruction(&Inst::If(BlockType::Result(ValType::I64)));
            f.instruction(&Inst::I64Const(0));
            f.instruction(&Inst::Else);
            f.instruction(&Inst::I64Const(42));
            f.instruction(&Inst::End);
            f.instruction(&Inst::End);
        });
        assert_eq!(run_module(&wasm), 42);
    }

    #[test]
    fn runtime_loop_with_br() {
        // Loop counting from 0 to 10, return final value
        let mut module = wb::Module::new();
        let mut types = TypeSection::new();
        types.ty().function(vec![], vec![ValType::I64]);
        module.section(&types);

        let mut functions = FunctionSection::new();
        functions.function(0);
        module.section(&functions);

        let mut exports = ExportSection::new();
        exports.export("main", ExportKind::Func, 0);
        module.section(&exports);

        let mut code = CodeSection::new();
        let mut func = Function::new(vec![(1, ValType::I64)]); // counter
        func.instruction(&Inst::I64Const(0));
        func.instruction(&Inst::LocalSet(0));
        // Block wrapping the loop (for break)
        func.instruction(&Inst::Block(BlockType::Empty));
        func.instruction(&Inst::Loop(BlockType::Empty));
        // counter += 1
        func.instruction(&Inst::LocalGet(0));
        func.instruction(&Inst::I64Const(1));
        func.instruction(&Inst::I64Add);
        func.instruction(&Inst::LocalSet(0));
        // if counter >= 10, break out of block
        func.instruction(&Inst::LocalGet(0));
        func.instruction(&Inst::I64Const(10));
        func.instruction(&Inst::I64GeS);
        func.instruction(&Inst::BrIf(1)); // break block (depth 1)
        // else continue loop
        func.instruction(&Inst::Br(0)); // continue loop (depth 0)
        func.instruction(&Inst::End); // end loop
        func.instruction(&Inst::End); // end block
        func.instruction(&Inst::LocalGet(0));
        func.instruction(&Inst::End); // end function
        code.function(&func);
        module.section(&code);

        let wasm = module.finish();
        assert_eq!(run_module(&wasm), 10);
    }

    #[test]
    fn runtime_memory_store_load() {
        let wasm = build_simple_module(|f| {
            // Store 42 at memory address 0
            f.instruction(&Inst::I32Const(0)); // addr
            f.instruction(&Inst::I64Const(42)); // value
            f.instruction(&Inst::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));
            // Load back from address 0
            f.instruction(&Inst::I32Const(0));
            f.instruction(&Inst::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
            f.instruction(&Inst::End);
        });
        assert_eq!(run_module(&wasm), 42);
    }

    #[test]
    fn runtime_f64_ops() {
        let wasm = build_simple_module(|f| {
            f.instruction(&Inst::F64Const(3.0));
            f.instruction(&Inst::F64Const(4.0));
            f.instruction(&Inst::F64Add);
            // Convert to i64 for return
            f.instruction(&Inst::I64ReinterpretF64);
            f.instruction(&Inst::End);
        });
        let result = run_module(&wasm);
        let f = f64::from_bits(result as u64);
        assert!((f - 7.0).abs() < 0.001);
    }

    #[test]
    fn runtime_select() {
        let wasm = build_simple_module(|f| {
            f.instruction(&Inst::I64Const(42)); // value if true
            f.instruction(&Inst::I64Const(0));  // value if false
            f.instruction(&Inst::I32Const(1));  // condition (true)
            f.instruction(&Inst::Select);
            f.instruction(&Inst::End);
        });
        assert_eq!(run_module(&wasm), 42);
    }

    #[test]
    fn runtime_host_function_call() {
        // Build module that imports and calls a host function
        let mut module = wb::Module::new();

        let mut types = TypeSection::new();
        types.ty().function(vec![ValType::I64], vec![ValType::I64]); // host fn type
        types.ty().function(vec![], vec![ValType::I64]); // main type
        module.section(&types);

        let mut imports = ImportSection::new();
        imports.import("env", "double", EntityType::Function(0));
        module.section(&imports);

        let mut functions = FunctionSection::new();
        functions.function(1); // main uses type 1
        module.section(&functions);

        let mut exports = ExportSection::new();
        exports.export("main", ExportKind::Func, 1); // func 1 (after 1 import)
        module.section(&exports);

        let mut code = CodeSection::new();
        let mut func = Function::new(vec![]);
        func.instruction(&Inst::I64Const(21));
        func.instruction(&Inst::Call(0)); // call imported "double"
        func.instruction(&Inst::End);
        code.function(&func);
        module.section(&code);

        let wasm = module.finish();

        let engine = Engine::default();
        let module = super::Module::new(&engine, &wasm).unwrap();
        let mut store = Store::new(&engine, ());
        let mut linker = Linker::new(&engine);
        linker.func_wrap_1_1("env", "double", |_inst: &mut Instance, val: i64| -> i64 {
            val * 2
        }).unwrap();
        let mut instance = linker.instantiate(&mut store, &module).unwrap();
        let result = instance.call("main", &mut store).unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn runtime_global_get_set() {
        let mut module = wb::Module::new();

        let mut types = TypeSection::new();
        types.ty().function(vec![], vec![ValType::I64]);
        module.section(&types);

        let mut functions = FunctionSection::new();
        functions.function(0);
        module.section(&functions);

        let mut globals = GlobalSection::new();
        globals.global(
            GlobalType { val_type: ValType::I32, mutable: true, shared: false },
            &ConstExpr::i32_const(100),
        );
        module.section(&globals);

        let mut exports = ExportSection::new();
        exports.export("main", ExportKind::Func, 0);
        exports.export("g", ExportKind::Global, 0);
        module.section(&exports);

        let mut code = CodeSection::new();
        let mut func = Function::new(vec![]);
        func.instruction(&Inst::GlobalGet(0));
        func.instruction(&Inst::I64ExtendI32U);
        func.instruction(&Inst::End);
        code.function(&func);
        module.section(&code);

        let wasm = module.finish();
        let engine = Engine::default();
        let module = super::Module::new(&engine, &wasm).unwrap();
        let mut store = Store::new(&engine, ());
        let linker = Linker::new(&engine);
        let mut instance = linker.instantiate(&mut store, &module).unwrap();
        let result = instance.call("main", &mut store).unwrap();
        assert_eq!(result, 100);

        // Test set_global via instance API
        instance.set_global("g", Val::I32(42)).unwrap();
        let result = instance.call("main", &mut store).unwrap();
        assert_eq!(result, 42);
    }
}
