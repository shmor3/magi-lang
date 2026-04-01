//! MAGI compiler — compiles MAGI AST to native code (LLVM) and WebAssembly.
//!
//! Architecture:
//! 1. AST → IR (stack-based intermediate representation)
//! 2. IR → LLVM IR → Machine Code (native compilation)
//! 3. IR → WASM binary (via own encoder)

mod compile;
mod ir;
mod wasm;
pub mod llvm;
pub mod bytecode; // Kept for runtime/vm.rs ClassFile tests
pub mod wasm_binary;
pub mod wasm_runtime;
pub mod webgpu;

pub use compile::{Compiler, SourceMapping};
pub use ir::*;
pub use wasm::WasmCodegen;

use crate::syntax::ast::Program;

/// Compile a MAGI program to a WASM binary.
pub fn compile_to_wasm(program: &Program) -> Result<Vec<u8>, CompileError> {
    let mut compiler = Compiler::new();
    let module = compiler.compile(program)?;
    let mut codegen = WasmCodegen::new();
    codegen.emit(&module)
}

/// Errors that can occur during compilation.
#[derive(Debug, Clone)]
pub enum CompileError {
    Error {
        line: u32,
        col: u32,
        message: String,
    },
    Unsupported(String),
    Internal(String),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Error { line, col, message } => {
                write!(f, "compile error at {}:{}: {}", line, col, message)
            }
            CompileError::Unsupported(msg) => write!(f, "unsupported feature: {}", msg),
            CompileError::Internal(msg) => write!(f, "internal compiler error: {}", msg),
        }
    }
}

impl std::error::Error for CompileError {}

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
