//! Stack-based intermediate representation for MAGI compilation.
//!
//! The IR is a sequence of instructions that operate on an implicit value stack,
//! mapping naturally to WebAssembly's stack machine model.

use serde::{Deserialize, Serialize};

/// A compiled MAGI module ready for WASM code generation.
#[derive(Debug, Clone)]
pub struct IrModule {
    /// All functions (including `__main` for top-level code).
    pub functions: Vec<IrFunction>,
    /// String constants pool.
    pub strings: Vec<String>,
    /// Global variable declarations.
    pub globals: Vec<IrGlobal>,
}

impl IrModule {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            strings: Vec::new(),
            globals: Vec::new(),
        }
    }

    /// Intern a string constant, returning its index.
    pub fn intern_string(&mut self, s: &str) -> u32 {
        if let Some(idx) = self.strings.iter().position(|x| x == s) {
            return idx as u32;
        }
        let idx = self.strings.len() as u32;
        self.strings.push(s.to_string());
        idx
    }
}

/// A compiled function.
#[derive(Debug, Clone)]
pub struct IrFunction {
    /// Function name (e.g., `__main`, `add`, `__lambda_0`).
    pub name: String,
    /// Parameter count.
    pub param_count: u32,
    /// Whether this function has a rest parameter.
    pub has_rest: bool,
    /// Local variable slots (params + locals).
    pub locals: Vec<IrLocal>,
    /// The instruction sequence.
    pub instructions: Vec<Instruction>,
    /// Whether this function is exported.
    pub exported: bool,
    /// Return type hint.
    pub return_type: ValType,
}

/// A local variable slot.
#[derive(Debug, Clone)]
pub struct IrLocal {
    pub name: String,
    pub val_type: ValType,
    pub mutable: bool,
}

/// A global variable.
#[derive(Debug, Clone)]
pub struct IrGlobal {
    pub name: String,
    pub val_type: ValType,
    pub mutable: bool,
    pub init: Vec<Instruction>,
}

/// Value types in the IR (maps to WASM value types + tagged union at runtime).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValType {
    /// 64-bit integer (also used for booleans, null sentinel).
    I64,
    /// 64-bit float.
    F64,
    /// 32-bit integer (used for i32 values and some control flow).
    I32,
    /// 32-bit float.
    F32,
    /// A tagged value — a 64-bit integer encoding type tag + payload.
    /// This is the primary runtime representation for dynamic MAGI values.
    Tagged,
}

