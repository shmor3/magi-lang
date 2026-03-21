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

// ============================================================================
// Minimal evaluator for compat tests
// ============================================================================

/// Type-preserving binary operation helper.
fn stub_binop(
    a: &DataType,
    b: &DataType,
    int_op: fn(i64, i64) -> i64,
    float_op: fn(f64, f64) -> f64,
) -> Result<DataType, EvalError> {
    match (a, b) {
        (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(int_op(*x, *y))),
        (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(float_op(*x, *y))),
        (DataType::Int64(x), DataType::Float64(y)) => {
            Ok(DataType::Float64(float_op(*x as f64, *y)))
        }
        (DataType::Float64(x), DataType::Int64(y)) => {
            Ok(DataType::Float64(float_op(*x, *y as f64)))
        }
        _ => match (a.to_i64(), b.to_i64()) {
            (Some(x), Some(y)) => Ok(DataType::Int64(int_op(x, y))),
            _ => match (a.to_f64(), b.to_f64()) {
                (Some(x), Some(y)) => Ok(DataType::Float64(float_op(x, y))),
                _ => Ok(DataType::Null),
            },
        },
    }
}

struct CompatEvaluator;

impl OperationEvaluator for CompatEvaluator {
    fn eval_operation(
        &self,
        op: OperationType,
        inputs: &HashMap<String, DataType>,
        _config: &HashMap<String, DataType>,
    ) -> Result<DataType, EvalError> {
        let a = inputs.get("a").cloned().unwrap_or(DataType::Null);
        let b = inputs.get("b").cloned().unwrap_or(DataType::Null);
        let input = inputs
            .get("input")
            .or_else(|| inputs.get("value"))
            .or_else(|| inputs.get("array"))
            .or_else(|| inputs.get("string"))
            .cloned()
            .unwrap_or(DataType::Null);
        let array = inputs.get("array").cloned().unwrap_or(DataType::Null);
        let value = inputs.get("value").cloned().unwrap_or(DataType::Null);

        match op {
            OperationType::Add => match (&a, &b) {
                (DataType::String(x), DataType::String(y)) => {
                    Ok(DataType::String(format!("{}{}", x, y)))
                }
                _ => stub_binop(&a, &b, i64::wrapping_add, |x, y| x + y),
            },
            OperationType::Subtract => stub_binop(&a, &b, i64::wrapping_sub, |x, y| x - y),
            OperationType::Multiply => stub_binop(&a, &b, i64::wrapping_mul, |x, y| x * y),
            OperationType::Divide => stub_binop(
                &a,
                &b,
                |x, y| if y == 0 { 0 } else { x / y },
                |x, y| x / y,
            ),
            OperationType::Modulo => stub_binop(
                &a,
                &b,
                |x, y| if y == 0 { 0 } else { x % y },
                |x, y| x % y,
            ),
            OperationType::Equal => Ok(DataType::Bool(a == b)),
            OperationType::NotEqual => Ok(DataType::Bool(a != b)),
            OperationType::Greater => match (a.to_f64(), b.to_f64()) {
                (Some(x), Some(y)) => Ok(DataType::Bool(x > y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::Less => match (a.to_f64(), b.to_f64()) {
                (Some(x), Some(y)) => Ok(DataType::Bool(x < y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::GreaterEq => match (a.to_f64(), b.to_f64()) {
                (Some(x), Some(y)) => Ok(DataType::Bool(x >= y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::LessEq => match (a.to_f64(), b.to_f64()) {
                (Some(x), Some(y)) => Ok(DataType::Bool(x <= y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::Negate => match &input {
                DataType::Int64(x) => Ok(DataType::Int64(x.wrapping_neg())),
                DataType::Float64(x) => Ok(DataType::Float64(-x)),
                _ => Ok(DataType::Null),
            },
            OperationType::ToString => match &input {
                DataType::Int64(n) => Ok(DataType::String(n.to_string())),
                DataType::Float64(n) => Ok(DataType::String(n.to_string())),
                DataType::Bool(b) => Ok(DataType::String(b.to_string())),
                DataType::String(s) => Ok(DataType::String(s.clone())),
                DataType::Null => Ok(DataType::String("null".to_string())),
                _ => Ok(DataType::String("?".to_string())),
            },
            OperationType::Concat => match (&a, &b) {
                (DataType::String(x), DataType::String(y)) => {
                    Ok(DataType::String(format!("{}{}", x, y)))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayLength => match &array {
                DataType::Array(arr) => Ok(DataType::Int64(arr.len() as i64)),
                _ => Ok(DataType::Int64(0)),
            },
            OperationType::ArrayPush => {
                let mut arr = match &array {
                    DataType::Array(a) => a.clone(),
                    _ => vec![],
                };
                arr.push(value.clone());
                Ok(DataType::Array(arr))
            }
            OperationType::ArrayGet => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match &array {
                    DataType::Array(arr) => {
                        let i = index.to_i64().unwrap_or(-1);
                        if i < 0 {
                            return Ok(DataType::Null);
                        }
                        Ok(arr.get(i as usize).cloned().unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::MapGet => {
                let map_val = inputs
                    .get("map")
                    .or(inputs.get("a"))
                    .cloned()
                    .unwrap_or(DataType::Null);
                let key_val = inputs
                    .get("key")
                    .or(inputs.get("b"))
                    .cloned()
                    .unwrap_or(DataType::Null);
                match (&map_val, &key_val) {
                    (DataType::Map(map), DataType::String(key)) => {
                        Ok(map.get(key).cloned().unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::MapSet => {
                let map_val = inputs.get("map").cloned().unwrap_or(DataType::Null);
                let key_val = inputs
                    .get("key")
                    .cloned()
                    .unwrap_or(DataType::String(String::new()));
                let val = inputs.get("value").cloned().unwrap_or(DataType::Null);
                match (&map_val, &key_val) {
                    (DataType::Map(map), DataType::String(k)) => {
                        let mut new_map = map.clone();
                        new_map.insert(k.clone(), val);
                        Ok(DataType::Map(new_map))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            _ => Ok(DataType::Null),
        }
    }
}

/// Helper: run a MAGI program and return the collected output lines.
fn run_program(source: &str) -> Vec<String> {
    let program = parse_v2(source).expect("program should parse");
    let evaluator = CompatEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    interp.execute(&program).expect("program should execute");
    interp.logs.iter().map(|l| l.message.clone()).collect()
}

// ============================================================================
// Arithmetic
// ============================================================================

#[test]
fn compat_integer_arithmetic() {
    let out = run_program(
        "output 2 + 3\noutput 10 - 4\noutput 3 * 7\noutput 20 / 4\noutput 17 % 5",
    );
    assert_eq!(out, vec!["5", "6", "21", "5", "2"]);
}

#[test]
fn compat_float_arithmetic() {
    let out = run_program("output 1.5 + 2.5\noutput 10.0 / 3.0");
    assert_eq!(out[0], "4");
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
    let out = run_program("let name = \"MAGI\"\noutput f\"Hello, {name}!\"");
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
    // In MAGI, push returns a new array (immutable semantics)
    let out = run_program("let mut arr = [1, 2]\narr = arr.push(3)\noutput arr");
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
    let out = run_program("let m = {\"a\": 1, \"b\": 2}\noutput m[\"a\"]");
    assert_eq!(out, vec!["1"]);
}

#[test]
fn compat_map_insert_and_access() {
    // Use a non-empty initial map to avoid type issues with empty map assignment
    let out = run_program(
        "let mut m = {\"x\": 1}\nm[\"key\"] = \"value\"\noutput m[\"key\"]",
    );
    assert_eq!(out, vec!["value"]);
}

// ============================================================================
// Functions
// ============================================================================

#[test]
fn compat_function_def_and_call() {
    let out = run_program("fn add(a, b) { a + b }\noutput add(3, 4)");
    assert_eq!(out, vec!["7"]);
}

#[test]
fn compat_function_return() {
    let out = run_program("fn double(x) { return x * 2 }\noutput double(5)");
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
    let out = run_program("let x = 10\nlet add_x = |y| x + y\noutput add_x(5)");
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
    let out =
        run_program("let x = 5\noutput if x > 3 { \"big\" } else { \"small\" }");
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
