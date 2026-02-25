//! MAGI language syntax — parser, AST, lexer, interpreter, type checker, and errors.

pub mod ast;
pub mod errors;
pub mod interpreter;
pub mod lexer;
pub mod parser;
pub mod type_checker;

use std::fmt;

/// A syntax error with line/column location.
#[derive(Debug, Clone)]
pub struct SyntaxError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for SyntaxError {}
