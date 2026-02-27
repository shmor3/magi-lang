//! AST-level type checker for the MAGI v2 language.
//!
//! Works directly on the parsed AST (before lowering to `GraphDef`), enabling
//! richer diagnostics than the `abstract_interp.rs` graph-level analysis:
//!
//! - Mutability tracking (`let` vs `let mut`)
//! - Use-before-define detection
//! - Assignment to immutable variable detection
//! - Span-based diagnostics (line/column)
//! - Unused variable and unused import warnings

use super::ast::*;
use super::lexer::is_reserved_keyword;
use crate::ops::{op_input_types, op_output_type};
use crate::types::{ChannelType, OperationType};
use std::collections::{HashMap, HashSet};

pub use crate::eval::DiagnosticSeverity;

// =============================================================================
// Public types
// =============================================================================

/// Result of AST-level type analysis.
#[derive(Debug, Clone)]
pub struct AstTypeAnalysis {
    /// Diagnostics with precise source locations.
    pub diagnostics: Vec<AstDiagnostic>,
    /// Inferred types for named variables (top-level scope).
    pub variable_types: HashMap<String, ChannelType>,
}

/// A diagnostic anchored to a source location (line/column).
#[derive(Debug, Clone)]
pub struct AstDiagnostic {
    pub line: u32,
    pub column: u32,
    pub message: String,
    pub severity: DiagnosticSeverity,
    /// Stable error code (e.g., "E200", "W100").
    pub code: Option<String>,
    /// Extended help text explaining the error and how to fix it.
    pub help: Option<String>,
    /// "Did you mean?" suggestion for typos.
    pub suggestion: Option<String>,
}

// =============================================================================
// Internal: function signature tracking
// =============================================================================

/// Tracked signature for a user-defined function.
#[derive(Clone)]
struct FunctionSig {
    params: Vec<(String, ChannelType)>,
    /// Number of required parameters (those without default values).
    required_params: usize,
    return_type: ChannelType,
    def_line: u32,
    used: bool,
}

// =============================================================================
// Internal: variable tracking
// =============================================================================

/// Metadata tracked per variable binding.
struct VarInfo {
    channel_type: ChannelType,
    mutable: bool,
    used: bool,
    mutated: bool,
    is_param: bool,
    def_line: u32,
    def_col: u32,
}

// =============================================================================
// Entry point
// =============================================================================

/// Type-check a parsed AST program.
///
/// `imports` is the set of plugin IDs brought into scope by `import` statements;
/// calls matching an import name are treated as plugin invocations with an
/// unknown (Null) return type.
pub fn check_types(program: &Program, imports: &HashSet<String>) -> AstTypeAnalysis {
    let mut checker = TypeChecker::new(imports);
    checker.check_program(program);
    checker.finalize()
}

// =============================================================================
// TypeChecker
// =============================================================================

struct TypeChecker {
    /// Scope stack — innermost scope is last.
    env: Vec<HashMap<String, VarInfo>>,
    diagnostics: Vec<AstDiagnostic>,
    /// Known plugin imports.
    imports: HashSet<String>,
    /// Subset of `imports` that have actually been referenced.
    used_imports: HashSet<String>,
    /// User-defined function signatures.
    function_sigs: HashMap<String, FunctionSig>,
    /// Depth of pipe expression nesting (for placeholder validation).
    pipe_depth: usize,
    /// Depth of loop nesting (for break/continue validation).
    loop_depth: usize,
    /// Depth of function nesting (for return validation).
    function_depth: usize,
    /// Declared return type of the current function (for return statement validation).
    current_return_type: ChannelType,
    /// Known enum definitions: enum_name → list of variant names.
    enum_variants: HashMap<String, Vec<String>>,
}

impl TypeChecker {
    fn new(imports: &HashSet<String>) -> Self {
        Self {
            env: vec![HashMap::new()], // start with one global scope
            diagnostics: Vec::new(),
            imports: imports.clone(),
            used_imports: HashSet::new(),
            function_sigs: HashMap::new(),
            enum_variants: HashMap::new(),
            pipe_depth: 0,
            loop_depth: 0,
            function_depth: 0,
            current_return_type: ChannelType::Null,
        }
    }

    // =========================================================================
    // Scope management
    // =========================================================================

    fn push_scope(&mut self) {
        self.env.push(HashMap::new());
    }

    /// Pop the current scope, emitting warnings for any unused variables
    /// (except those whose name starts with `_`).
    fn pop_scope(&mut self) {
        if let Some(scope) = self.env.pop() {
            for (name, info) in &scope {
                if name.starts_with('_') {
                    continue;
                }
                if !info.used {
                    let code = if info.is_param {
                        super::errors::ErrorCode::W109
                    } else {
                        super::errors::ErrorCode::W100
                    };
                    self.diagnostics.push(AstDiagnostic {
                        line: info.def_line,
                        column: info.def_col,
                        message: if info.is_param {
                            format!("Unused parameter '{}'", name)
                        } else {
                            format!("Unused variable '{}'", name)
                        },
                        severity: DiagnosticSeverity::Warning,
                        code: Some(code.to_string()),
                        help: Some(code.help().to_string()),
                        suggestion: None,
                    });
                } else if info.mutable && !info.mutated {
                    let code = super::errors::ErrorCode::W110;
                    self.diagnostics.push(AstDiagnostic {
                        line: info.def_line,
                        column: info.def_col,
                        message: format!("Variable '{}' declared as mutable but never reassigned", name),
                        severity: DiagnosticSeverity::Warning,
                        code: Some(code.to_string()),
                        help: Some(code.help().to_string()),
                        suggestion: None,
                    });
                }
            }
        }
    }

    // =========================================================================
    // Variable lookup
    // =========================================================================

    /// Look up a variable by name, searching from innermost scope outward.
    fn lookup(&self, name: &str) -> Option<&VarInfo> {
        for scope in self.env.iter().rev() {
            if let Some(info) = scope.get(name) {
                return Some(info);
            }
        }
        None
    }

    /// Mutable lookup so we can set the `used` flag.
    fn lookup_mut(&mut self, name: &str) -> Option<&mut VarInfo> {
        for scope in self.env.iter_mut().rev() {
            if let Some(info) = scope.get_mut(name) {
                return Some(info);
            }
        }
        None
    }

    /// Collect all variable names visible in the current scope stack.
    fn available_variable_names(&self) -> Vec<String> {
        let mut names = HashSet::new();
        for scope in self.env.iter().rev() {
            for key in scope.keys() {
                names.insert(key.clone());
            }
        }
        names.into_iter().collect()
    }

    /// Suggest a variable name using Levenshtein distance.
    fn suggest_variable(&self, name: &str) -> Option<String> {
        let names = self.available_variable_names();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        super::errors::suggest_name(name, &refs)
    }

    /// Define a variable in the current (innermost) scope.
    fn define_var(&mut self, name: &str, ct: ChannelType, mutable: bool, line: u32, col: u32) {
        if is_reserved_keyword(name) {
            self.emit_coded(
                line,
                col,
                format!("'{}' is a reserved keyword", name),
                DiagnosticSeverity::Warning,
                super::errors::ErrorCode::E200,
                None,
            );
        }
        // W102: Variable shadowing within the same scope.
        if let Some(scope) = self.env.last() {
            if let Some(prev) = scope.get(name) {
                self.emit_coded(
                    line,
                    col,
                    format!(
                        "Variable '{}' shadows previous definition on line {}",
                        name, prev.def_line
                    ),
                    DiagnosticSeverity::Warning,
                    super::errors::ErrorCode::W102,
                    None,
                );
            }
        }
        if let Some(scope) = self.env.last_mut() {
            scope.insert(
                name.to_string(),
                VarInfo {
                    channel_type: ct,
                    mutable,
                    used: false,
                    mutated: false,
                    is_param: false,
                    def_line: line,
                    def_col: col,
                },
            );
        }
    }

    // =========================================================================
    // Diagnostics
    // =========================================================================

    fn emit_coded(
        &mut self,
        line: u32,
        col: u32,
        msg: String,
        severity: DiagnosticSeverity,
        error_code: super::errors::ErrorCode,
        suggestion: Option<String>,
    ) {
        self.diagnostics.push(AstDiagnostic {
            line,
            column: col,
            message: msg,
            severity,
            code: Some(error_code.to_string()),
            help: Some(error_code.help().to_string()),
            suggestion,
        });
    }

    // =========================================================================
    // Program / statement checking
    // =========================================================================

    fn check_program(&mut self, program: &Program) {
        // Pass 1: collect function signatures and enum definitions
        for stmt in &program.statements {
            if let StatementKind::EnumDef { name, variants } = &stmt.kind {
                let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
                self.enum_variants.insert(name.clone(), variant_names);
            }
            if let StatementKind::FunctionDef(def) | StatementKind::AsyncFunctionDef(def) =
                &stmt.kind
            {
                let params: Vec<(String, ChannelType)> = def
                    .params
                    .iter()
                    .map(|p| {
                        let ct = p
                            .type_annotation
                            .as_deref()
                            .and_then(ChannelType::parse)
                            .unwrap_or(ChannelType::Null);
                        (p.name.clone(), ct)
                    })
                    .collect();
                let required_params = def.params.iter().filter(|p| p.default.is_none()).count();
                let return_type = def
                    .return_type
                    .as_deref()
                    .and_then(ChannelType::parse)
                    .unwrap_or(ChannelType::Null);
                self.function_sigs.insert(
                    def.name.clone(),
                    FunctionSig {
                        params,
                        required_params,
                        return_type,
                        def_line: stmt.span.start_line,
                        used: false,
                    },
                );
            }
        }

        // Pass 2: check all statements
        for stmt in &program.statements {
            self.check_statement(stmt);
        }
    }

    fn check_statement(&mut self, stmt: &Statement) {
        match &stmt.kind {
            // -----------------------------------------------------------------
            // import "plugin-id";
            // -----------------------------------------------------------------
            StatementKind::Import(_id) => {
                // Imports are already recorded in the `imports` set passed at
                // construction — nothing to type-check here.
            }

            // -----------------------------------------------------------------
            // let name = expr;  /  let name: type = expr;
            // -----------------------------------------------------------------
            StatementKind::Let {
                name,
                type_annotation,
                value,
            } => {
                let inferred = self.infer_expr(value);
                let ct = self.reconcile_annotation(
                    type_annotation.as_deref(),
                    inferred,
                    stmt.span.start_line,
                    stmt.span.start_col,
                    name,
                );
                self.define_var(name, ct, false, stmt.span.start_line, stmt.span.start_col);
            }

            // -----------------------------------------------------------------
            // let mut name = expr;  /  let mut name: type = expr;
            // -----------------------------------------------------------------
            StatementKind::LetMut {
                name,
                type_annotation,
                value,
            } => {
                let inferred = self.infer_expr(value);
                let ct = self.reconcile_annotation(
                    type_annotation.as_deref(),
                    inferred,
                    stmt.span.start_line,
                    stmt.span.start_col,
                    name,
                );
                self.define_var(name, ct, true, stmt.span.start_line, stmt.span.start_col);
            }

            // -----------------------------------------------------------------
            // name = expr;
            // -----------------------------------------------------------------
            StatementKind::Assignment { name, value } => {
                let new_type = self.infer_expr(value);

                // Check existence first.
                let (exists, is_mutable) = match self.lookup(name) {
                    Some(info) => (true, info.mutable),
                    None => (false, false),
                };

                if !exists {
                    let suggestion = self.suggest_variable(name);
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("Undefined variable '{}'", name),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E200,
                        suggestion,
                    );
                } else if !is_mutable {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("Cannot assign to immutable variable '{}'", name),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E404,
                        None,
                    );
                }

                // Update the variable's type and mark used + mutated.
                if let Some(info) = self.lookup_mut(name) {
                    info.channel_type = new_type;
                    info.used = true;
                    info.mutated = true;
                }
            }

            // -----------------------------------------------------------------
            // for item in iterable { body }
            // -----------------------------------------------------------------
            StatementKind::ForLoop {
                pattern,
                iterable,
                body,
            } => {
                let iter_type = self.infer_expr(iterable);
                if iter_type != ChannelType::Array && iter_type != ChannelType::Null {
                    self.emit_coded(
                        iterable.span.start_line,
                        iterable.span.start_col,
                        format!(
                            "For-loop iterable should be array, got {}",
                            iter_type.as_str()
                        ),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E102,
                        None,
                    );
                }

                if is_empty_block(body) {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        "Empty loop body".to_string(),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::W104,
                        None,
                    );
                }

