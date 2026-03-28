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

// Constant expression values (for constant folding)

/// A compile-time constant value produced by constant folding.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstLiteral {
    Int64(i64),
    Float64(f64),
    String(String),
    Bool(bool),
}


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
    /// Source file path for multi-file error aggregation (#135).
    pub source_file: Option<String>,
}

// Internal: function signature tracking

/// Tracked signature for a user-defined function.
#[derive(Clone)]
struct FunctionSig {
    params: Vec<(String, ChannelType)>,
    /// Number of required parameters (those without default values and not rest).
    required_params: usize,
    /// Whether the function has a rest parameter (...args).
    has_rest: bool,
    return_type: ChannelType,
    def_line: u32,
    def_col: u32,
    used: bool,
    /// Whether this function is marked `#[deprecated]`.
    deprecated: bool,
}

// Internal: variable tracking

/// Metadata tracked per variable binding.
struct VarInfo {
    channel_type: ChannelType,
    /// Original declared type annotation (if any). Used to warn on incompatible reassignment.
    declared_type: Option<ChannelType>,
    mutable: bool,
    used: bool,
    mutated: bool,
    is_param: bool,
    is_const: bool,
    def_line: u32,
    def_col: u32,
}


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

/// Type-check a parsed AST program, attributing all diagnostics to the given source file (#135).
///
/// Same as [`check_types`] but stamps each diagnostic with `source_file`.
pub fn check_types_with_source(
    program: &Program,
    imports: &HashSet<String>,
    source_file: &str,
) -> AstTypeAnalysis {
    let mut result = check_types(program, imports);
    let file = source_file.to_string();
    for diag in &mut result.diagnostics {
        diag.source_file = Some(file.clone());
    }
    result
}

// TypeChecker

struct TypeChecker {
    /// Scope stack — innermost scope is last.
    env: Vec<HashMap<String, VarInfo>>,
    diagnostics: Vec<AstDiagnostic>,
    /// Known plugin imports with their source locations.
    imports: HashSet<String>,
    /// Import source locations: import_id → (line, col).
    import_locations: HashMap<String, (u32, u32)>,
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
    /// Collected return statement types for the current function (for return type inference).
    collected_return_types: Vec<ChannelType>,
    /// Known enum definitions: enum_name → list of (variant_name, field_count).
    enum_variants: HashMap<String, Vec<(String, usize)>>,
    /// Known struct definitions: struct_name → list of (field_name, type_annotation).
    struct_defs: HashMap<String, Vec<(String, Option<String>)>>,
    /// Tracks seen `use` import paths for duplicate detection.
    seen_imports: HashSet<String>,
    /// Names brought into scope by `use` statements (suppresses E201).
    use_aliases: HashSet<String>,
    /// Type alias definitions: alias name → target type string.
    type_aliases: HashMap<String, String>,
    /// Known constant values for const propagation.
    const_values: HashMap<String, ChannelType>,
    /// Computed constant literal values for constant folding.
    const_literals: HashMap<String, ConstLiteral>,
    /// Known trait definitions: trait_name → list of (method_name, param_count, line, col, return_type).
    trait_defs: HashMap<String, Vec<(String, usize, u32, u32, Option<String>)>>,
    /// Active generic type parameters in scope (from current function definition).
    generic_params: HashSet<String>,
    /// Generic parameter bounds: type_param_name → list of trait bound names.
    generic_bounds: HashMap<String, Vec<String>>,
    /// Generic function signatures: name → list of type parameter names.
    generic_fn_params: HashMap<String, Vec<String>>,
}

