//! AST interpreter for the MAGI v2 language.
//!
//! Walks the AST directly, executing statements with support for loops,
//! mutable variables, and an environment. Delegates operation evaluation
//! to the injected `OperationEvaluator` — no duplication.

use super::ast::*;
use crate::eval::{EvalError, OperationEvaluator};
use crate::ops::op_input_ports;
use crate::types::{DataType, FutureState, OperationType};

use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Static empty HashMap used as a default config parameter to avoid
/// allocating a new HashMap on every evaluator call.
static EMPTY_CONFIG: std::sync::LazyLock<HashMap<String, DataType>> =
    std::sync::LazyLock::new(HashMap::new);

// =============================================================================
// Task registry — global storage for spawned thread join handles
// =============================================================================

/// Maximum number of pending spawned tasks.
const MAX_TASKS: usize = 4096;

/// Global task registry: maps task IDs to join handles that produce
/// `Result<DataType, String>` (Ok = resolved value, Err = error message).
static TASK_REGISTRY: std::sync::LazyLock<
    Mutex<HashMap<String, std::thread::JoinHandle<Result<DataType, String>>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Atomic counter for generating unique task IDs.
static TASK_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Generate a unique task ID.
fn task_id() -> String {
    let n = TASK_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("task:{}", n)
}

/// Store a join handle in the task registry.
fn task_store(
    id: &str,
    handle: std::thread::JoinHandle<Result<DataType, String>>,
) -> Result<(), String> {
    let mut map = TASK_REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= MAX_TASKS {
        return Err(format!("task limit reached (max {})", MAX_TASKS));
    }
    map.insert(id.to_string(), handle);
    Ok(())
}

/// Join a task by ID: removes it from the registry, blocks until the thread
/// finishes, and returns the result. Returns `Err` if the task is not found
/// or the thread panicked.
fn task_join(id: &str) -> Result<Result<DataType, String>, String> {
    let handle = {
        let mut map = TASK_REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(id).ok_or_else(|| format!("task not found: {}", id))?
    };
    handle.join().map_err(|_| "spawned thread panicked".to_string())
}

// =============================================================================
// Channel registry — global storage for mpsc channel endpoints
// =============================================================================

/// Maximum number of open channels.
const MAX_CHANNELS: usize = 4096;

/// Atomic counter for generating unique channel IDs.
static CHANNEL_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A sender endpoint stored in the channel registry (unbounded).
struct ChannelSender {
    tx: std::sync::mpsc::Sender<DataType>,
}

/// A bounded sender endpoint stored in the channel registry.
struct ChannelSyncSender {
    tx: std::sync::mpsc::SyncSender<DataType>,
}

/// A receiver endpoint stored in the channel registry.
/// Uses Arc so we can clone the handle, drop the registry lock, then recv.
struct ChannelReceiver {
    rx: Arc<Mutex<std::sync::mpsc::Receiver<DataType>>>,
}

/// Global channel registry: sender and receiver handles keyed by channel ID.
static CHANNEL_REGISTRY: std::sync::LazyLock<
    Mutex<HashMap<String, Box<dyn std::any::Any + Send>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Generate a unique channel sender/receiver ID pair.
fn channel_ids() -> (String, String) {
    let n = CHANNEL_COUNTER.fetch_add(1, Ordering::Relaxed);
    (format!("chan-tx:{}", n), format!("chan-rx:{}", n))
}

/// Store a channel endpoint in the global registry.
fn channel_store<T: Send + 'static>(id: &str, endpoint: T) -> Result<(), String> {
    let mut map = CHANNEL_REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= MAX_CHANNELS {
        return Err(format!("channel limit reached (max {})", MAX_CHANNELS));
    }
    map.insert(id.to_string(), Box::new(endpoint));
    Ok(())
}

/// Remove a channel endpoint from the registry.
fn channel_remove(id: &str) -> Result<(), String> {
    let mut map = CHANNEL_REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    map.remove(id)
        .ok_or_else(|| format!("channel endpoint not found: {}", id))?;
    Ok(())
}

/// A lightweight evaluator used by spawned threads that handles only
/// interpreter-level constructs (arithmetic, etc. are already handled
/// inline by the interpreter). Std operations dispatched via
/// OperationType fall through to the injected evaluator; for spawned
/// threads we use a stub that returns an error for unknown ops.
struct SpawnEvaluator;

impl OperationEvaluator for SpawnEvaluator {
    fn eval_operation(
        &self,
        op: OperationType,
        inputs: &HashMap<String, DataType>,
        config: &HashMap<String, DataType>,
    ) -> Result<DataType, EvalError> {
        // Re-use the same full evaluator from the `magi` binary if available.
        // In library/test mode, fall back to a minimal stub that handles
        // common pure operations to keep spawned closures useful.
        spawn_eval_operation(op, inputs, config)
    }
}

/// Minimal operation evaluator for spawned threads. Handles arithmetic,
/// comparison, logical, and conversion operations that are commonly used
/// inside concurrent closures.
fn spawn_eval_operation(
    op: OperationType,
    inputs: &HashMap<String, DataType>,
    _config: &HashMap<String, DataType>,
) -> Result<DataType, EvalError> {
    let a = inputs.get("a").cloned().unwrap_or(DataType::Null);
    let b = inputs.get("b").cloned().unwrap_or(DataType::Null);
    let input = inputs
        .get("input")
        .or_else(|| inputs.get("value"))
        .cloned()
        .unwrap_or(DataType::Null);

    match op {
        // Arithmetic
        OperationType::Add => match (&a, &b) {
            (DataType::String(x), DataType::String(y)) => {
                Ok(DataType::String(format!("{}{}", x, y)))
            }
            _ => spawn_binop(&a, &b, i64::wrapping_add, |x, y| x + y),
        },
        OperationType::Subtract => spawn_binop(&a, &b, i64::wrapping_sub, |x, y| x - y),
        OperationType::Multiply => spawn_binop(&a, &b, i64::wrapping_mul, |x, y| x * y),
        OperationType::Divide => {
            let is_int_zero = matches!(
                (&a, &b),
                (DataType::Int64(_), DataType::Int64(0))
                    | (DataType::Int32(_), DataType::Int32(0))
                    | (DataType::Uint32(_), DataType::Uint32(0))
                    | (DataType::Uint64(_), DataType::Uint64(0))
            );
            if is_int_zero {
                Err(EvalError::DivisionByZero)
            } else {
                spawn_binop(
                    &a,
                    &b,
                    |x, y| if y == 0 { 0 } else { x.checked_div(y).unwrap_or(0) },
                    |x, y| x / y,
                )
            }
        }
        OperationType::Modulo => {
            if b.to_i64() == Some(0) {
                Err(EvalError::DivisionByZero)
            } else {
                spawn_binop(
                    &a,
                    &b,
                    |x, y| x.checked_rem(y).unwrap_or(0),
                    |x, y| x % y,
                )
            }
        }

        // Comparison
        OperationType::Equal => Ok(DataType::Bool(a == b)),
        OperationType::NotEqual => Ok(DataType::Bool(a != b)),
        OperationType::Greater => match (a.to_f64(), b.to_f64()) {
            (Some(x), Some(y)) => Ok(DataType::Bool(x > y)),
            _ => Ok(DataType::Bool(false)),
        },
        OperationType::Less => match (a.to_f64(), b.to_f64()) {
            (Some(x), Some(y)) => Ok(DataType::Bool(x < y)),
            _ => Ok(DataType::Bool(false)),
        },
        OperationType::GreaterEq => match (a.to_f64(), b.to_f64()) {
            (Some(x), Some(y)) => Ok(DataType::Bool(x >= y)),
            _ => Ok(DataType::Bool(false)),
        },
        OperationType::LessEq => match (a.to_f64(), b.to_f64()) {
            (Some(x), Some(y)) => Ok(DataType::Bool(x <= y)),
            _ => Ok(DataType::Bool(false)),
        },

        // Logic
        OperationType::And => match (&a, &b) {
            (DataType::Bool(x), DataType::Bool(y)) => Ok(DataType::Bool(*x && *y)),
            _ => Ok(DataType::Bool(false)),
        },
        OperationType::Or => match (&a, &b) {
            (DataType::Bool(x), DataType::Bool(y)) => Ok(DataType::Bool(*x || *y)),
            _ => Ok(DataType::Bool(false)),
        },
        OperationType::Not => match &input {
            DataType::Bool(x) => Ok(DataType::Bool(!x)),
            _ => Ok(DataType::Bool(true)),
        },
        OperationType::Negate => match &input {
            DataType::Int64(x) => Ok(DataType::Int64(x.wrapping_neg())),
            DataType::Float64(x) => Ok(DataType::Float64(-x)),
            _ => Ok(DataType::Null),
        },

        // Conversion
        OperationType::ToString => Ok(DataType::String(input.to_string_lossy())),

        // Unsupported
        _ => Err(EvalError::InvalidInput(format!(
            "{:?} is not available inside spawned tasks",
            op
        ))),
    }
}

/// Type-preserving binary op helper for the spawn evaluator.
fn spawn_binop(
    a: &DataType,
    b: &DataType,
    int_op: fn(i64, i64) -> i64,
    float_op: fn(f64, f64) -> f64,
) -> Result<DataType, EvalError> {
    match (a, b) {
        (DataType::Int32(x), DataType::Int32(y)) => {
            Ok(DataType::Int32(int_op(*x as i64, *y as i64) as i32))
        }
        (DataType::Uint32(x), DataType::Uint32(y)) => {
            Ok(DataType::Uint32(int_op(*x as i64, *y as i64) as u32))
        }
        (DataType::Uint64(x), DataType::Uint64(y)) => {
            Ok(DataType::Uint64(int_op(*x as i64, *y as i64) as u64))
        }
        (DataType::Float32(x), DataType::Float32(y)) => {
            Ok(DataType::Float32(float_op(*x as f64, *y as f64) as f32))
        }
        (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(int_op(*x, *y))),
        (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(float_op(*x, *y))),
        (DataType::Int64(x), DataType::Float64(y)) => {
            Ok(DataType::Float64(float_op(*x as f64, *y)))
        }
        (DataType::Float64(x), DataType::Int64(y)) => {
            Ok(DataType::Float64(float_op(*x, *y as f64)))
        }
        _ => match (a.to_i64(), b.to_i64()) {
            (Some(x), Some(y)) => Ok(DataType::Int64(int_op(x, y))),
            _ => match (a.to_f64(), b.to_f64()) {
                (Some(x), Some(y)) => Ok(DataType::Float64(float_op(x, y))),
                _ => Ok(DataType::Null),
            },
        },
    }
}

/// Maximum iterations for while/for loops to prevent infinite loops.
const MAX_LOOP_ITERATIONS: usize = 1_000_000_000; // 1 billion — effectively unlimited like Go/Rust

/// Maximum call depth for recursion guard.
const MAX_CALL_DEPTH: usize = 128; // Safe for interpreter stack frames in debug builds

/// GC trigger threshold: collect after this many allocations since last GC.
const GC_ALLOC_THRESHOLD: usize = 256;

/// Maximum output string length (10 MB).
const MAX_STRING_OUTPUT: usize = 100_000_000; // 100 MB — generous like Go/Rust

/// Maximum array element count.
const MAX_ARRAY_ELEMENTS: usize = 100_000_000; // 100 million — memory is the real limit

/// Maximum number of variables across all scopes (#262).
const MAX_VARIABLES: usize = 10_000_000; // 10 million — effectively unlimited

/// Maximum number of function definitions (#263).
const MAX_FUNCTIONS: usize = 1_000_000; // 1 million — effectively unlimited

/// Maximum expression nesting depth (#261).
const MAX_EXPR_DEPTH: usize = 1024; // Deep nesting like Go/Rust compilers handle

/// Maximum identifier name length (#404).
const MAX_IDENTIFIER_LEN: usize = 1024;

/// A resolved package with its functions pre-extracted
#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    /// Package ID
    pub id: String,
    /// Parsed function definitions from the package source
    pub functions: HashMap<String, FunctionDef>,
    /// Use statements that need to be executed to set up std aliases
    pub use_statements: Vec<Statement>,
    /// Enum definitions from the package source
    pub enum_defs: Vec<(String, Vec<super::ast::EnumVariant>)>,
    /// Struct definitions from the package source
    pub struct_defs: Vec<(String, Vec<super::ast::StructField>)>,
}

// =============================================================================
// Memory addressing constants
// =============================================================================

/// Memory address type.
pub type MemAddr = u64;

/// Heap base address (64KB offset, typical heap start).
pub const HEAP_BASE: u64 = 0x10000;

/// Memory alignment (8-byte aligned, 64-bit architecture).
const ALIGNMENT: u64 = 8;

// =============================================================================
// Log entry
// =============================================================================

/// A log entry captured during execution.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    pub line: Option<u32>,
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "debug"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Warn => write!(f, "warn"),
            LogLevel::Error => write!(f, "error"),
        }
    }
}

/// Statistics from a garbage collection cycle.
#[derive(Debug, Clone, Default)]
pub struct GcStats {
    pub cycles: usize,
    pub total_swept: usize,
    pub total_bytes_reclaimed: u64,
    pub peak_live: usize,
}

// =============================================================================
// Virtual Heap
// =============================================================================

/// Metadata for a heap allocation.
#[derive(Debug, Clone)]
struct AllocMeta {
    size: u64,
}

/// Virtual heap with bump allocation, free list, scope tracking, and refcounting.
#[derive(Clone)]
struct Heap {
    values: HashMap<MemAddr, DataType>,
    metadata: HashMap<MemAddr, AllocMeta>,
    next_addr: MemAddr,
    free_list: Vec<(MemAddr, u64)>,
    scope_allocations: Vec<Vec<MemAddr>>,
    allocs_since_gc: usize,
    gc_stats: GcStats,
}

impl Heap {
    fn new() -> Self {
        Self {
            values: HashMap::new(),
            metadata: HashMap::new(),
            next_addr: HEAP_BASE,
            free_list: Vec::new(),
            scope_allocations: vec![Vec::new()],
            allocs_since_gc: 0,
            gc_stats: GcStats::default(),
        }
    }

    /// Calculate allocation size for a DataType value.
    fn size_of(value: &DataType) -> u64 {
        let raw = match value {
            DataType::Null => 0,
            DataType::Bool(_) => 1,
            DataType::Int32(_) | DataType::Uint32(_) | DataType::Float32(_) => 4,
            DataType::Int64(_) | DataType::Uint64(_) | DataType::Float64(_) => 8,
            DataType::String(s) => 8 + s.len() as u64,
            DataType::Bytes(b) => 8 + b.len() as u64,
            DataType::Array(arr) => 8 + 8 * (arr.len() as u64),
            DataType::Map(map) => 8 + 16 * (map.len() as u64),
            DataType::Set(items) => 8 + 8 * (items.len() as u64),
            DataType::Tuple(items) => 8 + 8 * (items.len() as u64),
            DataType::Future(_) => 16,
        };
        // Align to ALIGNMENT
        raw.div_ceil(ALIGNMENT) * ALIGNMENT
    }

    /// Allocate a value on the heap. Returns the address.
    fn alloc(&mut self, value: DataType) -> MemAddr {
        let size = Self::size_of(&value).max(ALIGNMENT);

        // Try first-fit from free list
        let addr = if let Some(idx) = self.free_list.iter().position(|(_, s)| *s >= size) {
            let (addr, _) = self.free_list.remove(idx);
            addr
        } else {
            let addr = self.next_addr;
            self.next_addr = self.next_addr.saturating_add(size);
            addr
        };

        self.values.insert(addr, value);
        self.metadata.insert(addr, AllocMeta { size });

        // Track in current scope
        if let Some(scope) = self.scope_allocations.last_mut() {
            scope.push(addr);
        }

        self.allocs_since_gc += 1;
        addr
    }

    /// Read a value at an address.
    fn read(&self, addr: MemAddr) -> Option<&DataType> {
        self.values.get(&addr)
    }

    /// Write a value at an existing address (for mutation).
    fn write(&mut self, addr: MemAddr, value: DataType) {
        let new_size = Self::size_of(&value).max(ALIGNMENT);
        self.values.insert(addr, value);
        if let Some(meta) = self.metadata.get_mut(&addr) {
            meta.size = new_size;
        }
    }

    /// Push a new scope.
    fn push_scope(&mut self) {
        self.scope_allocations.push(Vec::new());
    }

    /// Sweep all heap entries not in the root set. Returns number of swept entries.
    fn collect_garbage(&mut self, roots: &std::collections::HashSet<MemAddr>) -> usize {
        let all_addrs: Vec<MemAddr> = self.values.keys().copied().collect();
        let mut swept_bytes = 0u64;
        let mut swept_count = 0usize;

        for addr in all_addrs {
            if !roots.contains(&addr) {
                if let Some(meta) = self.metadata.remove(&addr) {
                    swept_bytes += meta.size;
                    self.free_list.push((addr, meta.size));
                }
                self.values.remove(&addr);
                swept_count += 1;
            }
        }

        // Clean dead entries from scope_allocations
        for scope_addrs in &mut self.scope_allocations {
            scope_addrs.retain(|addr| roots.contains(addr));
        }

        self.allocs_since_gc = 0;
        self.gc_stats.cycles += 1;
        self.gc_stats.total_swept += swept_count;
        self.gc_stats.total_bytes_reclaimed += swept_bytes;
        let live = self.values.len();
        if live > self.gc_stats.peak_live {
            self.gc_stats.peak_live = live;
        }
        swept_count
    }

    /// Check if GC should run based on allocation count.
    fn should_gc(&self) -> bool {
        self.allocs_since_gc >= GC_ALLOC_THRESHOLD
    }

    /// Pop a scope, freeing all allocations made within it.
    fn pop_scope(&mut self) {
        if let Some(addrs) = self.scope_allocations.pop() {
            for addr in addrs {
                if let Some(meta) = self.metadata.remove(&addr) {
                    self.values.remove(&addr);
                    self.free_list.push((addr, meta.size));
                }
            }
        }
        // Compact the free list when it grows beyond a threshold to avoid
        // unbounded growth. Coalesce adjacent entries to reduce fragmentation.
        if self.free_list.len() > FREE_LIST_COMPACT_THRESHOLD {
            self.compact_free_list();
        }
    }

    /// Sort the free list by address and coalesce adjacent entries.
    /// Two entries (addr1, size1) and (addr2, size2) are adjacent when
    /// addr1 + size1 == addr2, and they merge into (addr1, size1 + size2).
    fn compact_free_list(&mut self) {
        if self.free_list.len() <= 1 {
            return;
        }
        self.free_list.sort_unstable_by_key(|(addr, _)| *addr);
        let old = std::mem::take(&mut self.free_list);
        let mut compacted: Vec<(MemAddr, u64)> = Vec::with_capacity(old.len());
        let mut iter = old.into_iter();
        if let Some(first) = iter.next() {
            let mut current = first;
            for (addr, size) in iter {
                if current.0 + current.1 == addr {
                    // Adjacent — merge
                    current.1 += size;
                } else {
                    compacted.push(current);
                    current = (addr, size);
                }
            }
            compacted.push(current);
        }
        self.free_list = compacted;
    }
}

/// Threshold above which the Heap free list is compacted after pop_scope.
const FREE_LIST_COMPACT_THRESHOLD: usize = 64;

/// Symbol table entry: maps a name to a heap address.
#[derive(Debug, Clone)]
struct SymbolEntry {
    addr: MemAddr,
    mutable: bool,
}

// =============================================================================
// Interpreter
// =============================================================================

/// AST interpreter that executes programs with loops and mutable variables.
/// Uses a virtual heap with memory addresses for value storage.
pub struct Interpreter<'a> {
    /// Operation evaluator (injected dependency).
    evaluator: &'a dyn OperationEvaluator,
    /// Virtual heap for value storage.
    heap: Heap,
    /// Symbol table stack (one scope per level).
    symbols: Vec<HashMap<String, SymbolEntry>>,
    /// Saved symbol stacks from enclosing function calls (for GC root scanning).
    saved_symbol_stacks: Vec<Vec<HashMap<String, SymbolEntry>>>,
    /// Imported plugin IDs.
    imports: std::collections::HashSet<String>,
    /// User-defined functions.
    functions: HashMap<String, FunctionDef>,
    /// Async function names (functions defined with `async fn`).
    async_fns: std::collections::HashSet<String>,
    /// Closure captures: maps lambda function names to captured variable values.
    closure_captures: HashMap<String, Vec<(String, DataType, bool)>>,
    /// Standard library operation aliases: local_name → operation_name.
    /// Populated by `use std::*` statements.
    std_op_aliases: HashMap<String, String>,
    /// Counter for generating unique lambda names.
    lambda_counter: usize,
    /// Current call depth for recursion guard.
    call_depth: usize,
    /// Captured log entries.
    pub logs: Vec<LogEntry>,
    /// Cancellation token.
    cancel: Option<Arc<AtomicBool>>,
    /// Optional debug state for breakpoint/step-through sessions.
    debug: Option<DebugState>,
    /// Installed packages available for `use pkg::name::function` imports.
    packages: HashMap<String, ResolvedPackage>,
    /// Enum definitions: name → variants.
    enum_defs: HashMap<String, Vec<EnumVariant>>,
    /// Struct definitions: name → fields.
    struct_defs: HashMap<String, Vec<StructField>>,
    impl_methods: HashMap<String, HashMap<String, FunctionDef>>,
    trait_defs: HashMap<String, Vec<TraitMethod>>,
    /// Package import guard: tracks packages currently being imported (circular import detection).
    importing_packages: std::collections::HashSet<String>,
    /// Source file name for error messages (#134).
    source_file: Option<String>,
    /// Runtime call stack for error diagnostics (#133).
    call_stack_names: Vec<String>,
    /// Stack of deferred expressions per scope level.
    deferred: Vec<Vec<Expression>>,
    /// Maximum errors to collect in keep-going mode.
    max_errors: Option<usize>,
    /// Errors collected during keep-going execution.
    pub collected_errors: Vec<InterpError>,
    /// Current expression nesting depth (#261).
    expr_depth: usize,
}

impl<'a> Interpreter<'a> {
    pub fn new(evaluator: &'a dyn OperationEvaluator) -> Self {
        Self {
            evaluator,
            heap: Heap::new(),
            symbols: vec![HashMap::new()],
            saved_symbol_stacks: Vec::new(),
            imports: std::collections::HashSet::new(),
            functions: HashMap::new(),
            async_fns: std::collections::HashSet::new(),
            closure_captures: HashMap::new(),
            std_op_aliases: HashMap::new(),
            lambda_counter: 0,
            call_depth: 0,
            logs: Vec::new(),
            cancel: None,
            debug: None,
            packages: HashMap::new(),
            enum_defs: HashMap::new(),
            struct_defs: HashMap::new(),
            impl_methods: HashMap::new(),
            trait_defs: HashMap::new(),
            source_file: None,
            call_stack_names: Vec::new(),
            importing_packages: std::collections::HashSet::new(),
            deferred: vec![Vec::new()],
            max_errors: None,
            collected_errors: Vec::new(),
            expr_depth: 0,
        }
    }

    /// Set max errors for keep-going mode.
    pub fn with_max_errors(mut self, max: usize) -> Self {
        self.max_errors = Some(max);
        self
    }

    /// Collect an error in keep-going mode. Returns true if execution should continue,
    /// false if the max_errors limit has been reached.
    /// `max_errors = Some(0)` means unlimited; `Some(n)` caps at n errors.
    fn collect_error_keep_going(&mut self, error: InterpError) -> bool {
        self.collected_errors.push(error);
        match self.max_errors {
            Some(0) => true, // unlimited
            Some(max) => self.collected_errors.len() < max,
            None => false, // should not happen but treat as immediate abort
        }
    }

    /// Add a resolved package to the interpreter.
    /// Packages are parsed from source into `FunctionDef`s and made available
    /// for `use pkg::name::function` imports.
    pub fn with_package(mut self, package: ResolvedPackage) -> Self {
        self.packages.insert(package.id.clone(), package);
        self
    }

    /// Add multiple resolved packages to the interpreter.
    pub fn with_packages(mut self, packages: Vec<ResolvedPackage>) -> Self {
        for pkg in packages {
            self.packages.insert(pkg.id.clone(), pkg);
        }
        self
    }

    /// Attach a debug state for breakpoint/step-through debugging.
    pub fn with_debug(mut self, debug: DebugState) -> Self {
        self.debug = Some(debug);
        self
    }

    /// Look up a variable name in the symbol table.
    fn lookup(&self, name: &str) -> Option<&SymbolEntry> {
        for scope in self.symbols.iter().rev() {
            if let Some(entry) = scope.get(name) {
                return Some(entry);
            }
        }
        None
    }

    /// Suggest a variable name using Levenshtein distance.
    fn suggest_variable(&self, name: &str) -> Option<String> {
        let mut seen = std::collections::HashSet::new();
        let refs: Vec<&str> = self.symbols.iter().rev()
            .flat_map(|scope| scope.keys())
            .filter(|k| seen.insert(k.as_str()))
            .map(|k| k.as_str())
            .collect();
        super::errors::suggest_name(name, &refs)
    }

    /// Suggest a function name using Levenshtein distance.
    fn suggest_function(&self, name: &str) -> Option<String> {
        let refs: Vec<&str> = self.functions.keys().map(|s| s.as_str()).collect();
        super::errors::suggest_name(name, &refs)
    }

    /// Define a variable in the current scope.
    fn define(&mut self, name: &str, addr: MemAddr, mutable: bool) {
        if let Some(scope) = self.symbols.last_mut() {
            scope.insert(name.to_string(), SymbolEntry { addr, mutable });
        }
    }

    /// Total variable count across all scopes (for resource limiting).
    fn total_variable_count(&self) -> usize {
        self.symbols.iter().map(|s| s.len()).sum()
    }

    /// Set the cancellation token for checking during loops.
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Set the source file name for error messages (#134).
    pub fn with_source_file(mut self, path: String) -> Self {
        self.source_file = Some(path);
        self
    }

    /// Get the current call stack as a formatted string (#133).
    pub fn call_stack_trace(&self) -> String {
        if self.call_stack_names.is_empty() {
            return String::new();
        }
        let mut trace = String::from("\nCall stack:\n");
        for (i, name) in self.call_stack_names.iter().rev().enumerate() {
            trace.push_str(&format!("  {}: {}()\n", i, name));
        }
        trace
    }

    fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }

    /// Snapshot all visible variables for the debug client.
    fn snapshot_variables(&self) -> Vec<DebugVariable> {
        let mut vars = Vec::new();
        for (depth, scope) in self.symbols.iter().enumerate() {
            let scope_label = if depth == 0 {
                "global".to_string()
            } else {
                format!("scope_{}", depth)
            };
            for (name, entry) in scope {
                let (value, type_name) = if let Some(val) = self.heap.read(entry.addr) {
                    (
                        datatype_to_display(val),
                        val.type_name().to_string(),
                    )
                } else {
                    ("<freed>".to_string(), "unknown".to_string())
                };
                vars.push(DebugVariable {
                    name: name.clone(),
                    value,
                    type_name,
                    scope: scope_label.clone(),
                });
            }
        }
        vars
    }

    /// Check if we should pause at the given span. If so, send a Paused event
    /// and block until a DebugCommand is received.
    fn debug_check(&mut self, span: Span) {
        let should_pause = if let Some(ref debug) = self.debug {
            match debug.step_mode {
                StepMode::Continue => debug.breakpoints.contains(&span.start_line),
                StepMode::StepInto => true,
                StepMode::StepOver => self.call_depth <= debug.step_start_depth,
                StepMode::StepOut => self.call_depth < debug.step_start_depth,
            }
        } else {
            return;
        };

        if !should_pause {
            return;
        }

        let variables = self.snapshot_variables();
        let call_stack = self
            .debug
            .as_ref()
            .map(|d| d.call_stack.clone())
            .unwrap_or_default();

        let event = DebugEvent::Paused {
            line: span.start_line,
            column: span.start_col,
            variables,
            call_stack,
        };

        // Send the paused event (blocking — runs on spawn_blocking thread)
        if let Some(ref debug) = self.debug {
            let _ = debug.event_sender.blocking_send(event);
        }

        // Wait for a command (loop to allow Evaluate without resuming execution)
        loop {
            let cmd = self
                .debug
                .as_mut()
                .and_then(|d| d.command_receiver.blocking_recv());
            let Some(cmd) = cmd else { break };
            match cmd {
                DebugCommand::Continue => {
                    if let Some(ref mut debug) = self.debug {
                        debug.step_mode = StepMode::Continue;
                    }
                    break;
                }
                DebugCommand::StepOver => {
                    if let Some(ref mut debug) = self.debug {
                        debug.step_mode = StepMode::StepOver;
                        debug.step_start_depth = self.call_depth;
                    }
                    break;
                }
                DebugCommand::StepInto => {
                    if let Some(ref mut debug) = self.debug {
                        debug.step_mode = StepMode::StepInto;
                    }
                    break;
                }
                DebugCommand::StepOut => {
                    if let Some(ref mut debug) = self.debug {
                        debug.step_mode = StepMode::StepOut;
                        debug.step_start_depth = self.call_depth;
                    }
                    break;
                }
                DebugCommand::Evaluate(expr) => {
                    // Parse and evaluate expression in current scope
                    // Save interpreter state to prevent permanent mutation from debug expressions
                    let saved_functions = self.functions.clone();
                    let saved_enums = self.enum_defs.clone();
                    let saved_structs = self.struct_defs.clone();
                    let saved_async_fns = self.async_fns.clone();
                    let saved_aliases = self.std_op_aliases.clone();
                    let saved_closures = self.closure_captures.clone();
                    let saved_lambda_counter = self.lambda_counter;
                    let saved_imports = self.imports.clone();
                    let saved_importing = self.importing_packages.clone();
                    let saved_logs = self.logs.clone();
                    let saved_call_depth = self.call_depth;
                    let saved_symbols = self.symbols.clone();
                    let saved_stacks = self.saved_symbol_stacks.clone();
                    // Save heap state to prevent leaking allocations from debug eval
                    let saved_next_addr = self.heap.next_addr;
                    let saved_free_list = self.heap.free_list.clone();
                    let saved_allocs = self.heap.allocs_since_gc;
                    self.heap.push_scope();
                    // Temporarily disable debug to avoid recursive blocking_recv
                    let debug_state = self.debug.take();
                    let (result, error) = match crate::syntax::parser::parse_v2(&expr) {
                        Err(e) => (String::new(), Some(format!("Parse error: {}", e))),
                        Ok(ast) if ast.statements.is_empty() => {
                            (format!("{}", crate::types::DataType::Null), None)
                        }
                        Ok(ast) => match self.execute(&ast) {
                            Ok(val) => (format!("{}", val), None),
                            Err(e) => (String::new(), Some(format!("{}", e))),
                        },
                    };
                    self.debug = debug_state;
                    // Restore heap state — pop scope frees debug allocations
                    self.heap.pop_scope();
                    self.heap.next_addr = saved_next_addr;
                    self.heap.free_list = saved_free_list;
                    self.heap.allocs_since_gc = saved_allocs;
                    // Restore state to prevent permanent mutation
                    self.functions = saved_functions;
                    self.enum_defs = saved_enums;
                    self.struct_defs = saved_structs;
                    self.async_fns = saved_async_fns;
                    self.std_op_aliases = saved_aliases;
                    self.closure_captures = saved_closures;
                    self.lambda_counter = saved_lambda_counter;
                    self.imports = saved_imports;
                    self.importing_packages = saved_importing;
                    self.logs = saved_logs;
                    self.call_depth = saved_call_depth;
                    self.symbols = saved_symbols;
                    self.saved_symbol_stacks = saved_stacks;
                    if let Some(ref debug) = self.debug {
                        debug
                            .event_sender
                            .blocking_send(DebugEvent::EvaluateResult { result, error })
                            .ok();
                    }
                    // Stay paused — loop back to wait for next command
                }
            }
        }
    }

    /// Run GC if the allocation threshold has been reached.
    fn maybe_gc(&mut self) -> usize {
        if !self.heap.should_gc() {
            return 0;
        }
        self.run_gc()
    }

    /// Perform a full mark-and-sweep garbage collection cycle.
    fn run_gc(&mut self) -> usize {
        // Collect GC root addresses from all live scopes
        let roots: std::collections::HashSet<MemAddr> = self.symbols.iter()
            .flat_map(|scope| scope.values().map(|e| e.addr))
            .chain(self.saved_symbol_stacks.iter()
                .flat_map(|stack| stack.iter()
                    .flat_map(|scope| scope.values().map(|e| e.addr))))
            .collect();
        self.heap.collect_garbage(&roots)
    }

    /// Get GC statistics.
    pub fn gc_stats(&self) -> &GcStats {
        &self.heap.gc_stats
    }

    /// Number of live objects on the heap.
    pub fn heap_live_count(&self) -> usize {
        self.heap.values.len()
    }

    /// Recursively register all functions, enums, and structs from a module body
    /// with qualified names (e.g., `math::double`, `math::inner::helper`).
    /// Enums and structs are also registered unqualified for use within module functions.
    fn register_module(&mut self, prefix: &str, body: &Block) {
        // Guard against deeply nested module definitions (stack overflow protection)
        if prefix.matches("::").count() >= 64 {
            return;
        }
        for inner in &body.statements {
            match &inner.kind {
                StatementKind::FunctionDef(def) => {
                    let qualified = format!("{}::{}", prefix, def.name);
                    self.functions.insert(qualified, def.clone());
                }
                StatementKind::AsyncFunctionDef(def) => {
                    let qualified = format!("{}::{}", prefix, def.name);
                    self.async_fns.insert(qualified.clone());
                    self.functions.insert(qualified, def.clone());
                }
                StatementKind::EnumDef { name, variants, .. } => {
                    let qualified = format!("{}::{}", prefix, name);
                    self.enum_defs.insert(qualified, variants.clone());
                    // Register unqualified only if no top-level definition exists
                    // to prevent module enums from shadowing top-level ones
                    if !self.enum_defs.contains_key(name.as_str()) {
                        self.enum_defs.insert(name.clone(), variants.clone());
                    }
                }
                StatementKind::StructDef { name, fields, .. } => {
                    let qualified = format!("{}::{}", prefix, name);
                    self.struct_defs.insert(qualified, fields.clone());
                    // Register unqualified only if no top-level definition exists
                    if !self.struct_defs.contains_key(name.as_str()) {
                        self.struct_defs.insert(name.clone(), fields.clone());
                    }
                }
                StatementKind::ModuleDef { name, body: inner_body } => {
                    let nested_prefix = format!("{}::{}", prefix, name);
                    self.register_module(&nested_prefix, inner_body);
                }
                // pub use re-exports: make the imported item available under this module's prefix
                StatementKind::Use { path, alias, glob, is_pub: true } => {
                    let source_path = path.join("::");
                    if *glob {
                        // pub use inner_mod::* — re-export all items from inner module
                        let source_prefix = format!("{}::", source_path);
                        let matching_fns: Vec<(String, FunctionDef)> = self.functions.iter()
                            .filter(|(k, _)| k.starts_with(&source_prefix))
                            .map(|(k, v)| {
                                let short = k.strip_prefix(&source_prefix).unwrap_or(k).to_string();
                                (short, v.clone())
                            })
                            .filter(|(short, _)| !short.contains("::"))
                            .collect();
                        for (short, def) in matching_fns {
                            let reexported = format!("{}::{}", prefix, short);
                            self.functions.insert(reexported, def);
                        }
                        let matching_enums: Vec<(String, Vec<EnumVariant>)> = self.enum_defs.iter()
                            .filter(|(k, _)| k.starts_with(&source_prefix))
                            .map(|(k, v)| {
                                let short = k.strip_prefix(&source_prefix).unwrap_or(k).to_string();
                                (short, v.clone())
                            })
                            .filter(|(short, _)| !short.contains("::"))
                            .collect();
                        for (short, variants) in matching_enums {
                            let reexported = format!("{}::{}", prefix, short);
                            self.enum_defs.insert(reexported, variants);
                        }
                        let matching_structs: Vec<(String, Vec<StructField>)> = self.struct_defs.iter()
                            .filter(|(k, _)| k.starts_with(&source_prefix))
                            .map(|(k, v)| {
                                let short = k.strip_prefix(&source_prefix).unwrap_or(k).to_string();
                                (short, v.clone())
                            })
                            .filter(|(short, _)| !short.contains("::"))
                            .collect();
                        for (short, fields) in matching_structs {
                            let reexported = format!("{}::{}", prefix, short);
                            self.struct_defs.insert(reexported, fields);
                        }
                    } else {
                        let item_name = alias.as_ref().or_else(|| path.last()).cloned().unwrap_or_default();
                        let reexported = format!("{}::{}", prefix, item_name);
                        if let Some(func) = self.functions.get(&source_path).cloned() {
                            self.functions.insert(reexported, func);
                        } else if let Some(variants) = self.enum_defs.get(&source_path).cloned() {
                            self.enum_defs.insert(reexported, variants);
                        } else if let Some(fields) = self.struct_defs.get(&source_path).cloned() {
                            self.struct_defs.insert(reexported, fields);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // =========================================================================
    // Program execution
    // =========================================================================

    /// Execute a program and return the final output value (if any).
    ///
    /// Two-pass execution:
    /// - Pass 1: collect all `FunctionDef` statements into the function registry.
    /// - Pass 2: if a `main()` function exists, process only imports then call `main()`.
    ///   Otherwise, execute all top-level statements (skipping FunctionDef) as before.
    pub fn execute(&mut self, program: &Program) -> Result<DataType, InterpError> {
        // Pass 1: collect function definitions, module definitions (sync and async)
        for stmt in &program.statements {
            match &stmt.kind {
                StatementKind::FunctionDef(def) => {
                    self.functions.insert(def.name.clone(), def.clone());
                }
                StatementKind::AsyncFunctionDef(def) => {
                    self.async_fns.insert(def.name.clone());
                    self.functions.insert(def.name.clone(), def.clone());
                }
                StatementKind::EnumDef { name, variants, .. } => {
                    self.enum_defs.insert(name.clone(), variants.clone());
                }
                StatementKind::StructDef { name, fields, .. } => {
                    self.struct_defs.insert(name.clone(), fields.clone());
                }
                StatementKind::ImplBlock { type_name, methods } => {
                    let tm = self.impl_methods.entry(type_name.clone()).or_default();
                    for m in methods { tm.insert(m.name.clone(), m.clone()); }
                }
                StatementKind::TraitDef { name, methods } => {
                    self.trait_defs.insert(name.clone(), methods.clone());
                }
                StatementKind::ImplTrait { type_name, methods, .. } => {
                    let tm = self.impl_methods.entry(type_name.clone()).or_default();
                    for m in methods { tm.insert(m.name.clone(), m.clone()); }
                }
                StatementKind::ModuleDef { name, body } => {
                    self.register_module(name, body);
                }
                _ => {}
            }
        }

        // Pass 2: determine execution mode
        let has_main = self.functions.contains_key("main");

        if has_main {
            // Process imports, use statements, and top-level declarations, then call main()
            for stmt in &program.statements {
                match &stmt.kind {
                    StatementKind::Import(plugin_id) => {
                        self.imports.insert(plugin_id.clone());
                    }
                    StatementKind::Use { .. }
                    | StatementKind::ConstDef { .. }
                    | StatementKind::Let { .. }
                    | StatementKind::LetMut { .. }
                    | StatementKind::LetDestructure { .. } => {
                        self.exec_statement(stmt)?;
                    }
                    _ => {}
                }
            }
            // Snapshot top-level globals so they survive call_function's scope reset.
            // Inject them via closure_captures so they're available inside main().
            let globals: Vec<(String, DataType, bool)> = self.symbols.iter()
                .flat_map(|scope| scope.iter())
                .map(|(name, entry)| (name.clone(), self.heap.read(entry.addr).cloned().unwrap_or(DataType::Null), entry.mutable))
                .collect();
            if !globals.is_empty() {
                self.closure_captures.insert("main".to_string(), globals);
            }
            let main_span = self
                .functions
                .get("main")
                .map(|f| f.span)
                .unwrap_or_default();
            self.call_function("main", &[], main_span)
        } else {
            // Execute top-level statements, skip FunctionDefs/ModuleDefs (#132 keep-going)
            let keep_going = self.max_errors.is_some();
            let mut last_value = DataType::Null;
            for stmt in &program.statements {
                if matches!(
                    &stmt.kind,
                    StatementKind::FunctionDef(_)
                        | StatementKind::AsyncFunctionDef(_)
                        | StatementKind::ModuleDef { .. }
                        | StatementKind::EnumDef { .. }
                        | StatementKind::StructDef { .. }
                        | StatementKind::ImplBlock { .. }
                        | StatementKind::TraitDef { .. }
                        | StatementKind::ImplTrait { .. }
                ) {
                    continue;
                }
                last_value = match self.exec_statement(stmt) {
                    Ok(val) => val,
                    Err(InterpError::BreakSignal(_)) | Err(InterpError::LabeledBreak { .. }) => {
                        let err = InterpError::BreakOutsideLoop { span: stmt.span };
                        if keep_going && self.collect_error_keep_going(err) { DataType::Null } else if keep_going { break; } else { return Err(InterpError::BreakOutsideLoop { span: stmt.span }); }
                    }
                    Err(InterpError::ContinueSignal) | Err(InterpError::LabeledContinue { .. }) => {
                        let err = InterpError::ContinueOutsideLoop { span: stmt.span };
                        if keep_going && self.collect_error_keep_going(err) { DataType::Null } else if keep_going { break; } else { return Err(InterpError::ContinueOutsideLoop { span: stmt.span }); }
                    }
                    Err(InterpError::ReturnSignal(_)) => {
                        let err = InterpError::ReturnOutsideFunction { span: stmt.span };
                        if keep_going && self.collect_error_keep_going(err) { DataType::Null } else if keep_going { break; } else { return Err(InterpError::ReturnOutsideFunction { span: stmt.span }); }
                    }
                    Err(e) if keep_going && !matches!(e, InterpError::Cancelled) => {
                        if self.collect_error_keep_going(e) { DataType::Null } else { break; }
                    }
                    Err(e) => return Err(e),
                };
                if self.is_cancelled() {
                    return Err(InterpError::Cancelled);
                }
            }
            if !self.collected_errors.is_empty() {
                Err(self.collected_errors.remove(0))
            } else {
                Ok(last_value)
            }
        }
    }

    // =========================================================================
    // Statements
    // =========================================================================

    fn exec_statement(&mut self, stmt: &Statement) -> Result<DataType, InterpError> {
        self.debug_check(stmt.span);

        match &stmt.kind {
            StatementKind::Import(plugin_id) => {
                self.imports.insert(plugin_id.clone());
                Ok(DataType::Null)
            }

            StatementKind::Let { name, value, .. } => {
                if name.len() > MAX_IDENTIFIER_LEN {
                    return Err(InterpError::ResourceLimit { limit: format!("{} chars", MAX_IDENTIFIER_LEN), actual: format!("{} chars", name.len()), context: "identifier length".to_string(), span: stmt.span });
                }
                if self.total_variable_count() >= MAX_VARIABLES {
                    return Err(InterpError::ResourceLimit { limit: format!("{}", MAX_VARIABLES), actual: "limit reached".to_string(), context: "total variables".to_string(), span: stmt.span });
                }
                let val = self.eval_expr(value)?;
                let addr = self.heap.alloc(val.clone());
                self.define(name, addr, false);
                Ok(val)
            }

            StatementKind::LetMut { name, value, .. } => {
                if name.len() > MAX_IDENTIFIER_LEN {
                    return Err(InterpError::ResourceLimit { limit: format!("{} chars", MAX_IDENTIFIER_LEN), actual: format!("{} chars", name.len()), context: "identifier length".to_string(), span: stmt.span });
                }
                if self.total_variable_count() >= MAX_VARIABLES {
                    return Err(InterpError::ResourceLimit { limit: format!("{}", MAX_VARIABLES), actual: "limit reached".to_string(), context: "total variables".to_string(), span: stmt.span });
                }
                let val = self.eval_expr(value)?;
                let addr = self.heap.alloc(val.clone());
                self.define(name, addr, true);
                Ok(val)
            }

            StatementKind::Assignment { name, value } => {
                let val = self.eval_expr(value)?;
                let addr = match self.lookup(name) {
                    Some(entry) if !entry.mutable => {
                        return Err(InterpError::ImmutableAssignment {
                            name: name.clone(),
                            span: stmt.span,
                        });
                    }
                    None => {
                        let suggestion = self.suggest_variable(name);
                        return Err(InterpError::UndefinedVariable {
                            name: name.clone(),
                            span: stmt.span,
                            suggestion,
                        });
                    }
                    Some(entry) => entry.addr,
                };
                self.heap.write(addr, val.clone());
                Ok(val)
            }

            StatementKind::ForLoop { label, pattern, iterable, body } => {
                let iter_val = self.eval_expr(iterable)?;
                let items = match iter_val {
                    DataType::Array(arr) => arr,
                    DataType::Map(map) => {
                        map.into_iter()
                            .map(|(k, v)| {
                                let mut entry = indexmap::IndexMap::new();
                                entry.insert("key".to_string(), DataType::String(k));
                                entry.insert("value".to_string(), v);
                                DataType::Map(entry)
                            })
                            .collect()
                    }
                    DataType::String(s) => {
                        let char_count = s.chars().count();
                        if char_count > MAX_ARRAY_ELEMENTS {
                            return Err(InterpError::ResourceLimit {
                                limit: format!("{} chars", MAX_ARRAY_ELEMENTS),
                                actual: format!("{}", char_count),
                                context: "for loop string iteration".to_string(),
                                span: stmt.span,
                            });
                        }
                        s.chars()
                            .map(|c| DataType::String(c.to_string()))
                            .collect()
                    }
                    other => {
                        return Err(InterpError::TypeError {
                            expected: "Array, Map, or String".to_string(),
                            actual: other.type_name().to_string(),
                            context: "for loop iterable".to_string(),
                            span: iterable.span,
                        });
                    }
                };

                let mut last = DataType::Null;
                // Guard: `i` tracks the actual iteration count (0-based index into
                // the materialized `items` vec). This caps the number of loop body
                // executions, not the collection length (which is separately guarded
                // by MAX_ARRAY_ELEMENTS for strings). Arrays and maps are iterated
                // directly, so the cap applies to the number of elements visited.
                for (i, item) in items.into_iter().enumerate() {
                    if self.is_cancelled() {
                        return Err(InterpError::Cancelled);
                    }
                    if i >= MAX_LOOP_ITERATIONS {
                        return Err(InterpError::MaxIterations {
                            limit: MAX_LOOP_ITERATIONS,
                            span: stmt.span,
                        });
                    }
                    self.symbols.push(HashMap::new());
                    self.heap.push_scope();
                    let bind_result = match pattern {
                        ForPattern::Single(name) => {
                            let addr = self.heap.alloc(item);
                            self.define(name, addr, false);
                            Ok(())
                        }
                        ForPattern::ArrayDestructure(elements) => {
                            let destr = DestructurePattern::Array(elements.clone());
                            self.destructure_bind(&destr, &item, false, stmt.span)
                        }
                        ForPattern::MapDestructure(entries) => {
                            let destr = DestructurePattern::Map(entries.clone());
                            self.destructure_bind(&destr, &item, false, stmt.span)
                        }
                    };
                    if let Err(e) = bind_result {
                        self.heap.pop_scope();
                        self.symbols.pop();
                        return Err(e);
                    }
                    let result = self.exec_block(body);
                    self.heap.pop_scope();
                    self.symbols.pop();
                    match result {
                        Ok(val) => last = val,
                        Err(InterpError::BreakSignal(val)) => {
                            last = val;
                            break;
                        }
                        Err(InterpError::LabeledBreak { label: ref lbl, ref value })
                            if label.as_deref() == Some(lbl.as_str()) =>
                        {
                            last = value.clone();
                            break;
                        }
                        Err(InterpError::ContinueSignal) => continue,
                        Err(InterpError::LabeledContinue { label: ref lbl })
                            if label.as_deref() == Some(lbl.as_str()) => continue,
                        Err(e) => return Err(e),
                    }
                    self.maybe_gc();
                }
                Ok(last)
            }

            StatementKind::WhileLoop { label, condition, body } => {
                let mut iterations = 0;
                let mut last = DataType::Null;
                loop {
                    if self.is_cancelled() {
                        return Err(InterpError::Cancelled);
                    }
                    if iterations >= MAX_LOOP_ITERATIONS {
                        return Err(InterpError::MaxIterations {
                            limit: MAX_LOOP_ITERATIONS,
                            span: stmt.span,
                        });
                    }

                    let cond = self.eval_expr(condition)?;
                    let is_true = match &cond {
                        DataType::Bool(b) => *b,
                        other => {
                            return Err(InterpError::TypeError {
                                expected: "Bool".to_string(),
                                actual: other.type_name().to_string(),
                                context: "while condition".to_string(),
                                span: condition.span,
                            });
                        }
                    };

                    if !is_true {
                        break;
                    }

                    // Each iteration gets its own scope so loop body
                    // variables don't leak to the enclosing scope.
                    self.symbols.push(HashMap::new());
                    self.heap.push_scope();
                    let result = self.exec_block(body);
                    self.heap.pop_scope();
                    self.symbols.pop();
                    match result {
                        Ok(val) => { last = val; }
                        Err(InterpError::BreakSignal(val)) => {
                            last = val;
                            break;
                        }
                        Err(InterpError::LabeledBreak { label: ref lbl, ref value })
                            if label.as_deref() == Some(lbl.as_str()) =>
                        {
                            last = value.clone();
                            break;
                        }
                        Err(InterpError::ContinueSignal) => {}
                        Err(InterpError::LabeledContinue { label: ref lbl })
                            if label.as_deref() == Some(lbl.as_str()) => {}
                        Err(e) => return Err(e), // propagate return/errors
                    }
                    iterations += 1;
                    self.maybe_gc();
                }
                Ok(last)
            }

            StatementKind::CStyleFor { init, condition, update, body } => {
                self.symbols.push(HashMap::new());
                self.heap.push_scope();
                self.exec_statement(init)?;
                let mut iterations = 0;
                let mut last = DataType::Null;
                loop {
                    if self.is_cancelled() {
                        self.heap.pop_scope();
                        self.symbols.pop();
                        return Err(InterpError::Cancelled);
                    }
                    if iterations >= MAX_LOOP_ITERATIONS {
                        self.heap.pop_scope();
                        self.symbols.pop();
                        return Err(InterpError::MaxIterations { limit: MAX_LOOP_ITERATIONS, span: stmt.span });
                    }
                    let cond = self.eval_expr(condition)?;
                    if !cond.to_bool() { break; }
                    self.symbols.push(HashMap::new());
                    self.heap.push_scope();
                    let result = self.exec_block(body);
                    self.heap.pop_scope();
                    self.symbols.pop();
                    match result {
                        Ok(val) => { last = val; }
                        Err(InterpError::BreakSignal(val)) => { last = val; break; }
                        Err(InterpError::ContinueSignal) => {}
                        Err(e) => {
                            self.heap.pop_scope();
                            self.symbols.pop();
                            return Err(e);
                        }
                    }
                    self.exec_statement(update)?;
                    iterations += 1;
                    self.maybe_gc();
                }
                self.heap.pop_scope();
                self.symbols.pop();
                Ok(last)
            }

            StatementKind::DoWhileLoop { label, body, condition } => {
                let mut iterations = 0;
                let mut last = DataType::Null;
                loop {
                    if self.is_cancelled() {
                        return Err(InterpError::Cancelled);
                    }
                    if iterations >= MAX_LOOP_ITERATIONS {
                        return Err(InterpError::MaxIterations {
                            limit: MAX_LOOP_ITERATIONS,
                            span: stmt.span,
                        });
                    }

                    // Execute body first (guaranteed at least one execution)
                    self.symbols.push(HashMap::new());
                    self.heap.push_scope();
                    let result = self.exec_block(body);
                    self.heap.pop_scope();
                    self.symbols.pop();
                    match result {
                        Ok(val) => { last = val; }
                        Err(InterpError::BreakSignal(val)) => {
                            last = val;
                            break;
                        }
                        Err(InterpError::LabeledBreak { label: ref lbl, ref value })
                            if label.as_deref() == Some(lbl.as_str()) =>
                        {
                            last = value.clone();
                            break;
                        }
                        Err(InterpError::ContinueSignal) => {}
                        Err(InterpError::LabeledContinue { label: ref lbl })
                            if label.as_deref() == Some(lbl.as_str()) => {}
                        Err(e) => return Err(e),
                    }

                    // Then check condition
                    let cond = self.eval_expr(condition)?;
                    let is_true = match &cond {
                        DataType::Bool(b) => *b,
                        other => {
                            return Err(InterpError::TypeError {
                                expected: "Bool".to_string(),
                                actual: other.type_name().to_string(),
                                context: "do-while condition".to_string(),
                                span: condition.span,
                            });
                        }
                    };

                    if !is_true {
                        break;
                    }
                    iterations += 1;
                    self.maybe_gc();
                }
                Ok(last)
            }

            StatementKind::Defer(expr) => {
                // Push the deferred expression onto the current scope's deferred stack.
                // It will be executed (in reverse order) when the enclosing block exits.
                if let Some(scope) = self.deferred.last_mut() {
                    scope.push(expr.clone());
                }
                Ok(DataType::Null)
            }

            StatementKind::Output(expr) => {
                let val = self.eval_expr(expr)?;
                self.logs.push(LogEntry {
                    level: LogLevel::Info,
                    message: datatype_to_display(&val),
                    line: Some(stmt.span.start_line),
                    node_id: None,
                });
                Ok(val)
            }

            StatementKind::ExprStatement(expr) => self.eval_expr(expr),

            StatementKind::FunctionDef(def) => {
                if self.functions.len() >= MAX_FUNCTIONS {
                    return Err(InterpError::ResourceLimit { limit: format!("{}", MAX_FUNCTIONS), actual: "limit reached".to_string(), context: "function definitions".to_string(), span: stmt.span });
                }
                self.functions.insert(def.name.clone(), def.clone());
                Ok(DataType::Null)
            }
            StatementKind::AsyncFunctionDef(def) => {
                if self.functions.len() >= MAX_FUNCTIONS {
                    return Err(InterpError::ResourceLimit { limit: format!("{}", MAX_FUNCTIONS), actual: "limit reached".to_string(), context: "function definitions".to_string(), span: stmt.span });
                }
                self.async_fns.insert(def.name.clone());
                self.functions.insert(def.name.clone(), def.clone());
                Ok(DataType::Null)
            }

            StatementKind::Break { ref label, ref value } => {
                let val = match value {
                    Some(expr) => self.eval_expr(expr)?,
                    None => DataType::Null,
                };
                match label {
                    Some(lbl) => Err(InterpError::LabeledBreak { label: lbl.clone(), value: val }),
                    None => Err(InterpError::BreakSignal(val)),
                }
            }

            StatementKind::Continue { ref label } => {
                match label {
                    Some(lbl) => Err(InterpError::LabeledContinue { label: lbl.clone() }),
                    None => Err(InterpError::ContinueSignal),
                }
            }

            StatementKind::Return(ref val_expr) => {
                let val = match val_expr {
                    Some(expr) => self.eval_expr(expr)?,
                    None => DataType::Null,
                };
                Err(InterpError::ReturnSignal(val))
            }

            StatementKind::LetDestructure {
                pattern,
                mutable,
                value,
            } => {
                let val = self.eval_expr(value)?;
                self.destructure_bind(pattern, &val, *mutable, stmt.span)?;
                Ok(val)
            }

            StatementKind::CompoundAssign { name, op, value } => {
                let (addr, mutable) = match self.lookup(name) {
                    Some(entry) => (entry.addr, entry.mutable),
                    None => {
                        let suggestion = self.suggest_variable(name);
                        return Err(InterpError::UndefinedVariable {
                            name: name.clone(),
                            span: stmt.span,
                            suggestion,
                        });
                    }
                };
                if !mutable {
                    return Err(InterpError::ImmutableAssignment {
                        name: name.clone(),
                        span: stmt.span,
                    });
                }
                let current = self.heap.read(addr).cloned().ok_or_else(|| {
                    InterpError::UndefinedVariable {
                        name: name.clone(),
                        span: stmt.span,
                        suggestion: None,
                    }
                })?;
                let rhs = self.eval_expr(value)?;
                let op_type = OperationType::parse(op.operation_name()).ok_or_else(|| {
                    InterpError::UnknownOperation {
                        name: op.operation_name().to_string(),
                        span: stmt.span,
                        suggestion: None,
                    }
                })?;
                let input_ports = op_input_ports(op_type);
                let mut inputs = HashMap::with_capacity(2);
                if let Some(p) = input_ports.first() {
                    inputs.insert(p.to_string(), current);
                }
                if let Some(p) = input_ports.get(1) {
                    inputs.insert(p.to_string(), rhs);
                }
                let result = self.evaluator.eval_operation(op_type, &inputs, &EMPTY_CONFIG).map_err(|e| {
                    InterpError::EvalError {
                        error: e,
                        span: stmt.span,
                    }
                })?;
                self.heap.write(addr, result.clone());
                Ok(result)
            }

            // Field assignment: obj.field = value (#7)
            StatementKind::FieldAssignment { object, field, value } => {
                let val = self.eval_expr(value)?;
                match &object.kind {
                    ExpressionKind::Variable(name) => {
                        let entry = self.lookup(name).ok_or_else(|| InterpError::UndefinedVariable { name: name.clone(), span: stmt.span, suggestion: self.suggest_variable(name) })?;
                        if !entry.mutable {
                            return Err(InterpError::ImmutableAssignment { name: name.clone(), span: stmt.span });
                        }
                        let addr = entry.addr;
                        let obj = self.heap.read(addr).cloned().ok_or_else(|| InterpError::UndefinedVariable { name: name.clone(), span: stmt.span, suggestion: None })?;

                        // Check for property setter
                        if let DataType::Map(ref map) = obj {
                            if let Some(DataType::String(struct_name)) = map.get("__struct") {
                                if let Some(setter) = self.impl_methods
                                    .get(struct_name)
                                    .and_then(|m| m.get(field.as_str()))
                                    .filter(|m| m.is_setter)
                                    .cloned()
                                {
                                    self.call_depth += 1;
                                    self.symbols.push(HashMap::new());
                                    self.heap.push_scope();
                                    if let Some(p) = setter.params.first() {
                                        let a = self.heap.alloc(obj.clone());
                                        self.define(&p.name, a, true);
                                    }
                                    if let Some(p) = setter.params.get(1) {
                                        let a = self.heap.alloc(val.clone());
                                        self.define(&p.name, a, false);
                                    }
                                    let result = self.exec_block(&setter.body);
                                    // If the setter modified self, write it back
                                    if let Some(p) = setter.params.first() {
                                        if let Some(e) = self.lookup(&p.name) {
                                            if let Some(new_obj) = self.heap.read(e.addr).cloned() {
                                                self.heap.write(addr, new_obj);
                                            }
                                        }
                                    }
                                    self.heap.pop_scope();
                                    self.symbols.pop();
                                    self.call_depth -= 1;
                                    return match result {
                                        Ok(_) | Err(InterpError::ReturnSignal(_)) => Ok(val),
                                        Err(e) => Err(e),
                                    };
                                }
                            }
                        }

                        let mut obj = obj;
                        match &mut obj {
                            DataType::Map(map) => { map.insert(field.clone(), val.clone()); }
                            _ => return Err(InterpError::TypeError { expected: "Map or struct".to_string(), actual: obj.type_name().to_string(), context: format!("field assignment .{}", field), span: stmt.span }),
                        }
                        self.heap.write(addr, obj);
                        Ok(val)
                    }
                    _ => Err(InterpError::TypeError { expected: "mutable variable".to_string(), actual: "expression".to_string(), context: "field assignment target must be a variable".to_string(), span: stmt.span }),
                }
            }

            // Index assignment: obj[index] = value (#7)
            StatementKind::IndexAssignment { object, index, value } => {
                let idx_val = self.eval_expr(index)?;
                let val = self.eval_expr(value)?;
                match &object.kind {
                    ExpressionKind::Variable(name) => {
                        let entry = self.lookup(name).ok_or_else(|| InterpError::UndefinedVariable { name: name.clone(), span: stmt.span, suggestion: self.suggest_variable(name) })?;
                        if !entry.mutable {
                            return Err(InterpError::ImmutableAssignment { name: name.clone(), span: stmt.span });
                        }
                        let addr = entry.addr;
                        let mut obj = self.heap.read(addr).cloned().ok_or_else(|| InterpError::UndefinedVariable { name: name.clone(), span: stmt.span, suggestion: None })?;
                        match (&mut obj, &idx_val) {
                            (DataType::Array(arr), _) => {
                                let i = idx_val.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: idx_val.type_name().to_string(), context: "array index".to_string(), span: stmt.span })?;
                                let len = arr.len() as i64;
                                let idx = if i < 0 { (len + i).max(0) as usize } else { i as usize };
                                if idx < arr.len() {
                                    arr[idx] = val.clone();
                                } else {
                                    return Err(InterpError::TypeError { expected: format!("index 0..{}", arr.len()), actual: format!("{}", i), context: "index assignment out of bounds".to_string(), span: stmt.span });
                                }
                            }
                            (DataType::Map(map), DataType::String(key)) => {
                                map.insert(key.clone(), val.clone());
                            }
                            (DataType::Map(map), other) => {
                                map.insert(other.to_string_lossy(), val.clone());
                            }
                            _ => return Err(InterpError::TypeError { expected: "Array or Map".to_string(), actual: obj.type_name().to_string(), context: "index assignment".to_string(), span: stmt.span }),
                        }
                        self.heap.write(addr, obj);
                        Ok(val)
                    }
                    _ => Err(InterpError::TypeError { expected: "mutable variable".to_string(), actual: "expression".to_string(), context: "index assignment target must be a variable".to_string(), span: stmt.span }),
                }
            }

            StatementKind::TryCatch {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                // Scope-isolate the try block so variables don't leak
                self.symbols.push(HashMap::new());
                self.heap.push_scope();
                let try_result = self.exec_block(try_block);
                self.heap.pop_scope();
                self.symbols.pop();
                let result = match try_result {
                    Ok(val) => Ok(val),
                    Err(ref e) if is_control_flow(e) => {
                        if let Some(finally) = finally_block {
                            // Finally errors propagate (override control flow).
                            self.symbols.push(HashMap::new());
                            self.heap.push_scope();
                            let finally_result = self.exec_block(finally);
                            self.heap.pop_scope();
                            self.symbols.pop();
                            finally_result?;
                        }
                        return try_result;
                    }
                    Err(e) => {
                        let catch_value = match e {
                            InterpError::ThrownError { value, .. } => value,
                            other => DataType::String(format!("{}", other)),
                        };
                        self.symbols.push(HashMap::new());
                        self.heap.push_scope();
                        if let Some(var_name) = catch_var {
                            let addr = self.heap.alloc(catch_value);
                            self.define(var_name, addr, false);
                        }
                        let catch_result = self.exec_block(catch_block);
                        self.heap.pop_scope();
                        self.symbols.pop();
                        catch_result
                    }
                };
                if let Some(finally) = finally_block {
                    // Finally errors propagate (override try/catch result).
                    self.symbols.push(HashMap::new());
                    self.heap.push_scope();
                    let finally_result = self.exec_block(finally);
                    self.heap.pop_scope();
                    self.symbols.pop();
                    finally_result?;
                }
                result
            }

            StatementKind::Throw(expr) => {
                let val = self.eval_expr(expr)?;
                Err(InterpError::ThrownError {
                    value: val,
                    span: stmt.span,
                })
            }

            StatementKind::ConstDef { name, value, .. } => {
                let val = self.eval_expr(value)?;
                let addr = self.heap.alloc(val.clone());
                self.define(name, addr, false); // constants are always immutable
                Ok(val)
            }

            StatementKind::TypeAlias { .. } => {
                // Type aliases are compile-time only, no runtime effect
                Ok(DataType::Null)
            }

            StatementKind::ModuleDef { .. } => {
                // Module functions already collected in pass 1
                Ok(DataType::Null)
            }

            StatementKind::TestDef { .. } => {
                // Tests are only run via run_tests(), not during normal execution
                Ok(DataType::Null)
            }

            StatementKind::EnumDef { name, variants, .. } => {
                // Store enum definition
                self.enum_defs.insert(name.clone(), variants.clone());
                Ok(DataType::Null)
            }

            StatementKind::StructDef { name, fields, .. } => {
                // Store struct definition
                self.struct_defs.insert(name.clone(), fields.clone());
                Ok(DataType::Null)
            }

            StatementKind::ImplBlock { .. } => Ok(DataType::Null),
            StatementKind::TraitDef { .. } => Ok(DataType::Null),
            StatementKind::ImplTrait { .. } => Ok(DataType::Null),

            StatementKind::Use { path, alias, glob, .. } => {
                // Check if this is a std library import
                if path.first().map(|s| s.as_str()) == Some("std") {
                    return self.handle_std_use(path, alias.as_deref(), *glob, stmt.span);
                }
                // Check if this is a package import
                if path.first().map(|s| s.as_str()) == Some("pkg") {
                    return self.handle_pkg_use(path, alias.as_deref(), *glob, stmt.span);
                }
                let full_path = path.join("::");
                if *glob {
                    // Glob import: import direct children only (not nested modules)
                    let prefix = format!("{}::", full_path);
                    let matching_fns: Vec<(String, FunctionDef)> = self
                        .functions
                        .iter()
                        .filter(|(k, _)| k.starts_with(&prefix))
                        .map(|(k, v)| {
                            let short_name = k.strip_prefix(&*prefix).unwrap_or(k).to_string();
                            (short_name, v.clone())
                        })
                        .filter(|(short_name, _)| !short_name.contains("::"))
                        .collect();
                    for (short_name, def) in matching_fns {
                        self.functions.insert(short_name, def);
                    }
                    // Also import enums with qualified names under the prefix (direct only)
                    let matching_enums: Vec<(String, Vec<EnumVariant>)> = self
                        .enum_defs
                        .iter()
                        .filter(|(k, _)| k.starts_with(&prefix))
                        .map(|(k, v)| {
                            let short_name = k.strip_prefix(&*prefix).unwrap_or(k).to_string();
                            (short_name, v.clone())
                        })
                        .filter(|(short_name, _)| !short_name.contains("::"))
                        .collect();
                    for (short_name, variants) in matching_enums {
                        self.enum_defs.insert(short_name, variants);
                    }
                    // Also import structs with qualified names under the prefix (direct only)
                    let matching_structs: Vec<(String, Vec<StructField>)> = self
                        .struct_defs
                        .iter()
                        .filter(|(k, _)| k.starts_with(&prefix))
                        .map(|(k, v)| {
                            let short_name = k.strip_prefix(&*prefix).unwrap_or(k).to_string();
                            (short_name, v.clone())
                        })
                        .filter(|(short_name, _)| !short_name.contains("::"))
                        .collect();
                    for (short_name, fields) in matching_structs {
                        self.struct_defs.insert(short_name, fields);
                    }
                } else {
                    let item_name = path.last().cloned().unwrap_or_default();
                    let local_name = alias.as_ref().unwrap_or(&item_name).clone();
                    // Try function first
                    if let Some(func) = self.functions.get(&full_path).cloned() {
                        self.functions.insert(local_name, func);
                    } else if let Some(variants) = self.enum_defs.get(&full_path).cloned() {
                        // Try enum
                        self.enum_defs.insert(local_name, variants);
                    } else if let Some(fields) = self.struct_defs.get(&full_path).cloned() {
                        // Try struct
                        self.struct_defs.insert(local_name, fields);
                    } else {
                        // Collect available items from the module for suggestions
                        let module_prefix = if path.len() > 1 {
                            format!("{}::", path[..path.len()-1].join("::"))
                        } else {
                            String::new()
                        };
                        let mut available: Vec<String> = self.functions.keys()
                            .filter(|k| k.starts_with(&module_prefix))
                            .map(|k| k.strip_prefix(&*module_prefix).unwrap_or(k).to_string())
                            .collect();
                        // Also include enum and struct names
                        for k in self.enum_defs.keys() {
                            if k.starts_with(&module_prefix) {
                                available.push(k.strip_prefix(&*module_prefix).unwrap_or(k).to_string());
                            }
                        }
                        for k in self.struct_defs.keys() {
                            if k.starts_with(&module_prefix) {
                                available.push(k.strip_prefix(&*module_prefix).unwrap_or(k).to_string());
                            }
                        }
                        let refs: Vec<&str> = available.iter().map(|s| s.as_str()).collect();
                        let suggestion = super::errors::suggest_name(&item_name, &refs);
                        return Err(InterpError::UnknownOperation {
                            name: full_path,
                            span: stmt.span,
                            suggestion,
                        });
                    }
                }
                Ok(DataType::Null)
            }
        }
    }

    // =========================================================================
    // Spread-aware argument evaluation
    // =========================================================================

    fn eval_call_args(&mut self, args: &[Expression]) -> Result<Vec<DataType>, InterpError> {
        let mut result = Vec::new();
        for arg in args {
            if let ExpressionKind::Spread(inner) = &arg.kind {
                match self.eval_expr(inner)? {
                    DataType::Array(arr) => {
                        result.extend(arr);
                        if result.len() > MAX_ARRAY_ELEMENTS {
                            return Err(InterpError::ResourceLimit {
                                limit: format!("{} elements", MAX_ARRAY_ELEMENTS),
                                actual: format!("{} elements", result.len()),
                                context: "spread in function call".to_string(),
                                span: arg.span,
                            });
                        }
                    }
                    other => {
                        return Err(InterpError::TypeError {
                            expected: "Array".to_string(),
                            actual: other.type_name().to_string(),
                            context: "spread operator".to_string(),
                            span: arg.span,
                        });
                    }
                }
            } else {
                result.push(self.eval_expr(arg)?);
            }
        }
        Ok(result)
    }

    /// Merge named (keyword) arguments into a positional argument vector
    /// by matching kwarg names to the function's parameter names (#300).
    fn merge_kwargs_into_args(
        &mut self,
        fn_name: &str,
        args: &[Expression],
        kwargs: &[(String, Expression)],
        span: Span,
    ) -> Result<Vec<DataType>, InterpError> {
        let mut evaluated = self.eval_call_args(args)?;
        if kwargs.is_empty() {
            return Ok(evaluated);
        }
        let func = match self.functions.get(fn_name) {
            Some(f) => f.clone(),
            None => return Ok(evaluated),
        };
        // Check if the function has a **kwargs parameter
        let kwargs_param = func.params.iter().find(|p| p.kwargs).map(|p| p.name.clone());
        let mut extra_kwargs = IndexMap::new();

        // Extend the evaluated args vector to accommodate named args
        for (kwarg_name, kwarg_expr) in kwargs {
            let kwarg_val = self.eval_expr(kwarg_expr)?;
            // Find the parameter index by name
            if let Some(idx) = func.params.iter().position(|p| p.name == *kwarg_name && !p.kwargs) {
                // Grow the vector if needed (fill gaps with Null)
                while evaluated.len() <= idx {
                    evaluated.push(DataType::Null);
                }
                evaluated[idx] = kwarg_val;
            } else if kwargs_param.is_some() {
                // Collect into the kwargs map
                extra_kwargs.insert(kwarg_name.clone(), kwarg_val);
            } else {
                return Err(InterpError::EvalError {
                    error: EvalError::InvalidInput(
                        format!("function '{}' has no parameter named '{}'", fn_name, kwarg_name),
                    ),
                    span,
                });
            }
        }

        // If the function has a **kwargs param, add the collected map
        if kwargs_param.is_some() {
            evaluated.push(DataType::Map(extra_kwargs));
        }

        Ok(evaluated)
    }

    /// Spread-aware argument evaluation for pipe stages.
    /// Replaces Placeholder expressions with the piped value.
    fn eval_pipe_call_args(&mut self, args: &[Expression], piped_value: &DataType) -> Result<Vec<DataType>, InterpError> {
        let mut result = Vec::new();
        for arg in args {
            if matches!(arg.kind, ExpressionKind::Placeholder) {
                result.push(piped_value.clone());
            } else if let ExpressionKind::Spread(inner) = &arg.kind {
                match self.eval_expr(inner)? {
                    DataType::Array(arr) => {
                        result.extend(arr);
                        if result.len() > MAX_ARRAY_ELEMENTS {
                            return Err(InterpError::ResourceLimit {
                                limit: format!("{} elements", MAX_ARRAY_ELEMENTS),
                                actual: format!("{} elements", result.len()),
                                context: "spread in pipe call".to_string(),
                                span: arg.span,
                            });
                        }
                    }
                    other => {
                        return Err(InterpError::TypeError {
                            expected: "Array".to_string(),
                            actual: other.type_name().to_string(),
                            context: "spread operator in pipe".to_string(),
                            span: arg.span,
                        });
                    }
                }
            } else {
                result.push(self.eval_expr(arg)?);
            }
        }
        Ok(result)
    }

    // =========================================================================
    // Slice evaluation (arr[1..3], str[0..5])
    // =========================================================================

    fn eval_slice(&self, obj: &DataType, start: &DataType, end: &DataType, inclusive: bool, span: Span) -> Result<DataType, InterpError> {
        let len = match obj {
            DataType::Array(arr) => arr.len() as i64,
            DataType::String(s) => s.chars().count() as i64,
            _ => return Err(InterpError::TypeError {
                expected: "Array or String".to_string(),
                actual: obj.type_name().to_string(),
                context: "slice operation".to_string(),
                span,
            }),
        };
        let s_raw = start.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: start.type_name().to_string(), context: "slice start".to_string(), span })?;
        let e_raw_i = end.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: end.type_name().to_string(), context: "slice end".to_string(), span })?;
        // Wrap negative indices from end (Python-style)
        let s = if s_raw < 0 { (len + s_raw).max(0) as usize } else { usize::try_from(s_raw).unwrap_or(usize::MAX) };
        let e_raw = if e_raw_i < 0 { (len + e_raw_i).max(0) as usize } else { usize::try_from(e_raw_i).unwrap_or(usize::MAX) };
        let e = if inclusive { e_raw.saturating_add(1) } else { e_raw };

        match obj {
            DataType::Array(arr) => {
                let actual_end = e.min(arr.len());
                let actual_start = s.min(actual_end);
                Ok(DataType::Array(arr[actual_start..actual_end].to_vec()))
            }
            DataType::String(str_val) => {
                let chars: Vec<char> = str_val.chars().collect();
                let actual_end = e.min(chars.len());
                let actual_start = s.min(actual_end);
                Ok(DataType::String(chars[actual_start..actual_end].iter().collect()))
            }
            _ => Err(InterpError::TypeError {
                expected: "Array or String".to_string(),
                actual: obj.type_name().to_string(),
                context: "slice operation".to_string(),
                span,
            }),
        }
    }

    // =========================================================================
    // Resolve and call a lambda/function by name with args
    // =========================================================================

    fn call_lambda_with_args(&mut self, fn_arg: &Expression, args: &[DataType], span: Span) -> Result<DataType, InterpError> {
        let fn_val = self.eval_expr(fn_arg)?;
        let fn_name = match fn_val {
            DataType::String(s) => s,
            _ => return Err(InterpError::TypeError {
                expected: "function".to_string(),
                actual: fn_val.type_name().to_string(),
                context: "higher-order method callback".to_string(),
                span,
            }),
        };
        self.call_function(&fn_name, args, span)
    }

    // =========================================================================
    // Merge sort with fallible comparator for sort_by
    // =========================================================================

    fn merge_sort_by(
        &mut self,
        items: Vec<DataType>,
        comparator: &Expression,
        span: Span,
        comparison_count: &std::cell::Cell<usize>,
        max_comparisons: usize,
    ) -> Result<Vec<DataType>, InterpError> {
        let len = items.len();
        if len <= 1 {
            return Ok(items);
        }

        if self.is_cancelled() { return Err(InterpError::Cancelled); }

        let mid = len / 2;
        let left = self.merge_sort_by(items[..mid].to_vec(), comparator, span, comparison_count, max_comparisons)?;
        let right = self.merge_sort_by(items[mid..].to_vec(), comparator, span, comparison_count, max_comparisons)?;

        // Merge the two sorted halves
        let mut result = Vec::with_capacity(len);
        let mut li = 0;
        let mut ri = 0;

        while li < left.len() && ri < right.len() {
            if self.is_cancelled() { return Err(InterpError::Cancelled); }
            let count = comparison_count.get() + 1;
            comparison_count.set(count);
            if count > max_comparisons {
                return Err(InterpError::MaxIterations { limit: max_comparisons, span });
            }

            let cmp = self.call_lambda_with_args(comparator, &[left[li].clone(), right[ri].clone()], span)?;
            let cmp_val = cmp.to_f64().ok_or_else(|| InterpError::TypeError {
                expected: "number".to_string(),
                actual: cmp.type_name().to_string(),
                context: "sort_by comparator must return a number".to_string(),
                span,
            })?;
            if cmp_val.is_nan() {
                return Err(InterpError::TypeError {
                    expected: "finite number".to_string(),
                    actual: "NaN".to_string(),
                    context: "sort_by comparator returned NaN".to_string(),
                    span,
                });
            }

            if cmp_val <= 0.0 {
                result.push(left[li].clone());
                li += 1;
            } else {
                result.push(right[ri].clone());
                ri += 1;
            }
        }

        // Append remaining elements
        result.extend_from_slice(&left[li..]);
        result.extend_from_slice(&right[ri..]);

        Ok(result)
    }

    // =========================================================================
    // Higher-order function methods (Phase 1)
    // =========================================================================

    fn try_eval_hof_method(&mut self, obj: &DataType, method: &str, args: &[Expression], span: Span) -> Result<Option<DataType>, InterpError> {
        match obj {
            DataType::Array(arr) => {
                match method {
                    "map" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "map".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let mut result = Vec::with_capacity(arr.len().min(MAX_ARRAY_ELEMENTS));
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            if result.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit { limit: format!("{} elements", MAX_ARRAY_ELEMENTS), actual: format!("more than {}", MAX_ARRAY_ELEMENTS), context: "map result".to_string(), span });
                            }
                            result.push(self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?);
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "filter" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "filter".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let mut result = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let keep = self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?;
                            if keep.to_bool() {
                                result.push(item.clone());
                                if result.len() >= MAX_ARRAY_ELEMENTS {
                                    return Err(InterpError::ResourceLimit { limit: format!("{} elements", MAX_ARRAY_ELEMENTS), actual: format!("{} elements", result.len()), context: "filter".to_string(), span });
                                }
                            }
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "reduce" => {
                        if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "reduce".to_string(), expected: "2".to_string(), actual: args.len(), span }); }
                        let mut acc = self.eval_expr(&args[0])?;
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            acc = self.call_lambda_with_args(&args[1], &[acc, item.clone()], span)?;
                        }
                        Ok(Some(acc))
                    }
                    "find" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "find".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let matches = self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?;
                            if matches.to_bool() {
                                return Ok(Some(item.clone()));
                            }
                        }
                        Ok(Some(DataType::Null))
                    }
                    "find_index" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "find_index".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        for (i, item) in arr.iter().enumerate() {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let matches = self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?;
                            if matches.to_bool() {
                                return Ok(Some(DataType::Int64(i as i64)));
                            }
                        }
                        Ok(Some(DataType::Null))
                    }
                    "any" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "any".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let matches = self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?;
                            if matches.to_bool() {
                                return Ok(Some(DataType::Bool(true)));
                            }
                        }
                        Ok(Some(DataType::Bool(false)))
                    }
                    "all" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "all".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let matches = self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?;
                            if !matches.to_bool() {
                                return Ok(Some(DataType::Bool(false)));
                            }
                        }
                        Ok(Some(DataType::Bool(true)))
                    }
                    "flat_map" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "flat_map".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let mut result = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let mapped = self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?;
                            match mapped {
                                DataType::Array(inner) => result.extend(inner),
                                other => result.push(other),
                            }
                            if result.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit {
                                    limit: format!("{} elements", MAX_ARRAY_ELEMENTS),
                                    actual: format!("{} elements", result.len()),
                                    context: "flat_map".to_string(),
                                    span,
                                });
                            }
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "each" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "each".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?;
                        }
                        Ok(Some(DataType::Null))
                    }
                    "sort_by" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "sort_by".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let sorted = arr.clone();
                        let max_comparisons = MAX_LOOP_ITERATIONS * 10; // 100,000 comparisons
                        let comparison_count = std::cell::Cell::new(0usize);
                        let result = self.merge_sort_by(sorted, &args[0], span, &comparison_count, max_comparisons)?;
                        Ok(Some(DataType::Array(result)))
                    }
                    "group_by" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "group_by".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let mut groups: indexmap::IndexMap<String, Vec<DataType>> = indexmap::IndexMap::new();
                        let mut total_items: usize = 0;
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            total_items += 1;
                            if total_items > MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit {
                                    limit: format!("{} elements", MAX_ARRAY_ELEMENTS),
                                    actual: format!("more than {}", MAX_ARRAY_ELEMENTS),
                                    context: "group_by".to_string(),
                                    span,
                                });
                            }
                            let key = self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?;
                            let key_str = key.to_string_lossy();
                            groups.entry(key_str).or_default().push(item.clone());
                        }
                        let map: indexmap::IndexMap<String, DataType> = groups.into_iter()
                            .map(|(k, v)| (k, DataType::Array(v)))
                            .collect();
                        Ok(Some(DataType::Map(map)))
                    }
                    "min_by" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "min_by".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        if arr.is_empty() { return Ok(Some(DataType::Null)); }
                        let mut min = arr[0].clone();
                        for item in &arr[1..] {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let cmp = self.call_lambda_with_args(&args[0], &[min.clone(), item.clone()], span)?;
                            let cmp_val = cmp.to_f64().ok_or_else(|| InterpError::TypeError {
                                expected: "number".to_string(),
                                actual: cmp.type_name().to_string(),
                                context: "min_by comparator must return a number".to_string(),
                                span,
                            })?;
                            if cmp_val.is_nan() {
                                return Err(InterpError::TypeError {
                                    expected: "finite number".to_string(),
                                    actual: "NaN".to_string(),
                                    context: "min_by comparator returned NaN".to_string(),
                                    span,
                                });
                            }
                            if cmp_val > 0.0 {
                                min = item.clone();
                            }
                        }
                        Ok(Some(min))
                    }
                    "max_by" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "max_by".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        if arr.is_empty() { return Ok(Some(DataType::Null)); }
                        let mut max = arr[0].clone();
                        for item in &arr[1..] {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let cmp = self.call_lambda_with_args(&args[0], &[max.clone(), item.clone()], span)?;
                            let cmp_val = cmp.to_f64().ok_or_else(|| InterpError::TypeError {
                                expected: "number".to_string(),
                                actual: cmp.type_name().to_string(),
                                context: "max_by comparator must return a number".to_string(),
                                span,
                            })?;
                            if cmp_val.is_nan() {
                                return Err(InterpError::TypeError {
                                    expected: "finite number".to_string(),
                                    actual: "NaN".to_string(),
                                    context: "max_by comparator returned NaN".to_string(),
                                    span,
                                });
                            }
                            if cmp_val < 0.0 {
                                max = item.clone();
                            }
                        }
                        Ok(Some(max))
                    }
                    "take_while" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "take_while".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let mut result = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let keep = self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?;
                            if !keep.to_bool() { break; }
                            if result.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit { limit: format!("{} elements", MAX_ARRAY_ELEMENTS), actual: format!("more than {}", MAX_ARRAY_ELEMENTS), context: "take_while result".to_string(), span });
                            }
                            result.push(item.clone());
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "skip_while" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "skip_while".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let mut skipping = true;
                        let mut result = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            if skipping {
                                let skip = self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?;
                                if skip.to_bool() { continue; }
                                skipping = false;
                            }
                            result.push(item.clone());
                            if result.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit { limit: format!("{} elements", MAX_ARRAY_ELEMENTS), actual: format!("{} elements", result.len()), context: "skip_while".to_string(), span });
                            }
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "partition" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "partition".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let mut trues = Vec::new();
                        let mut falses = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let matches = self.call_lambda_with_args(&args[0], std::slice::from_ref(item), span)?;
                            if matches.to_bool() {
                                trues.push(item.clone());
                            } else {
                                falses.push(item.clone());
                            }
                            if trues.len().saturating_add(falses.len()) >= MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit { limit: format!("{} elements", MAX_ARRAY_ELEMENTS), actual: format!("{} elements", trues.len() + falses.len()), context: "partition".to_string(), span });
                            }
                        }
                        Ok(Some(DataType::Array(vec![
                            DataType::Array(trues),
                            DataType::Array(falses),
                        ])))
                    }
                    "scan" => {
                        if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "scan".to_string(), expected: "2".to_string(), actual: args.len(), span }); }
                        let mut acc = self.eval_expr(&args[0])?;
                        let mut result = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            acc = self.call_lambda_with_args(&args[1], &[acc.clone(), item.clone()], span)?;
                            result.push(acc.clone());
                            if result.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit {
                                    limit: format!("{} elements", MAX_ARRAY_ELEMENTS),
                                    actual: format!("{} elements", result.len()),
                                    context: "scan".to_string(),
                                    span,
                                });
                            }
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "enumerate" => {
                        if !args.is_empty() { return Err(InterpError::ArityMismatch { name: "enumerate".to_string(), expected: "0".to_string(), actual: args.len(), span }); }
                        let cap = arr.len().min(MAX_ARRAY_ELEMENTS);
                        let mut result = Vec::with_capacity(cap);
                        for (i, item) in arr.iter().enumerate() {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            result.push(DataType::Array(vec![DataType::Int64(i as i64), item.clone()]));
                            if result.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit { limit: format!("{} elements", MAX_ARRAY_ELEMENTS), actual: format!("{} elements", result.len()), context: "enumerate".to_string(), span });
                            }
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "zip" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "zip".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let other = self.eval_expr(&args[0])?;
                        let other_arr = match other {
                            DataType::Array(a) => a,
                            _ => return Err(InterpError::TypeError {
                                expected: "Array".to_string(),
                                actual: other.type_name().to_string(),
                                context: "zip argument".to_string(),
                                span,
                            }),
                        };
                        let cap = arr.len().min(other_arr.len()).min(MAX_ARRAY_ELEMENTS);
                        let mut result = Vec::with_capacity(cap);
                        for (a_item, b_item) in arr.iter().zip(other_arr.iter()) {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            result.push(DataType::Array(vec![a_item.clone(), b_item.clone()]));
                            if result.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit { limit: format!("{} elements", MAX_ARRAY_ELEMENTS), actual: format!("{} elements", result.len()), context: "zip".to_string(), span });
                            }
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "chunk" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "chunk".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let size_val = self.eval_expr(&args[0])?;
                        let size_i64 = size_val.to_i64().ok_or_else(|| InterpError::TypeError {
                            expected: "integer".to_string(),
                            actual: size_val.type_name().to_string(),
                            context: "chunk size".to_string(),
                            span,
                        })?;
                        if size_i64 <= 0 {
                            return Err(InterpError::TypeError {
                                expected: "positive integer".to_string(),
                                actual: format!("{}", size_i64),
                                context: "chunk size".to_string(),
                                span,
                            });
                        }
                        let size = usize::try_from(size_i64).unwrap_or(usize::MAX);
                        let mut result = Vec::new();
                        for chunk in arr.chunks(size) {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            result.push(DataType::Array(chunk.to_vec()));
                            if result.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit { limit: format!("{} elements", MAX_ARRAY_ELEMENTS), actual: format!("{} elements", result.len()), context: "chunk".to_string(), span });
                            }
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    _ => Ok(None),
                }
            }
            DataType::Map(map) => {
                match method {
                    "filter_entries" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "filter_entries".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let mut result = indexmap::IndexMap::new();
                        for (k, v) in map {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let keep = self.call_lambda_with_args(&args[0], &[DataType::String(k.clone()), v.clone()], span)?;
                            if keep.to_bool() {
                                result.insert(k.clone(), v.clone());
                                if result.len() >= MAX_ARRAY_ELEMENTS {
                                    return Err(InterpError::ResourceLimit {
                                        limit: format!("{} entries", MAX_ARRAY_ELEMENTS),
                                        actual: format!("{} entries", result.len()),
                                        context: "filter_entries".to_string(),
                                        span,
                                    });
                                }
                            }
                        }
                        Ok(Some(DataType::Map(result)))
                    }
                    "map_values" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "map_values".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let mut result = indexmap::IndexMap::new();
                        for (k, v) in map {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let new_v = self.call_lambda_with_args(&args[0], std::slice::from_ref(v), span)?;
                            result.insert(k.clone(), new_v);
                            if result.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit {
                                    limit: format!("{} entries", MAX_ARRAY_ELEMENTS),
                                    actual: format!("{} entries", result.len()),
                                    context: "map_values".to_string(),
                                    span,
                                });
                            }
                        }
                        Ok(Some(DataType::Map(result)))
                    }
                    "map_keys" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "map_keys".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let mut result = indexmap::IndexMap::new();
                        for (k, v) in map {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let new_k = self.call_lambda_with_args(&args[0], &[DataType::String(k.clone())], span)?;
                            let key_str = match new_k {
                                DataType::String(s) => s,
                                other => other.to_string_lossy(),
                            };
                            result.insert(key_str, v.clone());
                            if result.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(InterpError::ResourceLimit {
                                    limit: format!("{} entries", MAX_ARRAY_ELEMENTS),
                                    actual: format!("{} entries", result.len()),
                                    context: "map_keys".to_string(),
                                    span,
                                });
                            }
                        }
                        Ok(Some(DataType::Map(result)))
                    }
                    "map_entries" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "map_entries".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let mut result = indexmap::IndexMap::new();
                        for (k, v) in map {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let mapped = self.call_lambda_with_args(&args[0], &[DataType::String(k.clone()), v.clone()], span)?;
                            match mapped {
                                DataType::Array(pair) if pair.len() >= 2 => {
                                    let new_key = match &pair[0] { DataType::String(s) => s.clone(), other => other.to_string_lossy() };
                                    result.insert(new_key, pair[1].clone());
                                }
                                _ => return Err(InterpError::TypeError { expected: "[key, value] pair".to_string(), actual: mapped.type_name().to_string(), context: "map_entries callback must return a 2-element array".to_string(), span }),
                            }
                        }
                        Ok(Some(DataType::Map(result)))
                    }
                    // --- Non-HOF map methods (#113-120) ---
                    "invert" => {
                        let mut result = indexmap::IndexMap::new();
                        for (k, v) in map {
                            let new_key = match v { DataType::String(s) => s.clone(), other => other.to_string_lossy() };
                            result.insert(new_key, DataType::String(k.clone()));
                        }
                        Ok(Some(DataType::Map(result)))
                    }
                    "defaults" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "defaults".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let defaults_val = self.eval_expr(&args[0])?;
                        let defaults_map = match defaults_val { DataType::Map(m) => m, _ => return Err(InterpError::TypeError { expected: "Map".to_string(), actual: defaults_val.type_name().to_string(), context: "defaults argument".to_string(), span }) };
                        let mut result = map.clone();
                        for (k, v) in defaults_map {
                            result.entry(k).or_insert(v);
                        }
                        Ok(Some(DataType::Map(result)))
                    }
                    "pick" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pick".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let keys_val = self.eval_expr(&args[0])?;
                        let keys = match keys_val { DataType::Array(a) => a, _ => return Err(InterpError::TypeError { expected: "Array".to_string(), actual: keys_val.type_name().to_string(), context: "pick keys".to_string(), span }) };
                        let mut result = indexmap::IndexMap::new();
                        for k in keys {
                            let key_str = match k { DataType::String(s) => s, other => other.to_string_lossy() };
                            if let Some(v) = map.get(&key_str) { result.insert(key_str, v.clone()); }
                        }
                        Ok(Some(DataType::Map(result)))
                    }
                    "omit" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "omit".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let keys_val = self.eval_expr(&args[0])?;
                        let keys = match keys_val { DataType::Array(a) => a, _ => return Err(InterpError::TypeError { expected: "Array".to_string(), actual: keys_val.type_name().to_string(), context: "omit keys".to_string(), span }) };
                        let omit_set: std::collections::HashSet<String> = keys.into_iter().map(|k| match k { DataType::String(s) => s, other => other.to_string_lossy() }).collect();
                        let result: indexmap::IndexMap<String, DataType> = map.iter().filter(|(k, _)| !omit_set.contains(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
                        Ok(Some(DataType::Map(result)))
                    }
                    "deep_merge" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "deep_merge".to_string(), expected: "1".to_string(), actual: 0, span }); }
                        let other_val = self.eval_expr(&args[0])?;
                        let other_map = match other_val { DataType::Map(m) => m, _ => return Err(InterpError::TypeError { expected: "Map".to_string(), actual: other_val.type_name().to_string(), context: "deep_merge argument".to_string(), span }) };
                        fn deep_merge_maps(base: &indexmap::IndexMap<String, DataType>, overlay: &indexmap::IndexMap<String, DataType>) -> indexmap::IndexMap<String, DataType> {
                            let mut result = base.clone();
                            for (k, v) in overlay {
                                match (result.get(k), v) {
                                    (Some(DataType::Map(existing)), DataType::Map(incoming)) => {
                                        result.insert(k.clone(), DataType::Map(deep_merge_maps(existing, incoming)));
                                    }
                                    _ => { result.insert(k.clone(), v.clone()); }
                                }
                            }
                            result
                        }
                        Ok(Some(DataType::Map(deep_merge_maps(map, &other_map))))
                    }
                    "flatten_keys" => {
                        let separator = if !args.is_empty() {
                            match self.eval_expr(&args[0])? { DataType::String(s) => s, _ => ".".to_string() }
                        } else { ".".to_string() };
                        fn flatten_map(map: &indexmap::IndexMap<String, DataType>, prefix: &str, sep: &str, out: &mut indexmap::IndexMap<String, DataType>) {
                            for (k, v) in map {
                                let key = if prefix.is_empty() { k.clone() } else { format!("{}{}{}", prefix, sep, k) };
                                match v {
                                    DataType::Map(inner) => flatten_map(inner, &key, sep, out),
                                    _ => { out.insert(key, v.clone()); }
                                }
                            }
                        }
                        let mut result = indexmap::IndexMap::new();
                        flatten_map(map, "", &separator, &mut result);
                        Ok(Some(DataType::Map(result)))
                    }
                    _ => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    // =========================================================================
    // Numeric method helpers — shared min/max/clamp for all integer and float types
    // =========================================================================

    /// Evaluate min/max/clamp for integer types using i128 as the common representation.
    fn eval_int_min_max_clamp(&mut self, val: i128, method: &str, args: &[Expression], span: Span) -> Result<i128, InterpError> {
        match method {
            "min" => {
                if args.is_empty() { return Err(InterpError::ArityMismatch { name: "min".to_string(), expected: "1".to_string(), actual: 0, span }); }
                let arg = self.eval_expr(&args[0])?;
                let other = arg.to_i128()
                    .ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "min argument".to_string(), span })?;
                Ok(val.min(other))
            }
            "max" => {
                if args.is_empty() { return Err(InterpError::ArityMismatch { name: "max".to_string(), expected: "1".to_string(), actual: 0, span }); }
                let arg = self.eval_expr(&args[0])?;
                let other = arg.to_i128()
                    .ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "max argument".to_string(), span })?;
                Ok(val.max(other))
            }
            "clamp" => {
                if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "clamp".to_string(), expected: "2".to_string(), actual: args.len(), span }); }
                let lo_arg = self.eval_expr(&args[0])?;
                let hi_arg = self.eval_expr(&args[1])?;
                let min_val = lo_arg.to_i128()
                    .ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: lo_arg.type_name().to_string(), context: "clamp min bound".to_string(), span })?;
                let max_val = hi_arg.to_i128()
                    .ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: hi_arg.type_name().to_string(), context: "clamp max bound".to_string(), span })?;
                let (lo, hi) = if min_val <= max_val { (min_val, max_val) } else { (max_val, min_val) };
                Ok(val.max(lo).min(hi))
            }
            _ => Err(InterpError::TypeError { expected: "min, max, or clamp".to_string(), actual: method.to_string(), context: "integer method".to_string(), span }),
        }
    }

    /// Evaluate min/max/clamp for float types using f64 as the common representation.
    fn eval_float_min_max_clamp(&mut self, val: f64, method: &str, args: &[Expression], span: Span) -> Result<f64, InterpError> {
        match method {
            "min" => {
                if args.is_empty() { return Err(InterpError::ArityMismatch { name: "min".to_string(), expected: "1".to_string(), actual: 0, span }); }
                let arg = self.eval_expr(&args[0])?;
                let other = arg.to_f64()
                    .ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "min argument".to_string(), span })?;
                Ok(val.min(other))
            }
            "max" => {
                if args.is_empty() { return Err(InterpError::ArityMismatch { name: "max".to_string(), expected: "1".to_string(), actual: 0, span }); }
                let arg = self.eval_expr(&args[0])?;
                let other = arg.to_f64()
                    .ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "max argument".to_string(), span })?;
                Ok(val.max(other))
            }
            "clamp" => {
                if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "clamp".to_string(), expected: "2".to_string(), actual: args.len(), span }); }
                let lo_arg = self.eval_expr(&args[0])?;
                let hi_arg = self.eval_expr(&args[1])?;
                let min_val = lo_arg.to_f64()
                    .ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: lo_arg.type_name().to_string(), context: "clamp min bound".to_string(), span })?;
                let max_val = hi_arg.to_f64()
                    .ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: hi_arg.type_name().to_string(), context: "clamp max bound".to_string(), span })?;
                if min_val.is_nan() || max_val.is_nan() { return Ok(f64::NAN); }
                let (lo, hi) = if min_val <= max_val { (min_val, max_val) } else { (max_val, min_val) };
                Ok(val.max(lo).min(hi))
            }
            _ => Err(InterpError::TypeError { expected: "min, max, or clamp".to_string(), actual: method.to_string(), context: "float method".to_string(), span }),
        }
    }

    // =========================================================================
    // Direct interpreter methods (Phase 13, 16)
    // =========================================================================

    fn try_eval_direct_method(&mut self, obj: &DataType, method: &str, args: &[Expression], span: Span) -> Result<Option<DataType>, InterpError> {
        match obj {
            // Number methods (Phase 13)
            DataType::Int64(n) => match method {
                "abs" => match n.checked_abs() {
                    Some(v) => Ok(Some(DataType::Int64(v))),
                    None => Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("abs(i64::MIN)".to_string()), span }),
                },
                "to_string" => Ok(Some(DataType::String(n.to_string()))),
                "to_float64" => Ok(Some(DataType::Float64(*n as f64))),
                "to_int64" => Ok(Some(DataType::Int64(*n))),
                "pow" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pow".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let exp = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "pow exponent".to_string(), span })?;
                    if exp < 0 {
                        // Negative exponents on integers: 0^-n is undefined, |n|>1 → 0
                        if *n == 0 { Ok(Some(DataType::Null)) }
                        else if *n == 1 { Ok(Some(DataType::Int64(1))) }
                        else if *n == -1 { Ok(Some(DataType::Int64(if exp % 2 == 0 { 1 } else { -1 }))) }
                        else { Ok(Some(DataType::Int64(0))) }
                    } else if exp > u32::MAX as i64 {
                        Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("pow exponent too large".to_string()), span })
                    } else {
                        match n.checked_pow(exp as u32) {
                            Some(result) => Ok(Some(DataType::Int64(result))),
                            None => Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("integer pow overflow".to_string()), span }),
                        }
                    }
                }
                "min" | "max" | "clamp" => {
                    let result = self.eval_int_min_max_clamp(*n as i128, method, args, span)?;
                    Ok(Some(DataType::Int64(result.max(i64::MIN as i128).min(i64::MAX as i128) as i64)))
                }
                "sign" => Ok(Some(DataType::Int64(n.signum()))),
                _ => Ok(None),
            },
            DataType::Float64(n) => match method {
                "abs" => Ok(Some(DataType::Float64(n.abs()))),
                "round" => Ok(Some(DataType::Float64(n.round()))),
                "floor" => Ok(Some(DataType::Float64(n.floor()))),
                "ceil" => Ok(Some(DataType::Float64(n.ceil()))),
                "sqrt" => Ok(Some(DataType::Float64(n.sqrt()))),
                "is_nan" => Ok(Some(DataType::Bool(n.is_nan()))),
                "is_infinite" => Ok(Some(DataType::Bool(n.is_infinite()))),
                "is_finite" => Ok(Some(DataType::Bool(n.is_finite()))),
                "to_string" => Ok(Some(DataType::String(n.to_string()))),
                "to_float64" => Ok(Some(DataType::Float64(*n))),
                "to_float32" => Ok(Some(DataType::Float32(*n as f32))),
                "to_int64" => {
                    if !n.is_finite() || *n >= i64::MAX as f64 || *n < i64::MIN as f64 {
                        Ok(Some(DataType::Null))
                    } else {
                        Ok(Some(DataType::Int64(*n as i64)))
                    }
                }
                "pow" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pow".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let exp = arg.to_f64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "pow exponent".to_string(), span })?;
                    Ok(Some(DataType::Float64(n.powf(exp))))
                }
                "min" | "max" | "clamp" => {
                    let result = self.eval_float_min_max_clamp(*n as f64, method, args, span)?;
                    Ok(Some(DataType::Float64(result)))
                }
                "sign" => Ok(Some(DataType::Float64(n.signum()))),
                "ln" => Ok(Some(DataType::Float64(n.ln()))),
                "log2" => Ok(Some(DataType::Float64(n.log2()))),
                "log10" => Ok(Some(DataType::Float64(n.log10()))),
                "sin" => Ok(Some(DataType::Float64(n.sin()))),
                "cos" => Ok(Some(DataType::Float64(n.cos()))),
                "tan" => Ok(Some(DataType::Float64(n.tan()))),
                "asin" => Ok(Some(DataType::Float64(n.asin()))),
                "acos" => Ok(Some(DataType::Float64(n.acos()))),
                "atan" => Ok(Some(DataType::Float64(n.atan()))),
                "sinh" => Ok(Some(DataType::Float64(n.sinh()))),
                "cosh" => Ok(Some(DataType::Float64(n.cosh()))),
                "tanh" => Ok(Some(DataType::Float64(n.tanh()))),
                "exp" => Ok(Some(DataType::Float64(n.exp()))),
                _ => Ok(None),
            },
            DataType::Float32(n) => match method {
                "abs" => Ok(Some(DataType::Float32(n.abs()))),
                "round" => Ok(Some(DataType::Float32(n.round()))),
                "floor" => Ok(Some(DataType::Float32(n.floor()))),
                "ceil" => Ok(Some(DataType::Float32(n.ceil()))),
                "sqrt" => Ok(Some(DataType::Float32(n.sqrt()))),
                "is_nan" => Ok(Some(DataType::Bool(n.is_nan()))),
                "is_infinite" => Ok(Some(DataType::Bool(n.is_infinite()))),
                "is_finite" => Ok(Some(DataType::Bool(n.is_finite()))),
                "sign" => Ok(Some(DataType::Float32(n.signum()))),
                "to_string" => Ok(Some(DataType::String(n.to_string()))),
                "to_float32" => Ok(Some(DataType::Float32(*n))),
                "to_float64" => Ok(Some(DataType::Float64(*n as f64))),
                "to_int64" => {
                    let v = *n as f64;
                    if v.is_finite() && v >= i64::MIN as f64 && v < i64::MAX as f64 {
                        Ok(Some(DataType::Int64(v as i64)))
                    } else {
                        Ok(Some(DataType::Null))
                    }
                }
                "pow" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pow".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let exp = arg.to_f64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "pow exponent".to_string(), span })?;
                    Ok(Some(DataType::Float32(n.powf(exp as f32))))
                }
                "min" | "max" | "clamp" => {
                    let result = self.eval_float_min_max_clamp(*n as f64, method, args, span)?;
                    Ok(Some(DataType::Float32(result as f32)))
                }
                "ln" => Ok(Some(DataType::Float32(n.ln()))),
                "log2" => Ok(Some(DataType::Float32(n.log2()))),
                "log10" => Ok(Some(DataType::Float32(n.log10()))),
                "sin" => Ok(Some(DataType::Float32(n.sin()))),
                "cos" => Ok(Some(DataType::Float32(n.cos()))),
                "tan" => Ok(Some(DataType::Float32(n.tan()))),
                "asin" => Ok(Some(DataType::Float32(n.asin()))),
                "acos" => Ok(Some(DataType::Float32(n.acos()))),
                "atan" => Ok(Some(DataType::Float32(n.atan()))),
                "sinh" => Ok(Some(DataType::Float32(n.sinh()))),
                "cosh" => Ok(Some(DataType::Float32(n.cosh()))),
                "tanh" => Ok(Some(DataType::Float32(n.tanh()))),
                "exp" => Ok(Some(DataType::Float32(n.exp()))),
                _ => Ok(None),
            },
            DataType::Int32(n) => match method {
                "abs" => match n.checked_abs() {
                    Some(v) => Ok(Some(DataType::Int32(v))),
                    None => Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("abs(i32::MIN)".to_string()), span }),
                },
                "sign" => Ok(Some(DataType::Int32(n.signum()))),
                "to_string" => Ok(Some(DataType::String(n.to_string()))),
                "to_int32" => Ok(Some(DataType::Int32(*n))),
                "to_float64" => Ok(Some(DataType::Float64(*n as f64))),
                "to_int64" => Ok(Some(DataType::Int64(*n as i64))),
                "pow" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pow".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let exp = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "pow exponent".to_string(), span })?;
                    if exp < 0 {
                        if *n == 0 { Ok(Some(DataType::Null)) }
                        else if *n == 1 { Ok(Some(DataType::Int32(1))) }
                        else if *n == -1 { Ok(Some(DataType::Int32(if exp % 2 == 0 { 1 } else { -1 }))) }
                        else { Ok(Some(DataType::Int32(0))) }
                    } else if exp > u32::MAX as i64 {
                        Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("pow exponent too large".to_string()), span })
                    } else {
                        match n.checked_pow(exp as u32) {
                            Some(result) => Ok(Some(DataType::Int32(result))),
                            None => Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("integer pow overflow".to_string()), span }),
                        }
                    }
                }
                "min" | "max" | "clamp" => {
                    let result = self.eval_int_min_max_clamp(*n as i128, method, args, span)?;
                    Ok(Some(DataType::Int32(result.max(i32::MIN as i128).min(i32::MAX as i128) as i32)))
                }
                _ => Ok(None),
            },
            DataType::Uint32(n) => match method {
                "to_string" => Ok(Some(DataType::String(n.to_string()))),
                "to_uint32" => Ok(Some(DataType::Uint32(*n))),
                "to_float64" => Ok(Some(DataType::Float64(*n as f64))),
                "to_int64" => Ok(Some(DataType::Int64(*n as i64))),
                "abs" => Ok(Some(DataType::Uint32(*n))),
                "sign" => Ok(Some(DataType::Uint32(if *n == 0 { 0 } else { 1 }))),
                "pow" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pow".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let exp = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "pow exponent".to_string(), span })?;
                    if exp < 0 {
                        if *n == 0 { Ok(Some(DataType::Null)) }
                        else if *n == 1 { Ok(Some(DataType::Uint32(1))) }
                        else { Ok(Some(DataType::Uint32(0))) }
                    }
                    else if exp > u32::MAX as i64 { Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("pow exponent too large".to_string()), span }) }
                    else {
                        match n.checked_pow(exp as u32) {
                            Some(result) => Ok(Some(DataType::Uint32(result))),
                            None => Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("integer pow overflow".to_string()), span }),
                        }
                    }
                }
                "min" | "max" | "clamp" => {
                    let result = self.eval_int_min_max_clamp(*n as i128, method, args, span)?;
                    Ok(Some(DataType::Uint32(result.max(0).min(u32::MAX as i128) as u32)))
                }
                _ => Ok(None),
            },
            DataType::Uint64(n) => match method {
                "to_string" => Ok(Some(DataType::String(n.to_string()))),
                "to_uint64" => Ok(Some(DataType::Uint64(*n))),
                "to_float64" => Ok(Some(DataType::Float64(*n as f64))),
                "to_int64" => {
                    if *n > i64::MAX as u64 {
                        Ok(Some(DataType::Null))
                    } else {
                        Ok(Some(DataType::Int64(*n as i64)))
                    }
                }
                "abs" => Ok(Some(DataType::Uint64(*n))),
                "sign" => Ok(Some(DataType::Uint64(if *n == 0 { 0 } else { 1 }))),
                "pow" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pow".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let exp = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "pow exponent".to_string(), span })?;
                    if exp < 0 {
                        if *n == 0 { Ok(Some(DataType::Null)) }
                        else if *n == 1 { Ok(Some(DataType::Uint64(1))) }
                        else { Ok(Some(DataType::Uint64(0))) }
                    }
                    else if exp > u32::MAX as i64 { Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("pow exponent too large".to_string()), span }) }
                    else {
                        match n.checked_pow(exp as u32) {
                            Some(result) => Ok(Some(DataType::Uint64(result))),
                            None => Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("integer pow overflow".to_string()), span }),
                        }
                    }
                }
                "min" | "max" | "clamp" => {
                    let result = self.eval_int_min_max_clamp(*n as i128, method, args, span)?;
                    Ok(Some(DataType::Uint64(result.max(0).min(u64::MAX as i128) as u64)))
                }
                _ => Ok(None),
            },
            // String methods (Phase 16+)
            DataType::String(s) => match method {
                "is_empty" => Ok(Some(DataType::Bool(s.is_empty()))),
                "is_numeric" => Ok(Some(DataType::Bool(!s.is_empty() && s.as_str() == s.trim() && s.parse::<f64>().is_ok_and(|f| f.is_finite())))),
                "is_alphabetic" => Ok(Some(DataType::Bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic())))),
                "to_int" => Ok(Some(s.trim().parse::<i64>().map(DataType::Int64).unwrap_or(DataType::Null))),
                "to_float" => Ok(Some(s.trim().parse::<f64>().map(DataType::Float64).unwrap_or(DataType::Null))),
                "len" | "length" => Ok(Some(DataType::Int64(s.chars().count() as i64))),
                "trim" => Ok(Some(DataType::String(s.trim().to_string()))),
                "trim_start" => Ok(Some(DataType::String(s.trim_start().to_string()))),
                "trim_end" => Ok(Some(DataType::String(s.trim_end().to_string()))),
                "to_upper" | "to_uppercase" => {
                    // Check input length first to avoid allocating huge strings
                    // (case conversion can expand at most ~3x for edge cases like ß→SS)
                    if s.len() > MAX_STRING_OUTPUT {
                        return Err(InterpError::ResourceLimit {
                            limit: format!("{} bytes", MAX_STRING_OUTPUT),
                            actual: format!("{}", s.len()),
                            context: "to_uppercase input".to_string(),
                            span,
                        });
                    }
                    Ok(Some(DataType::String(s.to_uppercase())))
                },
                "to_lower" | "to_lowercase" => {
                    if s.len() > MAX_STRING_OUTPUT {
                        return Err(InterpError::ResourceLimit {
                            limit: format!("{} bytes", MAX_STRING_OUTPUT),
                            actual: format!("{}", s.len()),
                            context: "to_lowercase input".to_string(),
                            span,
                        });
                    }
                    Ok(Some(DataType::String(s.to_lowercase())))
                },
                "reverse" => Ok(Some(DataType::String(s.chars().rev().collect()))),
                "chars" => {
                    let chars: Vec<DataType> = s.chars().take(MAX_ARRAY_ELEMENTS + 1).map(|c| DataType::String(c.to_string())).collect();
                    if chars.len() > MAX_ARRAY_ELEMENTS {
                        return Err(InterpError::ResourceLimit { limit: format!("{} chars", MAX_ARRAY_ELEMENTS), actual: format!("more than {}", MAX_ARRAY_ELEMENTS), context: "string chars".to_string(), span });
                    }
                    Ok(Some(DataType::Array(chars)))
                }
                "lines" => {
                    let lines: Vec<DataType> = s.lines().take(MAX_ARRAY_ELEMENTS + 1).map(|l| DataType::String(l.to_string())).collect();
                    if lines.len() > MAX_ARRAY_ELEMENTS {
                        return Err(InterpError::ResourceLimit { limit: format!("{} lines", MAX_ARRAY_ELEMENTS), actual: format!("more than {}", MAX_ARRAY_ELEMENTS), context: "string lines".to_string(), span });
                    }
                    Ok(Some(DataType::Array(lines)))
                }
                "to_string" => Ok(Some(DataType::String(s.clone()))),
                "split" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "split".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let sep_val = self.eval_expr(&args[0])?;
                    let sep = match sep_val {
                        DataType::String(sep) => sep,
                        other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "split separator".to_string(), span }),
                    };
                    if sep.is_empty() {
                        return Err(InterpError::TypeError { expected: "non-empty separator".to_string(), actual: "empty string".to_string(), context: "split separator".to_string(), span });
                    }
                    let parts: Vec<DataType> = s.split(&sep).take(MAX_ARRAY_ELEMENTS + 1).map(|p| DataType::String(p.to_string())).collect();
                    if parts.len() > MAX_ARRAY_ELEMENTS {
                        return Err(InterpError::ResourceLimit { limit: format!("{} elements", MAX_ARRAY_ELEMENTS), actual: format!("more than {}", MAX_ARRAY_ELEMENTS), context: "string split".to_string(), span });
                    }
                    Ok(Some(DataType::Array(parts)))
                }
                "replace" => {
                    if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "replace".to_string(), expected: "2".to_string(), actual: args.len(), span }); }
                    let from = match self.eval_expr(&args[0])? { DataType::String(s) => s, other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "replace pattern".to_string(), span }) };
                    let to = match self.eval_expr(&args[1])? { DataType::String(s) => s, other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "replace replacement".to_string(), span }) };
                    if from.is_empty() {
                        // Rust's replace("", x) inserts x between every char and at both ends
                        let result_len = s.len().saturating_add((s.chars().count() + 1).saturating_mul(to.len()));
                        if result_len > MAX_STRING_OUTPUT {
                            return Err(InterpError::ResourceLimit { limit: format!("{} bytes", MAX_STRING_OUTPUT), actual: format!("{}", result_len), context: "string replace".to_string(), span });
                        }
                    }
                    // Single-pass replace then check result size (#366)
                    let result = s.replace(&from, &to);
                    if result.len() > MAX_STRING_OUTPUT {
                        return Err(InterpError::ResourceLimit { limit: format!("{} bytes", MAX_STRING_OUTPUT), actual: format!("{}", result.len()), context: "string replace".to_string(), span });
                    }
                    Ok(Some(DataType::String(result)))
                }
                "contains" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "contains".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let needle = match self.eval_expr(&args[0])? { DataType::String(s) => s, other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "contains argument".to_string(), span }) };
                    Ok(Some(DataType::Bool(s.contains(&needle))))
                }
                "starts_with" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "starts_with".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let prefix = match self.eval_expr(&args[0])? { DataType::String(s) => s, other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "starts_with argument".to_string(), span }) };
                    Ok(Some(DataType::Bool(s.starts_with(&prefix))))
                }
                "ends_with" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "ends_with".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let suffix = match self.eval_expr(&args[0])? { DataType::String(s) => s, other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "ends_with argument".to_string(), span }) };
                    Ok(Some(DataType::Bool(s.ends_with(&suffix))))
                }
                "index_of" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "index_of".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let needle = match self.eval_expr(&args[0])? { DataType::String(s) => s, other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "index_of argument".to_string(), span }) };
                    Ok(Some(match s.find(&needle) {
                        Some(byte_idx) => DataType::Int64(s[..byte_idx].chars().count() as i64),
                        None => DataType::Null,
                    }))
                }
                "repeat" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "repeat".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let n = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "repeat count".to_string(), span })?.max(0) as usize;
                    const MAX_REPEAT_LEN: usize = 10_000_000;
                    if n > 0 && s.len().saturating_mul(n) > MAX_REPEAT_LEN {
                        return Err(InterpError::ResourceLimit {
                            limit: format!("{} bytes", MAX_REPEAT_LEN),
                            actual: format!("{} * {} = {}", s.len(), n, s.len().saturating_mul(n)),
                            context: "string repeat".to_string(),
                            span,
                        });
                    }
                    Ok(Some(DataType::String(s.repeat(n))))
                }
                "char_at" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "char_at".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let idx = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "char_at index".to_string(), span })?;
                    if idx < 0 {
                        Ok(Some(DataType::Null))
                    } else {
                        Ok(Some(s.chars().nth(idx as usize).map(|c| DataType::String(c.to_string())).unwrap_or(DataType::Null)))
                    }
                }
                "pad_start" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pad_start".to_string(), expected: "1-2".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let width = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "pad_start width".to_string(), span })?.max(0) as usize;
                    const MAX_PAD_WIDTH: usize = 10_000_000;
                    if width > MAX_PAD_WIDTH {
                        return Err(InterpError::ResourceLimit {
                            limit: format!("{}", MAX_PAD_WIDTH),
                            actual: format!("{}", width),
                            context: "pad_start width".to_string(),
                            span,
                        });
                    }
                    let pad_str = if args.len() > 1 {
                        let fill = self.eval_expr(&args[1])?;
                        match fill {
                            DataType::String(c) if !c.is_empty() => c,
                            DataType::String(_) => " ".to_string(),
                            other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "pad_start fill".to_string(), span }),
                        }
                    } else { " ".to_string() };
                    let pad_len = width.saturating_sub(s.chars().count());
                    let max_pad_bytes = pad_len.saturating_mul(pad_str.len());
                    if s.len().saturating_add(max_pad_bytes) > MAX_STRING_OUTPUT {
                        return Err(InterpError::ResourceLimit {
                            limit: format!("{} bytes", MAX_STRING_OUTPUT),
                            actual: format!("{}", s.len().saturating_add(max_pad_bytes)),
                            context: "pad_start result".to_string(),
                            span,
                        });
                    }
                    let padding: String = pad_str.chars().cycle().take(pad_len).collect();
                    Ok(Some(DataType::String(format!("{}{}", padding, s))))
                }
                "pad_end" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pad_end".to_string(), expected: "1-2".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let width = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "pad_end width".to_string(), span })?.max(0) as usize;
                    const MAX_PAD_WIDTH: usize = 10_000_000;
                    if width > MAX_PAD_WIDTH {
                        return Err(InterpError::ResourceLimit {
                            limit: format!("{}", MAX_PAD_WIDTH),
                            actual: format!("{}", width),
                            context: "pad_end width".to_string(),
                            span,
                        });
                    }
                    let pad_str = if args.len() > 1 {
                        let fill = self.eval_expr(&args[1])?;
                        match fill {
                            DataType::String(c) if !c.is_empty() => c,
                            DataType::String(_) => " ".to_string(),
                            other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "pad_end fill".to_string(), span }),
                        }
                    } else { " ".to_string() };
                    let pad_len = width.saturating_sub(s.chars().count());
                    let max_pad_bytes = pad_len.saturating_mul(pad_str.len());
                    if s.len().saturating_add(max_pad_bytes) > MAX_STRING_OUTPUT {
                        return Err(InterpError::ResourceLimit {
                            limit: format!("{} bytes", MAX_STRING_OUTPUT),
                            actual: format!("{}", s.len().saturating_add(max_pad_bytes)),
                            context: "pad_end result".to_string(),
                            span,
                        });
                    }
                    let padding: String = pad_str.chars().cycle().take(pad_len).collect();
                    Ok(Some(DataType::String(format!("{}{}", s, padding))))
                }
                "substring" | "slice" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "substring".to_string(), expected: "1-2".to_string(), actual: 0, span }); }
                    let char_len = s.chars().count() as i64;
                    let start_arg = self.eval_expr(&args[0])?;
                    let raw_start = start_arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: start_arg.type_name().to_string(), context: "substring start".to_string(), span })?;
                    let raw_end = if args.len() > 1 {
                        let end_arg = self.eval_expr(&args[1])?;
                        end_arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: end_arg.type_name().to_string(), context: "substring end".to_string(), span })?
                    } else { char_len };
                    // Support negative indices (count from end)
                    let start = if raw_start < 0 { (char_len + raw_start).max(0) as usize } else { usize::try_from(raw_start).unwrap_or(usize::MAX).min(char_len as usize) };
                    let end = if raw_end < 0 { (char_len + raw_end).max(0) as usize } else { usize::try_from(raw_end).unwrap_or(usize::MAX).min(char_len as usize) };
                    if start >= end {
                        Ok(Some(DataType::String(String::new())))
                    } else {
                        Ok(Some(DataType::String(s.chars().skip(start).take(end - start).collect())))
                    }
                }
                // --- New string methods (#96-102) ---
                "capitalize" => {
                    let mut chars = s.chars();
                    let result = match chars.next() {
                        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                        None => String::new(),
                    };
                    Ok(Some(DataType::String(result)))
                }
                "uncapitalize" => {
                    let mut chars = s.chars();
                    let result = match chars.next() {
                        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
                        None => String::new(),
                    };
                    Ok(Some(DataType::String(result)))
                }
                "pad_center" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pad_center".to_string(), expected: "1-2".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let width = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "pad_center width".to_string(), span })?.max(0) as usize;
                    let pad_char = if args.len() > 1 {
                        // SAFETY: guard `!p.is_empty()` guarantees at least one char
                        match self.eval_expr(&args[1])? { DataType::String(p) if !p.is_empty() => p.chars().next().unwrap(), _ => ' ' }
                    } else { ' ' };
                    let char_len = s.chars().count();
                    if char_len >= width {
                        Ok(Some(DataType::String(s.to_string())))
                    } else {
                        let total_pad = width - char_len;
                        let left_pad = total_pad / 2;
                        let right_pad = total_pad - left_pad;
                        let left: String = std::iter::repeat(pad_char).take(left_pad).collect();
                        let right: String = std::iter::repeat(pad_char).take(right_pad).collect();
                        Ok(Some(DataType::String(format!("{}{}{}", left, s, right))))
                    }
                }
                "truncate" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "truncate".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let max_len = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "truncate length".to_string(), span })?.max(0) as usize;
                    let suffix = if args.len() > 1 {
                        match self.eval_expr(&args[1])? { DataType::String(p) => p, _ => "...".to_string() }
                    } else { "...".to_string() };
                    let char_len = s.chars().count();
                    if char_len <= max_len {
                        Ok(Some(DataType::String(s.to_string())))
                    } else if max_len <= suffix.chars().count() {
                        Ok(Some(DataType::String(s.chars().take(max_len).collect())))
                    } else {
                        let take = max_len - suffix.chars().count();
                        let truncated: String = s.chars().take(take).collect();
                        Ok(Some(DataType::String(format!("{}{}", truncated, suffix))))
                    }
                }
                "count_words" => {
                    Ok(Some(DataType::Int64(s.split_whitespace().count() as i64)))
                }
                "strip_prefix" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "strip_prefix".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let prefix = match self.eval_expr(&args[0])? { DataType::String(p) => p, other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "strip_prefix argument".to_string(), span }) };
                    Ok(Some(DataType::String(s.strip_prefix(&*prefix).unwrap_or(s).to_string())))
                }
                "strip_suffix" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "strip_suffix".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let suffix = match self.eval_expr(&args[0])? { DataType::String(p) => p, other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "strip_suffix argument".to_string(), span }) };
                    Ok(Some(DataType::String(s.strip_suffix(&*suffix).unwrap_or(s).to_string())))
                }
                "byte_length" | "byte_len" => {
                    Ok(Some(DataType::Int64(s.len() as i64)))
                }
                _ => Ok(None),
            },
            // Array methods (direct, no OperationEvaluator needed)
            DataType::Array(arr) => match method {
                "first" => Ok(Some(arr.first().cloned().unwrap_or(DataType::Null))),
                "last" => Ok(Some(arr.last().cloned().unwrap_or(DataType::Null))),
                "is_empty" => Ok(Some(DataType::Bool(arr.is_empty()))),
                "sum" => {
                    let mut int_sum: i64 = 0;
                    let mut has_float = false;
                    let mut float_sum: f64 = 0.0;
                    let mut int_overflow = false;
                    for item in arr {
                        if self.is_cancelled() { return Err(InterpError::Cancelled); }
                        match item {
                            DataType::Float64(f) => { has_float = true; float_sum += f; }
                            DataType::Float32(f) => { has_float = true; float_sum += *f as f64; }
                            DataType::Uint64(u) => {
                                if let Ok(i) = i64::try_from(*u) {
                                    if !int_overflow {
                                        match int_sum.checked_add(i) {
                                            Some(v) => int_sum = v,
                                            None => {
                                                has_float = true;
                                                float_sum += int_sum as f64;
                                                int_overflow = true;
                                                float_sum += i as f64;
                                            }
                                        }
                                    } else { float_sum += i as f64; }
                                } else {
                                    has_float = true;
                                    if !int_overflow {
                                        float_sum += int_sum as f64;
                                        int_overflow = true;
                                    }
                                    float_sum += *u as f64;
                                }
                            }
                            _ => {
                                let val = item.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: item.type_name().to_string(), context: "sum element".to_string(), span })?;
                                if !int_overflow {
                                    match int_sum.checked_add(val) {
                                        Some(v) => int_sum = v,
                                        None => {
                                            has_float = true;
                                            float_sum += int_sum as f64;
                                            int_overflow = true;
                                            float_sum += val as f64;
                                        }
                                    }
                                } else {
                                    float_sum += val as f64;
                                }
                            }
                        }
                    }
                    if has_float {
                        Ok(Some(DataType::Float64(float_sum + if int_overflow { 0.0 } else { int_sum as f64 })))
                    } else {
                        Ok(Some(DataType::Int64(int_sum)))
                    }
                }
                "product" => {
                    let mut int_prod: i64 = 1;
                    let mut has_float = false;
                    let mut float_prod: f64 = 1.0;
                    let mut int_overflow = false;
                    for item in arr {
                        if self.is_cancelled() { return Err(InterpError::Cancelled); }
                        match item {
                            DataType::Float64(f) => { has_float = true; float_prod *= f; }
                            DataType::Float32(f) => { has_float = true; float_prod *= *f as f64; }
                            DataType::Uint64(u) => {
                                if let Ok(i) = i64::try_from(*u) {
                                    if !int_overflow {
                                        match int_prod.checked_mul(i) {
                                            Some(v) => int_prod = v,
                                            None => {
                                                has_float = true;
                                                float_prod *= int_prod as f64;
                                                int_overflow = true;
                                                float_prod *= i as f64;
                                            }
                                        }
                                    } else { float_prod *= i as f64; }
                                } else {
                                    has_float = true;
                                    if !int_overflow {
                                        float_prod *= int_prod as f64;
                                        int_overflow = true;
                                    }
                                    float_prod *= *u as f64;
                                }
                            }
                            _ => {
                                let val = item.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: item.type_name().to_string(), context: "product element".to_string(), span })?;
                                if !int_overflow {
                                    match int_prod.checked_mul(val) {
                                        Some(v) => int_prod = v,
                                        None => {
                                            has_float = true;
                                            float_prod *= int_prod as f64;
                                            int_overflow = true;
                                            float_prod *= val as f64;
                                        }
                                    }
                                } else {
                                    float_prod *= val as f64;
                                }
                            }
                        }
                    }
                    if has_float {
                        if !int_overflow {
                            Ok(Some(DataType::Float64(float_prod * int_prod as f64)))
                        } else {
                            Ok(Some(DataType::Float64(float_prod)))
                        }
                    } else {
                        Ok(Some(DataType::Int64(int_prod)))
                    }
                }
                "min" => {
                    if arr.is_empty() { return Ok(Some(DataType::Null)); }
                    let mut min = arr[0].clone();
                    for item in &arr[1..] {
                        if self.is_cancelled() { return Err(InterpError::Cancelled); }
                        let cmp = match (&min, item) {
                            (DataType::Int64(a), DataType::Int64(b)) => *a > *b,
                            // NaN handling: if current min is NaN, replace it; if item is NaN, skip it
                            (DataType::Float64(a), DataType::Float64(b)) => a.is_nan() || (!b.is_nan() && *a > *b),
                            (DataType::Int64(a), DataType::Float64(b)) => !b.is_nan() && (*a as f64) > *b,
                            (DataType::Float64(a), DataType::Int64(b)) => a.is_nan() || *a > (*b as f64),
                            (DataType::String(a), DataType::String(b)) => a > b,
                            // Numeric fallback for Float32/Int32/Uint32/Uint64
                            (a, b) => match (a.to_f64(), b.to_f64()) {
                                (Some(fa), Some(fb)) => fa.is_nan() || (!fb.is_nan() && fa > fb),
                                _ => return Err(InterpError::TypeError { expected: "comparable types (all numbers or all strings)".to_string(), actual: format!("{} and {}", min.type_name(), item.type_name()), context: "array min".to_string(), span }),
                            },
                        };
                        if cmp { min = item.clone(); }
                    }
                    Ok(Some(min))
                }
                "max" => {
                    if arr.is_empty() { return Ok(Some(DataType::Null)); }
                    let mut max = arr[0].clone();
                    for item in &arr[1..] {
                        if self.is_cancelled() { return Err(InterpError::Cancelled); }
                        let cmp = match (&max, item) {
                            (DataType::Int64(a), DataType::Int64(b)) => *a < *b,
                            // NaN handling: if current max is NaN, replace it; if item is NaN, skip it
                            (DataType::Float64(a), DataType::Float64(b)) => a.is_nan() || (!b.is_nan() && *a < *b),
                            (DataType::Int64(a), DataType::Float64(b)) => !b.is_nan() && (*a as f64) < *b,
                            (DataType::Float64(a), DataType::Int64(b)) => a.is_nan() || *a < (*b as f64),
                            (DataType::String(a), DataType::String(b)) => a < b,
                            // Numeric fallback for Float32/Int32/Uint32/Uint64
                            (a, b) => match (a.to_f64(), b.to_f64()) {
                                (Some(fa), Some(fb)) => fa.is_nan() || (!fb.is_nan() && fa < fb),
                                _ => return Err(InterpError::TypeError { expected: "comparable types (all numbers or all strings)".to_string(), actual: format!("{} and {}", max.type_name(), item.type_name()), context: "array max".to_string(), span }),
                            },
                        };
                        if cmp { max = item.clone(); }
                    }
                    Ok(Some(max))
                }
                "join" => {
                    if args.len() > 1 { return Err(InterpError::ArityMismatch { name: "join".to_string(), expected: "0-1".to_string(), actual: args.len(), span }); }
                    let separator = if !args.is_empty() {
                        match self.eval_expr(&args[0])? {
                            DataType::String(s) => s,
                            other => other.to_string_lossy(),
                        }
                    } else {
                        ",".to_string()
                    };
                    let mut parts = Vec::with_capacity(arr.len());
                    for v in arr {
                        if self.is_cancelled() { return Err(InterpError::Cancelled); }
                        parts.push(v.to_string_lossy());
                    }
                    let estimated_len: usize = parts.iter().map(|p| p.len()).sum::<usize>() + separator.len().saturating_mul(parts.len().saturating_sub(1));
                    if estimated_len > MAX_STRING_OUTPUT {
                        return Err(InterpError::ResourceLimit { limit: format!("{} bytes", MAX_STRING_OUTPUT), actual: format!("{}", estimated_len), context: "array join".to_string(), span });
                    }
                    Ok(Some(DataType::String(parts.join(&separator))))
                }
                "sort" => {
                    let mut sorted = arr.clone();
                    sorted.sort_by(|a, b| {
                        // Type tier: Null(0) < Bool(1) < Numeric(2) < String(3) < Array(4) < Map(5) < Bytes(6)
                        fn type_tier(v: &DataType) -> u8 {
                            match v {
                                DataType::Null => 0,
                                DataType::Bool(_) => 1,
                                DataType::Int64(_) | DataType::Int32(_) | DataType::Uint32(_) |
                                DataType::Uint64(_) | DataType::Float64(_) | DataType::Float32(_) => 2,
                                DataType::String(_) => 3,
                                DataType::Array(_) => 4,
                                DataType::Map(_) => 5,
                                DataType::Bytes(_) => 6,
                                DataType::Set(_) => 7,
                                DataType::Tuple(_) => 8,
                                DataType::Future(_) => 9,
                            }
                        }
                        let ta = type_tier(a);
                        let tb = type_tier(b);
                        if ta != tb {
                            return ta.cmp(&tb);
                        }
                        match (a, b) {
                            (DataType::Null, DataType::Null) => std::cmp::Ordering::Equal,
                            (DataType::Bool(x), DataType::Bool(y)) => x.cmp(y),
                            _ if ta == 2 => {
                                // Integer-exact comparison via i128 when both are integer types
                                fn to_i128(v: &DataType) -> Option<i128> {
                                    match v {
                                        DataType::Int64(x) => Some(*x as i128),
                                        DataType::Int32(x) => Some(*x as i128),
                                        DataType::Uint32(x) => Some(*x as i128),
                                        DataType::Uint64(x) => Some(*x as i128),
                                        _ => None,
                                    }
                                }
                                if let (Some(ai), Some(bi)) = (to_i128(a), to_i128(b)) {
                                    return ai.cmp(&bi);
                                }
                                // Fall back to f64 total_cmp for float types
                                let fa = a.to_f64().unwrap_or(0.0);
                                let fb = b.to_f64().unwrap_or(0.0);
                                fa.total_cmp(&fb)
                            }
                            (DataType::String(x), DataType::String(y)) => x.cmp(y),
                            _ => a.to_string_lossy().cmp(&b.to_string_lossy()),
                        }
                    });
                    Ok(Some(DataType::Array(sorted)))
                }
                "reverse" => {
                    let mut reversed = arr.clone();
                    reversed.reverse();
                    Ok(Some(DataType::Array(reversed)))
                }
                // --- New array methods (#104-111) ---
                "flatten" => {
                    let depth = if !args.is_empty() {
                        let arg = self.eval_expr(&args[0])?;
                        arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "flatten depth".to_string(), span })?.max(0) as usize
                    } else { usize::MAX };
                    fn flatten_recursive(arr: &[DataType], depth: usize, out: &mut Vec<DataType>, limit: usize) -> bool {
                        for item in arr {
                            if out.len() >= limit { return false; }
                            if depth > 0 {
                                if let DataType::Array(inner) = item {
                                    if !flatten_recursive(inner, depth - 1, out, limit) { return false; }
                                    continue;
                                }
                            }
                            out.push(item.clone());
                        }
                        true
                    }
                    let mut result = Vec::new();
                    flatten_recursive(arr, depth, &mut result, MAX_ARRAY_ELEMENTS);
                    if result.len() >= MAX_ARRAY_ELEMENTS {
                        return Err(InterpError::ResourceLimit { limit: format!("{} elements", MAX_ARRAY_ELEMENTS), actual: format!("{} elements", result.len()), context: "flatten".to_string(), span });
                    }
                    Ok(Some(DataType::Array(result)))
                }
                "rotate_left" => {
                    if arr.is_empty() { return Ok(Some(DataType::Array(vec![]))); }
                    let n = if !args.is_empty() {
                        let arg = self.eval_expr(&args[0])?;
                        arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "rotate_left count".to_string(), span })? as usize % arr.len()
                    } else { 1 };
                    let mut rotated = arr.clone();
                    rotated.rotate_left(n);
                    Ok(Some(DataType::Array(rotated)))
                }
                "rotate_right" => {
                    if arr.is_empty() { return Ok(Some(DataType::Array(vec![]))); }
                    let n = if !args.is_empty() {
                        let arg = self.eval_expr(&args[0])?;
                        arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "rotate_right count".to_string(), span })? as usize % arr.len()
                    } else { 1 };
                    let mut rotated = arr.clone();
                    rotated.rotate_right(n);
                    Ok(Some(DataType::Array(rotated)))
                }
                "interleave" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "interleave".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let other = self.eval_expr(&args[0])?;
                    let other_arr = match other { DataType::Array(a) => a, _ => return Err(InterpError::TypeError { expected: "Array".to_string(), actual: other.type_name().to_string(), context: "interleave argument".to_string(), span }) };
                    let max_len = arr.len().max(other_arr.len());
                    let mut result = Vec::with_capacity(arr.len() + other_arr.len());
                    for i in 0..max_len {
                        if i < arr.len() { result.push(arr[i].clone()); }
                        if i < other_arr.len() { result.push(other_arr[i].clone()); }
                    }
                    Ok(Some(DataType::Array(result)))
                }
                "dedup" => {
                    if arr.is_empty() { return Ok(Some(DataType::Array(vec![]))); }
                    let mut result = vec![arr[0].clone()];
                    for item in &arr[1..] {
                        if result.last() != Some(item) { result.push(item.clone()); }
                    }
                    Ok(Some(DataType::Array(result)))
                }
                "transpose" => {
                    if arr.is_empty() { return Ok(Some(DataType::Array(vec![]))); }
                    let cols = match &arr[0] { DataType::Array(row) => row.len(), _ => return Err(InterpError::TypeError { expected: "Array of Arrays".to_string(), actual: arr[0].type_name().to_string(), context: "transpose".to_string(), span }) };
                    let mut result: Vec<Vec<DataType>> = (0..cols).map(|_| Vec::with_capacity(arr.len())).collect();
                    for row in arr {
                        match row {
                            DataType::Array(r) => {
                                for (j, val) in r.iter().enumerate() {
                                    if j < cols { result[j].push(val.clone()); }
                                }
                            }
                            _ => return Err(InterpError::TypeError { expected: "Array".to_string(), actual: row.type_name().to_string(), context: "transpose row".to_string(), span }),
                        }
                    }
                    Ok(Some(DataType::Array(result.into_iter().map(DataType::Array).collect())))
                }
                "combinations" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "combinations".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let k = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "combinations size".to_string(), span })?.max(0) as usize;
                    if k > arr.len() { return Ok(Some(DataType::Array(vec![]))); }
                    let mut result = Vec::new();
                    let mut indices: Vec<usize> = (0..k).collect();
                    loop {
                        if result.len() >= MAX_ARRAY_ELEMENTS { break; }
                        result.push(DataType::Array(indices.iter().map(|&i| arr[i].clone()).collect()));
                        let mut i = k;
                        loop {
                            if i == 0 { return Ok(Some(DataType::Array(result))); }
                            i -= 1;
                            indices[i] += 1;
                            if indices[i] <= arr.len() - k + i { break; }
                        }
                        for j in (i + 1)..k { indices[j] = indices[j - 1] + 1; }
                    }
                    Ok(Some(DataType::Array(result)))
                }
                "window" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "window".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let n = arg.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: arg.type_name().to_string(), context: "window size".to_string(), span })?;
                    if n <= 0 {
                        return Err(InterpError::EvalError { error: crate::eval::EvalError::InvalidInput("window size must be > 0".to_string()), span });
                    }
                    let n = n as usize;
                    if n > arr.len() { return Ok(Some(DataType::Array(vec![]))); }
                    let mut result = Vec::with_capacity(arr.len() - n + 1);
                    for i in 0..=(arr.len() - n) {
                        if self.is_cancelled() { return Err(InterpError::Cancelled); }
                        result.push(DataType::Array(arr[i..i + n].to_vec()));
                    }
                    Ok(Some(DataType::Array(result)))
                }
                "unique" => {
                    let mut seen = std::collections::HashSet::new();
                    let mut result = Vec::new();
                    for item in arr {
                        if self.is_cancelled() { return Err(InterpError::Cancelled); }
                        let key = format!("{:?}", item);
                        if seen.insert(key) {
                            result.push(item.clone());
                        }
                    }
                    Ok(Some(DataType::Array(result)))
                }
                _ => Ok(None),
            },
            DataType::Set(items) => match method {
                "len" | "length" | "size" => Ok(Some(DataType::Int64(items.len() as i64))),
                "is_empty" => Ok(Some(DataType::Bool(items.is_empty()))),
                "contains" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "contains".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let val = self.eval_expr(&args[0])?;
                    Ok(Some(DataType::Bool(items.iter().any(|x| datatype_eq(x, &val)))))
                }
                "add" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "add".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let val = self.eval_expr(&args[0])?;
                    let mut new_items = items.clone();
                    if !new_items.iter().any(|x| datatype_eq(x, &val)) {
                        new_items.push(val);
                    }
                    Ok(Some(DataType::Set(new_items)))
                }
                "remove" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "remove".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let val = self.eval_expr(&args[0])?;
                    let new_items: Vec<DataType> = items.iter()
                        .filter(|x| !datatype_eq(x, &val))
                        .cloned().collect();
                    Ok(Some(DataType::Set(new_items)))
                }
                "union" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "union".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let other = self.eval_expr(&args[0])?;
                    let other_items = match &other {
                        DataType::Set(s) => s,
                        DataType::Array(a) => a,
                        _ => return Err(InterpError::TypeError { expected: "Set or Array".to_string(), actual: other.type_name().to_string(), context: "union argument".to_string(), span }),
                    };
                    let mut result = items.clone();
                    for item in other_items {
                        if !result.iter().any(|x| datatype_eq(x, item)) {
                            result.push(item.clone());
                        }
                    }
                    Ok(Some(DataType::Set(result)))
                }
                "intersection" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "intersection".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let other = self.eval_expr(&args[0])?;
                    let other_items = match &other {
                        DataType::Set(s) => s,
                        DataType::Array(a) => a,
                        _ => return Err(InterpError::TypeError { expected: "Set or Array".to_string(), actual: other.type_name().to_string(), context: "intersection argument".to_string(), span }),
                    };
                    let result: Vec<DataType> = items.iter()
                        .filter(|x| other_items.iter().any(|y| datatype_eq(x, y)))
                        .cloned().collect();
                    Ok(Some(DataType::Set(result)))
                }
                "difference" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "difference".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let other = self.eval_expr(&args[0])?;
                    let other_items = match &other {
                        DataType::Set(s) => s,
                        DataType::Array(a) => a,
                        _ => return Err(InterpError::TypeError { expected: "Set or Array".to_string(), actual: other.type_name().to_string(), context: "difference argument".to_string(), span }),
                    };
                    let result: Vec<DataType> = items.iter()
                        .filter(|x| !other_items.iter().any(|y| datatype_eq(x, y)))
                        .cloned().collect();
                    Ok(Some(DataType::Set(result)))
                }
                "is_subset" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "is_subset".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let other = self.eval_expr(&args[0])?;
                    let other_items = match &other {
                        DataType::Set(s) => s, DataType::Array(a) => a,
                        _ => return Err(InterpError::TypeError { expected: "Set or Array".to_string(), actual: other.type_name().to_string(), context: "is_subset argument".to_string(), span }),
                    };
                    Ok(Some(DataType::Bool(items.iter().all(|x| other_items.iter().any(|y| datatype_eq(x, y))))))
                }
                "is_superset" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "is_superset".to_string(), expected: "1".to_string(), actual: 0, span }); }
                    let other = self.eval_expr(&args[0])?;
                    let other_items = match &other {
                        DataType::Set(s) => s, DataType::Array(a) => a,
                        _ => return Err(InterpError::TypeError { expected: "Set or Array".to_string(), actual: other.type_name().to_string(), context: "is_superset argument".to_string(), span }),
                    };
                    Ok(Some(DataType::Bool(other_items.iter().all(|x| items.iter().any(|y| datatype_eq(x, y))))))
                }
                "to_array" => Ok(Some(DataType::Array(items.clone()))),
                "clear" => Ok(Some(DataType::Set(Vec::new()))),
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// Call a user-defined function with the given arguments.
    /// Supports closures (captured variables) and default parameter values.
    fn call_function(
        &mut self,
        name: &str,
        args: &[DataType],
        call_span: Span,
    ) -> Result<DataType, InterpError> {
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(InterpError::MaxCallDepth {
                limit: MAX_CALL_DEPTH,
                span: call_span,
            });
        }

        let func = match self.functions.get(name).cloned() {
            Some(f) => f,
            None => {
                let suggestion = self.suggest_function(name);
                return Err(InterpError::UndefinedFunction {
                    name: name.to_string(),
                    span: call_span,
                    suggestion,
                });
            }
        };

        // Check arity (accounting for default parameters, rest params, and kwargs)
        let has_rest = func.params.last().is_some_and(|p| p.rest);
        let has_kwargs = func.params.iter().any(|p| p.kwargs);
        let required = func.params.iter().filter(|p| p.default.is_none() && !p.rest && !p.kwargs).count();
        let max_positional = if has_rest || has_kwargs { usize::MAX } else { func.params.len() };
        if args.len() < required || args.len() > max_positional {
            let expected = if has_rest {
                format!("at least {}", required)
            } else if required == func.params.len() {
                format!("{}", required)
            } else {
                format!("{}-{}", required, func.params.len())
            };
            return Err(InterpError::ArityMismatch {
                name: name.to_string(),
                expected,
                actual: args.len(),
                span: call_span,
            });
        }

        // Pre-evaluate default parameter values in the CALLER scope
        // (default expressions may reference caller-scope variables)
        let mut resolved_args: Vec<(String, DataType, bool)> = Vec::new();
        for (i, param) in func.params.iter().enumerate() {
            if param.kwargs {
                // kwargs param gets the Map from the corresponding position,
                // or an empty Map if not provided
                let kwargs_map = if i < args.len() {
                    args[i].clone()
                } else {
                    DataType::Map(IndexMap::new())
                };
                resolved_args.push((param.name.clone(), kwargs_map, false));
                break;
            }
            if param.rest {
                let rest_args = args[i..].to_vec();
                resolved_args.push((param.name.clone(), DataType::Array(rest_args), true));
                break;
            }
            let arg = if i < args.len() {
                args[i].clone()
            } else if let Some(ref default_expr) = param.default {
                self.eval_expr(default_expr)?
            } else {
                DataType::Null
            };
            resolved_args.push((param.name.clone(), arg, false));
        }

        // Track call stack for error diagnostics (#133)
        self.call_stack_names.push(name.to_string());

        // Save outer symbol table, but preserve global scope (#311)
        // Functions can see top-level (global) definitions like `const PI = 3.14`
        let global_scope = self.symbols.first().cloned().unwrap_or_default();
        let saved_symbols = std::mem::replace(&mut self.symbols, vec![global_scope, HashMap::new()]);
        self.saved_symbol_stacks.push(saved_symbols);
        self.heap.push_scope();
        self.call_depth += 1;

        // Reset expression nesting depth per function call so recursive calls
        // do not accumulate expression depth across the entire call stack.
        let saved_expr_depth = self.expr_depth;
        self.expr_depth = 0;

        // Track call stack for debugger
        if let Some(ref mut debug) = self.debug {
            debug.call_stack.push(name.to_string());
        }

        // Inject closure captures first (if this is a lambda/closure)
        if let Some(captures) = self.closure_captures.get(name).cloned() {
            for (var_name, val, mutable) in captures {
                let addr = self.heap.alloc(val);
                self.define(&var_name, addr, mutable);
            }
        }

        // Bind pre-resolved params in the function scope
        for (pname, pval, is_rest) in resolved_args {
            let addr = self.heap.alloc(pval);
            self.define(&pname, addr, false);
            if is_rest { break; }
        }

        // Execute body — catch `return` signals at the function boundary
        let result = match self.exec_block(&func.body) {
            Ok(val) => Ok(val),
            Err(InterpError::ReturnSignal(val)) => Ok(val),
            Err(InterpError::BreakSignal(_)) => {
                Err(InterpError::BreakOutsideLoop { span: call_span })
            }
            Err(InterpError::ContinueSignal) => {
                Err(InterpError::ContinueOutsideLoop { span: call_span })
            }
            Err(e) => Err(e),
        };

        // Restore outer symbols and pop function's heap scope
        self.symbols = self.saved_symbol_stacks.pop()
            .unwrap_or_else(|| vec![HashMap::new()]);
        self.heap.pop_scope();
        self.call_depth -= 1;
        self.expr_depth = saved_expr_depth;

        // Pop call stack for diagnostics (#133) and debugger
        self.call_stack_names.pop();
        if let Some(ref mut debug) = self.debug {
            debug.call_stack.pop();
        }

        // Async functions return Future(Resolved(result)), avoiding double-wrapping
        let is_async = self.async_fns.contains(name);
        if is_async {
            result.map(|val| {
                if matches!(val, DataType::Future(_)) {
                    val // Already a Future, don't double-wrap
                } else {
                    DataType::Future(Box::new(FutureState::Resolved(Box::new(val))))
                }
            })
        } else {
            result
        }
    }

    fn exec_block(&mut self, block: &Block) -> Result<DataType, InterpError> {
        // Push a new deferred scope for this block
        self.deferred.push(Vec::new());

        let mut last = DataType::Null;
        let mut block_err: Option<InterpError> = None;

        for stmt in &block.statements {
            match self.exec_statement(stmt) {
                Ok(val) => last = val,
                Err(e) => {
                    block_err = Some(e);
                    break;
                }
            }
        }
        if block_err.is_none() {
            if let Some(tail) = &block.tail_expr {
                match self.eval_expr(tail) {
                    Ok(val) => last = val,
                    Err(e) => block_err = Some(e),
                }
            }
        }

        // Execute deferred expressions in reverse order (LIFO)
        let deferred = self.deferred.pop().unwrap_or_default();
        for deferred_expr in deferred.iter().rev() {
            // Deferred expressions run even on error; errors from deferred
            // expressions are logged but do not override the original error.
            let _ = self.eval_expr(deferred_expr);
        }

        match block_err {
            Some(e) => Err(e),
            None => Ok(last),
        }
    }

    // =========================================================================
    // Expression evaluation
    // =========================================================================

    /// Evaluate a literal, recursively evaluating array/map element expressions.
    fn eval_literal(&mut self, lit: &Literal) -> Result<DataType, InterpError> {
        match lit {
            Literal::Int64(n) => Ok(DataType::Int64(*n)),
            Literal::Float64(f) => Ok(DataType::Float64(*f)),
            Literal::String(s) => Ok(DataType::String(s.clone())),
            Literal::Bool(b) => Ok(DataType::Bool(*b)),
            Literal::Null => Ok(DataType::Null),
            Literal::Array(elements) => {
                let mut items = Vec::with_capacity(elements.len());
                for elem in elements {
                    if let ExpressionKind::Spread(inner) = &elem.kind {
                        let val = self.eval_expr(inner)?;
                        match val {
                            DataType::Array(arr) => {
                                items.extend(arr);
                                if items.len() > MAX_ARRAY_ELEMENTS {
                                    return Err(InterpError::ResourceLimit {
                                        limit: format!("{} elements", MAX_ARRAY_ELEMENTS),
                                        actual: format!("{} elements", items.len()),
                                        context: "array spread".to_string(),
                                        span: elem.span,
                                    });
                                }
                            }
                            other => {
                                return Err(InterpError::TypeError {
                                    expected: "Array".to_string(),
                                    actual: other.type_name().to_string(),
                                    context: "spread in array literal".to_string(),
                                    span: elem.span,
                                });
                            }
                        }
                    } else {
                        items.push(self.eval_expr(elem)?);
                    }
                }
                Ok(DataType::Array(items))
            }
            Literal::Map(entries) => {
                let mut map = indexmap::IndexMap::new();
                for (key, value_expr) in entries {
                    map.insert(key.clone(), self.eval_expr(value_expr)?);
                }
                Ok(DataType::Map(map))
            }
            Literal::Set(elements) => {
                let mut items = Vec::with_capacity(elements.len());
                for elem in elements {
                    let val = self.eval_expr(elem)?;
                    if !items.iter().any(|x| datatype_eq(x, &val)) {
                        items.push(val);
                    }
                }
                Ok(DataType::Set(items))
            }
        }
    }

    fn eval_expr(&mut self, expr: &Expression) -> Result<DataType, InterpError> {
        // AST depth limit (#261)
        self.expr_depth += 1;
        if self.expr_depth > MAX_EXPR_DEPTH {
            self.expr_depth -= 1;
            return Err(InterpError::ResourceLimit {
                limit: format!("{} levels", MAX_EXPR_DEPTH),
                actual: format!("{} levels", self.expr_depth + 1),
                context: "expression nesting depth".to_string(),
                span: expr.span,
            });
        }
        let result = self.eval_expr_inner(expr);
        self.expr_depth -= 1;
        result
    }

    fn eval_expr_inner(&mut self, expr: &Expression) -> Result<DataType, InterpError> {
        match &expr.kind {
            ExpressionKind::Literal(lit) => self.eval_literal(lit),

            ExpressionKind::Variable(name) => {
                // Built-in constant: None
                if name == "None" {
                    return Ok(DataType::Null);
                }
                let addr = match self.lookup(name) {
                    Some(entry) => entry.addr,
                    None => {
                        let suggestion = self.suggest_variable(name);
                        return Err(InterpError::UndefinedVariable {
                            name: name.clone(),
                            span: expr.span,
                            suggestion,
                        });
                    }
                };
                self.heap
                    .read(addr)
                    .cloned()
                    .ok_or_else(|| InterpError::UndefinedVariable {
                        name: name.clone(),
                        span: expr.span,
                        suggestion: None,
                    })
            }

            ExpressionKind::BinaryOp { op, left, right } => {
                // Equality (==, !=) is delegated to the OperationEvaluator which uses
                // Rust's PartialEq on DataType (derived). For Float64, this gives IEEE 754
                // semantics: NaN == NaN is false, NaN != NaN is true. This is intentional.
                // Note: pattern matching in `match` expressions uses structural equality
                // (NaN matches NaN) — that is also intentional, as pattern matching is
                // structural/value-based, not arithmetic.

                // Short-circuit evaluation for logical operators using truthiness
                // Any value is accepted: falsy = false/null/0/0.0/"", truthy = everything else
                if *op == BinOp::And {
                    let lhs = self.eval_expr(left)?;
                    return if !lhs.to_bool() {
                        Ok(lhs)
                    } else {
                        self.eval_expr(right)
                    };
                }
                if *op == BinOp::Or {
                    let lhs = self.eval_expr(left)?;
                    return if lhs.to_bool() {
                        Ok(lhs)
                    } else {
                        self.eval_expr(right)
                    };
                }

                let lhs = self.eval_expr(left)?;
                let rhs = self.eval_expr(right)?;

                // `in` operator: containment check (#290)
                if *op == BinOp::In {
                    let result = match &rhs {
                        DataType::Array(arr) => arr.iter().any(|item| *item == lhs),
                        DataType::Set(set) => set.iter().any(|item| *item == lhs),
                        DataType::Map(map) => {
                            if let DataType::String(key) = &lhs {
                                map.contains_key(key.as_str())
                            } else {
                                false
                            }
                        }
                        DataType::String(s) => {
                            if let DataType::String(needle) = &lhs {
                                s.contains(needle.as_str())
                            } else {
                                false
                            }
                        }
                        _ => {
                            return Err(InterpError::TypeError {
                                expected: "Array, Map, Set, or String".to_string(),
                                actual: rhs.type_name().to_string(),
                                context: "'in' operator".to_string(),
                                span: expr.span,
                            });
                        }
                    };
                    return Ok(DataType::Bool(result));
                }

                // String multiplication: "ha" * 3 => "hahaha" (#289)
                if *op == BinOp::Mul {
                    if let DataType::String(ref s) = lhs {
                        if let Some(n) = rhs.to_i64() {
                            if n < 0 {
                                return Ok(DataType::String(String::new()));
                            }
                            let n = n as usize;
                            let result_len = s.len().saturating_mul(n);
                            if result_len > MAX_STRING_OUTPUT {
                                return Err(InterpError::ResourceLimit {
                                    limit: format!("{} bytes", MAX_STRING_OUTPUT),
                                    actual: format!("{} bytes", result_len),
                                    context: "string multiplication".to_string(),
                                    span: expr.span,
                                });
                            }
                            return Ok(DataType::String(s.repeat(n)));
                        }
                    }
                }

                // Operator overloading: check if lhs is a struct with a magic method (#86)
                if let DataType::Map(ref map) = lhs {
                    if let Some(DataType::String(struct_name)) = map.get("__struct") {
                        let magic = op.magic_method_name();
                        if let Some(im) = self.impl_methods.get(struct_name).and_then(|m| m.get(magic)).cloned() {
                            self.call_depth += 1;
                            if self.call_depth > MAX_CALL_DEPTH {
                                self.call_depth -= 1;
                                return Err(InterpError::ResourceLimit { limit: format!("{} calls", MAX_CALL_DEPTH), actual: format!("{} calls", self.call_depth + 1), context: format!("{}.{}", struct_name, magic), span: expr.span });
                            }
                            self.symbols.push(HashMap::new());
                            self.heap.push_scope();
                            if let Some(p) = im.params.first() { let addr = self.heap.alloc(lhs.clone()); self.define(&p.name, addr, false); }
                            if let Some(p) = im.params.get(1) { let addr = self.heap.alloc(rhs.clone()); self.define(&p.name, addr, false); }
                            let result = self.exec_block(&im.body);
                            self.heap.pop_scope();
                            self.symbols.pop();
                            self.call_depth -= 1;
                            return match result {
                                Ok(v) => Ok(v),
                                Err(InterpError::ReturnSignal(value)) => Ok(value),
                                Err(e) => Err(e),
                            };
                        }
                    }
                }

                let op_type = OperationType::parse(op.operation_name()).ok_or_else(|| {
                    InterpError::UnknownOperation {
                        name: op.operation_name().to_string(),
                        span: expr.span,
                        suggestion: None,
                    }
                })?;

                let input_ports = op_input_ports(op_type);
                let mut inputs = HashMap::with_capacity(2);
                if let Some(p) = input_ports.first() {
                    inputs.insert(p.to_string(), lhs);
                }
                if let Some(p) = input_ports.get(1) {
                    inputs.insert(p.to_string(), rhs);
                }

                self.evaluator.eval_operation(op_type, &inputs, &EMPTY_CONFIG).map_err(|e| {
                    InterpError::EvalError {
                        error: e,
                        span: expr.span,
                    }
                })
            }

            ExpressionKind::UnaryOp { op, operand } => {
                let val = self.eval_expr(operand)?;

                // Handle `!` (Not) directly in the interpreter with truthiness semantics,
                // so it works regardless of evaluator implementation.
                if *op == UnOp::Not {
                    let truthy = match &val {
                        DataType::Bool(b) => *b,
                        DataType::Int32(n) => *n != 0,
                        DataType::Int64(n) => *n != 0,
                        DataType::Uint32(n) => *n != 0,
                        DataType::Uint64(n) => *n != 0,
                        DataType::Float32(f) => *f != 0.0 && !f.is_nan(),
                        DataType::Float64(f) => *f != 0.0 && !f.is_nan(),
                        DataType::String(s) => !s.is_empty(),
                        DataType::Null => false,
                        DataType::Array(a) => !a.is_empty(),
                        DataType::Map(m) => !m.is_empty(),
                        DataType::Bytes(b) => !b.is_empty(),
                        _ => true,
                    };
                    return Ok(DataType::Bool(!truthy));
                }

                let op_type = OperationType::parse(op.operation_name()).ok_or_else(|| {
                    InterpError::UnknownOperation {
                        name: op.operation_name().to_string(),
                        span: expr.span,
                        suggestion: None,
                    }
                })?;

                let inputs = HashMap::from([("value".to_string(), val)]);

                self.evaluator.eval_operation(op_type, &inputs, &EMPTY_CONFIG).map_err(|e| {
                    InterpError::EvalError {
                        error: e,
                        span: expr.span,
                    }
                })
            }

            ExpressionKind::Call {
                name: fn_name,
                args,
                kwargs,
            } => {
                // Check if it's a user-defined function
                if self.functions.contains_key(fn_name.as_str()) {
                    let evaluated_args = self.merge_kwargs_into_args(fn_name, args, kwargs, expr.span)?;
                    return self.call_function(fn_name, &evaluated_args, expr.span);
                }

                // Check if it's a variable holding a function reference (lambda)
                if let Some(entry) = self.lookup(fn_name) {
                    let addr = entry.addr;
                    if let Some(DataType::String(ref_name)) = self.heap.read(addr).cloned() {
                        if self.functions.contains_key(ref_name.as_str()) {
                            let evaluated_args = self.merge_kwargs_into_args(&ref_name, args, kwargs, expr.span)?;
                            return self.call_function(&ref_name, &evaluated_args, expr.span);
                        }
                    }
                }

                // Check if it's an plugin call
                if self.imports.contains(fn_name.as_str()) {
                    self.logs.push(LogEntry {
                        level: LogLevel::Warn,
                        message: format!("Plugin '{}' call skipped in interpreter mode", fn_name),
                        line: Some(expr.span.start_line),
                        node_id: None,
                    });
                    return Ok(DataType::Null);
                }

                // Special built-in functions
                match fn_name.as_str() {
                    "debug_log" | "println" | "print" => {
                        if let Some(arg) = args.first() {
                            let val = self.eval_expr(arg)?;
                            self.logs.push(LogEntry {
                                level: if fn_name == "debug_log" {
                                    LogLevel::Debug
                                } else {
                                    LogLevel::Info
                                },
                                message: datatype_to_display(&val),
                                line: Some(expr.span.start_line),
                                node_id: None,
                            });
                            return Ok(val);
                        }
                        return Ok(DataType::Null);
                    }
                    "typeof" => {
                        if let Some(arg) = args.first() {
                            let val = self.eval_expr(arg)?;
                            return Ok(DataType::String(val.type_name().to_string()));
                        }
                        return Ok(DataType::String("null".to_string()));
                    }
                    "Set" => {
                        let mut items = Vec::new();
                        for arg in args {
                            let val = self.eval_expr(arg)?;
                            if args.len() == 1 {
                                if let DataType::Array(arr) = val {
                                    for item in arr {
                                        if !items.iter().any(|x: &DataType| datatype_eq(x, &item)) {
                                            items.push(item);
                                        }
                                    }
                                    return Ok(DataType::Set(items));
                                } else {
                                    if !items.iter().any(|x: &DataType| datatype_eq(x, &val)) {
                                        items.push(val);
                                    }
                                    return Ok(DataType::Set(items));
                                }
                            }
                            if !items.iter().any(|x: &DataType| datatype_eq(x, &val)) {
                                items.push(val);
                            }
                        }
                        return Ok(DataType::Set(items));
                    }
                    "Tuple" | "tuple" => {
                        let mut items = Vec::new();
                        for arg in args {
                            let val = self.eval_expr(arg)?;
                            if args.len() == 1 {
                                // Single array arg: convert to tuple
                                if let DataType::Array(arr) = val {
                                    return Ok(DataType::Tuple(arr));
                                }
                                items.push(val);
                                return Ok(DataType::Tuple(items));
                            }
                            items.push(val);
                        }
                        return Ok(DataType::Tuple(items));
                    }
                    // Option/Result constructors and helpers (#78)
                    "Some" => {
                        if let Some(arg) = args.first() {
                            return self.eval_expr(arg);
                        }
                        return Ok(DataType::Null);
                    }
                    "None" => {
                        return Ok(DataType::Null);
                    }
                    "Ok" => {
                        let val = if let Some(arg) = args.first() {
                            self.eval_expr(arg)?
                        } else {
                            DataType::Null
                        };
                        let mut map = IndexMap::new();
                        map.insert("ok".to_string(), val);
                        return Ok(DataType::Map(map));
                    }
                    "Err" => {
                        let val = if let Some(arg) = args.first() {
                            self.eval_expr(arg)?
                        } else {
                            DataType::String("error".to_string())
                        };
                        let mut map = IndexMap::new();
                        map.insert("err".to_string(), val);
                        return Ok(DataType::Map(map));
                    }
                    "is_some" => {
                        if let Some(arg) = args.first() {
                            let val = self.eval_expr(arg)?;
                            return Ok(DataType::Bool(!matches!(val, DataType::Null)));
                        }
                        return Ok(DataType::Bool(false));
                    }
                    "is_none" => {
                        if let Some(arg) = args.first() {
                            let val = self.eval_expr(arg)?;
                            return Ok(DataType::Bool(matches!(val, DataType::Null)));
                        }
                        return Ok(DataType::Bool(true));
                    }
                    "is_ok" => {
                        if let Some(arg) = args.first() {
                            let val = self.eval_expr(arg)?;
                            if let DataType::Map(ref m) = val {
                                return Ok(DataType::Bool(m.contains_key("ok")));
                            }
                        }
                        return Ok(DataType::Bool(false));
                    }
                    "is_err" => {
                        if let Some(arg) = args.first() {
                            let val = self.eval_expr(arg)?;
                            if let DataType::Map(ref m) = val {
                                return Ok(DataType::Bool(m.contains_key("err")));
                            }
                        }
                        return Ok(DataType::Bool(false));
                    }
                    "unwrap" => {
                        if let Some(arg) = args.first() {
                            let val = self.eval_expr(arg)?;
                            return match val {
                                DataType::Null => Err(InterpError::ThrownError {
                                    value: DataType::String("called unwrap on None value".to_string()),
                                    span: expr.span,
                                }),
                                DataType::Map(ref m) if m.contains_key("ok") => {
                                    Ok(m.get("ok").cloned().unwrap_or(DataType::Null))
                                }
                                DataType::Map(ref m) if m.contains_key("err") => {
                                    Err(InterpError::ThrownError {
                                        value: m.get("err").cloned().unwrap_or(DataType::String("error".to_string())),
                                        span: expr.span,
                                    })
                                }
                                other => Ok(other),
                            };
                        }
                        return Err(InterpError::ArityMismatch { name: "unwrap".to_string(), expected: "1".to_string(), actual: 0, span: expr.span });
                    }
                    "unwrap_or" => {
                        if args.len() < 2 {
                            return Err(InterpError::ArityMismatch { name: "unwrap_or".to_string(), expected: "2".to_string(), actual: args.len(), span: expr.span });
                        }
                        let val = self.eval_expr(&args[0])?;
                        let default = self.eval_expr(&args[1])?;
                        return match val {
                            DataType::Null => Ok(default),
                            DataType::Map(ref m) if m.contains_key("ok") => {
                                Ok(m.get("ok").cloned().unwrap_or(DataType::Null))
                            }
                            DataType::Map(ref m) if m.contains_key("err") => {
                                Ok(default)
                            }
                            other => Ok(other),
                        };
                    }
                    "stdin_read" | "input" => {
                        use std::io::BufRead;
                        let mut line = String::new();
                        std::io::stdin().lock().read_line(&mut line).ok();
                        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
                        return Ok(DataType::String(trimmed.to_string()));
                    }
                    "len" => {
                        if let Some(arg) = args.first() {
                            let val = self.eval_expr(arg)?;
                            let length = match &val {
                                DataType::Array(a) => a.len() as i64,
                                DataType::Set(s) => s.len() as i64,
                                DataType::String(s) => s.chars().count() as i64,
                                DataType::Map(m) => m.len() as i64,
                                DataType::Bytes(b) => b.len() as i64,
                                _ => {
                                    return Err(InterpError::TypeError {
                                        expected: "Array, String, Map, or Bytes".to_string(),
                                        actual: val.type_name().to_string(),
                                        context: "len()".to_string(),
                                        span: expr.span,
                                    })
                                }
                            };
                            return Ok(DataType::Int64(length));
                        }
                        return Err(InterpError::ArityMismatch { name: "len".to_string(), expected: "1".to_string(), actual: 0, span: expr.span });
                    }
                    "assert" => {
                        if args.is_empty() {
                            return Err(InterpError::ArityMismatch { name: "assert".to_string(), expected: "1-2".to_string(), actual: 0, span: expr.span });
                        }
                        let val = self.eval_expr(&args[0])?;
                        if val.to_bool() {
                            return Ok(DataType::Null);
                        } else {
                            let msg = if args.len() > 1 {
                                let msg_val = self.eval_expr(&args[1])?;
                                datatype_to_display(&msg_val)
                            } else {
                                "Assertion failed".to_string()
                            };
                            return Err(InterpError::AssertionFailed {
                                message: msg,
                                span: expr.span,
                            });
                        }
                    }
                    "assert_eq" => {
                        if args.len() < 2 {
                            return Err(InterpError::ArityMismatch {
                                name: "assert_eq".to_string(),
                                expected: "2".to_string(),
                                actual: args.len(),
                                span: expr.span,
                            });
                        }
                        let left = self.eval_expr(&args[0])?;
                        let right = self.eval_expr(&args[1])?;
                        if left == right {
                            return Ok(DataType::Null);
                        }
                        let msg = if args.len() > 2 {
                            let msg_val = self.eval_expr(&args[2])?;
                            datatype_to_display(&msg_val)
                        } else {
                            format!(
                                "Assertion failed: expected values to be equal\n  left:  {}\n  right: {}",
                                datatype_to_display(&left),
                                datatype_to_display(&right)
                            )
                        };
                        return Err(InterpError::AssertionFailed {
                            message: msg,
                            span: expr.span,
                        });
                    }
                    "assert_ne" => {
                        if args.len() < 2 {
                            return Err(InterpError::ArityMismatch {
                                name: "assert_ne".to_string(),
                                expected: "2".to_string(),
                                actual: args.len(),
                                span: expr.span,
                            });
                        }
                        let left = self.eval_expr(&args[0])?;
                        let right = self.eval_expr(&args[1])?;
                        if left != right {
                            return Ok(DataType::Null);
                        }
                        let msg = if args.len() > 2 {
                            let msg_val = self.eval_expr(&args[2])?;
                            datatype_to_display(&msg_val)
                        } else {
                            format!(
                                "Assertion failed: expected values to differ, both are: {}",
                                datatype_to_display(&left)
                            )
                        };
                        return Err(InterpError::AssertionFailed {
                            message: msg,
                            span: expr.span,
                        });
                    }
                    "assert_throws" => {
                        if args.is_empty() {
                            return Err(InterpError::ArityMismatch {
                                name: "assert_throws".to_string(),
                                expected: "1+".to_string(),
                                actual: 0,
                                span: expr.span,
                            });
                        }
                        // Evaluate the first arg as a function name
                        let fn_name_val = self.eval_expr(&args[0])?;
                        let target_fn = match &fn_name_val {
                            DataType::String(s) => s.clone(),
                            _ => datatype_to_display(&fn_name_val),
                        };
                        // Evaluate extra args to pass to the function (#313)
                        let call_args: Vec<DataType> = args[1..].iter()
                            .map(|a| self.eval_expr(a))
                            .collect::<Result<_, _>>()?;
                        match self.call_function(&target_fn, &call_args, expr.span) {
                            Err(e) if !is_control_flow(&e) => {
                                // Good — it threw an error as expected
                                return Ok(DataType::Null);
                            }
                            Ok(_) => {
                                let msg = format!(
                                    "Assertion failed: expected '{}' to throw, but it returned successfully",
                                    target_fn
                                );
                                return Err(InterpError::AssertionFailed {
                                    message: msg,
                                    span: expr.span,
                                });
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    // Math builtins (#121-125)
                    "factorial" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "factorial".to_string(), expected: "1".to_string(), actual: 0, span: expr.span }); }
                        let n = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "factorial".to_string(), span: expr.span })?;
                        if n < 0 { return Err(InterpError::EvalError { error: crate::eval::EvalError::InvalidInput("factorial of negative number".to_string()), span: expr.span }); }
                        if n > 20 { return Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("factorial overflow (max 20!)".to_string()), span: expr.span }); }
                        let mut result: i64 = 1;
                        for i in 2..=(n as u64) { result = result.saturating_mul(i as i64); }
                        return Ok(DataType::Int64(result));
                    }
                    "fibonacci" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "fibonacci".to_string(), expected: "1".to_string(), actual: 0, span: expr.span }); }
                        let n = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "fibonacci".to_string(), span: expr.span })?;
                        if n < 0 { return Err(InterpError::EvalError { error: crate::eval::EvalError::InvalidInput("fibonacci of negative number".to_string()), span: expr.span }); }
                        if n > 92 { return Err(InterpError::EvalError { error: crate::eval::EvalError::Overflow("fibonacci overflow (max fib(92))".to_string()), span: expr.span }); }
                        let (mut a, mut b): (i64, i64) = (0, 1);
                        for _ in 0..n { let t = b; b = a.saturating_add(b); a = t; }
                        return Ok(DataType::Int64(a));
                    }
                    "is_prime" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "is_prime".to_string(), expected: "1".to_string(), actual: 0, span: expr.span }); }
                        let n = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "is_prime".to_string(), span: expr.span })?;
                        if n < 2 { return Ok(DataType::Bool(false)); }
                        if n < 4 { return Ok(DataType::Bool(true)); }
                        if n % 2 == 0 || n % 3 == 0 { return Ok(DataType::Bool(false)); }
                        let mut i = 5i64;
                        while i * i <= n { if n % i == 0 || n % (i + 2) == 0 { return Ok(DataType::Bool(false)); } i += 6; }
                        return Ok(DataType::Bool(true));
                    }
                    "ncr" | "combinations" => {
                        if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "ncr".to_string(), expected: "2".to_string(), actual: args.len(), span: expr.span }); }
                        let n = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "ncr(n)".to_string(), span: expr.span })?;
                        let r = self.eval_expr(&args[1])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "ncr(r)".to_string(), span: expr.span })?;
                        if n < 0 || r < 0 || r > n { return Ok(DataType::Int64(0)); }
                        let r = r.min(n - r) as u64; // Optimize: C(n,r) = C(n,n-r)
                        let mut result: i64 = 1;
                        for i in 0..r { result = result.saturating_mul((n as u64 - i) as i64) / (i as i64 + 1); }
                        return Ok(DataType::Int64(result));
                    }
                    "npr" | "permutations" => {
                        if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "npr".to_string(), expected: "2".to_string(), actual: args.len(), span: expr.span }); }
                        let n = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "npr(n)".to_string(), span: expr.span })?;
                        let r = self.eval_expr(&args[1])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "npr(r)".to_string(), span: expr.span })?;
                        if n < 0 || r < 0 || r > n { return Ok(DataType::Int64(0)); }
                        let mut result: i64 = 1;
                        for i in 0..(r as u64) { result = result.saturating_mul((n as u64 - i) as i64); }
                        return Ok(DataType::Int64(result));
                    }
                    // String encode builtin (#103)
                    "encode" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "encode".to_string(), expected: "1-2".to_string(), actual: 0, span: expr.span }); }
                        let val = self.eval_expr(&args[0])?;
                        let s = match &val { DataType::String(s) => s.clone(), _ => return Err(InterpError::TypeError { expected: "string".to_string(), actual: val.type_name().to_string(), context: "encode".to_string(), span: expr.span }) };
                        let encoding = if args.len() > 1 {
                            let enc_val = self.eval_expr(&args[1])?;
                            match enc_val { DataType::String(e) => e, _ => return Err(InterpError::TypeError { expected: "string".to_string(), actual: enc_val.type_name().to_string(), context: "encode(encoding)".to_string(), span: expr.span }) }
                        } else {
                            "utf8".to_string()
                        };
                        let bytes = match encoding.to_lowercase().as_str() {
                            "utf8" | "utf-8" => s.as_bytes().to_vec(),
                            "utf16-le" | "utf16le" => {
                                let mut buf = Vec::with_capacity(s.len() * 2);
                                for code_unit in s.encode_utf16() { buf.extend_from_slice(&code_unit.to_le_bytes()); }
                                buf
                            }
                            "utf16-be" | "utf16be" => {
                                let mut buf = Vec::with_capacity(s.len() * 2);
                                for code_unit in s.encode_utf16() { buf.extend_from_slice(&code_unit.to_be_bytes()); }
                                buf
                            }
                            "ascii" => {
                                let mut buf = Vec::with_capacity(s.len());
                                for ch in s.chars() {
                                    if ch.is_ascii() { buf.push(ch as u8); }
                                    else { buf.push(b'?'); }
                                }
                                buf
                            }
                            other => return Err(InterpError::EvalError { error: crate::eval::EvalError::InvalidInput(format!("encode: unsupported encoding '{}' (supported: utf8, utf16-le, utf16-be, ascii)", other)), span: expr.span }),
                        };
                        return Ok(DataType::Bytes(bytes));
                    }
                    // Array binary_search_by builtin (#112)
                    "binary_search_by" => {
                        if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "binary_search_by".to_string(), expected: "2".to_string(), actual: args.len(), span: expr.span }); }
                        let arr_val = self.eval_expr(&args[0])?;
                        let arr = match arr_val { DataType::Array(a) => a, _ => return Err(InterpError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "binary_search_by".to_string(), span: expr.span }) };
                        // Binary search using comparator function
                        let mut lo: usize = 0;
                        let mut hi: usize = arr.len();
                        let mut found: Option<usize> = None;
                        while lo < hi {
                            let mid = lo + (hi - lo) / 2;
                            let cmp_result = self.call_lambda_with_args(&args[1], &[arr[mid].clone()], expr.span)?;
                            let ord = cmp_result.to_i64().unwrap_or(0);
                            if ord == 0 {
                                found = Some(mid);
                                break;
                            } else if ord < 0 {
                                lo = mid + 1;
                            } else {
                                hi = mid;
                            }
                        }
                        return match found {
                            Some(idx) => Ok(DataType::Int64(idx as i64)),
                            None => Ok(DataType::Null),
                        };
                    }
                    // File watch builtin (#130)
                    "fs_watch" => {
                        if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "fs_watch".to_string(), expected: "2-3".to_string(), actual: args.len(), span: expr.span }); }
                        let path_val = self.eval_expr(&args[0])?;
                        let path_str = match &path_val { DataType::String(s) => s.clone(), _ => return Err(InterpError::TypeError { expected: "string".to_string(), actual: path_val.type_name().to_string(), context: "fs_watch(path)".to_string(), span: expr.span }) };
                        let timeout_ms: u64 = if args.len() > 2 {
                            let t = self.eval_expr(&args[2])?;
                            t.to_i64().unwrap_or(5000).max(0) as u64
                        } else {
                            5000
                        };
                        let path = std::path::Path::new(&path_str);
                        let initial_mtime = std::fs::metadata(path)
                            .and_then(|m| m.modified())
                            .map_err(|e| InterpError::EvalError { error: crate::eval::EvalError::InvalidInput(format!("fs_watch: {}", e)), span: expr.span })?;
                        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
                        let mut changes_detected: i64 = 0;
                        loop {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            if std::time::Instant::now() >= deadline { break; }
                            std::thread::sleep(std::time::Duration::from_millis(500));
                            let current_mtime = std::fs::metadata(path)
                                .and_then(|m| m.modified())
                                .unwrap_or(initial_mtime);
                            if current_mtime != initial_mtime {
                                self.call_lambda_with_args(&args[1], &[DataType::String(path_str.clone())], expr.span)?;
                                changes_detected += 1;
                                break;
                            }
                        }
                        return Ok(DataType::Int64(changes_detected));
                    }
                    // Date/DateTime builtins (#81)
                    "date_now" => {
                        use std::time::SystemTime;
                        let now = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default();
                        let secs = now.as_secs() as i64;
                        // Format as ISO 8601 date string (UTC)
                        let days = secs / 86400;
                        let rem = secs % 86400;
                        // Civil date from days since epoch (algorithm from Howard Hinnant)
                        let z = days + 719468;
                        let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
                        let doe = (z - era * 146097) as u64;
                        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
                        let y = (yoe as i64) + era * 400;
                        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
                        let mp = (5 * doy + 2) / 153;
                        let d = doy - (153 * mp + 2) / 5 + 1;
                        let m = if mp < 10 { mp + 3 } else { mp - 9 };
                        let y = if m <= 2 { y + 1 } else { y };
                        let h = rem / 3600;
                        let min = (rem % 3600) / 60;
                        let s = rem % 60;
                        let iso = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, h, min, s);
                        return Ok(DataType::String(iso));
                    }
                    "date_parse" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "date_parse".to_string(), expected: "1".to_string(), actual: 0, span: expr.span }); }
                        let val = self.eval_expr(&args[0])?;
                        let s = match &val { DataType::String(s) => s.as_str(), _ => return Err(InterpError::TypeError { expected: "string".to_string(), actual: val.type_name().to_string(), context: "date_parse".to_string(), span: expr.span }) };
                        // Parse ISO 8601: YYYY-MM-DDThh:mm:ssZ or YYYY-MM-DD
                        let parts: Vec<&str> = s.split('T').collect();
                        let date_part = parts[0];
                        let dp: Vec<&str> = date_part.split('-').collect();
                        if dp.len() != 3 { return Err(InterpError::EvalError { error: crate::eval::EvalError::InvalidInput(format!("invalid date format: {}", s)), span: expr.span }); }
                        let y: i64 = dp[0].parse().map_err(|_| InterpError::EvalError { error: crate::eval::EvalError::InvalidInput(format!("invalid year: {}", dp[0])), span: expr.span })?;
                        let m: u32 = dp[1].parse().map_err(|_| InterpError::EvalError { error: crate::eval::EvalError::InvalidInput(format!("invalid month: {}", dp[1])), span: expr.span })?;
                        let d: u32 = dp[2].parse().map_err(|_| InterpError::EvalError { error: crate::eval::EvalError::InvalidInput(format!("invalid day: {}", dp[2])), span: expr.span })?;
                        if m < 1 || m > 12 || d < 1 || d > 31 { return Err(InterpError::EvalError { error: crate::eval::EvalError::InvalidInput(format!("invalid date: {}", s)), span: expr.span }); }
                        let (mut h, mut min, mut sec) = (0i64, 0i64, 0i64);
                        if parts.len() > 1 {
                            let time_part = parts[1].trim_end_matches('Z');
                            let tp: Vec<&str> = time_part.split(':').collect();
                            if !tp.is_empty() { h = tp[0].parse().unwrap_or(0); }
                            if tp.len() > 1 { min = tp[1].parse().unwrap_or(0); }
                            if tp.len() > 2 { sec = tp[2].parse().unwrap_or(0); }
                        }
                        // Convert to Unix timestamp (days since epoch algorithm)
                        let m_adj = if m <= 2 { m + 9 } else { m - 3 };
                        let y_adj = if m <= 2 { y - 1 } else { y };
                        let era = (if y_adj >= 0 { y_adj } else { y_adj - 399 }) / 400;
                        let yoe = (y_adj - era * 400) as u64;
                        let doy = (153 * (m_adj as u64) + 2) / 5 + (d as u64) - 1;
                        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
                        let days = (era as i64) * 146097 + (doe as i64) - 719468;
                        let timestamp = days * 86400 + h * 3600 + min * 60 + sec;
                        return Ok(DataType::Int64(timestamp));
                    }
                    "date_format" => {
                        if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "date_format".to_string(), expected: "2".to_string(), actual: args.len(), span: expr.span }); }
                        let ts = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "date_format(timestamp)".to_string(), span: expr.span })?;
                        let fmt_val = self.eval_expr(&args[1])?;
                        let fmt_str = match &fmt_val { DataType::String(s) => s.as_str(), _ => return Err(InterpError::TypeError { expected: "string".to_string(), actual: fmt_val.type_name().to_string(), context: "date_format(format)".to_string(), span: expr.span }) };
                        // Convert timestamp to date parts
                        let days = ts.div_euclid(86400);
                        let rem = ts.rem_euclid(86400);
                        let z = days + 719468;
                        let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
                        let doe = (z - era * 146097) as u64;
                        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
                        let y = (yoe as i64) + era * 400;
                        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
                        let mp = (5 * doy + 2) / 153;
                        let d = doy - (153 * mp + 2) / 5 + 1;
                        let m = if mp < 10 { mp + 3 } else { mp - 9 };
                        let y = if m <= 2 { y + 1 } else { y };
                        let h = rem / 3600;
                        let min = (rem % 3600) / 60;
                        let s = rem % 60;
                        let result = fmt_str
                            .replace("%Y", &format!("{:04}", y))
                            .replace("%m", &format!("{:02}", m))
                            .replace("%d", &format!("{:02}", d))
                            .replace("%H", &format!("{:02}", h))
                            .replace("%M", &format!("{:02}", min))
                            .replace("%S", &format!("{:02}", s));
                        return Ok(DataType::String(result));
                    }
                    "date_add" => {
                        if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "date_add".to_string(), expected: "2".to_string(), actual: args.len(), span: expr.span }); }
                        let ts = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "date_add(timestamp)".to_string(), span: expr.span })?;
                        let days = self.eval_expr(&args[1])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "date_add(days)".to_string(), span: expr.span })?;
                        return Ok(DataType::Int64(ts + days * 86400));
                    }
                    "date_diff" => {
                        if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "date_diff".to_string(), expected: "2".to_string(), actual: args.len(), span: expr.span }); }
                        let ts1 = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "date_diff(ts1)".to_string(), span: expr.span })?;
                        let ts2 = self.eval_expr(&args[1])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "integer".to_string(), actual: "non-integer".to_string(), context: "date_diff(ts2)".to_string(), span: expr.span })?;
                        return Ok(DataType::Int64((ts1 - ts2) / 86400));
                    }
                    // Duration helpers (#82)
                    "duration_ms" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "duration_ms".to_string(), expected: "1".to_string(), actual: 0, span: expr.span }); }
                        let n = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: "non-number".to_string(), context: "duration_ms".to_string(), span: expr.span })?;
                        return Ok(DataType::Int64(n));
                    }
                    "duration_secs" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "duration_secs".to_string(), expected: "1".to_string(), actual: 0, span: expr.span }); }
                        let n = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: "non-number".to_string(), context: "duration_secs".to_string(), span: expr.span })?;
                        return Ok(DataType::Int64(n * 1000));
                    }
                    "duration_mins" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "duration_mins".to_string(), expected: "1".to_string(), actual: 0, span: expr.span }); }
                        let n = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: "non-number".to_string(), context: "duration_mins".to_string(), span: expr.span })?;
                        return Ok(DataType::Int64(n * 60 * 1000));
                    }
                    "duration_hours" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "duration_hours".to_string(), expected: "1".to_string(), actual: 0, span: expr.span }); }
                        let n = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| InterpError::TypeError { expected: "number".to_string(), actual: "non-number".to_string(), context: "duration_hours".to_string(), span: expr.span })?;
                        return Ok(DataType::Int64(n * 60 * 60 * 1000));
                    }
                    // ========================================================
                    // Channel-based concurrency primitives
                    // ========================================================
                    "channel" => {
                        // channel() -> [sender_id, receiver_id]
                        // Optional capacity arg: channel(10) for bounded
                        let (tx_id, rx_id) = channel_ids();
                        let (tx, rx) = if !args.is_empty() {
                            let cap = self.eval_expr(&args[0])?.to_i64().ok_or_else(|| {
                                InterpError::TypeError {
                                    expected: "integer".to_string(),
                                    actual: "non-integer".to_string(),
                                    context: "channel(capacity)".to_string(),
                                    span: expr.span,
                                }
                            })?;
                            if cap <= 0 {
                                return Err(InterpError::EvalError {
                                    error: EvalError::InvalidInput(
                                        "channel capacity must be positive".to_string(),
                                    ),
                                    span: expr.span,
                                });
                            }
                            let (tx, rx) = std::sync::mpsc::sync_channel(cap as usize);
                            // Wrap SyncSender as a Sender-like interface
                            // We store them separately since SyncSender != Sender
                            channel_store(&tx_id, ChannelSyncSender { tx }).map_err(|e| {
                                InterpError::EvalError {
                                    error: EvalError::InvalidInput(e),
                                    span: expr.span,
                                }
                            })?;
                            channel_store(
                                &rx_id,
                                ChannelReceiver {
                                    rx: Arc::new(Mutex::new(rx)),
                                },
                            )
                            .map_err(|e| InterpError::EvalError {
                                error: EvalError::InvalidInput(e),
                                span: expr.span,
                            })?;
                            return Ok(DataType::Array(vec![
                                DataType::String(tx_id),
                                DataType::String(rx_id),
                            ]));
                        } else {
                            std::sync::mpsc::channel()
                        };
                        channel_store(&tx_id, ChannelSender { tx }).map_err(|e| {
                            InterpError::EvalError {
                                error: EvalError::InvalidInput(e),
                                span: expr.span,
                            }
                        })?;
                        channel_store(
                            &rx_id,
                            ChannelReceiver {
                                rx: Arc::new(Mutex::new(rx)),
                            },
                        )
                        .map_err(|e| InterpError::EvalError {
                            error: EvalError::InvalidInput(e),
                            span: expr.span,
                        })?;
                        return Ok(DataType::Array(vec![
                            DataType::String(tx_id),
                            DataType::String(rx_id),
                        ]));
                    }
                    "chan_send" => {
                        // chan_send(sender_id, value) -> null
                        if args.len() < 2 {
                            return Err(InterpError::ArityMismatch {
                                name: "chan_send".to_string(),
                                expected: "2".to_string(),
                                actual: args.len(),
                                span: expr.span,
                            });
                        }
                        let tx_id_val = self.eval_expr(&args[0])?;
                        let tx_id_str = match &tx_id_val {
                            DataType::String(s) => s.clone(),
                            _ => {
                                return Err(InterpError::TypeError {
                                    expected: "string (sender ID)".to_string(),
                                    actual: tx_id_val.type_name().to_string(),
                                    context: "chan_send".to_string(),
                                    span: expr.span,
                                })
                            }
                        };
                        let value = self.eval_expr(&args[1])?;

                        // Try as unbounded sender first, then bounded
                        let mut map = CHANNEL_REGISTRY
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        let entry = map.get_mut(&tx_id_str).ok_or_else(|| {
                            InterpError::EvalError {
                                error: EvalError::InvalidInput(format!(
                                    "sender not found: {}",
                                    tx_id_str
                                )),
                                span: expr.span,
                            }
                        })?;
                        if let Some(sender) = entry.downcast_mut::<ChannelSender>() {
                            sender.tx.send(value).map_err(|_| InterpError::EvalError {
                                error: EvalError::InvalidInput(
                                    "channel closed (receiver dropped)".to_string(),
                                ),
                                span: expr.span,
                            })?;
                        } else if let Some(sender) =
                            entry.downcast_mut::<ChannelSyncSender>()
                        {
                            // Release the lock before blocking on sync_channel send
                            let tx_clone = sender.tx.clone();
                            drop(map);
                            tx_clone.send(value).map_err(|_| InterpError::EvalError {
                                error: EvalError::InvalidInput(
                                    "channel closed (receiver dropped)".to_string(),
                                ),
                                span: expr.span,
                            })?;
                        } else {
                            return Err(InterpError::EvalError {
                                error: EvalError::InvalidInput(format!(
                                    "not a sender: {}",
                                    tx_id_str
                                )),
                                span: expr.span,
                            });
                        }
                        return Ok(DataType::Null);
                    }
                    "chan_recv" => {
                        // chan_recv(receiver_id) -> value
                        if args.is_empty() {
                            return Err(InterpError::ArityMismatch {
                                name: "chan_recv".to_string(),
                                expected: "1".to_string(),
                                actual: 0,
                                span: expr.span,
                            });
                        }
                        let rx_id_val = self.eval_expr(&args[0])?;
                        let rx_id_str = match &rx_id_val {
                            DataType::String(s) => s.clone(),
                            _ => {
                                return Err(InterpError::TypeError {
                                    expected: "string (receiver ID)".to_string(),
                                    actual: rx_id_val.type_name().to_string(),
                                    context: "chan_recv".to_string(),
                                    span: expr.span,
                                })
                            }
                        };

                        // Clone the Arc<Mutex<Receiver>> so we can drop the
                        // registry lock before blocking on recv.
                        let rx_arc = {
                            let map = CHANNEL_REGISTRY
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            let entry = map.get(&rx_id_str).ok_or_else(|| {
                                InterpError::EvalError {
                                    error: EvalError::InvalidInput(format!(
                                        "receiver not found: {}",
                                        rx_id_str
                                    )),
                                    span: expr.span,
                                }
                            })?;
                            let receiver =
                                entry.downcast_ref::<ChannelReceiver>().ok_or_else(|| {
                                    InterpError::EvalError {
                                        error: EvalError::InvalidInput(format!(
                                            "not a receiver: {}",
                                            rx_id_str
                                        )),
                                        span: expr.span,
                                    }
                                })?;
                            Arc::clone(&receiver.rx)
                        };
                        // Registry lock is now released; safe to block.
                        let rx_guard = rx_arc.lock().unwrap_or_else(|e| e.into_inner());
                        return match rx_guard.recv() {
                            Ok(val) => Ok(val),
                            // Channel closed (all senders dropped) — return null
                            // instead of blocking forever or erroring.
                            Err(_) => Ok(DataType::Null),
                        };
                    }
                    "chan_try_recv" => {
                        // chan_try_recv(receiver_id) -> value or null
                        if args.is_empty() {
                            return Err(InterpError::ArityMismatch {
                                name: "chan_try_recv".to_string(),
                                expected: "1".to_string(),
                                actual: 0,
                                span: expr.span,
                            });
                        }
                        let rx_id_val = self.eval_expr(&args[0])?;
                        let rx_id_str = match &rx_id_val {
                            DataType::String(s) => s.clone(),
                            _ => {
                                return Err(InterpError::TypeError {
                                    expected: "string (receiver ID)".to_string(),
                                    actual: rx_id_val.type_name().to_string(),
                                    context: "chan_try_recv".to_string(),
                                    span: expr.span,
                                })
                            }
                        };

                        let rx_arc = {
                            let map = CHANNEL_REGISTRY
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            let entry = map.get(&rx_id_str).ok_or_else(|| {
                                InterpError::EvalError {
                                    error: EvalError::InvalidInput(format!(
                                        "receiver not found: {}",
                                        rx_id_str
                                    )),
                                    span: expr.span,
                                }
                            })?;
                            let receiver =
                                entry.downcast_ref::<ChannelReceiver>().ok_or_else(|| {
                                    InterpError::EvalError {
                                        error: EvalError::InvalidInput(format!(
                                            "not a receiver: {}",
                                            rx_id_str
                                        )),
                                        span: expr.span,
                                    }
                                })?;
                            Arc::clone(&receiver.rx)
                        };
                        let rx_guard = rx_arc.lock().unwrap_or_else(|e| e.into_inner());
                        return match rx_guard.try_recv() {
                            Ok(val) => Ok(val),
                            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(DataType::Null),
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => Ok(DataType::Null),
                        };
                    }
                    "chan_close" => {
                        // chan_close(endpoint_id) -> null
                        if args.is_empty() {
                            return Err(InterpError::ArityMismatch {
                                name: "chan_close".to_string(),
                                expected: "1".to_string(),
                                actual: 0,
                                span: expr.span,
                            });
                        }
                        let id_val = self.eval_expr(&args[0])?;
                        let id_str = match &id_val {
                            DataType::String(s) => s.clone(),
                            _ => {
                                return Err(InterpError::TypeError {
                                    expected: "string (channel ID)".to_string(),
                                    actual: id_val.type_name().to_string(),
                                    context: "chan_close".to_string(),
                                    span: expr.span,
                                })
                            }
                        };
                        channel_remove(&id_str).map_err(|e| InterpError::EvalError {
                            error: EvalError::InvalidInput(e),
                            span: expr.span,
                        })?;
                        return Ok(DataType::Null);
                    }
                    _ => {}
                }

                // Check std library aliases (from `use std::*` imports)
                let resolved_name = self
                    .std_op_aliases
                    .get(fn_name.as_str())
                    .cloned()
                    .unwrap_or_else(|| fn_name.clone());
                let op_type = OperationType::parse(&resolved_name).ok_or_else(|| {
                    InterpError::UnknownOperation {
                        name: fn_name.clone(),
                        span: expr.span,
                        suggestion: None,
                    }
                })?;

                let input_ports = op_input_ports(op_type);

                let inputs: HashMap<String, DataType> = args.iter().enumerate()
                    .map(|(i, arg)| {
                        let val = self.eval_expr(arg)?;
                        let port = if i < input_ports.len() {
                            input_ports[i].to_string()
                        } else {
                            format!("input_{}", i)
                        };
                        Ok((port, val))
                    })
                    .collect::<Result<_, InterpError>>()?;

                let config: HashMap<String, DataType> = kwargs.iter()
                    .map(|(key, val_expr)| Ok((key.clone(), self.eval_expr(val_expr)?)))
                    .collect::<Result<_, InterpError>>()?;

                self.evaluator.eval_operation(op_type, &inputs, &config).map_err(|e| InterpError::EvalError {
                    error: e,
                    span: expr.span,
                })
            }

            ExpressionKind::Pipe { left, right } => {
                let left_val = self.eval_expr(left)?;
                // For pipe, we need to substitute `_` with the left value
                self.eval_pipe_stage(&left_val, right)
            }

            ExpressionKind::IfElse {
                condition,
                then_block,
                else_block,
            } => {
                let cond = self.eval_expr(condition)?;
                let is_true = match &cond {
                    DataType::Bool(b) => *b,
                    other => {
                        return Err(InterpError::TypeError {
                            expected: "Bool".to_string(),
                            actual: other.type_name().to_string(),
                            context: "if condition".to_string(),
                            span: condition.span,
                        });
                    }
                };

                // Lazy evaluation: only execute the matching branch
                if is_true {
                    self.symbols.push(HashMap::new());
                    self.heap.push_scope();
                    let result = self.exec_block(then_block);
                    self.heap.pop_scope();
                    self.symbols.pop();
                    result
                } else if let Some(else_b) = else_block {
                    self.symbols.push(HashMap::new());
                    self.heap.push_scope();
                    let result = self.exec_block(else_b);
                    self.heap.pop_scope();
                    self.symbols.pop();
                    result
                } else {
                    Ok(DataType::Null)
                }
            }

            ExpressionKind::Block(block) => {
                self.symbols.push(HashMap::new());
                self.heap.push_scope();
                let result = self.exec_block(block);
                self.heap.pop_scope();
                self.symbols.pop();
                result
            }

            ExpressionKind::Index { object, index } => {
                // Slice syntax: arr[1..3] or str[0..5]
                if let ExpressionKind::Range { start: rs, end: re, inclusive } = &index.kind {
                    let obj = self.eval_expr(object)?;
                    // Propagate null from optional chaining through index
                    if matches!(obj, DataType::Null) && Self::has_optional_chain(&object.kind) {
                        return Ok(DataType::Null);
                    }
                    let s = self.eval_expr(rs)?;
                    let e = self.eval_expr(re)?;
                    return self.eval_slice(&obj, &s, &e, *inclusive, expr.span);
                }
                let obj = self.eval_expr(object)?;
                // Propagate null from optional chaining through index
                if matches!(obj, DataType::Null) && Self::has_optional_chain(&object.kind) {
                    return Ok(DataType::Null);
                }
                let idx = self.eval_expr(index)?;

                // Dispatch based on type: MapGet for maps, CharAt for strings, ArrayGet for arrays
                if matches!(obj, DataType::Map(_)) {
                    let inputs = HashMap::from([
                        ("map".to_string(), obj),
                        ("key".to_string(), idx),
                    ]);
                    self.evaluator.eval_operation(OperationType::MapGet, &inputs, &EMPTY_CONFIG).map_err(|e| {
                        InterpError::EvalError {
                            error: e,
                            span: expr.span,
                        }
                    })
                } else if matches!(obj, DataType::String(_)) {
                    let inputs = HashMap::from([
                        ("input".to_string(), obj),
                        ("index".to_string(), idx),
                    ]);
                    self.evaluator.eval_operation(OperationType::CharAt, &inputs, &EMPTY_CONFIG).map_err(|e| {
                        InterpError::EvalError {
                            error: e,
                            span: expr.span,
                        }
                    })
                } else {
                    let inputs = HashMap::from([
                        ("array".to_string(), obj),
                        ("index".to_string(), idx),
                    ]);
                    self.evaluator.eval_operation(OperationType::ArrayGet, &inputs, &EMPTY_CONFIG).map_err(|e| {
                        InterpError::EvalError {
                            error: e,
                            span: expr.span,
                        }
                    })
                }
            }

            ExpressionKind::FieldAccess { object, field } => {
                let obj = self.eval_expr(object)?;

                // Optional chaining: propagate null from ?. through field access
                if matches!(obj, DataType::Null) && Self::has_optional_chain(&object.kind) {
                    return Ok(DataType::Null);
                }

                // Check for property getter: if the object is a struct with a getter for this field
                if let DataType::Map(ref map) = obj {
                    if let Some(DataType::String(struct_name)) = map.get("__struct") {
                        if let Some(getter) = self.impl_methods
                            .get(struct_name)
                            .and_then(|m| m.get(field.as_str()))
                            .filter(|m| m.is_getter)
                            .cloned()
                        {
                            // Call the getter with `self` bound to the object
                            self.call_depth += 1;
                            if self.call_depth > MAX_CALL_DEPTH {
                                self.call_depth -= 1;
                                return Err(InterpError::ResourceLimit {
                                    limit: format!("{} calls", MAX_CALL_DEPTH),
                                    actual: format!("{} calls", self.call_depth + 1),
                                    context: format!("{}.{}", struct_name, field),
                                    span: expr.span,
                                });
                            }
                            self.symbols.push(HashMap::new());
                            self.heap.push_scope();
                            if let Some(p) = getter.params.first() {
                                let addr = self.heap.alloc(obj.clone());
                                self.define(&p.name, addr, false);
                            }
                            let result = self.exec_block(&getter.body);
                            self.heap.pop_scope();
                            self.symbols.pop();
                            self.call_depth -= 1;
                            return match result {
                                Ok(val) => Ok(val),
                                Err(InterpError::ReturnSignal(val)) => Ok(val),
                                Err(e) => Err(e),
                            };
                        }
                    }
                }

                let inputs = HashMap::from([
                    ("map".to_string(), obj),
                    ("key".to_string(), DataType::String(field.clone())),
                ]);

                self.evaluator.eval_operation(OperationType::MapGet, &inputs, &EMPTY_CONFIG).map_err(|e| {
                    InterpError::EvalError {
                        error: e,
                        span: expr.span,
                    }
                })
            }

            ExpressionKind::Placeholder => Err(InterpError::InvalidPlaceholder { span: expr.span }),

            ExpressionKind::Await(inner) => {
                let val = self.eval_expr(inner)?;
                match val {
                    DataType::Future(state) => match *state {
                        FutureState::Resolved(resolved) => Ok(*resolved),
                        FutureState::Rejected(err) => Err(InterpError::EvalError {
                            error: EvalError::TypeError {
                                expected: "resolved future".to_string(),
                                actual: format!("rejected: {}", err),
                                context: "await".to_string(),
                            },
                            span: expr.span,
                        }),
                        FutureState::Pending(ref task_id) => {
                            // Join the spawned thread and retrieve its result
                            match task_join(task_id) {
                                Ok(Ok(val)) => {
                                    // Recursively unwrap if the thread returned a Future
                                    match val {
                                        DataType::Future(inner_state) => match *inner_state {
                                            FutureState::Resolved(resolved) => Ok(*resolved),
                                            FutureState::Rejected(err) => Err(InterpError::EvalError {
                                                error: EvalError::TypeError {
                                                    expected: "resolved future".to_string(),
                                                    actual: format!("rejected: {}", err),
                                                    context: "await".to_string(),
                                                },
                                                span: expr.span,
                                            }),
                                            FutureState::Pending(ref inner_tid) => {
                                                // Nested pending: join the inner task too
                                                match task_join(inner_tid) {
                                                    Ok(Ok(v)) => Ok(v),
                                                    Ok(Err(e)) => Err(InterpError::EvalError {
                                                        error: EvalError::InvalidInput(
                                                            format!("spawned task failed: {}", e),
                                                        ),
                                                        span: expr.span,
                                                    }),
                                                    Err(e) => Err(InterpError::EvalError {
                                                        error: EvalError::InvalidInput(
                                                            format!("spawned task panicked: {}", e),
                                                        ),
                                                        span: expr.span,
                                                    }),
                                                }
                                            }
                                        },
                                        other => Ok(other),
                                    }
                                }
                                Ok(Err(err_msg)) => Err(InterpError::EvalError {
                                    error: EvalError::InvalidInput(
                                        format!("spawned task failed: {}", err_msg),
                                    ),
                                    span: expr.span,
                                }),
                                Err(join_err) => Err(InterpError::EvalError {
                                    error: EvalError::InvalidInput(
                                        format!("spawned task panicked: {}", join_err),
                                    ),
                                    span: expr.span,
                                }),
                            }
                        }
                    },
                    // Await on non-Future is identity
                    other => Ok(other),
                }
            }

            ExpressionKind::Spawn(inner) => {
                // Real concurrent spawn: evaluate the expression in a
                // new OS thread with a snapshot of the current interpreter
                // state (functions, enums, structs, std_op_aliases, closures).
                let expr_clone = (**inner).clone();
                let functions = self.functions.clone();
                let enum_defs = self.enum_defs.clone();
                let struct_defs = self.struct_defs.clone();
                let std_op_aliases = self.std_op_aliases.clone();
                let closure_captures = self.closure_captures.clone();
                let impl_methods = self.impl_methods.clone();
                let async_fns = self.async_fns.clone();

                // Snapshot visible variables so the spawned expression can
                // reference outer-scope values (capture by value).
                let mut captured_vars: Vec<(String, DataType, bool)> = Vec::new();
                for scope in &self.symbols {
                    for (name, entry) in scope {
                        if let Some(val) = self.heap.read(entry.addr) {
                            captured_vars.push((name.clone(), val.clone(), entry.mutable));
                        }
                    }
                }

                let tid = task_id();
                let tid_clone = tid.clone();

                let handle = std::thread::spawn(move || {
                    // Use a leaked SpawnEvaluator for the 'static lifetime
                    let evaluator: &'static dyn OperationEvaluator =
                        Box::leak(Box::new(SpawnEvaluator));
                    let mut interp = Interpreter::new(evaluator);
                    interp.functions = functions;
                    interp.enum_defs = enum_defs;
                    interp.struct_defs = struct_defs;
                    interp.std_op_aliases = std_op_aliases;
                    interp.closure_captures = closure_captures;
                    interp.impl_methods = impl_methods;
                    interp.async_fns = async_fns;

                    // Inject captured variables into the global scope
                    for (name, val, mutable) in captured_vars {
                        let addr = interp.heap.alloc(val);
                        interp.define(&name, addr, mutable);
                    }

                    interp
                        .eval_expr(&expr_clone)
                        .map_err(|e| format!("{}", e))
                });

                task_store(&tid_clone, handle).map_err(|e| InterpError::EvalError {
                    error: EvalError::InvalidInput(e),
                    span: expr.span,
                })?;

                Ok(DataType::Future(Box::new(FutureState::Pending(tid_clone))))
            }

            ExpressionKind::Range { start, end, inclusive } => {
                let start_val = self.eval_expr(start)?;
                let end_val = self.eval_expr(end)?;
                match (&start_val, &end_val) {
                    (DataType::Int64(a), DataType::Int64(b)) => {
                        let end_v = if *inclusive {
                            b.checked_add(1).ok_or_else(|| InterpError::TypeError {
                                expected: "range end within i64 bounds".to_string(),
                                actual: format!("{}..={} overflows", a, b),
                                context: "inclusive range".to_string(),
                                span: expr.span,
                            })?
                        } else {
                            *b
                        };
                        const MAX_RANGE_SIZE: i64 = 10_000_000;
                        let range_size = end_v.saturating_sub(*a);
                        if range_size > MAX_RANGE_SIZE {
                            return Err(InterpError::ResourceLimit {
                                limit: format!("{} elements", MAX_RANGE_SIZE),
                                actual: format!("{} elements", range_size),
                                context: "range creation".to_string(),
                                span: expr.span,
                            });
                        }
                        let arr: Vec<DataType> = (*a..end_v).map(DataType::Int64).collect();
                        Ok(DataType::Array(arr))
                    }
                    _ => {
                        // Fallback to evaluator for non-int ranges
                        let mut inputs = HashMap::from([
                            ("start".to_string(), start_val),
                            ("end".to_string(), end_val),
                        ]);
                        if *inclusive {
                            inputs.insert("inclusive".to_string(), DataType::Bool(true));
                        }
                        self.evaluator.eval_operation(OperationType::Range, &inputs, &EMPTY_CONFIG).map_err(|e| {
                            InterpError::EvalError {
                                error: e,
                                span: expr.span,
                            }
                        })
                    }
                }
            }

            ExpressionKind::MethodCall {
                object,
                method,
                args,
                kwargs,
            } => {
                let obj = self.eval_expr(object)?;

                // Optional chaining: if object came from ?. and evaluated to null, propagate null
                if matches!(obj, DataType::Null)
                    && Self::has_optional_chain(&object.kind)
                {
                    return Ok(DataType::Null);
                }

                // Try HOF methods first (they need interpreter for lambda calls)
                if let Some(result) = self.try_eval_hof_method(&obj, method, args, expr.span)? {
                    return Ok(result);
                }

                // Try direct interpreter methods (no OperationEvaluator needed)
                if let Some(result) = self.try_eval_direct_method(&obj, method, args, expr.span)? {
                    return Ok(result);
                }

                // Try impl block methods for struct values
                if let DataType::Map(ref map) = obj {
                    if let Some(DataType::String(struct_name)) = map.get("__struct") {
                        if let Some(impl_method) = self.impl_methods.get(struct_name).and_then(|m| m.get(method.as_str())).cloned() {
                            self.call_depth += 1;
                            if self.call_depth > MAX_CALL_DEPTH {
                                self.call_depth -= 1;
                                return Err(InterpError::ResourceLimit { limit: format!("{} calls", MAX_CALL_DEPTH), actual: format!("{} calls", self.call_depth + 1), context: format!("{}.{}", struct_name, method), span: expr.span });
                            }
                            self.symbols.push(HashMap::new());
                            self.heap.push_scope();
                            if let Some(p) = impl_method.params.first() {
                                let addr = self.heap.alloc(obj.clone());
                                self.define(&p.name, addr, false);
                            }
                            let mut eval_args = Vec::new();
                            for arg in args { eval_args.push(self.eval_expr(arg)?); }
                            for (i, param) in impl_method.params.iter().skip(1).enumerate() {
                                let val = if i < eval_args.len() { eval_args[i].clone() }
                                    else if let Some(default) = &param.default { self.eval_expr(default)? }
                                    else { DataType::Null };
                                let addr = self.heap.alloc(val);
                                self.define(&param.name, addr, false);
                            }
                            let result = self.exec_block(&impl_method.body);
                            self.heap.pop_scope();
                            self.symbols.pop();
                            self.call_depth -= 1;
                            return match result {
                                Ok(v) => Ok(v),
                                Err(InterpError::ReturnSignal(value)) => Ok(value),
                                Err(e) => Err(e),
                            };
                        }
                    }
                }

                let op_type =
                    resolve_method(&obj, method).ok_or_else(|| {
                        let available = available_methods_for_type(&obj);
                        let suggestion = super::errors::suggest_name(method, &available);
                        InterpError::UnknownOperation {
                            name: format!("{}.{}", obj.type_name(), method),
                            span: expr.span,
                            suggestion,
                        }
                    })?;
                let input_ports = op_input_ports(op_type);
                let mut inputs = HashMap::with_capacity(args.len() + 1);
                if let Some(p) = input_ports.first() {
                    inputs.insert(p.to_string(), obj);
                }
                for (i, arg) in args.iter().enumerate() {
                    let val = self.eval_expr(arg)?;
                    let port = if i + 1 < input_ports.len() {
                        input_ports[i + 1].to_string()
                    } else {
                        format!("input_{}", i + 1)
                    };
                    inputs.insert(port, val);
                }
                let config: HashMap<String, DataType> = kwargs.iter()
                    .map(|(key, val_expr)| Ok((key.clone(), self.eval_expr(val_expr)?)))
                    .collect::<Result<_, InterpError>>()?;
                self.evaluator.eval_operation(op_type, &inputs, &config).map_err(|e| InterpError::EvalError {
                    error: e,
                    span: expr.span,
                })
            }

            ExpressionKind::Lambda { params, body } => {
                let name = format!("__lambda_{}", self.lambda_counter);
                self.lambda_counter = self.lambda_counter.saturating_add(1);
                // Capture current scope variables (by value, innermost scope wins)
                let mut seen = std::collections::HashSet::new();
                let captures: Vec<(String, DataType, bool)> = self.symbols.iter().rev()
                    .flat_map(|scope| scope.iter())
                    .filter_map(|(var_name, entry)| {
                        if !seen.insert(var_name.clone()) { return None; }
                        self.heap.read(entry.addr)
                            .map(|val| (var_name.clone(), val.clone(), entry.mutable))
                    })
                    .collect();
                self.closure_captures.insert(name.clone(), captures);
                // Create function def from lambda body
                let func_body = Block {
                    statements: vec![],
                    tail_expr: Some(body.clone()),
                    tail_comments: Vec::new(),
                    span: body.span,
                };
                let func_def = FunctionDef {
                    name: name.clone(),
                    params: params.clone(),
                    return_type: None,
                    body: func_body,
                    span: expr.span,
                    is_getter: false,
                    is_setter: false,
                    deprecated: false,
                };
                self.functions.insert(name.clone(), func_def);
                Ok(DataType::String(name))
            }

            ExpressionKind::Match { value, arms } => {
                let val = self.eval_expr(value)?;
                for arm in arms {
                    if let Some(bindings) = match_pattern(&val, &arm.pattern) {
                        self.symbols.push(HashMap::new());
                        self.heap.push_scope();
                        for (bname, bval) in &bindings {
                            let addr = self.heap.alloc(bval.clone());
                            self.define(bname, addr, false);
                        }
                        // Check guard — ensure scope cleanup on error
                        let guard_result = if let Some(guard) = &arm.guard {
                            self.eval_expr(guard)
                        } else {
                            Ok(DataType::Bool(true))
                        };
                        let guard_ok = match guard_result {
                            Ok(DataType::Bool(b)) => b,
                            Ok(other) => {
                                self.heap.pop_scope();
                                self.symbols.pop();
                                return Err(InterpError::TypeError {
                                    expected: "Bool".to_string(),
                                    actual: other.type_name().to_string(),
                                    context: "match guard".to_string(),
                                    span: arm.guard.as_ref().map(|g| g.span).unwrap_or(arm.body.span),
                                });
                            }
                            Err(e) => {
                                self.heap.pop_scope();
                                self.symbols.pop();
                                return Err(e);
                            }
                        };
                        if guard_ok {
                            let result = self.exec_block(&arm.body);
                            self.heap.pop_scope();
                            self.symbols.pop();
                            return result;
                        }
                        self.heap.pop_scope();
                        self.symbols.pop();
                    }
                }
                Ok(DataType::Null)
            }

            ExpressionKind::StringInterpolation { parts } => {
                use std::fmt::Write;
                // Pre-calculate approximate capacity from literal segments to reduce
                // intermediate allocations. Each expression segment gets a small estimate.
                let estimated_cap: usize = parts.iter().map(|p| match p {
                    StringPart::Literal(s) => s.len(),
                    StringPart::Expr(_) => 16, // heuristic for typical interpolated values
                }).sum();
                let mut result = String::with_capacity(estimated_cap);
                for part in parts {
                    match part {
                        StringPart::Literal(s) => result.push_str(s),
                        StringPart::Expr(e) => {
                            let val = self.eval_expr(e)?;
                            // Write directly into the result buffer to avoid an
                            // intermediate String allocation from datatype_to_display.
                            // SAFETY: fmt::Write for String is infallible
                            write!(result, "{}", DataTypeDisplay(&val)).unwrap();
                        }
                    }
                    if result.len() > MAX_STRING_OUTPUT {
                        return Err(InterpError::ResourceLimit {
                            limit: format!("{} bytes", MAX_STRING_OUTPUT),
                            actual: format!("{} bytes", result.len()),
                            context: "string interpolation".to_string(),
                            span: expr.span,
                        });
                    }
                }
                Ok(DataType::String(result))
            }

            ExpressionKind::NullCoalesce { left, right } => {
                let lhs = self.eval_expr(left)?;
                if matches!(lhs, DataType::Null) {
                    self.eval_expr(right)
                } else {
                    Ok(lhs)
                }
            }

            ExpressionKind::OptionalChain { object, field } => {
                let obj = self.eval_expr(object)?;
                if matches!(obj, DataType::Null) {
                    Ok(DataType::Null)
                } else if field.is_empty() {
                    // Empty field = null-check marker for optional method calls (obj?.method())
                    // Just return the object itself — the MethodCall wrapper handles the actual call
                    Ok(obj)
                } else {
                    let inputs = HashMap::from([
                        ("map".to_string(), obj),
                        ("key".to_string(), DataType::String(field.clone())),
                    ]);
                    self.evaluator.eval_operation(OperationType::MapGet, &inputs, &EMPTY_CONFIG).map_err(|e| {
                        InterpError::EvalError {
                            error: e,
                            span: expr.span,
                        }
                    })
                }
            }

            ExpressionKind::Spread(_) => Err(InterpError::TypeError {
                expected: "array or map literal context".to_string(),
                actual: "standalone spread".to_string(),
                context: "spread can only be used in array/map literals".to_string(),
                span: expr.span,
            }),

            ExpressionKind::ListComprehension { expr: body_expr, pattern, iterable, condition } => {
                let iter_val = self.eval_expr(iterable)?;
                let items = match iter_val {
                    DataType::Array(arr) => arr,
                    DataType::Map(map) => {
                        map.into_iter()
                            .map(|(k, v)| {
                                let mut entry = indexmap::IndexMap::new();
                                entry.insert("key".to_string(), DataType::String(k));
                                entry.insert("value".to_string(), v);
                                DataType::Map(entry)
                            })
                            .collect()
                    }
                    DataType::String(s) => {
                        let char_count = s.chars().count();
                        if char_count > MAX_ARRAY_ELEMENTS {
                            return Err(InterpError::ResourceLimit {
                                limit: format!("{} chars", MAX_ARRAY_ELEMENTS),
                                actual: format!("{}", char_count),
                                context: "list comprehension string iteration".to_string(),
                                span: iterable.span,
                            });
                        }
                        s.chars()
                            .map(|c| DataType::String(c.to_string()))
                            .collect()
                    }
                    other => return Err(InterpError::TypeError {
                        expected: "Array, Map, or String".to_string(),
                        actual: other.type_name().to_string(),
                        context: "list comprehension".to_string(),
                        span: iterable.span,
                    }),
                };
                let mut result = Vec::new();
                for (iter_count, item) in items.into_iter().enumerate() {
                    if self.is_cancelled() {
                        return Err(InterpError::Cancelled);
                    }
                    if iter_count >= MAX_LOOP_ITERATIONS {
                        return Err(InterpError::MaxIterations { limit: MAX_LOOP_ITERATIONS, span: expr.span });
                    }
                    self.symbols.push(HashMap::new());
                    self.heap.push_scope();
                    let bind_result = match pattern {
                        ForPattern::Single(name) => {
                            let addr = self.heap.alloc(item);
                            self.define(name, addr, false);
                            Ok(())
                        }
                        ForPattern::ArrayDestructure(elements) => {
                            let destr = DestructurePattern::Array(elements.clone());
                            self.destructure_bind(&destr, &item, false, expr.span)
                        }
                        ForPattern::MapDestructure(entries) => {
                            let destr = DestructurePattern::Map(entries.clone());
                            self.destructure_bind(&destr, &item, false, expr.span)
                        }
                    };
                    if let Err(e) = bind_result {
                        self.heap.pop_scope();
                        self.symbols.pop();
                        return Err(e);
                    }
                    let iter_result = (|| {
                        let include = if let Some(cond) = condition {
                            let cond_val = self.eval_expr(cond)?;
                            cond_val.to_bool()
                        } else {
                            true
                        };
                        if include {
                            Ok(Some(self.eval_expr(body_expr)?))
                        } else {
                            Ok(None)
                        }
                    })();
                    self.heap.pop_scope();
                    self.symbols.pop();
                    if let Some(val) = iter_result? {
                        result.push(val);
                        if result.len() >= MAX_ARRAY_ELEMENTS {
                            return Err(InterpError::ResourceLimit {
                                limit: format!("{} elements", MAX_ARRAY_ELEMENTS),
                                actual: format!("{}", result.len()),
                                context: "list comprehension".to_string(),
                                span: expr.span,
                            });
                        }
                    }
                }
                Ok(DataType::Array(result))
            }

            ExpressionKind::MapComprehension { key_expr, value_expr, pattern, iterable, condition } => {
                let iter_val = self.eval_expr(iterable)?;
                let items = match iter_val {
                    DataType::Array(arr) => arr,
                    DataType::Map(map) => {
                        map.into_iter()
                            .map(|(k, v)| {
                                let mut entry = indexmap::IndexMap::new();
                                entry.insert("key".to_string(), DataType::String(k));
                                entry.insert("value".to_string(), v);
                                DataType::Map(entry)
                            })
                            .collect()
                    }
                    DataType::String(s) => {
                        let char_count = s.chars().count();
                        if char_count > MAX_ARRAY_ELEMENTS {
                            return Err(InterpError::ResourceLimit {
                                limit: format!("{} chars", MAX_ARRAY_ELEMENTS),
                                actual: format!("{}", char_count),
                                context: "map comprehension string iteration".to_string(),
                                span: iterable.span,
                            });
                        }
                        s.chars()
                            .map(|c| DataType::String(c.to_string()))
                            .collect()
                    }
                    other => return Err(InterpError::TypeError {
                        expected: "Array, Map, or String".to_string(),
                        actual: other.type_name().to_string(),
                        context: "map comprehension".to_string(),
                        span: iterable.span,
                    }),
                };
                let mut result = indexmap::IndexMap::new();
                for (iter_count, item) in items.into_iter().enumerate() {
                    if self.is_cancelled() {
                        return Err(InterpError::Cancelled);
                    }
                    if iter_count >= MAX_LOOP_ITERATIONS {
                        return Err(InterpError::MaxIterations { limit: MAX_LOOP_ITERATIONS, span: expr.span });
                    }
                    self.symbols.push(HashMap::new());
                    self.heap.push_scope();
                    let iter_result = (|| {
                        let bind_result = match pattern {
                            ForPattern::Single(name) => {
                                let addr = self.heap.alloc(item);
                                self.define(name, addr, false);
                                Ok(())
                            }
                            ForPattern::ArrayDestructure(elements) => {
                                let destr = DestructurePattern::Array(elements.clone());
                                self.destructure_bind(&destr, &item, false, expr.span)
                            }
                            ForPattern::MapDestructure(entries) => {
                                let destr = DestructurePattern::Map(entries.clone());
                                self.destructure_bind(&destr, &item, false, expr.span)
                            }
                        };
                        bind_result?;
                        let include = if let Some(cond) = condition {
                            let cond_val = self.eval_expr(cond)?;
                            cond_val.to_bool()
                        } else {
                            true
                        };
                        if include {
                            let k = self.eval_expr(key_expr)?;
                            let v = self.eval_expr(value_expr)?;
                            let key_str = match k {
                                DataType::String(s) => s,
                                other => other.to_string_lossy(),
                            };
                            Ok(Some((key_str, v)))
                        } else {
                            Ok(None)
                        }
                    })();
                    self.heap.pop_scope();
                    self.symbols.pop();
                    if let Some((k, v)) = iter_result? {
                        result.insert(k, v);
                        if result.len() >= MAX_ARRAY_ELEMENTS {
                            return Err(InterpError::ResourceLimit {
                                limit: format!("{} entries", MAX_ARRAY_ELEMENTS),
                                actual: format!("{}", result.len()),
                                context: "map comprehension".to_string(),
                                span: expr.span,
                            });
                        }
                    }
                }
                Ok(DataType::Map(result))
            }

            ExpressionKind::EnumConstruct { enum_name, variant, args } => {
                // Check if it's an enum construction or a qualified module function call.
                // The parser treats `Foo::Bar(args)` as EnumConstruct, but it could also
                // be a module function call like `math::double(5)`.
                if !self.enum_defs.contains_key(enum_name.as_str()) {
                    let qualified_name = format!("{}::{}", enum_name, variant);
                    if self.functions.contains_key(qualified_name.as_str()) {
                        let evaluated_args = self.eval_call_args(args)?;
                        return self.call_function(&qualified_name, &evaluated_args, expr.span);
                    }
                    return Err(InterpError::TypeError {
                        expected: "defined enum or module".to_string(),
                        actual: enum_name.clone(),
                        context: "enum construction or module call".to_string(),
                        span: expr.span,
                    });
                }
                let variants = match self.enum_defs.get(enum_name).cloned() {
                    Some(v) => v,
                    None => return Err(InterpError::TypeError {
                        expected: "defined enum".to_string(),
                        actual: enum_name.clone(),
                        context: "enum construction".to_string(),
                        span: expr.span,
                    }),
                };
                // Validate variant exists
                let variant_def = variants.iter().find(|v| v.name == *variant).ok_or_else(|| {
                    InterpError::TypeError {
                        expected: format!("variant of {}", enum_name),
                        actual: variant.clone(),
                        context: "enum construction".to_string(),
                        span: expr.span,
                    }
                })?;
                if args.len() != variant_def.fields.len() {
                    return Err(InterpError::ArityMismatch {
                        name: format!("{}::{}", enum_name, variant),
                        expected: variant_def.fields.len().to_string(),
                        actual: args.len(),
                        span: expr.span,
                    });
                }
                let evaluated_args: Vec<DataType> = args.iter()
                    .map(|a| self.eval_expr(a))
                    .collect::<Result<_, _>>()?;
                let mut map = indexmap::IndexMap::new();
                map.insert("__enum".to_string(), DataType::String(enum_name.clone()));
                map.insert("__variant".to_string(), DataType::String(variant.clone()));
                map.insert("__data".to_string(), DataType::Array(evaluated_args));
                Ok(DataType::Map(map))
            }

            ExpressionKind::StructConstruct { name, fields } => {
                // Validate struct exists
                let field_defs = self.struct_defs.get(name).cloned().ok_or_else(|| {
                    InterpError::TypeError {
                        expected: "defined struct".to_string(),
                        actual: name.clone(),
                        context: "struct construction".to_string(),
                        span: expr.span,
                    }
                })?;
                let mut map = indexmap::IndexMap::new();
                map.insert("__struct".to_string(), DataType::String(name.clone()));
                for (field_name, field_expr) in fields {
                    // Struct update syntax: `__spread` is the marker for `...base_expr`
                    if field_name == "__spread" {
                        let base_val = self.eval_expr(field_expr)?;
                        if let DataType::Map(base_map) = base_val {
                            for (k, v) in base_map {
                                if k != "__struct" {
                                    map.insert(k, v);
                                }
                            }
                        } else {
                            return Err(InterpError::TypeError {
                                expected: "Map or struct".to_string(),
                                actual: base_val.type_name().to_string(),
                                context: "struct update spread".to_string(),
                                span: expr.span,
                            });
                        }
                        continue;
                    }
                    if map.contains_key(field_name) && field_name != "__struct" {
                        // Override spread fields with explicit fields (not an error)
                    }
                    let val = self.eval_expr(field_expr)?;
                    map.insert(field_name.clone(), val);
                }
                // Fill in defaults for missing fields (#294)
                for fd in &field_defs {
                    if !map.contains_key(&fd.name) {
                        if let Some(ref default_expr) = fd.default {
                            let default_val = self.eval_expr(default_expr)?;
                            map.insert(fd.name.clone(), default_val);
                        }
                    }
                }
                // Validate all required fields are present (fields without defaults)
                for fd in &field_defs {
                    if !map.contains_key(&fd.name) {
                        return Err(InterpError::TypeError {
                            expected: format!("field '{}'", fd.name),
                            actual: "missing".to_string(),
                            context: format!("struct '{}' construction", name),
                            span: expr.span,
                        });
                    }
                }
                // Reject unknown fields
                let known_fields: Vec<&str> = field_defs.iter().map(|f| f.name.as_str()).collect();
                for (field_name, _) in fields {
                    if field_name != "__struct" && field_name != "__spread" && !known_fields.contains(&field_name.as_str()) {
                        return Err(InterpError::TypeError {
                            expected: format!("known field of struct '{}'", name),
                            actual: field_name.clone(),
                            context: format!("struct '{}' has no field '{}'", name, field_name),
                            span: expr.span,
                        });
                    }
                }
                Ok(DataType::Map(map))
            }

            ExpressionKind::TryPropagate(inner) => {
                let span = expr.span;
                match self.eval_expr(inner) {
                    Ok(DataType::Null) => Err(InterpError::ThrownError {
                        value: DataType::String("unwrap on null value".to_string()),
                        span,
                    }),
                    Ok(val) => {
                        // Check if it's a Result enum
                        if let DataType::Map(ref m) = val {
                            if m.get("__enum").map(|v| v.to_string_lossy()) == Some("Result".to_string()) {
                                let variant = m.get("__variant").map(|v| v.to_string_lossy());
                                if variant.as_deref() == Some("Err") {
                                    // Result::Err — throw the error value
                                    let error_val = m.get("__data")
                                        .and_then(|d| if let DataType::Array(arr) = d { arr.first().cloned() } else { None })
                                        .unwrap_or(val.clone());
                                    return Err(InterpError::ThrownError {
                                        value: error_val,
                                        span,
                                    });
                                }
                                if variant.as_deref() == Some("Ok") {
                                    // Result::Ok — unwrap the inner value
                                    let ok_val = m.get("__data")
                                        .and_then(|d| if let DataType::Array(arr) = d { arr.first().cloned() } else { None })
                                        .unwrap_or(DataType::Null);
                                    return Ok(ok_val);
                                }
                            }
                        }
                        Ok(val)
                    }
                    // Propagate control flow signals as-is
                    Err(e) if is_control_flow(&e) => Err(e),
                    // Convert runtime errors to thrown errors (catchable by try/catch)
                    Err(e) => {
                        Err(InterpError::ThrownError {
                            value: DataType::String(format!("{}", e)),
                            span,
                        })
                    }
                }
            }

            ExpressionKind::Loop { label, body: block } => {
                let mut iterations = 0;
                loop {
                    if self.is_cancelled() {
                        return Err(InterpError::Cancelled);
                    }
                    if iterations >= MAX_LOOP_ITERATIONS {
                        return Err(InterpError::MaxIterations {
                            limit: MAX_LOOP_ITERATIONS,
                            span: expr.span,
                        });
                    }
                    self.symbols.push(HashMap::new());
                    self.heap.push_scope();
                    let result = self.exec_block(block);
                    self.heap.pop_scope();
                    self.symbols.pop();
                    match result {
                        Ok(_) => {}
                        Err(InterpError::BreakSignal(val)) => return Ok(val),
                        Err(InterpError::LabeledBreak { label: ref lbl, ref value })
                            if label.as_deref() == Some(lbl.as_str()) => return Ok(value.clone()),
                        Err(InterpError::ContinueSignal) => {}
                        Err(InterpError::LabeledContinue { label: ref lbl })
                            if label.as_deref() == Some(lbl.as_str()) => {}
                        Err(e) => return Err(e),
                    }
                    iterations += 1;
                    self.maybe_gc();
                }
            }

            ExpressionKind::TryCatchExpr {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                // Scope-isolate the try block so variables don't leak
                self.symbols.push(HashMap::new());
                self.heap.push_scope();
                let try_result = self.exec_block(try_block);
                self.heap.pop_scope();
                self.symbols.pop();
                let result = match try_result {
                    Ok(val) => Ok(val),
                    Err(ref e) if is_control_flow(e) => {
                        // Execute finally even on control flow (return/break/continue)
                        if let Some(finally) = finally_block {
                            self.symbols.push(HashMap::new());
                            self.heap.push_scope();
                            let finally_result = self.exec_block(finally);
                            self.heap.pop_scope();
                            self.symbols.pop();
                            finally_result?;
                        }
                        return try_result;
                    }
                    Err(e) => {
                        let catch_value = match e {
                            InterpError::ThrownError { value, .. } => value,
                            other => DataType::String(format!("{}", other)),
                        };
                        self.symbols.push(HashMap::new());
                        self.heap.push_scope();
                        if let Some(var_name) = catch_var {
                            let addr = self.heap.alloc(catch_value);
                            self.define(var_name, addr, false);
                        }
                        let catch_result = self.exec_block(catch_block);
                        self.heap.pop_scope();
                        self.symbols.pop();
                        catch_result
                    }
                };
                // Execute finally block if present (always runs, can override result)
                if let Some(finally) = finally_block {
                    self.symbols.push(HashMap::new());
                    self.heap.push_scope();
                    let finally_result = self.exec_block(finally);
                    self.heap.pop_scope();
                    self.symbols.pop();
                    finally_result?;
                }
                result
            }
        }
    }

    /// Evaluate a pipe stage, substituting `_` with the piped value.
    fn eval_pipe_stage(
        &mut self,
        piped_value: &DataType,
        stage: &Expression,
    ) -> Result<DataType, InterpError> {
        match &stage.kind {
            ExpressionKind::Call {
                name: fn_name,
                args,
                kwargs,
            } => {
                // Handle built-in functions directly (these aren't in self.functions)
                if fn_name == "len" {
                    let evaluated_args: Vec<DataType> = args.iter()
                        .map(|arg| {
                            if matches!(arg.kind, ExpressionKind::Placeholder) {
                                Ok(piped_value.clone())
                            } else {
                                self.eval_expr(arg)
                            }
                        })
                        .collect::<Result<_, _>>()?;
                    // If no args provided, use the piped value
                    let val = evaluated_args.first().unwrap_or(piped_value);
                    let length = match val {
                        DataType::Array(a) => a.len() as i64,
                        DataType::String(s) => s.chars().count() as i64,
                        DataType::Map(m) => m.len() as i64,
                        DataType::Bytes(b) => b.len() as i64,
                        other => {
                            return Err(InterpError::TypeError {
                                expected: "Array, String, Map, or Bytes".to_string(),
                                actual: other.type_name().to_string(),
                                context: "len".to_string(),
                                span: stage.span,
                            });
                        }
                    };
                    return Ok(DataType::Int64(length));
                }
                if fn_name == "typeof" {
                    let evaluated_args: Vec<DataType> = args.iter()
                        .map(|arg| {
                            if matches!(arg.kind, ExpressionKind::Placeholder) {
                                Ok(piped_value.clone())
                            } else {
                                self.eval_expr(arg)
                            }
                        })
                        .collect::<Result<_, _>>()?;
                    let val = evaluated_args.first().unwrap_or(piped_value);
                    return Ok(DataType::String(val.type_name().to_string()));
                }
                if matches!(fn_name.as_str(), "println" | "print" | "debug_log") {
                    let evaluated_args: Vec<DataType> = args.iter()
                        .map(|arg| {
                            if matches!(arg.kind, ExpressionKind::Placeholder) {
                                Ok(piped_value.clone())
                            } else {
                                self.eval_expr(arg)
                            }
                        })
                        .collect::<Result<_, _>>()?;
                    let val = evaluated_args.first().unwrap_or(piped_value);
                    self.logs.push(LogEntry {
                        level: if fn_name == "debug_log" { LogLevel::Debug } else { LogLevel::Info },
                        message: datatype_to_display(val),
                        line: Some(stage.span.start_line),
                        node_id: None,
                    });
                    return Ok(val.clone());
                }
                if fn_name == "assert" {
                    let evaluated_args: Vec<DataType> = args.iter()
                        .map(|arg| {
                            if matches!(arg.kind, ExpressionKind::Placeholder) {
                                Ok(piped_value.clone())
                            } else {
                                self.eval_expr(arg)
                            }
                        })
                        .collect::<Result<_, _>>()?;
                    let val = evaluated_args.first().unwrap_or(piped_value);
                    if val.to_bool() {
                        return Ok(DataType::Null);
                    } else {
                        let msg = evaluated_args.get(1).map(datatype_to_display)
                            .unwrap_or_else(|| "Assertion failed".to_string());
                        return Err(InterpError::AssertionFailed { message: msg, span: stage.span });
                    }
                }
                if matches!(fn_name.as_str(), "assert_eq" | "assert_ne") {
                    let has_placeholder = args.iter().any(|a| matches!(a.kind, ExpressionKind::Placeholder));
                    let evaluated_args: Vec<DataType> = args.iter()
                        .map(|arg| {
                            if matches!(arg.kind, ExpressionKind::Placeholder) {
                                Ok(piped_value.clone())
                            } else {
                                self.eval_expr(arg)
                            }
                        })
                        .collect::<Result<_, _>>()?;
                    // When no placeholder is used, prepend piped value as first arg
                    let (left, right) = if !has_placeholder && evaluated_args.len() < 2 {
                        (piped_value as &DataType, evaluated_args.first().unwrap_or(&DataType::Null))
                    } else {
                        (evaluated_args.first().unwrap_or(piped_value), evaluated_args.get(1).unwrap_or(&DataType::Null))
                    };
                    let eq = left == right;
                    let pass = if fn_name == "assert_eq" { eq } else { !eq };
                    if pass { return Ok(DataType::Null); }
                    let msg = evaluated_args.get(2).map(datatype_to_display).unwrap_or_else(|| {
                        if fn_name == "assert_eq" {
                            format!("Assertion failed: expected values to be equal\n  left:  {}\n  right: {}", datatype_to_display(left), datatype_to_display(right))
                        } else {
                            format!("Assertion failed: expected values to differ, both are: {}", datatype_to_display(left))
                        }
                    });
                    return Err(InterpError::AssertionFailed { message: msg, span: stage.span });
                }
                if self.functions.contains_key(fn_name.as_str()) {
                    let evaluated_args = self.eval_pipe_call_args(args, piped_value)?;
                    return self.call_function(fn_name, &evaluated_args, stage.span);
                }

                // Check if it's a variable holding a function reference (lambda)
                if let Some(entry) = self.lookup(fn_name) {
                    let addr = entry.addr;
                    if let Some(DataType::String(ref_name)) = self.heap.read(addr).cloned() {
                        if self.functions.contains_key(ref_name.as_str()) {
                            let evaluated_args = self.eval_pipe_call_args(args, piped_value)?;
                            return self.call_function(&ref_name, &evaluated_args, stage.span);
                        }
                    }
                }

                // Check std library aliases (from `use std::*` imports)
                let resolved_name = self
                    .std_op_aliases
                    .get(fn_name.as_str())
                    .cloned()
                    .unwrap_or_else(|| fn_name.clone());
                let op_type =
                    OperationType::parse(&resolved_name).ok_or_else(|| InterpError::UnknownOperation {
                        name: fn_name.clone(),
                        span: stage.span,
                        suggestion: None,
                    })?;

                let input_ports = op_input_ports(op_type);

                let evaluated_args = self.eval_pipe_call_args(args, piped_value)?;
                let inputs: HashMap<String, DataType> = evaluated_args.into_iter().enumerate()
                    .map(|(i, val)| {
                        let port = if i < input_ports.len() {
                            input_ports[i].to_string()
                        } else {
                            format!("input_{}", i)
                        };
                        (port, val)
                    })
                    .collect();

                let config: HashMap<String, DataType> = kwargs.iter()
                    .map(|(key, val_expr)| Ok((key.clone(), self.eval_expr(val_expr)?)))
                    .collect::<Result<_, InterpError>>()?;

                self.evaluator.eval_operation(op_type, &inputs, &config).map_err(|e| InterpError::EvalError {
                    error: e,
                    span: stage.span,
                })
            }
            ExpressionKind::Pipe { left, right } => {
                let mid = self.eval_pipe_stage(piped_value, left)?;
                self.eval_pipe_stage(&mid, right)
            }
            _ => Err(InterpError::InvalidPipeStage { span: stage.span }),
        }
    }

    /// Check if an expression kind contains an OptionalChain node (for null propagation).
    fn has_optional_chain(kind: &ExpressionKind) -> bool {
        match kind {
            ExpressionKind::OptionalChain { .. } => true,
            ExpressionKind::FieldAccess { object, .. }
            | ExpressionKind::Index { object, .. }
            | ExpressionKind::MethodCall { object, .. } => Self::has_optional_chain(&object.kind),
            _ => false,
        }
    }
}

impl<'a> Interpreter<'a> {
    /// Bind variables from a destructuring pattern.
    fn destructure_bind(
        &mut self,
        pattern: &DestructurePattern,
        value: &DataType,
        mutable: bool,
        span: Span,
    ) -> Result<(), InterpError> {
        match pattern {
            DestructurePattern::Array(elements) => {
                let arr = match value {
                    DataType::Array(a) => a,
                    _ => {
                        return Err(InterpError::TypeError {
                            expected: "Array".to_string(),
                            actual: value.type_name().to_string(),
                            context: "array destructuring".to_string(),
                            span,
                        })
                    }
                };
                // Find the rest element position (if any)
                let rest_pos = elements
                    .iter()
                    .position(|e| matches!(e, DestructureElement::Rest(_)));
                let trailing_count = rest_pos.map_or(0, |p| elements.len() - p - 1);

                // Count non-rest elements before and after rest
                let before_rest = rest_pos.unwrap_or(elements.len());
                // Validate array has enough elements for fixed positions
                let required = before_rest + trailing_count;
                if arr.len() < required {
                    return Err(InterpError::TypeError {
                        expected: format!("at least {} elements", required),
                        actual: format!("{} elements", arr.len()),
                        context: "array destructuring".to_string(),
                        span,
                    });
                }
                for (i, elem) in elements.iter().enumerate() {
                    match elem {
                        DestructureElement::Name(name) => {
                            let val = if rest_pos.is_some_and(|rp| i > rp) {
                                // Element after rest: index from end, but only if
                                // there are enough elements to avoid overlap
                                let idx = arr.len().saturating_sub(elements.len().saturating_sub(i));
                                if idx >= before_rest && idx < arr.len() {
                                    arr[idx].clone()
                                } else {
                                    DataType::Null
                                }
                            } else {
                                arr.get(i).cloned().unwrap_or(DataType::Null)
                            };
                            let addr = self.heap.alloc(val);
                            self.define(name, addr, mutable);
                        }
                        DestructureElement::Rest(name) => {
                            let end = arr.len().saturating_sub(trailing_count);
                            let remaining = if i <= end {
                                arr[i..end].to_vec()
                            } else {
                                Vec::new()
                            };
                            let addr = self.heap.alloc(DataType::Array(remaining));
                            self.define(name, addr, mutable);
                        }
                    }
                }
                Ok(())
            }
            DestructurePattern::Map(entries) => {
                let map = match value {
                    DataType::Map(m) => m,
                    _ => {
                        return Err(InterpError::TypeError {
                            expected: "Map".to_string(),
                            actual: value.type_name().to_string(),
                            context: "map destructuring".to_string(),
                            span,
                        })
                    }
                };
                for (key, alias) in entries {
                    let val = map.get(key).cloned().unwrap_or(DataType::Null);
                    let bind_name = alias.as_deref().unwrap_or(key.as_str());
                    let addr = self.heap.alloc(val);
                    self.define(bind_name, addr, mutable);
                }
                Ok(())
            }
        }
    }

    /// Handle `use std::module::func` imports from the standard library.
    fn handle_std_use(
        &mut self,
        path: &[String],
        alias: Option<&str>,
        glob: bool,
        span: Span,
    ) -> Result<DataType, InterpError> {
        // path is ["std", "math", "sqrt"] or ["std", "math"]
        if path.len() < 2 {
            return Err(InterpError::UnknownOperation {
                name: "std".to_string(),
                span,
                suggestion: None,
            });
        }
        let module = path[1].as_str();
        let ops = std_module_ops(module);
        if ops.is_empty() {
            let suggestion = super::errors::suggest_name(module, STD_MODULE_NAMES);
            return Err(InterpError::UndefinedFunction {
                name: format!("std::{}", module),
                span,
                suggestion,
            });
        }

        if glob || path.len() == 2 {
            // `use std::math::*` or `use std::math` (glob import all)
            for op_name in ops {
                self.std_op_aliases
                    .insert(op_name.to_string(), op_name.to_string());
            }
        } else if path.len() >= 3 {
            // `use std::math::sqrt` or `use std::math::sqrt as s`
            let func_name = &path[2];
            if ops.contains(&func_name.as_str()) {
                let local_name = alias.unwrap_or(func_name.as_str());
                self.std_op_aliases
                    .insert(local_name.to_string(), func_name.clone());
            } else {
                let suggestion = super::errors::suggest_name(func_name, &ops);
                return Err(InterpError::UnknownOperation {
                    name: format!("std::{}::{}", module, func_name),
                    span,
                    suggestion,
                });
            }
        }
        Ok(DataType::Null)
    }

    /// Handle `use pkg::name::function` imports.
    ///
    /// Resolves package functions from the pre-loaded package registry
    /// and registers them in the interpreter's function table.
    fn handle_pkg_use(
        &mut self,
        path: &[String],
        alias: Option<&str>,
        glob: bool,
        span: Span,
    ) -> Result<DataType, InterpError> {
        // path is ["pkg", "collections", "sorted_unique"] or ["pkg", "collections"]
        if path.len() < 2 {
            return Err(InterpError::UnknownOperation {
                name: "pkg".to_string(),
                span,
                suggestion: None,
            });
        }
        let package_id = &path[1];

        // Circular import detection
        if self.importing_packages.contains(package_id) {
            return Err(InterpError::EvalError {
                error: EvalError::InvalidInput(
                    format!("circular import detected: pkg::{} is already being imported", package_id),
                ),
                span,
            });
        }
        self.importing_packages.insert(package_id.clone());

        // Try exact match first, then with underscores→hyphens (identifiers use _)
        let pkg = match self.packages.get(package_id).or_else(|| {
            let hyphenated = package_id.replace('_', "-");
            self.packages.get(&hyphenated)
        }) {
            Some(p) => p.clone(),
            None => {
                self.importing_packages.remove(package_id);
                let available: Vec<&str> = self.packages.keys().map(|s| s.as_str()).collect();
                let suggestion = super::errors::suggest_name(package_id, &available);
                return Err(InterpError::UnknownOperation {
                    name: format!("pkg::{}", package_id),
                    span,
                    suggestion,
                });
            }
        };

        // Save state that may be modified by package use statements
        let saved_aliases = self.std_op_aliases.clone();
        let saved_enums = self.enum_defs.clone();
        let saved_structs = self.struct_defs.clone();
        let saved_functions = self.functions.clone();

        // Execute the package's own use statements (e.g. `use std::array`)
        // so that std aliases are available when package functions run
        for use_stmt in &pkg.use_statements {
            if let Err(e) = self.exec_statement(use_stmt) {
                self.std_op_aliases = saved_aliases;
                self.enum_defs = saved_enums;
                self.struct_defs = saved_structs;
                self.functions = saved_functions;
                self.importing_packages.remove(package_id);
                return Err(InterpError::EvalError {
                    error: EvalError::InvalidInput(format!("package '{}' setup failed: {}", package_id, e)),
                    span,
                });
            }
        }

        if glob || path.len() == 2 {
            // `use pkg::collections::*` or `use pkg::collections` — import all exports
            // Register all package enum/struct definitions for glob/module-level imports
            for (name, variants) in &pkg.enum_defs {
                self.enum_defs.insert(name.clone(), variants.clone());
            }
            for (name, fields) in &pkg.struct_defs {
                self.struct_defs.insert(name.clone(), fields.clone());
            }
            for (name, func) in &pkg.functions {
                self.functions.insert(name.clone(), func.clone());
            }
        } else if path.len() >= 3 {
            // `use pkg::collections::sorted_unique` or with alias
            let func_name = &path[2];
            let local_name = alias.unwrap_or(func_name.as_str());
            if let Some(func) = pkg.functions.get(func_name) {
                self.functions.insert(local_name.to_string(), func.clone());
            } else if let Some((_, variants)) = pkg.enum_defs.iter().find(|(n, _)| n == func_name) {
                self.enum_defs.insert(local_name.to_string(), variants.clone());
            } else if let Some((_, fields)) = pkg.struct_defs.iter().find(|(n, _)| n == func_name) {
                self.struct_defs.insert(local_name.to_string(), fields.clone());
            } else {
                self.enum_defs = saved_enums;
                self.struct_defs = saved_structs;
                self.std_op_aliases = saved_aliases;
                self.functions = saved_functions;
                self.importing_packages.remove(package_id);
                let mut available: Vec<&str> = pkg.functions.keys().map(|s| s.as_str()).collect();
                for (n, _) in &pkg.enum_defs {
                    available.push(n.as_str());
                }
                for (n, _) in &pkg.struct_defs {
                    available.push(n.as_str());
                }
                let suggestion = super::errors::suggest_name(func_name, &available);
                return Err(InterpError::UnknownOperation {
                    name: format!("pkg::{}::{}", package_id, func_name),
                    span,
                    suggestion,
                });
            }
        }
        self.importing_packages.remove(package_id);
        Ok(DataType::Null)
    }

    /// Execute a program in debug mode. Sends Completed/Error events when done.
    pub fn execute_debug(&mut self, program: &Program) -> Result<DataType, InterpError> {
        let result = self.execute(program);
        match &result {
            Ok(val) => {
                if let Some(ref debug) = self.debug {
                    let _ = debug.event_sender.blocking_send(DebugEvent::Completed {
                        result: datatype_to_display(val),
                    });
                }
            }
            Err(e) => {
                let (line, col) = match e {
                    InterpError::UndefinedVariable { span, .. }
                    | InterpError::ImmutableAssignment { span, .. }
                    | InterpError::UnknownOperation { span, .. }
                    | InterpError::TypeError { span, .. }
                    | InterpError::EvalError { span, .. }
                    | InterpError::MaxIterations { span, .. }
                    | InterpError::UndefinedFunction { span, .. }
                    | InterpError::MaxCallDepth { span, .. }
                    | InterpError::ArityMismatch { span, .. }
                    | InterpError::BreakOutsideLoop { span }
                    | InterpError::ContinueOutsideLoop { span }
                    | InterpError::ReturnOutsideFunction { span }
                    | InterpError::NotImplemented { span, .. }
                    | InterpError::ResourceLimit { span, .. }
                    | InterpError::ThrownError { span, .. }
                    | InterpError::InvalidPlaceholder { span }
                    | InterpError::InvalidPipeStage { span }
                    | InterpError::AssertionFailed { span, .. } => (span.start_line, span.start_col),
                    _ => (0, 0),
                };
                if let Some(ref debug) = self.debug {
                    let _ = debug.event_sender.blocking_send(DebugEvent::Error {
                        message: e.to_string(),
                        line,
                        column: col,
                    });
                }
            }
        }
        result
    }

    /// Run all test definitions in a program, returning results for each test.
    ///
    /// Algorithm:
    /// 1. Collect function defs (same as `execute()`)
    /// 2. Execute non-test top-level statements (setup code)
    /// 3. For each `TestDef`, run body in isolated scope; capture pass/fail + logs
    pub fn run_tests(&mut self, program: &Program) -> Vec<TestResult> {
        let mut results = Vec::new();

        // Pass 1: Collect function/enum/struct definitions and run setup code
        for stmt in &program.statements {
            match &stmt.kind {
                StatementKind::FunctionDef(func) => {
                    self.functions.insert(func.name.clone(), func.clone());
                }
                StatementKind::AsyncFunctionDef(func) => {
                    self.async_fns.insert(func.name.clone());
                    self.functions.insert(func.name.clone(), func.clone());
                }
                StatementKind::EnumDef { name, variants, .. } => {
                    self.enum_defs.insert(name.clone(), variants.clone());
                }
                StatementKind::StructDef { name, fields, .. } => {
                    self.struct_defs.insert(name.clone(), fields.clone());
                }
                StatementKind::ModuleDef { name, body } => {
                    self.register_module(name, body);
                }
                StatementKind::ImplBlock { type_name, methods } => {
                    let tm = self.impl_methods.entry(type_name.clone()).or_default();
                    for m in methods { tm.insert(m.name.clone(), m.clone()); }
                }
                StatementKind::TraitDef { name, methods } => {
                    self.trait_defs.insert(name.clone(), methods.clone());
                }
                StatementKind::ImplTrait { type_name, methods, .. } => {
                    let tm = self.impl_methods.entry(type_name.clone()).or_default();
                    for m in methods { tm.insert(m.name.clone(), m.clone()); }
                }
                StatementKind::TestDef { .. } => {
                    // Skip tests in pass 1
                }
                _ => {
                    // Execute setup code (imports, let bindings, etc.)
                    if let Err(e) = self.exec_statement(stmt) {
                        if is_control_flow(&e) {
                            continue;
                        }
                        // If setup fails, report as a setup error
                        results.push(TestResult {
                            name: format!("setup (line {})", stmt.span.start_line),
                            passed: false,
                            error_message: Some(e.to_string()),
                        });
                        return results;
                    }
                }
            }
        }

        // Pass 2: Run each test
        for stmt in &program.statements {
            if let StatementKind::TestDef { name, body } = &stmt.kind {
                // Save state for isolation
                let saved_symbols_len = self.symbols.len();
                let saved_functions = self.functions.clone();
                let saved_aliases = self.std_op_aliases.clone();
                let saved_enums = self.enum_defs.clone();
                let saved_structs = self.struct_defs.clone();
                let saved_closures = self.closure_captures.clone();
                let saved_stacks = self.saved_symbol_stacks.clone();
                let saved_async_fns = self.async_fns.clone();
                let saved_importing = self.importing_packages.clone();
                let saved_logs = self.logs.clone();
                let saved_lambda_counter = self.lambda_counter;
                let saved_imports = self.imports.clone();
                let saved_call_depth = self.call_depth;
                let saved_heap = self.heap.clone();
                self.symbols.push(HashMap::new());
                self.heap.push_scope();

                let test_result = self.exec_block(body);

                // Restore all state (heap restore ensures heap value mutations don't leak)
                self.heap = saved_heap;
                while self.symbols.len() > saved_symbols_len {
                    self.symbols.pop();
                }
                self.functions = saved_functions;
                self.std_op_aliases = saved_aliases;
                self.enum_defs = saved_enums;
                self.struct_defs = saved_structs;
                self.closure_captures = saved_closures;
                self.saved_symbol_stacks = saved_stacks;
                self.async_fns = saved_async_fns;
                self.importing_packages = saved_importing;
                self.logs = saved_logs;
                self.lambda_counter = saved_lambda_counter;
                self.imports = saved_imports;
                self.call_depth = saved_call_depth;

                match test_result {
                    Ok(_) => {
                        results.push(TestResult {
                            name: name.clone(),
                            passed: true,
                            error_message: None,
                        });
                    }
                    Err(e) if is_control_flow(&e) => {
                        results.push(TestResult {
                            name: name.clone(),
                            passed: false,
                            error_message: Some(format!("unexpected control flow in test: {}", e)),
                        });
                    }
                    Err(e) => {
                        results.push(TestResult {
                            name: name.clone(),
                            passed: false,
                            error_message: Some(e.to_string()),
                        });
                    }
                }
            }
        }

        results
    }
}

/// Result of running a single test.
#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub error_message: Option<String>,
}

// =============================================================================
// Debug / Breakpoint Support
// =============================================================================

/// Step mode for the debugger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMode {
    /// Run until the next breakpoint.
    Continue,
    /// Step to the next statement at the same or shallower call depth.
    StepOver,
    /// Step to the very next statement.
    StepInto,
    /// Run until returning from the current function.
    StepOut,
}

/// A variable snapshot sent to the debug client when paused.
#[derive(Debug, Clone)]
pub struct DebugVariable {
    pub name: String,
    pub value: String,
    pub type_name: String,
    pub scope: String,
}

/// Events emitted from the interpreter to the debug client.
#[derive(Debug, Clone)]
pub enum DebugEvent {
    /// Execution paused at a breakpoint or step.
    Paused {
        line: u32,
        column: u32,
        variables: Vec<DebugVariable>,
        call_stack: Vec<String>,
    },
    /// Execution completed normally.
    Completed { result: String },
    /// Execution ended with an error.
    Error {
        message: String,
        line: u32,
        column: u32,
    },
    /// Result of evaluating an expression in the current scope.
    EvaluateResult { result: String, error: Option<String> },
}

/// Commands sent from the debug client to the interpreter.
#[derive(Debug, Clone)]
pub enum DebugCommand {
    Continue,
    StepOver,
    StepInto,
    StepOut,
    /// Evaluate an expression in the current scope and return the result.
    Evaluate(String),
}

/// Debug state held by the interpreter during a debug session.
pub struct DebugState {
    pub breakpoints: std::collections::HashSet<u32>,
    pub step_mode: StepMode,
    pub event_sender: tokio::sync::mpsc::Sender<DebugEvent>,
    pub command_receiver: tokio::sync::mpsc::Receiver<DebugCommand>,
    /// Call depth when a step-over or step-out was initiated.
    pub step_start_depth: usize,
    /// Function call stack names for display.
    pub call_stack: Vec<String>,
}

/// All available standard library module names.
pub const STD_MODULE_NAMES: &[&str] = &[
    "math", "cmp", "logic", "bits", "str", "convert", "array", "map",
    "bytes", "json", "time", "hash", "io", "control", "rand", "fs",
    "env", "net", "tcp", "udp", "ws", "sse", "http_server", "path",
    "yaml", "csv", "toml", "regex", "uuid", "crypto", "compress",
    "fmt", "stats", "text", "encode", "reflect", "collections", "sort",
    "cert", "concurrent", "itertools", "template", "flag",
];

/// Get the list of operation names in a standard library module.
pub fn std_module_ops(module: &str) -> Vec<&'static str> {
    match module {
        "math" => vec![
            "add",
            "subtract",
            "multiply",
            "divide",
            "modulo",
            "power",
            "sqrt",
            "abs",
            "negate",
            "min",
            "max",
            "round",
            "floor",
            "ceil",
            "sin",
            "cos",
            "tan",
            "asin",
            "acos",
            "atan",
            "atan2",
            "sinh",
            "cosh",
            "tanh",
            "log",
            "ln",
            "log2",
            "log10",
            "exp",
            "to_radians",
            "to_degrees",
            "clamp",
            "lerp",
            "remap",
            "sign",
            "gcd",
            "lcm",
            "is_nan",
            "is_infinite",
            "is_finite",
            "approx_eq",
            "math_sum",
            "math_product",
            "math_average",
            "math_min_of",
            "math_max_of",
            "math_count",
            "factorial",
            "fibonacci",
            "is_prime",
            "ncr",
            "npr",
            "combinations",
            "permutations",
        ],
        "cmp" => vec![
            "equal",
            "not_equal",
            "greater",
            "less",
            "greater_eq",
            "less_eq",
        ],
        "logic" => vec!["and", "or", "not", "xor"],
        "bits" => vec![
            "bit_and",
            "bit_or",
            "bit_xor",
            "bit_not",
            "bit_shift_left",
            "bit_shift_right",
        ],
        "str" => vec![
            "concat",
            "split",
            "substring",
            "length",
            "replace",
            "to_upper",
            "to_lower",
            "trim",
            "trim_start",
            "trim_end",
            "contains",
            "starts_with",
            "ends_with",
            "char_at",
            "index_of",
            "pad_start",
            "pad_end",
            "string_repeat",
            "string_reverse",
            "string_lines",
            "string_words",
            "string_count",
            "string_chars",
            "string_join",
            "string_template",
            "string_format",
            "regex_match",
            "regex_replace",
            "regex_extract",
            "encode",
        ],
        "convert" => vec![
            "to_string",
            "to_int64",
            "to_float64",
            "to_bool",
            "to_bytes",
            "from_bytes",
            "parse_json",
            "to_json",
            "parse_int",
            "parse_float",
            "typeof",
            "default",
            "is_null",
            "is_string",
            "is_number",
            "is_array",
            "is_map",
            "is_bool",
            "is_bytes",
        ],
        "array" => vec![
            "array_get",
            "array_set",
            "array_push",
            "array_pop",
            "array_shift",
            "array_length",
            "array_slice",
            "array_concat",
            "array_contains",
            "array_sort",
            "array_reverse",
            "array_flatten",
            "array_filter_nulls",
            "array_join",
            "array_unique",
            "array_insert",
            "array_remove",
            "array_from_map",
            "reduce",
            "range",
        ],
        "map" => vec![
            "map_get",
            "map_set",
            "map_delete",
            "map_has",
            "map_keys",
            "map_values",
            "map_entries",
            "map_merge",
            "map_size",
            "map_from_entries",
            "map_update",
        ],
        "bytes" => vec![
            "bytes_length",
            "bytes_slice",
            "bytes_concat",
            "bytes_contains",
            "base64_encode",
            "base64_decode",
        ],
        "json" => vec![
            "json_get",
            "json_set",
            "json_delete",
            "json_flatten",
            "json_merge",
            "json_type",
            "json_validate",
            "json_pretty_print",
            "json_compact",
            "json_query",
        ],
        "time" => vec![
            "now_timestamp",
            "format_timestamp",
            "parse_timestamp",
            "timestamp_add",
            "timestamp_diff",
            "sleep",
            "duration",
            "elapsed",
            "time_sleep",
            "add_duration",
            "sub_duration",
            "time_diff",
            "start_of",
            "end_of",
            "date_now",
            "date_parse",
            "date_format",
            "date_add",
            "date_diff",
            "duration_ms",
            "duration_secs",
            "duration_mins",
            "duration_hours",
        ],
        "hash" => vec![
            "hash_sha256",
            "hash_blake3",
            "hash_md5",
            "url_encode",
            "url_decode",
            "hex_encode",
            "hex_decode",
            "hash_sha512",
            "hmac_sha256",
            "hash_crc32",
            "constant_time_eq",
        ],
        "io" => vec!["debug_log", "assert", "error"],
        "control" => vec!["if_else", "switch", "coalesce", "try_catch", "error"],
        "rand" => vec![
            "random_int",
            "random_float",
            "random_bool",
            "random_bytes",
            "random_range",
            "random_choice",
            "random_shuffle",
            "random_sample",
            "random_uuid",
            "random_string",
        ],
        "fs" => vec![
            "fs_read",
            "fs_write",
            "fs_append",
            "fs_exists",
            "fs_remove",
            "fs_list",
            "fs_mkdir",
            "fs_copy",
            "fs_move",
            "fs_size",
            "fs_is_file",
            "fs_is_dir",
            "fs_watch",
        ],
        "env" => vec![
            "env_get",
            "env_has",
            "env_keys",
            "os_name",
            "os_arch",
            "process_pid",
            "current_dir",
        ],
        "net" => vec![
            "http_get",
            "http_post",
            "http_put",
            "http_delete",
            "http_request",
            "http_head",
            "http_options",
            "http_patch",
            "url_parse",
            "url_join",
        ],
        "tcp" => vec![
            "tcp_connect",
            "tcp_write",
            "tcp_read",
            "tcp_close",
            "tcp_bind",
            "tcp_accept",
            "tcp_server_close",
        ],
        "udp" => vec!["udp_bind", "udp_send_to", "udp_recv_from", "udp_close"],
        "ws" => vec!["ws_connect", "ws_send", "ws_receive", "ws_close"],
        "sse" => vec!["sse_connect", "sse_read_event", "sse_close"],
        "http_server" => vec![
            "http_server_start",
            "http_server_receive",
            "http_server_respond",
            "http_server_stop",
        ],
        "cert" => vec![
            "cert_generate",
            "cert_parse",
            "cert_info",
            "cert_verify",
            "key_generate",
            "cert_self_signed",
        ],
        "path" => vec![
            "path_join",
            "path_basename",
            "path_dirname",
            "path_extension",
            "path_stem",
            "path_is_absolute",
            "path_normalize",
            "path_split",
            "path_with_extension",
            "path_parent",
        ],
        "yaml" => vec![
            "yaml_parse",
            "yaml_stringify",
            "yaml_validate",
            "yaml_to_json",
            "yaml_from_json",
            "yaml_merge",
        ],
        "csv" => vec![
            "csv_parse",
            "csv_stringify",
            "csv_headers",
            "csv_parse_rows",
        ],
        "toml" => vec!["toml_parse", "toml_stringify"],
        "regex" => vec![
            "regex_split",
            "regex_escape",
            "regex_test",
            "regex_captures",
            "regex_find_all",
        ],
        "uuid" => vec!["uuid_v4", "uuid_parse", "uuid_is_valid", "uuid_nil"],
        "crypto" => vec![
            "hash_sha512",
            "hmac_sha256",
            "hash_crc32",
            "constant_time_eq",
        ],
        "compress" => vec![
            "compress_zstd",
            "decompress_zstd",
            "compress_lz4",
            "decompress_lz4",
        ],
        "fmt" => vec![
            "fmt_number",
            "fmt_bytes",
            "fmt_duration",
            "fmt_hex",
            "fmt_binary",
            "fmt_percent",
        ],
        "stats" => vec![
            "stats_mean",
            "stats_median",
            "stats_mode",
            "stats_variance",
            "stats_std_dev",
            "stats_min_by",
            "stats_max_by",
            "stats_sum",
            "stats_percentile",
            "stats_quantile",
            "stats_covariance",
            "stats_correlation",
        ],
        "text" => vec![
            "text_wrap",
            "text_dedent",
            "text_indent",
            "text_pad_left",
            "text_pad_right",
            "text_truncate",
            "text_slug",
            "text_camel_case",
            "text_snake_case",
            "text_title_case",
        ],
        "encode" => vec![
            "html_escape",
            "html_unescape",
            "base32_encode",
            "base32_decode",
        ],
        "reflect" => vec![
            "reflect_type_of",
            "reflect_type_name",
            "reflect_is_type",
            "reflect_fields",
            "reflect_has_field",
            "reflect_callable",
            "reflect_arity",
            "reflect_inspect",
        ],
        "collections" => vec![
            "set_from",
            "set_union",
            "set_intersection",
            "set_difference",
            "set_symmetric_difference",
            "counter",
            "most_common",
            "ordered_map",
        ],
        "sort" => vec![
            "sort_asc",
            "sort_desc",
            "sort_by",
            "sort_by_key",
            "stable_sort",
            "is_sorted",
            "binary_search",
            "binary_search_by",
            "sort_reverse",
        ],
        "concurrent" => vec![
            "channel",
            "chan_send",
            "chan_recv",
            "chan_try_recv",
            "chan_close",
        ],
        "itertools" => vec![
            "iter_chain",
            "iter_cycle",
            "iter_repeat",
            "iter_product",
            "iter_pairwise",
        ],
        "template" => vec!["template_render"],
        "flag" => vec!["flag_parse", "flag_args"],
        _ => vec![],
    }
}

// =============================================================================
// Free-standing helper functions
// =============================================================================

/// Check if an error is a control flow signal (break/continue/return/cancel).
fn is_control_flow(err: &InterpError) -> bool {
    matches!(
        err,
        InterpError::BreakSignal(_)
            | InterpError::ContinueSignal
            | InterpError::LabeledBreak { .. }
            | InterpError::LabeledContinue { .. }
            | InterpError::ReturnSignal(_)
            | InterpError::Cancelled
    )
}

/// Convert a DataType to a human-readable display string (for interpolation/print).
fn datatype_to_display(val: &DataType) -> String {
    datatype_to_display_depth(val, 0)
}

/// Wrapper for writing a DataType display directly into a `fmt::Write` sink
/// (e.g., a `String`), avoiding an intermediate String allocation.
struct DataTypeDisplay<'a>(&'a DataType);

impl std::fmt::Display for DataTypeDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Delegate to the existing depth-limited display logic.
        // For top-level interpolation (depth 0) the cost of one allocation
        // inside datatype_to_display_depth is acceptable for nested types;
        // the key win is avoiding the extra allocation at the call site.
        f.write_str(&datatype_to_display_depth(self.0, 0))
    }
}

fn datatype_to_display_depth(val: &DataType, depth: usize) -> String {
    const MAX_DISPLAY_DEPTH: usize = 64;
    match val {
        DataType::Null => "null".to_string(),
        DataType::Bool(b) => b.to_string(),
        DataType::Int32(n) => n.to_string(),
        DataType::Int64(n) => n.to_string(),
        DataType::Uint32(n) => n.to_string(),
        DataType::Uint64(n) => n.to_string(),
        DataType::Float32(f) => f.to_string(),
        DataType::Float64(f) => f.to_string(),
        DataType::String(s) => s.clone(),
        DataType::Bytes(b) => format!("<bytes:{}>", b.len()),
        DataType::Array(arr) => {
            if depth >= MAX_DISPLAY_DEPTH {
                return "[...]".to_string();
            }
            const MAX_DISPLAY_ELEMENTS: usize = 1000;
            let truncated = arr.len() > MAX_DISPLAY_ELEMENTS;
            let items: Vec<String> = arr.iter().take(MAX_DISPLAY_ELEMENTS)
                .map(|v| datatype_to_display_depth(v, depth + 1)).collect();
            if truncated {
                format!("[{}, ...({} more)]", items.join(", "), arr.len() - MAX_DISPLAY_ELEMENTS)
            } else {
                format!("[{}]", items.join(", "))
            }
        }
        DataType::Set(items) => {
            if depth >= MAX_DISPLAY_DEPTH {
                return "Set(...)".to_string();
            }
            const MAX_DISPLAY_ELEMENTS: usize = 1000;
            let truncated = items.len() > MAX_DISPLAY_ELEMENTS;
            let elems: Vec<String> = items.iter().take(MAX_DISPLAY_ELEMENTS)
                .map(|v| datatype_to_display_depth(v, depth + 1)).collect();
            if truncated {
                format!("Set({{{}, ...({} more)}})", elems.join(", "), items.len() - MAX_DISPLAY_ELEMENTS)
            } else {
                format!("Set({{{}}})", elems.join(", "))
            }
        }
        DataType::Map(map) => {
            if depth >= MAX_DISPLAY_DEPTH {
                return "{...}".to_string();
            }
            const MAX_DISPLAY_ENTRIES: usize = 1000;
            let truncated = map.len() > MAX_DISPLAY_ENTRIES;
            let entries: Vec<String> = map
                .iter()
                .take(MAX_DISPLAY_ENTRIES)
                .map(|(k, v)| format!("{}: {}", k, datatype_to_display_depth(v, depth + 1)))
                .collect();
            if truncated {
                format!("{{{}, ...({} more)}}", entries.join(", "), map.len() - MAX_DISPLAY_ENTRIES)
            } else {
                format!("{{{}}}", entries.join(", "))
            }
        }
        DataType::Tuple(items) => {
            if depth >= MAX_DISPLAY_DEPTH {
                return "(...)".to_string();
            }
            let elems: Vec<String> = items.iter()
                .map(|v| datatype_to_display_depth(v, depth + 1)).collect();
            if items.len() == 1 {
                format!("({},)", elems[0])
            } else {
                format!("({})", elems.join(", "))
            }
        }
        DataType::Future(_) => "<future>".to_string(),
    }
}

/// Convert a DataType to i128 for wide numeric comparisons (handles Uint64 > i64::MAX).
/// Compare two DataType values for equality (used for Set dedup).
fn datatype_eq(a: &DataType, b: &DataType) -> bool {
    match (a, b) {
        (DataType::Null, DataType::Null) => true,
        (DataType::Bool(a), DataType::Bool(b)) => a == b,
        (DataType::Int64(a), DataType::Int64(b)) => a == b,
        (DataType::Int32(a), DataType::Int32(b)) => a == b,
        (DataType::Uint32(a), DataType::Uint32(b)) => a == b,
        (DataType::Uint64(a), DataType::Uint64(b)) => a == b,
        (DataType::Float64(a), DataType::Float64(b)) => a.to_bits() == b.to_bits(),
        (DataType::Float32(a), DataType::Float32(b)) => a.to_bits() == b.to_bits(),
        (DataType::String(a), DataType::String(b)) => a == b,
        (DataType::Bytes(a), DataType::Bytes(b)) => a == b,
        (DataType::Array(a), DataType::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| datatype_eq(x, y))
        }
        (DataType::Set(a), DataType::Set(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| datatype_eq(x, y))
        }
        (DataType::Tuple(a), DataType::Tuple(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| datatype_eq(x, y))
        }
        _ => false,
    }
}

/// Resolve a method call to an OperationType based on receiver type and method name.
fn resolve_method(obj: &DataType, method: &str) -> Option<OperationType> {
    let result = match obj {
        DataType::Array(_) => match method {
            "push" => Some(OperationType::ArrayPush),
            "pop" => Some(OperationType::ArrayPop),
            "shift" => Some(OperationType::ArrayShift),
            "len" | "length" => Some(OperationType::ArrayLength),
            "get" => Some(OperationType::ArrayGet),
            "set" => Some(OperationType::ArraySet),
            "slice" => Some(OperationType::ArraySlice),
            "contains" => Some(OperationType::ArrayContains),
            "sort" => Some(OperationType::ArraySort),
            "reverse" => Some(OperationType::ArrayReverse),
            "flatten" => Some(OperationType::ArrayFlatten),
            "join" => Some(OperationType::ArrayJoin),
            "concat" => Some(OperationType::ArrayConcat),
            "unique" => Some(OperationType::ArrayUnique),
            "window" => Some(OperationType::ArrayWindow),
            "insert" => Some(OperationType::ArrayInsert),
            "remove" => Some(OperationType::ArrayRemove),
            "filter_nulls" => Some(OperationType::ArrayFilterNulls),
            _ => None,
        },
        DataType::String(_) => match method {
            "len" | "length" => Some(OperationType::Length),
            "split" => Some(OperationType::Split),
            "contains" => Some(OperationType::Contains),
            "replace" => Some(OperationType::Replace),
            "trim" => Some(OperationType::Trim),
            "trim_start" => Some(OperationType::TrimStart),
            "trim_end" => Some(OperationType::TrimEnd),
            "to_upper" | "to_uppercase" => Some(OperationType::ToUpper),
            "to_lower" | "to_lowercase" => Some(OperationType::ToLower),
            "starts_with" => Some(OperationType::StartsWith),
            "ends_with" => Some(OperationType::EndsWith),
            "substring" | "slice" => Some(OperationType::Substring),
            "chars" => Some(OperationType::StringChars),
            "repeat" => Some(OperationType::StringRepeat),
            "lines" => Some(OperationType::StringLines),
            "words" => Some(OperationType::StringWords),
            "reverse" => Some(OperationType::StringReverse),
            "index_of" => Some(OperationType::IndexOf),
            "count" => Some(OperationType::StringCount),
            "pad_start" => Some(OperationType::PadStart),
            "pad_end" => Some(OperationType::PadEnd),
            "char_at" => Some(OperationType::CharAt),
            _ => None,
        },
        DataType::Map(_) => match method {
            "get" => Some(OperationType::MapGet),
            "set" => Some(OperationType::MapSet),
            "delete" => Some(OperationType::MapDelete),
            "has" => Some(OperationType::MapHas),
            "keys" => Some(OperationType::MapKeys),
            "values" => Some(OperationType::MapValues),
            "entries" => Some(OperationType::MapEntries),
            "merge" => Some(OperationType::MapMerge),
            "len" | "length" | "size" => Some(OperationType::MapSize),
            _ => None,
        },
        DataType::Bytes(_) => match method {
            "len" | "length" => Some(OperationType::BytesLength),
            "slice" => Some(OperationType::BytesSlice),
            "concat" => Some(OperationType::BytesConcat),
            "contains" => Some(OperationType::BytesContains),
            "base64_encode" => Some(OperationType::Base64Encode),
            "base64_decode" => Some(OperationType::Base64Decode),
            _ => None,
        },
        DataType::Set(_) => match method {
            "len" | "length" | "size" => Some(OperationType::ArrayLength),
            _ => None,
        },
        _ => None,
    };
    // Fall back to generic methods that work on any type
    result.or(match method {
        "to_string" => Some(OperationType::ToString),
        "to_int64" => Some(OperationType::ToInt64),
        "to_float64" => Some(OperationType::ToFloat64),
        "to_bool" => Some(OperationType::ToBool),
        "to_json" => Some(OperationType::ToJson),
        "typeof" => Some(OperationType::Typeof),
        _ => None,
    })
}

/// Get available method names for a DataType (for error suggestions).
fn available_methods_for_type(obj: &DataType) -> Vec<&'static str> {
    let mut methods: Vec<&'static str> = Vec::new();
    match obj {
        DataType::Array(_) => {
            // Direct methods
            methods.extend_from_slice(&["first", "last", "is_empty", "sum", "product", "min", "max", "join",
                "flatten", "rotate_left", "rotate_right", "interleave", "dedup", "transpose", "combinations"]);
            // HOF methods
            methods.extend_from_slice(&["map", "filter", "reduce", "find", "find_index", "any", "all",
                "flat_map", "each", "sort_by", "group_by", "min_by", "max_by",
                "take_while", "skip_while", "partition", "scan", "enumerate", "zip", "chunk"]);
            // Evaluator methods
            methods.extend_from_slice(&["push", "pop", "shift", "len", "length", "get", "set",
                "slice", "contains", "sort", "reverse", "concat",
                "unique", "insert", "remove", "filter_nulls", "window"]);
        }
        DataType::String(_) => {
            // Direct methods
            methods.extend_from_slice(&["is_empty", "is_numeric", "is_alphabetic", "to_int", "to_float",
                "len", "length", "trim", "trim_start", "trim_end", "to_upper", "to_uppercase",
                "to_lower", "to_lowercase", "reverse", "chars", "lines", "pad_start", "pad_end",
                "char_at", "repeat", "substring", "slice", "index_of",
                "capitalize", "uncapitalize", "pad_center", "truncate", "count_words",
                "strip_prefix", "strip_suffix", "byte_length", "byte_len"]);
            // Evaluator methods
            methods.extend_from_slice(&["split", "contains", "replace", "starts_with", "ends_with",
                "words", "count"]);
        }
        DataType::Int64(_) => {
            methods.extend_from_slice(&["abs", "sign", "pow", "min", "max", "clamp"]);
        }
        DataType::Int32(_) => {
            methods.extend_from_slice(&["abs", "sign", "to_int32", "pow", "min", "max", "clamp"]);
        }
        DataType::Uint32(_) => {
            methods.extend_from_slice(&["abs", "sign", "to_uint32", "pow", "min", "max", "clamp"]);
        }
        DataType::Uint64(_) => {
            methods.extend_from_slice(&["abs", "sign", "to_uint64", "pow", "min", "max", "clamp"]);
        }
        DataType::Float64(_) => {
            methods.extend_from_slice(&["abs", "round", "floor", "ceil", "sqrt", "is_nan", "is_infinite", "is_finite",
                "sign", "to_float32", "pow", "min", "max", "clamp",
                "ln", "log2", "log10", "sin", "cos", "tan",
                "asin", "acos", "atan", "sinh", "cosh", "tanh", "exp"]);
        }
        DataType::Float32(_) => {
            methods.extend_from_slice(&["abs", "round", "floor", "ceil", "sqrt", "is_nan", "is_infinite", "is_finite",
                "sign", "to_float32", "pow", "min", "max", "clamp",
                "ln", "log2", "log10", "sin", "cos", "tan",
                "asin", "acos", "atan", "sinh", "cosh", "tanh", "exp"]);
        }
        DataType::Map(_) => {
            methods.extend_from_slice(&["get", "set", "delete", "has", "keys", "values", "entries",
                "merge", "len", "length", "size",
                "invert", "defaults", "pick", "omit", "deep_merge", "flatten_keys"]);
            // HOF methods
            methods.extend_from_slice(&["filter_entries", "map_values", "map_keys", "map_entries"]);
        }
        DataType::Bytes(_) => {
            methods.extend_from_slice(&["len", "length", "slice", "concat", "contains",
                "base64_encode", "base64_decode"]);
        }
        DataType::Set(_) => {
            methods.extend_from_slice(&["add", "remove", "contains", "len", "length", "size",
                "union", "intersection", "difference", "is_subset", "is_superset",
                "to_array", "clear", "is_empty"]);
        }
        _ => {}
    }
    // Generic methods available on all types
    methods.extend_from_slice(&["to_string", "to_int64", "to_float64", "to_bool", "to_json", "typeof"]);
    methods
}

/// Match a value against a pattern, returning variable bindings on success.
fn match_pattern(value: &DataType, pattern: &Pattern) -> Option<Vec<(String, DataType)>> {
    match_pattern_depth(value, pattern, 0)
}

const MAX_PATTERN_DEPTH: usize = 64;

fn match_pattern_depth(value: &DataType, pattern: &Pattern, depth: usize) -> Option<Vec<(String, DataType)>> {
    if depth > MAX_PATTERN_DEPTH {
        return None;
    }
    match pattern {
        Pattern::Wildcard => Some(vec![]),
        Pattern::Variable(name) => Some(vec![(name.clone(), value.clone())]),
        Pattern::Literal(lit) => {
            let matches = match (lit, value) {
                (Literal::Int64(a), DataType::Int64(b)) => a == b,
                (Literal::Int64(a), DataType::Float64(b)) => (*a as f64) == *b,
                (Literal::Int64(a), DataType::Int32(b)) => *a == (*b as i64),
                (Literal::Int64(a), DataType::Uint32(b)) => *a == (*b as i64),
                (Literal::Int64(a), DataType::Uint64(b)) => *a >= 0 && (*a as u64) == *b,
                (Literal::Int64(a), DataType::Float32(b)) => (*a as f64) == (*b as f64),
                // Pattern matching uses structural equality: NaN matches NaN.
                // This differs from `==` (IEEE 754, NaN != NaN) by design —
                // match arms should be exhaustive over all values including NaN.
                (Literal::Float64(a), DataType::Float64(b)) => a == b || (a.is_nan() && b.is_nan()),
                (Literal::Float64(a), DataType::Int64(b)) => *a == (*b as f64),
                (Literal::Float64(a), DataType::Float32(b)) => *a == (*b as f64),
                (Literal::Float64(a), DataType::Int32(b)) => *a == (*b as f64),
                (Literal::Float64(a), DataType::Uint32(b)) => *a == (*b as f64),
                (Literal::Float64(a), DataType::Uint64(b)) => *a == (*b as f64),
                (Literal::String(a), DataType::String(b)) => a == b,
                (Literal::Bool(a), DataType::Bool(b)) => a == b,
                (Literal::Null, DataType::Null) => true,
                _ => false,
            };
            if matches {
                Some(vec![])
            } else {
                None
            }
        }
        Pattern::Array(patterns) => {
            let arr = match value {
                DataType::Array(a) => a,
                _ => return None,
            };
            // Find rest pattern position
            let rest_pos = patterns.iter().position(|p| matches!(p, Pattern::Rest(_)));
            if let Some(rp) = rest_pos {
                let before = rp;
                let after = patterns.len() - rp - 1;
                if arr.len() < before + after {
                    return None;
                }
                let mut bindings = vec![];
                // Match elements before rest
                for (i, pat) in patterns[..rp].iter().enumerate() {
                    let sub = match_pattern_depth(&arr[i], pat, depth + 1)?;
                    bindings.extend(sub);
                }
                // Match rest
                if let Pattern::Rest(Some(name)) = &patterns[rp] {
                    let rest_end = arr.len() - after;
                    let rest_val = DataType::Array(arr[rp..rest_end].to_vec());
                    bindings.push((name.clone(), rest_val));
                }
                // Match elements after rest
                for (i, pat) in patterns[rp + 1..].iter().enumerate() {
                    let idx = arr.len() - after + i;
                    let sub = match_pattern_depth(&arr[idx], pat, depth + 1)?;
                    bindings.extend(sub);
                }
                Some(bindings)
            } else {
                if arr.len() != patterns.len() {
                    return None;
                }
                arr.iter().zip(patterns.iter())
                    .try_fold(vec![], |mut bindings, (elem, pat)| {
                        bindings.extend(match_pattern_depth(elem, pat, depth + 1)?);
                        Some(bindings)
                    })
            }
        }
        Pattern::Map(entries) => {
            let map = match value {
                DataType::Map(m) => m,
                _ => return None,
            };
            entries.iter().try_fold(vec![], |mut bindings, (key, pat)| {
                let val = map.get(key)?;
                bindings.extend(match_pattern_depth(val, pat, depth + 1)?);
                Some(bindings)
            })
        }
        Pattern::Or(patterns) => {
            for pat in patterns {
                if let Some(bindings) = match_pattern_depth(value, pat, depth + 1) {
                    return Some(bindings);
                }
            }
            None
        }
        Pattern::Rest(_) => {
            // Rest pattern should only appear inside array patterns
            None
        }
        Pattern::EnumPattern { enum_name, variant, bindings } => {
            let map = match value {
                DataType::Map(m) => m,
                _ => return None,
            };
            // Check __enum and __variant fields
            let val_enum = map.get("__enum")?.to_string_lossy();
            let val_variant = map.get("__variant")?.to_string_lossy();
            if val_enum != *enum_name || val_variant != *variant {
                return None;
            }
            let data = match map.get("__data") {
                Some(DataType::Array(arr)) => arr.clone(),
                _ => Vec::new(),
            };
            if bindings.len() > data.len() {
                return None;
            }
            let mut all_bindings = Vec::new();
            for (i, pat) in bindings.iter().enumerate() {
                if let Some(inner) = data.get(i) {
                    let sub = match_pattern_depth(inner, pat, depth + 1)?;
                    all_bindings.extend(sub);
                } else {
                    return None;
                }
            }
            Some(all_bindings)
        }
        Pattern::TypePattern { name, type_name } => {
            let actual_type = value.type_name();
            if actual_type == type_name {
                Some(vec![(name.clone(), value.clone())])
            } else {
                None
            }
        }
        Pattern::RangePattern { start, end, inclusive } => {
            // Extract literal values from expressions for range comparison.
            // Support both Int64 and Float64 range patterns.
            let start_f = match &start.kind {
                ExpressionKind::Literal(Literal::Int64(v)) => *v as f64,
                ExpressionKind::Literal(Literal::Float64(v)) => *v,
                _ => return None,
            };
            let end_f = match &end.kind {
                ExpressionKind::Literal(Literal::Int64(v)) => *v as f64,
                ExpressionKind::Literal(Literal::Float64(v)) => *v,
                _ => return None,
            };
            let val_f = match value {
                DataType::Int64(v) => *v as f64,
                DataType::Float64(v) => *v,
                DataType::Float32(v) => *v as f64,
                DataType::Int32(v) => *v as f64,
                DataType::Uint32(v) => *v as f64,
                DataType::Uint64(v) => *v as f64,
                _ => return None,
            };
            let in_range = if *inclusive {
                val_f >= start_f && val_f <= end_f
            } else {
                val_f >= start_f && val_f < end_f
            };
            if in_range { Some(vec![]) } else { None }
        }
    }
}

// literal_to_datatype removed — replaced by Interpreter::eval_literal()
// which properly evaluates non-literal expressions inside array/map literals.

// =============================================================================
// Error type
// =============================================================================

/// Interpreter error type.
#[derive(Debug)]
pub enum InterpError {
    UndefinedVariable {
        name: String,
        span: Span,
        suggestion: Option<String>,
    },
    ImmutableAssignment {
        name: String,
        span: Span,
    },
    UnknownOperation {
        name: String,
        span: Span,
        suggestion: Option<String>,
    },
    TypeError {
        expected: String,
        actual: String,
        context: String,
        span: Span,
    },
    EvalError {
        error: EvalError,
        span: Span,
    },
    MaxIterations {
        limit: usize,
        span: Span,
    },
    Cancelled,
    InvalidPlaceholder {
        span: Span,
    },
    InvalidPipeStage {
        span: Span,
    },
    UndefinedFunction {
        name: String,
        span: Span,
        suggestion: Option<String>,
    },
    MaxCallDepth {
        limit: usize,
        span: Span,
    },
    ArityMismatch {
        name: String,
        expected: String,
        actual: usize,
        span: Span,
    },
    /// Control flow signal: `break` (caught by loops, not a real error)
    BreakSignal(DataType),
    /// Control flow signal: `continue` (caught by loops)
    ContinueSignal,
    /// Control flow signal: `break 'label` (caught by the matching labeled loop)
    LabeledBreak { label: String, value: DataType },
    /// Control flow signal: `continue 'label` (caught by the matching labeled loop)
    LabeledContinue { label: String },
    /// Control flow signal: `return expr` (caught by function calls)
    ReturnSignal(DataType),
    /// `break` used outside a loop
    BreakOutsideLoop {
        span: Span,
    },
    /// `continue` used outside a loop
    ContinueOutsideLoop {
        span: Span,
    },
    /// `return` used outside a function
    ReturnOutsideFunction {
        span: Span,
    },
    /// Resource limit exceeded (string too large, too many elements, etc.)
    ResourceLimit {
        limit: String,
        actual: String,
        context: String,
        span: Span,
    },
    /// Feature not yet implemented
    NotImplemented {
        message: String,
        span: Span,
    },
    /// User-thrown error via `throw expr;`
    ThrownError {
        value: DataType,
        span: Span,
    },
    /// Assertion failure via `assert()`, `assert_eq()`, or `assert_ne()`
    AssertionFailed {
        message: String,
        span: Span,
    },
}

impl std::fmt::Display for InterpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InterpError::UndefinedVariable {
                name,
                span,
                suggestion,
            } => {
                write!(f, "{} [E200]: Undefined variable '{}'", span, name)?;
                if let Some(s) = suggestion {
                    write!(f, " ({})", s)?;
                }
                Ok(())
            }
            InterpError::ImmutableAssignment { name, span } => {
                write!(
                    f,
                    "{} [E404]: Cannot assign to immutable variable '{}'",
                    span, name
                )
            }
            InterpError::UnknownOperation {
                name,
                span,
                suggestion,
            } => {
                write!(f, "{} [E202]: Unknown operation '{}'", span, name)?;
                if let Some(s) = suggestion {
                    write!(f, " ({})", s)?;
                }
                Ok(())
            }
            InterpError::TypeError {
                expected,
                actual,
                context,
                span,
            } => {
                write!(
                    f,
                    "{} [E100]: Type error in {}: expected {}, got {}",
                    span, context, expected, actual
                )
            }
            InterpError::EvalError { error, span } => {
                write!(f, "{} [{}]: {}", span, error.error_code(), error)
            }
            InterpError::MaxIterations { limit, span } => {
                write!(
                    f,
                    "{} [E400]: Loop exceeded maximum iterations ({})",
                    span, limit
                )
            }
            InterpError::Cancelled => write!(f, "[E407]: Execution cancelled"),
            InterpError::InvalidPlaceholder { span } => {
                write!(
                    f,
                    "{} [E303]: '_' can only be used inside pipe expressions",
                    span
                )
            }
            InterpError::InvalidPipeStage { span } => {
                write!(f, "{} [E304]: Pipe stage must be a function call", span)
            }
            InterpError::UndefinedFunction {
                name,
                span,
                suggestion,
            } => {
                write!(f, "{} [E201]: Undefined function '{}'", span, name)?;
                if let Some(s) = suggestion {
                    write!(f, " ({})", s)?;
                }
                Ok(())
            }
            InterpError::MaxCallDepth { limit, span } => {
                write!(
                    f,
                    "{} [E401]: Maximum call depth exceeded ({})",
                    span, limit
                )
            }
            InterpError::ArityMismatch {
                name,
                expected,
                actual,
                span,
            } => {
                write!(
                    f,
                    "{} [E405]: Function '{}' expects {} argument(s), got {}",
                    span, name, expected, actual
                )
            }
            InterpError::BreakSignal(_) => write!(f, "break"),
            InterpError::ContinueSignal => write!(f, "continue"),
            InterpError::LabeledBreak { label, .. } => write!(f, "break '{}", label),
            InterpError::LabeledContinue { label } => write!(f, "continue '{}", label),
            InterpError::ReturnSignal(_) => write!(f, "return"),
            InterpError::BreakOutsideLoop { span } => {
                write!(f, "{} [E300]: 'break' used outside of a loop", span)
            }
            InterpError::ContinueOutsideLoop { span } => {
                write!(f, "{} [E301]: 'continue' used outside of a loop", span)
            }
            InterpError::ReturnOutsideFunction { span } => {
                write!(f, "{} [E302]: 'return' used outside of a function", span)
            }
            InterpError::ResourceLimit { limit, actual, context, span } => {
                write!(f, "{} [E409]: Resource limit in {}: max {}, got {}", span, context, limit, actual)
            }
            InterpError::NotImplemented { message, span } => {
                write!(f, "{} [E408]: {}", span, message)
            }
            InterpError::ThrownError { value, span } => {
                write!(f, "{} [E403]: Uncaught error: {}", span, datatype_to_display(value))
            }
            InterpError::AssertionFailed { message, span } => {
                write!(f, "{} [E402]: {}", span, message)
            }
        }
    }
}

impl InterpError {
    /// Returns the stable MAGI error code for this error variant (#136).
    ///
    /// Uses enum-based dispatch. Control flow signals return `None`.
    pub fn error_code(&self) -> Option<&'static str> {
        match self {
            InterpError::UndefinedVariable { .. } => Some("E200"),
            InterpError::UndefinedFunction { .. } => Some("E201"),
            InterpError::UnknownOperation { .. } => Some("E202"),
            InterpError::TypeError { .. } => Some("E100"),
            InterpError::EvalError { error, .. } => Some(error.error_code()),
            InterpError::ImmutableAssignment { .. } => Some("E404"),
            InterpError::ArityMismatch { .. } => Some("E405"),
            InterpError::InvalidPlaceholder { .. } => Some("E303"),
            InterpError::InvalidPipeStage { .. } => Some("E304"),
            InterpError::BreakOutsideLoop { .. } => Some("E300"),
            InterpError::ContinueOutsideLoop { .. } => Some("E301"),
            InterpError::ReturnOutsideFunction { .. } => Some("E302"),
            InterpError::MaxIterations { .. } => Some("E400"),
            InterpError::MaxCallDepth { .. } => Some("E401"),
            InterpError::AssertionFailed { .. } => Some("E402"),
            InterpError::ThrownError { .. } => Some("E403"),
            InterpError::Cancelled => Some("E407"),
            InterpError::NotImplemented { .. } => Some("E408"),
            InterpError::ResourceLimit { .. } => Some("E409"),
            InterpError::BreakSignal(_)
            | InterpError::ContinueSignal
            | InterpError::LabeledBreak { .. }
            | InterpError::LabeledContinue { .. }
            | InterpError::ReturnSignal(_) => None,
        }
    }

    /// Returns the source span associated with this error, if any.
    pub fn span(&self) -> Option<Span> {
        match self {
            InterpError::UndefinedVariable { span, .. }
            | InterpError::ImmutableAssignment { span, .. }
            | InterpError::UnknownOperation { span, .. }
            | InterpError::TypeError { span, .. }
            | InterpError::EvalError { span, .. }
            | InterpError::MaxIterations { span, .. }
            | InterpError::InvalidPlaceholder { span, .. }
            | InterpError::InvalidPipeStage { span, .. }
            | InterpError::UndefinedFunction { span, .. }
            | InterpError::MaxCallDepth { span, .. }
            | InterpError::ArityMismatch { span, .. }
            | InterpError::BreakOutsideLoop { span, .. }
            | InterpError::ContinueOutsideLoop { span, .. }
            | InterpError::ReturnOutsideFunction { span, .. }
            | InterpError::ResourceLimit { span, .. }
            | InterpError::NotImplemented { span, .. }
            | InterpError::ThrownError { span, .. }
            | InterpError::AssertionFailed { span, .. } => Some(*span),
            InterpError::Cancelled
            | InterpError::BreakSignal(_)
            | InterpError::ContinueSignal
            | InterpError::LabeledBreak { .. }
            | InterpError::LabeledContinue { .. }
            | InterpError::ReturnSignal(_) => None,
        }
    }
}

impl std::error::Error for InterpError {}

/// Parse MAGI V2 source code into a `ResolvedPackage`.
///
/// Extracts all top-level function definitions from the source and
/// packages them for use with `interpreter.with_package()`.
pub fn resolve_package_from_source(id: &str, source: &str) -> Result<ResolvedPackage, String> {
    let program = super::parser::parse_v2(source)
        .map_err(|e| format!("Failed to parse package '{}': {}", id, e))?;

    let mut functions = HashMap::new();
    let mut use_statements = Vec::new();
    let mut enum_defs = Vec::new();
    let mut struct_defs = Vec::new();

    for stmt in &program.statements {
        match &stmt.kind {
            StatementKind::FunctionDef(def) | StatementKind::AsyncFunctionDef(def) => {
                functions.insert(def.name.clone(), def.clone());
            }
            StatementKind::Use { .. } => {
                use_statements.push(stmt.clone());
            }
            StatementKind::EnumDef { .. } | StatementKind::StructDef { .. }
                        | StatementKind::ImplBlock { .. }
                        | StatementKind::TraitDef { .. }
                        | StatementKind::ImplTrait { .. } => {
                if let StatementKind::EnumDef { name, variants, .. } = &stmt.kind {
                    enum_defs.push((name.clone(), variants.clone()));
                } else if let StatementKind::StructDef { name, fields, .. } = &stmt.kind {
                    struct_defs.push((name.clone(), fields.clone()));
                }
            }
            _ => {}
        }
    }

    Ok(ResolvedPackage {
        id: id.to_string(),
        functions,
        use_statements,
        enum_defs,
        struct_defs,
    })
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;
    use crate::syntax::ast::{Expression, ExpressionKind, Span};

    struct NoOpEvaluator;
    impl crate::eval::OperationEvaluator for NoOpEvaluator {
        fn eval_operation(
            &self,
            _op: crate::types::OperationType,
            _inputs: &std::collections::HashMap<String, DataType>,
            _config: &std::collections::HashMap<String, DataType>,
        ) -> Result<DataType, crate::eval::EvalError> {
            Ok(DataType::Null)
        }
    }

    fn make_interp() -> Interpreter<'static> {
        let evaluator: &'static dyn crate::eval::OperationEvaluator = Box::leak(Box::new(NoOpEvaluator));
        Interpreter::new(evaluator)
    }

    fn make_float_literal(val: f64) -> Expression {
        use crate::syntax::ast::Literal;
        Expression {
            kind: ExpressionKind::Literal(Literal::Float64(val)),
            span: Span::new(1, 1, 1, 1),
        }
    }

    #[test]
    fn test_float32_pow_returns_float32() {
        let mut interp = make_interp();
        let obj = DataType::Float32(2.0);
        let args = vec![make_float_literal(3.0)];
        let span = Span::new(1, 1, 1, 1);
        let result = interp.try_eval_direct_method(&obj, "pow", &args, span).unwrap();
        match result {
            Some(DataType::Float32(v)) => assert!((v - 8.0).abs() < 0.001, "Expected ~8.0, got {}", v),
            other => panic!("Expected Float32, got {:?}", other),
        }
    }

    #[test]
    fn test_uint32_sign_returns_uint32() {
        let mut interp = make_interp();
        let obj = DataType::Uint32(42);
        let args = vec![];
        let span = Span::new(1, 1, 1, 1);
        let result = interp.try_eval_direct_method(&obj, "sign", &args, span).unwrap();
        assert_eq!(result, Some(DataType::Uint32(1)));
    }

    #[test]
    fn test_uint32_sign_zero_returns_uint32() {
        let mut interp = make_interp();
        let obj = DataType::Uint32(0);
        let args = vec![];
        let span = Span::new(1, 1, 1, 1);
        let result = interp.try_eval_direct_method(&obj, "sign", &args, span).unwrap();
        assert_eq!(result, Some(DataType::Uint32(0)));
    }

    #[test]
    fn test_uint64_sign_returns_uint64() {
        let mut interp = make_interp();
        let obj = DataType::Uint64(99);
        let args = vec![];
        let span = Span::new(1, 1, 1, 1);
        let result = interp.try_eval_direct_method(&obj, "sign", &args, span).unwrap();
        assert_eq!(result, Some(DataType::Uint64(1)));
    }

    #[test]
    fn test_uint64_sign_zero_returns_uint64() {
        let mut interp = make_interp();
        let obj = DataType::Uint64(0);
        let args = vec![];
        let span = Span::new(1, 1, 1, 1);
        let result = interp.try_eval_direct_method(&obj, "sign", &args, span).unwrap();
        assert_eq!(result, Some(DataType::Uint64(0)));
    }

    #[test]
    fn test_float32_to_float32_identity() {
        let mut interp = make_interp();
        let obj = DataType::Float32(3.14);
        let args = vec![];
        let span = Span::new(1, 1, 1, 1);
        let result = interp.try_eval_direct_method(&obj, "to_float32", &args, span).unwrap();
        assert_eq!(result, Some(DataType::Float32(3.14)));
    }

    #[test]
    fn test_float64_to_float32_narrowing() {
        let mut interp = make_interp();
        let obj = DataType::Float64(3.14);
        let args = vec![];
        let span = Span::new(1, 1, 1, 1);
        let result = interp.try_eval_direct_method(&obj, "to_float32", &args, span).unwrap();
        assert_eq!(result, Some(DataType::Float32(3.14_f64 as f32)));
    }

    #[test]
    fn test_int32_to_int32_identity() {
        let mut interp = make_interp();
        let obj = DataType::Int32(42);
        let args = vec![];
        let span = Span::new(1, 1, 1, 1);
        let result = interp.try_eval_direct_method(&obj, "to_int32", &args, span).unwrap();
        assert_eq!(result, Some(DataType::Int32(42)));
    }

    #[test]
    fn test_uint32_to_uint32_identity() {
        let mut interp = make_interp();
        let obj = DataType::Uint32(99);
        let args = vec![];
        let span = Span::new(1, 1, 1, 1);
        let result = interp.try_eval_direct_method(&obj, "to_uint32", &args, span).unwrap();
        assert_eq!(result, Some(DataType::Uint32(99)));
    }

    #[test]
    fn test_uint64_to_uint64_identity() {
        let mut interp = make_interp();
        let obj = DataType::Uint64(12345);
        let args = vec![];
        let span = Span::new(1, 1, 1, 1);
        let result = interp.try_eval_direct_method(&obj, "to_uint64", &args, span).unwrap();
        assert_eq!(result, Some(DataType::Uint64(12345)));
    }

    #[test]
    fn test_literal_pattern_matches_int32() {
        use crate::syntax::ast::{Literal, Pattern};
        let value = DataType::Int32(42);
        let pattern = Pattern::Literal(Literal::Int64(42));
        let result = match_pattern_depth(&value, &pattern, 0);
        assert!(result.is_some(), "Int64(42) pattern should match Int32(42)");
    }

    #[test]
    fn test_literal_pattern_matches_uint32() {
        use crate::syntax::ast::{Literal, Pattern};
        let value = DataType::Uint32(100);
        let pattern = Pattern::Literal(Literal::Int64(100));
        let result = match_pattern_depth(&value, &pattern, 0);
        assert!(result.is_some(), "Int64(100) pattern should match Uint32(100)");
    }

    #[test]
    fn test_literal_pattern_matches_uint64() {
        use crate::syntax::ast::{Literal, Pattern};
        let value = DataType::Uint64(999);
        let pattern = Pattern::Literal(Literal::Int64(999));
        let result = match_pattern_depth(&value, &pattern, 0);
        assert!(result.is_some(), "Int64(999) pattern should match Uint64(999)");
    }

    #[test]
    fn test_literal_pattern_matches_float32() {
        use crate::syntax::ast::{Literal, Pattern};
        // Use 2.0 which is exactly representable in both f32 and f64
        let value = DataType::Float32(2.0);
        let pattern = Pattern::Literal(Literal::Float64(2.0));
        let result = match_pattern_depth(&value, &pattern, 0);
        assert!(result.is_some(), "Float64(2.0) pattern should match Float32(2.0)");
    }

    #[test]
    fn test_literal_pattern_no_match_wrong_value() {
        use crate::syntax::ast::{Literal, Pattern};
        let value = DataType::Uint32(42);
        let pattern = Pattern::Literal(Literal::Int64(43));
        let result = match_pattern_depth(&value, &pattern, 0);
        assert!(result.is_none(), "Int64(43) should NOT match Uint32(42)");
    }

    // =========================================================================
    // #14: NaN equality — IEEE 754 semantics via PartialEq
    // =========================================================================

    #[test]
    fn test_nan_equality_partial_eq_is_false() {
        // DataType derives PartialEq, which delegates to f64's PartialEq.
        // IEEE 754: NaN == NaN must be false.
        let a = DataType::Float64(f64::NAN);
        let b = DataType::Float64(f64::NAN);
        assert_ne!(a, b, "NaN == NaN should be false (IEEE 754)");
        assert!(a != b, "NaN != NaN should be true (IEEE 754)");
    }

    #[test]
    fn test_nan_pattern_match_is_structural() {
        // In contrast to ==, pattern matching treats NaN as a concrete value.
        use crate::syntax::ast::{Literal, Pattern};
        let value = DataType::Float64(f64::NAN);
        let pattern = Pattern::Literal(Literal::Float64(f64::NAN));
        let result = match_pattern_depth(&value, &pattern, 0);
        assert!(result.is_some(), "NaN should match NaN in pattern matching (structural equality)");
    }

    // =========================================================================
    // #48: Free list coalescing
    // =========================================================================

    #[test]
    fn test_free_list_coalescing_adjacent_entries() {
        let mut heap = Heap::new();
        // Allocate many small values contiguously, then free them via pop_scope.
        // This should produce adjacent free list entries that get coalesced.
        heap.push_scope();
        let mut addrs = Vec::new();
        for i in 0..100 {
            let addr = heap.alloc(DataType::Int64(i));
            addrs.push(addr);
        }
        // All 100 allocations are contiguous (bump allocator).
        // After pop_scope, the free list should be compacted.
        heap.pop_scope();
        // With 100 freed entries (> threshold of 64), compaction should run.
        // All entries are contiguous, so they should coalesce into 1 entry.
        assert_eq!(
            heap.free_list.len(), 1,
            "100 contiguous freed entries should coalesce into 1, got {}",
            heap.free_list.len()
        );
        // The single entry should span from the first address to the end.
        let (start, total_size) = heap.free_list[0];
        assert_eq!(start, addrs[0], "coalesced entry should start at first allocation");
        let expected_size: u64 = (0..100).map(|_| Heap::size_of(&DataType::Int64(0)).max(ALIGNMENT)).sum();
        assert_eq!(total_size, expected_size, "coalesced size should equal sum of individual sizes");
    }

    #[test]
    fn test_free_list_no_coalescing_below_threshold() {
        let mut heap = Heap::new();
        // Allocate fewer entries than the threshold — no compaction should occur.
        heap.push_scope();
        for i in 0..10 {
            heap.alloc(DataType::Int64(i));
        }
        heap.pop_scope();
        // Below threshold (64), free list entries are left as-is.
        assert_eq!(
            heap.free_list.len(), 10,
            "below threshold, free list should not be compacted"
        );
    }

    #[test]
    fn test_free_list_coalescing_non_adjacent() {
        let mut heap = Heap::new();
        // Allocate entries, free alternate ones to create non-adjacent gaps.
        let mut addrs = Vec::new();
        for i in 0..200 {
            let addr = heap.alloc(DataType::Int64(i));
            addrs.push(addr);
        }
        // Manually free every other entry to create non-adjacent free list entries.
        for (idx, addr) in addrs.iter().enumerate() {
            if idx % 2 == 0 {
                if let Some(meta) = heap.metadata.remove(addr) {
                    heap.values.remove(addr);
                    heap.free_list.push((*addr, meta.size));
                }
            }
        }
        // 100 non-adjacent entries — above threshold, compaction runs.
        assert!(heap.free_list.len() > FREE_LIST_COMPACT_THRESHOLD);
        heap.compact_free_list();
        // Non-adjacent entries cannot be coalesced, so count stays at 100.
        assert_eq!(
            heap.free_list.len(), 100,
            "non-adjacent entries should not coalesce"
        );
    }

    // =========================================================================
    // #132: Error recovery (keep-going mode)
    // =========================================================================

    #[test]
    fn test_keep_going_collects_errors() {
        let evaluator: &'static dyn crate::eval::OperationEvaluator =
            Box::leak(Box::new(NoOpEvaluator));
        let mut interp = Interpreter::new(evaluator).with_max_errors(10);
        // Program referencing undefined variables -- each statement should produce an error
        let src = "let a = x;\nlet b = y;\nlet c = z;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let result = interp.execute(&program);
        // Should return the first error
        assert!(result.is_err());
        // Additional errors should be in collected_errors
        assert!(
            interp.collected_errors.len() >= 1,
            "keep-going mode should collect additional errors, got {}",
            interp.collected_errors.len()
        );
    }

    #[test]
    fn test_default_mode_aborts_on_first_error() {
        let evaluator: &'static dyn crate::eval::OperationEvaluator =
            Box::leak(Box::new(NoOpEvaluator));
        let mut interp = Interpreter::new(evaluator);
        let src = "let a = x;\nlet b = y;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let result = interp.execute(&program);
        assert!(result.is_err());
        assert!(
            interp.collected_errors.is_empty(),
            "default mode should not collect errors"
        );
    }

    #[test]
    fn test_keep_going_max_errors_limit() {
        let evaluator: &'static dyn crate::eval::OperationEvaluator =
            Box::leak(Box::new(NoOpEvaluator));
        let mut interp = Interpreter::new(evaluator).with_max_errors(2);
        // 5 statements each referencing undefined variables
        let src = "let a = x;\nlet b = y;\nlet c = z;\nlet d = w;\nlet e = v;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let result = interp.execute(&program);
        assert!(result.is_err());
        // With max_errors=2, at most 1 additional error should be collected
        // (first error is returned as Err, rest go into collected_errors,
        //  but collection stops when limit is reached)
        assert!(
            interp.collected_errors.len() <= 2,
            "max_errors should limit collection, got {}",
            interp.collected_errors.len()
        );
    }

    // =========================================================================
    // #136: InterpError::error_code()
    // =========================================================================

    #[test]
    fn test_interp_error_code_dispatch() {
        let span = Span::new(1, 1, 1, 1);
        assert_eq!(
            InterpError::UndefinedVariable {
                name: "x".into(),
                span,
                suggestion: None,
            }
            .error_code(),
            Some("E200")
        );
        assert_eq!(
            InterpError::UndefinedFunction {
                name: "f".into(),
                span,
                suggestion: None,
            }
            .error_code(),
            Some("E201")
        );
        assert_eq!(
            InterpError::TypeError {
                expected: "Int".into(),
                actual: "String".into(),
                context: "test".into(),
                span,
            }
            .error_code(),
            Some("E100")
        );
        assert_eq!(
            InterpError::ArityMismatch {
                name: "f".into(),
                expected: "2".into(),
                actual: 1,
                span,
            }
            .error_code(),
            Some("E405")
        );
        assert_eq!(InterpError::Cancelled.error_code(), Some("E407"));
        assert_eq!(
            InterpError::ResourceLimit {
                limit: "10".into(),
                actual: "20".into(),
                context: "test".into(),
                span,
            }
            .error_code(),
            Some("E409")
        );
        // Control flow signals return None
        assert_eq!(InterpError::BreakSignal(DataType::Null).error_code(), None);
        assert_eq!(InterpError::ContinueSignal.error_code(), None);
        assert_eq!(
            InterpError::ReturnSignal(DataType::Null).error_code(),
            None
        );
    }

    // =========================================================================
    // #282: Method arity errors show expected + actual
    // =========================================================================

    #[test]
    fn test_method_arity_error_shows_expected_and_actual() {
        let span = Span::new(1, 1, 1, 1);
        let err = InterpError::ArityMismatch {
            name: "map".to_string(),
            expected: "1".to_string(),
            actual: 0,
            span,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("expects 1 argument(s)"),
            "should show expected: {}",
            msg
        );
        assert!(msg.contains("got 0"), "should show actual: {}", msg);
        assert!(msg.contains("map"), "should show method name: {}", msg);
    }

    // =========================================================================
    // Concurrency: task registry + channel registry
    // =========================================================================

    #[test]
    fn test_task_store_and_join() {
        let tid = task_id();
        let handle = std::thread::spawn(|| Ok(DataType::Int64(42)));
        task_store(&tid, handle).unwrap();
        let result = task_join(&tid).unwrap().unwrap();
        assert_eq!(result, DataType::Int64(42));
    }

    #[test]
    fn test_task_join_not_found() {
        let result = task_join("nonexistent:0");
        assert!(result.is_err());
    }

    #[test]
    fn test_channel_store_send_recv() {
        let (tx_id, rx_id) = channel_ids();
        let (tx, rx) = std::sync::mpsc::channel();
        channel_store(&tx_id, ChannelSender { tx }).unwrap();
        channel_store(
            &rx_id,
            ChannelReceiver {
                rx: Arc::new(Mutex::new(rx)),
            },
        )
        .unwrap();

        // Send via the registry
        {
            let map = CHANNEL_REGISTRY.lock().unwrap();
            let entry = map.get(&tx_id).unwrap();
            let sender = entry.downcast_ref::<ChannelSender>().unwrap();
            sender.tx.send(DataType::String("hello".into())).unwrap();
        }

        // Recv via the registry
        {
            let map = CHANNEL_REGISTRY.lock().unwrap();
            let entry = map.get(&rx_id).unwrap();
            let receiver = entry.downcast_ref::<ChannelReceiver>().unwrap();
            let rx_guard = receiver.rx.lock().unwrap();
            let val = rx_guard.recv().unwrap();
            assert_eq!(val, DataType::String("hello".into()));
        }

        // Cleanup
        channel_remove(&tx_id).unwrap();
        channel_remove(&rx_id).unwrap();
    }

    #[test]
    fn test_channel_remove_not_found() {
        let result = channel_remove("nonexistent:0");
        assert!(result.is_err());
    }

    #[test]
    fn test_concurrent_module_in_std_modules() {
        assert!(STD_MODULE_NAMES.contains(&"concurrent"));
    }

    #[test]
    fn test_concurrent_module_ops() {
        let ops = std_module_ops("concurrent");
        assert!(ops.contains(&"channel"));
        assert!(ops.contains(&"chan_send"));
        assert!(ops.contains(&"chan_recv"));
        assert!(ops.contains(&"chan_try_recv"));
        assert!(ops.contains(&"chan_close"));
    }

    #[test]
    fn test_spawn_evaluator_basic_arithmetic() {
        let inputs = HashMap::from([
            ("a".to_string(), DataType::Int64(3)),
            ("b".to_string(), DataType::Int64(4)),
        ]);
        let config = HashMap::new();
        let result = spawn_eval_operation(OperationType::Add, &inputs, &config).unwrap();
        assert_eq!(result, DataType::Int64(7));
    }

    #[test]
    fn test_spawn_evaluator_comparison() {
        let inputs = HashMap::from([
            ("a".to_string(), DataType::Int64(3)),
            ("b".to_string(), DataType::Int64(4)),
        ]);
        let config = HashMap::new();
        let result = spawn_eval_operation(OperationType::Less, &inputs, &config).unwrap();
        assert_eq!(result, DataType::Bool(true));
    }

    // =========================================================================
    // Concurrency edge cases
    // =========================================================================

    #[test]
    fn test_spawned_thread_panic_returns_error() {
        // A spawned thread that panics should return an error, not crash the parent.
        let tid = task_id();
        let handle = std::thread::spawn(|| -> Result<DataType, String> {
            panic!("intentional panic in spawned thread");
        });
        task_store(&tid, handle).unwrap();
        let result = task_join(&tid);
        assert!(result.is_err(), "panicked thread should return Err");
        assert!(
            result.unwrap_err().contains("panicked"),
            "error should mention panic"
        );
    }

    #[test]
    fn test_spawned_thread_error_returns_err_value() {
        // A spawned thread that returns Err should propagate that error.
        let tid = task_id();
        let handle = std::thread::spawn(|| -> Result<DataType, String> {
            Err("computation failed".to_string())
        });
        task_store(&tid, handle).unwrap();
        let result = task_join(&tid);
        assert!(result.is_ok(), "join should succeed");
        let inner = result.unwrap();
        assert!(inner.is_err(), "inner result should be Err");
        assert_eq!(inner.unwrap_err(), "computation failed");
    }

    #[test]
    fn test_chan_recv_on_closed_channel_returns_null() {
        // When all senders are dropped, chan_recv should return null, not block.
        let (tx_id, rx_id) = channel_ids();
        let (tx, rx) = std::sync::mpsc::channel::<DataType>();
        channel_store(
            &rx_id,
            ChannelReceiver {
                rx: Arc::new(Mutex::new(rx)),
            },
        )
        .unwrap();

        // Drop the sender to close the channel.
        drop(tx);

        // Directly recv on the stored receiver — should get Null.
        let rx_arc = {
            let map = CHANNEL_REGISTRY.lock().unwrap();
            let entry = map.get(&rx_id).unwrap();
            let receiver = entry.downcast_ref::<ChannelReceiver>().unwrap();
            Arc::clone(&receiver.rx)
        };
        let rx_guard = rx_arc.lock().unwrap();
        // This mirrors the updated chan_recv behavior.
        let result = match rx_guard.recv() {
            Ok(val) => val,
            Err(_) => DataType::Null,
        };
        assert_eq!(result, DataType::Null, "recv on closed channel should return null");

        // Cleanup
        channel_remove(&rx_id).unwrap();
        // tx_id was never stored, so don't try to remove it.
    }

    #[test]
    fn test_multiple_receivers_on_same_channel() {
        // Multiple receivers sharing the same Arc<Mutex<Receiver>> should work:
        // each recv call gets a different value (no duplication).
        let (tx_id, rx_id) = channel_ids();
        let (tx, rx) = std::sync::mpsc::channel::<DataType>();
        channel_store(&tx_id, ChannelSender { tx: tx.clone() }).unwrap();
        channel_store(
            &rx_id,
            ChannelReceiver {
                rx: Arc::new(Mutex::new(rx)),
            },
        )
        .unwrap();

        // Send two values.
        tx.send(DataType::Int64(1)).unwrap();
        tx.send(DataType::Int64(2)).unwrap();

        // Clone the Arc to simulate multiple receivers.
        let rx_arc = {
            let map = CHANNEL_REGISTRY.lock().unwrap();
            let entry = map.get(&rx_id).unwrap();
            let receiver = entry.downcast_ref::<ChannelReceiver>().unwrap();
            Arc::clone(&receiver.rx)
        };

        // Two sequential recv calls should get different values (no duplication).
        let r1 = {
            let guard = rx_arc.lock().unwrap();
            guard.recv().unwrap()
        };
        let r2 = {
            let guard = rx_arc.lock().unwrap();
            guard.recv().unwrap()
        };

        assert_eq!(r1, DataType::Int64(1));
        assert_eq!(r2, DataType::Int64(2));

        // Cleanup
        channel_remove(&tx_id).unwrap();
        channel_remove(&rx_id).unwrap();
    }

    #[test]
    fn test_chan_try_recv_on_closed_channel_returns_null() {
        // chan_try_recv on a closed (disconnected) channel should return null.
        let (_tx_id, rx_id) = channel_ids();
        let (tx, rx) = std::sync::mpsc::channel::<DataType>();
        channel_store(
            &rx_id,
            ChannelReceiver {
                rx: Arc::new(Mutex::new(rx)),
            },
        )
        .unwrap();

        // Drop sender to close.
        drop(tx);

        let rx_arc = {
            let map = CHANNEL_REGISTRY.lock().unwrap();
            let entry = map.get(&rx_id).unwrap();
            let receiver = entry.downcast_ref::<ChannelReceiver>().unwrap();
            Arc::clone(&receiver.rx)
        };
        let rx_guard = rx_arc.lock().unwrap();
        let result = match rx_guard.try_recv() {
            Ok(val) => val,
            Err(_) => DataType::Null,
        };
        assert_eq!(result, DataType::Null);

        channel_remove(&rx_id).unwrap();
    }
}
