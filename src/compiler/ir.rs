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
    /// HashMap index for O(1) string deduplication during interning.
    #[doc(hidden)]
    pub string_index: std::collections::HashMap<String, u32>,
}

impl Default for IrModule {
    fn default() -> Self {
        Self::new()
    }
}

impl IrModule {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            strings: Vec::new(),
            globals: Vec::new(),
            string_index: std::collections::HashMap::new(),
        }
    }

    /// Intern a string constant, returning its index.
    /// Uses a HashMap for O(1) deduplication instead of linear scan.
    pub fn intern_string(&mut self, s: &str) -> u32 {
        if let Some(&idx) = self.string_index.get(s) {
            return idx;
        }
        let idx = self.strings.len() as u32;
        self.string_index.insert(s.to_string(), idx);
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
/// The IR uses a NaN-boxing tagged value representation where each value is a 64-bit integer:
/// - Float64 values are stored as raw IEEE 754 bits (unmodified)
/// - Non-float values are stored in the quiet NaN space: `0xFFF8 | (tag << 48) | payload`
///   where tag is 3 bits (0-7) and payload is 48 bits
///
/// See `tag` module for full encoding details.
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
    /// If-else (value-producing). Pops condition from stack.
    /// Emits a WASM `if` block with `Result(I64)` — MUST have a matching `Else`.
    If,
    /// If (void/statement context). Pops condition from stack.
    /// Emits a WASM `if` block with `Empty` type — no value produced, `Else` optional.
    IfVoid,
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

/// Type tags for the NaN-boxing tagged value representation.
///
/// ## NaN-Boxing Scheme
///
/// All values are represented as a single i64 (reinterpreted as f64 bits):
///
/// - **Float64**: Stored as raw IEEE 754 f64 bits, completely unmodified.
///   Any i64 that is NOT a quiet NaN with our tag signature is a float.
///   Real NaN values are canonicalized to `CANON_NAN`.
///
/// - **Non-float values**: Encoded using the quiet NaN space:
///   `0xFFF8_TTTT_PPPP_PPPP` where:
///   - Bits 63-51: `0xFFF8 >> 1` = quiet NaN prefix (all exponent + quiet bit)
///   - Bits 50-48: Type tag (3 bits, values 0-7)
///   - Bits 47-0: Payload (48 bits — pointers, small integers, booleans)
///
/// Detection: `(val & NANBOX_MASK) == NANBOX_SIG` → tagged non-float.
///
/// ## Tag Values
///
/// Tags are stored in bits 50-48 of the NaN payload.
pub mod tag {
    /// Canonical NaN used for real NaN values (quiet NaN with zero payload).
    pub const CANON_NAN: i64 = 0x7FF8_0000_0000_0000_u64 as i64;

    /// Mask to detect NaN-boxed values: check bits 63-51 (sign=1, exp=all-1, quiet=1).
    pub const NANBOX_MASK: i64 = 0xFFF8_0000_0000_0000_u64 as i64;
    /// Signature for NaN-boxed values (negative quiet NaN space).
    pub const NANBOX_SIG: i64 = 0xFFF8_0000_0000_0000_u64 as i64;

    /// Mask to extract 48-bit payload.
    pub const PAYLOAD_MASK: i64 = 0x0000_FFFF_FFFF_FFFF_u64 as i64;
    /// Mask to extract 3-bit tag from bits 50-48.
    pub const TAG_MASK: i64 = 0x0007_0000_0000_0000_u64 as i64;
    /// Bit shift for tag extraction (bits 48-50).
    pub const TAG_SHIFT: i64 = 48;

    // Tag values (3 bits: 0-7) for NaN-boxed types
    pub const NULL: u8 = 0;
    pub const BOOL: u8 = 1;
    pub const I64: u8 = 2;
    pub const STRING: u8 = 3;
    pub const ARRAY: u8 = 4;
    pub const MAP: u8 = 5;
    pub const I32: u8 = 6;
    pub const F32: u8 = 7;
    /// Sentinel tag for Float64. F64 values are NOT NaN-boxed (stored as raw bits),
    /// but GetTag returns this sentinel (8) when it detects a non-NaN-boxed value.
    /// This value is outside the 3-bit range (0-7) so it never collides with real tags.
    pub const F64: u8 = 8;

    /// Build a tagged value from tag and payload: `NANBOX_SIG | (tag << 48) | payload`.
    pub const fn encode(tag: u8, payload: i64) -> i64 {
        NANBOX_SIG | ((tag as i64) << TAG_SHIFT) | (payload & PAYLOAD_MASK)
    }
}
