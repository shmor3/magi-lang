//! Evaluation traits and error types for the MAGI language.

use crate::types::{DataType, OperationType};
use std::collections::HashMap;

/// Error type for operation evaluation.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("Missing input: {0}")]
    MissingInput(String),

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Type conversion error: {0}")]
    TypeConversion(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Type error: expected {expected} for '{context}', got {actual}")]
    TypeError {
        expected: String,
        actual: String,
        context: String,
    },

    #[error("Index out of bounds: index {index}, length {length}")]
    IndexOutOfBounds { index: i64, length: usize },

    #[error("Arithmetic overflow: {0}")]
    Overflow(String),
}

/// Trait for evaluating MAGI operations.
///
/// The interpreter delegates operation execution through this trait,
/// allowing different backends (full evaluator in magi-api, or stubs for testing).
pub trait OperationEvaluator {
    fn eval_operation(
        &self,
        op: OperationType,
        inputs: &HashMap<String, DataType>,
        config: &HashMap<String, DataType>,
    ) -> Result<DataType, EvalError>;
}

/// Severity levels for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}