                self.push_scope();
                self.loop_depth += 1;
                match pattern {
                    ForPattern::Single(variable) => {
                        self.define_var(
                            variable,
                            ChannelType::Null,
                            false,
                            stmt.span.start_line,
                            stmt.span.start_col,
                        );
                    }
                    ForPattern::ArrayDestructure(elements) => {
                        for elem in elements {
                            let name = match elem {
                                DestructureElement::Name(n) => n,
                                DestructureElement::Rest(n) => n,
                            };
                            self.define_var(name, ChannelType::Null, false, stmt.span.start_line, stmt.span.start_col);
                        }
                    }
                    ForPattern::MapDestructure(entries) => {
                        for (key, alias) in entries {
                            let name = alias.as_ref().unwrap_or(key);
                            self.define_var(name, ChannelType::Null, false, stmt.span.start_line, stmt.span.start_col);
                        }
                    }
                }
                self.check_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
            }

            // -----------------------------------------------------------------
            // while condition { body }
            // -----------------------------------------------------------------
            StatementKind::WhileLoop { condition, body } => {
                let cond_type = self.infer_expr(condition);
                if cond_type != ChannelType::Bool && cond_type != ChannelType::Null {
                    self.emit_coded(
                        condition.span.start_line,
                        condition.span.start_col,
                        format!("While condition should be bool, got {}", cond_type.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E101,
                        None,
                    );
                }

                // W105: Infinite while loop (literal true condition).
                if matches!(
                    &condition.kind,
                    ExpressionKind::Literal(Literal::Bool(true))
                ) {
                    self.emit_coded(
                        condition.span.start_line,
                        condition.span.start_col,
                        "Loop condition is always true".to_string(),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::W105,
                        None,
                    );
                }

                // W104: Empty loop body.
                if is_empty_block(body) {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        "Empty loop body".to_string(),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::W104,
                        None,
                    );
                }

                self.push_scope();
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
            }

            // -----------------------------------------------------------------
            // output expr;
            // -----------------------------------------------------------------
            StatementKind::Output(expr) => {
                let _ = self.infer_expr(expr);
            }

            // -----------------------------------------------------------------
            // expr;
            // -----------------------------------------------------------------
            StatementKind::ExprStatement(expr) => {
                let _ = self.infer_expr(expr);
            }

            // -----------------------------------------------------------------
            // fn name(params) -> type { body }
            // -----------------------------------------------------------------
            StatementKind::FunctionDef(def) | StatementKind::AsyncFunctionDef(def) => {
                self.push_scope();
                self.function_depth += 1;
                let saved_loop_depth = self.loop_depth;
                self.loop_depth = 0;
                let prev_return_type = std::mem::replace(
                    &mut self.current_return_type,
                    def.return_type.as_deref()
                        .and_then(ChannelType::parse)
                        .unwrap_or(ChannelType::Null),
                );
                // Define params as immutable variables
                for param in &def.params {
                    let ct = param
                        .type_annotation
                        .as_deref()
                        .and_then(ChannelType::parse)
                        .unwrap_or(ChannelType::Null);
                    // Type-check default param expression if present
                    if let Some(default_expr) = &param.default {
                        let default_type = self.infer_expr(default_expr);
                        if ct != ChannelType::Null
                            && default_type != ChannelType::Null
                            && !default_type.is_compatible_with(&ct)
                        {
                            let code = super::errors::ErrorCode::W106;
                            self.diagnostics.push(AstDiagnostic {
                                line: default_expr.span.start_line,
                                column: default_expr.span.start_col,
                                message: format!(
                                    "default value type '{}' doesn't match parameter type '{}'",
                                    default_type, ct
                                ),
                                severity: DiagnosticSeverity::Warning,
                                code: Some(code.to_string()),
                                help: Some(code.help().to_string()),
                                suggestion: None,
                            });
                        }
                    }
                    self.define_var(
                        &param.name,
                        ct,
                        false,
                        param.span.start_line,
                        param.span.start_col,
                    );
                    if let Some(info) = self.lookup_mut(&param.name) {
                        info.is_param = true;
                    }
                }
                // Infer body type and validate against declared return type
                let body_type = self.infer_block_no_scope(&def.body);
                let declared_return = def
                    .return_type
                    .as_deref()
                    .and_then(ChannelType::parse)
                    .unwrap_or(ChannelType::Null);
                if declared_return != ChannelType::Null
                    && body_type != ChannelType::Null
                    && !body_type.is_compatible_with(&declared_return)
                {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!(
                            "Function '{}' declares return type '{}' but body evaluates to '{}'",
                            def.name,
                            declared_return.as_str(),
                            body_type.as_str(),
                        ),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E100,
                        None,
                    );
                }
                // W108: unnecessary return in tail position
                if let Some(last_stmt) = def.body.statements.last() {
                    if matches!(&last_stmt.kind, StatementKind::Return(Some(_))) && def.body.tail_expr.is_none() {
                        self.emit_coded(
                            last_stmt.span.start_line,
                            last_stmt.span.start_col,
                            "unnecessary `return` in tail position".to_string(),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::W108,
                            Some("Remove `return` and use the expression as a tail expression".to_string()),
                        );
                    }
                }
                self.function_depth -= 1;
                self.loop_depth = saved_loop_depth;
                self.current_return_type = prev_return_type;
                self.pop_scope();
            }

            StatementKind::Break(ref val_expr) => {
                if self.loop_depth == 0 {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        "'break' used outside of a loop".to_string(),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E300,
                        None,
                    );
                }
                if let Some(expr) = val_expr {
                    let _ = self.infer_expr(expr);
                }
            }

            StatementKind::Continue => {
                if self.loop_depth == 0 {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        "'continue' used outside of a loop".to_string(),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E301,
                        None,
                    );
                }
            }

            StatementKind::Return(ref val_expr) => {
                if self.function_depth == 0 {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        "'return' used outside of a function".to_string(),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E302,
                        None,
                    );
                }
                if let Some(expr) = val_expr {
                    let ret_type = self.infer_expr(expr);
                    // Validate return value against declared return type
                    if self.current_return_type != ChannelType::Null
                        && ret_type != ChannelType::Null
                        && !ret_type.is_compatible_with(&self.current_return_type)
                    {
                        self.emit_coded(
                            stmt.span.start_line,
                            stmt.span.start_col,
                            format!(
                                "return type mismatch: expected '{}' but returning '{}'",
                                self.current_return_type.as_str(),
                                ret_type.as_str(),
                            ),
                            DiagnosticSeverity::Error,
                            super::errors::ErrorCode::E100,
                            None,
                        );
                    }
                }
            }

            // -----------------------------------------------------------------
            // let [a, b] = expr; / let {x, y} = expr;
            // -----------------------------------------------------------------
            StatementKind::LetDestructure {
                pattern,
                mutable,
                value,
            } => {
                let val_type = self.infer_expr(value);

                match pattern {
                    DestructurePattern::Array(elements) => {
                        if val_type != ChannelType::Array && val_type != ChannelType::Null {
                            self.emit_coded(
                                stmt.span.start_line,
                                stmt.span.start_col,
                                format!(
                                    "Array destructuring requires array, got {}",
                                    val_type.as_str()
                                ),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::E102,
                                None,
                            );
                        }
                        for elem in elements {
                            match elem {
                                DestructureElement::Name(name) => {
                                    self.define_var(
                                        name,
                                        ChannelType::Null, // element type unknown
                                        *mutable,
                                        stmt.span.start_line,
                                        stmt.span.start_col,
                                    );
                                }
                                DestructureElement::Rest(name) => {
                                    self.define_var(
                                        name,
                                        ChannelType::Array,
                                        *mutable,
                                        stmt.span.start_line,
                                        stmt.span.start_col,
                                    );
                                }
                            }
                        }
                    }
                    DestructurePattern::Map(entries) => {
                        if val_type != ChannelType::Map && val_type != ChannelType::Null {
                            self.emit_coded(
                                stmt.span.start_line,
                                stmt.span.start_col,
                                format!(
                                    "Map destructuring requires map, got {}",
                                    val_type.as_str()
                                ),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::E100,
                                None,
                            );
                        }
                        for (key, alias) in entries {
                            let var_name = alias.as_deref().unwrap_or(key);
                            self.define_var(
                                var_name,
                                ChannelType::Null, // value type unknown
                                *mutable,
                                stmt.span.start_line,
                                stmt.span.start_col,
                            );
                        }
                    }
                }
            }

            // -----------------------------------------------------------------
            // name += expr; / name -= expr; etc.
            // -----------------------------------------------------------------
            StatementKind::CompoundAssign { name, op, value } => {
                let val_type = self.infer_expr(value);

                let (exists, is_mutable, var_type) = match self.lookup(name) {
                    Some(info) => (true, info.mutable, info.channel_type),
                    None => (false, false, ChannelType::Null),
                };

                if !exists {
                    let suggestion = self.suggest_variable(name);
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("Undefined variable '{}'", name),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E200,
                        suggestion,
                    );
                } else if !is_mutable {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("Cannot assign to immutable variable '{}'", name),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E404,
                        None,
                    );
                }

                // Check that operation makes sense for the types
                let result_type = self.infer_binop(*op, var_type, val_type, stmt.span);

                if let Some(info) = self.lookup_mut(name) {
                    info.channel_type = result_type;
                    info.used = true;
                    info.mutated = true;
                }
            }

            // -----------------------------------------------------------------
            // try { ... } catch err { ... } finally { ... }
            // -----------------------------------------------------------------
            StatementKind::TryCatch {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                self.push_scope();
                self.check_block(try_block);
                self.pop_scope();

                self.push_scope();
                if let Some(var_name) = catch_var {
                    // Error variable can be any thrown type
                    self.define_var(
                        var_name,
                        ChannelType::Null,
                        false,
                        stmt.span.start_line,
                        stmt.span.start_col,
                    );
                }
                self.check_block(catch_block);
                self.pop_scope();

                if let Some(fb) = finally_block {
                    self.push_scope();
                    self.check_block(fb);
                    self.pop_scope();
                }
            }

            // -----------------------------------------------------------------
            // throw expr;
            // -----------------------------------------------------------------
            StatementKind::Throw(expr) => {
                let _ = self.infer_expr(expr);
            }

            // -----------------------------------------------------------------
            // const NAME = expr;
            // -----------------------------------------------------------------
            StatementKind::ConstDef {
                name,
                type_annotation,
                value,
            } => {
                let inferred = self.infer_expr(value);
                let ct = self.reconcile_annotation(
                    type_annotation.as_deref(),
                    inferred,
                    stmt.span.start_line,
                    stmt.span.start_col,
                    name,
                );
                // Constants are immutable
                self.define_var(name, ct, false, stmt.span.start_line, stmt.span.start_col);
            }

            // -----------------------------------------------------------------
            // type Name = target;
            // -----------------------------------------------------------------
            StatementKind::TypeAlias { name, target } => {
                // Validate the target type exists
                if ChannelType::parse(target).is_none() {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("Unknown type '{}' in type alias '{}'", target, name),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E100,
                        None,
                    );
                }
            }

            // -----------------------------------------------------------------
            // mod name { body }
            // -----------------------------------------------------------------
            StatementKind::ModuleDef { name: _, body } => {
                self.push_scope();
                for s in &body.statements {
                    self.check_statement(s);
                }
                if let Some(tail) = &body.tail_expr {
                    let _ = self.infer_expr(tail);
                }
                self.pop_scope();
            }

            // -----------------------------------------------------------------
            // use path::to::item;
            // -----------------------------------------------------------------
            StatementKind::Use { path, .. } => {
                // Validate that std module paths are known
                if path.first().map(|s| s.as_str()) == Some("std") && path.len() >= 2 {
                    let known_modules = [
                        "math", "cmp", "logic", "bits", "str", "convert", "array", "map", "bytes",
                        "json", "time", "hash", "io", "control", "rand", "fs", "env", "net", "tcp",
                        "udp", "ws", "sse", "http_server", "path", "yaml", "csv", "toml", "regex",
                        "uuid", "crypto", "compress", "fmt", "stats", "text", "encode", "reflect",
                        "collections", "sort", "cert",
                    ];
                    if !known_modules.contains(&path[1].as_str()) {
                        self.emit_coded(
                            stmt.span.start_line,
                            stmt.span.start_col,
                            format!("Unknown standard library module 'std::{}'", path[1]),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::E203,
                            None,
                        );
                    }
                }
            }

            StatementKind::TestDef { body, .. } => {
                self.push_scope();
                self.check_block(body);
                self.pop_scope();
            }

            StatementKind::EnumDef { name, variants } => {
                let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
                self.enum_variants.insert(name.clone(), variant_names);
            }
            StatementKind::StructDef { .. } => {
                // Struct definitions are valid at any scope
            }
        }
    }

    // =========================================================================
    /// Check if match arms exhaustively cover all variants of a known enum.
    fn check_enum_exhaustive(&self, arms: &[crate::syntax::ast::MatchArm]) -> bool {
        use std::collections::HashSet;
        // Collect (enum_name, variant) pairs from unguarded EnumPattern arms
        let mut enum_name: Option<&str> = None;
        let mut covered = HashSet::new();
        for arm in arms {
            if arm.guard.is_some() {
                continue; // guarded arms don't guarantee coverage
            }
            if let Pattern::EnumPattern { enum_name: en, variant, .. } = &arm.pattern {
                match enum_name {
                    None => { enum_name = Some(en); }
                    Some(existing) if existing != en.as_str() => return false, // mixed enums
                    _ => {}
                }
                covered.insert(variant.as_str());
            }
            // Or patterns can also contribute
            if let Pattern::Or(alternatives) = &arm.pattern {
                for alt in alternatives {
                    if let Pattern::EnumPattern { enum_name: en, variant, .. } = alt {
                        match enum_name {
                            None => { enum_name = Some(en); }
                            Some(existing) if existing != en.as_str() => return false,
                            _ => {}
                        }
                        covered.insert(variant.as_str());
                    }
                }
            }
        }
        // Look up the enum definition
        if let Some(name) = enum_name {
            if let Some(variants) = self.enum_variants.get(name) {
                return variants.iter().all(|v| covered.contains(v.as_str()));
            }
        }
        false
    }

    // Block
    // =========================================================================

    fn check_block(&mut self, block: &Block) {
        for stmt in &block.statements {
            self.check_statement(stmt);
        }
        if let Some(tail) = &block.tail_expr {
            let _ = self.infer_expr(tail);
        }
    }

    fn infer_block(&mut self, block: &Block) -> ChannelType {
        self.push_scope();
        let ty = self.infer_block_no_scope(block);
        self.pop_scope();
        ty
    }

    /// Infer the type of a block without pushing/popping scope.
    /// Used when the caller already manages the scope (e.g., function defs).
    fn infer_block_no_scope(&mut self, block: &Block) -> ChannelType {
        for stmt in &block.statements {
            self.check_statement(stmt);
        }
        if let Some(tail) = &block.tail_expr {
            self.infer_expr(tail)
        } else {
            ChannelType::Null
        }
    }

    // =========================================================================
    // Expression type inference
    // =========================================================================

    fn infer_expr(&mut self, expr: &Expression) -> ChannelType {
        match &expr.kind {
            // -----------------------------------------------------------------
            // Literals
            // -----------------------------------------------------------------
            ExpressionKind::Literal(lit) => match lit {
                Literal::Int64(_) => ChannelType::Int64,
                Literal::Float64(_) => ChannelType::Float64,
                Literal::String(_) => ChannelType::String,
                Literal::Bool(_) => ChannelType::Bool,
                Literal::Null => ChannelType::Null,
                Literal::Array(elements) => {
                    // Infer each element for side-effect diagnostics.
                    for el in elements {
                        let _ = self.infer_expr(el);
                    }
                    ChannelType::Array
                }
                Literal::Map(entries) => {
                    // E107: Duplicate map keys.
                    let mut seen_keys = HashSet::new();
                    for (key, val) in entries {
                        if !seen_keys.insert(key.as_str()) {
                            self.emit_coded(
                                expr.span.start_line,
                                expr.span.start_col,
                                format!("Duplicate key '{}' in map literal", key),
                                DiagnosticSeverity::Error,
                                super::errors::ErrorCode::E107,
                                None,
                            );
                        }
                        let _ = self.infer_expr(val);
                    }
                    ChannelType::Map
                }
            },

            // -----------------------------------------------------------------
            // Variable reference
            // -----------------------------------------------------------------
            ExpressionKind::Variable(name) => {
                // Copy out what we need before the mutable borrow.
                let ct = match self.lookup(name) {
                    Some(info) => info.channel_type,
                    None => {
                        // Check if it's a known function name (first-class function reference).
                        if self.function_sigs.contains_key(name.as_str()) {
                            if let Some(sig) = self.function_sigs.get_mut(name.as_str()) {
                                sig.used = true;
                            }
                            return ChannelType::Null; // function type is opaque
                        }
                        let suggestion = self.suggest_variable(name);
                        self.emit_coded(
                            expr.span.start_line,
                            expr.span.start_col,
                            format!("Undefined variable '{}'", name),
                            DiagnosticSeverity::Error,
                            super::errors::ErrorCode::E200,
                            suggestion,
                        );
                        return ChannelType::Null;
                    }
                };
                // Mark used.
                if let Some(info) = self.lookup_mut(name) {
                    info.used = true;
                }
                ct
            }

            // -----------------------------------------------------------------
            // Binary operations
            // -----------------------------------------------------------------
            ExpressionKind::BinaryOp { op, left, right } => {
                let left_ty = self.infer_expr(left);
                let right_ty = self.infer_expr(right);
                self.check_binop_literals(*op, left, right, left_ty, right_ty, expr.span);
                self.infer_binop(*op, left_ty, right_ty, expr.span)
            }

            // -----------------------------------------------------------------
            // Unary operations
            // -----------------------------------------------------------------
            ExpressionKind::UnaryOp { op, operand } => {
                // W106: Double negation or double NOT.
                if let ExpressionKind::UnaryOp { op: inner_op, .. } = &operand.kind {
                    if op == inner_op {
                        let msg = match op {
                            UnOp::Neg => "Double negation is redundant",
                            UnOp::Not => "Double logical NOT is redundant",
                        };
                        self.emit_coded(
                            expr.span.start_line,
                            expr.span.start_col,
                            msg.to_string(),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::W106,
                            None,
                        );
                    }
                }
                let operand_ty = self.infer_expr(operand);
                match op {
                    UnOp::Not => {
                        if operand_ty != ChannelType::Bool && operand_ty != ChannelType::Null {
                            self.emit_coded(
                                expr.span.start_line,
                                expr.span.start_col,
                                format!("Logical NOT expects bool, got {}", operand_ty.as_str()),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::E101,
                                None,
                            );
                        }
                        ChannelType::Bool
                    }
                    UnOp::Neg => {
                        if !is_numeric(operand_ty) && operand_ty != ChannelType::Null {
                            self.emit_coded(
                                expr.span.start_line,
                                expr.span.start_col,
                                format!(
                                    "Negation expects numeric type, got {}",
                                    operand_ty.as_str()
                                ),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::E103,
                                None,
                            );
                        }
                        // Preserve the input numeric type.
                        if is_numeric(operand_ty) {
                            operand_ty
                        } else {
                            ChannelType::Null
                        }
                    }
                }
            }

            // -----------------------------------------------------------------
            // Function / operation call
            // -----------------------------------------------------------------
            ExpressionKind::Call { name, args, kwargs } => {
                // Infer all argument types first (for side effects + diagnostics).
                let arg_types: Vec<ChannelType> = args.iter().map(|a| self.infer_expr(a)).collect();
                for (_, v) in kwargs {
                    self.infer_expr(v);
                }

                // W7: Empty range via range() call.
                if name == "range" && args.len() >= 2 {
                    if let (Some(s), Some(e)) = (literal_int(&args[0]), literal_int(&args[1])) {
                        if s >= e {
                            self.emit_coded(
                                expr.span.start_line,
                                expr.span.start_col,
                                "Range will produce empty array (start >= end)".to_string(),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::W107,
                                None,
                            );
                        }
                    }
                }

                // Is it a user-defined function?
                if let Some(sig) = self.function_sigs.get(name).cloned() {
                    // Mark as used
                    if let Some(sig_mut) = self.function_sigs.get_mut(name) {
                        sig_mut.used = true;
                    }
                    // Check arity (accounting for default parameters)
                    if arg_types.len() < sig.required_params || arg_types.len() > sig.params.len() {
                        let arity_msg = if sig.required_params == sig.params.len() {
                            format!("{}", sig.params.len())
                        } else {
                            format!("{}-{}", sig.required_params, sig.params.len())
                        };
                        self.emit_coded(
                            expr.span.start_line,
                            expr.span.start_col,
                            format!(
                                "Function '{}' expects {} arguments, got {}",
                                name,
                                arity_msg,
                                arg_types.len()
                            ),
                            DiagnosticSeverity::Error,
                            super::errors::ErrorCode::E405,
                            None,
                        );
                    }
                    // Check param types
                    for (i, (param_name, expected_type)) in sig.params.iter().enumerate() {
                        if let Some(&actual_type) = arg_types.get(i) {
                            if actual_type != ChannelType::Null
                                && *expected_type != ChannelType::Null
                                && !actual_type.is_compatible_with(expected_type)
                            {
                                if let Some(arg_span) = args.get(i).map(|a| a.span) {
                                    self.emit_coded(
                                        arg_span.start_line,
                                        arg_span.start_col,
                                        format!(
                                            "Type mismatch on '{}': got {} but expected {}",
                                            param_name,
                                            actual_type.as_str(),
                                            expected_type.as_str(),
                                        ),
                                        DiagnosticSeverity::Error,
                                        super::errors::ErrorCode::E100,
                                        None,
                                    );
                                }
                            }
                        }
                    }
                    return sig.return_type;
                }

                // Is it a variable holding a lambda (callable value)?
                // Check before operations so user-defined names shadow built-ins.
                if self.lookup(name).is_some() {
                    if let Some(info) = self.lookup_mut(name) {
                        info.used = true;
                    }
                    return ChannelType::Null;
                }

                // Is it an plugin call?
                if self.imports.contains(name.as_str()) {
                    self.used_imports.insert(name.clone());
                    return ChannelType::Null;
                }

                // Is it a known operation?
                if let Some(op) = OperationType::parse(name) {
                    let expected_inputs = op_input_types(op);

                    // Check positional arg types against expected input ports.
                    for (i, (port_name, expected_type)) in expected_inputs.iter().enumerate() {
                        if let Some(&actual_type) = arg_types.get(i) {
                            if actual_type != ChannelType::Null
                                && *expected_type != ChannelType::Null
                                && !actual_type.is_compatible_with(expected_type)
                            {
                                let arg_span = &args[i].span;
                                self.emit_coded(
                                    arg_span.start_line,
                                    arg_span.start_col,
                                    format!(
                                        "Type mismatch on '{}': got {} but expected {}",
                                        port_name,
                                        actual_type.as_str(),
                                        expected_type.as_str(),
                                    ),
                                    DiagnosticSeverity::Warning,
                                    super::errors::ErrorCode::E103,
                                    None,
                                );
                            }
                        }
                    }

                    return refine_call_output(op, &arg_types);
                }

                // Unknown function.
                self.emit_coded(
                    expr.span.start_line,
                    expr.span.start_col,
                    format!("Unknown operation '{}'", name),
                    DiagnosticSeverity::Error,
                    super::errors::ErrorCode::E201,
                    self.suggest_variable(name).map(|s| format!("Did you mean '{}'?", s)),
                );
                ChannelType::Null
            }

            // -----------------------------------------------------------------
            // Pipe: left |> right
            // -----------------------------------------------------------------
            ExpressionKind::Pipe { left, right } => {
                self.infer_expr(left);
                // The right side may contain a Placeholder that receives left_ty,
                // but since we can't thread the pipe type through without
                // modifying the AST, just infer the right side normally.
                self.pipe_depth += 1;
                let right_ty = self.infer_expr(right);
                self.pipe_depth -= 1;
                right_ty
            }

            // -----------------------------------------------------------------
            // If/else expression
            // -----------------------------------------------------------------
            ExpressionKind::IfElse {
                condition,
                then_block,
                else_block,
            } => {
                let cond_ty = self.infer_expr(condition);
                if cond_ty != ChannelType::Bool && cond_ty != ChannelType::Null {
                    self.emit_coded(
                        condition.span.start_line,
                        condition.span.start_col,
                        format!("If condition should be bool, got {}", cond_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E101,
                        None,
                    );
                }

                let then_ty = self.infer_block(then_block);
                let else_ty = if let Some(else_blk) = else_block {
                    self.infer_block(else_blk)
                } else {
                    ChannelType::Null
                };

                // Try to unify branches.
                if then_ty == else_ty {
                    then_ty
                } else if then_ty == ChannelType::Null {
                    else_ty
                } else if else_ty == ChannelType::Null {
                    then_ty
                } else {
                    // Branches disagree — warn and use Null.
                    self.emit_coded(
                        expr.span.start_line,
                        expr.span.start_col,
                        format!(
                            "If/else branches have mismatched types: '{}' vs '{}'",
                            then_ty.as_str(),
                            else_ty.as_str(),
                        ),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E100,
                        None,
                    );
                    ChannelType::Null
                }
            }

            // -----------------------------------------------------------------
            // Block expression
            // -----------------------------------------------------------------
            ExpressionKind::Block(block) => self.infer_block(block),

            // -----------------------------------------------------------------
            // Index: arr[i]
            // -----------------------------------------------------------------
            ExpressionKind::Index { object, index } => {
                let obj_ty = self.infer_expr(object);
                let idx_ty = self.infer_expr(index);

                if obj_ty != ChannelType::Array
                    && obj_ty != ChannelType::Null
                    && obj_ty != ChannelType::Map
                {
                    self.emit_coded(
                        object.span.start_line,
                        object.span.start_col,
                        format!("Indexing requires array or map, got {}", obj_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E100,
                        None,
                    );
                }

                if obj_ty == ChannelType::Array
                    && !is_integer(idx_ty)
                    && idx_ty != ChannelType::Null
                {
                    self.emit_coded(
                        index.span.start_line,
                        index.span.start_col,
                        format!("Array index should be integer, got {}", idx_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E103,
                        None,
                    );
                }

                // E105: Negative array index literal.
                if obj_ty == ChannelType::Array || obj_ty == ChannelType::Null {
                    if let Some(idx_val) = literal_int(index) {
                        if idx_val < 0 {
                            self.emit_coded(
                                index.span.start_line,
                                index.span.start_col,
                                format!("Negative array index ({})", idx_val),
                                DiagnosticSeverity::Error,
                                super::errors::ErrorCode::E105,
                                None,
                            );
                        }
                    }
                }

                // E106: Index into empty array literal.
                if let ExpressionKind::Literal(Literal::Array(elements)) = &object.kind {
                    if elements.is_empty() {
                        self.emit_coded(
                            object.span.start_line,
                            object.span.start_col,
                            "Index into empty array literal".to_string(),
                            DiagnosticSeverity::Error,
                            super::errors::ErrorCode::E106,
                            None,
                        );
                    }
                }

                // Element type unknown.
                ChannelType::Null
            }

            // -----------------------------------------------------------------
            // Field access: obj.field
            // -----------------------------------------------------------------
            ExpressionKind::FieldAccess { object, field: _ } => {
                let obj_ty = self.infer_expr(object);
                if obj_ty != ChannelType::Map && obj_ty != ChannelType::Null {
                    self.emit_coded(
                        object.span.start_line,
                        object.span.start_col,
                        format!("Field access requires map, got {}", obj_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E100,
                        None,
                    );
                }
                ChannelType::Null
            }

            // -----------------------------------------------------------------
            // Placeholder (_)
            // -----------------------------------------------------------------
            ExpressionKind::Placeholder => {
                if self.pipe_depth == 0 {
                    self.emit_coded(
                        expr.span.start_line,
                        expr.span.start_col,
                        "Placeholder '_' can only be used inside pipe expressions".to_string(),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E303,
                        None,
                    );
                }
                ChannelType::Null
            }

            // -----------------------------------------------------------------
            // Range expression: range(start, end)
            // -----------------------------------------------------------------
            ExpressionKind::Range { start, end, .. } => {
                let start_ty = self.infer_expr(start);
                let end_ty = self.infer_expr(end);

                if !is_numeric(start_ty) && start_ty != ChannelType::Null {
                    self.emit_coded(
                        start.span.start_line,
                        start.span.start_col,
                        format!("Range start should be numeric, got {}", start_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E103,
                        None,
                    );
                }
                if !is_numeric(end_ty) && end_ty != ChannelType::Null {
                    self.emit_coded(
                        end.span.start_line,
                        end.span.start_col,
                        format!("Range end should be numeric, got {}", end_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E103,
                        None,
                    );
                }

                // W7: Empty range (start >= end with literals).
                if let (Some(s), Some(e)) = (literal_int(start), literal_int(end)) {
                    if s >= e {
                        self.emit_coded(
                            expr.span.start_line,
                            expr.span.start_col,
                            "Range will produce empty array (start >= end)".to_string(),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::W107,
                            None,
                        );
                    }
                }

                ChannelType::Array
            }

            ExpressionKind::Await(inner) => {
                // Await unwraps the future — return the inner type
                self.infer_expr(inner)
            }

            ExpressionKind::Spawn(inner) => {
                // Spawn wraps the result in a Future
                self.infer_expr(inner);
                ChannelType::Null // Future type not in ChannelType yet
            }

            // -----------------------------------------------------------------
            // Method call: obj.method(args)
            // -----------------------------------------------------------------
            ExpressionKind::MethodCall {
                object,
                method,
                args,
                kwargs,
            } => {
                let obj_ty = self.infer_expr(object);
                let mut arg_types: Vec<ChannelType> =
                    args.iter().map(|a| self.infer_expr(a)).collect();
                for (_, v) in kwargs {
                    let _ = self.infer_expr(v);
                }

                // Mark receiver as mutated for known in-place mutating methods
                const MUTATING_METHODS: &[&str] = &[
                    "push", "pop", "set", "remove", "insert", "clear",
                    "delete", "merge", "sort", "reverse", "extend",
                ];
                if MUTATING_METHODS.contains(&method.as_str()) {
                    if let ExpressionKind::Variable(ref var_name) = object.kind {
                        if let Some(info) = self.lookup_mut(var_name) {
                            info.mutated = true;
                        }
                    }
                }

                // Resolve method to an OperationType based on receiver type + method name
                let op_name = resolve_method_type(obj_ty, method);
                if let Some(name) = op_name {
                    if let Some(op) = OperationType::parse(&name) {
                        // Prepend receiver type as first arg for type refinement
                        arg_types.insert(0, obj_ty);
                        return refine_call_output(op, &arg_types);
                    }
                    // Known built-in method handled by the interpreter directly
                    // (e.g., array.first(), int.abs(), string.is_empty())
                    // Return Null (unknown type) without warning
                    return ChannelType::Null;
                }

                // Unknown method — warn (but suppress if receiver type is unknown/Null)
                if obj_ty != ChannelType::Null {
                    self.emit_coded(
                        expr.span.start_line,
                        expr.span.start_col,
                        format!("Unknown method '{}' on type '{}'", method, obj_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E201,
                        None,
                    );
                }
                ChannelType::Null
            }

            // -----------------------------------------------------------------
            // Lambda: |params| expr
            // -----------------------------------------------------------------
            ExpressionKind::Lambda { params, body } => {
                self.push_scope();
                self.function_depth += 1;
                let saved_loop_depth = self.loop_depth;
                self.loop_depth = 0;
                let prev_return_type = std::mem::replace(&mut self.current_return_type, ChannelType::Null);
                for param in params {
                    let ct = param
                        .type_annotation
                        .as_deref()
                        .and_then(ChannelType::parse)
                        .unwrap_or(ChannelType::Null);
                    // Type-check default param expression if present
                    if let Some(default_expr) = &param.default {
                        let default_type = self.infer_expr(default_expr);
                        if ct != ChannelType::Null
                            && default_type != ChannelType::Null
                            && !default_type.is_compatible_with(&ct)
                        {
                            let code = super::errors::ErrorCode::W106;
                            self.diagnostics.push(AstDiagnostic {
                                line: default_expr.span.start_line,
                                column: default_expr.span.start_col,
                                message: format!(
                                    "default value type '{}' doesn't match parameter type '{}'",
                                    default_type, ct
                                ),
                                severity: DiagnosticSeverity::Warning,
                                code: Some(code.to_string()),
                                help: Some(code.help().to_string()),
                                suggestion: None,
                            });
                        }
                    }
                    self.define_var(
                        &param.name,
                        ct,
                        false,
                        param.span.start_line,
                        param.span.start_col,
                    );
                    if let Some(info) = self.lookup_mut(&param.name) {
                        info.is_param = true;
                    }
                }
                let _ = self.infer_expr(body);
                self.function_depth -= 1;
                self.loop_depth = saved_loop_depth;
                self.current_return_type = prev_return_type;
                self.pop_scope();
                // Lambdas are callable values — Null since we don't have a function type
                ChannelType::Null
            }

            // -----------------------------------------------------------------
            // Match expression: match value { pattern => body, ... }
            // -----------------------------------------------------------------
            ExpressionKind::Match { value, arms } => {
                let val_type = self.infer_expr(value);

                if arms.is_empty() {
                    self.emit_coded(
                        expr.span.start_line,
                        expr.span.start_col,
                        "Empty match expression".to_string(),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::W206,
                        None,
                    );
                    return ChannelType::Null;
                }

                // Check for exhaustiveness
                let has_catchall = arms.iter().any(|arm| {
                    matches!(arm.pattern, Pattern::Wildcard | Pattern::Variable(_))
                        && arm.guard.is_none()
                });
                if !has_catchall {
                    // Check if all enum variants are covered
                    let enum_exhaustive = self.check_enum_exhaustive(arms);
                    if !enum_exhaustive {
                        self.emit_coded(
                            expr.span.start_line,
                            expr.span.start_col,
                            "Non-exhaustive match: consider adding a wildcard '_' arm".to_string(),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::W203,
                            Some("Add a `_ => ...` arm to handle remaining cases".to_string()),
                        );
                    }
                }

                let mut arm_types = Vec::new();
                for arm in arms {
                    self.push_scope();
                    // Bind pattern variables
                    self.bind_pattern_vars(&arm.pattern, val_type, &arm.span);
                    if let Some(guard) = &arm.guard {
                        let guard_ty = self.infer_expr(guard);
                        if guard_ty != ChannelType::Bool && guard_ty != ChannelType::Null {
                            self.emit_coded(
                                guard.span.start_line,
                                guard.span.start_col,
                                format!("Match guard should be bool, got {}", guard_ty.as_str()),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::E101,
                                None,
                            );
                        }
                    }
                    let body_ty = self.infer_block_no_scope(&arm.body);
                    arm_types.push(body_ty);
                    self.pop_scope();
                }

                // Unify arm types
                unify_types(&arm_types)
            }

            // -----------------------------------------------------------------
            // String interpolation: f"text {expr} text"
            // -----------------------------------------------------------------
            ExpressionKind::StringInterpolation { parts } => {
                for part in parts {
                    if let StringPart::Expr(e) = part {
                        let _ = self.infer_expr(e);
                    }
                }
                ChannelType::String
            }

            // -----------------------------------------------------------------
            // Null coalescing: x ?? default
            // -----------------------------------------------------------------
            ExpressionKind::NullCoalesce { left, right } => {
                let left_ty = self.infer_expr(left);
                let right_ty = self.infer_expr(right);
                // Result type: right side if left is null, otherwise left type
                if left_ty == ChannelType::Null {
                    right_ty
                } else {
                    left_ty
                }
            }

            // -----------------------------------------------------------------
            // Optional chaining: obj?.field
            // -----------------------------------------------------------------
            ExpressionKind::OptionalChain { object, field: _ } => {
                let obj_ty = self.infer_expr(object);
                if obj_ty != ChannelType::Map && obj_ty != ChannelType::Null {
                    self.emit_coded(
                        object.span.start_line,
                        object.span.start_col,
                        format!(
                            "Optional chaining requires map or null, got {}",
                            obj_ty.as_str()
                        ),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E100,
                        None,
                    );
                }
                // Result is always nullable — field type is unknown
                ChannelType::Null
            }

            // -----------------------------------------------------------------
            // Spread: ...expr
            // -----------------------------------------------------------------
            ExpressionKind::Spread(inner) => {
                let inner_ty = self.infer_expr(inner);
                if inner_ty != ChannelType::Array
                    && inner_ty != ChannelType::Map
                    && inner_ty != ChannelType::Null
                {
                    self.emit_coded(
                        expr.span.start_line,
                        expr.span.start_col,
                        format!("Spread requires array or map, got {}", inner_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E103,
                        None,
                    );
                }
                inner_ty
            }

            // -----------------------------------------------------------------
            // loop { body } — infinite loop with break value
            // -----------------------------------------------------------------
            ExpressionKind::Loop(block) => {
                if is_empty_block(block) {
                    self.emit_coded(
                        expr.span.start_line,
                        expr.span.start_col,
                        "Empty loop body".to_string(),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::W104,
                        None,
                    );
                }
                self.push_scope();
                self.loop_depth += 1;
                self.check_block(block);
                self.loop_depth -= 1;
                self.pop_scope();
                // Break value type unknown at static analysis time
                ChannelType::Null
            }

            // -----------------------------------------------------------------
            // try { ... } catch err { ... } — expression form
            // -----------------------------------------------------------------
            ExpressionKind::TryCatchExpr {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                let try_ty = self.infer_block(try_block);

                self.push_scope();
                if let Some(var_name) = catch_var {
                    // Error variable can be any thrown type
                    self.define_var(
                        var_name,
                        ChannelType::Null,
                        false,
                        expr.span.start_line,
                        expr.span.start_col,
                    );
                }
                let catch_ty = self.infer_block_no_scope(catch_block);
                self.pop_scope();

                if let Some(finally) = finally_block {
                    self.infer_block(finally);
                }

                // Unify try/catch types
                if try_ty == catch_ty {
                    try_ty
                } else if try_ty == ChannelType::Null {
                    catch_ty
                } else if catch_ty == ChannelType::Null {
                    try_ty
                } else {
                    ChannelType::Null
                }
            }

            ExpressionKind::ListComprehension { expr: body, pattern, iterable, condition } => {
                self.infer_expr(iterable);
                self.push_scope();
                match pattern {
                    ForPattern::Single(name) => {
                        self.define_var(name, ChannelType::Null, false, expr.span.start_line, expr.span.start_col);
                    }
                    ForPattern::ArrayDestructure(elements) => {
                        for elem in elements {
                            let name = match elem {
                                DestructureElement::Name(n) => n,
                                DestructureElement::Rest(n) => n,
                            };
                            self.define_var(name, ChannelType::Null, false, expr.span.start_line, expr.span.start_col);
                        }
                    }
                    ForPattern::MapDestructure(entries) => {
                        for (key, alias) in entries {
                            let name = alias.as_ref().unwrap_or(key);
                            self.define_var(name, ChannelType::Null, false, expr.span.start_line, expr.span.start_col);
                        }
                    }
                }
                if let Some(cond) = condition {
                    self.infer_expr(cond);
                }
                self.infer_expr(body);
                self.pop_scope();
                ChannelType::Array
            }

            ExpressionKind::MapComprehension { key_expr, value_expr, pattern, iterable, condition } => {
                self.infer_expr(iterable);
                self.push_scope();
                match pattern {
                    ForPattern::Single(name) => {
                        self.define_var(name, ChannelType::Null, false, expr.span.start_line, expr.span.start_col);
                    }
                    ForPattern::ArrayDestructure(elements) => {
                        for elem in elements {
                            let name = match elem {
                                DestructureElement::Name(n) => n,
                                DestructureElement::Rest(n) => n,
                            };
                            self.define_var(name, ChannelType::Null, false, expr.span.start_line, expr.span.start_col);
                        }
                    }
                    ForPattern::MapDestructure(entries) => {
                        for (key, alias) in entries {
                            let name = alias.as_ref().unwrap_or(key);
                            self.define_var(name, ChannelType::Null, false, expr.span.start_line, expr.span.start_col);
                        }
                    }
                }
                if let Some(cond) = condition {
                    self.infer_expr(cond);
                }
                self.infer_expr(key_expr);
                self.infer_expr(value_expr);
                self.pop_scope();
                ChannelType::Map
            }

            ExpressionKind::EnumConstruct { enum_name, variant, args } => {
                // Validate enum variant exists
                if let Some(variants) = self.enum_variants.get(enum_name.as_str()) {
                    if !variants.iter().any(|v| v == variant) {
                        let code = super::errors::ErrorCode::E202;
                        self.diagnostics.push(AstDiagnostic {
                            line: expr.span.start_line,
                            column: expr.span.start_col,
                            message: format!(
                                "enum '{}' has no variant '{}' (available: {})",
                                enum_name,
                                variant,
                                variants.join(", ")
                            ),
                            severity: DiagnosticSeverity::Error,
                            code: Some(code.to_string()),
                            help: Some(code.help().to_string()),
                            suggestion: super::errors::suggest_name(variant, &variants.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
                        });
                    }
                }
                for arg in args {
                    self.infer_expr(arg);
                }
                ChannelType::Map
            }

            ExpressionKind::StructConstruct { name, fields } => {
                // Check for duplicate field names
                let mut seen = HashSet::new();
                for (field_name, field_expr) in fields {
                    if !seen.insert(field_name.as_str()) {
                        let code = super::errors::ErrorCode::E107;
                        self.diagnostics.push(AstDiagnostic {
                            line: expr.span.start_line,
                            column: expr.span.start_col,
                            message: format!("duplicate field '{}' in struct '{}' constructor", field_name, name),
                            severity: DiagnosticSeverity::Warning,
                            code: Some(code.to_string()),
                            help: Some(code.help().to_string()),
                            suggestion: None,
                        });
                    }
                    self.infer_expr(field_expr);
                }
                ChannelType::Map
            }

            ExpressionKind::TryPropagate(inner) => {
                self.infer_expr(inner)
            }
        }
    }

    // =========================================================================
    // Pattern variable binding (for match expressions)
    // =========================================================================

    fn bind_pattern_vars(&mut self, pattern: &Pattern, val_type: ChannelType, span: &Span) {
        match pattern {
            Pattern::Literal(_) | Pattern::Wildcard => {
                // No variables to bind
            }
            Pattern::Variable(name) => {
                self.define_var(
                    name,
                    val_type,
                    false,
                    span.start_line,
                    span.start_col,
                );
            }
            Pattern::Array(sub_patterns) => {
                // Array element types are unknown at pattern level
                for sub in sub_patterns {
                    self.bind_pattern_vars(sub, ChannelType::Null, span);
                }
            }
            Pattern::Map(entries) => {
                // Map value types are unknown at pattern level
                for (_, sub_pattern) in entries {
                    self.bind_pattern_vars(sub_pattern, ChannelType::Null, span);
                }
            }
            Pattern::Or(alternatives) => {
                // Bind vars from the first alternative only to avoid duplicate W102 warnings.
                // All alternatives should bind the same names in valid code.
                if let Some(first) = alternatives.first() {
                    self.bind_pattern_vars(first, val_type, span);
                }
            }
            Pattern::Rest(name) => {
                if let Some(name) = name {
                    self.define_var(
                        name,
                        ChannelType::Array,
                        false,
                        span.start_line,
                        span.start_col,
                    );
                }
            }
            Pattern::EnumPattern { bindings, .. } => {
                for sub in bindings {
                    self.bind_pattern_vars(sub, ChannelType::Null, span);
                }
            }
            Pattern::TypePattern { name, .. } => {
                self.define_var(name, ChannelType::Null, false, span.start_line, span.start_col);
            }
            Pattern::RangePattern { .. } => {
                // No variables to bind
            }
        }
    }

    // =========================================================================
    // Binary operator type inference
    // =========================================================================

    fn infer_binop(
        &mut self,
        op: BinOp,
        left: ChannelType,
        right: ChannelType,
        span: Span,
    ) -> ChannelType {
        match op {
            // Comparison operators always return Bool.
            BinOp::Eq | BinOp::NotEq | BinOp::Gt | BinOp::Lt | BinOp::GtEq | BinOp::LtEq => {
                ChannelType::Bool
            }

            // Logical operators: both sides should be Bool.
            BinOp::And | BinOp::Or => {
                if left != ChannelType::Bool && left != ChannelType::Null {
                    self.emit_coded(
                        span.start_line,
                        span.start_col,
                        format!(
                            "Left operand of '{}' should be bool, got {}",
                            op,
                            left.as_str()
                        ),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E101,
                        None,
                    );
                }
                if right != ChannelType::Bool && right != ChannelType::Null {
                    self.emit_coded(
                        span.start_line,
                        span.start_col,
                        format!(
                            "Right operand of '{}' should be bool, got {}",
                            op,
                            right.as_str()
                        ),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E101,
                        None,
                    );
                }
                ChannelType::Bool
            }

            // Arithmetic operators: use the operation's typing rules.
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                // Division always returns Float64.
                if op == BinOp::Div {
                    return ChannelType::Float64;
                }

                // Look up via OperationType for consistency with abstract_interp.
                if let Some(ot) = OperationType::parse(op.operation_name()) {
                    let static_ty = op_output_type(ot);
                    if static_ty != ChannelType::Null {
                        return static_ty;
                    }
                }

                // Polymorphic: promote from inputs.
                promote_numeric(&[left, right])
            }
        }
    }

    // =========================================================================
    // Binary operator literal checks
    // =========================================================================

    /// Check binary operations for common mistakes involving literals.
    fn check_binop_literals(
        &mut self,
        op: BinOp,
        left: &Expression,
        right: &Expression,
        left_ty: ChannelType,
        right_ty: ChannelType,
        span: Span,
    ) {
        // E104: Division/modulo by literal zero.
        if (op == BinOp::Div || op == BinOp::Mod)
            && (literal_int(right) == Some(0) || literal_float(right) == Some(0.0))
        {
            self.emit_coded(
                span.start_line,
                span.start_col,
                "Division by zero".to_string(),
                DiagnosticSeverity::Error,
                super::errors::ErrorCode::E104,
                None,
            );
        }

        // W107: Modulo by 1.
        if op == BinOp::Mod && literal_int(right) == Some(1) {
            self.emit_coded(
                span.start_line,
                span.start_col,
                "Modulo by 1 always returns 0".to_string(),
                DiagnosticSeverity::Warning,
                super::errors::ErrorCode::W107,
                None,
            );
        }

        // W107: Multiply by 0.
        if op == BinOp::Mul
            && (literal_int(left) == Some(0)
                || literal_int(right) == Some(0)
                || literal_float(left) == Some(0.0)
                || literal_float(right) == Some(0.0))
        {
            self.emit_coded(
                span.start_line,
                span.start_col,
                "Multiplication by 0 always returns 0".to_string(),
                DiagnosticSeverity::Warning,
                super::errors::ErrorCode::W107,
                None,
            );
        }

        // W106: Boolean literal comparison.
        if matches!(op, BinOp::Eq | BinOp::NotEq)
            && (is_literal_bool(left, true)
                || is_literal_bool(left, false)
                || is_literal_bool(right, true)
                || is_literal_bool(right, false))
        {
            self.emit_coded(
                span.start_line,
                span.start_col,
                "Comparison with boolean literal is unnecessary".to_string(),
                DiagnosticSeverity::Warning,
                super::errors::ErrorCode::W106,
                None,
            );
        }

        // W106: Self-comparison.
        if matches!(
            op,
            BinOp::Eq | BinOp::NotEq | BinOp::Gt | BinOp::Lt | BinOp::GtEq | BinOp::LtEq
        ) {
            if let (Some(l), Some(r)) = (as_variable(left), as_variable(right)) {
                if l == r {
                    self.emit_coded(
                        span.start_line,
                        span.start_col,
                        format!("Comparing variable '{}' with itself", l),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::W106,
                        None,
                    );
                }
            }
        }

        // W2: Arithmetic on non-numeric types.
        // Exempt Add on strings (string concatenation is valid).
        if matches!(
            op,
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod
        ) {
            let is_string_concat = op == BinOp::Add
                && (left_ty == ChannelType::String || right_ty == ChannelType::String);
            if !is_string_concat {
                for ty in [left_ty, right_ty] {
                    if ty != ChannelType::Null && !is_numeric(ty) {
                        self.emit_coded(
                            span.start_line,
                            span.start_col,
                            format!(
                                "Arithmetic operator '{}' expects numeric operands, got {}",
                                op,
                                ty.as_str()
                            ),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::E103,
                            None,
                        );
                        break; // One warning per operation.
                    }
                }
            }
        }

        // W3: Comparison type mismatch.
        if matches!(
            op,
            BinOp::Eq
                | BinOp::NotEq
                | BinOp::Gt
                | BinOp::Lt
                | BinOp::GtEq
                | BinOp::LtEq
        ) && left_ty != ChannelType::Null
            && right_ty != ChannelType::Null
            && left_ty != right_ty
            // Allow cross-numeric comparisons (e.g. int64 == float64).
            && !(is_numeric(left_ty) && is_numeric(right_ty))
        {
            self.emit_coded(
                span.start_line,
                span.start_col,
                format!("Comparing {} with {}", left_ty.as_str(), right_ty.as_str()),
                DiagnosticSeverity::Warning,
                super::errors::ErrorCode::E100,
                None,
            );
        }
    }

    // =========================================================================
    // Annotation reconciliation
    // =========================================================================

    /// If a type annotation is present, parse it and check compatibility with
    /// the inferred type. Returns the definitive type (annotation wins if valid).
    fn reconcile_annotation(
        &mut self,
        annotation: Option<&str>,
        inferred: ChannelType,
        line: u32,
        col: u32,
        var_name: &str,
    ) -> ChannelType {
        let ann_str = match annotation {
            Some(s) => s,
            None => return inferred,
        };

        let ann_type = match ChannelType::parse(ann_str) {
            Some(ct) => ct,
            None => {
                self.emit_coded(
                    line,
                    col,
                    format!("Unknown type annotation '{}' on '{}'", ann_str, var_name),
                    DiagnosticSeverity::Error,
                    super::errors::ErrorCode::E100,
                    None,
                );
                return inferred;
            }
        };

        // If the inferred type is Null (unknown), trust the annotation.
        if inferred == ChannelType::Null {
            return ann_type;
        }

        // If annotation and inferred match, great.
        if inferred == ann_type || inferred.is_compatible_with(&ann_type) {
            return ann_type;
        }

        // Mismatch.
        self.emit_coded(
            line,
            col,
            format!(
                "Type annotation '{}' on '{}' conflicts with inferred type '{}'",
                ann_str,
                var_name,
                inferred.as_str()
            ),
            DiagnosticSeverity::Warning,
            super::errors::ErrorCode::E100,
            None,
        );
        ann_type
    }

    // =========================================================================
    // Finalize: collect unused variables/imports, build result
    // =========================================================================

    fn finalize(mut self) -> AstTypeAnalysis {
        // Collect variable types from all scopes before popping.
        let mut variable_types = HashMap::new();
        for scope in &self.env {
            for (name, info) in scope {
                if !name.starts_with('_') {
                    variable_types.insert(name.clone(), info.channel_type);
                }
            }
        }

        // Pop all remaining scopes (usually just the global scope), collecting
        // unused-variable and unnecessary-mut warnings.
        while let Some(scope) = self.env.pop() {
            for (name, info) in &scope {
                if name.starts_with('_') {
                    continue;
                }
                if !info.used {
                    let code = super::errors::ErrorCode::W100;
                    self.diagnostics.push(AstDiagnostic {
                        line: info.def_line,
                        column: info.def_col,
                        message: format!("Unused variable '{}'", name),
                        severity: DiagnosticSeverity::Warning,
                        code: Some(code.to_string()),
                        help: Some(code.help().to_string()),
                        suggestion: None,
                    });
                } else if info.mutable && !info.mutated {
                    let code = super::errors::ErrorCode::W110;
                    self.diagnostics.push(AstDiagnostic {
                        line: info.def_line,
                        column: info.def_col,
                        message: format!("Variable '{}' declared as mutable but never reassigned", name),
                        severity: DiagnosticSeverity::Warning,
                        code: Some(code.to_string()),
                        help: Some(code.help().to_string()),
                        suggestion: None,
                    });
                }
            }
        }

        // Unused functions (skip `main` and `_`-prefixed).
        for (name, sig) in &self.function_sigs {
            if !sig.used && name != "main" && !name.starts_with('_') {
                let code = super::errors::ErrorCode::W103;
                self.diagnostics.push(AstDiagnostic {
                    line: sig.def_line,
                    column: 1,
                    message: format!("Unused function '{}'", name),
                    severity: DiagnosticSeverity::Warning,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: None,
                });
            }
        }

        // Unused imports.
        for import_id in &self.imports {
            if !self.used_imports.contains(import_id) {
                let code = super::errors::ErrorCode::W101;
                self.diagnostics.push(AstDiagnostic {
                    line: 0,
                    column: 0,
                    message: format!("Unused import '{}'", import_id),
                    severity: DiagnosticSeverity::Warning,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: None,
                });
            }
        }

        AstTypeAnalysis {
            diagnostics: self.diagnostics,
            variable_types,
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Check if a `ChannelType` is a numeric type.
fn is_numeric(ct: ChannelType) -> bool {
    matches!(
        ct,
        ChannelType::Int32
            | ChannelType::Int64
            | ChannelType::Uint32
            | ChannelType::Uint64
            | ChannelType::Float32
            | ChannelType::Float64
    )
}

/// Check if a `ChannelType` is an integer type.
fn is_integer(ct: ChannelType) -> bool {
    matches!(
        ct,
        ChannelType::Int32 | ChannelType::Int64 | ChannelType::Uint32 | ChannelType::Uint64
    )
}

/// Promote a set of numeric input types to the widest common type.
///
/// Matches the logic in `abstract_interp::promote_numeric`:
/// - Any float in inputs => Float64
/// - Mixed int sizes => Int64
/// - Same int => same int type
/// - All Null => Null
fn promote_numeric(inputs: &[ChannelType]) -> ChannelType {
    let mut has_float = false;
    let mut has_int = false;
    let mut common: Option<ChannelType> = None;

    for ct in inputs {
        match ct {
            ChannelType::Float32 | ChannelType::Float64 => {
                has_float = true;
            }
            ChannelType::Int32 | ChannelType::Int64 | ChannelType::Uint32 | ChannelType::Uint64 => {
                has_int = true;
                common = Some(match common {
                    None => *ct,
                    Some(prev) if prev == *ct => prev,
                    Some(_) => ChannelType::Int64,
                });
            }
            _ => {} // Null or non-numeric — skip
        }
    }

    if has_float {
        ChannelType::Float64
    } else if has_int {
        common.unwrap_or(ChannelType::Int64)
    } else {
        ChannelType::Null
    }
}

/// Refine the output type of an operation call based on argument types.
///
/// Uses `op_output_type` for the static type; if polymorphic (Null), promotes
/// from the provided argument types using the same logic as
/// `abstract_interp::refine_output_type`.
fn refine_call_output(op: OperationType, arg_types: &[ChannelType]) -> ChannelType {
    use OperationType::*;

    let static_type = op_output_type(op);
    if static_type != ChannelType::Null {
        return static_type;
    }

    match op {
        // Arithmetic: promote based on inputs.
        Add | Subtract | Multiply | Modulo | Power | Min | Max | Negate | Abs | Round | Floor
        | Ceil => promote_numeric(arg_types),

        Sqrt => ChannelType::Float64,

        // Bitwise: integer promotion.
        BitAnd | BitOr | BitXor | BitNot | BitShiftLeft | BitShiftRight => {
            promote_integer(arg_types)
        }

        // Control flow: pass-through first non-Null.
        IfElse => {
            if arg_types.len() >= 2 {
                first_non_null(&arg_types[1..])
            } else {
                ChannelType::Null
            }
        }
        Switch | Coalesce | TryCatch | Default => first_non_null(arg_types),
        DebugLog => first_non_null(arg_types),
        Error | Sleep => ChannelType::Null,

        // Element access: unknown element type.
        ArrayGet | ArrayFind | Reduce | MapGet => ChannelType::Null,

        // JSON: dynamic.
        ParseJson | JsonGet | JsonSet | JsonDelete | JsonMerge => ChannelType::Null,

        // Math aggregate on arrays.
        MathSum | MathProduct | MathMinOf | MathMaxOf => ChannelType::Null,

        // Fallback.
        _ => static_type,
    }
}

/// Promote integer input types.
fn promote_integer(inputs: &[ChannelType]) -> ChannelType {
    let mut common: Option<ChannelType> = None;

    for ct in inputs {
        match ct {
            ChannelType::Int32 | ChannelType::Int64 | ChannelType::Uint32 | ChannelType::Uint64 => {
                common = Some(match common {
                    None => *ct,
                    Some(prev) if prev == *ct => prev,
                    Some(_) => ChannelType::Int64,
                });
            }
            _ => {}
        }
    }

    common.unwrap_or(ChannelType::Null)
}

/// Return the first non-Null type in a slice.
fn first_non_null(types: &[ChannelType]) -> ChannelType {
    types
        .iter()
        .find(|ct| **ct != ChannelType::Null)
        .copied()
        .unwrap_or(ChannelType::Null)
}

/// Extract a literal integer from an expression (including negated literals).
fn literal_int(expr: &Expression) -> Option<i64> {
    match &expr.kind {
        ExpressionKind::Literal(Literal::Int64(v)) => Some(*v),
        ExpressionKind::UnaryOp {
            op: UnOp::Neg,
            operand,
        } => {
            if let ExpressionKind::Literal(Literal::Int64(v)) = &operand.kind {
                Some(-v)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Extract a literal float from an expression (including negated literals).
fn literal_float(expr: &Expression) -> Option<f64> {
    match &expr.kind {
        ExpressionKind::Literal(Literal::Float64(v)) => Some(*v),
        ExpressionKind::UnaryOp {
            op: UnOp::Neg,
            operand,
        } => {
            if let ExpressionKind::Literal(Literal::Float64(v)) = &operand.kind {
                Some(-v)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Check if an expression is a specific boolean literal.
fn is_literal_bool(expr: &Expression, val: bool) -> bool {
    matches!(&expr.kind, ExpressionKind::Literal(Literal::Bool(b)) if *b == val)
}

/// Extract the variable name from an expression, if it is a simple variable reference.
fn as_variable(expr: &Expression) -> Option<&str> {
    match &expr.kind {
        ExpressionKind::Variable(name) => Some(name.as_str()),
        _ => None,
    }
}

/// Check if a block is empty (no statements and no tail expression).
fn is_empty_block(block: &Block) -> bool {
    block.statements.is_empty() && block.tail_expr.is_none()
}

/// Resolve a method name on a given type to an OperationType name.
/// Returns None if the method is unknown for that type.
fn resolve_method_type(obj_type: ChannelType, method: &str) -> Option<String> {
    match obj_type {
        ChannelType::Array => match method {
            "push" => Some("array_push".into()),
            "pop" => Some("array_pop".into()),
            "len" | "length" => Some("array_length".into()),
            "get" => Some("array_get".into()),
            "set" => Some("array_set".into()),
            "map" => Some("array_map".into()),
            "filter" => Some("array_filter".into()),
            "reduce" => Some("reduce".into()),
            "sort" => Some("sort".into()),
            "reverse" => Some("reverse".into()),
            "contains" => Some("array_contains".into()),
            "find" => Some("array_find".into()),
            "find_index" => Some("array_find_index".into()),
            "flatten" => Some("array_flatten".into()),
            "join" => Some("array_join".into()),
            "slice" => Some("array_slice".into()),
            "concat" => Some("array_concat".into()),
            "unique" => Some("array_unique".into()),
            // HOF methods handled by interpreter directly
            "any" | "all" | "flat_map" | "each" | "sort_by" | "min_by"
            | "max_by" | "partition" | "group_by" | "scan" | "take_while"
            | "skip_while" | "zip" | "enumerate" | "chunk" | "windows" => {
                Some("array_hof".into())
            }
            // Direct methods
            "first" | "last" | "is_empty" | "sum" | "product" | "min"
            | "max" => Some("array_direct".into()),
            _ => None,
        },
        ChannelType::String => match method {
            "len" | "length" => Some("string_length".into()),
            "split" => Some("split".into()),
            "trim" | "trim_start" | "trim_end" => Some("trim".into()),
            "to_upper" | "to_uppercase" => Some("to_upper".into()),
            "to_lower" | "to_lowercase" => Some("to_lower".into()),
            "contains" => Some("string_contains".into()),
            "starts_with" => Some("starts_with".into()),
            "ends_with" => Some("ends_with".into()),
            "replace" => Some("replace".into()),
            "chars" | "lines" => Some("string_chars".into()),
            "repeat" => Some("string_repeat".into()),
            "substring" | "slice" => Some("substring".into()),
            "index_of" => Some("index_of".into()),
            "pad_start" => Some("pad_start".into()),
            "pad_end" => Some("pad_end".into()),
            "reverse" => Some("string_reverse".into()),
            "is_empty" | "is_numeric" | "is_alphabetic" => Some("string_predicate".into()),
            "to_int" | "to_float" => Some("string_convert".into()),
            "char_at" => Some("string_char_at".into()),
            _ => None,
        },
        ChannelType::Map => match method {
            "get" => Some("map_get".into()),
            "set" => Some("map_set".into()),
            "has" => Some("map_has".into()),
            "delete" => Some("map_delete".into()),
            "keys" => Some("map_keys".into()),
            "values" => Some("map_values".into()),
            "entries" => Some("map_entries".into()),
            "merge" => Some("map_merge".into()),
            "len" | "length" | "size" => Some("map_size".into()),
            // HOF methods
            "filter_entries" | "map_values" | "map_keys" => Some("map_hof".into()),
            _ => None,
        },
        ChannelType::Bytes => match method {
            "len" | "length" => Some("bytes_length".into()),
            "slice" => Some("bytes_slice".into()),
            "concat" => Some("bytes_concat".into()),
            "contains" => Some("bytes_contains".into()),
            _ => None,
        },
        ChannelType::Int64 | ChannelType::Int32 | ChannelType::Uint32 | ChannelType::Uint64 => match method {
            "abs" | "sign" | "to_string" | "to_float64" | "to_int64" | "pow" | "min"
            | "max" | "clamp" => Some("numeric_method".into()),
            _ => None,
        },
        ChannelType::Float64 | ChannelType::Float32 => match method {
            "abs" | "round" | "floor" | "ceil" | "sqrt" | "sign" | "to_string"
            | "to_int64" | "pow" | "min" | "max" | "clamp" | "is_nan"
            | "is_infinite" | "ln" | "log2" | "log10" | "sin" | "cos"
            | "tan" => Some("numeric_method".into()),
            _ => None,
        },
        _ => {
            // Generic methods available on any type
            match method {
                "to_string" => Some("to_string".into()),
                "clone" => None, // clone is a no-op at type level
                _ => None,
            }
        }
    }
}

/// Unify a set of types into a single common type.
/// Returns Null if types disagree (after ignoring Nulls).
fn unify_types(types: &[ChannelType]) -> ChannelType {
    let non_null: Vec<ChannelType> = types
        .iter()
        .copied()
        .filter(|t| *t != ChannelType::Null)
        .collect();

    if non_null.is_empty() {
        return ChannelType::Null;
    }

    let first = non_null[0];
    if non_null.iter().all(|t| *t == first) {
        first
    } else {
        // Check if all are numeric — promote
        if non_null.iter().all(|t| is_numeric(*t)) {
            promote_numeric(&non_null)
        } else {
            ChannelType::Null
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parser::parse_v2;

    /// Helper: parse v2 code and type-check it.
    fn check(code: &str) -> AstTypeAnalysis {
        let ast = parse_v2(code).unwrap();
        // Extract imports from AST
        let mut imports = HashSet::new();
        for stmt in &ast.statements {
            if let StatementKind::Import(id) = &stmt.kind {
                imports.insert(id.clone());
            }
        }
        check_types(&ast, &imports)
    }

    fn errors(analysis: &AstTypeAnalysis) -> Vec<&AstDiagnostic> {
        analysis
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .collect()
    }

    fn warnings(analysis: &AstTypeAnalysis) -> Vec<&AstDiagnostic> {
        analysis
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .collect()
    }

    // =========================================================================
    // Variable type inference
    // =========================================================================

    #[test]
    fn test_int_literal_type() {
        let a = check("let x = 42;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_float_literal_type() {
        let a = check("let x = 3.14;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_string_literal_type() {
        let a = check(r#"let x = "hello";"#);
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::String));
    }

    #[test]
    fn test_bool_literal_type() {
        let a = check("let x = true;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Bool));
    }

    #[test]
    fn test_null_literal_type() {
        let a = check("let x = null;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Null));
    }

    #[test]
    fn test_array_literal_type() {
        let a = check("let x = [1, 2, 3];");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Array));
    }

    #[test]
    fn test_type_annotation() {
        let a = check("let x: float64 = 3.14;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_type_annotation_mismatch_warns() {
        let a = check(
            r#"let x: int64 = "hello";
output x;"#,
        );
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("conflicts")));
    }

    // =========================================================================
    // Arithmetic type promotion
    // =========================================================================

    #[test]
    fn test_add_int_int_produces_int() {
        let a = check("let x = 10;\nlet y = 20;\nlet sum = x + y;\noutput sum;");
        assert_eq!(a.variable_types.get("sum"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_add_int_float_promotes_to_float() {
        let a = check("let x = 10;\nlet y = 3.14;\nlet sum = x + y;\noutput sum;");
        assert_eq!(a.variable_types.get("sum"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_divide_always_float() {
        let a = check("let x = 10;\nlet y = 3;\nlet r = x / y;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_multiply_int_int() {
        let a = check("let x = 5;\nlet y = 3;\nlet r = x * y;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
    }

    // =========================================================================
    // Comparison and logical operators
    // =========================================================================

    #[test]
    fn test_comparison_returns_bool() {
        let a = check("let x = 10;\nlet y = 20;\nlet r = x > y;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Bool));
    }

    #[test]
    fn test_equality_returns_bool() {
        let a = check("let x = 10;\nlet y = 10;\nlet r = x == y;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Bool));
    }

    #[test]
    fn test_logical_and_returns_bool() {
        let a = check("let x = true;\nlet y = false;\nlet r = x && y;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Bool));
    }

    #[test]
    fn test_logical_or_returns_bool() {
        let a = check("let x = true;\nlet y = false;\nlet r = x || y;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Bool));
    }

    #[test]
    fn test_logical_non_bool_warns() {
        let a = check("let x = 10;\nlet y = 20;\nlet r = x && y;\noutput r;");
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("should be bool")));
    }

    // =========================================================================
    // Unary operators
    // =========================================================================

    #[test]
    fn test_not_returns_bool() {
        let a = check("let x = true;\nlet r = !x;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Bool));
    }

    #[test]
    fn test_not_non_bool_warns() {
        let a = check("let x = 42;\nlet r = !x;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Logical NOT expects bool")));
    }

    #[test]
    fn test_negate_preserves_type() {
        let a = check("let x = 3.14;\nlet r = -x;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_negate_non_numeric_warns() {
        let a = check(
            r#"let x = "hello";
let r = -x;
output r;"#,
        );
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Negation expects numeric")));
    }

    // =========================================================================
    // Mutability
    // =========================================================================

    #[test]
    fn test_mut_assignment_ok() {
        let a = check("let mut x = 0;\nx = 42;\noutput x;");
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_immutable_assignment_error() {
        let a = check("let x = 0;\nx = 42;\noutput x;");
        let e = errors(&a);
        assert_eq!(e.len(), 1);
        assert!(e[0].message.contains("Cannot assign to immutable"));
    }

    #[test]
    fn test_assignment_updates_type() {
        let a = check("let mut x = 0;\nx = 3.14;\noutput x;");
        // After assignment, x should be Float64
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_assignment_undefined_error() {
        let a = check("y = 42;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("Undefined variable")));
    }

    // =========================================================================
    // Use-before-define
    // =========================================================================

    #[test]
    fn test_undefined_variable_error() {
        let a = check("let r = x + 1;\noutput r;");
        let e = errors(&a);
        assert!(e
            .iter()
            .any(|d| d.message.contains("Undefined variable 'x'")));
    }

    #[test]
    fn test_defined_variable_no_error() {
        let a = check("let x = 10;\nlet r = x + 1;\noutput r;");
        let e = errors(&a);
        assert!(e.is_empty());
    }

    // =========================================================================
    // Unused variables
    // =========================================================================

    #[test]
    fn test_unused_variable_warns() {
        let a = check("let x = 42;");
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("Unused variable 'x'")));
    }

    #[test]
    fn test_used_variable_no_warning() {
        let a = check("let x = 42;\noutput x;");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("Unused")));
    }

    #[test]
    fn test_underscore_prefix_no_unused_warning() {
        let a = check("let _x = 42;");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("Unused")));
    }

    #[test]
    fn test_variable_used_in_closure_no_warning() {
        let a = check("let x = 42;\nlet add = |n| n + x;\noutput add(1);");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("Unused variable")),
            "No variables should be reported as unused. Got: {:?}", w);
    }

    #[test]
    fn test_variable_used_in_nested_function_no_warning() {
        let a = check("let x = 10;\nfn foo() { output x; }\nfoo();");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("Unused variable 'x'")),
            "Variable 'x' used in nested function should not be reported as unused. Got: {:?}", w);
    }

    // =========================================================================
    // Unused imports
    // =========================================================================

    #[test]
    fn test_unused_import_warns() {
        let a = check(r#"import "capture";"#);
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Unused import 'capture'")));
    }

    #[test]
    fn test_used_import_no_warning() {
        let a = check(
            r#"import "capture";
let frame = capture();
output frame;"#,
        );
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("Unused import")));
    }

    // =========================================================================
    // Function calls
    // =========================================================================

    #[test]
    fn test_known_operation_call() {
        let a = check(
            r#"let s = "hello";
let r = to_upper(s);
output r;"#,
        );
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::String));
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_unknown_operation_error() {
        let a = check("let r = foobar(42);");
        let e = errors(&a);
        assert!(e
            .iter()
            .any(|d| d.message.contains("Unknown operation 'foobar'")));
    }

    #[test]
    fn test_operation_type_mismatch_warns() {
        // concat expects (string, string) but got (int64, string)
        let a = check(
            r#"let x = 42;
let y = "hello";
let r = concat(x, y);
output r;"#,
        );
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("Type mismatch")));
    }

    // =========================================================================
    // If/else
    // =========================================================================

    #[test]
    fn test_if_else_unifies_branch_types() {
        let a = check("let c = true;\nlet r = if c { 10 } else { 20 };\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_if_non_bool_condition_warns() {
        let a = check("let c = 42;\nlet r = if c { 10 } else { 20 };\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("condition should be bool")));
    }

    #[test]
    fn test_if_no_else_returns_null() {
        let a = check("let c = true;\nlet r = if c { 10 };\noutput r;");
        // Without else, type is then_ty if else is Null → then_ty wins
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
    }

    // =========================================================================
    // For loops
    // =========================================================================

    #[test]
    fn test_for_loop_iterable_type() {
        let a = check("let items = [1, 2, 3];\nfor _item in items { 0; }");
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_for_loop_non_array_warns() {
        let a = check("let x = 42;\nfor _item in x { 0; }");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("iterable should be array")));
    }

    // =========================================================================
    // While loops
    // =========================================================================

    #[test]
    fn test_while_loop_condition_type() {
        let a = check("let mut c = true;\nwhile c { c = false; }");
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_while_loop_non_bool_warns() {
        let a = check("let mut x = 10;\nwhile x { x = 0; }");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("condition should be bool")));
    }

    // =========================================================================
    // Index and field access
    // =========================================================================

    #[test]
    fn test_array_index_ok() {
        let a = check("let arr = [1, 2, 3];\nlet r = arr[0];\noutput r;");
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_index_non_array_warns() {
        let a = check("let x = 42;\nlet r = x[0];\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Indexing requires array")));
    }

    #[test]
    fn test_index_non_int_warns() {
        let a = check(
            r#"let arr = [1, 2, 3];
let r = arr["key"];
output r;"#,
        );
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("index should be integer")));
    }

    // =========================================================================
    // Range
    // =========================================================================

    #[test]
    fn test_range_returns_array() {
        // range() is parsed as a Call, not ExpressionKind::Range
        // op_input_types for Range are polymorphic (Null), so no type warnings
        let a = check("let r = range(0, 10);\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Array));
        assert!(errors(&a).is_empty());
    }

    // =========================================================================
    // Scoping
    // =========================================================================

    #[test]
    fn test_block_scope_isolation() {
        // Variable defined inside a block shouldn't leak out
        // (In practice our type checker tracks it but the lowering handles scoping)
        let a = check("let x = 10;\nif true { let _y = 20; }\noutput x;");
        assert!(errors(&a).is_empty());
    }

    // =========================================================================
    // Full programs
    // =========================================================================

    #[test]
    fn test_full_program_no_errors() {
        let a = check(
            r#"
import "capture";
import "text-llm";

let threshold = 0.8;
let prompt = "describe this";
let frame = capture(resolution="1080p");
let text = to_string(frame);
let combined = concat(text, prompt);
let response = text-llm(combined, temperature=0.7);
let result = to_upper(response);
output result;
"#,
        );
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_full_program_with_loop() {
        let a = check(
            r#"
let items = [1, 2, 3];
let mut total = 0;
for _item in items {
    total = total + 1;
}
output total;
"#,
        );
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_full_program_with_while() {
        let a = check(
            r#"
let mut count = 0;
let limit = 10;
while count < limit {
    count = count + 1;
}
output count;
"#,
        );
        assert!(errors(&a).is_empty());
        assert_eq!(a.variable_types.get("count"), Some(&ChannelType::Int64));
    }

    // =========================================================================
    // Plugin calls
    // =========================================================================

    #[test]
    fn test_plugin_call_returns_null() {
        let a = check(
            r#"import "capture";
let frame = capture();
output frame;"#,
        );
        assert_eq!(a.variable_types.get("frame"), Some(&ChannelType::Null));
    }

    // =========================================================================
    // Pipe expressions
    // =========================================================================

    #[test]
    fn test_pipe_expression() {
        let a = check(
            r#"let x = "hello";
let r = x |> to_upper(_);
output r;"#,
        );
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::String));
        assert!(errors(&a).is_empty());
    }

    // =========================================================================
    // User-defined functions
    // =========================================================================

    #[test]
    fn test_fn_return_type_propagates() {
        let a = check("fn double(x: int64) -> int64 { x * 2 }\nlet r = double(5);\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_fn_param_types() {
        let a = check("fn add_nums(a: int64, b: int64) -> int64 { a + b }\nlet r = add_nums(1, 2);\noutput r;");
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_fn_arity_mismatch_error() {
        let a = check("fn one(x: int64) -> int64 { x }\nlet r = one(1, 2);\noutput r;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("expects 1 arguments")));
    }

    #[test]
    fn test_fn_unused_warning() {
        let a = check("fn unused_fn(x: int64) -> int64 { x }");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Unused function 'unused_fn'")));
    }

    #[test]
    fn test_fn_used_no_warning() {
        let a = check("fn double(x: int64) -> int64 { x * 2 }\nlet r = double(5);\noutput r;");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("Unused function")));
    }

    #[test]
    fn test_fn_main_no_unused_warning() {
        let a = check("fn main() { let x = 42; output x; }");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("Unused function")));
    }

    #[test]
    fn test_fn_recursive_call_types() {
        let a = check("fn factorial(n: int64) -> int64 {\n    if n == 0 { 1 } else { n * factorial(n - 1) }\n}\nlet r = factorial(5);\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_fn_unknown_still_errors() {
        let a = check("let r = totally_unknown(42);");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("Unknown operation")));
    }

    #[test]
    fn test_fn_untyped_return_is_null() {
        let a = check("fn notype(x: int64) { x; }\nlet r = notype(5);\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Null));
    }

    // =========================================================================
    // Reserved keyword warnings
    // =========================================================================

    #[test]
    fn test_reserved_keyword_warning_in_define_var() {
        // Direct unit test of the type checker's define_var warning
        let imports = HashSet::new();
        let mut checker = TypeChecker::new(&imports);
        checker.define_var("trait", ChannelType::Int64, false, 1, 1);
        let result = checker.finalize();
        let w = result
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .collect::<Vec<_>>();
        assert!(w.iter().any(|d| d.message.contains("reserved keyword")));
    }

    #[test]
    fn test_non_reserved_name_no_warning() {
        let imports = HashSet::new();
        let mut checker = TypeChecker::new(&imports);
        checker.define_var("my_var", ChannelType::Int64, false, 1, 1);
        // Mark as used to avoid unused warnings
        if let Some(info) = checker.lookup_mut("my_var") {
            info.used = true;
        }
        let result = checker.finalize();
        let w = result
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .collect::<Vec<_>>();
        assert!(w.iter().all(|d| !d.message.contains("reserved keyword")));
    }

    #[test]
    fn test_fn_param_type_mismatch_is_error() {
        let a = check(
            r#"fn expects_int(x: int64) -> int64 { x }
let r = expects_int("hello");
output r;"#,
        );
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("Type mismatch")));
    }

    // =========================================================================
    // Linting diagnostics — errors
    // =========================================================================

    #[test]
    fn test_division_by_zero_error() {
        let a = check("let x = 10;\nlet r = x / 0;\noutput r;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("Division by zero")));
    }

    #[test]
    fn test_modulo_by_zero_error() {
        let a = check("let x = 10;\nlet r = x % 0;\noutput r;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("Division by zero")));
    }

    #[test]
    fn test_division_by_float_zero_error() {
        let a = check("let x = 10.0;\nlet r = x / 0.0;\noutput r;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("Division by zero")));
    }

    #[test]
    fn test_negative_array_index_error() {
        let a = check("let arr = [1, 2, 3];\nlet r = arr[-1];\noutput r;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("Negative array index")));
    }

    #[test]
    fn test_empty_array_index_error() {
        let a = check("let r = [][0];\noutput r;");
        let e = errors(&a);
        assert!(e
            .iter()
            .any(|d| d.message.contains("Index into empty array literal")));
    }

    #[test]
    fn test_duplicate_map_keys_error() {
        let a = check(r#"let m = {"a": 1, "a": 2}; output m;"#);
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("Duplicate key 'a'")));
    }

    #[test]
    fn test_placeholder_outside_pipe_error() {
        let a = check("let x = _;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d
            .message
            .contains("Placeholder '_' can only be used inside pipe")));
    }

    #[test]
    fn test_placeholder_inside_pipe_ok() {
        let a = check(
            r#"let x = "hello";
let r = x |> to_upper(_);
output r;"#,
        );
        let e = errors(&a);
        assert!(e.iter().all(|d| !d.message.contains("Placeholder")));
    }

    #[test]
    fn test_unknown_type_annotation_is_error() {
        let a = check("let x: foobar = 1;\noutput x;");
        let e = errors(&a);
        assert!(e
            .iter()
            .any(|d| d.message.contains("Unknown type annotation")));
    }

    // =========================================================================
    // Linting diagnostics — warnings
    // =========================================================================

    #[test]
    fn test_variable_shadowing_warns() {
        let a = check("let x = 1;\nlet x = 2;\noutput x;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("shadows previous definition")));
    }

    #[test]
    fn test_variable_shadowing_different_scope_ok() {
        let a = check("let x = 1;\nif true { let _x = 2; }\noutput x;");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("shadows")));
    }

    #[test]
    fn test_arithmetic_on_string_warns() {
        // String subtraction should warn (only + is valid for string concat)
        let a = check(r#"let r = "a" - 1; output r;"#);
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Arithmetic operator") && d.message.contains("string")));
    }

    #[test]
    fn test_comparison_type_mismatch_warns() {
        let a = check(r#"let r = 42 == "hello"; output r;"#);
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Comparing int64 with string")));
    }

    #[test]
    fn test_comparison_numeric_cross_ok() {
        let a = check("let r = 42 == 3.14;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .all(|d| !d.message.starts_with("Comparing") || d.message.contains("variable")));
    }

    #[test]
    fn test_bool_literal_comparison_warns() {
        let a = check("let x = true;\nlet r = x == true;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Comparison with boolean literal")));
    }

    #[test]
    fn test_empty_for_body_warns() {
        let a = check("let items = [1, 2, 3];\nfor _x in items {}");
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("Empty loop body")));
    }

    #[test]
    fn test_empty_while_body_warns() {
        let a = check("let mut c = true;\nwhile c {}");
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("Empty loop body")));
    }

    #[test]
    fn test_infinite_while_warns() {
        let a = check("while true { 1; }");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Loop condition is always true")));
    }

    #[test]
    fn test_empty_range_warns() {
        let a = check("let r = range(5, 0);\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Range will produce empty array")));
    }

    #[test]
    fn test_double_negation_warns() {
        let a = check("let x = 5;\nlet r = --x;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Double negation is redundant")));
    }

    #[test]
    fn test_double_not_warns() {
        let a = check("let x = true;\nlet r = !!x;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Double logical NOT is redundant")));
    }

    #[test]
    fn test_modulo_by_one_warns() {
        let a = check("let x = 10;\nlet r = x % 1;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Modulo by 1 always returns 0")));
    }

    #[test]
    fn test_multiply_by_zero_warns() {
        let a = check("let x = 10;\nlet r = x * 0;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Multiplication by 0 always returns 0")));
    }

    #[test]
    fn test_self_comparison_warns() {
        let a = check("let x = 5;\nlet r = x == x;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Comparing variable 'x' with itself")));
    }

    // =========================================================================
    // Return type validation
    // =========================================================================

    #[test]
    fn test_fn_return_type_mismatch_is_error() {
        let a = check(
            r#"fn bad() -> int64 { "not an int" }
let r = bad();
output r;"#,
        );
        let e = errors(&a);
        assert!(e.iter().any(|d| d
            .message
            .contains("declares return type 'int64' but body evaluates to 'string'")));
    }

    #[test]
    fn test_fn_return_type_match_ok() {
        let a = check(
            r#"fn good() -> int64 { 42 }
let r = good();
output r;"#,
        );
        let e = errors(&a);
        assert!(e
            .iter()
            .all(|d| !d.message.contains("declares return type")));
    }

    #[test]
    fn test_fn_no_return_type_no_validation() {
        // Functions without a declared return type should not trigger validation
        let a = check(
            r#"fn flexible() { "anything" }
let r = flexible();
output r;"#,
        );
        let e = errors(&a);
        assert!(e
            .iter()
            .all(|d| !d.message.contains("declares return type")));
    }

    // =========================================================================
    // If/else branch type mismatch warning
    // =========================================================================

    #[test]
    fn test_if_else_branch_mismatch_warns() {
        let a = check(r#"let c = true; let r = if c { 42 } else { "text" }; output r;"#);
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("If/else branches have mismatched types")));
    }

    #[test]
    fn test_if_else_same_type_no_warning() {
        let a = check("let c = true; let r = if c { 1 } else { 2 }; output r;");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("mismatched types")));
    }

    // =========================================================================
    // Gap #4: break/continue/return outside valid context
    // =========================================================================

    #[test]
    fn test_break_outside_loop_error() {
        let a = check("break;");
        let e = errors(&a);
        assert!(
            e.iter()
                .any(|d| d.message.contains("'break' used outside of a loop")),
            "Expected break-outside-loop error, got: {:?}",
            e
        );
    }

    #[test]
    fn test_continue_outside_loop_error() {
        let a = check("continue;");
        let e = errors(&a);
        assert!(
            e.iter()
                .any(|d| d.message.contains("'continue' used outside of a loop")),
            "Expected continue-outside-loop error, got: {:?}",
            e
        );
    }

    #[test]
    fn test_return_outside_function_error() {
        let a = check("return 42;");
        let e = errors(&a);
        assert!(
            e.iter()
                .any(|d| d.message.contains("'return' used outside of a function")),
            "Expected return-outside-function error, got: {:?}",
            e
        );
    }

    #[test]
    fn test_break_inside_loop_ok() {
        let a = check("for x in [1, 2, 3] { break; }\noutput 0;");
        let e = errors(&a);
        assert!(
            e.iter().all(|d| !d.message.contains("outside of a loop")),
            "break inside loop should not produce error, got: {:?}",
            e
        );
    }

    #[test]
    fn test_continue_inside_loop_ok() {
        let a = check("for x in [1, 2, 3] { continue; }\noutput 0;");
        let e = errors(&a);
        assert!(
            e.iter().all(|d| !d.message.contains("outside of a loop")),
            "continue inside loop should not produce error, got: {:?}",
            e
        );
    }

    #[test]
    fn test_return_inside_function_ok() {
        let a = check("fn f() -> int64 { return 1; }\nlet r = f();\noutput r;");
        let e = errors(&a);
        assert!(
            e.iter()
                .all(|d| !d.message.contains("outside of a function")),
            "return inside function should not produce error, got: {:?}",
            e
        );
    }

    // =========================================================================
    // Destructuring
    // =========================================================================

    #[test]
    fn test_array_destructure_defines_vars() {
        let a = check("let [a, b] = [1, 2];\noutput a;\noutput b;");
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_array_destructure_non_array_warns() {
        let a = check("let [a, b] = 42;\noutput a;\noutput b;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Array destructuring requires array")));
    }

    #[test]
    fn test_map_destructure_defines_vars() {
        let a = check(r#"let {x, y} = {"x": 1, "y": 2}; output x; output y;"#);
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_map_destructure_non_map_warns() {
        let a = check("let {x, y} = [1, 2];\noutput x;\noutput y;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Map destructuring requires map")));
    }

    #[test]
    fn test_destructure_rest_is_array() {
        let a = check("let [first, ...rest] = [1, 2, 3];\noutput first;\noutput rest;");
        assert!(errors(&a).is_empty());
        assert_eq!(a.variable_types.get("rest"), Some(&ChannelType::Array));
    }

    // =========================================================================
    // Compound assignment
    // =========================================================================

    #[test]
    fn test_compound_assign_mut_ok() {
        let a = check("let mut x = 10;\nx += 5;\noutput x;");
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_compound_assign_immutable_error() {
        let a = check("let x = 10;\nx += 5;\noutput x;");
        let e = errors(&a);
        assert!(e
            .iter()
            .any(|d| d.message.contains("Cannot assign to immutable")));
    }

    #[test]
    fn test_compound_assign_undefined_error() {
        let a = check("y += 5;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("Undefined variable")));
    }

    // =========================================================================
    // Try/catch
    // =========================================================================

    #[test]
    fn test_try_catch_no_errors() {
        let a = check(
            r#"try {
    let _x = 42;
} catch err {
    output err;
}"#,
        );
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_try_catch_error_var_is_string() {
        let a = check(
            r#"let mut result = "";
try {
    result = "ok";
} catch err {
    result = err;
}
output result;"#,
        );
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_try_catch_finally() {
        let a = check(
            r#"try {
    let _x = 42;
} catch _err {
    0;
} finally {
    let _cleanup = true;
}"#,
        );
        assert!(errors(&a).is_empty());
    }

    // =========================================================================
    // Const
    // =========================================================================

    #[test]
    fn test_const_defines_variable() {
        let a = check("const PI = 3.14;\noutput PI;");
        assert_eq!(a.variable_types.get("PI"), Some(&ChannelType::Float64));
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_const_with_type_annotation() {
        let a = check("const MAX: int64 = 100;\noutput MAX;");
        assert_eq!(a.variable_types.get("MAX"), Some(&ChannelType::Int64));
        assert!(errors(&a).is_empty());
    }

    // =========================================================================
    // Type alias
    // =========================================================================

    #[test]
    fn test_type_alias_valid() {
        let a = check("type Number = int64;");
        // Valid type alias should not produce errors
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_type_alias_unknown_target_warns() {
        let a = check("type Foo = nonexistent;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Unknown type 'nonexistent'")));
    }

    // =========================================================================
    // Module definitions
    // =========================================================================

    #[test]
    fn test_module_scoping() {
        let a = check(
            r#"mod math {
    fn _double(x: int64) -> int64 { x * 2 }
}"#,
        );
        assert!(errors(&a).is_empty());
    }

    // =========================================================================
    // Use statements
    // =========================================================================

    #[test]
    fn test_use_known_std_module() {
        let a = check("use std::math::sqrt;");
        // Known module — no warnings
        let w = warnings(&a);
        assert!(w
            .iter()
            .all(|d| !d.message.contains("Unknown standard library module")));
    }

    #[test]
    fn test_use_unknown_std_module_warns() {
        let a = check("use std::nonexistent::thing;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Unknown standard library module")));
    }

    // =========================================================================
    // Method calls
    // =========================================================================

    #[test]
    fn test_array_method_push() {
        let a = check("let arr = [1, 2, 3];\nlet r = arr.push(4);\noutput r;");
        // push returns array
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Array));
    }

    #[test]
    fn test_string_method_to_upper() {
        let a = check(r#"let s = "hello"; let r = s.to_upper(); output r;"#);
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::String));
    }

    #[test]
    fn test_string_method_split() {
        let a = check(r#"let s = "a,b,c"; let r = s.split(","); output r;"#);
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Array));
    }

    #[test]
    fn test_map_method_keys() {
        let a = check(r#"let m = {"a": 1}; let r = m.keys(); output r;"#);
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Array));
    }

    #[test]
    fn test_unknown_method_warns() {
        let a = check("let arr = [1, 2, 3];\nlet r = arr.nonexistent();\noutput r;");
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("Unknown method")));
    }

    // =========================================================================
    // Lambda expressions
    // =========================================================================

    #[test]
    fn test_lambda_no_errors() {
        let a = check("let _f = |x| x + 1;");
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_lambda_body_type_checked() {
        // The body of a lambda should be type checked
        let a = check("let _f = |x: int64| x / 0;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("Division by zero")));
    }

    // =========================================================================
    // Match expressions
    // =========================================================================

    #[test]
    fn test_match_unifies_arm_types() {
        let a = check(
            r#"let x = 1;
let r = match x {
    1 => 10,
    2 => 20,
    _ => 30,
};
output r;"#,
        );
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_match_no_catchall_warns() {
        let a = check(
            r#"let x = 1;
let r = match x {
    1 => 10,
    2 => 20,
};
output r;"#,
        );
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("Non-exhaustive match")));
    }

    #[test]
    fn test_match_empty_warns() {
        let a = check("let x = 1;\nlet r = match x {};\noutput r;");
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("Empty match")));
    }

    #[test]
    fn test_match_with_guard() {
        let a = check(
            r#"let x = 5;
let r = match x {
    n if n > 0 => 1,
    _ => 0,
};
output r;"#,
        );
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_match_guard_non_bool_warns() {
        let a = check(
            r#"let x = 5;
let r = match x {
    n if n + 1 => 1,
    _ => 0,
};
output r;"#,
        );
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("guard should be bool")));
    }

    // =========================================================================
    // String interpolation
    // =========================================================================

    #[test]
    fn test_string_interp_returns_string() {
        let a = check(r#"let name = "world"; let r = f"hello {name}"; output r;"#);
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::String));
    }

    #[test]
    fn test_string_interp_checks_inner_expr() {
        let a = check(r#"let r = f"value: {undefined_var}"; output r;"#);
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("Undefined variable")));
    }

    // =========================================================================
    // Null coalescing
    // =========================================================================

    #[test]
    fn test_null_coalesce_returns_right_type() {
        let a = check("let x = null;\nlet r = x ?? 42;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_null_coalesce_returns_left_type_when_not_null() {
        let a = check(r#"let x = "hello"; let r = x ?? "default"; output r;"#);
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::String));
    }

    // =========================================================================
    // Optional chaining
    // =========================================================================

    #[test]
    fn test_optional_chain_returns_null() {
        let a = check(r#"let m = {"a": 1}; let r = m?.a; output r;"#);
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Null));
    }

    #[test]
    fn test_optional_chain_non_map_warns() {
        let a = check("let x = 42;\nlet r = x?.field;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Optional chaining requires map")));
    }

    // =========================================================================
    // Spread
    // =========================================================================

    #[test]
    fn test_spread_non_array_warns() {
        let a = check("let x = 42;\nlet r = [...x];\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("Spread requires array")));
    }

    // =========================================================================
    // Loop expression
    // =========================================================================

    #[test]
    fn test_loop_empty_body_warns() {
        let a = check("let _r = loop {};");
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("Empty loop body")));
    }

    #[test]
    fn test_loop_break_inside_ok() {
        let a = check(
            r#"let _r = loop {
    break 42;
};"#,
        );
        assert!(errors(&a).is_empty());
    }

    // =========================================================================
    // Try/catch expression
    // =========================================================================

    #[test]
    fn test_try_catch_expr_unifies_types() {
        let a = check(
            r#"let r = try { 42 } catch _err { 0 };
output r;"#,
        );
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_try_catch_expr_error_var_scoped() {
        let a = check(
            r#"let r = try { 42 } catch err {
    output err;
    0
};
output r;"#,
        );
        assert!(errors(&a).is_empty());
    }

    // =========================================================================
    // Default parameters
    // =========================================================================

    #[test]
    fn test_fn_default_params_accept_fewer_args() {
        let a = check(
            r#"fn greet(name: string, greeting: string = "hello") -> string {
    concat(greeting, name)
}
let r = greet("world");
output r;"#,
        );
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_fn_default_params_accept_all_args() {
        let a = check(
            r#"fn greet(name: string, greeting: string = "hello") -> string {
    concat(greeting, name)
}
let r = greet("world", "hi");
output r;"#,
        );
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_fn_default_params_too_few_error() {
        let a = check(
            r#"fn greet(name: string, greeting: string = "hello") -> string {
    concat(greeting, name)
}
let r = greet();
output r;"#,
        );
        let e = errors(&a);
        assert!(e
            .iter()
            .any(|d| d.message.contains("expects 1-2 arguments")));
    }

    // =========================================================================
    // Throw
    // =========================================================================

    #[test]
    fn test_throw_type_checks_expr() {
        let a = check(r#"throw "error message";"#);
        assert!(errors(&a).is_empty());
    }

    // =========================================================================
    // Combined new features
    // =========================================================================

    #[test]
    fn test_combined_destructure_and_match() {
        let a = check(
            r#"let [a, b, c] = [1, 2, 3];
let r = match a {
    1 => b + c,
    _ => 0,
};
output r;"#,
        );
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_combined_try_catch_with_compound() {
        let a = check(
            r#"let mut total = 0;
try {
    total += 10;
    total += 20;
} catch _err {
    total = 0;
}
output total;"#,
        );
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_combined_loop_with_method_call() {
        let a = check(
            r#"let items = [1, 2, 3];
let r = items.length();
output r;"#,
        );
        assert!(errors(&a).is_empty());
    }

    // --- Error codes and suggestions in diagnostics ---

    #[test]
    fn test_diagnostic_code_for_undefined_variable() {
        let a = check("let r = xyz + 1; output r;");
        let errs = errors(&a);
        assert!(!errs.is_empty());
        let d = &errs[0];
        assert!(d.message.contains("Undefined variable"));
        assert_eq!(d.code.as_deref(), Some("E200"));
        assert!(d.help.is_some());
    }

    #[test]
    fn test_diagnostic_suggestion_for_typo() {
        let a = check("let count = 10;\nlet r = counr + 1;\noutput r;");
        let errs = errors(&a);
        assert!(!errs.is_empty());
        let d = errs
            .iter()
            .find(|d| d.message.contains("Undefined"))
            .unwrap();
        assert_eq!(d.code.as_deref(), Some("E200"));
        assert_eq!(d.suggestion.as_deref(), Some("did you mean 'count'?"));
    }

    #[test]
    fn test_diagnostic_code_for_immutable_assign() {
        let a = check("let x = 1;\nx = 2;");
        let errs = errors(&a);
        let d = errs
            .iter()
            .find(|d| d.message.contains("immutable"))
            .unwrap();
        assert_eq!(d.code.as_deref(), Some("E404"));
    }

    #[test]
    fn test_diagnostic_code_for_unused_variable() {
        let a = check("let x = 42;");
        let warns = warnings(&a);
        let d = warns
            .iter()
            .find(|d| d.message.contains("Unused variable"))
            .unwrap();
        assert_eq!(d.code.as_deref(), Some("W100"));
        assert!(d.help.is_some());
    }

    #[test]
    fn test_diagnostic_code_for_unused_function() {
        let a = check("fn unused_fn(x: int64) -> int64 { x }");
        let warns = warnings(&a);
        let d = warns
            .iter()
            .find(|d| d.message.contains("Unused function"))
            .unwrap();
        assert_eq!(d.code.as_deref(), Some("W103"));
    }

    #[test]
    fn test_diagnostic_code_for_unused_import() {
        let a = check("import \"capture\";");
        let warns = warnings(&a);
        let d = warns
            .iter()
            .find(|d| d.message.contains("Unused import"))
            .unwrap();
        assert_eq!(d.code.as_deref(), Some("W101"));
    }

    #[test]
    fn test_diagnostic_code_for_shadowing() {
        let a = check("let x = 1;\nlet x = 2;\noutput x;");
        let warns = warnings(&a);
        let d = warns
            .iter()
            .find(|d| d.message.contains("shadows"))
            .unwrap();
        assert_eq!(d.code.as_deref(), Some("W102"));
    }

    #[test]
    fn test_diagnostic_no_suggestion_for_distant_name() {
        let a = check("let r = xyz + 1; output r;");
        let errs = errors(&a);
        let d = errs
            .iter()
            .find(|d| d.message.contains("Undefined"))
            .unwrap();
        assert!(
            d.suggestion.is_none(),
            "No suggestion expected for distant names"
        );
    }

    #[test]
    fn test_test_def_body_type_checked() {
        // Errors inside test body should be caught
        let a = check(r#"test "uses undefined" { let r = xyz + 1; }"#);
        let errs = errors(&a);
        assert!(
            errs.iter().any(|d| d.message.contains("Undefined")),
            "Type checker should find errors inside test body: {:?}",
            errs
        );
    }

    #[test]
    fn test_test_def_scope_isolation() {
        // Variables defined in one test should not leak to outer scope
        let a = check(
            r#"test "first" { let inner = 42; }
let r = inner;"#,
        );
        let errs = errors(&a);
        assert!(
            errs.iter().any(|d| d.message.contains("Undefined")),
            "Variable from test body should not leak to outer scope: {:?}",
            errs
        );
    }

    #[test]
    fn test_test_def_can_access_outer_scope() {
        // Tests should be able to access variables from outer scope
        let a = check(
            r#"let x = 42;
test "reads outer" { let r = x + 1; output r; }"#,
        );
        let errs = errors(&a);
        let undef_errs: Vec<_> = errs
            .iter()
            .filter(|d| d.message.contains("Undefined"))
            .collect();
        assert!(
            undef_errs.is_empty(),
            "Test body should access outer scope variables: {:?}",
            undef_errs
        );
    }
}
