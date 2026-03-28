//! Evaluation traits and error types for the MAGI language.

use crate::types::{DataType, OperationType};
use std::collections::HashMap;

/// Error type for operation evaluation.
#[derive(Debug)]
pub enum EvalError {
    DivisionByZero,
    InvalidInput(String),
    ResourceLimit(String),
    TypeError {
        expected: String,
        actual: String,
        context: String,
    },
    Overflow(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::DivisionByZero => write!(f, "Division by zero"),
            EvalError::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            EvalError::ResourceLimit(msg) => write!(f, "Resource limit exceeded: {}", msg),
            EvalError::TypeError { expected, actual, context } => {
                write!(f, "Type error: expected {} for '{}', got {}", expected, context, actual)
            }
            EvalError::Overflow(msg) => write!(f, "Arithmetic overflow: {}", msg),
        }
    }
}

impl std::error::Error for EvalError {}

impl EvalError {
    /// Returns the appropriate MAGI error code for this error variant.
    ///
    /// Uses enum-based dispatch instead of string matching (#137).
    pub fn error_code(&self) -> &'static str {
        match self {
            EvalError::DivisionByZero => "E104",
            EvalError::TypeError { .. } => "E100",
            EvalError::Overflow(_) => "E103",
            EvalError::ResourceLimit(_) => "E409",
            EvalError::InvalidInput(_) => "E406",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_enum_dispatch() {
        assert_eq!(EvalError::DivisionByZero.error_code(), "E104");
        assert_eq!(
            EvalError::TypeError {
                expected: "Int".into(),
                actual: "String".into(),
                context: "add".into(),
            }
            .error_code(),
            "E100"
        );
        assert_eq!(EvalError::Overflow("test".into()).error_code(), "E103");
        assert_eq!(EvalError::InvalidInput("bad".into()).error_code(), "E406");
        assert_eq!(
            EvalError::ResourceLimit("too big".into()).error_code(),
            "E409"
        );
    }

    #[test]
    fn test_resource_limit_replaces_string_matching() {
        // Previously, InvalidInput with these patterns would return E409 via msg.contains().
        // Now ResourceLimit is the proper variant for resource limit errors.
        let resource_err = EvalError::ResourceLimit("exceeds limit".into());
        assert_eq!(resource_err.error_code(), "E409");

        // InvalidInput always returns E406 now, regardless of message content.
        let input_err = EvalError::InvalidInput("exceeds limit".into());
        assert_eq!(input_err.error_code(), "E406");
    }
}
