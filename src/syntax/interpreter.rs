//! AST interpreter for the MAGI v2 language.
//!
//! Walks the AST directly, executing statements with support for loops,
//! mutable variables, and an environment. Delegates operation evaluation
//! to the injected `OperationEvaluator` — no duplication.

use super::ast::*;
use crate::eval::{EvalError, OperationEvaluator};
use crate::ops::op_input_ports;
use crate::types::{DataType, FutureState, OperationType};

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Maximum iterations for while loops to prevent infinite loops.
const MAX_LOOP_ITERATIONS: usize = 10_000;

/// Maximum call depth for recursion guard.
const MAX_CALL_DEPTH: usize = 48;

/// GC trigger threshold: collect after this many allocations since last GC.
const GC_ALLOC_THRESHOLD: usize = 256;

/// Maximum output string length (10 MB).
const MAX_STRING_OUTPUT: usize = 10_000_000;

/// Maximum array element count.
const MAX_ARRAY_ELEMENTS: usize = 10_000_000;

/// A resolved package with its functions pre-extracted
#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    /// Package ID
    pub id: String,
    /// Parsed function definitions from the package source
    pub functions: HashMap<String, FunctionDef>,
    /// Use statements that need to be executed to set up std aliases
    pub use_statements: Vec<Statement>,
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
            self.next_addr += size;
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
    }
}

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
    /// Package import guard: tracks packages currently being imported (circular import detection).
    importing_packages: std::collections::HashSet<String>,
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
            importing_packages: std::collections::HashSet::new(),
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

    /// Collect all variable names visible in the current scope stack.
    fn available_variable_names(&self) -> Vec<String> {
        self.symbols.iter().rev()
            .flat_map(|scope| scope.keys().cloned())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect()
    }

    /// Collect all function names currently defined.
    fn available_function_names(&self) -> Vec<String> {
        self.functions.keys().cloned().collect()
    }

    /// Suggest a variable name using Levenshtein distance.
    fn suggest_variable(&self, name: &str) -> Option<String> {
        let names = self.available_variable_names();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        super::errors::suggest_name(name, &refs)
    }

    /// Suggest a function name using Levenshtein distance.
    fn suggest_function(&self, name: &str) -> Option<String> {
        let names = self.available_function_names();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        super::errors::suggest_name(name, &refs)
    }

    /// Define a variable in the current scope.
    fn define(&mut self, name: &str, addr: MemAddr, mutable: bool) {
        if let Some(scope) = self.symbols.last_mut() {
            scope.insert(name.to_string(), SymbolEntry { addr, mutable });
        }
    }

    /// Set the cancellation token for checking during loops.
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
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
                        datatype_type_name(val).to_string(),
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
                StatementKind::EnumDef { name, variants } => {
                    self.enum_defs.insert(name.clone(), variants.clone());
                }
                StatementKind::StructDef { name, fields } => {
                    self.struct_defs.insert(name.clone(), fields.clone());
                }
                StatementKind::ModuleDef { name, body } => {
                    // Register module functions with qualified names
                    for inner in &body.statements {
                        match &inner.kind {
                            StatementKind::FunctionDef(def) => {
                                let qualified = format!("{}::{}", name, def.name);
                                self.functions.insert(qualified, def.clone());
                            }
                            StatementKind::AsyncFunctionDef(def) => {
                                let qualified = format!("{}::{}", name, def.name);
                                self.async_fns.insert(qualified.clone());
                                self.functions.insert(qualified, def.clone());
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Pass 2: determine execution mode
        let has_main = self.functions.contains_key("main");

        if has_main {
            // Process imports and use statements, then call main()
            for stmt in &program.statements {
                match &stmt.kind {
                    StatementKind::Import(plugin_id) => {
                        self.imports.insert(plugin_id.clone());
                    }
                    StatementKind::Use { .. } => {
                        self.exec_statement(stmt)?;
                    }
                    _ => {}
                }
            }
            let main_span = self
                .functions
                .get("main")
                .map(|f| f.span)
                .unwrap_or_default();
            self.call_function("main", &[], main_span)
        } else {
            // Backward compat: execute top-level statements, skip FunctionDefs/ModuleDefs
            let mut last_value = DataType::Null;
            for stmt in &program.statements {
                if matches!(
                    &stmt.kind,
                    StatementKind::FunctionDef(_)
                        | StatementKind::AsyncFunctionDef(_)
                        | StatementKind::ModuleDef { .. }
                ) {
                    continue;
                }
                last_value = match self.exec_statement(stmt) {
                    Ok(val) => val,
                    Err(InterpError::BreakSignal(_)) => {
                        return Err(InterpError::BreakOutsideLoop { span: stmt.span });
                    }
                    Err(InterpError::ContinueSignal) => {
                        return Err(InterpError::ContinueOutsideLoop { span: stmt.span });
                    }
                    Err(InterpError::ReturnSignal(_)) => {
                        return Err(InterpError::ReturnOutsideFunction { span: stmt.span });
                    }
                    Err(e) => return Err(e),
                };
                if self.is_cancelled() {
                    return Err(InterpError::Cancelled);
                }
            }
            Ok(last_value)
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
                let val = self.eval_expr(value)?;
                let addr = self.heap.alloc(val.clone());
                self.define(name, addr, false);
                Ok(val)
            }

            StatementKind::LetMut { name, value, .. } => {
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

            StatementKind::ForLoop {
                pattern,
                iterable,
                body,
            } => {
                let iter_val = self.eval_expr(iterable)?;
                let items = match iter_val {
                    DataType::Array(arr) => arr,
                    DataType::Map(map) => {
                        map.into_iter()
                            .map(|(k, v)| {
                                let mut entry = std::collections::BTreeMap::new();
                                entry.insert("key".to_string(), DataType::String(k));
                                entry.insert("value".to_string(), v);
                                DataType::Map(entry)
                            })
                            .collect()
                    }
                    DataType::String(s) => {
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
                        Err(InterpError::ContinueSignal) => continue,
                        Err(e) => return Err(e),
                    }
                    self.maybe_gc();
                }
                Ok(last)
            }

            StatementKind::WhileLoop { condition, body } => {
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
                                actual: datatype_type_name(&other).to_string(),
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
                        Err(InterpError::ContinueSignal) => {}
                        Err(e) => return Err(e), // propagate return/errors
                    }
                    iterations += 1;
                    self.maybe_gc();
                }
                Ok(last)
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

            StatementKind::FunctionDef(_) | StatementKind::AsyncFunctionDef(_) => {
                // Already collected in pass 1; nothing to do here.
                Ok(DataType::Null)
            }

            StatementKind::Break(ref val_expr) => {
                let val = match val_expr {
                    Some(expr) => self.eval_expr(expr)?,
                    None => DataType::Null,
                };
                Err(InterpError::BreakSignal(val))
            }

            StatementKind::Continue => Err(InterpError::ContinueSignal),

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
                let inputs: HashMap<String, DataType> = input_ports.first()
                    .map(|p| (p.to_string(), current))
                    .into_iter()
                    .chain(input_ports.get(1).map(|p| (p.to_string(), rhs)))
                    .collect();
                let result = self.evaluator.eval_operation(op_type, &inputs, &HashMap::new()).map_err(|e| {
                    InterpError::EvalError {
                        error: e,
                        span: stmt.span,
                    }
                })?;
                self.heap.write(addr, result.clone());
                Ok(result)
            }

            StatementKind::TryCatch {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                let try_result = self.exec_block(try_block);
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

            StatementKind::EnumDef { name, variants } => {
                // Store enum definition
                self.enum_defs.insert(name.clone(), variants.clone());
                Ok(DataType::Null)
            }

            StatementKind::StructDef { name, fields } => {
                // Store struct definition
                self.struct_defs.insert(name.clone(), fields.clone());
                Ok(DataType::Null)
            }

            StatementKind::Use { path, alias, glob } => {
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
                    let prefix = format!("{}::", full_path);
                    let matching: Vec<(String, FunctionDef)> = self
                        .functions
                        .iter()
                        .filter(|(k, _)| k.starts_with(&prefix))
                        .map(|(k, v)| {
                            let short_name = k[prefix.len()..].to_string();
                            (short_name, v.clone())
                        })
                        .collect();
                    for (short_name, def) in matching {
                        self.functions.insert(short_name, def);
                    }
                } else {
                    let func_name = path.last().cloned().unwrap_or_default();
                    if let Some(func) = self.functions.get(&full_path).cloned() {
                        let local_name = alias.as_ref().unwrap_or(&func_name).clone();
                        self.functions.insert(local_name, func.clone());
                        // Also register under unqualified name for recursive calls
                        if alias.is_some() && !self.functions.contains_key(&func_name) {
                            self.functions.insert(func_name, func);
                        }
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
                    DataType::Array(arr) => result.extend(arr),
                    other => {
                        return Err(InterpError::TypeError {
                            expected: "Array".to_string(),
                            actual: datatype_type_name(&other).to_string(),
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

    // =========================================================================
    // Slice evaluation (arr[1..3], str[0..5])
    // =========================================================================

    fn eval_slice(&self, obj: &DataType, start: &DataType, end: &DataType, inclusive: bool, span: Span) -> Result<DataType, InterpError> {
        let s_raw = start.to_i64().unwrap_or(0);
        let e_raw_i = end.to_i64().unwrap_or(0);
        let s = if s_raw < 0 { 0usize } else { s_raw as usize };
        let e_raw = if e_raw_i < 0 { 0usize } else { e_raw_i as usize };
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
                actual: datatype_type_name(obj).to_string(),
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
                actual: datatype_type_name(&fn_val).to_string(),
                context: "higher-order method callback".to_string(),
                span,
            }),
        };
        self.call_function(&fn_name, args, span)
    }

    // =========================================================================
    // Higher-order function methods (Phase 1)
    // =========================================================================

    fn try_eval_hof_method(&mut self, obj: &DataType, method: &str, args: &[Expression], span: Span) -> Result<Option<DataType>, InterpError> {
        match obj {
            DataType::Array(arr) => {
                match method {
                    "map" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "map".to_string(), expected: 1, actual: 0, span }); }
                        let mut result = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            result.push(self.call_lambda_with_args(&args[0], &[item.clone()], span)?);
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "filter" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "filter".to_string(), expected: 1, actual: 0, span }); }
                        let mut result = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let keep = self.call_lambda_with_args(&args[0], &[item.clone()], span)?;
                            if keep.to_bool() {
                                result.push(item.clone());
                            }
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "reduce" => {
                        if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "reduce".to_string(), expected: 2, actual: args.len(), span }); }
                        let mut acc = self.eval_expr(&args[0])?;
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            acc = self.call_lambda_with_args(&args[1], &[acc, item.clone()], span)?;
                        }
                        Ok(Some(acc))
                    }
                    "find" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "find".to_string(), expected: 1, actual: 0, span }); }
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let matches = self.call_lambda_with_args(&args[0], &[item.clone()], span)?;
                            if matches.to_bool() {
                                return Ok(Some(item.clone()));
                            }
                        }
                        Ok(Some(DataType::Null))
                    }
                    "find_index" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "find_index".to_string(), expected: 1, actual: 0, span }); }
                        for (i, item) in arr.iter().enumerate() {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let matches = self.call_lambda_with_args(&args[0], &[item.clone()], span)?;
                            if matches.to_bool() {
                                return Ok(Some(DataType::Int64(i as i64)));
                            }
                        }
                        Ok(Some(DataType::Null))
                    }
                    "any" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "any".to_string(), expected: 1, actual: 0, span }); }
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let matches = self.call_lambda_with_args(&args[0], &[item.clone()], span)?;
                            if matches.to_bool() {
                                return Ok(Some(DataType::Bool(true)));
                            }
                        }
                        Ok(Some(DataType::Bool(false)))
                    }
                    "all" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "all".to_string(), expected: 1, actual: 0, span }); }
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let matches = self.call_lambda_with_args(&args[0], &[item.clone()], span)?;
                            if !matches.to_bool() {
                                return Ok(Some(DataType::Bool(false)));
                            }
                        }
                        Ok(Some(DataType::Bool(true)))
                    }
                    "flat_map" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "flat_map".to_string(), expected: 1, actual: 0, span }); }
                        let mut result = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let mapped = self.call_lambda_with_args(&args[0], &[item.clone()], span)?;
                            match mapped {
                                DataType::Array(inner) => result.extend(inner),
                                other => result.push(other),
                            }
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "each" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "each".to_string(), expected: 1, actual: 0, span }); }
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            self.call_lambda_with_args(&args[0], &[item.clone()], span)?;
                        }
                        Ok(Some(DataType::Null))
                    }
                    "sort_by" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "sort_by".to_string(), expected: 1, actual: 0, span }); }
                        let mut sorted = arr.clone();
                        // Simple insertion sort with comparator
                        for i in 1..sorted.len() {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let key = sorted[i].clone();
                            let mut j = i;
                            while j > 0 {
                                let cmp = self.call_lambda_with_args(&args[0], &[sorted[j - 1].clone(), key.clone()], span)?;
                                let cmp_val = cmp.to_f64().ok_or_else(|| InterpError::TypeError {
                                    expected: "number".to_string(),
                                    actual: datatype_type_name(&cmp).to_string(),
                                    context: "sort_by comparator must return a number".to_string(),
                                    span,
                                })?;
                                if cmp_val > 0.0 {
                                    sorted[j] = sorted[j - 1].clone();
                                    j -= 1;
                                } else {
                                    break;
                                }
                            }
                            sorted[j] = key;
                        }
                        Ok(Some(DataType::Array(sorted)))
                    }
                    "group_by" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "group_by".to_string(), expected: 1, actual: 0, span }); }
                        let mut groups: std::collections::BTreeMap<String, Vec<DataType>> = std::collections::BTreeMap::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let key = self.call_lambda_with_args(&args[0], &[item.clone()], span)?;
                            let key_str = key.to_string_lossy();
                            groups.entry(key_str).or_default().push(item.clone());
                        }
                        let map: std::collections::BTreeMap<String, DataType> = groups.into_iter()
                            .map(|(k, v)| (k, DataType::Array(v)))
                            .collect();
                        Ok(Some(DataType::Map(map)))
                    }
                    "min_by" => {
                        if arr.is_empty() { return Ok(Some(DataType::Null)); }
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "min_by".to_string(), expected: 1, actual: 0, span }); }
                        let mut min = arr[0].clone();
                        for item in &arr[1..] {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let cmp = self.call_lambda_with_args(&args[0], &[min.clone(), item.clone()], span)?;
                            let cmp_val = cmp.to_f64().ok_or_else(|| InterpError::TypeError {
                                expected: "number".to_string(),
                                actual: datatype_type_name(&cmp).to_string(),
                                context: "min_by comparator must return a number".to_string(),
                                span,
                            })?;
                            if cmp_val > 0.0 {
                                min = item.clone();
                            }
                        }
                        Ok(Some(min))
                    }
                    "max_by" => {
                        if arr.is_empty() { return Ok(Some(DataType::Null)); }
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "max_by".to_string(), expected: 1, actual: 0, span }); }
                        let mut max = arr[0].clone();
                        for item in &arr[1..] {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let cmp = self.call_lambda_with_args(&args[0], &[max.clone(), item.clone()], span)?;
                            let cmp_val = cmp.to_f64().ok_or_else(|| InterpError::TypeError {
                                expected: "number".to_string(),
                                actual: datatype_type_name(&cmp).to_string(),
                                context: "max_by comparator must return a number".to_string(),
                                span,
                            })?;
                            if cmp_val < 0.0 {
                                max = item.clone();
                            }
                        }
                        Ok(Some(max))
                    }
                    "take_while" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "take_while".to_string(), expected: 1, actual: 0, span }); }
                        let mut result = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let keep = self.call_lambda_with_args(&args[0], &[item.clone()], span)?;
                            if !keep.to_bool() { break; }
                            result.push(item.clone());
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "skip_while" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "skip_while".to_string(), expected: 1, actual: 0, span }); }
                        let mut skipping = true;
                        let mut result = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            if skipping {
                                let skip = self.call_lambda_with_args(&args[0], &[item.clone()], span)?;
                                if skip.to_bool() { continue; }
                                skipping = false;
                            }
                            result.push(item.clone());
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "partition" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "partition".to_string(), expected: 1, actual: 0, span }); }
                        let mut trues = Vec::new();
                        let mut falses = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let matches = self.call_lambda_with_args(&args[0], &[item.clone()], span)?;
                            if matches.to_bool() {
                                trues.push(item.clone());
                            } else {
                                falses.push(item.clone());
                            }
                        }
                        Ok(Some(DataType::Array(vec![
                            DataType::Array(trues),
                            DataType::Array(falses),
                        ])))
                    }
                    "scan" => {
                        if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "scan".to_string(), expected: 2, actual: args.len(), span }); }
                        let mut acc = self.eval_expr(&args[0])?;
                        let mut result = Vec::new();
                        for item in arr {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            acc = self.call_lambda_with_args(&args[1], &[acc.clone(), item.clone()], span)?;
                            result.push(acc.clone());
                        }
                        Ok(Some(DataType::Array(result)))
                    }
                    "enumerate" => {
                        let result: Vec<DataType> = arr.iter().enumerate()
                            .map(|(i, item)| DataType::Array(vec![DataType::Int64(i as i64), item.clone()]))
                            .collect();
                        Ok(Some(DataType::Array(result)))
                    }
                    "zip" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "zip".to_string(), expected: 1, actual: 0, span }); }
                        let other = self.eval_expr(&args[0])?;
                        let other_arr = match other {
                            DataType::Array(a) => a,
                            _ => return Err(InterpError::TypeError {
                                expected: "Array".to_string(),
                                actual: datatype_type_name(&other).to_string(),
                                context: "zip argument".to_string(),
                                span,
                            }),
                        };
                        let len = arr.len().min(other_arr.len());
                        let result: Vec<DataType> = (0..len)
                            .map(|i| DataType::Array(vec![arr[i].clone(), other_arr[i].clone()]))
                            .collect();
                        Ok(Some(DataType::Array(result)))
                    }
                    "chunk" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "chunk".to_string(), expected: 1, actual: 0, span }); }
                        let size_val = self.eval_expr(&args[0])?;
                        let size = size_val.to_i64().unwrap_or(1).max(1) as usize;
                        let result: Vec<DataType> = arr.chunks(size)
                            .map(|chunk| DataType::Array(chunk.to_vec()))
                            .collect();
                        Ok(Some(DataType::Array(result)))
                    }
                    _ => Ok(None),
                }
            }
            DataType::Map(map) => {
                match method {
                    "filter_entries" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "filter_entries".to_string(), expected: 1, actual: 0, span }); }
                        let mut result = std::collections::BTreeMap::new();
                        for (k, v) in map {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let keep = self.call_lambda_with_args(&args[0], &[DataType::String(k.clone()), v.clone()], span)?;
                            if keep.to_bool() {
                                result.insert(k.clone(), v.clone());
                            }
                        }
                        Ok(Some(DataType::Map(result)))
                    }
                    "map_values" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "map_values".to_string(), expected: 1, actual: 0, span }); }
                        let mut result = std::collections::BTreeMap::new();
                        for (k, v) in map {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let new_v = self.call_lambda_with_args(&args[0], &[v.clone()], span)?;
                            result.insert(k.clone(), new_v);
                        }
                        Ok(Some(DataType::Map(result)))
                    }
                    "map_keys" => {
                        if args.is_empty() { return Err(InterpError::ArityMismatch { name: "map_keys".to_string(), expected: 1, actual: 0, span }); }
                        let mut result = std::collections::BTreeMap::new();
                        for (k, v) in map {
                            if self.is_cancelled() { return Err(InterpError::Cancelled); }
                            let new_k = self.call_lambda_with_args(&args[0], &[DataType::String(k.clone())], span)?;
                            let key_str = match new_k {
                                DataType::String(s) => s,
                                other => other.to_string_lossy(),
                            };
                            result.insert(key_str, v.clone());
                        }
                        Ok(Some(DataType::Map(result)))
                    }
                    _ => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    // =========================================================================
    // Direct interpreter methods (Phase 13, 16)
    // =========================================================================

    fn try_eval_direct_method(&mut self, obj: &DataType, method: &str, args: &[Expression], span: Span) -> Result<Option<DataType>, InterpError> {
        match obj {
            // Number methods (Phase 13)
            DataType::Int64(n) => match method {
                "abs" => Ok(Some(match n.checked_abs() {
                    Some(v) => DataType::Int64(v),
                    None => DataType::Null, // i64::MIN overflow
                })),
                "to_string" => Ok(Some(DataType::String(n.to_string()))),
                "to_float64" => Ok(Some(DataType::Float64(*n as f64))),
                "pow" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pow".to_string(), expected: 1, actual: 0, span }); }
                    let exp = self.eval_expr(&args[0])?.to_i64().unwrap_or(0);
                    if exp < 0 {
                        // Negative exponents on integers would produce fractions; return 0
                        // (integer division: 1 / n^|exp| rounds to 0 for |n| > 1)
                        if *n == 1 { Ok(Some(DataType::Int64(1))) }
                        else if *n == -1 { Ok(Some(DataType::Int64(if exp % 2 == 0 { 1 } else { -1 }))) }
                        else { Ok(Some(DataType::Int64(0))) }
                    } else if exp > u32::MAX as i64 {
                        Ok(Some(DataType::Null)) // exponent too large
                    } else {
                        match n.checked_pow(exp as u32) {
                            Some(result) => Ok(Some(DataType::Int64(result))),
                            None => Ok(Some(DataType::Null)), // overflow
                        }
                    }
                }
                "min" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "min".to_string(), expected: 1, actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let other = arg.to_i64().or_else(|| arg.to_f64().map(|f| f as i64)).unwrap_or(*n);
                    Ok(Some(DataType::Int64((*n).min(other))))
                }
                "max" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "max".to_string(), expected: 1, actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let other = arg.to_i64().or_else(|| arg.to_f64().map(|f| f as i64)).unwrap_or(*n);
                    Ok(Some(DataType::Int64((*n).max(other))))
                }
                "sign" => Ok(Some(DataType::Int64(n.signum()))),
                "clamp" => {
                    if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "clamp".to_string(), expected: 2, actual: args.len(), span }); }
                    let lo_arg = self.eval_expr(&args[0])?;
                    let hi_arg = self.eval_expr(&args[1])?;
                    let min_val = lo_arg.to_i64().or_else(|| lo_arg.to_f64().map(|f| f as i64)).unwrap_or(i64::MIN);
                    let max_val = hi_arg.to_i64().or_else(|| hi_arg.to_f64().map(|f| f as i64)).unwrap_or(i64::MAX);
                    let (lo, hi) = if min_val <= max_val { (min_val, max_val) } else { (max_val, min_val) };
                    Ok(Some(DataType::Int64((*n).max(lo).min(hi))))
                }
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
                "to_string" => Ok(Some(DataType::String(n.to_string()))),
                "to_int64" => {
                    if !n.is_finite() {
                        Ok(Some(DataType::Null))
                    } else if *n > i64::MAX as f64 || *n < i64::MIN as f64 {
                        Ok(Some(DataType::Null))
                    } else {
                        Ok(Some(DataType::Int64(*n as i64)))
                    }
                }
                "pow" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pow".to_string(), expected: 1, actual: 0, span }); }
                    let exp = self.eval_expr(&args[0])?.to_f64().unwrap_or(0.0);
                    Ok(Some(DataType::Float64(n.powf(exp))))
                }
                "min" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "min".to_string(), expected: 1, actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let other = arg.to_f64().unwrap_or(*n);
                    Ok(Some(DataType::Float64(n.min(other))))
                }
                "max" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "max".to_string(), expected: 1, actual: 0, span }); }
                    let arg = self.eval_expr(&args[0])?;
                    let other = arg.to_f64().unwrap_or(*n);
                    Ok(Some(DataType::Float64(n.max(other))))
                }
                "sign" => Ok(Some(DataType::Float64(n.signum()))),
                "ln" => Ok(Some(DataType::Float64(n.ln()))),
                "log2" => Ok(Some(DataType::Float64(n.log2()))),
                "log10" => Ok(Some(DataType::Float64(n.log10()))),
                "sin" => Ok(Some(DataType::Float64(n.sin()))),
                "cos" => Ok(Some(DataType::Float64(n.cos()))),
                "tan" => Ok(Some(DataType::Float64(n.tan()))),
                "clamp" => {
                    if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "clamp".to_string(), expected: 2, actual: args.len(), span }); }
                    let lo_arg = self.eval_expr(&args[0])?;
                    let hi_arg = self.eval_expr(&args[1])?;
                    let min_val = lo_arg.to_f64().unwrap_or(f64::NEG_INFINITY);
                    let max_val = hi_arg.to_f64().unwrap_or(f64::INFINITY);
                    let (lo, hi) = if min_val <= max_val { (min_val, max_val) } else { (max_val, min_val) };
                    Ok(Some(DataType::Float64(n.max(lo).min(hi))))
                }
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
                _ => Ok(None),
            },
            DataType::Int32(n) => match method {
                "abs" => Ok(Some(match n.checked_abs() {
                    Some(v) => DataType::Int32(v),
                    None => DataType::Null, // i32::MIN overflow
                })),
                _ => Ok(None),
            },
            // String methods (Phase 16+)
            DataType::String(s) => match method {
                "is_empty" => Ok(Some(DataType::Bool(s.is_empty()))),
                "is_numeric" => Ok(Some(DataType::Bool(!s.is_empty() && s.parse::<f64>().is_ok()))),
                "is_alphabetic" => Ok(Some(DataType::Bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic())))),
                "to_int" => Ok(Some(s.parse::<i64>().map(DataType::Int64).unwrap_or(DataType::Null))),
                "to_float" => Ok(Some(s.parse::<f64>().map(DataType::Float64).unwrap_or(DataType::Null))),
                "len" | "length" => Ok(Some(DataType::Int64(s.chars().count() as i64))),
                "trim" => Ok(Some(DataType::String(s.trim().to_string()))),
                "trim_start" => Ok(Some(DataType::String(s.trim_start().to_string()))),
                "trim_end" => Ok(Some(DataType::String(s.trim_end().to_string()))),
                "to_upper" | "to_uppercase" => Ok(Some(DataType::String(s.to_uppercase()))),
                "to_lower" | "to_lowercase" => Ok(Some(DataType::String(s.to_lowercase()))),
                "reverse" => Ok(Some(DataType::String(s.chars().rev().collect()))),
                "chars" => {
                    if s.chars().count() > MAX_ARRAY_ELEMENTS {
                        return Err(InterpError::TypeError { expected: format!("string at most {} chars for chars()", MAX_ARRAY_ELEMENTS), actual: format!("{}", s.chars().count()), context: "string chars".to_string(), span });
                    }
                    Ok(Some(DataType::Array(s.chars().map(|c| DataType::String(c.to_string())).collect())))
                }
                "lines" => {
                    let lines: Vec<DataType> = s.lines().map(|l| DataType::String(l.to_string())).collect();
                    if lines.len() > MAX_ARRAY_ELEMENTS {
                        return Err(InterpError::TypeError { expected: format!("string at most {} lines for lines()", MAX_ARRAY_ELEMENTS), actual: format!("{}", lines.len()), context: "string lines".to_string(), span });
                    }
                    Ok(Some(DataType::Array(lines)))
                }
                "to_string" => Ok(Some(DataType::String(s.clone()))),
                "split" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "split".to_string(), expected: 1, actual: 0, span }); }
                    let sep = match self.eval_expr(&args[0])? {
                        DataType::String(sep) => sep,
                        _ => return Err(InterpError::TypeError { expected: "String".to_string(), actual: "non-string".to_string(), context: "split separator".to_string(), span }),
                    };
                    let parts: Vec<DataType> = s.split(&sep).take(MAX_ARRAY_ELEMENTS + 1).map(|p| DataType::String(p.to_string())).collect();
                    if parts.len() > MAX_ARRAY_ELEMENTS {
                        return Err(InterpError::TypeError { expected: format!("split result at most {} elements", MAX_ARRAY_ELEMENTS), actual: format!("more than {}", MAX_ARRAY_ELEMENTS), context: "string split".to_string(), span });
                    }
                    Ok(Some(DataType::Array(parts)))
                }
                "replace" => {
                    if args.len() < 2 { return Err(InterpError::ArityMismatch { name: "replace".to_string(), expected: 2, actual: args.len(), span }); }
                    let from = match self.eval_expr(&args[0])? { DataType::String(s) => s, other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "replace pattern".to_string(), span }) };
                    let to = match self.eval_expr(&args[1])? { DataType::String(s) => s, other => return Err(InterpError::TypeError { expected: "String".to_string(), actual: other.type_name().to_string(), context: "replace replacement".to_string(), span }) };
                    if !from.is_empty() && to.len() > from.len() {
                        let match_count = s.matches(&from).count();
                        let growth = match_count.saturating_mul(to.len().saturating_sub(from.len()));
                        if s.len().saturating_add(growth) > MAX_STRING_OUTPUT {
                            return Err(InterpError::TypeError { expected: format!("replace result at most {} bytes", MAX_STRING_OUTPUT), actual: format!("{}", s.len().saturating_add(growth)), context: "string replace".to_string(), span });
                        }
                    }
                    Ok(Some(DataType::String(s.replace(&from, &to))))
                }
                "contains" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "contains".to_string(), expected: 1, actual: 0, span }); }
                    let needle = match self.eval_expr(&args[0])? { DataType::String(s) => s, _ => return Ok(Some(DataType::Bool(false))) };
                    Ok(Some(DataType::Bool(s.contains(&needle))))
                }
                "starts_with" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "starts_with".to_string(), expected: 1, actual: 0, span }); }
                    let prefix = match self.eval_expr(&args[0])? { DataType::String(s) => s, _ => return Ok(Some(DataType::Bool(false))) };
                    Ok(Some(DataType::Bool(s.starts_with(&prefix))))
                }
                "ends_with" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "ends_with".to_string(), expected: 1, actual: 0, span }); }
                    let suffix = match self.eval_expr(&args[0])? { DataType::String(s) => s, _ => return Ok(Some(DataType::Bool(false))) };
                    Ok(Some(DataType::Bool(s.ends_with(&suffix))))
                }
                "index_of" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "index_of".to_string(), expected: 1, actual: 0, span }); }
                    let needle = match self.eval_expr(&args[0])? { DataType::String(s) => s, _ => return Ok(Some(DataType::Int64(-1))) };
                    Ok(Some(match s.find(&needle) {
                        Some(byte_idx) => DataType::Int64(s[..byte_idx].chars().count() as i64),
                        None => DataType::Int64(-1),
                    }))
                }
                "repeat" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "repeat".to_string(), expected: 1, actual: 0, span }); }
                    let n = self.eval_expr(&args[0])?.to_i64().unwrap_or(0).max(0) as usize;
                    const MAX_REPEAT_LEN: usize = 10_000_000;
                    if n > 0 && s.len().saturating_mul(n) > MAX_REPEAT_LEN {
                        return Err(InterpError::TypeError {
                            expected: format!("repeat count producing at most {} chars", MAX_REPEAT_LEN),
                            actual: format!("{} * {} = {}", s.len(), n, s.len().saturating_mul(n)),
                            context: "string repeat".to_string(),
                            span,
                        });
                    }
                    Ok(Some(DataType::String(s.repeat(n))))
                }
                "char_at" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "char_at".to_string(), expected: 1, actual: 0, span }); }
                    let idx = self.eval_expr(&args[0])?.to_i64().unwrap_or(-1);
                    if idx < 0 {
                        Ok(Some(DataType::Null))
                    } else {
                        Ok(Some(s.chars().nth(idx as usize).map(|c| DataType::String(c.to_string())).unwrap_or(DataType::Null)))
                    }
                }
                "pad_start" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pad_start".to_string(), expected: 1, actual: 0, span }); }
                    let width = self.eval_expr(&args[0])?.to_i64().unwrap_or(0).max(0) as usize;
                    const MAX_PAD_WIDTH: usize = 10_000_000;
                    if width > MAX_PAD_WIDTH {
                        return Err(InterpError::TypeError {
                            expected: format!("pad width at most {}", MAX_PAD_WIDTH),
                            actual: format!("{}", width),
                            context: "pad_start".to_string(),
                            span,
                        });
                    }
                    let pad_char = if args.len() > 1 { match self.eval_expr(&args[1])? { DataType::String(c) => c.chars().next().unwrap_or(' '), _ => ' ' } } else { ' ' };
                    let pad_len = width.saturating_sub(s.chars().count());
                    Ok(Some(DataType::String(format!("{}{}", std::iter::repeat(pad_char).take(pad_len).collect::<String>(), s))))
                }
                "pad_end" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "pad_end".to_string(), expected: 1, actual: 0, span }); }
                    let width = self.eval_expr(&args[0])?.to_i64().unwrap_or(0).max(0) as usize;
                    const MAX_PAD_WIDTH: usize = 10_000_000;
                    if width > MAX_PAD_WIDTH {
                        return Err(InterpError::TypeError {
                            expected: format!("pad width at most {}", MAX_PAD_WIDTH),
                            actual: format!("{}", width),
                            context: "pad_end".to_string(),
                            span,
                        });
                    }
                    let pad_char = if args.len() > 1 { match self.eval_expr(&args[1])? { DataType::String(c) => c.chars().next().unwrap_or(' '), _ => ' ' } } else { ' ' };
                    let pad_len = width.saturating_sub(s.chars().count());
                    Ok(Some(DataType::String(format!("{}{}", s, std::iter::repeat(pad_char).take(pad_len).collect::<String>()))))
                }
                "substring" | "slice" => {
                    if args.is_empty() { return Err(InterpError::ArityMismatch { name: "substring".to_string(), expected: 1, actual: 0, span }); }
                    let char_len = s.chars().count();
                    let start = self.eval_expr(&args[0])?.to_i64().unwrap_or(0).max(0) as usize;
                    let end = if args.len() > 1 { self.eval_expr(&args[1])?.to_i64().unwrap_or(char_len as i64).max(0) as usize } else { char_len };
                    let start = start.min(char_len);
                    let end = end.min(char_len);
                    if start >= end {
                        Ok(Some(DataType::String(String::new())))
                    } else {
                        Ok(Some(DataType::String(s.chars().skip(start).take(end - start).collect())))
                    }
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
                        match item {
                            DataType::Float64(f) => { has_float = true; float_sum += f; }
                            DataType::Float32(f) => { has_float = true; float_sum += *f as f64; }
                            _ => {
                                if !int_overflow {
                                    match int_sum.checked_add(item.to_i64().unwrap_or(0)) {
                                        Some(v) => int_sum = v,
                                        None => {
                                            has_float = true;
                                            float_sum += int_sum as f64 + item.to_i64().unwrap_or(0) as f64;
                                            int_overflow = true;
                                        }
                                    }
                                } else {
                                    float_sum += item.to_i64().unwrap_or(0) as f64;
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
                        match item {
                            DataType::Float64(f) => { has_float = true; float_prod *= f; }
                            DataType::Float32(f) => { has_float = true; float_prod *= *f as f64; }
                            _ => {
                                if !int_overflow {
                                    match int_prod.checked_mul(item.to_i64().unwrap_or(1)) {
                                        Some(v) => int_prod = v,
                                        None => {
                                            // Overflow: promote to float
                                            has_float = true;
                                            float_prod *= int_prod as f64 * item.to_i64().unwrap_or(1) as f64;
                                            int_overflow = true;
                                        }
                                    }
                                } else {
                                    float_prod *= item.to_i64().unwrap_or(1) as f64;
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
                        let cmp = match (&min, item) {
                            (DataType::Int64(a), DataType::Int64(b)) => *a > *b,
                            (DataType::Float64(a), DataType::Float64(b)) => *a > *b,
                            (DataType::Int64(a), DataType::Float64(b)) => (*a as f64) > *b,
                            (DataType::Float64(a), DataType::Int64(b)) => *a > (*b as f64),
                            (DataType::String(a), DataType::String(b)) => a > b,
                            _ => false,
                        };
                        if cmp { min = item.clone(); }
                    }
                    Ok(Some(min))
                }
                "max" => {
                    if arr.is_empty() { return Ok(Some(DataType::Null)); }
                    let mut max = arr[0].clone();
                    for item in &arr[1..] {
                        let cmp = match (&max, item) {
                            (DataType::Int64(a), DataType::Int64(b)) => *a < *b,
                            (DataType::Float64(a), DataType::Float64(b)) => *a < *b,
                            (DataType::Int64(a), DataType::Float64(b)) => (*a as f64) < *b,
                            (DataType::Float64(a), DataType::Int64(b)) => *a < (*b as f64),
                            (DataType::String(a), DataType::String(b)) => a < b,
                            _ => false,
                        };
                        if cmp { max = item.clone(); }
                    }
                    Ok(Some(max))
                }
                "join" => {
                    let separator = if !args.is_empty() {
                        match self.eval_expr(&args[0])? {
                            DataType::String(s) => s,
                            other => other.to_string_lossy(),
                        }
                    } else {
                        ",".to_string()
                    };
                    let parts: Vec<String> = arr.iter().map(|v| v.to_string_lossy()).collect();
                    let estimated_len: usize = parts.iter().map(|p| p.len()).sum::<usize>() + separator.len().saturating_mul(parts.len().saturating_sub(1));
                    if estimated_len > MAX_STRING_OUTPUT {
                        return Err(InterpError::TypeError { expected: format!("join result at most {} bytes", MAX_STRING_OUTPUT), actual: format!("{}", estimated_len), context: "array join".to_string(), span });
                    }
                    Ok(Some(DataType::String(parts.join(&separator))))
                }
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

        // Check arity (accounting for default parameters and rest params)
        let has_rest = func.params.last().map_or(false, |p| p.rest);
        let required = func.params.iter().filter(|p| p.default.is_none() && !p.rest).count();
        let max_positional = if has_rest { usize::MAX } else { func.params.len() };
        if args.len() < required || args.len() > max_positional {
            return Err(InterpError::ArityMismatch {
                name: name.to_string(),
                expected: func.params.len(),
                actual: args.len(),
                span: call_span,
            });
        }

        // Pre-evaluate default parameter values in the CALLER scope
        // (default expressions may reference caller-scope variables)
        let mut resolved_args: Vec<(String, DataType, bool)> = Vec::new();
        for (i, param) in func.params.iter().enumerate() {
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

        // Save outer symbol table
        let saved_symbols = std::mem::replace(&mut self.symbols, vec![HashMap::new()]);
        self.saved_symbol_stacks.push(saved_symbols);
        self.heap.push_scope();
        self.call_depth += 1;

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

        // Pop call stack for debugger
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
        let mut last = DataType::Null;
        for stmt in &block.statements {
            last = self.exec_statement(stmt)?;
        }
        if let Some(tail) = &block.tail_expr {
            last = self.eval_expr(tail)?;
        }
        Ok(last)
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
                            DataType::Array(arr) => items.extend(arr),
                            other => {
                                return Err(InterpError::TypeError {
                                    expected: "Array".to_string(),
                                    actual: datatype_type_name(&other).to_string(),
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
                let mut map = std::collections::BTreeMap::new();
                for (key, value_expr) in entries {
                    map.insert(key.clone(), self.eval_expr(value_expr)?);
                }
                Ok(DataType::Map(map))
            }
        }
    }

    fn eval_expr(&mut self, expr: &Expression) -> Result<DataType, InterpError> {
        match &expr.kind {
            ExpressionKind::Literal(lit) => self.eval_literal(lit),

            ExpressionKind::Variable(name) => {
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
                // Short-circuit evaluation for logical operators
                if *op == BinOp::And {
                    let lhs = self.eval_expr(left)?;
                    return match lhs {
                        DataType::Bool(false) => Ok(DataType::Bool(false)),
                        DataType::Bool(true) => self.eval_expr(right),
                        other => Err(InterpError::TypeError {
                            expected: "Bool".to_string(),
                            actual: datatype_type_name(&other).to_string(),
                            context: "left side of &&".to_string(),
                            span: left.span,
                        }),
                    };
                }
                if *op == BinOp::Or {
                    let lhs = self.eval_expr(left)?;
                    return match lhs {
                        DataType::Bool(true) => Ok(DataType::Bool(true)),
                        DataType::Bool(false) => self.eval_expr(right),
                        other => Err(InterpError::TypeError {
                            expected: "Bool".to_string(),
                            actual: datatype_type_name(&other).to_string(),
                            context: "left side of ||".to_string(),
                            span: left.span,
                        }),
                    };
                }

                let lhs = self.eval_expr(left)?;
                let rhs = self.eval_expr(right)?;

                let op_type = OperationType::parse(op.operation_name()).ok_or_else(|| {
                    InterpError::UnknownOperation {
                        name: op.operation_name().to_string(),
                        span: expr.span,
                        suggestion: None,
                    }
                })?;

                let input_ports = op_input_ports(op_type);
                let inputs: HashMap<String, DataType> = input_ports.first()
                    .map(|p| (p.to_string(), lhs))
                    .into_iter()
                    .chain(input_ports.get(1).map(|p| (p.to_string(), rhs)))
                    .collect();

                self.evaluator.eval_operation(op_type, &inputs, &HashMap::new()).map_err(|e| {
                    InterpError::EvalError {
                        error: e,
                        span: expr.span,
                    }
                })
            }

            ExpressionKind::UnaryOp { op, operand } => {
                let val = self.eval_expr(operand)?;

                let op_type = OperationType::parse(op.operation_name()).ok_or_else(|| {
                    InterpError::UnknownOperation {
                        name: op.operation_name().to_string(),
                        span: expr.span,
                        suggestion: None,
                    }
                })?;

                let inputs = HashMap::from([("value".to_string(), val)]);

                self.evaluator.eval_operation(op_type, &inputs, &HashMap::new()).map_err(|e| {
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
                    let evaluated_args = self.eval_call_args(args)?;
                    return self.call_function(fn_name, &evaluated_args, expr.span);
                }

                // Check if it's a variable holding a function reference (lambda)
                if let Some(entry) = self.lookup(fn_name) {
                    let addr = entry.addr;
                    if let Some(DataType::String(ref_name)) = self.heap.read(addr).cloned() {
                        if self.functions.contains_key(ref_name.as_str()) {
                            let evaluated_args = self.eval_call_args(args)?;
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
                            return Ok(DataType::String(datatype_type_name(&val).to_string()));
                        }
                        return Ok(DataType::String("null".to_string()));
                    }
                    "len" => {
                        if let Some(arg) = args.first() {
                            let val = self.eval_expr(arg)?;
                            let length = match &val {
                                DataType::Array(a) => a.len() as i64,
                                DataType::String(s) => s.chars().count() as i64,
                                DataType::Map(m) => m.len() as i64,
                                DataType::Bytes(b) => b.len() as i64,
                                _ => {
                                    return Err(InterpError::TypeError {
                                        expected: "Array, String, Map, or Bytes".to_string(),
                                        actual: datatype_type_name(&val).to_string(),
                                        context: "len()".to_string(),
                                        span: expr.span,
                                    })
                                }
                            };
                            return Ok(DataType::Int64(length));
                        }
                        return Err(InterpError::ArityMismatch { name: "len".to_string(), expected: 1, actual: 0, span: expr.span });
                    }
                    "assert" => {
                        if let Some(arg) = args.first() {
                            let val = self.eval_expr(arg)?;
                            match &val {
                                DataType::Bool(true) => return Ok(DataType::Null),
                                DataType::Bool(false) => {
                                    let msg = if args.len() > 1 {
                                        let msg_val = self.eval_expr(&args[1])?;
                                        datatype_to_display(&msg_val)
                                    } else {
                                        "Assertion failed".to_string()
                                    };
                                    return Err(InterpError::ThrownError {
                                        value: DataType::String(msg),
                                        span: expr.span,
                                    });
                                }
                                other => {
                                    return Err(InterpError::TypeError {
                                        expected: "Bool".to_string(),
                                        actual: datatype_type_name(other).to_string(),
                                        context: "assert()".to_string(),
                                        span: expr.span,
                                    })
                                }
                            }
                        }
                        return Ok(DataType::Null);
                    }
                    "assert_eq" => {
                        if args.len() < 2 {
                            return Err(InterpError::ArityMismatch {
                                name: "assert_eq".to_string(),
                                expected: 2,
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
                        return Err(InterpError::ThrownError {
                            value: DataType::String(msg),
                            span: expr.span,
                        });
                    }
                    "assert_ne" => {
                        if args.len() < 2 {
                            return Err(InterpError::ArityMismatch {
                                name: "assert_ne".to_string(),
                                expected: 2,
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
                        return Err(InterpError::ThrownError {
                            value: DataType::String(msg),
                            span: expr.span,
                        });
                    }
                    "assert_throws" => {
                        if args.is_empty() {
                            return Err(InterpError::ArityMismatch {
                                name: "assert_throws".to_string(),
                                expected: 1,
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
                        // Try to call the function with no args
                        match self.call_function(&target_fn, &[], expr.span) {
                            Err(e) if !is_control_flow(&e) => {
                                // Good — it threw an error as expected
                                return Ok(DataType::Null);
                            }
                            Ok(_) => {
                                let msg = format!(
                                    "Assertion failed: expected '{}' to throw, but it returned successfully",
                                    target_fn
                                );
                                return Err(InterpError::ThrownError {
                                    value: DataType::String(msg),
                                    span: expr.span,
                                });
                            }
                            Err(e) => return Err(e),
                        }
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
                            actual: datatype_type_name(other).to_string(),
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
                    let s = self.eval_expr(rs)?;
                    let e = self.eval_expr(re)?;
                    return self.eval_slice(&obj, &s, &e, *inclusive, expr.span);
                }
                let obj = self.eval_expr(object)?;
                let idx = self.eval_expr(index)?;

                let inputs = HashMap::from([
                    ("array".to_string(), obj),
                    ("index".to_string(), idx),
                ]);

                self.evaluator.eval_operation(OperationType::ArrayGet, &inputs, &HashMap::new()).map_err(|e| {
                    InterpError::EvalError {
                        error: e,
                        span: expr.span,
                    }
                })
            }

            ExpressionKind::FieldAccess { object, field } => {
                let obj = self.eval_expr(object)?;

                let inputs = HashMap::from([
                    ("map".to_string(), obj),
                    ("key".to_string(), DataType::String(field.clone())),
                ]);

                self.evaluator.eval_operation(OperationType::MapGet, &inputs, &HashMap::new()).map_err(|e| {
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
                        FutureState::Pending => Err(InterpError::EvalError {
                            error: EvalError::InvalidInput(
                                "Cannot await a pending future in synchronous execution"
                                    .to_string(),
                            ),
                            span: expr.span,
                        }),
                    },
                    // Await on non-Future is identity
                    other => Ok(other),
                }
            }

            ExpressionKind::Spawn(inner) => {
                // In the synchronous interpreter, spawn is eager:
                // evaluate immediately and wrap in Future(Resolved),
                // or capture errors as Future(Rejected)
                match self.eval_expr(inner) {
                    Ok(val) if matches!(val, DataType::Future(_)) => {
                        Ok(val) // Already a Future (e.g. from async fn), don't double-wrap
                    }
                    Ok(val) => {
                        Ok(DataType::Future(Box::new(FutureState::Resolved(Box::new(val)))))
                    }
                    Err(e) if !is_control_flow(&e) => {
                        Ok(DataType::Future(Box::new(FutureState::Rejected(format!("{}", e)))))
                    }
                    Err(e) => Err(e), // control flow signals still propagate
                }
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
                            return Err(InterpError::TypeError {
                                expected: format!("range size at most {}", MAX_RANGE_SIZE),
                                actual: format!("range size {}", range_size),
                                context: "range creation".to_string(),
                                span: expr.span,
                            });
                        }
                        let arr: Vec<DataType> = (*a..end_v).map(DataType::Int64).collect();
                        Ok(DataType::Array(arr))
                    }
                    _ => {
                        // Fallback to evaluator for non-int ranges
                        let inputs = HashMap::from([
                            ("start".to_string(), start_val),
                            ("end".to_string(), end_val),
                        ]);
                        self.evaluator.eval_operation(OperationType::Range, &inputs, &HashMap::new()).map_err(|e| {
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
                    && matches!(object.kind, ExpressionKind::OptionalChain { .. })
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

                let op_type =
                    resolve_method(&obj, method).ok_or_else(|| InterpError::UnknownOperation {
                        name: format!("{}.{}", datatype_type_name(&obj), method),
                        span: expr.span,
                        suggestion: None,
                    })?;
                let input_ports = op_input_ports(op_type);
                let inputs: HashMap<String, DataType> = input_ports.first()
                    .map(|p| (p.to_string(), obj))
                    .into_iter()
                    .chain(args.iter().enumerate()
                        .map(|(i, arg)| {
                            let val = self.eval_expr(arg)?;
                            let port = if i + 1 < input_ports.len() {
                                input_ports[i + 1].to_string()
                            } else {
                                format!("input_{}", i + 1)
                            };
                            Ok((port, val))
                        })
                        .collect::<Result<Vec<_>, InterpError>>()?
                    )
                    .collect();
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
                self.lambda_counter += 1;
                // Capture current scope variables (by value)
                let captures: Vec<(String, DataType, bool)> = self.symbols.iter()
                    .flat_map(|scope| scope.iter())
                    .filter_map(|(var_name, entry)| {
                        self.heap.read(entry.addr)
                            .map(|val| (var_name.clone(), val.clone(), entry.mutable))
                    })
                    .collect();
                self.closure_captures.insert(name.clone(), captures);
                // Create function def from lambda body
                let func_body = Block {
                    statements: vec![],
                    tail_expr: Some(body.clone()),
                    span: body.span,
                };
                let func_def = FunctionDef {
                    name: name.clone(),
                    params: params.clone(),
                    return_type: None,
                    body: func_body,
                    span: expr.span,
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
                                    actual: datatype_type_name(&other).to_string(),
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
                let mut result = String::new();
                for part in parts {
                    match part {
                        StringPart::Literal(s) => result.push_str(s),
                        StringPart::Expr(e) => {
                            let val = self.eval_expr(e)?;
                            result.push_str(&datatype_to_display(&val));
                        }
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
                    self.evaluator.eval_operation(OperationType::MapGet, &inputs, &HashMap::new()).map_err(|e| {
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
                                let mut entry = std::collections::BTreeMap::new();
                                entry.insert("key".to_string(), DataType::String(k));
                                entry.insert("value".to_string(), v);
                                DataType::Map(entry)
                            })
                            .collect()
                    }
                    DataType::String(s) => {
                        s.chars()
                            .map(|c| DataType::String(c.to_string()))
                            .collect()
                    }
                    other => return Err(InterpError::TypeError {
                        expected: "Array, Map, or String".to_string(),
                        actual: datatype_type_name(&other).to_string(),
                        context: "list comprehension".to_string(),
                        span: iterable.span,
                    }),
                };
                let mut result = Vec::new();
                for item in items {
                    if self.is_cancelled() {
                        return Err(InterpError::Cancelled);
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
                                let mut entry = std::collections::BTreeMap::new();
                                entry.insert("key".to_string(), DataType::String(k));
                                entry.insert("value".to_string(), v);
                                DataType::Map(entry)
                            })
                            .collect()
                    }
                    DataType::String(s) => {
                        s.chars()
                            .map(|c| DataType::String(c.to_string()))
                            .collect()
                    }
                    other => return Err(InterpError::TypeError {
                        expected: "Array, Map, or String".to_string(),
                        actual: datatype_type_name(&other).to_string(),
                        context: "map comprehension".to_string(),
                        span: iterable.span,
                    }),
                };
                let mut result = std::collections::BTreeMap::new();
                for item in items {
                    if self.is_cancelled() {
                        return Err(InterpError::Cancelled);
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
                        let evaluated_args: Vec<DataType> = args.iter()
                            .map(|a| self.eval_expr(a))
                            .collect::<Result<_, _>>()?;
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
                        expected: variant_def.fields.len(),
                        actual: args.len(),
                        span: expr.span,
                    });
                }
                let evaluated_args: Vec<DataType> = args.iter()
                    .map(|a| self.eval_expr(a))
                    .collect::<Result<_, _>>()?;
                let mut map = std::collections::BTreeMap::new();
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
                let mut map = std::collections::BTreeMap::new();
                map.insert("__struct".to_string(), DataType::String(name.clone()));
                for (field_name, field_expr) in fields {
                    if map.contains_key(field_name) && field_name != "__struct" {
                        return Err(InterpError::TypeError {
                            expected: format!("unique field in struct '{}'", name),
                            actual: format!("duplicate field '{}'", field_name),
                            context: format!("struct '{}' construction", name),
                            span: expr.span,
                        });
                    }
                    let val = self.eval_expr(field_expr)?;
                    map.insert(field_name.clone(), val);
                }
                // Validate all required fields are present
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
                    if field_name != "__struct" && !known_fields.contains(&field_name.as_str()) {
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
                        // Check if it's an error-like enum (Result::Err)
                        if let DataType::Map(ref m) = val {
                            if m.get("__variant").map(|v| v.to_string_lossy()) == Some("Err".to_string()) {
                                // Extract the error data for the thrown value
                                let error_val = m.get("__data")
                                    .and_then(|d| if let DataType::Array(arr) = d { arr.first().cloned() } else { None })
                                    .unwrap_or(val.clone());
                                return Err(InterpError::ThrownError {
                                    value: error_val,
                                    span,
                                });
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

            ExpressionKind::Loop(block) => {
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
                        Err(InterpError::ContinueSignal) => {}
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
            } => {
                let try_result = self.exec_block(try_block);
                match try_result {
                    Ok(val) => Ok(val),
                    Err(ref e) if is_control_flow(e) => try_result,
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
                }
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
                    let val = evaluated_args.first().unwrap_or(&piped_value);
                    let length = match val {
                        DataType::Array(a) => a.len() as i64,
                        DataType::String(s) => s.chars().count() as i64,
                        DataType::Map(m) => m.len() as i64,
                        DataType::Bytes(b) => b.len() as i64,
                        other => {
                            return Err(InterpError::TypeError {
                                expected: "Array, String, Map, or Bytes".to_string(),
                                actual: datatype_type_name(other).to_string(),
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
                    if let Some(val) = evaluated_args.first() {
                        return Ok(DataType::String(datatype_type_name(val).to_string()));
                    }
                    return Ok(DataType::String("null".to_string()));
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
                    if let Some(val) = evaluated_args.first() {
                        self.logs.push(LogEntry {
                            level: if fn_name == "debug_log" { LogLevel::Debug } else { LogLevel::Info },
                            message: datatype_to_display(val),
                            line: Some(stage.span.start_line),
                            node_id: None,
                        });
                        return Ok(val.clone());
                    }
                    return Ok(DataType::Null);
                }
                if self.functions.contains_key(fn_name.as_str()) {
                    let evaluated_args: Vec<DataType> = args.iter()
                        .map(|arg| {
                            if matches!(arg.kind, ExpressionKind::Placeholder) {
                                Ok(piped_value.clone())
                            } else {
                                self.eval_expr(arg)
                            }
                        })
                        .collect::<Result<_, _>>()?;
                    return self.call_function(fn_name, &evaluated_args, stage.span);
                }

                // Check if it's a variable holding a function reference (lambda)
                if let Some(entry) = self.lookup(fn_name) {
                    let addr = entry.addr;
                    if let Some(DataType::String(ref_name)) = self.heap.read(addr).cloned() {
                        if self.functions.contains_key(ref_name.as_str()) {
                            let evaluated_args: Vec<DataType> = args.iter()
                                .map(|arg| {
                                    if matches!(arg.kind, ExpressionKind::Placeholder) {
                                        Ok(piped_value.clone())
                                    } else {
                                        self.eval_expr(arg)
                                    }
                                })
                                .collect::<Result<_, _>>()?;
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

                let inputs: HashMap<String, DataType> = args.iter().enumerate()
                    .map(|(i, arg)| {
                        let port = if i < input_ports.len() {
                            input_ports[i].to_string()
                        } else {
                            format!("input_{}", i)
                        };
                        let val = if matches!(arg.kind, ExpressionKind::Placeholder) {
                            piped_value.clone()
                        } else {
                            self.eval_expr(arg)?
                        };
                        Ok((port, val))
                    })
                    .collect::<Result<_, InterpError>>()?;

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
                            actual: datatype_type_name(value).to_string(),
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
                            actual: datatype_type_name(value).to_string(),
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
            let available_modules = [
                "math",
                "cmp",
                "logic",
                "bits",
                "str",
                "convert",
                "array",
                "map",
                "bytes",
                "json",
                "time",
                "hash",
                "io",
                "control",
                "rand",
                "fs",
                "env",
                "net",
                "tcp",
                "udp",
                "ws",
                "sse",
                "http_server",
                "path",
                "yaml",
                "csv",
                "toml",
                "regex",
                "uuid",
                "crypto",
                "compress",
                "fmt",
                "stats",
                "text",
                "encode",
                "reflect",
                "collections",
                "sort",
                "cert",
            ];
            let suggestion = super::errors::suggest_name(module, &available_modules);
            return Err(InterpError::UnknownOperation {
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
            return Err(InterpError::TypeError {
                expected: "non-circular import".to_string(),
                actual: format!("circular import of pkg::{}", package_id),
                context: "package import".to_string(),
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
                let available: Vec<&str> = self.packages.keys().map(|s| s.as_str()).collect();
                let suggestion = super::errors::suggest_name(package_id, &available);
                return Err(InterpError::UnknownOperation {
                    name: format!("pkg::{}", package_id),
                    span,
                    suggestion,
                });
            }
        };

        // Execute the package's own use statements (e.g. `use std::array`)
        // so that std aliases are available when package functions run
        for use_stmt in &pkg.use_statements {
            if let Err(e) = self.exec_statement(use_stmt) {
                tracing::debug!("Package use-statement failed: {}", e);
            }
        }

        if glob || path.len() == 2 {
            // `use pkg::collections::*` or `use pkg::collections` — import all exports
            for (name, func) in &pkg.functions {
                self.functions.insert(name.clone(), func.clone());
            }
        } else if path.len() >= 3 {
            // `use pkg::collections::sorted_unique` or with alias
            let func_name = &path[2];
            if let Some(func) = pkg.functions.get(func_name) {
                let local_name = alias.unwrap_or(func_name.as_str());
                self.functions.insert(local_name.to_string(), func.clone());
            } else {
                self.importing_packages.remove(package_id);
                let available: Vec<&str> = pkg.functions.keys().map(|s| s.as_str()).collect();
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
                    | InterpError::ThrownError { span, .. }
                    | InterpError::InvalidPlaceholder { span }
                    | InterpError::InvalidPipeStage { span } => (span.start_line, span.start_col),
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
                StatementKind::EnumDef { name, variants } => {
                    self.enum_defs.insert(name.clone(), variants.clone());
                }
                StatementKind::StructDef { name, fields } => {
                    self.struct_defs.insert(name.clone(), fields.clone());
                }
                StatementKind::ModuleDef { name, body } => {
                    for inner in &body.statements {
                        match &inner.kind {
                            StatementKind::FunctionDef(def) => {
                                let qualified = format!("{}::{}", name, def.name);
                                self.functions.insert(qualified, def.clone());
                            }
                            StatementKind::AsyncFunctionDef(def) => {
                                let qualified = format!("{}::{}", name, def.name);
                                self.async_fns.insert(qualified.clone());
                                self.functions.insert(qualified, def.clone());
                            }
                            _ => {}
                        }
                    }
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
                self.symbols.push(HashMap::new());
                self.heap.push_scope();

                let test_result = self.exec_block(body);

                // Restore all state
                self.heap.pop_scope();
                while self.symbols.len() > saved_symbols_len {
                    self.symbols.pop();
                }
                self.functions = saved_functions;
                self.std_op_aliases = saved_aliases;
                self.enum_defs = saved_enums;
                self.struct_defs = saved_structs;
                self.closure_captures = saved_closures;
                self.saved_symbol_stacks = saved_stacks;

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
            "regex_match",
            "regex_replace",
            "regex_extract",
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
            "sort_reverse",
        ],
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
            | InterpError::ReturnSignal(_)
            | InterpError::Cancelled
    )
}

/// Convert a DataType to a human-readable display string (for interpolation/print).
fn datatype_to_display(val: &DataType) -> String {
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
            let items: Vec<String> = arr.iter().map(datatype_to_display).collect();
            format!("[{}]", items.join(", "))
        }
        DataType::Map(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", k, datatype_to_display(v)))
                .collect();
            format!("{{{}}}", entries.join(", "))
        }
        DataType::Future(_) => "<future>".to_string(),
    }
}

/// Get the type name of a DataType value.
fn datatype_type_name(val: &DataType) -> &'static str {
    match val {
        DataType::Null => "null",
        DataType::Bool(_) => "bool",
        DataType::Int32(_) => "int32",
        DataType::Int64(_) => "int64",
        DataType::Uint32(_) => "uint32",
        DataType::Uint64(_) => "uint64",
        DataType::Float32(_) => "float32",
        DataType::Float64(_) => "float64",
        DataType::String(_) => "string",
        DataType::Bytes(_) => "bytes",
        DataType::Array(_) => "array",
        DataType::Map(_) => "map",
        DataType::Future(_) => "future",
    }
}

/// Resolve a method call to an OperationType based on receiver type and method name.
fn resolve_method(obj: &DataType, method: &str) -> Option<OperationType> {
    match obj {
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
        // Generic methods that work on any type
        _ => match method {
            "to_string" => Some(OperationType::ToString),
            "to_int64" => Some(OperationType::ToInt64),
            "to_float64" => Some(OperationType::ToFloat64),
            "to_bool" => Some(OperationType::ToBool),
            "to_json" => Some(OperationType::ToJson),
            _ => None,
        },
    }
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
                (Literal::Float64(a), DataType::Float64(b)) => a.to_bits() == b.to_bits(),
                (Literal::Float64(a), DataType::Int64(b)) => *a == (*b as f64),
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
            let actual_type = datatype_type_name(value);
            if actual_type == type_name {
                Some(vec![(name.clone(), value.clone())])
            } else {
                None
            }
        }
        Pattern::RangePattern { start, end, inclusive } => {
            // Extract literal values from expressions for range comparison
            let start_val = match &start.kind {
                ExpressionKind::Literal(Literal::Int64(v)) => *v,
                _ => return None,
            };
            let end_val = match &end.kind {
                ExpressionKind::Literal(Literal::Int64(v)) => *v,
                _ => return None,
            };
            let val = match value {
                DataType::Int64(v) => *v,
                _ => return None,
            };
            let in_range = if *inclusive {
                val >= start_val && val <= end_val
            } else {
                val >= start_val && val < end_val
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
        expected: usize,
        actual: usize,
        span: Span,
    },
    /// Control flow signal: `break` (caught by loops, not a real error)
    BreakSignal(DataType),
    /// Control flow signal: `continue` (caught by loops)
    ContinueSignal,
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
                write!(f, "{} [E406]: {}", span, error)
            }
            InterpError::MaxIterations { limit, span } => {
                write!(
                    f,
                    "{} [E400]: Loop exceeded maximum iterations ({})",
                    span, limit
                )
            }
            InterpError::Cancelled => write!(f, "[E407] Execution cancelled"),
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
                    "{} [E405]: Function '{}' expects {} arguments, got {}",
                    span, name, expected, actual
                )
            }
            InterpError::BreakSignal(_) => write!(f, "break"),
            InterpError::ContinueSignal => write!(f, "continue"),
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
            InterpError::NotImplemented { message, span } => {
                write!(f, "{} [E408]: {}", span, message)
            }
            InterpError::ThrownError { value, span } => {
                write!(f, "{} [E403]: Uncaught error: {}", span, datatype_to_display(value))
            }
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

    for stmt in &program.statements {
        match &stmt.kind {
            StatementKind::FunctionDef(def) | StatementKind::AsyncFunctionDef(def) => {
                functions.insert(def.name.clone(), def.clone());
            }
            StatementKind::Use { .. } => {
                use_statements.push(stmt.clone());
            }
            _ => {}
        }
    }

    Ok(ResolvedPackage {
        id: id.to_string(),
        functions,
        use_statements,
    })
}
