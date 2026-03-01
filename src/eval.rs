//! Evaluation traits and error types for the MAGI language.

use crate::types::{DataType, OperationType};
use std::collections::HashMap;

/// Error type for operation evaluation.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("Division by zero")]
    DivisionByZero,

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Type error: expected {expected} for '{context}', got {actual}")]
    TypeError {
        expected: String,
        actual: String,
        context: String,
    },

    #[error("Arithmetic overflow: {0}")]
    Overflow(String),
}

impl EvalError {
    /// Returns the appropriate MAGI error code for this error variant.
    pub fn error_code(&self) -> &'static str {
        match self {
            EvalError::DivisionByZero => "E104",
            EvalError::TypeError { .. } => "E100",
            EvalError::Overflow(_) => "E103",
            EvalError::InvalidInput(msg) => {
                if msg.contains("exceeds")
                    || msg.contains("would produce")
                    || msg.contains("would exceed")
                    || msg.contains("byte limit")
                    || msg.contains("element limit")
                {
                    "E409"
                } else {
                    "E406"
                }
            }
        }
    }
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
