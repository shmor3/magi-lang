//! # MAGI Programming Language
//!
//! MAGI is a hybrid compiled/interpreted programming language with Rust-inspired
//! syntax, dynamic typing, and first-class support for both tree-walking
//! interpretation and WASM compilation.
//!
//! ## Architecture
//!
//! ```text
//! Source (.magi)
//!     → Lexer (syntax::lexer)
//!     → Parser (syntax::parser)
//!     → AST (syntax::ast)
//!     → Optimizer (optimizer)
//!     → Type Checker (syntax::type_checker)
//!     → Linter (linter)
//!     → Interpreter (syntax::interpreter) OR Compiler (compiler) → WASM
//!     → Formatter (formatter)
//!     → LSP Server (lsp)
//! ```
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use magi_lang::syntax::parser::parse_v2;
//! use magi_lang::syntax::interpreter::Interpreter;
//! use magi_lang::eval::StubEvaluator;
//!
//! let program = parse_v2("output 1 + 2;").unwrap();
//! let evaluator = StubEvaluator;
//! let mut interp = Interpreter::new(&evaluator);
//! interp.execute(&program).unwrap();
//! ```

/// WASM and IR compiler for MAGI programs.
pub mod compiler;
/// Rich diagnostic rendering with ariadne.
pub mod diagnostics;
/// Operation evaluation traits and error types.
pub mod eval;
/// AST pretty-printer / code formatter.
pub mod formatter;
/// Lint rules for code quality warnings.
pub mod linter;
/// Language Server Protocol implementation.
pub mod lsp;
/// Operation type metadata (ports, types, output types).
pub mod ops;
/// Constant folding and dead code elimination.
pub mod optimizer;
/// Lexer, parser, AST, type checker, and interpreter.
pub mod syntax;
/// Opt-in local telemetry for performance stats.
pub mod telemetry;
/// Data types, channel types, and operation type enums.
pub mod types;
/// Own implementations replacing external crates.
pub mod util;
/// Semantic versioning and feature tracking.
pub mod version;
