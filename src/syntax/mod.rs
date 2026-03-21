//! MAGI language syntax — parser, AST, lexer, interpreter, type checker, and errors.

pub mod ast;
pub mod errors;
pub mod interpreter;
pub mod lexer;
pub mod parser;
pub mod type_ann;
pub mod type_checker;

use std::fmt;

/// A syntax error with line/column location and optional stable error code.
#[derive(Debug, Clone)]
pub struct SyntaxError {
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub code: Option<String>,
}

impl SyntaxError {
    /// Create a new SyntaxError without an error code.
    pub fn new(line: usize, column: usize, message: String) -> Self {
        Self { line, column, message, code: None }
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(code) = &self.code {
            write!(f, "[{}] line {}:{}: {}", code, self.line, self.column, self.message)
        } else {
            write!(f, "line {}:{}: {}", self.line, self.column, self.message)
        }
    }
}

impl std::error::Error for SyntaxError {}
