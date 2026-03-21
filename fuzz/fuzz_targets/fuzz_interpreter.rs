#![no_main]

use libfuzzer_sys::fuzz_target;
use magi_lang::eval::{EvalError, OperationEvaluator};
use magi_lang::types::{DataType, OperationType};
use std::collections::HashMap;

/// A stub evaluator that returns Null for every operation.
/// This lets the interpreter run without a full stdlib implementation.
struct StubEvaluator;

impl OperationEvaluator for StubEvaluator {
    fn eval_operation(
        &self,
        _op: OperationType,
        _inputs: &HashMap<String, DataType>,
        _config: &HashMap<String, DataType>,
    ) -> Result<DataType, EvalError> {
        Ok(DataType::Null)
    }
}

fuzz_target!(|data: &[u8]| {
    // Convert arbitrary bytes to a UTF-8 string, skipping invalid inputs.
    if let Ok(source) = std::str::from_utf8(data) {
        // Reject oversized inputs to keep fuzzing fast.
        if source.len() > 4096 {
            return;
        }

        // Parse the source. If parsing fails, that's fine.
        let program = match magi_lang::syntax::parser::parse_v2(source) {
            Ok(prog) => prog,
            Err(_) => return,
        };

        // Execute with a stub evaluator.
        // The interpreter has its own MAX_LOOP_ITERATIONS (10,000) and
        // MAX_CALL_DEPTH (48) guards, so infinite loops and deep recursion
        // are bounded. Runtime errors (InterpError) are expected and OK.
        let evaluator = StubEvaluator;
        let mut interp = magi_lang::syntax::interpreter::Interpreter::new(&evaluator);
        let _ = interp.execute(&program);
    }
});
