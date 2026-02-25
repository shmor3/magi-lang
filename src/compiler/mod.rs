//! MAGI compiler — compiles MAGI AST to WebAssembly.
//!
//! Architecture:
//! 1. AST → IR (stack-based intermediate representation)
//! 2. IR → WASM binary (via wasm-encoder)

mod compile;
mod ir;
mod wasm;

pub use compile::Compiler;
pub use ir::*;
pub use wasm::WasmCodegen;

use crate::syntax::ast::Program;

/// Compile a MAGI program to a WASM binary.
pub fn compile_to_wasm(program: &Program) -> Result<Vec<u8>, CompileError> {
    let mut compiler = Compiler::new();
    let module = compiler.compile(program)?;
    let codegen = WasmCodegen::new();
    codegen.emit(&module)
}

/// Errors that can occur during compilation.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CompileError {
    #[error("compile error at {line}:{col}: {message}")]
    Error {
        line: u32,
        col: u32,
        message: String,
    },

    #[error("unsupported feature: {0}")]
    Unsupported(String),

    #[error("internal compiler error: {0}")]
    Internal(String),
}

impl CompileError {
    pub fn at(line: u32, col: u32, msg: impl Into<String>) -> Self {
        Self::Error {
            line,
            col,
            message: msg.into(),
        }
    }

    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::Unsupported(msg.into())
    }
}
