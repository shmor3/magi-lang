//! Backward compatibility tests for the MAGI language.
//!
//! These tests verify that fundamental MAGI programs continue to work correctly
//! across versions. They serve as a compatibility guarantee for the core language
//! features: arithmetic, strings, arrays, maps, functions, and closures.

use std::collections::HashMap;

use magi_lang::eval::{EvalError, OperationEvaluator};
use magi_lang::syntax::interpreter::Interpreter;
use magi_lang::syntax::parser::parse_v2;
use magi_lang::types::{DataType, OperationType};

/// Minimal stub evaluator that handles basic arithmetic operations.
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

/// Helper: run a MAGI program and return the collected output lines.
fn run_program(source: &str) -> Vec<String> {
    let program = parse_v2(source).expect("program should parse");
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    interp.execute(&program).expect("program should execute");
    interp.logs.iter().map(|l| l.message.clone()).collect()
}

// ============================================================================
// Arithmetic
// ============================================================================

#[test]
fn compat_integer_arithmetic() {
    let out = run_program("output 2 + 3\noutput 10 - 4\noutput 3 * 7\noutput 20 / 4\noutput 17 % 5");
    assert_eq!(out, vec!["5", "6", "21", "5", "2"]);
}

#[test]
fn compat_float_arithmetic() {
    let out = run_program("output 1.5 + 2.5\noutput 10.0 / 3.0");
    assert_eq!(out[0], "4");
    // 10/3 = 3.3333...
    assert!(out[1].starts_with("3.3333"));
}

#[test]
fn compat_negative_numbers() {
    let out = run_program("output -5\noutput -3 + 10");
    assert_eq!(out, vec!["-5", "7"]);
}

#[test]
fn compat_comparison_operators() {
    let out = run_program(
        "output 1 < 2\noutput 2 > 1\noutput 3 == 3\noutput 3 != 4\noutput 5 >= 5\noutput 4 <= 4",
    );
    assert_eq!(out, vec!["true", "true", "true", "true", "true", "true"]);
}

// ============================================================================
// Strings
// ============================================================================

#[test]
fn compat_string_concatenation() {
    let out = run_program(r#"output "hello" + " " + "world""#);
    assert_eq!(out, vec!["hello world"]);
}

#[test]
fn compat_string_interpolation() {
    let out = run_program(r#"let name = "MAGI"
output f"Hello, {name}!""#);
    assert_eq!(out, vec!["Hello, MAGI!"]);
}

#[test]
fn compat_string_length() {
    let out = run_program(r#"output "hello".len()"#);
    assert_eq!(out, vec!["5"]);
}

// ============================================================================
// Arrays
// ============================================================================

#[test]
fn compat_array_literal() {
    let out = run_program("let arr = [1, 2, 3]\noutput arr");
    assert_eq!(out, vec!["[1, 2, 3]"]);
}

#[test]
fn compat_array_index() {
    let out = run_program("let arr = [10, 20, 30]\noutput arr[1]");
    assert_eq!(out, vec!["20"]);
}

#[test]
fn compat_array_push() {
    let out = run_program("let mut arr = [1, 2]\narr.push(3)\noutput arr");
    assert_eq!(out, vec!["[1, 2, 3]"]);
}

#[test]
fn compat_array_len() {
    let out = run_program("let arr = [1, 2, 3, 4]\noutput arr.len()");
    assert_eq!(out, vec!["4"]);
}

// ============================================================================
// Maps
// ============================================================================

#[test]
fn compat_map_literal() {
    let out = run_program(r#"let m = {"a": 1, "b": 2}
output m["a"]"#);
    assert_eq!(out, vec!["1"]);
}

#[test]
fn compat_map_insert_and_access() {
    let out = run_program(r#"let mut m = {}
m["key"] = "value"
output m["key"]"#);
    assert_eq!(out, vec!["value"]);
}

// ============================================================================
// Functions
// ============================================================================

#[test]
fn compat_function_def_and_call() {
    let out = run_program(
        "fn add(a, b) { a + b }\noutput add(3, 4)",
    );
    assert_eq!(out, vec!["7"]);
}

#[test]
fn compat_function_return() {
    let out = run_program(
        "fn double(x) { return x * 2 }\noutput double(5)",
    );
    assert_eq!(out, vec!["10"]);
}

#[test]
fn compat_recursive_function() {
    let out = run_program(
        "fn factorial(n) {\n  if n <= 1 { 1 } else { n * factorial(n - 1) }\n}\noutput factorial(5)",
    );
    assert_eq!(out, vec!["120"]);
}

#[test]
fn compat_default_params() {
    let out = run_program(
        "fn greet(name, greeting = \"Hello\") { f\"{greeting}, {name}!\" }\noutput greet(\"World\")\noutput greet(\"World\", \"Hi\")",
    );
    assert_eq!(out, vec!["Hello, World!", "Hi, World!"]);
}

// ============================================================================
// Closures
// ============================================================================

#[test]
fn compat_closure_capture() {
    let out = run_program(
        "let x = 10\nlet add_x = |y| x + y\noutput add_x(5)",
    );
    assert_eq!(out, vec!["15"]);
}

#[test]
fn compat_closure_as_argument() {
    let out = run_program(
        "fn apply(f, val) { f(val) }\nlet double = |x| x * 2\noutput apply(double, 7)",
    );
    assert_eq!(out, vec!["14"]);
}

// ============================================================================
// Control flow
// ============================================================================

#[test]
fn compat_if_else() {
    let out = run_program("let x = 5\noutput if x > 3 { \"big\" } else { \"small\" }");
    assert_eq!(out, vec!["big"]);
}

#[test]
fn compat_for_loop() {
    let out = run_program(
        "let mut sum = 0\nfor i in [1, 2, 3, 4, 5] { sum += i }\noutput sum",
    );
    assert_eq!(out, vec!["15"]);
}

#[test]
fn compat_while_loop() {
    let out = run_program(
        "let mut i = 0\nlet mut sum = 0\nwhile i < 5 { sum += i; i += 1 }\noutput sum",
    );
    assert_eq!(out, vec!["10"]);
}
