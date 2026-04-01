//! Own WASM binary encoder — replaces the `wasm-encoder` crate.
//!
//! Provides a builder API that emits valid WASM 1.0 binary modules.

// ── Value types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
    FuncRef,
}

impl ValType {
    fn encode(&self) -> u8 {
        match self {
            ValType::I32 => 0x7F,
            ValType::I64 => 0x7E,
            ValType::F32 => 0x7D,
            ValType::F64 => 0x7C,
            ValType::FuncRef => 0x70,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BlockType {
    Empty,
    Result(ValType),
}

#[derive(Debug, Clone, Copy)]
pub struct MemArg {
    pub offset: u64,
    pub align: u32,
    pub memory_index: u32,
}

#[derive(Debug, Clone, Copy)]
pub enum ExportKind {
    Func,
    Table,
    Memory,
    Global,
}

#[derive(Debug, Clone, Copy)]
pub enum EntityType {
    Function(u32),
}

#[derive(Debug, Clone, Copy)]
pub struct GlobalType {
    pub val_type: ValType,
    pub mutable: bool,
    pub shared: bool,
}

#[derive(Debug, Clone)]
pub struct MemoryType {
    pub minimum: u64,
    pub maximum: Option<u64>,
    pub memory64: bool,
    pub shared: bool,
    pub page_size_log2: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct TableType {
    pub element_type: ValType,
    pub minimum: u64,
    pub maximum: Option<u64>,
    pub shared: bool,
    pub table64: bool,
}

pub struct RefType;
impl RefType {
    pub const FUNCREF: ValType = ValType::FuncRef;
}

// ── LEB128 encoding ─────────────────────────────────────────────────────

fn encode_u32_leb128(mut val: u32, out: &mut Vec<u8>) {
    loop {
        let byte = (val & 0x7F) as u8;
        val >>= 7;
        if val == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn encode_i32_leb128(mut val: i32, out: &mut Vec<u8>) {
    loop {
        let byte = (val & 0x7F) as u8;
        val >>= 7;
        if (val == 0 && byte & 0x40 == 0) || (val == -1 && byte & 0x40 != 0) {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn encode_i64_leb128(mut val: i64, out: &mut Vec<u8>) {
    loop {
        let byte = (val & 0x7F) as u8;
        val >>= 7;
        if (val == 0 && byte & 0x40 == 0) || (val == -1 && byte & 0x40 != 0) {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn encode_vec_len(len: usize, out: &mut Vec<u8>) {
    encode_u32_leb128(len as u32, out);
}

fn encode_name(name: &str, out: &mut Vec<u8>) {
    encode_vec_len(name.len(), out);
    out.extend_from_slice(name.as_bytes());
}

// ── Constant expressions ────────────────────────────────────────────────

pub struct ConstExpr;

impl ConstExpr {
    pub fn i32_const(val: i32) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(0x41); // i32.const
        encode_i32_leb128(val, &mut out);
        out.push(0x0B); // end
        out
    }

    pub fn i64_const(val: i64) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(0x42); // i64.const
        encode_i64_leb128(val, &mut out);
        out.push(0x0B); // end
        out
    }
}

// ── Instructions ────────────────────────────────────────────────────────

pub enum Inst {
    Unreachable,
    Nop,
    Block(BlockType),
    Loop(BlockType),
    If(BlockType),
    Else,
    End,
    Br(u32),
    BrIf(u32),
    BrTable { targets: Vec<u32>, default: u32 },
    Return,
    Call(u32),
    CallIndirect { type_idx: u32, table_idx: u32 },
    Drop,
    Select,

    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    GlobalGet(u32),
    GlobalSet(u32),

    I32Load(MemArg),
    I32Load8U(MemArg),
    I64Load(MemArg),
    F64Load(MemArg),
    I32Store(MemArg),
    I64Store(MemArg),
    MemorySize(u32),
    MemoryGrow(u32),
    MemoryCopy { dst_mem: u32, src_mem: u32 },

    I32Const(i32),
    I64Const(i64),
    F64Const(f64),

    // i32 ops
    I32Eqz,
    I32Eq,
    I32Ne,
    I32LtS,
    I32LtU,
    I32GtU,
    I32GeU,
    I32Add,
    I32Mul,
    I32And,
    I32Or,
    I32Shl,
    I32WrapI64,

    // i64 ops
    I64Eqz,
    I64Eq,
    I64Ne,
    I64LtS,
    I64GtS,
    I64LeS,
    I64GeS,
    I64GeU,
    I64Add,
    I64Sub,
    I64Mul,
    I64DivS,
    I64RemS,
    I64And,
    I64Or,
    I64Xor,
    I64Shl,
    I64ShrS,
    I64ShrU,
    I64ExtendI32U,

    // f64 ops
    F64Eq,
    F64Ne,
    F64Lt,
    F64Gt,
    F64Le,
    F64Ge,
    F64Abs,
    F64Neg,
    F64Ceil,
    F64Floor,
    F64Trunc,
    F64Sqrt,
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,

    I64TruncF64S,
    F64ConvertI64S,
    I64ReinterpretF64,
    F64ReinterpretI64,
}

impl Inst {
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Inst::Unreachable => out.push(0x00),
            Inst::Nop => out.push(0x01),
            Inst::Block(bt) => { out.push(0x02); encode_block_type(bt, out); }
            Inst::Loop(bt) => { out.push(0x03); encode_block_type(bt, out); }
            Inst::If(bt) => { out.push(0x04); encode_block_type(bt, out); }
            Inst::Else => out.push(0x05),
            Inst::End => out.push(0x0B),
            Inst::Br(l) => { out.push(0x0C); encode_u32_leb128(*l, out); }
            Inst::BrIf(l) => { out.push(0x0D); encode_u32_leb128(*l, out); }
            Inst::BrTable { targets, default } => {
                out.push(0x0E);
                encode_vec_len(targets.len(), out);
                for t in targets { encode_u32_leb128(*t, out); }
                encode_u32_leb128(*default, out);
            }
            Inst::Return => out.push(0x0F),
            Inst::Call(idx) => { out.push(0x10); encode_u32_leb128(*idx, out); }
            Inst::CallIndirect { type_idx, table_idx } => {
                out.push(0x11);
                encode_u32_leb128(*type_idx, out);
                encode_u32_leb128(*table_idx, out);
            }
            Inst::Drop => out.push(0x1A),
            Inst::Select => out.push(0x1B),

            Inst::LocalGet(i) => { out.push(0x20); encode_u32_leb128(*i, out); }
            Inst::LocalSet(i) => { out.push(0x21); encode_u32_leb128(*i, out); }
            Inst::LocalTee(i) => { out.push(0x22); encode_u32_leb128(*i, out); }
            Inst::GlobalGet(i) => { out.push(0x23); encode_u32_leb128(*i, out); }
            Inst::GlobalSet(i) => { out.push(0x24); encode_u32_leb128(*i, out); }

            Inst::I32Load(m) => { out.push(0x28); encode_memarg(m, out); }
            Inst::I32Load8U(m) => { out.push(0x2D); encode_memarg(m, out); }
            Inst::I64Load(m) => { out.push(0x29); encode_memarg(m, out); }
            Inst::F64Load(m) => { out.push(0x2B); encode_memarg(m, out); }
            Inst::I32Store(m) => { out.push(0x36); encode_memarg(m, out); }
            Inst::I64Store(m) => { out.push(0x37); encode_memarg(m, out); }
            Inst::MemorySize(idx) => { out.push(0x3F); encode_u32_leb128(*idx, out); }
            Inst::MemoryGrow(idx) => { out.push(0x40); encode_u32_leb128(*idx, out); }
            Inst::MemoryCopy { dst_mem, src_mem } => {
                out.push(0xFC);
                encode_u32_leb128(10, out); // memory.copy prefix
                encode_u32_leb128(*dst_mem, out);
                encode_u32_leb128(*src_mem, out);
            }

            Inst::I32Const(v) => { out.push(0x41); encode_i32_leb128(*v, out); }
            Inst::I64Const(v) => { out.push(0x42); encode_i64_leb128(*v, out); }
            Inst::F64Const(v) => { out.push(0x44); out.extend_from_slice(&v.to_le_bytes()); }

            // i32
            Inst::I32Eqz => out.push(0x45),
            Inst::I32Eq => out.push(0x46),
            Inst::I32Ne => out.push(0x47),
            Inst::I32LtS => out.push(0x48),
            Inst::I32LtU => out.push(0x49),
            Inst::I32GtU => out.push(0x4B),
            Inst::I32GeU => out.push(0x4E),
            Inst::I32Add => out.push(0x6A),
            Inst::I32Mul => out.push(0x6C),
            Inst::I32And => out.push(0x71),
            Inst::I32Or => out.push(0x72),
            Inst::I32Shl => out.push(0x74),
            Inst::I32WrapI64 => out.push(0xA7),

            // i64
            Inst::I64Eqz => out.push(0x50),
            Inst::I64Eq => out.push(0x51),
            Inst::I64Ne => out.push(0x52),
            Inst::I64LtS => out.push(0x53),
            Inst::I64GtS => out.push(0x55),
            Inst::I64LeS => out.push(0x57),
            Inst::I64GeS => out.push(0x59),
            Inst::I64GeU => out.push(0x5A),
            Inst::I64Add => out.push(0x7C),
            Inst::I64Sub => out.push(0x7D),
            Inst::I64Mul => out.push(0x7E),
            Inst::I64DivS => out.push(0x7F),
            Inst::I64RemS => out.push(0x81),
            Inst::I64And => out.push(0x83),
            Inst::I64Or => out.push(0x84),
            Inst::I64Xor => out.push(0x85),
            Inst::I64Shl => out.push(0x86),
            Inst::I64ShrS => out.push(0x87),
            Inst::I64ShrU => out.push(0x88),
            Inst::I64ExtendI32U => out.push(0xAD),

            // f64
            Inst::F64Eq => out.push(0x61),
            Inst::F64Ne => out.push(0x62),
            Inst::F64Lt => out.push(0x63),
            Inst::F64Gt => out.push(0x64),
            Inst::F64Le => out.push(0x65),
            Inst::F64Ge => out.push(0x66),
            Inst::F64Abs => out.push(0x99),
            Inst::F64Neg => out.push(0x9A),
            Inst::F64Ceil => out.push(0x9B),
            Inst::F64Floor => out.push(0x9C),
            Inst::F64Trunc => out.push(0x9D),
            Inst::F64Sqrt => out.push(0x9F),
            Inst::F64Add => out.push(0xA0),
            Inst::F64Sub => out.push(0xA1),
            Inst::F64Mul => out.push(0xA2),
            Inst::F64Div => out.push(0xA3),

            Inst::I64TruncF64S => out.push(0xB0),
            Inst::F64ConvertI64S => out.push(0xB9),
            Inst::I64ReinterpretF64 => out.push(0xBD),
            Inst::F64ReinterpretI64 => out.push(0xBF),
        }
    }
}

fn encode_block_type(bt: &BlockType, out: &mut Vec<u8>) {
    match bt {
        BlockType::Empty => out.push(0x40),
        BlockType::Result(vt) => out.push(vt.encode()),
    }
}

fn encode_memarg(m: &MemArg, out: &mut Vec<u8>) {
    encode_u32_leb128(m.align, out);
    encode_u32_leb128(m.offset as u32, out);
}

// ── Function builder ────────────────────────────────────────────────────

/// A WASM function body being built.
pub struct Function {
    locals: Vec<(u32, ValType)>,
    body: Vec<u8>,
}

impl Function {
    pub fn new(locals: Vec<(u32, ValType)>) -> Self {
        Function { locals, body: Vec::new() }
    }

    pub fn instruction(&mut self, inst: &Inst) -> &mut Self {
        inst.encode(&mut self.body);
        self
    }

    /// Encode the complete function body (locals + code + end).
    fn encode(&self) -> Vec<u8> {
        let mut func_body = Vec::new();

        encode_vec_len(self.locals.len(), &mut func_body);
        for (count, vt) in &self.locals {
            encode_u32_leb128(*count, &mut func_body);
            func_body.push(vt.encode());
        }

        // Code body (already includes End instruction emitted by caller)
        func_body.extend_from_slice(&self.body);

        // Wrap with size prefix
        let mut out = Vec::new();
        encode_vec_len(func_body.len(), &mut out);
        out.extend_from_slice(&func_body);
        out
    }
}

// ── Section builders ────────────────────────────────────────────────────

pub struct TypeSection { data: Vec<u8>, count: u32 }
pub struct ImportSection { data: Vec<u8>, count: u32 }
pub struct FunctionSection { data: Vec<u8>, count: u32 }
pub struct TableSection { data: Vec<u8>, count: u32 }
pub struct MemorySection { data: Vec<u8>, count: u32 }
pub struct GlobalSection { data: Vec<u8>, count: u32 }
pub struct ExportSection { data: Vec<u8>, count: u32 }
pub struct ElementSection { data: Vec<u8>, count: u32 }
pub struct CodeSection { data: Vec<u8>, count: u32 }
pub struct DataSection { data: Vec<u8>, count: u32 }
pub struct NameSection { data: Vec<u8> }
pub struct NameMap { data: Vec<u8>, count: u32 }

impl TypeSection {
    pub fn new() -> Self { TypeSection { data: Vec::new(), count: 0 } }
    pub fn ty(&mut self) -> TypeBuilder<'_> { TypeBuilder { section: self } }
}

pub struct TypeBuilder<'a> { section: &'a mut TypeSection }

impl<'a> TypeBuilder<'a> {
    pub fn function(&mut self, params: Vec<ValType>, results: Vec<ValType>) {
        self.section.data.push(0x60); // functype tag
        encode_vec_len(params.len(), &mut self.section.data);
        for p in &params { self.section.data.push(p.encode()); }
        encode_vec_len(results.len(), &mut self.section.data);
        for r in &results { self.section.data.push(r.encode()); }
        self.section.count += 1;
    }
}

impl ImportSection {
    pub fn new() -> Self { ImportSection { data: Vec::new(), count: 0 } }
    pub fn import(&mut self, module: &str, name: &str, ty: EntityType) {
        encode_name(module, &mut self.data);
        encode_name(name, &mut self.data);
        match ty {
            EntityType::Function(idx) => {
                self.data.push(0x00); // func import
                encode_u32_leb128(idx, &mut self.data);
            }
        }
        self.count += 1;
    }
}

impl FunctionSection {
    pub fn new() -> Self { FunctionSection { data: Vec::new(), count: 0 } }
    pub fn function(&mut self, type_idx: u32) {
        encode_u32_leb128(type_idx, &mut self.data);
        self.count += 1;
    }
}

impl TableSection {
    pub fn new() -> Self { TableSection { data: Vec::new(), count: 0 } }
    pub fn table(&mut self, tt: TableType) {
        self.data.push(tt.element_type.encode());
        encode_limits(tt.minimum, tt.maximum, &mut self.data);
        self.count += 1;
    }
}

impl MemorySection {
    pub fn new() -> Self { MemorySection { data: Vec::new(), count: 0 } }
    pub fn memory(&mut self, mt: MemoryType) {
        encode_limits(mt.minimum, mt.maximum, &mut self.data);
        self.count += 1;
    }
}

impl GlobalSection {
    pub fn new() -> Self { GlobalSection { data: Vec::new(), count: 0 } }
    pub fn global(&mut self, gt: GlobalType, init: &[u8]) {
        self.data.push(gt.val_type.encode());
        self.data.push(if gt.mutable { 0x01 } else { 0x00 });
        self.data.extend_from_slice(init);
        self.count += 1;
    }
}

impl ExportSection {
    pub fn new() -> Self { ExportSection { data: Vec::new(), count: 0 } }
    pub fn export(&mut self, name: &str, kind: ExportKind, idx: u32) {
        encode_name(name, &mut self.data);
        self.data.push(match kind {
            ExportKind::Func => 0x00,
            ExportKind::Table => 0x01,
            ExportKind::Memory => 0x02,
            ExportKind::Global => 0x03,
        });
        encode_u32_leb128(idx, &mut self.data);
        self.count += 1;
    }
}

impl ElementSection {
    pub fn new() -> Self { ElementSection { data: Vec::new(), count: 0 } }
    pub fn active(&mut self, table: Option<u32>, offset_expr: &[u8], funcs: Elements) {
        // Active element segment, table 0, funcref
        let _table_idx = table.unwrap_or(0);
        self.data.push(0x00); // flags: active, table 0, funcrefs
        self.data.extend_from_slice(offset_expr);
        match funcs {
            Elements::Functions(indices) => {
                encode_vec_len(indices.len(), &mut self.data);
                for idx in &indices {
                    encode_u32_leb128(*idx, &mut self.data);
                }
            }
        }
        self.count += 1;
    }
}

pub enum Elements {
    Functions(Vec<u32>),
}

impl CodeSection {
    pub fn new() -> Self { CodeSection { data: Vec::new(), count: 0 } }
    pub fn function(&mut self, func: &Function) {
        self.data.extend_from_slice(&func.encode());
        self.count += 1;
    }
}

impl DataSection {
    pub fn new() -> Self { DataSection { data: Vec::new(), count: 0 } }
    pub fn active(&mut self, offset_expr: &[u8], bytes: impl IntoIterator<Item = u8>) {
        let data: Vec<u8> = bytes.into_iter().collect();
        self.data.push(0x00); // flags: active, memory 0
        self.data.extend_from_slice(offset_expr);
        encode_vec_len(data.len(), &mut self.data);
        self.data.extend_from_slice(&data);
        self.count += 1;
    }
}

impl NameSection {
    pub fn new() -> Self { NameSection { data: Vec::new() } }
    pub fn functions(&mut self, names: &NameMap) {
        self.data.push(0x01); // function names subsection
        let mut subsection = Vec::new();
        encode_vec_len(names.count as usize, &mut subsection);
        subsection.extend_from_slice(&names.data);
        encode_vec_len(subsection.len(), &mut self.data);
        self.data.extend_from_slice(&subsection);
    }
}

impl NameMap {
    pub fn new() -> Self { NameMap { data: Vec::new(), count: 0 } }
    pub fn append(&mut self, idx: u32, name: &str) {
        encode_u32_leb128(idx, &mut self.data);
        encode_name(name, &mut self.data);
        self.count += 1;
    }
}

fn encode_limits(min: u64, max: Option<u64>, out: &mut Vec<u8>) {
    if let Some(max) = max {
        out.push(0x01); // has maximum
        encode_u32_leb128(min as u32, out);
        encode_u32_leb128(max as u32, out);
    } else {
        out.push(0x00); // no maximum
        encode_u32_leb128(min as u32, out);
    }
}

// ── Module builder ──────────────────────────────────────────────────────

/// A WASM module being built.
pub struct Module {
    sections: Vec<Vec<u8>>,
}

impl Module {
    pub fn new() -> Self {
        Module { sections: Vec::new() }
    }

    fn add_section(&mut self, section_id: u8, count: u32, data: &[u8]) {
        let mut section = Vec::new();
        section.push(section_id);
        // Section content = count + data
        let mut content = Vec::new();
        encode_vec_len(count as usize, &mut content);
        content.extend_from_slice(data);
        encode_vec_len(content.len(), &mut section);
        section.extend_from_slice(&content);
        self.sections.push(section);
    }

    fn add_section_raw(&mut self, section_id: u8, data: &[u8]) {
        let mut section = Vec::new();
        section.push(section_id);
        encode_vec_len(data.len(), &mut section);
        section.extend_from_slice(data);
        self.sections.push(section);
    }

    pub fn section(&mut self, s: &dyn Section) {
        s.write_to(self);
    }

    pub fn finish(self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\0asm");     // magic
        out.extend_from_slice(&1u32.to_le_bytes()); // version
        for section in &self.sections {
            out.extend_from_slice(section);
        }
        out
    }
}

pub trait Section {
    fn write_to(&self, module: &mut Module);
}

impl Section for TypeSection {
    fn write_to(&self, module: &mut Module) { module.add_section(1, self.count, &self.data); }
}
impl Section for ImportSection {
    fn write_to(&self, module: &mut Module) { module.add_section(2, self.count, &self.data); }
}
impl Section for FunctionSection {
    fn write_to(&self, module: &mut Module) { module.add_section(3, self.count, &self.data); }
}
impl Section for TableSection {
    fn write_to(&self, module: &mut Module) { module.add_section(4, self.count, &self.data); }
}
impl Section for MemorySection {
    fn write_to(&self, module: &mut Module) { module.add_section(5, self.count, &self.data); }
}
impl Section for GlobalSection {
    fn write_to(&self, module: &mut Module) { module.add_section(6, self.count, &self.data); }
}
impl Section for ExportSection {
    fn write_to(&self, module: &mut Module) { module.add_section(7, self.count, &self.data); }
}
impl Section for ElementSection {
    fn write_to(&self, module: &mut Module) { module.add_section(9, self.count, &self.data); }
}
impl Section for CodeSection {
    fn write_to(&self, module: &mut Module) { module.add_section(10, self.count, &self.data); }
}
impl Section for DataSection {
    fn write_to(&self, module: &mut Module) { module.add_section(11, self.count, &self.data); }
}
impl Section for NameSection {
    fn write_to(&self, module: &mut Module) {
        // Name section is a custom section (id 0) with name "name"
        let mut content = Vec::new();
        encode_name("name", &mut content);
        content.extend_from_slice(&self.data);
        module.add_section_raw(0, &content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leb128_u32_encoding() {
        let mut out = Vec::new();
        encode_u32_leb128(0, &mut out);
        assert_eq!(out, vec![0x00]);

        out.clear();
        encode_u32_leb128(127, &mut out);
        assert_eq!(out, vec![0x7F]);

        out.clear();
        encode_u32_leb128(128, &mut out);
        assert_eq!(out, vec![0x80, 0x01]);

        out.clear();
        encode_u32_leb128(624485, &mut out);
        assert_eq!(out, vec![0xE5, 0x8E, 0x26]);
    }

    #[test]
    fn leb128_i32_encoding() {
        let mut out = Vec::new();
        encode_i32_leb128(-1, &mut out);
        assert_eq!(out, vec![0x7F]);

        out.clear();
        encode_i32_leb128(0, &mut out);
        assert_eq!(out, vec![0x00]);

        out.clear();
        encode_i32_leb128(-128, &mut out);
        assert_eq!(out, vec![0x80, 0x7F]);
    }

    #[test]
    fn leb128_i64_encoding() {
        let mut out = Vec::new();
        encode_i64_leb128(0, &mut out);
        assert_eq!(out, vec![0x00]);

        out.clear();
        encode_i64_leb128(-1, &mut out);
        assert_eq!(out, vec![0x7F]);

        out.clear();
        encode_i64_leb128(42, &mut out);
        assert_eq!(out, vec![42]);
    }

    #[test]
    fn empty_module_produces_valid_wasm() {
        let module = Module::new();
        let bytes = module.finish();
        assert_eq!(&bytes[0..4], b"\0asm");
        assert_eq!(&bytes[4..8], &[1, 0, 0, 0]);
        crate::util::validate_wasm(&bytes).unwrap();
    }

    #[test]
    fn module_with_type_section() {
        let mut module = Module::new();
        let mut types = TypeSection::new();
        types.ty().function(vec![ValType::I64], vec![ValType::I64]);
        module.section(&types);
        let bytes = module.finish();
        assert_eq!(&bytes[0..4], b"\0asm");
        crate::util::validate_wasm(&bytes).unwrap();
    }

    #[test]
    fn const_expr_i32() {
        let expr = ConstExpr::i32_const(42);
        assert_eq!(expr, vec![0x41, 42, 0x0B]);
    }

    #[test]
    fn const_expr_i32_negative() {
        let expr = ConstExpr::i32_const(-1);
        assert_eq!(expr, vec![0x41, 0x7F, 0x0B]);
    }

    #[test]
    fn instruction_encoding_basic() {
        let mut out = Vec::new();
        Inst::Nop.encode(&mut out);
        assert_eq!(out, vec![0x01]);

        out.clear();
        Inst::Unreachable.encode(&mut out);
        assert_eq!(out, vec![0x00]);

        out.clear();
        Inst::Return.encode(&mut out);
        assert_eq!(out, vec![0x0F]);

        out.clear();
        Inst::Drop.encode(&mut out);
        assert_eq!(out, vec![0x1A]);
    }

    #[test]
    fn instruction_encoding_i64_const() {
        let mut out = Vec::new();
        Inst::I64Const(42).encode(&mut out);
        assert_eq!(out, vec![0x42, 42]);
    }

    #[test]
    fn instruction_encoding_i32_const() {
        let mut out = Vec::new();
        Inst::I32Const(100).encode(&mut out);
        assert_eq!(out[0], 0x41);
    }

    #[test]
    fn instruction_encoding_f64_const() {
        let mut out = Vec::new();
        Inst::F64Const(1.0).encode(&mut out);
        assert_eq!(out[0], 0x44);
        assert_eq!(out.len(), 9); // opcode + 8 bytes
        assert_eq!(f64::from_le_bytes(out[1..9].try_into().unwrap()), 1.0);
    }

    #[test]
    fn function_body_encoding() {
        let mut func = Function::new(vec![(1, ValType::I64)]);
        func.instruction(&Inst::LocalGet(0));
        func.instruction(&Inst::Return);
        func.instruction(&Inst::End);
        let encoded = func.encode();
        assert!(!encoded.is_empty());
        // First byte(s) are the size prefix (LEB128)
        // Then locals count, then code, then no extra end (caller provides end)
    }

    #[test]
    fn full_module_with_function() {
        let mut module = Module::new();

        // Type: () -> i64
        let mut types = TypeSection::new();
        types.ty().function(vec![], vec![ValType::I64]);
        module.section(&types);

        // Function: type index 0
        let mut functions = FunctionSection::new();
        functions.function(0);
        module.section(&functions);

        // Export: "main" = func 0
        let mut exports = ExportSection::new();
        exports.export("main", ExportKind::Func, 0);
        module.section(&exports);

        // Code: return 42
        let mut code = CodeSection::new();
        let mut func = Function::new(vec![]);
        func.instruction(&Inst::I64Const(42));
        func.instruction(&Inst::End);
        code.function(&func);
        module.section(&code);

        let bytes = module.finish();
        crate::util::validate_wasm(&bytes).unwrap();
    }

    #[test]
    fn memory_section_encoding() {
        let mut module = Module::new();
        let mut mem = MemorySection::new();
        mem.memory(MemoryType { minimum: 1, maximum: Some(16), memory64: false, shared: false, page_size_log2: None });
        module.section(&mem);
        let bytes = module.finish();
        crate::util::validate_wasm(&bytes).unwrap();
    }

    #[test]
    fn global_section_encoding() {
        let mut module = Module::new();
        let mut globals = GlobalSection::new();
        globals.global(
            GlobalType { val_type: ValType::I32, mutable: true, shared: false },
            &ConstExpr::i32_const(0),
        );
        module.section(&globals);
        let bytes = module.finish();
        crate::util::validate_wasm(&bytes).unwrap();
    }

    #[test]
    fn data_section_encoding() {
        let mut module = Module::new();

        let mut mem = MemorySection::new();
        mem.memory(MemoryType { minimum: 1, maximum: None, memory64: false, shared: false, page_size_log2: None });
        module.section(&mem);

        let mut data = DataSection::new();
        data.active(&ConstExpr::i32_const(0), b"hello".iter().copied());
        module.section(&data);

        let bytes = module.finish();
        crate::util::validate_wasm(&bytes).unwrap();
    }

    #[test]
    fn name_section_encoding() {
        let mut module = Module::new();
        let mut names = NameSection::new();
        let mut func_names = NameMap::new();
        func_names.append(0, "main");
        func_names.append(1, "helper");
        names.functions(&func_names);
        module.section(&names);
        let bytes = module.finish();
        // Name section is custom (id 0), validation should pass
        crate::util::validate_wasm(&bytes).unwrap();
    }
}