/// Stack-based instructions for the MAGI IR.
///
/// The IR uses a tagged value representation where each value is a 64-bit integer:
/// - Bits 56-63: type tag (0=null, 1=bool, 2=i64, 3=f64, 4=string_ref, 5=array_ref, 6=map_ref)
/// - Bits 0-55: payload (value or heap pointer)
///
/// For numeric-heavy code, the compiler can use unboxed I64/F64 locals and
/// only box when needed (e.g., storing in arrays).
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    // ── Constants ──────────────────────────────────────────────
    /// Push a null value.
    PushNull,
    /// Push a boolean value.
    PushBool(bool),
    /// Push a 64-bit integer.
    PushI64(i64),
    /// Push a 64-bit float.
    PushF64(f64),
    /// Push a 32-bit integer.
    PushI32(i32),
    /// Push a 32-bit float.
    PushF32(f32),
    /// Push a string constant by index into the string pool.
    PushString(u32),

    // ── Locals & Globals ──────────────────────────────────────
    /// Load a local variable onto the stack.
    LocalGet(u32),
    /// Store the top of stack into a local variable.
    LocalSet(u32),
    /// Copy top of stack into a local without popping.
    LocalTee(u32),
    /// Load a global variable.
    GlobalGet(u32),
    /// Store into a global variable.
    GlobalSet(u32),

    // ── Arithmetic (i64) ──────────────────────────────────────
    I64Add,
    I64Sub,
    I64Mul,
    I64Div,
    I64Rem,
    I64Neg,

    // ── Arithmetic (f64) ──────────────────────────────────────
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64Neg,
    F64Sqrt,
    F64Floor,
    F64Ceil,
    F64Abs,

    // ── Comparison (i64) ──────────────────────────────────────
    I64Eq,
    I64Ne,
    I64Lt,
    I64Gt,
    I64Le,
    I64Ge,

    // ── Comparison (f64) ──────────────────────────────────────
    F64Eq,
    F64Ne,
    F64Lt,
    F64Gt,
    F64Le,
    F64Ge,

    // ── Logical ───────────────────────────────────────────────
    /// Boolean NOT (i64: 0 → 1, nonzero → 0).
    BoolNot,

    // ── Conversions ───────────────────────────────────────────
    /// Convert i64 to f64.
    I64ToF64,
    /// Convert f64 to i64 (truncation).
    F64ToI64,

    // ── Tagged value operations ───────────────────────────────
    /// Box an i64 into a tagged value.
    TagI64,
    /// Box an f64 into a tagged value (reinterpret bits).
    TagF64,
    /// Box a bool into a tagged value.
    TagBool,
    /// Tag a string reference.
    TagString,
    /// Unbox a tagged value to i64 (runtime type check).
    UntagI64,
    /// Unbox a tagged value to f64.
    UntagF64,
    /// Unbox a tagged value to bool.
    UntagBool,
    /// Get the type tag of a tagged value (returns i32).
    GetTag,

    // ── Control flow ──────────────────────────────────────────
    /// Unconditional branch to label.
    Br(u32),
    /// Branch if top of stack is nonzero (truthy).
    BrIf(u32),
    /// Branch table (switch).
    BrTable(Vec<u32>, u32),
    /// Start a block (label for forward branches).
    Block,
    /// Start a loop (label for backward branches).
    Loop,
    /// End of block/loop/if.
    End,
    /// If-else. Pops condition from stack.
    If,
    /// Else branch.
    Else,
    /// Return from function.
    Return,
    /// Unreachable trap.
    Unreachable,
    /// No operation.
    Nop,
    /// Drop top of stack.
    Drop,

    // ── Function calls ────────────────────────────────────────
    /// Call a function by index.
    Call(u32),
    /// Call indirect (function pointer on stack).
    CallIndirect(u32),

    // ── Memory (linear memory for heap-allocated objects) ─────
    /// Allocate n bytes on the heap, push pointer.
    HeapAlloc(u32),
    /// Load i64 from memory at address on stack.
    MemLoadI64,
    /// Store i64 to memory (address, value on stack).
    MemStoreI64,
    /// Load f64 from memory.
    MemLoadF64,
    /// Store f64 to memory.
    MemStoreF64,
    /// Load i32 from memory.
    MemLoadI32,
    /// Store i32 to memory.
    MemStoreI32,

    // ── Runtime support calls ─────────────────────────────────
    /// Create an array from top N values on stack.
    ArrayNew(u32),
    /// Get array element (array_ref, index on stack).
    ArrayGet,
    /// Set array element (array_ref, index, value on stack).
    ArraySet,
    /// Get array length.
    ArrayLen,
    /// Create a map (pairs of key, value on stack).
    MapNew(u32),
    /// Get map value by key.
    MapGet,
    /// Set map key-value.
    MapSet,
    /// String concatenation.
    StringConcat,
    /// String length.
    StringLen,
    /// Print/output a value (calls imported host function).
    Print,
    /// Call a runtime built-in by name index (string pool index).
    RuntimeCall { name: u32, arg_count: u32 },
}

/// Type tags for the tagged value representation.
pub mod tag {
    pub const NULL: u8 = 0;
    pub const BOOL: u8 = 1;
    pub const I64: u8 = 2;
    pub const F64: u8 = 3;
    pub const STRING: u8 = 4;
    pub const ARRAY: u8 = 5;
    pub const MAP: u8 = 6;
    pub const I32: u8 = 7;
    pub const F32: u8 = 8;
}