impl TypeChecker {
    fn new(imports: &HashSet<String>) -> Self {
        Self {
            env: vec![HashMap::new()], // start with one global scope
            diagnostics: Vec::new(),
            imports: imports.clone(),
            import_locations: HashMap::new(),
            used_imports: HashSet::new(),
            function_sigs: HashMap::new(),
            enum_variants: HashMap::new(),
            struct_defs: HashMap::new(),
            seen_imports: HashSet::new(),
            use_aliases: HashSet::new(),
            type_aliases: HashMap::new(),
            const_values: HashMap::new(),
            const_literals: HashMap::new(),
            trait_defs: HashMap::new(),
            generic_params: HashSet::new(),
            generic_bounds: HashMap::new(),
            generic_fn_params: HashMap::new(),
            pipe_depth: 0,
            loop_depth: 0,
            function_depth: 0,
            current_return_type: ChannelType::Null,
            collected_return_types: Vec::new(),
        }
    }


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
                            format!("unused parameter '{}'", name)
                        } else if info.is_const {
                            format!("unused constant '{}'", name)
                        } else {
                            format!("unused variable '{}'", name)
                        },
                        severity: DiagnosticSeverity::Warning,
                        code: Some(code.to_string()),
                        help: Some(code.help().to_string()),
                        suggestion: None,
                        source_file: None,
                    });
                } else if info.mutable && !info.mutated {
                    let code = super::errors::ErrorCode::W110;
                    self.diagnostics.push(AstDiagnostic {
                        line: info.def_line,
                        column: info.def_col,
                        message: format!("variable '{}' declared as mutable but never reassigned", name),
                        severity: DiagnosticSeverity::Warning,
                        code: Some(code.to_string()),
                        help: Some(code.help().to_string()),
                        suggestion: None,
                        source_file: None,
                    });
                }
            }
        }
    }


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
    /// Suggest a variable name using Levenshtein distance.
    fn suggest_variable(&self, name: &str) -> Option<String> {
        // Collect unique variable names from all scopes without cloning into a HashSet
        let mut seen = HashSet::new();
        let refs: Vec<&str> = self.env.iter().rev()
            .flat_map(|scope| scope.keys())
            .filter(|k| seen.insert(k.as_str()))
            .map(|k| k.as_str())
            .collect();
        super::errors::suggest_name(name, &refs)
    }

    /// Suggest a function name using Levenshtein distance.
    fn suggest_function(&self, name: &str) -> Option<String> {
        let refs: Vec<&str> = self.function_sigs.keys().map(|s| s.as_str()).collect();
        super::errors::suggest_name(name, &refs)
    }

    /// Define a variable in the current (innermost) scope.
    fn check_default_param_type(&mut self, default_expr: &Expression, annotation: &ChannelType) {
        let default_type = self.infer_expr(default_expr);
        if *annotation != ChannelType::Null
            && default_type != ChannelType::Null
            && !default_type.is_compatible_with(annotation)
        {
            let code = super::errors::ErrorCode::W112;
            self.diagnostics.push(AstDiagnostic {
                line: default_expr.span.start_line,
                column: default_expr.span.start_col,
                message: format!(
                    "default value type '{}' doesn't match parameter type '{}'",
                    default_type, annotation
                ),
                severity: DiagnosticSeverity::Warning,
                code: Some(code.to_string()),
                help: Some(code.help().to_string()),
                suggestion: None,
                source_file: None,
            });
        }
    }

    fn define_var(&mut self, name: &str, ct: ChannelType, mutable: bool, line: u32, col: u32) {
        if is_reserved_keyword(name) {
            self.emit_coded(
                line,
                col,
                format!("'{}' is a reserved keyword", name),
                DiagnosticSeverity::Warning,
                super::errors::ErrorCode::W111,
                None,
            );
        }
        // W102 (same-scope shadowing) is handled by the linter as W209.
        // Removed from type checker to avoid duplicate diagnostics in LSP.
        if let Some(scope) = self.env.last_mut() {
            scope.insert(
                name.to_string(),
                VarInfo {
                    channel_type: ct,
                    declared_type: None,
                    mutable,
                    used: false,
                    mutated: false,
                    is_param: false,
                    is_const: false,
                    def_line: line,
                    def_col: col,
                },
            );
        }
    }


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
            source_file: None,
        });
    }


    /// Collect type alias definitions from a list of statements.
    /// Handles both top-level and module-scoped aliases (with module:: prefix).
    fn collect_type_aliases(&mut self, statements: &[Statement]) {
        for stmt in statements {
            if let StatementKind::TypeAlias { name, target } = &stmt.kind {
                self.type_aliases.insert(name.clone(), target.to_string());
            }
            // Also collect from module bodies (both qualified and unqualified)
            if let StatementKind::ModuleDef { name: mod_name, body } = &stmt.kind {
                for s in &body.statements {
                    if let StatementKind::TypeAlias { name, target } = &s.kind {
                        let qualified = format!("{}::{}", mod_name, name);
                        self.type_aliases.insert(qualified, target.to_string());
                        // Also register unqualified so it resolves inside the module body
                        self.type_aliases.entry(name.clone()).or_insert_with(|| target.to_string());
                    }
                }
            }
        }
    }

    // Program / statement checking

    fn check_program(&mut self, program: &Program) {
        // Pass 1a: collect type aliases first (so they're available for function sigs)
        self.collect_type_aliases(&program.statements);

        // Pass 1b: collect function signatures, enum definitions, struct definitions, and trait definitions
        for stmt in &program.statements {
            if let StatementKind::EnumDef { name, variants, .. } = &stmt.kind {
                let variant_info: Vec<(String, usize)> = variants.iter().map(|v| (v.name.clone(), v.fields.len())).collect();
                self.enum_variants.insert(name.clone(), variant_info);
            }
            if let StatementKind::StructDef { name, fields, .. } = &stmt.kind {
                let field_info: Vec<(String, Option<String>)> = fields.iter().map(|f| (f.name.clone(), f.type_annotation.as_ref().map(|t| t.to_string()))).collect();
                self.struct_defs.insert(name.clone(), field_info);
            }
            if let StatementKind::TraitDef { name, methods, .. } = &stmt.kind {
                let method_info: Vec<(String, usize, u32, u32, Option<String>)> = methods.iter().map(|m| {
                    (m.name.clone(), m.params.len(), m.span.start_line, m.span.start_col, m.return_type.as_ref().map(|t| t.to_string()))
                }).collect();
                self.trait_defs.insert(name.clone(), method_info);
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
                            .as_ref()
                    .and_then(|ta| self.resolve_type(&ta.to_string()))
                            .unwrap_or(ChannelType::Null);
                        (p.name.clone(), ct)
                    })
                    .collect();
                let has_rest = def.params.iter().any(|p| p.rest);
                let required_params = def.params.iter().filter(|p| p.default.is_none() && !p.rest && !p.kwargs).count();
                let return_type = def
                    .return_type
                    .as_ref()
                    .and_then(|ta| self.resolve_type(&ta.to_string()))
                    .unwrap_or(ChannelType::Null);
                // Check for duplicate function definitions
                if let Some(existing) = self.function_sigs.get(&def.name) {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("duplicate definition of function '{}' (first defined at line {})", def.name, existing.def_line),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::W113,
                        None,
                    );
                }
                self.function_sigs.insert(
                    def.name.clone(),
                    FunctionSig {
                        params,
                        required_params,
                        has_rest,
                        return_type,
                        def_line: stmt.span.start_line,
                        def_col: stmt.span.start_col,
                        used: false,
                        deprecated: def.deprecated,
                    },
                );
            }
            // Pre-register functions, enums, and structs inside module bodies with qualified names
            if let StatementKind::ModuleDef { name: mod_name, body } = &stmt.kind {
                for s in &body.statements {
                    if let StatementKind::FunctionDef(def) | StatementKind::AsyncFunctionDef(def) =
                        &s.kind
                    {
                        let params: Vec<(String, ChannelType)> = def
                            .params
                            .iter()
                            .map(|p| {
                                let ct = p
                                    .type_annotation
                                    .as_ref()
                    .and_then(|ta| self.resolve_type(&ta.to_string()))
                                    .unwrap_or(ChannelType::Null);
                                (p.name.clone(), ct)
                            })
                            .collect();
                        let has_rest = def.params.iter().any(|p| p.rest);
                        let required_params = def.params.iter().filter(|p| p.default.is_none() && !p.rest && !p.kwargs).count();
                        let return_type = def
                            .return_type
                            .as_ref()
                    .and_then(|ta| self.resolve_type(&ta.to_string()))
                            .unwrap_or(ChannelType::Null);
                        let qualified_name = format!("{}::{}", mod_name, def.name);
                        self.function_sigs.insert(
                            qualified_name,
                            FunctionSig {
                                params,
                                required_params,
                                has_rest,
                                return_type,
                                def_line: s.span.start_line,
                                def_col: s.span.start_col,
                                used: false,
                                deprecated: def.deprecated,
                            },
                        );
                    }
                    // Register module-scoped enums with both qualified and unqualified names
                    if let StatementKind::EnumDef { name, variants, .. } = &s.kind {
                        let variant_info: Vec<(String, usize)> = variants.iter().map(|v| (v.name.clone(), v.fields.len())).collect();
                        let qualified = format!("{}::{}", mod_name, name);
                        self.enum_variants.insert(qualified, variant_info.clone());
                        self.enum_variants.entry(name.clone()).or_insert(variant_info);
                    }
                    // Register module-scoped structs with both qualified and unqualified names
                    if let StatementKind::StructDef { name, fields, .. } = &s.kind {
                        let field_info: Vec<(String, Option<String>)> = fields.iter().map(|f| (f.name.clone(), f.type_annotation.as_ref().map(|t| t.to_string()))).collect();
                        let qualified = format!("{}::{}", mod_name, name);
                        self.struct_defs.insert(qualified, field_info.clone());
                        self.struct_defs.entry(name.clone()).or_insert(field_info);
                    }
                }
            }
        }

        // Pass 2: check all statements
        for stmt in &program.statements {
            self.check_statement(stmt);
        }
    }

    fn check_statement(&mut self, stmt: &Statement) {
        match &stmt.kind {
            // import "plugin-id";
            StatementKind::Import(id) => {
                // Record import location for unused import diagnostics.
                self.import_locations.entry(id.clone())
                    .or_insert((stmt.span.start_line, stmt.span.start_col));
            }

            // let name = expr;  /  let name: type = expr;
            StatementKind::Let {
                name,
                type_annotation,
                value,
            } => {
                let inferred = self.infer_expr(value);
                let ct = self.reconcile_annotation(
                    type_annotation.as_ref(),
                    inferred,
                    stmt.span.start_line,
                    stmt.span.start_col,
                    name,
                );
                self.define_var(name, ct, false, stmt.span.start_line, stmt.span.start_col);
            }

            // let mut name = expr;  /  let mut name: type = expr;
            StatementKind::LetMut {
                name,
                type_annotation,
                value,
            } => {
                let inferred = self.infer_expr(value);
                let ct = self.reconcile_annotation(
                    type_annotation.as_ref(),
                    inferred,
                    stmt.span.start_line,
                    stmt.span.start_col,
                    name,
                );
                self.define_var(name, ct, true, stmt.span.start_line, stmt.span.start_col);
                // Store declared type annotation for reassignment checks.
                if let Some(ann) = type_annotation.as_ref().and_then(|ta| self.resolve_type_annotation(ta)) {
                    if let Some(info) = self.lookup_mut(name) {
                        info.declared_type = Some(ann);
                    }
                }
            }

            // name = expr;
            StatementKind::Assignment { name, value } => {
                let new_type = self.infer_expr(value);

                // Check existence first.
                let (exists, is_mutable, declared_type) = match self.lookup(name) {
                    Some(info) => (true, info.mutable, info.declared_type),
                    None => (false, false, None),
                };

                if !exists {
                    let suggestion = self.suggest_variable(name);
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("undefined variable '{}'", name),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E200,
                        suggestion,
                    );
                } else if !is_mutable {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("cannot assign to immutable variable '{}'", name),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E404,
                        None,
                    );
                }

                // Warn if assigning an incompatible type to a type-annotated variable.
                if let Some(decl) = declared_type {
                    if new_type != ChannelType::Null
                        && !new_type.is_compatible_with(&decl)
                    {
                        self.emit_coded(
                            stmt.span.start_line,
                            stmt.span.start_col,
                            format!(
                                "assigning '{}' to variable '{}' declared as '{}'",
                                new_type.as_str(),
                                name,
                                decl.as_str(),
                            ),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::E100,
                            None,
                        );
                    }
                }

                // Update the variable's type and mark used + mutated.
                if let Some(info) = self.lookup_mut(name) {
                    info.channel_type = new_type;
                    info.used = true;
                    info.mutated = true;
                }
            }

            // for item in iterable { body }
            StatementKind::ForLoop {
                label: _,
                pattern,
                iterable,
                body,
            } => {
                let iter_type = self.infer_expr(iterable);
                if iter_type != ChannelType::Array
                    && iter_type != ChannelType::Map
                    && iter_type != ChannelType::String
                    && iter_type != ChannelType::Null
                {
                    self.emit_coded(
                        iterable.span.start_line,
                        iterable.span.start_col,
                        format!(
                            "for-loop iterable should be array, map, or string, got {}",
                            iter_type.as_str()
                        ),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E102,
                        None,
                    );
                }

                // W104 (empty block) is handled by the linter as W206.

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

            // while condition { body }
            StatementKind::WhileLoop { condition, body, .. } => {
                let cond_type = self.infer_expr(condition);
                if cond_type != ChannelType::Bool && cond_type != ChannelType::Null {
                    self.emit_coded(
                        condition.span.start_line,
                        condition.span.start_col,
                        format!("while condition should be bool, got {}", cond_type.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E101,
                        None,
                    );
                }

                // W105 (while-true) is handled by the linter as W204.
                // W104 (empty block) is handled by the linter as W206.

                self.push_scope();
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
            }

            // output expr;
            StatementKind::Output(expr) => {
                let _ = self.infer_expr(expr);
            }

            // expr;
            StatementKind::ExprStatement(expr) => {
                let _ = self.infer_expr(expr);
            }

            // fn name(params) -> type { body }
            StatementKind::FunctionDef(def) | StatementKind::AsyncFunctionDef(def) => {
                self.push_scope();
                self.function_depth += 1;
                let saved_loop_depth = self.loop_depth;
                self.loop_depth = 0;
                // Track generic type parameters
                let saved_generic_params = std::mem::take(&mut self.generic_params);
                let saved_generic_bounds = std::mem::take(&mut self.generic_bounds);
                if !def.type_params.is_empty() {
                    self.generic_fn_params.insert(def.name.clone(), def.type_params.clone());
                    for tp in &def.type_params {
                        self.generic_params.insert(tp.clone());
                        self.define_var(tp, ChannelType::Null, false, def.span.start_line, def.span.start_col);
                    }
                    // Extract bounds from where clauses
                    for wc in &def.where_clauses {
                        self.generic_bounds
                            .entry(wc.type_param.clone())
                            .or_default()
                            .extend(wc.bounds.iter().cloned());
                    }
                    // Validate: warn if generic param has bounds on unknown trait
                    let bound_warnings: Vec<_> = self.generic_bounds.iter()
                        .flat_map(|(p, bs)| bs.iter().map(move |b| (p.clone(), b.clone())))
                        .filter(|(_, b)| !self.trait_defs.contains_key(b) && !["Display","Debug","Clone","Copy","Send","Sync","Sized","Hash","Eq","Ord","Default","Iterator"].contains(&b.as_str()))
                        .collect();
                    for (param, bound) in bound_warnings {
                        self.emit_coded(def.span.start_line, def.span.start_col,
                            format!("generic bound '{}' on '{}' references unknown trait", bound, param),
                            DiagnosticSeverity::Warning, super::errors::ErrorCode::W114, None);
                    }
                }
                let saved_return_types = std::mem::take(&mut self.collected_return_types);
                let resolved_return = def.return_type.as_ref()
                    .and_then(|ta| self.resolve_type(&ta.to_string()))
                    .unwrap_or(ChannelType::Null);
                let prev_return_type = std::mem::replace(
                    &mut self.current_return_type,
                    resolved_return,
                );
                // Check for duplicate parameter names
                {
                    let mut seen_params = HashSet::new();
                    for param in &def.params {
                        if !seen_params.insert(param.name.clone()) {
                            self.emit_coded(
                                param.span.start_line,
                                param.span.start_col,
                                format!(
                                    "duplicate parameter name '{}' in function '{}'",
                                    param.name, def.name
                                ),
                                DiagnosticSeverity::Error,
                                super::errors::ErrorCode::E100,
                                None,
                            );
                        }
                    }
                }
                // Define params as immutable variables
                for param in &def.params {
                    let ct = param
                        .type_annotation
                        .as_ref()
                    .and_then(|ta| self.resolve_type(&ta.to_string()))
                        .unwrap_or(ChannelType::Null);
                    // Type-check default param expression if present
                    if let Some(default_expr) = &param.default {
                        self.check_default_param_type(default_expr, &ct);
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
                    .as_ref()
                    .and_then(|ta| self.resolve_type(&ta.to_string()))
                    .unwrap_or(ChannelType::Null);
                if declared_return != ChannelType::Null
                    && body_type != ChannelType::Null
                    && !body_type.is_compatible_with(&declared_return)
                {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!(
                            "function '{}' declares return type '{}' but body evaluates to '{}'",
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
                // Infer return type (#66): if no explicit return type, unify body type
                // with collected return statement types and store in function_sigs
                if declared_return == ChannelType::Null {
                    let mut all_types = std::mem::take(&mut self.collected_return_types);
                    if body_type != ChannelType::Null {
                        all_types.push(body_type);
                    }
                    let inferred_return = if all_types.is_empty() {
                        ChannelType::Null
                    } else {
                        unify_types(&all_types)
                    };
                    if inferred_return != ChannelType::Null {
                        if let Some(sig) = self.function_sigs.get_mut(&def.name) {
                            sig.return_type = inferred_return;
                        }
                    }
                }
                self.collected_return_types = saved_return_types;
                self.function_depth -= 1;
                self.loop_depth = saved_loop_depth;
                self.current_return_type = prev_return_type;
                self.generic_params = saved_generic_params;
                self.generic_bounds = saved_generic_bounds;
                self.pop_scope();
            }

            StatementKind::Break { value: ref val_expr, .. } => {
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

            StatementKind::Continue { .. } => {
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
                    // Collect return type for inference (#66)
                    self.collected_return_types.push(ret_type);
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
                } else {
                    // `return;` with no value returns Null
                    self.collected_return_types.push(ChannelType::Null);
                }
            }

            // let [a, b] = expr; / let {x, y} = expr;
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
                                    "array destructuring requires array, got {}",
                                    val_type.as_str()
                                ),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::E100,
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
                                    "map destructuring requires map, got {}",
                                    val_type.as_str()
                                ),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::E100,
                                None,
                            );
                        }
                        for (key, alias) in entries {
                            let var_name = alias.as_ref().unwrap_or(key);
                            self.define_var(
                                var_name,
                                ChannelType::Null, // value type unknown
                                *mutable,
                                stmt.span.start_line,
                                stmt.span.start_col,
                            );
                        }
                    }
                    DestructurePattern::Tuple(elements) => {
                        // Tuple destructuring desugars same as array destructuring
                        if val_type != ChannelType::Array && val_type != ChannelType::Null {
                            self.emit_coded(
                                stmt.span.start_line,
                                stmt.span.start_col,
                                format!(
                                    "tuple destructuring requires array/tuple, got {}",
                                    val_type.as_str()
                                ),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::E100,
                                None,
                            );
                        }
                        for elem in elements {
                            match elem {
                                DestructureElement::Name(name) => {
                                    self.define_var(
                                        name,
                                        ChannelType::Null,
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
                }
            }

            // name += expr; / name -= expr; etc.
            StatementKind::CompoundAssign { name, op, value } => {
                let val_type = self.infer_expr(value);

                let (exists, is_mutable, var_type, declared_type) = match self.lookup(name) {
                    Some(info) => (true, info.mutable, info.channel_type, info.declared_type),
                    None => (false, false, ChannelType::Null, None),
                };

                if !exists {
                    let suggestion = self.suggest_variable(name);
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("undefined variable '{}'", name),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E200,
                        suggestion,
                    );
                } else if !is_mutable {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("cannot assign to immutable variable '{}'", name),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E404,
                        None,
                    );
                }

                // Check that operation makes sense for the types
                let result_type = self.infer_binop(*op, var_type, val_type, stmt.span);

                // Check for suspicious literal patterns (E104, W107)
                // E104: Division/modulo by zero literal
                if (*op == BinOp::Div || *op == BinOp::Mod)
                    && (literal_int(value) == Some(0) || literal_float(value) == Some(0.0))
                {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        "division by zero".to_string(),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E104,
                        None,
                    );
                }
                // W107: Modulo by 1
                if *op == BinOp::Mod && literal_int(value) == Some(1) {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        "modulo by 1 always returns 0".to_string(),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::W107,
                        None,
                    );
                }
                // W107: Multiply by 0
                if *op == BinOp::Mul
                    && (literal_int(value) == Some(0) || literal_float(value) == Some(0.0))
                {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        "multiplication by 0 always returns 0".to_string(),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::W107,
                        None,
                    );
                }

                // Warn if compound assignment produces a type incompatible with the declared annotation.
                if let Some(decl) = declared_type {
                    if result_type != ChannelType::Null
                        && !result_type.is_compatible_with(&decl)
                    {
                        self.emit_coded(
                            stmt.span.start_line,
                            stmt.span.start_col,
                            format!(
                                "compound assignment produces '{}' but variable '{}' is declared as '{}'",
                                result_type.as_str(),
                                name,
                                decl.as_str(),
                            ),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::E100,
                            None,
                        );
                    }
                }

                if let Some(info) = self.lookup_mut(name) {
                    info.channel_type = result_type;
                    info.used = true;
                    info.mutated = true;
                }
            }

            // name++ / name-- (increment/decrement)
            StatementKind::Increment { name } | StatementKind::Decrement { name } => {
                let (exists, is_mutable) = match self.lookup(name) {
                    Some(info) => (true, info.mutable),
                    None => (false, false),
                };

                if !exists {
                    let suggestion = self.suggest_variable(name);
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("undefined variable '{}'", name),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E200,
                        suggestion,
                    );
                } else if !is_mutable {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("cannot assign to immutable variable '{}'", name),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E404,
                        None,
                    );
                }

                if let Some(info) = self.lookup_mut(name) {
                    info.used = true;
                    info.mutated = true;
                }
            }

            // Field assignment: obj.field = value (#7)
            StatementKind::FieldAssignment { object, field: _, value } => {
                self.infer_expr(object);
                self.infer_expr(value);
            }

            // Index assignment: obj[index] = value (#7)
            StatementKind::IndexAssignment { object, index, value } => {
                self.infer_expr(object);
                self.infer_expr(index);
                self.infer_expr(value);
            }

            // try { ... } catch err { ... } finally { ... }
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

            // throw expr;
            StatementKind::Throw(expr) => {
                let _ = self.infer_expr(expr);
            }

            // const NAME = expr;
            StatementKind::ConstDef {
                name,
                type_annotation,
                value,
            } => {
                let inferred = self.infer_expr(value);
                // Try constant folding to compute and record the actual value.
                let folded = self.try_const_fold(value);
                // For simple literal values, track the precise type for const propagation.
                let precise_type = match &folded {
                    Some(ConstLiteral::Int64(_)) => ChannelType::Int64,
                    Some(ConstLiteral::Float64(_)) => ChannelType::Float64,
                    Some(ConstLiteral::String(_)) => ChannelType::String,
                    Some(ConstLiteral::Bool(_)) => ChannelType::Bool,
                    None => match &value.kind {
                        ExpressionKind::Literal(Literal::Int64(_)) => ChannelType::Int64,
                        ExpressionKind::Literal(Literal::Float64(_)) => ChannelType::Float64,
                        ExpressionKind::Literal(Literal::String(_)) => ChannelType::String,
                        ExpressionKind::Literal(Literal::Bool(_)) => ChannelType::Bool,
                        _ => inferred,
                    },
                };
                let ct = self.reconcile_annotation(
                    type_annotation.as_ref(),
                    precise_type,
                    stmt.span.start_line,
                    stmt.span.start_col,
                    name,
                );
                // Store known const value type for propagation.
                self.const_values.insert(name.clone(), ct);
                // Store computed literal for use in match patterns and array sizes.
                if let Some(lit) = folded {
                    self.const_literals.insert(name.clone(), lit);
                }
                self.define_var(name, ct, false, stmt.span.start_line, stmt.span.start_col);
                // Mark as const for unused-constant diagnostics.
                if let Some(info) = self.lookup_mut(name) {
                    info.is_const = true;
                }
            }

            // static name = expr; / static mut name = expr;
            StatementKind::StaticDef { name, type_annotation, value, mutable } => {
                let inferred = self.infer_expr(value);
                let ct = self.reconcile_annotation(
                    type_annotation.as_ref(),
                    inferred,
                    stmt.span.start_line,
                    stmt.span.start_col,
                    name,
                );
                self.define_var(name, ct, *mutable, stmt.span.start_line, stmt.span.start_col);
            }

            // type Name = target;
            StatementKind::TypeAlias { name, target } => {
                // Validate the target type resolves (could be a built-in or another alias)
                if self.resolve_type(&target.to_string()).is_none() {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("unknown type '{}' in type alias '{}'", target, name),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E100,
                        None,
                    );
                }
            }

            // mod name { body }
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

            // use path::to::item;
            StatementKind::Use { path, alias, glob, .. } => {
                // W208: Duplicate import detection
                let import_key = path.join("::");
                if !self.seen_imports.insert(import_key.clone()) {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("duplicate import '{}'", import_key),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::W208,
                        None,
                    );
                }
                // Validate that std module paths are known
                if path.first().map(|s| s.as_str()) == Some("std")
                    && path.len() >= 2
                    && !super::interpreter::STD_MODULE_NAMES.contains(&path[1].as_str())
                {
                    self.emit_coded(
                        stmt.span.start_line,
                        stmt.span.start_col,
                        format!("unknown standard library module 'std::{}'", path[1]),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E203,
                        None,
                    );
                }
                // Register the imported name so it's recognized in call resolution
                if path.first().map(|s| s.as_str()) == Some("std") && path.len() >= 2
                    && (*glob || path.len() == 2)
                {
                    // Glob import or module-level import (use std::math):
                    // register all operations from the module
                    for op_name in super::interpreter::std_module_ops(&path[1]) {
                        self.use_aliases.insert(op_name.to_string());
                    }
                } else if !*glob {
                    if let Some(local_name) = alias.as_ref().or_else(|| path.last()) {
                        self.use_aliases.insert(local_name.clone());
                    }
                }
            }

            StatementKind::TestDef { body, .. } => {
                self.push_scope();
                self.check_block(body);
                self.pop_scope();
            }

            StatementKind::EnumDef { name, variants, .. } => {
                let variant_info: Vec<(String, usize)> = variants.iter().map(|v| (v.name.clone(), v.fields.len())).collect();
                self.enum_variants.insert(name.clone(), variant_info);
                // W235: Duplicate enum variant names
                {
                    let mut seen = std::collections::HashSet::new();
                    for v in variants {
                        if !seen.insert(&v.name) {
                            self.emit_coded(
                                v.span.start_line,
                                v.span.start_col,
                                format!("duplicate variant '{}' in enum '{}'", v.name, name),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::W235,
                                None,
                            );
                        }
                    }
                }
            }
            StatementKind::StructDef { name, fields, .. } => {
                let field_info: Vec<(String, Option<String>)> = fields.iter().map(|f| (f.name.clone(), f.type_annotation.as_ref().map(|t| t.to_string()))).collect();
                self.struct_defs.insert(name.clone(), field_info);
                // W234: Duplicate struct field names
                {
                    let mut seen = std::collections::HashSet::new();
                    for field in fields {
                        if !seen.insert(&field.name) {
                            self.emit_coded(
                                field.span.start_line,
                                field.span.start_col,
                                format!("duplicate field '{}' in struct '{}'", field.name, name),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::W234,
                                None,
                            );
                        }
                    }
                }
                // Validate field type annotations against known types
                for field in fields {
                    if let Some(ref ann) = field.type_annotation {
                        let ann_str = ann.to_string();
                        if self.resolve_type(&ann_str).is_none()
                            && !self.struct_defs.contains_key(ann_str.as_str())
                            && !self.enum_variants.contains_key(ann_str.as_str())
                        {
                            self.emit_coded(
                                field.span.start_line,
                                field.span.start_col,
                                format!(
                                    "unknown type '{}' in struct field '{}.{}'",
                                    ann, name, field.name
                                ),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::E100,
                                None,
                            );
                        }
                    }
                }
            }

            StatementKind::ImplBlock { methods, .. } => {
                for method in methods {
                    self.push_scope();
                    for param in &method.params {
                        let ct = param.type_annotation.as_ref()
                            .and_then(|ta| self.resolve_type(&ta.to_string()))
                            .unwrap_or(ChannelType::Null);
                        self.define_var(&param.name, ct, false, param.span.start_line, param.span.start_col);
                    }
                    self.check_block(&method.body);
                    self.pop_scope();
                }
            }

            StatementKind::TraitDef { .. } => {
                // Trait definitions are collected in pass 1b (check_program).
            }

            StatementKind::ImplTrait { trait_name, type_name, methods } => {
                // Validate trait conformance: check that all required methods are implemented
                // with correct arity and matching return types.
                if let Some(trait_methods) = self.trait_defs.get(trait_name).cloned() {
                    let impl_method_map: HashMap<&str, &FunctionDef> = methods.iter()
                        .map(|m| (m.name.as_str(), m))
                        .collect();

                    for (method_name, expected_params, _line, _col, expected_return) in &trait_methods {
                        match impl_method_map.get(method_name.as_str()) {
                            None => {
                                self.emit_coded(
                                    stmt.span.start_line,
                                    stmt.span.start_col,
                                    format!(
                                        "trait '{}' requires method '{}' but it is not implemented for '{}'",
                                        trait_name, method_name, type_name
                                    ),
                                    DiagnosticSeverity::Error,
                                    super::errors::ErrorCode::E100,
                                    None,
                                );
                            }
                            Some(impl_method) => {
                                let actual_params = impl_method.params.len();
                                if actual_params != *expected_params {
                                    self.emit_coded(
                                        stmt.span.start_line,
                                        stmt.span.start_col,
                                        format!(
                                            "method '{}' for trait '{}' on '{}' has {} parameter(s), but the trait requires {}",
                                            method_name, trait_name, type_name, actual_params, expected_params
                                        ),
                                        DiagnosticSeverity::Error,
                                        super::errors::ErrorCode::E100,
                                        None,
                                    );
                                }
                                // Check return type matches if trait specifies one.
                                if let Some(expected_ret) = expected_return {
                                    let actual_ret = impl_method.return_type.as_ref().map(|t| t.to_string());
                                    match &actual_ret {
                                        None => {
                                            self.emit_coded(
                                                stmt.span.start_line,
                                                stmt.span.start_col,
                                                format!(
                                                    "method '{}' for trait '{}' on '{}' is missing return type annotation, trait requires -> {}",
                                                    method_name, trait_name, type_name, expected_ret
                                                ),
                                                DiagnosticSeverity::Warning,
                                                super::errors::ErrorCode::E100,
                                                None,
                                            );
                                        }
                                        Some(actual) if actual != expected_ret => {
                                            self.emit_coded(
                                                stmt.span.start_line,
                                                stmt.span.start_col,
                                                format!(
                                                    "method '{}' for trait '{}' on '{}' returns '{}', but the trait requires '{}'",
                                                    method_name, trait_name, type_name, actual, expected_ret
                                                ),
                                                DiagnosticSeverity::Error,
                                                super::errors::ErrorCode::E100,
                                                None,
                                            );
                                        }
                                        _ => {} // types match
                                    }
                                }
                            }
                        }
                    }
                }

                // Type-check the method bodies.
                for method in methods {
                    self.push_scope();
                    for param in &method.params {
                        let ct = param.type_annotation.as_ref()
                            .and_then(|ta| self.resolve_type(&ta.to_string()))
                            .unwrap_or(ChannelType::Null);
                        self.define_var(&param.name, ct, false, param.span.start_line, param.span.start_col);
                    }
                    self.check_block(&method.body);
                    self.pop_scope();
                }
            }

            StatementKind::DoWhileLoop { condition, body, .. } => {
                let cond_type = self.infer_expr(condition);
                if cond_type != ChannelType::Bool && cond_type != ChannelType::Null {
                    self.emit_coded(
                        condition.span.start_line,
                        condition.span.start_col,
                        format!("do-while condition should be bool, got {}", cond_type.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E101,
                        None,
                    );
                }
                self.push_scope();
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
            }

            StatementKind::Defer(expr) => {
                let _ = self.infer_expr(expr);
            }

            StatementKind::CStyleFor { init, condition, update, body } => {
                self.push_scope();
                self.check_statement(init);
                let cond_type = self.infer_expr(condition);
                if cond_type != ChannelType::Bool && cond_type != ChannelType::Null {
                    self.emit_coded(
                        condition.span.start_line,
                        condition.span.start_col,
                        format!("for condition should be bool, got {}", cond_type.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E101,
                        None,
                    );
                }
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                self.check_statement(update);
                self.pop_scope();
            }

            // (a, b) = (expr1, expr2); — tuple/swap assignment
            StatementKind::TupleAssignment { names, value } => {
                let _ = self.infer_expr(value);
                for name in names {
                    let (exists, is_mutable) = match self.lookup(name) {
                        Some(info) => (true, info.mutable),
                        None => (false, false),
                    };
                    if !exists {
                        let suggestion = self.suggest_variable(name);
                        self.emit_coded(
                            stmt.span.start_line,
                            stmt.span.start_col,
                            format!("undefined variable '{}'", name),
                            DiagnosticSeverity::Error,
                            super::errors::ErrorCode::E200,
                            suggestion,
                        );
                    } else if !is_mutable {
                        self.emit_coded(
                            stmt.span.start_line,
                            stmt.span.start_col,
                            format!("cannot assign to immutable variable '{}'", name),
                            DiagnosticSeverity::Error,
                            super::errors::ErrorCode::E404,
                            None,
                        );
                    }
                    if let Some(info) = self.lookup_mut(name) {
                        info.used = true;
                        info.mutated = true;
                    }
                }
            }
        }
    }

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
            if !Self::collect_enum_variants(&arm.pattern, &mut enum_name, &mut covered) {
                return false; // mixed enums
            }
        }
        // Look up the enum definition
        if let Some(name) = enum_name {
            if let Some(variants) = self.enum_variants.get(name) {
                return variants.iter().all(|(v, _)| covered.contains(v.as_str()));
            }
        }
        false
    }

    /// Collect variable names bound by a pattern (for or-pattern validation).
    fn collect_pattern_var_names(pattern: &Pattern) -> std::collections::BTreeSet<String> {
        let mut names = std::collections::BTreeSet::new();
        Self::collect_pattern_var_names_inner(pattern, &mut names);
        names
    }

    fn collect_pattern_var_names_inner(pattern: &Pattern, names: &mut std::collections::BTreeSet<String>) {
        match pattern {
            Pattern::Variable(name) => { names.insert(name.clone()); }
            Pattern::TypePattern { name, .. } => { names.insert(name.clone()); }
            Pattern::Rest(Some(name)) => { names.insert(name.clone()); }
            Pattern::Array(subs) => {
                for sub in subs { Self::collect_pattern_var_names_inner(sub, names); }
            }
            Pattern::Map(entries) => {
                for (_, sub) in entries { Self::collect_pattern_var_names_inner(sub, names); }
            }
            Pattern::Or(alts) => {
                for alt in alts { Self::collect_pattern_var_names_inner(alt, names); }
            }
            Pattern::EnumPattern { bindings, .. } => {
                for sub in bindings { Self::collect_pattern_var_names_inner(sub, names); }
            }
            Pattern::Binding { name, pattern } => {
                names.insert(name.clone());
                Self::collect_pattern_var_names_inner(pattern, names);
            }
            Pattern::Literal(_) | Pattern::Wildcard | Pattern::Rest(None) | Pattern::RangePattern { .. } => {}
        }
    }

    /// Recursively collect enum variants from a pattern (handles nested Or patterns).
    fn collect_enum_variants<'a>(
        pattern: &'a Pattern,
        enum_name: &mut Option<&'a str>,
        covered: &mut std::collections::HashSet<&'a str>,
    ) -> bool {
        match pattern {
            Pattern::EnumPattern { enum_name: en, variant, .. } => {
                match *enum_name {
                    None => { *enum_name = Some(en); }
                    Some(existing) if existing != en.as_str() => return false,
                    _ => {}
                }
                covered.insert(variant.as_str());
                true
            }
            Pattern::Or(alternatives) => {
                for alt in alternatives {
                    if !Self::collect_enum_variants(alt, enum_name, covered) {
                        return false;
                    }
                }
                true
            }
            _ => true,
        }
    }

    /// Check if match arms exhaustively cover boolean values (true and false).
    fn check_bool_exhaustive(&self, arms: &[crate::syntax::ast::MatchArm]) -> bool {
        let mut has_true = false;
        let mut has_false = false;
        for arm in arms {
            if arm.guard.is_some() {
                continue;
            }
            match &arm.pattern {
                Pattern::Literal(Literal::Bool(true)) => has_true = true,
                Pattern::Literal(Literal::Bool(false)) => has_false = true,
                Pattern::Or(alternatives) => {
                    for alt in alternatives {
                        match alt {
                            Pattern::Literal(Literal::Bool(true)) => has_true = true,
                            Pattern::Literal(Literal::Bool(false)) => has_false = true,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        has_true && has_false
    }


    fn check_block(&mut self, block: &Block) {
        let mut found_terminal = false;
        for stmt in &block.statements {
            if found_terminal {
                // Unreachable code after return/break/continue/throw
                self.emit_coded(
                    stmt.span.start_line,
                    stmt.span.start_col,
                    "unreachable code after return/break/continue/throw".to_string(),
                    DiagnosticSeverity::Warning,
                    super::errors::ErrorCode::W106,
                    None,
                );
                break;
            }
            self.check_statement(stmt);
            // Check if this statement terminates control flow
            match &stmt.kind {
                StatementKind::Return(_) | StatementKind::Throw(_) => { found_terminal = true; }
                StatementKind::Break { .. } | StatementKind::Continue { .. } => {
                    if self.loop_depth > 0 { found_terminal = true; }
                }
                _ => {}
            }
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


    fn infer_expr(&mut self, expr: &Expression) -> ChannelType {
        match &expr.kind {
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
                                format!("duplicate key '{}' in map literal", key),
                                DiagnosticSeverity::Error,
                                super::errors::ErrorCode::E107,
                                None,
                            );
                        }
                        let _ = self.infer_expr(val);
                    }
                    ChannelType::Map
                }
                Literal::Set(elements) => {
                    for el in elements {
                        let _ = self.infer_expr(el);
                    }
                    ChannelType::Array
                }
            },

            ExpressionKind::Variable(name) => {
                // Built-in constant: None (#78)
                if name == "None" {
                    return ChannelType::Null;
                }
                // Use const-propagated type if available (more precise than scope lookup).
                let const_type = self.const_values.get(name.as_str()).copied();
                // Copy out what we need before the mutable borrow.
                let ct = match self.lookup(name) {
                    Some(info) => const_type.unwrap_or(info.channel_type),
                    None => {
                        if self.function_sigs.contains_key(name.as_str()) {
                            if let Some(sig) = self.function_sigs.get_mut(name.as_str()) {
                                sig.used = true;
                            }
                            return ChannelType::Null; // function type is opaque
                        }
                        if self.use_aliases.contains(name.as_str()) {
                            return ChannelType::Null;
                        }
                        let suggestion = self.suggest_variable(name);
                        self.emit_coded(
                            expr.span.start_line,
                            expr.span.start_col,
                            format!("undefined variable '{}'", name),
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

            ExpressionKind::BinaryOp { op, left, right } => {
                let left_ty = self.infer_expr(left);
                let right_ty = self.infer_expr(right);
                self.check_binop_literals(*op, left, right, left_ty, right_ty, expr.span);
                self.infer_binop(*op, left_ty, right_ty, expr.span)
            }

            ExpressionKind::UnaryOp { op, operand } => {
                // W106: Double negation or double NOT.
                if let ExpressionKind::UnaryOp { op: inner_op, .. } = &operand.kind {
                    if op == inner_op {
                        let msg = match op {
                            UnOp::Neg => "double negation is redundant",
                            UnOp::Not => "double logical NOT is redundant",
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
                                format!("logical NOT expects bool, got {}", operand_ty.as_str()),
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
                                    "negation expects numeric type, got {}",
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

            // Function / operation call
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
                                "range will produce empty array (start >= end)".to_string(),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::W107,
                                None,
                            );
                        }
                    }
                }

                // Is it a user-defined function?
                if let Some(sig) = self.function_sigs.get(name).cloned() {
                    if let Some(sig_mut) = self.function_sigs.get_mut(name) {
                        sig_mut.used = true;
                    }
                    // W114: warn on calling a deprecated function
                    if sig.deprecated {
                        self.emit_coded(
                            expr.span.start_line,
                            expr.span.start_col,
                            format!("use of deprecated function '{}'", name),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::W114,
                            None,
                        );
                    }
                    // Check arity (accounting for default parameters)
                    let max_args = if sig.has_rest { usize::MAX } else { sig.params.len() };
                    if arg_types.len() < sig.required_params || arg_types.len() > max_args {
                        let arity_msg = if sig.has_rest {
                            format!("at least {}", sig.required_params)
                        } else if sig.required_params == sig.params.len() {
                            format!("{}", sig.params.len())
                        } else {
                            format!("{}-{}", sig.required_params, sig.params.len())
                        };
                        self.emit_coded(
                            expr.span.start_line,
                            expr.span.start_col,
                            format!(
                                "function '{}' expects {} arguments, got {}",
                                name,
                                arity_msg,
                                arg_types.len()
                            ),
                            DiagnosticSeverity::Error,
                            super::errors::ErrorCode::E405,
                            None,
                        );
                    }
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
                                            "type mismatch on '{}': got {} but expected {}",
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

                    if arg_types.len() != expected_inputs.len() {
                        self.emit_coded(
                            expr.span.start_line, expr.span.start_col,
                            format!("operation '{}' expects {} arguments, got {}", name, expected_inputs.len(), arg_types.len()),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::E405, None,
                        );
                    }

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
                                        "type mismatch on '{}': got {} but expected {}",
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

                // Is it a module-qualified operation (e.g., Math::sqrt)?
                // Strip the module prefix and try to resolve via OperationType.
                if name.contains("::") {
                    if let Some((_module, func)) = name.rsplit_once("::") {
                        if let Some(op) = OperationType::parse(func) {
                            let expected_inputs = op_input_types(op);
                            if arg_types.len() != expected_inputs.len() {
                                self.emit_coded(
                                    expr.span.start_line, expr.span.start_col,
                                    format!(
                                        "operation '{}' expects {} arguments, got {}",
                                        name, expected_inputs.len(), arg_types.len()
                                    ),
                                    DiagnosticSeverity::Warning,
                                    super::errors::ErrorCode::E405, None,
                                );
                            }
                            // Check positional arg types against expected input ports.
                            for (i, (port_name, expected_type)) in expected_inputs.iter().enumerate() {
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
                                                    "type mismatch on '{}': got {} but expected {}",
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
                            }
                            return refine_call_output(op, &arg_types);
                        }
                    }
                }

                // Is it a use-imported name?
                if self.use_aliases.contains(name.as_str()) {
                    return ChannelType::Null;
                }

                // Is it an interpreter built-in function?
                match name.as_str() {
                    "len" => return ChannelType::Int64,
                    "typeof" => return ChannelType::String,
                    "println" | "print" | "debug_log" => {
                        // Args already inferred at arg_types collection above
                        return arg_types.first().copied().unwrap_or(ChannelType::Null);
                    }
                    "assert" | "assert_eq" | "assert_ne" | "assert_throws" => {
                        // Args already inferred at arg_types collection above
                        return ChannelType::Null;
                    }
                    // Option/Result constructors and helpers (#78)
                    "Some" => return arg_types.first().copied().unwrap_or(ChannelType::Null),
                    "None" => return ChannelType::Null,
                    "Ok" | "Err" => return ChannelType::Map,
                    "is_some" | "is_none" | "is_ok" | "is_err" => return ChannelType::Bool,
                    "unwrap" => return arg_types.first().copied().unwrap_or(ChannelType::Null),
                    "unwrap_or" => return arg_types.first().copied().unwrap_or(ChannelType::Null),
                    // Concurrency: channel operations
                    "channel" => return ChannelType::Array,
                    "chan_send" => return ChannelType::Null,
                    "chan_recv" => return ChannelType::Null,
                    "chan_try_recv" => return ChannelType::Null,
                    "chan_close" => return ChannelType::Null,
                    // Concurrency: select returns [index, value]
                    "select" => return ChannelType::Array,
                    // Sync module: mutex, rwlock, barrier, etc.
                    "mutex_new" | "mutex_lock" | "mutex_unlock" | "mutex_try_lock"
                    | "rwlock_new" | "rwlock_read" | "rwlock_write" | "rwlock_unlock"
                    | "once_new" | "once_call"
                    | "barrier_new" | "barrier_wait"
                    | "semaphore_new" | "semaphore_acquire" | "semaphore_release"
                    | "condvar_new" | "condvar_wait" | "condvar_notify_one" | "condvar_notify_all"
                    | "atomic_new" | "atomic_load" | "atomic_store"
                    | "atomic_add" | "atomic_sub" | "atomic_compare_exchange" => return ChannelType::Null,
                    _ => {}
                }

                // Unknown function.
                self.emit_coded(
                    expr.span.start_line,
                    expr.span.start_col,
                    format!("undefined function or operation '{}'", name),
                    DiagnosticSeverity::Error,
                    super::errors::ErrorCode::E201,
                    self.suggest_function(name),
                );
                ChannelType::Null
            }

            // Pipe: left |> right
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

            // If/else expression
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
                        format!("if condition should be bool, got {}", cond_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E101,
                        None,
                    );
                }

                // Type narrowing (#67): if condition is `typeof(x) == "type_name"`,
                // narrow x's type in the then-block
                let narrowing = extract_typeof_narrowing(condition);
                let then_ty = if let Some((ref var_name, narrowed_type)) = narrowing {
                    self.push_scope();
                    // Narrow the variable's type in the then-block scope
                    self.define_var(var_name, narrowed_type, false, condition.span.start_line, condition.span.start_col);
                    if let Some(info) = self.lookup_mut(var_name) {
                        info.used = true; // Don't warn about this shadow
                    }
                    let ty = self.infer_block_no_scope(then_block);
                    self.pop_scope();
                    ty
                } else {
                    self.infer_block(then_block)
                };
                let else_ty = if let Some(else_blk) = else_block {
                    self.infer_block(else_blk)
                } else {
                    ChannelType::Null
                };

                // Unify branch types (supports numeric promotion).
                let unified = unify_types(&[then_ty, else_ty]);
                if unified == ChannelType::Null
                    && then_ty != ChannelType::Null
                    && else_ty != ChannelType::Null
                    && then_ty != else_ty
                {
                    self.emit_coded(
                        expr.span.start_line,
                        expr.span.start_col,
                        format!(
                            "if/else branches have mismatched types: '{}' vs '{}'",
                            then_ty.as_str(),
                            else_ty.as_str(),
                        ),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E100,
                        None,
                    );
                }
                unified
            }

            ExpressionKind::Block(block) => self.infer_block(block),

            // Index: arr[i]
            ExpressionKind::Index { object, index } => {
                let obj_ty = self.infer_expr(object);
                let idx_ty = self.infer_expr(index);

                if obj_ty != ChannelType::Array
                    && obj_ty != ChannelType::Null
                    && obj_ty != ChannelType::Map
                    && obj_ty != ChannelType::String
                {
                    self.emit_coded(
                        object.span.start_line,
                        object.span.start_col,
                        format!("indexing requires array, map, or string, got {}", obj_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E100,
                        None,
                    );
                }

                // Range expressions in index position are valid (array slicing)
                let is_range_slice = matches!(&index.kind, ExpressionKind::Range { .. });
                if obj_ty == ChannelType::Array
                    && !is_integer(idx_ty)
                    && idx_ty != ChannelType::Null
                    && !is_range_slice
                {
                    self.emit_coded(
                        index.span.start_line,
                        index.span.start_col,
                        format!("array index should be integer, got {}", idx_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E103,
                        None,
                    );
                }

                // E105: Negative array index literal.
                if obj_ty == ChannelType::Array {
                    if let Some(idx_val) = literal_int(index) {
                        if idx_val < 0 {
                            self.emit_coded(
                                index.span.start_line,
                                index.span.start_col,
                                format!("negative array index ({})", idx_val),
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
                            "index into empty array literal".to_string(),
                            DiagnosticSeverity::Error,
                            super::errors::ErrorCode::E106,
                            None,
                        );
                    }
                }

                // Range slice returns same collection type; element access is unknown.
                if is_range_slice {
                    match obj_ty {
                        ChannelType::Array => return ChannelType::Array,
                        ChannelType::String => return ChannelType::String,
                        _ => {}
                    }
                }
                ChannelType::Null
            }

            // Field access: obj.field
            ExpressionKind::FieldAccess { object, field } => {
                let obj_ty = self.infer_expr(object);
                // Null safety: warn on field access on potentially null value
                if obj_ty == ChannelType::Null {
                    // Check if it's using optional chaining (?.)
                    let has_optional = matches!(&object.kind, ExpressionKind::OptionalChain { .. });
                    if !has_optional {
                        self.emit_coded(
                            object.span.start_line,
                            object.span.start_col,
                            format!("field access '.{}' on potentially null value — use '?.' for safe access", field),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::W114,
                            None,
                        );
                    }
                } else if obj_ty != ChannelType::Map
                    && obj_ty != ChannelType::String
                    && obj_ty != ChannelType::Null
                {
                    self.emit_coded(
                        object.span.start_line,
                        object.span.start_col,
                        format!("field access requires map or string, got {}", obj_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E100,
                        None,
                    );
                }
                ChannelType::Null
            }

            // Placeholder (_)
            ExpressionKind::Placeholder => {
                if self.pipe_depth == 0 {
                    self.emit_coded(
                        expr.span.start_line,
                        expr.span.start_col,
                        "placeholder '_' can only be used inside pipe expressions".to_string(),
                        DiagnosticSeverity::Error,
                        super::errors::ErrorCode::E303,
                        None,
                    );
                }
                ChannelType::Null
            }

            // Range expression: range(start, end)
            ExpressionKind::Range { start, end, inclusive } => {
                let start_ty = self.infer_expr(start);
                let end_ty = self.infer_expr(end);

                if !is_numeric(start_ty) && start_ty != ChannelType::Null {
                    self.emit_coded(
                        start.span.start_line,
                        start.span.start_col,
                        format!("range start should be numeric, got {}", start_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E103,
                        None,
                    );
                }
                if !is_numeric(end_ty) && end_ty != ChannelType::Null {
                    self.emit_coded(
                        end.span.start_line,
                        end.span.start_col,
                        format!("range end should be numeric, got {}", end_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E103,
                        None,
                    );
                }

                // W7: Empty range (start >= end with literals).
                if let (Some(s), Some(e)) = (literal_int(start), literal_int(end)) {
                    let is_empty = if *inclusive { s > e } else { s >= e };
                    if is_empty {
                        self.emit_coded(
                            expr.span.start_line,
                            expr.span.start_col,
                            "range will produce empty array (start >= end)".to_string(),
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

            // Method call: obj.method(args)
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
                    "push", "pop", "set", "remove", "insert",
                    "delete", "merge", "sort", "reverse",
                    "shift", "filter_nulls",
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
                    // Known built-in method handled by the interpreter directly.
                    // Refine return type for common patterns.
                    return match name.as_str() {
                        "string_predicate" => ChannelType::Bool, // is_empty, is_numeric, is_alphabetic
                        "array_length" | "string_length" | "map_size" | "bytes_length" => ChannelType::Int64,
                        "array_contains" | "string_contains" | "bytes_contains" | "map_has" => ChannelType::Bool,
                        "generic_method" => match method.as_str() {
                            "to_string" | "to_json" => ChannelType::String,
                            "to_int64" => ChannelType::Int64,
                            "to_float64" => ChannelType::Float64,
                            "to_bool" => ChannelType::Bool,
                            "typeof" => ChannelType::String,
                            _ => ChannelType::Null,
                        },
                        "numeric_method" => match method.as_str() {
                            "to_string" => ChannelType::String,
                            "to_int64" => ChannelType::Int64,
                            "to_int32" => ChannelType::Int32,
                            "to_uint32" => ChannelType::Uint32,
                            "to_uint64" => ChannelType::Uint64,
                            "to_float64" => ChannelType::Float64,
                            "to_float32" => ChannelType::Float32,
                            "is_nan" | "is_infinite" | "is_finite" => ChannelType::Bool,
                            _ => obj_ty, // abs, sign, pow, min, max, clamp preserve receiver type
                        },
                        "array_hof" => match method.as_str() {
                            "any" | "all" => ChannelType::Bool,
                            "group_by" => ChannelType::Map,
                            "each" => ChannelType::Null,
                            "min_by" | "max_by" => ChannelType::Null, // element type unknown
                            _ => ChannelType::Array, // map, filter, flat_map, sort_by, partition, scan, take_while, skip_while, zip, enumerate, chunk
                        },
                        "map_hof" => ChannelType::Map,
                        "base64_encode" => ChannelType::String,
                        "base64_decode" => ChannelType::Bytes,
                        "array_direct" => match method.as_str() {
                            "is_empty" => ChannelType::Bool,
                            "join" => ChannelType::String,
                            _ => ChannelType::Null, // first, last, min, max, sum, product — element type unknown
                        },
                        "string_char_at" => ChannelType::String,
                        "string_convert" => match method.as_str() {
                            "to_int" => ChannelType::Int64,
                            "to_float" => ChannelType::Float64,
                            _ => ChannelType::Null,
                        },
                        _ => ChannelType::Null,
                    };
                }

                // Unknown method — warn (but suppress if receiver type is unknown/Null)
                if obj_ty != ChannelType::Null {
                    let available = available_methods_for_channel_type(obj_ty);
                    let suggestion = super::errors::suggest_name(method, &available);
                    self.emit_coded(
                        expr.span.start_line,
                        expr.span.start_col,
                        format!("unknown method '{}' on type '{}'", method, obj_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E202,
                        suggestion,
                    );
                }
                ChannelType::Null
            }

            // Lambda: |params| expr
            ExpressionKind::Lambda { params, body } => {
                self.push_scope();
                self.function_depth += 1;
                let saved_loop_depth = self.loop_depth;
                self.loop_depth = 0;
                let prev_return_type = std::mem::replace(&mut self.current_return_type, ChannelType::Null);
                for param in params {
                    let ct = param
                        .type_annotation
                        .as_ref()
                    .and_then(|ta| self.resolve_type(&ta.to_string()))
                        .unwrap_or(ChannelType::Null);
                    // Type-check default param expression if present
                    if let Some(default_expr) = &param.default {
                        self.check_default_param_type(default_expr, &ct);
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

            // Match expression: match value { pattern => body, ... }
            ExpressionKind::Match { value, arms } => {
                let val_type = self.infer_expr(value);

                if arms.is_empty() {
                    self.emit_coded(
                        expr.span.start_line,
                        expr.span.start_col,
                        "empty match expression".to_string(),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::W206,
                        None,
                    );
                    return ChannelType::Null;
                }

                let has_catchall = arms.iter().any(|arm| {
                    if arm.guard.is_some() {
                        return false;
                    }
                    match &arm.pattern {
                        Pattern::Wildcard | Pattern::Variable(_) => true,
                        Pattern::Or(alternatives) => alternatives.iter().any(|alt| {
                            matches!(alt, Pattern::Wildcard | Pattern::Variable(_))
                        }),
                        _ => false,
                    }
                });
                if !has_catchall {
                    // Check if all enum variants are covered
                    let enum_exhaustive = self.check_enum_exhaustive(arms);
                    // Check if boolean true/false are both covered
                    let bool_exhaustive = self.check_bool_exhaustive(arms);
                    if !enum_exhaustive && !bool_exhaustive {
                        self.emit_coded(
                            expr.span.start_line,
                            expr.span.start_col,
                            "non-exhaustive match: consider adding a wildcard '_' arm".to_string(),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::W203,
                            Some("add a `_ => ...` arm to handle remaining cases".to_string()),
                        );
                    }
                }

                let mut arm_types = Vec::new();
                for arm in arms {
                    self.push_scope();
                    self.bind_pattern_vars(&arm.pattern, val_type, &arm.span);
                    if let Some(guard) = &arm.guard {
                        let guard_ty = self.infer_expr(guard);
                        if guard_ty != ChannelType::Bool && guard_ty != ChannelType::Null {
                            self.emit_coded(
                                guard.span.start_line,
                                guard.span.start_col,
                                format!("match guard should be bool, got {}", guard_ty.as_str()),
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

                unify_types(&arm_types)
            }

            // String interpolation: f"text {expr} text"
            ExpressionKind::StringInterpolation { parts } => {
                for part in parts {
                    if let StringPart::Expr(e) = part {
                        let _ = self.infer_expr(e);
                    }
                }
                ChannelType::String
            }

            // Null coalescing: x ?? default
            ExpressionKind::NullCoalesce { left, right } => {
                let left_ty = self.infer_expr(left);
                let right_ty = self.infer_expr(right);
                // Unify both branches — left may be null at runtime
                unify_types(&[left_ty, right_ty])
            }

            // Optional chaining: obj?.field
            ExpressionKind::OptionalChain { object, field } => {
                let obj_ty = self.infer_expr(object);
                // Empty field means this is an index-access marker (expr?[index]),
                // which is valid on any nullable type, not just maps.
                if !field.is_empty() && obj_ty != ChannelType::Map && obj_ty != ChannelType::Null {
                    self.emit_coded(
                        object.span.start_line,
                        object.span.start_col,
                        format!(
                            "optional chaining requires map or null, got {}",
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

            // Spread: ...expr
            ExpressionKind::Spread(inner) => {
                let inner_ty = self.infer_expr(inner);
                if inner_ty != ChannelType::Array
                    && inner_ty != ChannelType::Map
                    && inner_ty != ChannelType::Null
                {
                    self.emit_coded(
                        expr.span.start_line,
                        expr.span.start_col,
                        format!("spread requires array or map, got {}", inner_ty.as_str()),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E103,
                        None,
                    );
                }
                inner_ty
            }

            // loop { body } — infinite loop with break value
            ExpressionKind::Loop { body: block, .. } => {
                // W104 (empty block) is handled by the linter as W206.
                self.push_scope();
                self.loop_depth += 1;
                self.check_block(block);
                self.loop_depth -= 1;
                self.pop_scope();
                // Break value type unknown at static analysis time
                ChannelType::Null
            }

            // try { ... } catch err { ... } — expression form
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

                // Unify try/catch types (supports numeric promotion).
                unify_types(&[try_ty, catch_ty])
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
                // Validate enum variant exists and check arity
                if let Some(variants) = self.enum_variants.get(enum_name.as_str()) {
                    if let Some((_, expected_fields)) = variants.iter().find(|(v, _)| v == variant) {
                        // Variant exists — check argument count
                        if args.len() != *expected_fields {
                            self.emit_coded(
                                expr.span.start_line,
                                expr.span.start_col,
                                format!(
                                    "enum variant '{}::{}' expects {} arguments, got {}",
                                    enum_name, variant, expected_fields, args.len()
                                ),
                                DiagnosticSeverity::Error,
                                super::errors::ErrorCode::E405,
                                None,
                            );
                        }
                    } else {
                        let variant_names: Vec<&str> = variants.iter().map(|(v, _)| v.as_str()).collect();
                        let code = super::errors::ErrorCode::E202;
                        self.diagnostics.push(AstDiagnostic {
                            line: expr.span.start_line,
                            column: expr.span.start_col,
                            message: format!(
                                "enum '{}' has no variant '{}' (available: {})",
                                enum_name,
                                variant,
                                variant_names.join(", ")
                            ),
                            severity: DiagnosticSeverity::Error,
                            code: Some(code.to_string()),
                            help: Some(code.help().to_string()),
                            suggestion: super::errors::suggest_name(variant, &variant_names),
                            source_file: None,
                        });
                    }
                } else if !enum_name.contains("::") && !self.use_aliases.contains(enum_name.as_str()) {
                    // Check if this is a module-qualified function call (e.g., Math::sqrt).
                    // The parser treats X::Y(...) as EnumConstruct, but it might be a stdlib call.
                    let arg_types: Vec<ChannelType> = args.iter().map(|a| self.infer_expr(a)).collect();
                    if let Some(op) = OperationType::parse(variant) {
                        let expected_inputs = op_input_types(op);
                        if arg_types.len() != expected_inputs.len() {
                            self.emit_coded(
                                expr.span.start_line, expr.span.start_col,
                                format!(
                                    "operation '{}::{}' expects {} arguments, got {}",
                                    enum_name, variant, expected_inputs.len(), arg_types.len()
                                ),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::E405, None,
                            );
                        }
                        // Check positional arg types against expected input ports.
                        for (i, (port_name, expected_type)) in expected_inputs.iter().enumerate() {
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
                                                "type mismatch on '{}': got {} but expected {}",
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
                        }
                        return refine_call_output(op, &arg_types);
                    }
                    // Also check user-defined module functions
                    let qualified_name = format!("{}::{}", enum_name, variant);
                    if let Some(sig) = self.function_sigs.get(&qualified_name).cloned() {
                        if let Some(sig_mut) = self.function_sigs.get_mut(&qualified_name) {
                            sig_mut.used = true;
                        }
                        let max_args = if sig.has_rest { usize::MAX } else { sig.params.len() };
                        if arg_types.len() < sig.required_params || arg_types.len() > max_args {
                            let arity_msg = if sig.has_rest {
                                format!("at least {}", sig.required_params)
                            } else if sig.required_params == sig.params.len() {
                                format!("{}", sig.params.len())
                            } else {
                                format!("{}-{}", sig.required_params, sig.params.len())
                            };
                            self.emit_coded(
                                expr.span.start_line,
                                expr.span.start_col,
                                format!(
                                    "function '{}' expects {} arguments, got {}",
                                    qualified_name, arity_msg, arg_types.len()
                                ),
                                DiagnosticSeverity::Error,
                                super::errors::ErrorCode::E405,
                                None,
                            );
                        }
                        return sig.return_type;
                    }
                    self.emit_coded(
                        expr.span.start_line, expr.span.start_col,
                        format!("undefined enum '{}'", enum_name),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E201, None,
                    );
                    return ChannelType::Map;
                }
                for arg in args {
                    self.infer_expr(arg);
                }
                ChannelType::Map
            }

            ExpressionKind::StructConstruct { name, fields } => {
                // Check for duplicate field names and infer field types
                let mut seen = HashSet::new();
                let mut field_types: HashMap<&str, ChannelType> = HashMap::new();
                let mut has_spread = false;
                for (field_name, field_expr) in fields {
                    // Skip spread entries (__spread is the struct update syntax marker)
                    if field_name == "__spread" {
                        has_spread = true;
                        self.infer_expr(field_expr);
                        continue;
                    }
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
                            source_file: None,
                        });
                    }
                    let ft = self.infer_expr(field_expr);
                    field_types.insert(field_name.as_str(), ft);
                }
                // Validate fields against struct definition
                if let Some(def_fields) = self.struct_defs.get(name.as_str()).cloned() {
                    let provided: HashSet<&str> = fields.iter()
                        .filter(|(f, _)| f != "__spread")
                        .map(|(f, _)| f.as_str()).collect();
                    let defined: HashSet<&str> = def_fields.iter().map(|(f, _)| f.as_str()).collect();
                    let def_names: Vec<&str> = def_fields.iter().map(|(f, _)| f.as_str()).collect();
                    // Field count mismatch warning (skip if spread is present or defaults exist)
                    if !has_spread && provided.len() != def_fields.len() {
                        self.emit_coded(
                            expr.span.start_line,
                            expr.span.start_col,
                            format!(
                                "struct '{}' expects {} fields, got {}",
                                name, def_fields.len(), provided.len(),
                            ),
                            DiagnosticSeverity::Warning,
                            super::errors::ErrorCode::E100,
                            None,
                        );
                    }
                    // Check for unknown fields
                    for (field_name, _) in fields {
                        if field_name == "__spread" { continue; }
                        if !defined.contains(field_name.as_str()) {
                            let suggestion = super::errors::suggest_name(field_name, &def_names);
                            self.emit_coded(
                                expr.span.start_line,
                                expr.span.start_col,
                                format!(
                                    "struct '{}' has no field '{}'",
                                    name, field_name,
                                ),
                                DiagnosticSeverity::Error,
                                super::errors::ErrorCode::E202,
                                suggestion,
                            );
                        }
                    }
                    // Check for missing fields (skip if spread is present since spread may fill them)
                    if !has_spread {
                        for (def_field, _) in &def_fields {
                            if !provided.contains(def_field.as_str()) {
                                self.emit_coded(
                                    expr.span.start_line,
                                    expr.span.start_col,
                                    format!(
                                        "missing field '{}' in struct '{}' constructor",
                                        def_field, name,
                                    ),
                                    DiagnosticSeverity::Warning,
                                    super::errors::ErrorCode::E100,
                                    None,
                                );
                            }
                        }
                    }
                    // Validate field types against annotations
                    for (def_field, ann) in &def_fields {
                        if let Some(ann_str) = ann {
                            if let Some(inferred) = field_types.get(def_field.as_str()) {
                                if let Some(expected) = self.resolve_type(ann_str) {
                                    if *inferred != ChannelType::Null
                                        && *inferred != expected
                                        && !inferred.is_compatible_with(&expected)
                                    {
                                        self.emit_coded(
                                            expr.span.start_line,
                                            expr.span.start_col,
                                            format!(
                                                "field '{}' in struct '{}' expects type '{}' but got '{}'",
                                                def_field, name, ann_str, inferred.as_str()
                                            ),
                                            DiagnosticSeverity::Warning,
                                            super::errors::ErrorCode::W112,
                                            None,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                ChannelType::Map
            }

            ExpressionKind::TryPropagate(inner) => {
                self.infer_expr(inner)
            }

            ExpressionKind::Yield(inner) => {
                self.infer_expr(inner)
            }

            ExpressionKind::UnsafeBlock(block) => {
                self.infer_block(block)
            }

            ExpressionKind::InlineAsm { operands, .. } => {
                for op in operands {
                    self.infer_expr(op);
                }
                ChannelType::Null
            }

            ExpressionKind::Ref(inner) => {
                self.infer_expr(inner)
            }

            ExpressionKind::MoveClosure { params, body } => {
                self.push_scope();
                self.function_depth += 1;
                let saved_loop_depth = self.loop_depth;
                self.loop_depth = 0;
                let prev_return_type = std::mem::replace(&mut self.current_return_type, ChannelType::Null);
                for param in params {
                    let ct = param
                        .type_annotation
                        .as_ref()
                        .and_then(|ta| self.resolve_type(&ta.to_string()))
                        .unwrap_or(ChannelType::Null);
                    if let Some(default_expr) = &param.default {
                        self.check_default_param_type(default_expr, &ct);
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
                ChannelType::Null
            }

            ExpressionKind::DynTrait(_) => {
                ChannelType::Null
            }
        }
    }

    // Pattern variable binding (for match expressions)

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
                // Validate that all alternatives bind the same set of variables.
                if alternatives.len() >= 2 {
                    let first_vars = Self::collect_pattern_var_names(&alternatives[0]);
                    for alt in &alternatives[1..] {
                        let alt_vars = Self::collect_pattern_var_names(alt);
                        if first_vars != alt_vars {
                            self.emit_coded(
                                span.start_line,
                                span.start_col,
                                "or-pattern alternatives bind different variables; all alternatives must bind the same names".to_string(),
                                DiagnosticSeverity::Warning,
                                super::errors::ErrorCode::W113,
                                None,
                            );
                            break;
                        }
                    }
                }
                // Collect all unique variable names from all alternatives,
                // binding each only once to avoid duplicate definitions.
                let mut bound = std::collections::HashSet::new();
                for alt in alternatives {
                    self.bind_pattern_vars_collecting(alt, val_type, span, &mut bound);
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
            Pattern::EnumPattern { enum_name, variant, bindings } => {
                // Validate variant exists and check binding count
                if let Some(variants) = self.enum_variants.get(enum_name.as_str()) {
                    if let Some((_, expected_fields)) = variants.iter().find(|(v, _)| v == variant) {
                        if bindings.len() != *expected_fields {
                            self.emit_coded(
                                span.start_line,
                                span.start_col,
                                format!(
                                    "enum pattern '{}::{}' expects {} bindings, got {}",
                                    enum_name, variant, expected_fields, bindings.len()
                                ),
                                DiagnosticSeverity::Error,
                                super::errors::ErrorCode::E405,
                                None,
                            );
                        }
                    } else {
                        let variant_names: Vec<&str> = variants.iter().map(|(v, _)| v.as_str()).collect();
                        self.emit_coded(
                            span.start_line,
                            span.start_col,
                            format!(
                                "enum '{}' has no variant '{}'",
                                enum_name, variant,
                            ),
                            DiagnosticSeverity::Error,
                            super::errors::ErrorCode::E202,
                            super::errors::suggest_name(variant, &variant_names),
                        );
                    }
                }
                for sub in bindings {
                    self.bind_pattern_vars(sub, ChannelType::Null, span);
                }
            }
            Pattern::TypePattern { name, type_name } => {
                let resolved = self.resolve_type(type_name);
                let ct = resolved.unwrap_or(ChannelType::Null);
                if resolved.is_none() {
                    self.emit_coded(
                        span.start_line,
                        span.start_col,
                        format!("unknown type '{}' in type pattern", type_name),
                        DiagnosticSeverity::Warning,
                        super::errors::ErrorCode::E100,
                        None,
                    );
                }
                self.define_var(name, ct, false, span.start_line, span.start_col);
            }
            Pattern::Binding { name, pattern } => {
                self.define_var(name, val_type, false, span.start_line, span.start_col);
                self.bind_pattern_vars(pattern, val_type, span);
            }
            Pattern::RangePattern { start, end, .. } => {
                // No variables to bind, but walk the bound expressions for type checking
                self.infer_expr(start);
                self.infer_expr(end);
            }
        }
    }

    /// Like `bind_pattern_vars`, but tracks already-bound names to avoid duplicates
    /// across Or-pattern alternatives.
    fn bind_pattern_vars_collecting(
        &mut self,
        pattern: &Pattern,
        val_type: ChannelType,
        span: &Span,
        bound: &mut std::collections::HashSet<String>,
    ) {
        match pattern {
            Pattern::Literal(_) | Pattern::Wildcard => {}
            Pattern::Variable(name) => {
                if bound.insert(name.clone()) {
                    self.define_var(name, val_type, false, span.start_line, span.start_col);
                }
            }
            Pattern::Array(sub_patterns) => {
                for sub in sub_patterns {
                    self.bind_pattern_vars_collecting(sub, ChannelType::Null, span, bound);
                }
            }
            Pattern::Map(entries) => {
                for (_, sub_pattern) in entries {
                    self.bind_pattern_vars_collecting(sub_pattern, ChannelType::Null, span, bound);
                }
            }
            Pattern::Or(alternatives) => {
                for alt in alternatives {
                    self.bind_pattern_vars_collecting(alt, val_type, span, bound);
                }
            }
            Pattern::Rest(name) => {
                if let Some(name) = name {
                    if bound.insert(name.clone()) {
                        self.define_var(name, ChannelType::Array, false, span.start_line, span.start_col);
                    }
                }
            }
            Pattern::EnumPattern { bindings, .. } => {
                for sub in bindings {
                    self.bind_pattern_vars_collecting(sub, ChannelType::Null, span, bound);
                }
            }
            Pattern::TypePattern { name, type_name } => {
                if bound.insert(name.clone()) {
                    let ct = self.resolve_type(type_name).unwrap_or(ChannelType::Null);
                    self.define_var(name, ct, false, span.start_line, span.start_col);
                }
            }
            Pattern::Binding { name, pattern } => {
                if bound.insert(name.clone()) {
                    self.define_var(name, val_type, false, span.start_line, span.start_col);
                }
                self.bind_pattern_vars_collecting(pattern, val_type, span, bound);
            }
            Pattern::RangePattern { start, end, .. } => {
                self.infer_expr(start);
                self.infer_expr(end);
            }
        }
    }

    // Binary operator type inference

    fn infer_binop(
        &mut self,
        op: BinOp,
        left: ChannelType,
        right: ChannelType,
        _span: Span,
    ) -> ChannelType {
        match op {
            // Comparison operators always return Bool.
            BinOp::Eq | BinOp::NotEq | BinOp::Gt | BinOp::Lt | BinOp::GtEq | BinOp::LtEq => {
                ChannelType::Bool
            }

            // Containment operator always returns Bool.
            BinOp::In => ChannelType::Bool,

            // Logical operators: accept any truthy/falsy value (&&/|| use truthiness).
            // Short-circuit: && returns lhs if falsy, else rhs; || returns lhs if truthy, else rhs.
            BinOp::And | BinOp::Or => {
                ChannelType::Null
            }

            // Arithmetic operators: use the operation's typing rules.
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                // String concatenation: Add with any String operand returns String.
                if op == BinOp::Add && (left == ChannelType::String || right == ChannelType::String) {
                    return ChannelType::String;
                }

                // String repetition: "abc" * 3 or 3 * "abc" returns String.
                if op == BinOp::Mul
                    && ((left == ChannelType::String && is_numeric(right))
                        || (is_numeric(left) && right == ChannelType::String))
                {
                    return ChannelType::String;
                }

                // Division: uses numeric promotion for all cases (int/int = int, preserves Float32)
                if op == BinOp::Div {
                    return promote_numeric(&[left, right]);
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

            // Bitwise operators always return Int64.
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr | BinOp::AndNot => {
                ChannelType::Int64
            }
        }
    }

    // Binary operator literal checks

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
                "division by zero".to_string(),
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
                "modulo by 1 always returns 0".to_string(),
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
                "multiplication by 0 always returns 0".to_string(),
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
                "comparison with boolean literal is unnecessary".to_string(),
                DiagnosticSeverity::Warning,
                super::errors::ErrorCode::W106,
                None,
            );
        }

        // Self-comparison is handled by the linter as W205 (more thorough:
        // covers literals and complex expressions, not just variables).

        // W2: Arithmetic on non-numeric types.
        // Exempt Add on strings (string concatenation is valid).
        if matches!(
            op,
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod
        ) {
            let is_string_concat = op == BinOp::Add
                && (left_ty == ChannelType::String || right_ty == ChannelType::String);
            let is_string_repeat = op == BinOp::Mul
                && ((left_ty == ChannelType::String && is_numeric(right_ty))
                    || (is_numeric(left_ty) && right_ty == ChannelType::String));
            if !is_string_concat && !is_string_repeat {
                for ty in [left_ty, right_ty] {
                    if ty != ChannelType::Null && !is_numeric(ty) {
                        self.emit_coded(
                            span.start_line,
                            span.start_col,
                            format!(
                                "arithmetic operator '{}' expects numeric operands, got {}",
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
                format!("comparing {} with {}", left_ty.as_str(), right_ty.as_str()),
                DiagnosticSeverity::Warning,
                super::errors::ErrorCode::E100,
                None,
            );
        }
    }


    /// Resolve a type name, following type aliases if necessary.
    /// Returns `Some(ChannelType)` if the name is a built-in type or resolves
    /// through aliases to one. Follows chains (A → B → int64) with a depth
    /// limit to guard against cycles.
    fn resolve_type(&self, name: &str) -> Option<ChannelType> {
        if let Some(ct) = ChannelType::parse(name) {
            return Some(ct);
        }
        if self.generic_params.contains(name) {
            return Some(ChannelType::Null); // Generic params accept any type
        }
        // Follow alias chain with depth limit to prevent infinite loops on cycles.
        let mut current = name;
        for _ in 0..32 {
            match self.type_aliases.get(current) {
                Some(target) => {
                    if let Some(ct) = ChannelType::parse(target) {
                        return Some(ct);
                    }
                    if self.generic_params.contains(target.as_str()) {
                        return Some(ChannelType::Null);
                    }
                    current = target;
                }
                None => return None,
            }
        }
        None // cycle detected or chain too long
    }


    fn resolve_type_annotation(&self, ann: &TypeAnnotation) -> Option<ChannelType> {
        match ann {
            TypeAnnotation::Simple(name) => self.resolve_type(name),
            TypeAnnotation::Generic { base, .. } => self.resolve_type(base),
            TypeAnnotation::Union(_) => Some(ChannelType::Null),
            TypeAnnotation::Optional(_) => Some(ChannelType::Null),
            TypeAnnotation::Function { .. } => Some(ChannelType::Null),
            TypeAnnotation::Tuple(_) => Some(ChannelType::Array),
        }
    }


    /// If a type annotation is present, parse it and check compatibility with
    /// the inferred type. Returns the definitive type (annotation wins if valid).
    fn reconcile_annotation(
        &mut self,
        annotation: Option<&TypeAnnotation>,
        inferred: ChannelType,
        line: u32,
        col: u32,
        var_name: &str,
    ) -> ChannelType {
        let ann = match annotation {
            Some(a) => a,
            None => return inferred,
        };

        let ann_type = match self.resolve_type_annotation(ann) {
            Some(ct) => ct,
            None => {
                self.emit_coded(
                    line,
                    col,
                    format!("unknown type annotation '{}' on '{}'", ann, var_name),
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
                "type annotation '{}' on '{}' conflicts with inferred type '{}'",
                ann,
                var_name,
                inferred.as_str()
            ),
            DiagnosticSeverity::Warning,
            super::errors::ErrorCode::E100,
            None,
        );
        ann_type
    }


    /// Try to evaluate an expression at compile time, returning a constant
    /// literal if the expression is a literal or simple arithmetic on constants.
    fn try_const_fold(&self, expr: &Expression) -> Option<ConstLiteral> {
        match &expr.kind {
            ExpressionKind::Literal(Literal::Int64(n)) => Some(ConstLiteral::Int64(*n)),
            ExpressionKind::Literal(Literal::Float64(n)) => Some(ConstLiteral::Float64(*n)),
            ExpressionKind::Literal(Literal::String(s)) => Some(ConstLiteral::String(s.clone())),
            ExpressionKind::Literal(Literal::Bool(b)) => Some(ConstLiteral::Bool(*b)),
            ExpressionKind::Variable(name) => self.const_literals.get(name.as_str()).cloned(),
            ExpressionKind::UnaryOp { op, operand } => {
                let val = self.try_const_fold(operand)?;
                match (op, &val) {
                    (UnOp::Neg, ConstLiteral::Int64(n)) => Some(ConstLiteral::Int64(-n)),
                    (UnOp::Neg, ConstLiteral::Float64(n)) => Some(ConstLiteral::Float64(-n)),
                    (UnOp::Not, ConstLiteral::Bool(b)) => Some(ConstLiteral::Bool(!b)),
                    _ => None,
                }
            }
            ExpressionKind::BinaryOp { op, left, right } => {
                let lhs = self.try_const_fold(left)?;
                let rhs = self.try_const_fold(right)?;
                match (op, &lhs, &rhs) {
                    (BinOp::Add, ConstLiteral::Int64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Int64(a.wrapping_add(*b))),
                    (BinOp::Sub, ConstLiteral::Int64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Int64(a.wrapping_sub(*b))),
                    (BinOp::Mul, ConstLiteral::Int64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Int64(a.wrapping_mul(*b))),
                    (BinOp::Div, ConstLiteral::Int64(a), ConstLiteral::Int64(b)) if *b != 0 => Some(ConstLiteral::Int64(a / b)),
                    (BinOp::Mod, ConstLiteral::Int64(a), ConstLiteral::Int64(b)) if *b != 0 => Some(ConstLiteral::Int64(a % b)),
                    (BinOp::Add, ConstLiteral::Float64(a), ConstLiteral::Float64(b)) => Some(ConstLiteral::Float64(a + b)),
                    (BinOp::Sub, ConstLiteral::Float64(a), ConstLiteral::Float64(b)) => Some(ConstLiteral::Float64(a - b)),
                    (BinOp::Mul, ConstLiteral::Float64(a), ConstLiteral::Float64(b)) => Some(ConstLiteral::Float64(a * b)),
                    (BinOp::Div, ConstLiteral::Float64(a), ConstLiteral::Float64(b)) if *b != 0.0 => Some(ConstLiteral::Float64(a / b)),
                    // Mixed int/float
                    (BinOp::Add, ConstLiteral::Int64(a), ConstLiteral::Float64(b)) => Some(ConstLiteral::Float64(*a as f64 + b)),
                    (BinOp::Add, ConstLiteral::Float64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Float64(a + *b as f64)),
                    (BinOp::Sub, ConstLiteral::Int64(a), ConstLiteral::Float64(b)) => Some(ConstLiteral::Float64(*a as f64 - b)),
                    (BinOp::Sub, ConstLiteral::Float64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Float64(a - *b as f64)),
                    (BinOp::Mul, ConstLiteral::Int64(a), ConstLiteral::Float64(b)) => Some(ConstLiteral::Float64(*a as f64 * b)),
                    (BinOp::Mul, ConstLiteral::Float64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Float64(a * *b as f64)),
                    (BinOp::Div, ConstLiteral::Int64(a), ConstLiteral::Float64(b)) if *b != 0.0 => Some(ConstLiteral::Float64(*a as f64 / b)),
                    (BinOp::Div, ConstLiteral::Float64(a), ConstLiteral::Int64(b)) if *b != 0 => Some(ConstLiteral::Float64(a / *b as f64)),
                    (BinOp::Add, ConstLiteral::String(a), ConstLiteral::String(b)) => Some(ConstLiteral::String(format!("{}{}", a, b))),
                    (BinOp::And, ConstLiteral::Bool(a), ConstLiteral::Bool(b)) => Some(ConstLiteral::Bool(*a && *b)),
                    (BinOp::Or, ConstLiteral::Bool(a), ConstLiteral::Bool(b)) => Some(ConstLiteral::Bool(*a || *b)),
                    (BinOp::Eq, ConstLiteral::Int64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Bool(a == b)),
                    (BinOp::NotEq, ConstLiteral::Int64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Bool(a != b)),
                    (BinOp::Lt, ConstLiteral::Int64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Bool(a < b)),
                    (BinOp::Gt, ConstLiteral::Int64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Bool(a > b)),
                    (BinOp::LtEq, ConstLiteral::Int64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Bool(a <= b)),
                    (BinOp::GtEq, ConstLiteral::Int64(a), ConstLiteral::Int64(b)) => Some(ConstLiteral::Bool(a >= b)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    // Finalize: collect unused variables/imports, build result

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
                        message: if info.is_const {
                            format!("unused constant '{}'", name)
                        } else {
                            format!("unused variable '{}'", name)
                        },
                        severity: DiagnosticSeverity::Warning,
                        code: Some(code.to_string()),
                        help: Some(code.help().to_string()),
                        suggestion: None,
                        source_file: None,
                    });
                } else if info.mutable && !info.mutated {
                    let code = super::errors::ErrorCode::W110;
                    self.diagnostics.push(AstDiagnostic {
                        line: info.def_line,
                        column: info.def_col,
                        message: format!("variable '{}' declared as mutable but never reassigned", name),
                        severity: DiagnosticSeverity::Warning,
                        code: Some(code.to_string()),
                        help: Some(code.help().to_string()),
                        suggestion: None,
                        source_file: None,
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
                    column: sig.def_col,
                    message: format!("unused function '{}'", name),
                    severity: DiagnosticSeverity::Warning,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: None,
                    source_file: None,
                });
            }
        }

        // Unused imports.
        for import_id in &self.imports {
            if !self.used_imports.contains(import_id) {
                let code = super::errors::ErrorCode::W101;
                let (line, col) = self.import_locations.get(import_id).copied().unwrap_or((1, 1));
                self.diagnostics.push(AstDiagnostic {
                    line,
                    column: col,
                    message: format!("unused import '{}'", import_id),
                    severity: DiagnosticSeverity::Warning,
                    code: Some(code.to_string()),
                    help: Some(code.help().to_string()),
                    suggestion: None,
                    source_file: None,
                });
            }
        }

        AstTypeAnalysis {
            diagnostics: self.diagnostics,
            variable_types,
        }
    }
}


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
    let mut has_float64 = false;
    let mut has_float32 = false;
    let mut has_int = false;
    let mut common: Option<ChannelType> = None;

    for ct in inputs {
        match ct {
            ChannelType::Float64 => {
                has_float64 = true;
            }
            ChannelType::Float32 => {
                has_float32 = true;
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

    if has_float64 {
        ChannelType::Float64
    } else if has_float32 {
        if has_int { ChannelType::Float64 } else { ChannelType::Float32 }
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
        Add | Subtract | Multiply | Divide | Modulo | Power | Min | Max | Negate | Abs | Round
        | Floor | Ceil | Sign | Clamp => promote_numeric(arg_types),

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

// is_empty_block, block_contains_break, expr_contains_break removed —
// these checks are now handled by the linter (W206, W204).

/// Return available method names for a given ChannelType (for "did you mean?" suggestions).
fn available_methods_for_channel_type(obj_type: ChannelType) -> Vec<&'static str> {
    let mut methods: Vec<&'static str> = Vec::new();
    match obj_type {
        ChannelType::Array => {
            methods.extend_from_slice(&["first", "last", "is_empty", "sum", "product", "min", "max", "join"]);
            methods.extend_from_slice(&["map", "filter", "reduce", "find", "find_index", "any", "all",
                "flat_map", "each", "sort_by", "group_by", "min_by", "max_by",
                "take_while", "skip_while", "partition", "scan", "enumerate", "zip", "chunk"]);
            methods.extend_from_slice(&["push", "pop", "shift", "len", "length", "get", "set",
                "slice", "contains", "sort", "reverse", "flatten", "concat",
                "unique", "insert", "remove", "filter_nulls"]);
        }
        ChannelType::String => {
            methods.extend_from_slice(&["is_empty", "is_numeric", "is_alphabetic", "to_int", "to_float",
                "len", "length", "trim", "trim_start", "trim_end", "to_upper", "to_uppercase",
                "to_lower", "to_lowercase", "reverse", "chars", "lines", "pad_start", "pad_end",
                "char_at", "repeat", "substring", "slice", "index_of",
                "split", "contains", "replace", "starts_with", "ends_with",
                "words", "count"]);
        }
        ChannelType::Int64 => {
            methods.extend_from_slice(&["abs", "sign", "pow", "min", "max", "clamp"]);
        }
        ChannelType::Int32 => {
            methods.extend_from_slice(&["abs", "sign", "to_int32", "pow", "min", "max", "clamp"]);
        }
        ChannelType::Uint32 => {
            methods.extend_from_slice(&["abs", "sign", "to_uint32", "pow", "min", "max", "clamp"]);
        }
        ChannelType::Uint64 => {
            methods.extend_from_slice(&["abs", "sign", "to_uint64", "pow", "min", "max", "clamp"]);
        }
        ChannelType::Float64 => {
            methods.extend_from_slice(&["abs", "round", "floor", "ceil", "sqrt", "is_nan", "is_infinite", "is_finite",
                "sign", "to_float32", "pow", "min", "max", "clamp",
                "ln", "log2", "log10", "sin", "cos", "tan",
                "asin", "acos", "atan", "sinh", "cosh", "tanh", "exp"]);
        }
        ChannelType::Float32 => {
            methods.extend_from_slice(&["abs", "round", "floor", "ceil", "sqrt", "is_nan", "is_infinite", "is_finite",
                "sign", "to_float32", "pow", "min", "max", "clamp",
                "ln", "log2", "log10", "sin", "cos", "tan",
                "asin", "acos", "atan", "sinh", "cosh", "tanh", "exp"]);
        }
        ChannelType::Map => {
            methods.extend_from_slice(&["get", "set", "delete", "has", "keys", "values", "entries",
                "merge", "len", "length", "size",
                "filter_entries", "map_values", "map_keys"]);
        }
        ChannelType::Bytes => {
            methods.extend_from_slice(&["len", "length", "slice", "concat", "contains",
                "base64_encode", "base64_decode"]);
        }
        _ => {}
    }
    // Generic methods available on all types
    methods.extend_from_slice(&["to_string", "to_int64", "to_float64", "to_bool", "to_json", "typeof"]);
    methods
}

/// Resolve a method name on a given type to an OperationType name.
/// Returns None if the method is unknown for that type.
fn resolve_method_type(obj_type: ChannelType, method: &str) -> Option<String> {
    // Generic methods available on ALL types
    match method {
        "to_string" | "to_json" | "to_int64" | "to_float64" | "to_bool" | "typeof" => {
            return Some("generic_method".into());
        }
        _ => {}
    }
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
            "sort" => Some("array_sort".into()),
            "reverse" => Some("array_reverse".into()),
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
            | "skip_while" | "zip" | "enumerate" | "chunk" => {
                Some("array_hof".into())
            }
            // Evaluator-dispatched methods
            "insert" => Some("array_insert".into()),
            "remove" => Some("array_remove".into()),
            "shift" => Some("array_shift".into()),
            "filter_nulls" => Some("array_filter_nulls".into()),
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
            "chars" => Some("string_chars".into()),
            "lines" => Some("string_lines".into()),
            "repeat" => Some("string_repeat".into()),
            "substring" | "slice" => Some("substring".into()),
            "index_of" => Some("index_of".into()),
            "pad_start" => Some("pad_start".into()),
            "pad_end" => Some("pad_end".into()),
            "reverse" => Some("string_reverse".into()),
            "is_empty" | "is_numeric" | "is_alphabetic" => Some("string_predicate".into()),
            "to_int" | "to_float" => Some("string_convert".into()),
            "char_at" => Some("string_char_at".into()),
            "words" => Some("string_words".into()),
            "count" => Some("string_count".into()),
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
            "base64_encode" => Some("base64_encode".into()),
            "base64_decode" => Some("base64_decode".into()),
            _ => None,
        },
        ChannelType::Int64 => match method {
            "abs" | "sign" | "to_string" | "to_float64" | "to_int64" | "pow" | "min"
            | "max" | "clamp" => Some("numeric_method".into()),
            _ => None,
        },
        ChannelType::Int32 => match method {
            "abs" | "sign" | "to_string" | "to_float64" | "to_int64" | "to_int32" | "pow" | "min"
            | "max" | "clamp" => Some("numeric_method".into()),
            _ => None,
        },
        ChannelType::Uint32 => match method {
            "abs" | "sign" | "to_string" | "to_float64" | "to_int64" | "to_uint32" | "pow" | "min"
            | "max" | "clamp" => Some("numeric_method".into()),
            _ => None,
        },
        ChannelType::Uint64 => match method {
            "abs" | "sign" | "to_string" | "to_float64" | "to_int64" | "to_uint64" | "pow" | "min"
            | "max" | "clamp" => Some("numeric_method".into()),
            _ => None,
        },
        ChannelType::Float64 => match method {
            "abs" | "round" | "floor" | "ceil" | "sqrt" | "sign" | "to_string"
            | "to_int64" | "to_float64" | "to_float32" | "pow" | "min" | "max" | "clamp" | "is_nan"
            | "is_infinite" | "is_finite" | "ln" | "log2" | "log10" | "sin" | "cos"
            | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "exp" => Some("numeric_method".into()),
            _ => None,
        },
        ChannelType::Float32 => match method {
            "abs" | "round" | "floor" | "ceil" | "sqrt" | "sign" | "to_string"
            | "to_int64" | "to_float64" | "to_float32" | "pow" | "min" | "max" | "clamp" | "is_nan"
            | "is_infinite" | "is_finite" | "ln" | "log2" | "log10" | "sin" | "cos"
            | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "exp" => Some("numeric_method".into()),
            _ => None,
        },
        _ => None,
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
        if non_null.iter().all(|t| is_numeric(*t)) {
            promote_numeric(&non_null)
        } else {
            ChannelType::Null
        }
    }
}

/// Extract type narrowing info from an `if` condition.
/// Recognizes patterns like `typeof(x) == "string"` or `"string" == typeof(x)`.
/// Returns `Some((variable_name, narrowed_type))` if the pattern matches.
fn extract_typeof_narrowing(condition: &Expression) -> Option<(String, ChannelType)> {
    if let ExpressionKind::BinaryOp { op: BinOp::Eq, left, right } = &condition.kind {
        // typeof(x) == "type_name"
        if let (Some(var_name), Some(type_name)) = (extract_typeof_var(left), extract_string_literal(right)) {
            return ChannelType::parse(&type_name).map(|ct| (var_name, ct));
        }
        // "type_name" == typeof(x)
        if let (Some(type_name), Some(var_name)) = (extract_string_literal(left), extract_typeof_var(right)) {
            return ChannelType::parse(&type_name).map(|ct| (var_name, ct));
        }
    }
    None
}

/// Extract the variable name from a `typeof(x)` call expression.
fn extract_typeof_var(expr: &Expression) -> Option<String> {
    if let ExpressionKind::Call { name, args, .. } = &expr.kind {
        if name == "typeof" && args.len() == 1 {
            if let ExpressionKind::Variable(var_name) = &args[0].kind {
                return Some(var_name.clone());
            }
        }
    }
    None
}

/// Extract a string literal value from an expression.
fn extract_string_literal(expr: &Expression) -> Option<String> {
    if let ExpressionKind::Literal(Literal::String(s)) = &expr.kind {
        Some(s.clone())
    } else {
        None
    }
}


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
    fn test_divide_int_int_returns_int() {
        let a = check("let x = 10;\nlet y = 3;\nlet r = x / y;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_divide_int_float_returns_float() {
        let a = check("let x = 10;\nlet y = 3.0;\nlet r = x / y;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_multiply_int_int() {
        let a = check("let x = 5;\nlet y = 3;\nlet r = x * y;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
    }

    // Comparison and logical operators

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
    fn test_logical_and_returns_any() {
        // &&/|| use truthiness and return either operand, so type is polymorphic (Null = any)
        let a = check("let x = true;\nlet y = false;\nlet r = x && y;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Null));
    }

    #[test]
    fn test_logical_or_returns_any() {
        let a = check("let x = true;\nlet y = false;\nlet r = x || y;\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Null));
    }

    #[test]
    fn test_logical_non_bool_no_warning() {
        // &&/|| accept any truthy/falsy value — no warning for non-bool operands
        let a = check("let x = 10;\nlet y = 20;\nlet r = x && y;\noutput r;");
        let w = warnings(&a);
        assert!(!w.iter().any(|d| d.message.contains("should be bool")));
    }


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
            .any(|d| d.message.contains("logical NOT expects bool")));
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
            .any(|d| d.message.contains("negation expects numeric")));
    }


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
        assert!(e[0].message.contains("cannot assign to immutable"));
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
        assert!(e.iter().any(|d| d.message.contains("undefined variable")));
    }

    // Use-before-define

    #[test]
    fn test_undefined_variable_error() {
        let a = check("let r = x + 1;\noutput r;");
        let e = errors(&a);
        assert!(e
            .iter()
            .any(|d| d.message.contains("undefined variable 'x'")));
    }

    #[test]
    fn test_defined_variable_no_error() {
        let a = check("let x = 10;\nlet r = x + 1;\noutput r;");
        let e = errors(&a);
        assert!(e.is_empty());
    }


    #[test]
    fn test_unused_variable_warns() {
        let a = check("let x = 42;");
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("unused variable 'x'")));
    }

    #[test]
    fn test_used_variable_no_warning() {
        let a = check("let x = 42;\noutput x;");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("unused")));
    }

    #[test]
    fn test_underscore_prefix_no_unused_warning() {
        let a = check("let _x = 42;");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("unused")));
    }

    #[test]
    fn test_variable_used_in_closure_no_warning() {
        let a = check("let x = 42;\nlet add = |n| n + x;\noutput add(1);");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("unused variable")),
            "No variables should be reported as unused. Got: {:?}", w);
    }

    #[test]
    fn test_variable_used_in_nested_function_no_warning() {
        let a = check("let x = 10;\nfn foo() { output x; }\nfoo();");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("unused variable 'x'")),
            "Variable 'x' used in nested function should not be reported as unused. Got: {:?}", w);
    }


    #[test]
    fn test_unused_import_warns() {
        let a = check(r#"import "capture";"#);
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("unused import 'capture'")));
    }

    #[test]
    fn test_used_import_no_warning() {
        let a = check(
            r#"import "capture";
let frame = capture();
output frame;"#,
        );
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("unused import")));
    }


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
            .any(|d| d.message.contains("undefined function or operation 'foobar'")));
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
        assert!(w.iter().any(|d| d.message.contains("type mismatch")));
    }

    // If/else

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

    // Index and field access

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
            .any(|d| d.message.contains("indexing requires array")));
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


    #[test]
    fn test_range_returns_array() {
        // range() is parsed as a Call, not ExpressionKind::Range
        // op_input_types for Range are polymorphic (Null), so no type warnings
        let a = check("let r = range(0, 10);\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Array));
        assert!(errors(&a).is_empty());
    }


    #[test]
    fn test_block_scope_isolation() {
        // Variable defined inside a block shouldn't leak out
        // (In practice our type checker tracks it but the lowering handles scoping)
        let a = check("let x = 10;\nif true { let _y = 20; }\noutput x;");
        assert!(errors(&a).is_empty());
    }


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


    #[test]
    fn test_plugin_call_returns_null() {
        let a = check(
            r#"import "capture";
let frame = capture();
output frame;"#,
        );
        assert_eq!(a.variable_types.get("frame"), Some(&ChannelType::Null));
    }


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

    // User-defined functions

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
            .any(|d| d.message.contains("unused function 'unused_fn'")));
    }

    #[test]
    fn test_fn_used_no_warning() {
        let a = check("fn double(x: int64) -> int64 { x * 2 }\nlet r = double(5);\noutput r;");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("unused function")));
    }

    #[test]
    fn test_fn_main_no_unused_warning() {
        let a = check("fn main() { let x = 42; output x; }");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("unused function")));
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
        assert!(e.iter().any(|d| d.message.contains("undefined function or operation")));
    }

    #[test]
    fn test_fn_untyped_return_is_null() {
        let a = check("fn notype(x: int64) { x; }\nlet r = notype(5);\noutput r;");
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Null));
    }


    #[test]
    fn test_reserved_keyword_warning_in_define_var() {
        // Direct unit test of the type checker's define_var warning
        let imports = HashSet::new();
        let mut checker = TypeChecker::new(&imports);
        checker.define_var("ref", ChannelType::Int64, false, 1, 1);
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
        assert!(e.iter().any(|d| d.message.contains("type mismatch")));
    }

    // Linting diagnostics — errors

    #[test]
    fn test_division_by_zero_error() {
        let a = check("let x = 10;\nlet r = x / 0;\noutput r;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("division by zero")));
    }

    #[test]
    fn test_modulo_by_zero_error() {
        let a = check("let x = 10;\nlet r = x % 0;\noutput r;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("division by zero")));
    }

    #[test]
    fn test_division_by_float_zero_error() {
        let a = check("let x = 10.0;\nlet r = x / 0.0;\noutput r;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("division by zero")));
    }

    #[test]
    fn test_negative_array_index_error() {
        let a = check("let arr = [1, 2, 3];\nlet r = arr[-1];\noutput r;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("negative array index")));
    }

    #[test]
    fn test_empty_array_index_error() {
        let a = check("let r = [][0];\noutput r;");
        let e = errors(&a);
        assert!(e
            .iter()
            .any(|d| d.message.contains("index into empty array literal")));
    }

    #[test]
    fn test_duplicate_map_keys_error() {
        let a = check(r#"let m = {"a": 1, "a": 2}; output m;"#);
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("duplicate key 'a'")));
    }

    #[test]
    fn test_placeholder_outside_pipe_error() {
        let a = check("let x = _;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d
            .message
            .contains("placeholder '_' can only be used inside pipe")));
    }

    #[test]
    fn test_placeholder_inside_pipe_ok() {
        let a = check(
            r#"let x = "hello";
let r = x |> to_upper(_);
output r;"#,
        );
        let e = errors(&a);
        assert!(e.iter().all(|d| !d.message.contains("placeholder")));
    }

    #[test]
    fn test_unknown_type_annotation_is_error() {
        let a = check("let x: foobar = 1;\noutput x;");
        let e = errors(&a);
        assert!(e
            .iter()
            .any(|d| d.message.contains("unknown type annotation")));
    }

    // Linting diagnostics — warnings

    #[test]
    fn test_variable_shadowing_moved_to_linter() {
        // W102 (shadowing) now handled by linter W209
        let a = check("let x = 1;\nlet x = 2;\noutput x;");
        let w = warnings(&a);
        assert!(!w
            .iter()
            .any(|d| d.message.contains("shadows previous definition")),
            "shadowing check should no longer be emitted by type checker");
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
            .any(|d| d.message.contains("arithmetic operator") && d.message.contains("string")));
    }

    #[test]
    fn test_comparison_type_mismatch_warns() {
        let a = check(r#"let r = 42 == "hello"; output r;"#);
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("comparing int64 with string")));
    }

    #[test]
    fn test_comparison_numeric_cross_ok() {
        let a = check("let r = 42 == 3.14;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .all(|d| !d.message.starts_with("comparing") || d.message.contains("variable")));
    }

    #[test]
    fn test_bool_literal_comparison_warns() {
        let a = check("let x = true;\nlet r = x == true;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("comparison with boolean literal")));
    }

    #[test]
    fn test_empty_for_body_moved_to_linter() {
        // W104 (empty block) now handled by linter W206
        let a = check("let items = [1, 2, 3];\nfor _x in items {}");
        let w = warnings(&a);
        assert!(!w.iter().any(|d| d.message.contains("empty loop body")),
            "empty block check should no longer be emitted by type checker");
    }

    #[test]
    fn test_empty_while_body_moved_to_linter() {
        // W104 (empty block) now handled by linter W206
        let a = check("let mut c = true;\nwhile c {}");
        let w = warnings(&a);
        assert!(!w.iter().any(|d| d.message.contains("empty loop body")),
            "empty block check should no longer be emitted by type checker");
    }

    #[test]
    fn test_infinite_while_moved_to_linter() {
        // W105 (while true) now handled by linter W204
        let a = check("while true { 1; }");
        let w = warnings(&a);
        assert!(!w
            .iter()
            .any(|d| d.message.contains("loop condition is always true")),
            "while-true check should no longer be emitted by type checker");
    }

    #[test]
    fn test_empty_range_warns() {
        let a = check("let r = range(5, 0);\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("range will produce empty array")));
    }

    #[test]
    fn test_double_negation_warns() {
        // With ++/-- operators, `--x` is now decrement; use `-(-x)` for double negation
        let a = check("let x = 5;\nlet r = -(-x);\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("double negation is redundant")));
    }

    #[test]
    fn test_double_not_warns() {
        let a = check("let x = true;\nlet r = !!x;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("double logical NOT is redundant")));
    }

    #[test]
    fn test_modulo_by_one_warns() {
        let a = check("let x = 10;\nlet r = x % 1;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("modulo by 1 always returns 0")));
    }

    #[test]
    fn test_multiply_by_zero_warns() {
        let a = check("let x = 10;\nlet r = x * 0;\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("multiplication by 0 always returns 0")));
    }

    #[test]
    fn test_self_comparison_not_in_type_checker() {
        // Self-comparison is handled by linter as W205, not type checker
        let a = check("let x = 5;\nlet r = x == x;\noutput r;");
        let w = warnings(&a);
        assert!(
            !w.iter().any(|d| d.message.contains("comparing") && d.message.contains("itself")),
            "Type checker should not emit self-comparison warning (linter handles it as W205)"
        );
    }


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

    // If/else branch type mismatch warning

    #[test]
    fn test_if_else_branch_mismatch_warns() {
        let a = check(r#"let c = true; let r = if c { 42 } else { "text" }; output r;"#);
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("if/else branches have mismatched types")));
    }

    #[test]
    fn test_if_else_same_type_no_warning() {
        let a = check("let c = true; let r = if c { 1 } else { 2 }; output r;");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("mismatched types")));
    }

    // Gap #4: break/continue/return outside valid context

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
            .any(|d| d.message.contains("array destructuring requires array")));
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
            .any(|d| d.message.contains("map destructuring requires map")));
    }

    #[test]
    fn test_destructure_rest_is_array() {
        let a = check("let [first, ...rest] = [1, 2, 3];\noutput first;\noutput rest;");
        assert!(errors(&a).is_empty());
        assert_eq!(a.variable_types.get("rest"), Some(&ChannelType::Array));
    }


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
            .any(|d| d.message.contains("cannot assign to immutable")));
    }

    #[test]
    fn test_compound_assign_undefined_error() {
        let a = check("y += 5;");
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("undefined variable")));
    }

    // Try/catch

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

    #[test]
    fn test_const_propagation_int() {
        // Const should propagate its known type to subsequent uses.
        let a = check("const MAX = 100;\nlet x = MAX + 1;\noutput x;");
        // MAX is Int64, so MAX + 1 should be Int64
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_const_propagation_string() {
        let a = check(r#"const GREETING = "hello"; let x = GREETING; output x;"#);
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::String));
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_const_propagation_bool() {
        let a = check("const FLAG = true;\nlet x = FLAG;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Bool));
        assert!(errors(&a).is_empty());
    }

    #[test]
    fn test_const_propagation_float() {
        let a = check("const PI = 3.14;\nlet x = PI * 2.0;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
        assert!(errors(&a).is_empty());
    }


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
            .any(|d| d.message.contains("unknown type 'nonexistent'")));
    }


    #[test]
    fn test_module_scoping() {
        let a = check(
            r#"mod math {
    fn _double(x: int64) -> int64 { x * 2 }
}"#,
        );
        assert!(errors(&a).is_empty());
    }


    #[test]
    fn test_use_known_std_module() {
        let a = check("use std::math::sqrt;");
        // Known module — no warnings
        let w = warnings(&a);
        assert!(w
            .iter()
            .all(|d| !d.message.contains("unknown standard library module")));
    }

    #[test]
    fn test_use_unknown_std_module_errors() {
        let a = check("use std::nonexistent::thing;");
        let e = errors(&a);
        assert!(e
            .iter()
            .any(|d| d.message.contains("unknown standard library module")));
    }


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
        assert!(w.iter().any(|d| d.message.contains("unknown method")));
    }


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
        assert!(e.iter().any(|d| d.message.contains("division by zero")));
    }


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
        assert!(w.iter().any(|d| d.message.contains("non-exhaustive match")));
    }

    #[test]
    fn test_match_empty_warns() {
        let a = check("let x = 1;\nlet r = match x {};\noutput r;");
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("empty match")));
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

    #[test]
    fn test_or_pattern_inconsistent_vars_warns() {
        let a = check(
            r#"let x = 5;
let r = match x {
    a | 2 => 0,
    _ => 1,
};
output r;"#,
        );
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("or-pattern alternatives bind different variables")),
            "expected warning for inconsistent or-pattern vars, got {:?}",
            w.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_or_pattern_consistent_vars_no_warning() {
        let a = check(
            r#"let x = 5;
let r = match x {
    1 | 2 | 3 => 0,
    _ => 1,
};
output r;"#,
        );
        let w = warnings(&a);
        assert!(
            !w.iter().any(|d| d.message.contains("or-pattern alternatives")),
            "no warning expected for consistent or-pattern, got {:?}",
            w.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }


    #[test]
    fn test_string_interp_returns_string() {
        let a = check(r#"let name = "world"; let r = f"hello {name}"; output r;"#);
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::String));
    }

    #[test]
    fn test_string_interp_checks_inner_expr() {
        let a = check(r#"let r = f"value: {undefined_var}"; output r;"#);
        let e = errors(&a);
        assert!(e.iter().any(|d| d.message.contains("undefined variable")));
    }


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
            .any(|d| d.message.contains("optional chaining requires map")));
    }


    #[test]
    fn test_spread_non_array_warns() {
        let a = check("let x = 42;\nlet r = [...x];\noutput r;");
        let w = warnings(&a);
        assert!(w
            .iter()
            .any(|d| d.message.contains("spread requires array")));
    }


    #[test]
    fn test_loop_empty_body_moved_to_linter() {
        // W104 (empty block) now handled by linter W206
        let a = check("let _r = loop {};");
        let w = warnings(&a);
        assert!(!w.iter().any(|d| d.message.contains("empty loop body")),
            "empty block check should no longer be emitted by type checker");
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

    // Try/catch expression

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


    #[test]
    fn test_throw_type_checks_expr() {
        let a = check(r#"throw "error message";"#);
        assert!(errors(&a).is_empty());
    }


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
        assert!(d.message.contains("undefined variable"));
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
            .find(|d| d.message.contains("undefined"))
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
            .find(|d| d.message.contains("unused variable"))
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
            .find(|d| d.message.contains("unused function"))
            .unwrap();
        assert_eq!(d.code.as_deref(), Some("W103"));
    }

    #[test]
    fn test_diagnostic_code_for_unused_import() {
        let a = check("import \"capture\";");
        let warns = warnings(&a);
        let d = warns
            .iter()
            .find(|d| d.message.contains("unused import"))
            .unwrap();
        assert_eq!(d.code.as_deref(), Some("W101"));
    }

    #[test]
    fn test_w102_shadowing_moved_to_linter() {
        // W102 is now handled by the linter as W209.
        let a = check("let x = 1;\nlet x = 2;\noutput x;");
        let warns = warnings(&a);
        let shadow = warns.iter().find(|d| d.code.as_deref() == Some("W102"));
        assert!(shadow.is_none(), "W102 should no longer be emitted by type checker");
    }

    #[test]
    fn test_diagnostic_no_suggestion_for_distant_name() {
        let a = check("let r = xyz + 1; output r;");
        let errs = errors(&a);
        let d = errs
            .iter()
            .find(|d| d.message.contains("undefined"))
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
            errs.iter().any(|d| d.message.contains("undefined")),
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
            errs.iter().any(|d| d.message.contains("undefined")),
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
            .filter(|d| d.message.contains("undefined"))
            .collect();
        assert!(
            undef_errs.is_empty(),
            "Test body should access outer scope variables: {:?}",
            undef_errs
        );
    }

    #[test]
    fn test_w208_duplicate_import() {
        let a = check("use std::math;\nuse std::math;\noutput 1;");
        let warns = warnings(&a);
        let dup = warns.iter().find(|d| d.code.as_deref() == Some("W208"));
        assert!(dup.is_some(), "Expected W208 for duplicate import");
    }

    #[test]
    fn test_w208_no_false_positive() {
        let a = check("use std::math;\nuse std::json;\noutput 1;");
        let warns = warnings(&a);
        let dup = warns.iter().find(|d| d.code.as_deref() == Some("W208"));
        assert!(dup.is_none(), "No W208 for different imports");
    }

    #[test]
    fn test_w102_no_longer_emitted() {
        // W102 is now handled by the linter as W209.
        let a = check("let _x = 1;\nlet _x = 2;\noutput _x;");
        let warns = warnings(&a);
        let shadow = warns.iter().find(|d| d.code.as_deref() == Some("W102"));
        assert!(shadow.is_none(), "W102 should no longer be emitted by type checker");
    }

    #[test]
    fn test_no_e201_for_array_insert_remove() {
        // These methods exist in the interpreter; should not produce E201 warnings
        let a = check(r#"
            let arr = [1, 2, 3];
            arr.insert(1, 99);
            arr.remove(0);
            arr.shift();
            arr.filter_nulls();
            output arr;
        "#);
        let warns = warnings(&a);
        let unknown = warns.iter().filter(|d| d.message.contains("unknown method")).collect::<Vec<_>>();
        assert!(unknown.is_empty(), "Expected no E202 for known array methods, got: {:?}", unknown);
    }

    // Enum variant arity validation

    #[test]
    fn test_enum_construct_arity_ok() {
        let a = check(r#"
            enum Result { Ok(value), Err(msg) }
            let x = Result::Ok(42);
            output x;
        "#);
        let errs = errors(&a);
        assert!(errs.is_empty(), "Expected no errors for correct arity, got: {:?}", errs);
    }

    #[test]
    fn test_enum_construct_arity_too_many() {
        let a = check(r#"
            enum Result { Ok(value), Err(msg) }
            let x = Result::Ok(1, 2);
            output x;
        "#);
        let errs = errors(&a);
        assert!(
            errs.iter().any(|d| d.message.contains("expects 1 arguments, got 2")),
            "Expected arity error, got: {:?}", errs
        );
    }

    #[test]
    fn test_enum_construct_arity_too_few() {
        let a = check(r#"
            enum Result { Ok(value), Err(msg) }
            let x = Result::Ok();
            output x;
        "#);
        let errs = errors(&a);
        assert!(
            errs.iter().any(|d| d.message.contains("expects 1 arguments, got 0")),
            "Expected arity error, got: {:?}", errs
        );
    }

    #[test]
    fn test_enum_construct_unit_variant_no_args() {
        let a = check(r#"
            enum Color { Red, Green, Blue }
            let x = Color::Red();
            output x;
        "#);
        // Unit variant has 0 fields, calling with 0 args is OK
        let errs = errors(&a);
        assert!(errs.is_empty(), "Expected no errors for unit variant with no args, got: {:?}", errs);
    }

    #[test]
    fn test_enum_pattern_unknown_variant() {
        let a = check(r#"
            enum Color { Red, Green, Blue }
            let c = Color::Red();
            let name = match c {
                Color::Red => "red",
                Color::Yellow => "yellow",
                _ => "other",
            };
            output name;
        "#);
        let errs = errors(&a);
        assert!(
            errs.iter().any(|d| d.message.contains("has no variant 'Yellow'")),
            "Expected unknown variant error, got: {:?}", errs
        );
    }

    #[test]
    fn test_enum_pattern_arity_mismatch() {
        let a = check(r#"
            enum Result { Ok(value), Err(msg) }
            let r = Result::Ok(42);
            let v = match r {
                Result::Ok(a, b) => a,
                _ => 0,
            };
            output v;
        "#);
        let errs = errors(&a);
        assert!(
            errs.iter().any(|d| d.message.contains("expects 1 bindings, got 2")),
            "Expected binding count error, got: {:?}", errs
        );
    }


    #[test]
    fn test_struct_construct_valid() {
        let a = check(r#"
            struct Point { x: float64, y: float64 }
            let p = Point { x: 1.0, y: 2.0 };
            output p;
        "#);
        let errs = errors(&a);
        assert!(errs.is_empty(), "Expected no errors for valid struct, got: {:?}", errs);
    }

    #[test]
    fn test_struct_construct_unknown_field() {
        let a = check(r#"
            struct Point { x: float64, y: float64 }
            let p = Point { x: 1.0, z: 3.0 };
            output p;
        "#);
        let errs = errors(&a);
        assert!(
            errs.iter().any(|d| d.message.contains("has no field 'z'")),
            "Expected unknown field error, got: {:?}", errs
        );
    }

    #[test]
    fn test_struct_construct_missing_field() {
        let a = check(r#"
            struct Point { x: float64, y: float64 }
            let p = Point { x: 1.0 };
            output p;
        "#);
        let warns = warnings(&a);
        assert!(
            warns.iter().any(|d| d.message.contains("missing field 'y'")),
            "Expected missing field warning, got: {:?}", warns
        );
    }

    #[test]
    fn test_struct_field_count_mismatch() {
        let a = check(r#"
            struct Point { x: float64, y: float64 }
            let p = Point { x: 1.0 };
            output p;
        "#);
        let warns = warnings(&a);
        assert!(
            warns.iter().any(|d| d.message.contains("expects 2 fields, got 1")),
            "Expected field count mismatch warning, got: {:?}", warns
        );
    }

    #[test]
    fn test_struct_field_count_exact_no_warning() {
        let a = check(r#"
            struct Point { x: float64, y: float64 }
            let p = Point { x: 1.0, y: 2.0 };
            output p;
        "#);
        let warns = warnings(&a);
        assert!(
            !warns.iter().any(|d| d.message.contains("expects") && d.message.contains("fields")),
            "No field count warning expected, got: {:?}", warns
        );
    }

    // Module-qualified function arity

    #[test]
    fn test_module_qualified_call_arity_check() {
        let a = check("use std::math::*;\nlet r = Math::sqrt(1, 2);\noutput r;");
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("expects") && d.message.contains("arguments")),
            "Expected arity warning for module-qualified call, got: {:?}", w
        );
    }

    // Audit: Binary operation type inference

    #[test]
    fn test_binop_int64_plus_int64() {
        let a = check("let x = 1 + 2;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_binop_int64_plus_float64() {
        let a = check("let x = 1 + 2.5;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_binop_float64_plus_float64() {
        let a = check("let x = 1.0 + 2.5;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_binop_string_concat_left() {
        let a = check("let x = \"hello\" + 42;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::String));
    }

    #[test]
    fn test_binop_string_concat_right() {
        let a = check("let x = 42 + \"hello\";\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::String));
    }

    #[test]
    fn test_binop_comparison_returns_bool() {
        let a = check("let x = 1 > 2;\nlet y = 1 == 1;\nlet z = 1 != 2;\noutput x;\noutput y;\noutput z;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Bool));
        assert_eq!(a.variable_types.get("y"), Some(&ChannelType::Bool));
        assert_eq!(a.variable_types.get("z"), Some(&ChannelType::Bool));
    }

    #[test]
    fn test_binop_div_int_returns_int64() {
        let a = check("let x = 10 / 3;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_binop_div_float_returns_float64() {
        let a = check("let x = 10.0 / 3;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_binop_mod_preserves_int64() {
        let a = check("let x = 10 % 3;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_binop_mod_with_float_returns_float64() {
        let a = check("let x = 10.5 % 3.0;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_binop_sub_preserves_int64() {
        let a = check("let x = 10 - 3;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_binop_mul_preserves_int64() {
        let a = check("let x = 10 * 3;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_binop_and_returns_any() {
        // &&/|| use truthiness and return either operand, so type is polymorphic (Null = any)
        let a = check("let x = true && false;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Null));
    }

    #[test]
    fn test_binop_or_returns_any() {
        let a = check("let x = true || false;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Null));
    }

    // Audit: Unary operation type inference

    #[test]
    fn test_unary_neg_int64() {
        let a = check("let x = -42;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_unary_neg_float64() {
        let a = check("let x = -3.14;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_unary_not_bool() {
        let a = check("let x = !true;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Bool));
    }

    #[test]
    fn test_unary_not_int_warns_returns_bool() {
        let a = check("let n = 42;\nlet x = !n;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Bool));
        // Should warn about non-bool operand
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("logical NOT expects bool")),
            "Expected warning about NOT on non-bool, got: {:?}", w
        );
    }

    #[test]
    fn test_unary_neg_string_warns() {
        let a = check(r#"let s = "hello"; let x = -s; output x;"#);
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("negation expects numeric")),
            "Expected warning about negation on string, got: {:?}", w
        );
    }

    // Audit: Assignment type compatibility

    #[test]
    fn test_assignment_type_annotated_compatible() {
        // Int64 assigned Int64 - no warning
        let a = check("let mut x: int64 = 1;\nx = 42;\noutput x;");
        let w: Vec<_> = warnings(&a).into_iter()
            .filter(|d| d.message.contains("assigning"))
            .collect();
        assert!(w.is_empty(), "Expected no assignment warning, got: {:?}", w);
    }

    #[test]
    fn test_assignment_type_annotated_incompatible() {
        // Int64 assigned String - should warn
        let a = check(r#"let mut x: int64 = 1; x = "hello"; output x;"#);
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("assigning") && d.message.contains("string") && d.message.contains("int64")),
            "Expected type incompatibility warning, got: {:?}", w
        );
    }

    #[test]
    fn test_assignment_type_annotated_numeric_widening() {
        // Float64 assigned Int64 - compatible (Int64 is compatible with Float64)
        let a = check("let mut x: float64 = 1.0;\nx = 42;\noutput x;");
        let w: Vec<_> = warnings(&a).into_iter()
            .filter(|d| d.message.contains("assigning"))
            .collect();
        assert!(w.is_empty(), "Expected no assignment warning for numeric widening, got: {:?}", w);
    }

    #[test]
    fn test_assignment_no_annotation_no_warning() {
        // No type annotation - no incompatibility warning even for different types
        let a = check(r#"let mut x = 1; x = "hello"; output x;"#);
        let w: Vec<_> = warnings(&a).into_iter()
            .filter(|d| d.message.contains("assigning"))
            .collect();
        assert!(w.is_empty(), "Expected no assignment warning without annotation, got: {:?}", w);
    }

    #[test]
    fn test_compound_assignment_type_annotated_incompatible() {
        // String += with Int64 variable
        let a = check(r#"let mut x: int64 = 1; x += "hello"; output x;"#);
        let w = warnings(&a);
        // The compound assignment x += "hello" does infer_binop(Add, Int64, String) = String
        // String is not compatible with Int64, so should warn
        assert!(
            w.iter().any(|d| d.message.contains("compound assignment") && d.message.contains("string") && d.message.contains("int64")),
            "Expected compound assignment type incompatibility warning, got: {:?}", w
        );
    }

    // Audit: Function call type inference (multi-branch returns)

    #[test]
    fn test_fn_returns_declared_type_at_callsite() {
        // With a declared return type, call site gets that type
        let a = check(r#"
            fn foo(x: bool) -> float64 {
                if x { 1 } else { 2.5 }
            }
            let r = foo(true);
            output r;
        "#);
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_fn_without_return_type_infers_at_callsite() {
        // Return type inference (#66): inferred from body tail expression
        let a = check(r#"
            fn foo(x: bool) {
                if x { 1 } else { 2.5 }
            }
            let r = foo(true);
            output r;
        "#);
        // With return type inference, the body type (Float64) is now used at call site
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_fn_body_unifies_branch_types() {
        // The body's IfElse unifies Int64 and Float64 to Float64 internally
        // No body/return mismatch error since return type is Float64, body is Float64
        let a = check(r#"
            fn foo(x: bool) -> float64 {
                if x { 1 } else { 2.5 }
            }
            output foo(true);
        "#);
        let errs = errors(&a);
        assert!(errs.is_empty(), "Expected no errors, got: {:?}", errs);
    }

    #[test]
    fn test_fn_body_mismatched_warns() {
        // Function body returns String or Int64 - IfElse warns about mismatch
        let a = check(r#"
            fn bar(x: bool) {
                if x { "hello" } else { 42 }
            }
            let r = bar(true);
            output r;
        "#);
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("mismatched types")),
            "Expected mismatched types warning in if/else, got: {:?}", w
        );
    }

    #[test]
    fn test_fn_declared_return_type_propagates() {
        let a = check(r#"
            fn add(a: int64, b: int64) -> int64 {
                a + b
            }
            let r = add(1, 2);
            output r;
        "#);
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
    }

    // Audit: Match expression type inference

    #[test]
    fn test_match_unifies_same_type() {
        let a = check(r#"
            let x = 1;
            let r = match x {
                1 => 10,
                2 => 20,
                _ => 30,
            };
            output r;
        "#);
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_match_unifies_numeric_promotion() {
        let a = check(r#"
            let x = 1;
            let r = match x {
                1 => 10,
                2 => 2.5,
                _ => 30,
            };
            output r;
        "#);
        // Int64 and Float64 should unify to Float64
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_match_mismatched_arms_returns_null() {
        let a = check(r#"
            let x = 1;
            let r = match x {
                1 => "hello",
                2 => 42,
                _ => true,
            };
            output r;
        "#);
        // String, Int64, Bool cannot unify -> Null
        assert_eq!(a.variable_types.get("r"), Some(&ChannelType::Null));
    }

    // Audit: Array element types

    #[test]
    fn test_array_literal_uniform_type() {
        let a = check("let x = [1, 2, 3];\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Array));
    }

    #[test]
    fn test_array_literal_mixed_types() {
        // Mixed types in array literal - should still be Array type (no element tracking)
        let a = check(r#"let x = [1, "hello", true]; output x;"#);
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Array));
    }

    #[test]
    fn test_array_literal_empty() {
        let a = check("let x = [];\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Array));
    }

    // Audit: Additional edge cases

    #[test]
    fn test_if_else_numeric_promotion() {
        let a = check("let x = if true { 1 } else { 2.5 };\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_try_catch_numeric_promotion() {
        let a = check(r#"
            let x = try { 42 } catch e { 3.14 };
            output x;
        "#);
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_null_coalesce_type() {
        let a = check("let x = null ?? 42;\noutput x;");
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_binop_arithmetic_on_bool_warns() {
        let a = check("let x = true + 1;\noutput x;");
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("arithmetic operator") && d.message.contains("bool")),
            "Expected arithmetic on bool warning, got: {:?}", w
        );
    }

    #[test]
    fn test_binop_comparison_cross_type_warns() {
        let a = check(r#"let x = 1 == "hello"; output x;"#);
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("comparing")),
            "Expected cross-type comparison warning, got: {:?}", w
        );
    }

    #[test]
    fn test_binop_comparison_cross_numeric_no_warning() {
        // Int64 == Float64 should not warn (cross-numeric is allowed)
        let a = check("let x = 1 == 1.0;\noutput x;");
        let w: Vec<_> = warnings(&a).into_iter()
            .filter(|d| d.message.contains("comparing"))
            .collect();
        assert!(w.is_empty(), "Expected no comparison warning for cross-numeric, got: {:?}", w);
    }


    #[test]
    fn test_type_alias_basic() {
        // type Score = int64; let x: Score = 42; should resolve without errors.
        let a = check("type Score = int64;\nlet x: Score = 42;\noutput x;");
        let e = errors(&a);
        assert!(e.is_empty(), "Expected no errors, got: {:?}", e);
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_type_alias_chained() {
        // type A = int64; type B = A; let x: B = 10;
        let a = check("type A = int64;\ntype B = A;\nlet x: B = 10;\noutput x;");
        let e = errors(&a);
        assert!(e.is_empty(), "Expected no errors for chained alias, got: {:?}", e);
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_type_alias_in_function_params() {
        // type Score = float64; fn add(a: Score, b: Score) -> Score { a }
        let a = check(r#"
            type Score = float64;
            fn add(a: Score, b: Score) -> Score { a }
            let x = add(1.0, 2.0);
            output x;
        "#);
        let e = errors(&a);
        assert!(e.is_empty(), "Expected no errors, got: {:?}", e);
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Float64));
    }

    #[test]
    fn test_type_alias_in_const() {
        let a = check("type Name = string;\nconst greeting: Name = \"hello\";\noutput greeting;");
        let e = errors(&a);
        assert!(e.is_empty(), "Expected no errors, got: {:?}", e);
        assert_eq!(a.variable_types.get("greeting"), Some(&ChannelType::String));
    }

    #[test]
    fn test_type_alias_in_let_mut() {
        let a = check("type Count = int64;\nlet mut c: Count = 0;\nc = 5;\noutput c;");
        let e = errors(&a);
        assert!(e.is_empty(), "Expected no errors, got: {:?}", e);
        assert_eq!(a.variable_types.get("c"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_type_alias_unknown_target() {
        // type Bad = nonexistent; should warn about unknown target type
        let a = check("type Bad = nonexistent;");
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("unknown type 'nonexistent'")),
            "Expected unknown type warning, got: {:?}", w
        );
    }

    #[test]
    fn test_type_alias_annotation_mismatch() {
        // type Score = int64; let x: Score = "hello"; should warn about mismatch
        let a = check(r#"type Score = int64; let x: Score = "hello"; output x;"#);
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("conflicts with inferred type")),
            "Expected type mismatch warning, got: {:?}", w
        );
    }

    #[test]
    fn test_type_alias_triple_chain() {
        // type A = int64; type B = A; type C = B;
        let a = check("type A = int64;\ntype B = A;\ntype C = B;\nlet x: C = 1;\noutput x;");
        let e = errors(&a);
        assert!(e.is_empty(), "Expected no errors for triple chain, got: {:?}", e);
        assert_eq!(a.variable_types.get("x"), Some(&ChannelType::Int64));
    }

    #[test]
    fn test_type_alias_in_lambda() {
        let a = check(r#"
            type Num = float64;
            let f = |x: Num| x;
            let _y = f(3.14);
        "#);
        let e = errors(&a);
        assert!(e.is_empty(), "Expected no errors, got: {:?}", e);
    }

    #[test]
    fn test_type_alias_inside_module() {
        // Type aliases inside a module body resolve for code within that module.
        let a = check(r#"
            mod math {
                type Scalar = float64;
                fn scale(x: Scalar) -> Scalar { x }
            }
        "#);
        let e = errors(&a);
        assert!(e.is_empty(), "Expected no errors for alias inside module, got: {:?}", e);
    }

    #[test]
    fn test_type_alias_various_types() {
        let a = check(r#"
            type Flag = bool;
            type Text = string;
            type Data = bytes;
            type Items = array;
            type Dict = map;
            let f: Flag = true;
            let t: Text = "hi";
            let b: Data = null;
            let i: Items = [1, 2];
            let d: Dict = null;
            output f;
            output t;
            output b;
            output i;
            output d;
        "#);
        let e = errors(&a);
        assert!(e.is_empty(), "Expected no errors, got: {:?}", e);
        assert_eq!(a.variable_types.get("f"), Some(&ChannelType::Bool));
        assert_eq!(a.variable_types.get("t"), Some(&ChannelType::String));
        assert_eq!(a.variable_types.get("i"), Some(&ChannelType::Array));
    }

    #[test]
    fn test_type_alias_function_return_type_check() {
        // Verify that function return type mismatch is caught when using alias
        let a = check(r#"
            type Score = int64;
            fn get_score() -> Score { "not a number" }
        "#);
        let e = errors(&a);
        assert!(
            e.iter().any(|d| d.message.contains("declares return type") && d.message.contains("body evaluates to")),
            "Expected return type mismatch error, got: {:?}", e
        );
    }

    #[test]
    fn test_module_scoped_enum_registered() {
        // Module-scoped enums should be registered with both qualified and unqualified names
        // so that arity checking works inside module functions.
        let a = check(r#"
            mod shapes {
                enum Shape {
                    Circle(radius),
                    Rect(w, h),
                }
                fn make_circle() {
                    Shape::Circle(10)
                }
                fn bad_circle() {
                    Shape::Circle(10, 20)
                }
            }
        "#);
        let e = errors(&a);
        // The bad_circle function constructs Circle with wrong arity (2 instead of 1)
        assert!(e.iter().any(|d| d.message.contains("expects 1 arguments, got 2")),
            "Expected arity error for wrong enum construction, got: {:?}", e);
    }

    #[test]
    fn test_module_scoped_enum_pattern_check() {
        // Module-scoped enum pattern matching should validate variant arity.
        let a = check(r#"
            mod colors {
                enum Color {
                    Red,
                    Green,
                    Blue,
                }
                fn is_red(c) {
                    match c {
                        Color::Red() => true,
                        Color::Green() => false,
                        Color::Blue() => false,
                    }
                }
            }
        "#);
        let e = errors(&a);
        assert!(e.is_empty(), "Expected no errors for correct pattern, got: {:?}", e);
    }

    #[test]
    fn test_module_scoped_struct_registered() {
        // Module-scoped structs should be registered so field validation works.
        let a = check(r#"
            mod geo {
                struct Point {
                    x: float64,
                    y: float64,
                }
                fn make_point() {
                    Point { x: 1.0, y: 2.0 }
                }
            }
        "#);
        let e = errors(&a);
        assert!(e.is_empty(), "Expected no errors for correct struct, got: {:?}", e);
    }

    // Item #329: Unused const warning

    #[test]
    fn test_unused_const_warns() {
        let a = check("const MAX = 100;");
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("unused constant 'MAX'")),
            "Expected 'unused constant' warning, got: {:?}", w
        );
    }

    #[test]
    fn test_unused_const_has_w100_code() {
        let a = check("const MAX = 100;");
        let w = warnings(&a);
        let diag = w.iter().find(|d| d.message.contains("unused constant"));
        assert!(diag.is_some(), "Expected unused constant warning");
        assert_eq!(diag.unwrap().code.as_deref(), Some("W100"));
    }

    #[test]
    fn test_used_const_no_warning() {
        let a = check("const MAX = 100;\noutput MAX;");
        let w = warnings(&a);
        assert!(
            w.iter().all(|d| !d.message.contains("unused constant")),
            "Used const should not trigger unused warning, got: {:?}", w
        );
    }

    #[test]
    fn test_underscore_const_no_unused_warning() {
        let a = check("const _INTERNAL = 42;");
        let w = warnings(&a);
        assert!(
            w.iter().all(|d| !d.message.contains("unused")),
            "Underscore-prefixed const should not trigger unused warning, got: {:?}", w
        );
    }

    #[test]
    fn test_unused_const_not_variable_message() {
        // Ensure the message says "unused constant", not "unused variable"
        let a = check("const PI = 3.14;");
        let w = warnings(&a);
        assert!(
            w.iter().all(|d| !d.message.contains("unused variable")),
            "Unused const should say 'unused constant', not 'unused variable', got: {:?}", w
        );
        assert!(
            w.iter().any(|d| d.message.contains("unused constant 'PI'")),
            "Expected 'unused constant' warning for PI, got: {:?}", w
        );
    }

    // Item #326: Duplicate parameter names

    /// Helper to build an AST with a function that has duplicate parameter names,
    /// bypassing the parser's own duplicate check.
    fn build_fn_with_params(fn_name: &str, param_names: &[&str]) -> Program {
        use crate::syntax::ast::*;
        let span = Span::new(1, 1, 1, 1);
        let params: Vec<FunctionParam> = param_names.iter().enumerate().map(|(i, name)| {
            FunctionParam {
                name: name.to_string(),
                type_annotation: None,
                default: None,
                rest: false,
                kwargs: false,
                span: Span::new(1, (5 + i * 3) as u32, 1, (7 + i * 3) as u32),
            }
        }).collect();
        let body = Block {
            statements: vec![
                Statement::new(
                    StatementKind::Output(Expression {
                        kind: ExpressionKind::Variable(param_names[0].to_string()),
                        span,
                    }),
                    span,
                ),
            ],
            tail_expr: None,
            tail_comments: Vec::new(),
            span,
        };
        Program {
            statements: vec![Statement::new(
                StatementKind::FunctionDef(FunctionDef {
                    name: fn_name.to_string(),
                    type_params: Vec::new(),
                    params,
                    return_type: None,
                    body,
                    span,
                    is_getter: false,
                    is_setter: false,
                    where_clauses: Vec::new(),
                    deprecated: false,
                }),
                span,
            )],
            span,
            trailing_comments: Vec::new(),
        }
    }

    #[test]
    fn test_duplicate_param_names_error() {
        let ast = build_fn_with_params("foo", &["x", "x"]);
        let a = check_types(&ast, &HashSet::new());
        let e = errors(&a);
        assert!(
            e.iter().any(|d| d.message.contains("duplicate parameter name 'x' in function 'foo'")),
            "Expected duplicate parameter name error, got: {:?}", e
        );
    }

    #[test]
    fn test_duplicate_param_names_has_e100_code() {
        let ast = build_fn_with_params("foo", &["x", "x"]);
        let a = check_types(&ast, &HashSet::new());
        let e = errors(&a);
        let diag = e.iter().find(|d| d.message.contains("duplicate parameter name"));
        assert!(diag.is_some(), "Expected duplicate parameter name error");
        assert_eq!(diag.unwrap().code.as_deref(), Some("E100"));
    }

    #[test]
    fn test_no_duplicate_param_names_ok() {
        let a = check("fn foo(x: int64, y: string) { output x; output y; }");
        let e = errors(&a);
        assert!(
            e.iter().all(|d| !d.message.contains("duplicate parameter")),
            "No duplicate params should produce no error, got: {:?}", e
        );
    }

    #[test]
    fn test_duplicate_param_names_multiple() {
        let ast = build_fn_with_params("bar", &["a", "b", "a", "b"]);
        let a = check_types(&ast, &HashSet::new());
        let e = errors(&a);
        let dup_errors: Vec<_> = e.iter().filter(|d| d.message.contains("duplicate parameter name")).collect();
        assert_eq!(dup_errors.len(), 2, "Expected 2 duplicate param errors, got: {:?}", dup_errors);
    }

    // Item #330: Struct field type annotations validated

    #[test]
    fn test_struct_field_known_type_ok() {
        let a = check(r#"
            struct Point {
                x: float64,
                y: float64,
            }
        "#);
        let w = warnings(&a);
        assert!(
            w.iter().all(|d| !d.message.contains("unknown type")),
            "Known field types should not produce warnings, got: {:?}", w
        );
    }

    #[test]
    fn test_struct_field_unknown_type_warns() {
        let a = check(r#"
            struct Foo {
                x: int64,
                y: widget,
            }
        "#);
        let w = warnings(&a);
        assert!(
            w.iter().any(|d| d.message.contains("unknown type 'widget' in struct field 'Foo.y'")),
            "Expected unknown type warning for 'widget', got: {:?}", w
        );
    }

    #[test]
    fn test_struct_field_unknown_type_has_e100_code() {
        let a = check(r#"
            struct Foo {
                val: badtype,
            }
        "#);
        let w = warnings(&a);
        let diag = w.iter().find(|d| d.message.contains("unknown type"));
        assert!(diag.is_some(), "Expected unknown type warning");
        assert_eq!(diag.unwrap().code.as_deref(), Some("E100"));
    }

    #[test]
    fn test_struct_field_struct_type_ok() {
        // A struct field that references another known struct should not warn
        let a = check(r#"
            struct Inner {
                val: int64,
            }
            struct Outer {
                child: Inner,
            }
        "#);
        let w = warnings(&a);
        assert!(
            w.iter().all(|d| !d.message.contains("unknown type")),
            "Struct type reference should not produce unknown type warning, got: {:?}", w
        );
    }

    #[test]
    fn test_struct_field_enum_type_ok() {
        // A struct field that references a known enum should not warn
        let a = check(r#"
            enum Color { Red, Green, Blue }
            struct Pixel {
                color: Color,
                x: int64,
            }
        "#);
        let w = warnings(&a);
        assert!(
            w.iter().all(|d| !d.message.contains("unknown type")),
            "Enum type reference should not produce unknown type warning, got: {:?}", w
        );
    }

    #[test]
    fn test_struct_field_type_alias_ok() {
        // A struct field that references a type alias should not warn
        let a = check(r#"
            type Number = int64;
            struct Data {
                count: Number,
            }
        "#);
        let w = warnings(&a);
        assert!(
            w.iter().all(|d| !d.message.contains("unknown type")),
            "Type alias reference should not produce unknown type warning, got: {:?}", w
        );
    }

    #[test]
    fn test_struct_field_no_annotation_ok() {
        // A struct field without a type annotation should not produce any warning
        let a = check(r#"
            struct Flexible {
                x,
                y,
            }
        "#);
        let w = warnings(&a);
        assert!(
            w.iter().all(|d| !d.message.contains("unknown type")),
            "Fields without annotation should not produce unknown type warning, got: {:?}", w
        );
    }

    #[test]
    fn test_duplicate_struct_field_caught_by_parser() {
        // The parser catches duplicate struct fields before the type checker sees them.
        // The type checker has a defensive check for programmatic AST construction.
        use crate::syntax::parser::parse_v2;
        let result = parse_v2("struct P { x: int64, x: int64 }");
        assert!(result.is_err(), "Parser should reject duplicate struct fields");
    }

    #[test]
    fn test_no_duplicate_struct_field_ok() {
        let a = check("struct P { x: int64, y: int64 }");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("duplicate field")),
            "Should not warn about duplicate fields, got: {:?}", w);
    }

    #[test]
    fn test_duplicate_enum_variant_caught_by_parser() {
        // The parser catches duplicate enum variants before the type checker sees them.
        use crate::syntax::parser::parse_v2;
        let result = parse_v2("enum E { A, A }");
        assert!(result.is_err(), "Parser should reject duplicate enum variants");
    }

    #[test]
    fn test_no_duplicate_enum_variant_ok() {
        let a = check("enum E { A, B, C }");
        let w = warnings(&a);
        assert!(w.iter().all(|d| !d.message.contains("duplicate variant")),
            "Should not warn about duplicate variants, got: {:?}", w);
    }

    #[test]
    fn test_source_file_none_by_default() {
        let a = check("let x = 1;");
        for d in &a.diagnostics {
            assert!(d.source_file.is_none(), "source_file should be None by default");
        }
    }

    #[test]
    fn test_check_types_with_source_stamps_file() {
        let program = parse_v2("let x = 1;").unwrap();
        let imports = std::collections::HashSet::new();
        let result = check_types_with_source(&program, &imports, "main.magi");
        for d in &result.diagnostics {
            assert_eq!(d.source_file.as_deref(), Some("main.magi"),
                "All diagnostics should be attributed to the source file");
        }
    }


    #[test]
    fn test_impl_trait_missing_method() {
        let a = check(r#"
            trait Greet {
                fn hello(self);
            }
            struct Person { name: string }
            impl Greet for Person {
            }
        "#);
        let errs = errors(&a);
        assert!(!errs.is_empty(), "should report missing method");
        assert!(errs.iter().any(|e| e.message.contains("requires method 'hello'")),
            "error should mention missing method 'hello'");
    }

    #[test]
    fn test_impl_trait_wrong_arity() {
        let a = check(r#"
            trait Greet {
                fn hello(self, name: string);
            }
            struct Person { name: string }
            impl Greet for Person {
                fn hello(self) {
                }
            }
        "#);
        let errs = errors(&a);
        assert!(!errs.is_empty(), "should report arity mismatch");
        assert!(errs.iter().any(|e| e.message.contains("parameter(s)")),
            "error should mention parameter count mismatch");
    }

    #[test]
    fn test_impl_trait_conforming() {
        let a = check(r#"
            trait Greet {
                fn hello(self);
                fn goodbye(self, reason: string);
            }
            struct Person { name: string }
            impl Greet for Person {
                fn hello(self) {
                    let _x = 1;
                }
                fn goodbye(self, reason: string) {
                    let _y = reason;
                }
            }
        "#);
        let errs = errors(&a);
        assert!(errs.is_empty(), "conforming impl should have no errors, got: {:?}",
            errs.iter().map(|e| &e.message).collect::<Vec<_>>());
    }

    #[test]
    fn test_impl_trait_return_type_mismatch() {
        let a = check(r#"
            trait Converter {
                fn convert(self) -> string;
            }
            struct Num { val: int64 }
            impl Converter for Num {
                fn convert(self) -> int64 {
                    let _x = 1;
                }
            }
        "#);
        let errs = errors(&a);
        assert!(!errs.is_empty(), "should report return type mismatch");
        assert!(errs.iter().any(|e| e.message.contains("returns 'int64'") && e.message.contains("requires 'string'")),
            "error should mention return type mismatch, got: {:?}",
            errs.iter().map(|e| &e.message).collect::<Vec<_>>());
    }

    #[test]
    fn test_impl_trait_missing_return_type_annotation() {
        let a = check(r#"
            trait Converter {
                fn convert(self) -> string;
            }
            struct Num { val: int64 }
            impl Converter for Num {
                fn convert(self) {
                    let _x = 1;
                }
            }
        "#);
        let w = warnings(&a);
        assert!(!w.is_empty(), "should warn about missing return type annotation");
        assert!(w.iter().any(|d| d.message.contains("missing return type annotation")),
            "warning should mention missing return type, got: {:?}",
            w.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[test]
    fn test_impl_trait_matching_return_type() {
        let a = check(r#"
            trait Converter {
                fn convert(self) -> string;
            }
            struct Num { val: int64 }
            impl Converter for Num {
                fn convert(self) -> string {
                    let _x = "hello";
                }
            }
        "#);
        let errs = errors(&a);
        // Filter out errors unrelated to return type.
        let return_errs: Vec<_> = errs.iter().filter(|e| e.message.contains("return")).collect();
        assert!(return_errs.is_empty(), "matching return type should produce no return-type errors, got: {:?}",
            return_errs.iter().map(|e| &e.message).collect::<Vec<_>>());
    }

    // DoWhileLoop and CStyleFor condition type checking

    #[test]
    fn test_do_while_condition_warns_on_non_bool() {
        let a = check(r#"
            let mut _x = 0;
            do {
                _x = _x + 1;
            } while "true";
        "#);
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("do-while condition should be bool")),
            "should warn about non-bool do-while condition, got: {:?}",
            w.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[test]
    fn test_do_while_condition_no_warning_on_bool() {
        let a = check(r#"
            let mut _x = 0;
            do {
                _x = _x + 1;
            } while _x < 10;
        "#);
        let w = warnings(&a);
        let cond_warns: Vec<_> = w.iter().filter(|d| d.message.contains("do-while condition")).collect();
        assert!(cond_warns.is_empty(), "bool condition should produce no do-while warning, got: {:?}",
            cond_warns.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[test]
    fn test_c_style_for_condition_warns_on_non_bool() {
        let a = check(r#"
            for (let mut _i = 0; "yes"; _i = _i + 1) {
                let _x = 1;
            }
        "#);
        let w = warnings(&a);
        assert!(w.iter().any(|d| d.message.contains("for condition should be bool")),
            "should warn about non-bool for condition, got: {:?}",
            w.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[test]
    fn test_c_style_for_scopes_variables() {
        // Variables declared in the init clause of a C-style for loop
        // should not leak into the outer scope.
        let a = check(r#"
            for (let mut _i = 0; _i < 10; _i = _i + 1) {
                let _x = _i;
            }
            let _y = _i;
        "#);
        let errs = errors(&a);
        assert!(errs.iter().any(|e| e.message.contains("undefined variable '_i'")),
            "C-style for init variable should be scoped, got: {:?}",
            errs.iter().map(|e| &e.message).collect::<Vec<_>>());
    }
}
