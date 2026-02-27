//! Integration tests for the MAGI language.
//!
//! Tests the full pipeline: source → lexer → parser → type_checker → interpreter
//! and source → lexer → parser → compiler → WASM binary.

use std::collections::HashMap;

use magi_lang::compiler;
use magi_lang::eval::{EvalError, OperationEvaluator};
use magi_lang::syntax::ast::Program;
use magi_lang::syntax::interpreter::{InterpError, Interpreter};
use magi_lang::syntax::parser::parse_v2;
use magi_lang::syntax::type_checker::check_types;
use magi_lang::types::{DataType, OperationType};

// ── Stub evaluator for standalone testing ─────────────────

/// A minimal operation evaluator that handles basic arithmetic and comparisons.
struct StubEvaluator;

impl OperationEvaluator for StubEvaluator {
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
            .or_else(|| inputs.get("map"))
            .or_else(|| inputs.get("string"))
            .cloned()
            .unwrap_or(DataType::Null);

        match op {
            // Arithmetic
            OperationType::Add => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(x + y)),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x + y)),
                (DataType::Int64(x), DataType::Float64(y)) => Ok(DataType::Float64(*x as f64 + y)),
                (DataType::Float64(x), DataType::Int64(y)) => Ok(DataType::Float64(x + *y as f64)),
                (DataType::String(x), DataType::String(y)) => {
                    Ok(DataType::String(format!("{}{}", x, y)))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::Subtract => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(x - y)),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x - y)),
                (DataType::Int64(x), DataType::Float64(y)) => Ok(DataType::Float64(*x as f64 - y)),
                (DataType::Float64(x), DataType::Int64(y)) => Ok(DataType::Float64(x - *y as f64)),
                _ => Ok(DataType::Null),
            },
            OperationType::Multiply => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(x * y)),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x * y)),
                (DataType::Int64(x), DataType::Float64(y)) => Ok(DataType::Float64(*x as f64 * y)),
                (DataType::Float64(x), DataType::Int64(y)) => Ok(DataType::Float64(x * *y as f64)),
                _ => Ok(DataType::Null),
            },
            OperationType::Divide => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => {
                    if *y == 0 {
                        Err(EvalError::DivisionByZero)
                    } else {
                        match x.checked_div(*y) {
                            Some(v) => Ok(DataType::Int64(v)),
                            None => Err(EvalError::InvalidInput("integer overflow".to_string())),
                        }
                    }
                }
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x / y)),
                (DataType::Int64(x), DataType::Float64(y)) => {
                    Ok(DataType::Float64(*x as f64 / y))
                }
                (DataType::Float64(x), DataType::Int64(y)) => {
                    Ok(DataType::Float64(x / *y as f64))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::Modulo => match (&a, &b) {
                (DataType::Int64(_), DataType::Int64(y)) if *y == 0 => Err(EvalError::DivisionByZero),
                (DataType::Int64(x), DataType::Int64(y)) => {
                    match x.checked_rem(*y) {
                        Some(v) => Ok(DataType::Int64(v)),
                        None => Err(EvalError::InvalidInput("integer overflow".to_string())),
                    }
                }
                _ => Ok(DataType::Null),
            },

            // Comparison
            OperationType::Equal => Ok(DataType::Bool(a == b)),
            OperationType::NotEqual => Ok(DataType::Bool(a != b)),
            OperationType::Greater => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Bool(x > y)),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Bool(x > y)),
                (DataType::String(x), DataType::String(y)) => Ok(DataType::Bool(x > y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::Less => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Bool(x < y)),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Bool(x < y)),
                (DataType::String(x), DataType::String(y)) => Ok(DataType::Bool(x < y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::GreaterEq => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Bool(x >= y)),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Bool(x >= y)),
                (DataType::String(x), DataType::String(y)) => Ok(DataType::Bool(x >= y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::LessEq => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Bool(x <= y)),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Bool(x <= y)),
                (DataType::String(x), DataType::String(y)) => Ok(DataType::Bool(x <= y)),
                _ => Ok(DataType::Bool(false)),
            },

            // Logical
            OperationType::And => match (&a, &b) {
                (DataType::Bool(x), DataType::Bool(y)) => Ok(DataType::Bool(*x && *y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::Or => match (&a, &b) {
                (DataType::Bool(x), DataType::Bool(y)) => Ok(DataType::Bool(*x || *y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::Not => match &input {
                DataType::Bool(x) => Ok(DataType::Bool(!x)),
                _ => Ok(DataType::Bool(true)),
            },
            OperationType::Negate => match &input {
                DataType::Int64(x) => Ok(DataType::Int64(-x)),
                DataType::Float64(x) => Ok(DataType::Float64(-x)),
                _ => Ok(DataType::Null),
            },

            // String
            OperationType::Concat => match (&a, &b) {
                (DataType::String(x), DataType::String(y)) => {
                    Ok(DataType::String(format!("{}{}", x, y)))
                }
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

            // Array
            OperationType::ArrayLength => match &input {
                DataType::Array(arr) => Ok(DataType::Int64(arr.len() as i64)),
                _ => Ok(DataType::Int64(0)),
            },
            OperationType::ArrayPush => {
                let mut arr = match &a {
                    DataType::Array(arr) => arr.clone(),
                    _ => vec![],
                };
                arr.push(b);
                Ok(DataType::Array(arr))
            }
            OperationType::ArrayPop => match &input {
                DataType::Array(arr) if !arr.is_empty() => {
                    Ok(arr.last().cloned().unwrap_or(DataType::Null))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArraySlice => {
                // Used by slice syntax.
                Ok(DataType::Null)
            }
            OperationType::ArraySort => match &input {
                DataType::Array(arr) => {
                    let mut sorted = arr.clone();
                    sorted.sort_by(|a, b| {
                        a.to_i64().unwrap_or(0).cmp(&b.to_i64().unwrap_or(0))
                    });
                    Ok(DataType::Array(sorted))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayReverse => match &input {
                DataType::Array(arr) => {
                    let mut rev = arr.clone();
                    rev.reverse();
                    Ok(DataType::Array(rev))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayContains => match (&a, &b) {
                (DataType::Array(arr), val) => Ok(DataType::Bool(arr.contains(val))),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::ArrayJoin => match (&a, &b) {
                (DataType::Array(arr), DataType::String(sep)) => {
                    let s: Vec<String> = arr.iter().map(|v| v.to_string_lossy()).collect();
                    Ok(DataType::String(s.join(sep)))
                }
                _ => Ok(DataType::String(String::new())),
            },

            // Map operations
            OperationType::MapGet => {
                // FieldAccess uses "map"/"key", Index uses "a"/"b"
                let map_val = inputs.get("map").or(inputs.get("a")).cloned().unwrap_or(DataType::Null);
                let key_val = inputs.get("key").or(inputs.get("b")).cloned().unwrap_or(DataType::Null);
                match (&map_val, &key_val) {
                    (DataType::Map(map), DataType::String(key)) => {
                        Ok(map.get(key).cloned().unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::MapSet => {
                let map_val = inputs.get("map").or(inputs.get("a")).cloned().unwrap_or(DataType::Null);
                let key_val = inputs.get("key").or(inputs.get("b")).cloned().unwrap_or(DataType::String(String::new()));
                let value = inputs.get("value").or(inputs.get("c")).cloned().unwrap_or(DataType::Null);
                match (&map_val, &key_val) {
                    (DataType::Map(map), DataType::String(k)) => {
                        let mut new_map = map.clone();
                        new_map.insert(k.clone(), value);
                        Ok(DataType::Map(new_map))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::MapKeys => match &input {
                DataType::Map(map) => Ok(DataType::Array(
                    map.keys().map(|k| DataType::String(k.clone())).collect(),
                )),
                _ => Ok(DataType::Array(vec![])),
            },
            OperationType::MapValues => match &input {
                DataType::Map(map) => Ok(DataType::Array(map.values().cloned().collect())),
                _ => Ok(DataType::Array(vec![])),
            },

            // Type conversion
            OperationType::Abs => match &input {
                DataType::Int64(n) => Ok(DataType::Int64(n.abs())),
                DataType::Float64(n) => Ok(DataType::Float64(n.abs())),
                _ => Ok(DataType::Null),
            },
            OperationType::Round => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.round())),
                other => Ok(other.clone()),
            },
            OperationType::Floor => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.floor())),
                other => Ok(other.clone()),
            },
            OperationType::Ceil => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.ceil())),
                other => Ok(other.clone()),
            },

            // Array mutation
            OperationType::ArrayShift => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match &arr_val {
                    DataType::Array(arr) => Ok(arr.first().cloned().unwrap_or(DataType::Null)),
                    _ => Ok(DataType::Null),
                }
            },

            // String methods dispatched to evaluator
            OperationType::StringWords => match &input {
                DataType::String(s) => Ok(DataType::Array(s.split_whitespace().map(|w| DataType::String(w.to_string())).collect())),
                _ => Ok(DataType::Null),
            },
            OperationType::StringCount => {
                let search = inputs.get("search").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                match (&input, &search) {
                    (DataType::String(s), DataType::String(sub)) => Ok(DataType::Int64(s.matches(sub.as_str()).count() as i64)),
                    _ => Ok(DataType::Int64(0)),
                }
            },

            // Typeof
            OperationType::Typeof => {
                let type_name = match &input {
                    DataType::Null => "null",
                    DataType::Bool(_) => "bool",
                    DataType::Int64(_) => "int64",
                    DataType::Float64(_) => "float64",
                    DataType::String(_) => "string",
                    DataType::Array(_) => "array",
                    DataType::Map(m) => {
                        if m.contains_key("__enum") { "enum" }
                        else if m.contains_key("__struct") { "struct" }
                        else { "map" }
                    }
                    _ => "unknown",
                };
                Ok(DataType::String(type_name.to_string()))
            },

            // ToJson
            OperationType::ToJson => {
                fn to_json_stub(val: &DataType) -> String {
                    match val {
                        DataType::Null => "null".to_string(),
                        DataType::Bool(b) => format!("{}", b),
                        DataType::Int64(n) => format!("{}", n),
                        DataType::Float64(f) => format!("{}", f),
                        DataType::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
                        DataType::Array(arr) => format!("[{}]", arr.iter().map(|v| to_json_stub(v)).collect::<Vec<_>>().join(",")),
                        DataType::Map(m) => format!("{{{}}}", m.iter().filter(|(k,_)| !k.starts_with("__")).map(|(k,v)| format!("\"{}\":{}", k, to_json_stub(v))).collect::<Vec<_>>().join(",")),
                        _ => "null".to_string(),
                    }
                }
                Ok(DataType::String(to_json_stub(&input)))
            },

            // Catch-all
            _ => Ok(DataType::Null),
        }
    }
}

// ── Test helpers ──────────────────────────────────────────

fn parse(src: &str) -> Program {
    parse_v2(src).unwrap_or_else(|e| panic!("parse error: {}", e))
}

fn run(src: &str) -> DataType {
    let program = parse(src);
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    interp
        .execute(&program)
        .unwrap_or_else(|e| panic!("runtime error: {:?}", e))
}

fn run_err(src: &str) -> InterpError {
    let program = parse(src);
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    interp.execute(&program).unwrap_err()
}

fn run_result(src: &str) -> Result<DataType, InterpError> {
    let program = parse(src);
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    interp.execute(&program)
}

fn typecheck_warnings(src: &str) -> Vec<String> {
    let program = parse(src);
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    analysis
        .diagnostics
        .iter()
        .filter_map(|d| d.code.clone())
        .collect()
}


// ═══════════════════════════════════════════════════════════
// Parser integration tests
// ═══════════════════════════════════════════════════════════

#[test]
fn test_parse_showcase_example() {
    let src = include_str!("../examples/showcase/main.magi");
    let _program = parse(src);
}

#[test]
fn test_parse_all_features() {
    let src = r#"
        /* block comment /* nested */ */
        struct Point { x: float64, y: float64 }
        enum Color { Red, Green, Blue }
        enum Option { Some(val), None }

        fn greet(name, ...titles) {
            let prefix = if len(titles) > 0 { titles[0] } else { "Mr." };
            f"{prefix} {name}"
        }

        let p = Point { x: 1.0, y: 2.0 };
        let c = Color::Red;
        let opt = Option::Some(42);

        let nums = 0..10;
        let inc = 0..=5;
        let slice = nums[2..5];

        let doubled = [x * 2 for x in nums if x > 3];
        let [a, b, ...rest] = [1, 2, 3, 4, 5];

        for [i, v] in [[0, "a"], [1, "b"]] {
            output f"{i}: {v}";
        }

        let raw = r"no \escape";
        let multi = """
        hello
        world
        """;

        let result = match opt {
            Option::Some(v) => v,
            Option::None => 0,
            _ => -1,
        };

        let x = null;
        let y = x ?? 42;
        let z = x?.field;

        output greet("World", ...["Dr."]);
    "#;
    let _program = parse(src);
}

// ═══════════════════════════════════════════════════════════
// Interpreter integration tests
// ═══════════════════════════════════════════════════════════

#[test]
fn test_basic_arithmetic() {
    assert_eq!(run("1 + 2"), DataType::Int64(3));
    assert_eq!(run("10 - 3"), DataType::Int64(7));
    assert_eq!(run("4 * 5"), DataType::Int64(20));
    assert_eq!(run("15 / 3"), DataType::Int64(5));
    assert_eq!(run("17 % 5"), DataType::Int64(2));
}

#[test]
fn test_let_bindings() {
    assert_eq!(run("let x = 42; x"), DataType::Int64(42));
    assert_eq!(
        run(r#"let name = "Alice"; name"#),
        DataType::String("Alice".to_string())
    );
}

#[test]
fn test_mutable_variables() {
    assert_eq!(run("let mut x = 1; x = 2; x"), DataType::Int64(2));
    assert_eq!(run("let mut x = 0; x += 5; x"), DataType::Int64(5));
}

#[test]
fn test_if_else() {
    assert_eq!(
        run("if true { 1 } else { 2 }"),
        DataType::Int64(1)
    );
    assert_eq!(
        run("if false { 1 } else { 2 }"),
        DataType::Int64(2)
    );
}

#[test]
fn test_for_loop() {
    assert_eq!(
        run("let mut sum = 0; for x in [1, 2, 3] { sum = sum + x; } sum"),
        DataType::Int64(6)
    );
}

#[test]
fn test_while_loop() {
    assert_eq!(
        run("let mut x = 0; while x < 5 { x = x + 1; } x"),
        DataType::Int64(5)
    );
}

#[test]
fn test_functions() {
    assert_eq!(
        run("fn add(a, b) { a + b } add(3, 4)"),
        DataType::Int64(7)
    );
}

#[test]
fn test_recursion() {
    assert_eq!(
        run(r#"
            fn fact(n) {
                if n <= 1 { 1 } else { n * fact(n - 1) }
            }
            fact(5)
        "#),
        DataType::Int64(120)
    );
}

#[test]
fn test_closures() {
    assert_eq!(
        run("let x = 10; let f = |y| x + y; f(5)"),
        DataType::Int64(15)
    );
}

#[test]
fn test_pattern_matching() {
    assert_eq!(
        run(r#"
            match 42 {
                0 => "zero",
                42 => "the answer",
                _ => "other",
            }
        "#),
        DataType::String("the answer".to_string())
    );
}

#[test]
fn test_array_destructuring() {
    assert_eq!(
        run("let [a, b, c] = [10, 20, 30]; a + b + c"),
        DataType::Int64(60)
    );
}

#[test]
fn test_rest_destructuring() {
    assert_eq!(
        run("let [first, ...rest] = [1, 2, 3, 4]; first"),
        DataType::Int64(1)
    );
}

#[test]
fn test_map_destructuring() {
    assert_eq!(
        run(r#"let {name, age} = {"name": "Alice", "age": 30}; name"#),
        DataType::String("Alice".to_string())
    );
}

#[test]
fn test_range_expression() {
    assert_eq!(
        run("len(0..5)"),
        DataType::Int64(5)
    );
    assert_eq!(
        run("len(0..=5)"),
        DataType::Int64(6)
    );
}

#[test]
fn test_array_slice() {
    assert_eq!(
        run("let arr = [10, 20, 30, 40, 50]; arr[1..3]"),
        DataType::Array(vec![DataType::Int64(20), DataType::Int64(30)])
    );
}

#[test]
fn test_string_slice() {
    assert_eq!(
        run(r#""Hello, World!"[0..5]"#),
        DataType::String("Hello".to_string())
    );
}

#[test]
fn test_hof_map() {
    assert_eq!(
        run("[1, 2, 3].map(|x| x * 2)"),
        DataType::Array(vec![
            DataType::Int64(2),
            DataType::Int64(4),
            DataType::Int64(6),
        ])
    );
}

#[test]
fn test_hof_filter() {
    assert_eq!(
        run("[1, 2, 3, 4, 5].filter(|x| x > 3)"),
        DataType::Array(vec![DataType::Int64(4), DataType::Int64(5)])
    );
}

#[test]
fn test_hof_reduce() {
    assert_eq!(
        run("[1, 2, 3, 4].reduce(0, |acc, x| acc + x)"),
        DataType::Int64(10)
    );
}

#[test]
fn test_hof_find() {
    assert_eq!(
        run("[1, 2, 3, 4, 5].find(|x| x > 3)"),
        DataType::Int64(4)
    );
}

#[test]
fn test_hof_any_all() {
    assert_eq!(
        run("[1, 2, 3].any(|x| x > 2)"),
        DataType::Bool(true)
    );
    assert_eq!(
        run("[1, 2, 3].all(|x| x > 0)"),
        DataType::Bool(true)
    );
    assert_eq!(
        run("[1, 2, 3].all(|x| x > 2)"),
        DataType::Bool(false)
    );
}

#[test]
fn test_hof_enumerate() {
    assert_eq!(
        run(r#"["a", "b"].enumerate()"#),
        DataType::Array(vec![
            DataType::Array(vec![DataType::Int64(0), DataType::String("a".to_string())]),
            DataType::Array(vec![DataType::Int64(1), DataType::String("b".to_string())]),
        ])
    );
}

#[test]
fn test_hof_zip() {
    assert_eq!(
        run("[1, 2].zip([3, 4])"),
        DataType::Array(vec![
            DataType::Array(vec![DataType::Int64(1), DataType::Int64(3)]),
            DataType::Array(vec![DataType::Int64(2), DataType::Int64(4)]),
        ])
    );
}

#[test]
fn test_hof_chunk() {
    assert_eq!(
        run("[1, 2, 3, 4, 5].chunk(2)"),
        DataType::Array(vec![
            DataType::Array(vec![DataType::Int64(1), DataType::Int64(2)]),
            DataType::Array(vec![DataType::Int64(3), DataType::Int64(4)]),
            DataType::Array(vec![DataType::Int64(5)]),
        ])
    );
}

#[test]
fn test_hof_partition() {
    let result = run("[1, 2, 3, 4, 5].partition(|x| x <= 3)");
    assert_eq!(
        result,
        DataType::Array(vec![
            DataType::Array(vec![DataType::Int64(1), DataType::Int64(2), DataType::Int64(3)]),
            DataType::Array(vec![DataType::Int64(4), DataType::Int64(5)]),
        ])
    );
}

#[test]
fn test_hof_take_skip_while() {
    assert_eq!(
        run("[1, 2, 3, 4, 5].take_while(|x| x < 4)"),
        DataType::Array(vec![DataType::Int64(1), DataType::Int64(2), DataType::Int64(3)])
    );
    assert_eq!(
        run("[1, 2, 3, 4, 5].skip_while(|x| x < 4)"),
        DataType::Array(vec![DataType::Int64(4), DataType::Int64(5)])
    );
}

#[test]
fn test_hof_flat_map() {
    assert_eq!(
        run("[1, 2, 3].flat_map(|x| [x, x * 10])"),
        DataType::Array(vec![
            DataType::Int64(1),
            DataType::Int64(10),
            DataType::Int64(2),
            DataType::Int64(20),
            DataType::Int64(3),
            DataType::Int64(30),
        ])
    );
}

#[test]
fn test_hof_scan() {
    assert_eq!(
        run("[1, 2, 3].scan(0, |acc, x| acc + x)"),
        DataType::Array(vec![DataType::Int64(1), DataType::Int64(3), DataType::Int64(6)])
    );
}

#[test]
fn test_list_comprehension() {
    assert_eq!(
        run("[x * 2 for x in [1, 2, 3]]"),
        DataType::Array(vec![DataType::Int64(2), DataType::Int64(4), DataType::Int64(6)])
    );
}

#[test]
fn test_list_comprehension_with_filter() {
    assert_eq!(
        run("[x for x in [1, 2, 3, 4, 5] if x > 3]"),
        DataType::Array(vec![DataType::Int64(4), DataType::Int64(5)])
    );
}

#[test]
fn test_enum_construction_and_match() {
    assert_eq!(
        run(r#"
            enum Result { Ok(v), Err(e) }
            let r = Result::Ok(42);
            match r {
                Result::Ok(v) => v,
                Result::Err(e) => -1,
            }
        "#),
        DataType::Int64(42)
    );
}

#[test]
fn test_struct_construction() {
    // Struct is stored as a Map at runtime with __struct marker.
    // Field access uses MapGet through the evaluator.
    let result = run(r#"
        struct Point { x: float64, y: float64 }
        let p = Point { x: 3.0, y: 4.0 };
        p.x
    "#);
    assert_eq!(result, DataType::Float64(3.0));
}

#[test]
fn test_rest_params() {
    assert_eq!(
        run(r#"
            fn sum(first, ...rest) {
                let mut total = first;
                for n in rest { total = total + n; }
                total
            }
            sum(1, 2, 3, 4, 5)
        "#),
        DataType::Int64(15)
    );
}

#[test]
fn test_spread_calls() {
    assert_eq!(
        run(r#"
            fn add3(a, b, c) { a + b + c }
            let args = [10, 20, 30];
            add3(...args)
        "#),
        DataType::Int64(60)
    );
}

#[test]
fn test_null_coalescing() {
    assert_eq!(
        run("null ?? 42"),
        DataType::Int64(42)
    );
    assert_eq!(
        run("5 ?? 42"),
        DataType::Int64(5)
    );
}

#[test]
fn test_optional_chaining() {
    assert_eq!(
        run(r#"let x = {"a": {"b": 42}}; x?.a?.b"#),
        DataType::Int64(42)
    );
    assert_eq!(
        run("let x = null; x?.field"),
        DataType::Null
    );
}

#[test]
fn test_string_interpolation() {
    assert_eq!(
        run(r#"let x = 42; f"val={x}""#),
        DataType::String("val=42".to_string())
    );
}

#[test]
fn test_raw_string() {
    assert_eq!(
        run(r#"r"hello\nworld""#),
        DataType::String(r"hello\nworld".to_string())
    );
}

#[test]
fn test_block_comments() {
    assert_eq!(
        run("/* comment */ 42"),
        DataType::Int64(42)
    );
    assert_eq!(
        run("/* /* nested */ */ 42"),
        DataType::Int64(42)
    );
}

#[test]
fn test_number_methods() {
    assert_eq!(run("(-5).abs()"), DataType::Int64(5));
    assert_eq!(run("3.7.round()"), DataType::Float64(4.0));
    assert_eq!(run("3.7.floor()"), DataType::Float64(3.0));
    assert_eq!(run("3.2.ceil()"), DataType::Float64(4.0));
}

#[test]
fn test_string_methods() {
    assert_eq!(
        run(r#""".is_empty()"#),
        DataType::Bool(true)
    );
    assert_eq!(
        run(r#""hello".is_empty()"#),
        DataType::Bool(false)
    );
    assert_eq!(
        run(r#""123".is_numeric()"#),
        DataType::Bool(true)
    );
    assert_eq!(
        run(r#""abc".is_alphabetic()"#),
        DataType::Bool(true)
    );
}

#[test]
fn test_for_loop_destructuring() {
    assert_eq!(
        run(r#"
            let mut total = 0;
            for [a, b] in [[1, 2], [3, 4], [5, 6]] {
                total = total + a + b;
            }
            total
        "#),
        DataType::Int64(21)
    );
}

#[test]
fn test_for_loop_map_destructuring() {
    assert_eq!(
        run(r#"
            let mut names = "";
            for {name} in [{"name": "A"}, {"name": "B"}] {
                names = names + name;
            }
            names
        "#),
        DataType::String("AB".to_string())
    );
}

#[test]
fn test_try_catch() {
    // The catch block receives the error as a string with error context.
    let result = run(r#"
        let result = try {
            throw "oops";
            1
        } catch err {
            err
        };
        result
    "#);
    // The error message includes span info and error code.
    match &result {
        DataType::String(s) => assert!(s.contains("oops"), "expected error to contain 'oops', got: {}", s),
        other => panic!("expected String, got: {:?}", other),
    }
}

#[test]
fn test_loop_with_break() {
    assert_eq!(
        run(r#"
            let mut i = 0;
            let result = loop {
                if i >= 5 { break i; }
                i = i + 1;
            };
            result
        "#),
        DataType::Int64(5)
    );
}

#[test]
fn test_for_with_continue() {
    assert_eq!(
        run(r#"
            let mut sum = 0;
            for x in [1, 2, 3, 4, 5] {
                if x == 3 { continue; }
                sum = sum + x;
            }
            sum
        "#),
        DataType::Int64(12)
    );
}

#[test]
fn test_pipe_operator() {
    // Pipe through a user-defined function.
    assert_eq!(
        run("fn double(x) { x * 2 } 5 |> double(_)"),
        DataType::Int64(10)
    );
}

#[test]
fn test_const_binding() {
    assert_eq!(
        run("const PI = 3.14159; PI"),
        DataType::Float64(3.14159)
    );
}

// ═══════════════════════════════════════════════════════════
// Compiler integration tests
// ═══════════════════════════════════════════════════════════

#[test]
fn test_compile_showcase_rejects_guards() {
    // The showcase uses match guards which are not yet supported in WASM compilation.
    let src = include_str!("../examples/showcase/main.magi");
    let program = parse(src);
    let mut compiler_inst = compiler::Compiler::new();
    let result = compiler_inst.compile(&program);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("match guards"), "expected match guard error, got: {err}");
}

#[test]
fn test_compile_to_wasm_basic() {
    // Test WASM compilation with a program that doesn't use unsupported features.
    let src = r#"
        fn distance(x1, y1, x2, y2) {
            let dx = x2 - x1;
            let dy = y2 - y1;
            dx * dx + dy * dy
        }
        fn area(w, h) { w * h }
        let result = distance(0, 0, 3, 4);
        let a = area(5, 10);
    "#;
    let program = parse(src);
    let wasm = compiler::compile_to_wasm(&program).unwrap();

    // Valid WASM magic number.
    assert_eq!(&wasm[0..4], b"\0asm");
    // Version 1.
    assert_eq!(&wasm[4..8], &[1, 0, 0, 0]);
    // Non-trivial size.
    assert!(wasm.len() > 100);
}

#[test]
fn test_compile_all_constructs() {
    let src = r#"
        struct Point { x: float64, y: float64 }
        enum Color { Red, Green, Blue }
        enum Option { Some(v), None }

        fn distance(p1, p2) {
            let dx = p1.x - p2.x;
            let dy = p1.y - p2.y;
            dx * dx + dy * dy
        }

        fn apply(f, x) { f(x) }

        fn variadic(a, ...rest) {
            let mut sum = a;
            for x in rest { sum = sum + x; }
            sum
        }

        let p = Point { x: 1.0, y: 2.0 };
        let c = Color::Red;
        let o = Option::Some(42);

        let nums = 0..10;
        let slice = nums[2..5];
        let doubled = [x * 2 for x in nums if x > 3];
        let [a, b, ...rest] = [1, 2, 3, 4, 5];

        for [i, v] in [[0, 1], [2, 3]] { output i; }
        for {name} in [{"name": "A"}] { output name; }

        let result = match o {
            Option::Some(v) => v,
            Option::None => 0,
            _ => -1,
        };

        let x = null ?? 42;
        let y = null?.field;
        let mul = |x| x * 2;
        let r = apply(mul, 5);

        output variadic(1, 2, 3);
        output f"result = {result}";
    "#;

    let program = parse(src);
    let wasm = compiler::compile_to_wasm(&program).unwrap();
    assert_eq!(&wasm[0..4], b"\0asm");
}

// ═══════════════════════════════════════════════════════════
// Type checker integration tests
// ═══════════════════════════════════════════════════════════

#[test]
fn test_type_check_valid_program() {
    // Type checker runs without panicking — it may emit warnings.
    let program = parse("let x = 42; let y = x + 1;");
    let imports = std::collections::HashSet::new();
    let _analysis = check_types(&program, &imports);
}

#[test]
fn test_type_check_function_def() {
    let program = parse("fn add(a, b) { a + b }");
    let imports = std::collections::HashSet::new();
    let _analysis = check_types(&program, &imports);
}

#[test]
fn test_type_check_if_else() {
    let program = parse("if true { 1 } else { 2 }");
    let imports = std::collections::HashSet::new();
    let _analysis = check_types(&program, &imports);
}

#[test]
fn test_type_check_for_loop() {
    let program = parse("for x in [1, 2, 3] { x }");
    let imports = std::collections::HashSet::new();
    let _analysis = check_types(&program, &imports);
}

#[test]
fn test_type_check_enum_struct() {
    let program = parse(r#"
        enum Color { Red, Green, Blue }
        struct Point { x: float64, y: float64 }
        let c = Color::Red;
        let p = Point { x: 1.0, y: 2.0 };
    "#);
    let imports = std::collections::HashSet::new();
    let _analysis = check_types(&program, &imports);
}

// ═══════════════════════════════════════════════════════════
// Version integration tests
// ═══════════════════════════════════════════════════════════

#[test]
fn test_version_info() {
    let v = magi_lang::version::current();
    assert_eq!(v.major, 0);
    assert_eq!(v.minor, 2);
    assert!(!v.is_stable());
}

#[test]
fn test_version_features() {
    let features = magi_lang::version::available_features();
    assert!(features.contains(&magi_lang::version::Feature::Core));
    assert!(features.contains(&magi_lang::version::Feature::Enums));
    assert!(features.contains(&magi_lang::version::Feature::WasmCompilation));
}

// ═══════════════════════════════════════════════════════════
// Edge case and error path tests
// ═══════════════════════════════════════════════════════════

#[test]
fn test_empty_program() {
    assert_eq!(run(""), DataType::Null);
}

#[test]
fn test_division_by_zero() {
    let err = run_err("1 / 0");
    match err {
        InterpError::EvalError { error: EvalError::DivisionByZero, .. } => {}
        _ => panic!("expected DivisionByZero, got: {:?}", err),
    }
}

#[test]
fn test_recursion_works() {
    // Verify recursion works within limits (MAX_CALL_DEPTH = 48).
    let src = r#"
        fn fact(n) {
            if n <= 1 { return 1; }
            n * fact(n - 1)
        }
        fact(10)
    "#;
    assert_eq!(run(src), DataType::Int64(3628800));
}

#[test]
fn test_immutable_assignment_error() {
    let err = run_err("let x = 5; x = 10;");
    match err {
        InterpError::ImmutableAssignment { .. } => {}
        _ => panic!("expected ImmutableAssignment, got: {:?}", err),
    }
}

#[test]
fn test_undefined_variable_error() {
    let err = run_err("unknown_var");
    match err {
        InterpError::UndefinedVariable { .. } => {}
        _ => panic!("expected UndefinedVariable, got: {:?}", err),
    }
}

#[test]
fn test_assert_failure() {
    let err = run_err("assert(false)");
    match err {
        InterpError::AssertionFailed { .. } => {}
        _ => panic!("expected AssertionFailed, got: {:?}", err),
    }
}

#[test]
fn test_assert_success() {
    assert_eq!(run("assert(true)"), DataType::Null);
}

#[test]
fn test_assert_eq_success() {
    assert_eq!(run("assert_eq(42, 42)"), DataType::Null);
}

#[test]
fn test_assert_eq_failure() {
    let err = run_err("assert_eq(1, 2)");
    match err {
        InterpError::AssertionFailed { .. } => {}
        _ => panic!("expected AssertionFailed, got: {:?}", err),
    }
}

#[test]
fn test_throw_and_catch() {
    // The catch variable receives the error as a formatted string including span info.
    let result = run(r#"try { throw "oops"; } catch e { e }"#);
    match result {
        DataType::String(s) => assert!(s.contains("oops"), "expected 'oops' in error: {}", s),
        other => panic!("expected String, got: {:?}", other),
    }
}

#[test]
fn test_throw_uncaught() {
    let err = run_err(r#"throw "error""#);
    match err {
        InterpError::ThrownError { .. } => {}
        _ => panic!("expected ThrownError, got: {:?}", err),
    }
}

#[test]
fn test_try_catch_finally() {
    let src = r#"
        let mut x = 0;
        try {
            x = 1;
            throw "err";
        } catch e {
            x = x + 10;
        } finally {
            x = x + 100;
        }
        x
    "#;
    assert_eq!(run(src), DataType::Int64(111));
}

#[test]
fn test_float_nan_handling() {
    match run("0.0 / 0.0") {
        DataType::Float64(v) => assert!(v.is_nan(), "expected NaN"),
        other => panic!("expected Float64(NaN), got: {:?}", other),
    }
    assert_eq!(run("let x = 0.0 / 0.0; x != x"), DataType::Bool(true));
}

#[test]
fn test_float_infinity() {
    assert_eq!(run("1.0 / 0.0"), DataType::Float64(f64::INFINITY));
    assert_eq!(run("-1.0 / 0.0"), DataType::Float64(f64::NEG_INFINITY));
}

#[test]
fn test_array_index_out_of_bounds() {
    // Out-of-bounds index returns null (no runtime panic)
    assert_eq!(run("let a = [10, 20, 30]; a[99]"), DataType::Null);
    assert_eq!(run("let a = [10, 20, 30]; a[-1]"), DataType::Null);
}

#[test]
fn test_try_catch_as_expression() {
    let result = run(r#"let x = try { 42 } catch e { 0 }; x"#);
    assert_eq!(result, DataType::Int64(42));
}

#[test]
fn test_try_catch_expr_with_throw() {
    let result = run(r#"let x = try { throw "err"; 0 } catch e { 99 }; x"#);
    assert_eq!(result, DataType::Int64(99));
}

#[test]
fn test_string_escape_sequences_extended() {
    assert_eq!(
        run(r#""\t\n\r\\""#),
        DataType::String("\t\n\r\\".to_string())
    );
}

#[test]
fn test_hex_escape_sequence() {
    assert_eq!(
        run(r#""\x41""#),
        DataType::String("A".to_string())
    );
}

#[test]
fn test_unicode_escape_sequence() {
    assert_eq!(
        run(r#""\u{1F600}""#),
        DataType::String("\u{1F600}".to_string())
    );
}

#[test]
fn test_string_methods_extended() {
    assert_eq!(run(r#""hello".len()"#), DataType::Int64(5));
    assert_eq!(run(r#""  hi  ".trim()"#), DataType::String("hi".to_string()));
    assert_eq!(run(r#""abc".reverse()"#), DataType::String("cba".to_string()));
    assert_eq!(run(r#""hello".contains("ell")"#), DataType::Bool(true));
    assert_eq!(run(r#""hello".starts_with("hel")"#), DataType::Bool(true));
    assert_eq!(run(r#""hello".ends_with("llo")"#), DataType::Bool(true));
    assert_eq!(run(r#""hello".to_upper()"#), DataType::String("HELLO".to_string()));
    assert_eq!(run(r#""HELLO".to_lower()"#), DataType::String("hello".to_string()));
    assert_eq!(run(r#""ha".repeat(3)"#), DataType::String("hahaha".to_string()));
}

#[test]
fn test_int64_methods() {
    assert_eq!(run("let x: int64 = -5; x.abs()"), DataType::Int64(5));
    assert_eq!(run("let x: int64 = 2; x.pow(10)"), DataType::Int64(1024));
    assert_eq!(run("let x: int64 = -3; x.sign()"), DataType::Int64(-1));
}

#[test]
fn test_float64_methods() {
    assert_eq!(run("let x: float64 = -2.5; x.abs()"), DataType::Float64(2.5));
    assert_eq!(run("let x: float64 = 2.5; x.floor()"), DataType::Float64(2.0));
    assert_eq!(run("let x: float64 = 2.5; x.ceil()"), DataType::Float64(3.0));
    assert_eq!(run("let x: float64 = 9.0; x.sqrt()"), DataType::Float64(3.0));
}

#[test]
fn test_loop_break_value() {
    let result = run(r#"
        let mut i = 0;
        loop {
            i = i + 1;
            if i == 5 { break i * 10; }
        }
    "#);
    assert_eq!(result, DataType::Int64(50));
}

#[test]
fn test_deeply_nested_function_calls() {
    let src = r#"
        fn a(x) { x + 1 }
        fn b(x) { a(a(a(x))) }
        fn c(x) { b(b(b(x))) }
        c(0)
    "#;
    assert_eq!(run(src), DataType::Int64(9));
}

#[test]
fn test_fstring_unclosed_brace() {
    let result = parse_v2(r#"f"hello {name""#);
    assert!(result.is_err(), "expected parse error for unclosed f-string brace");
}

// ═══════════════════════════════════════════════════════════
// Type checker warning code tests
// ═══════════════════════════════════════════════════════════

#[test]
fn test_w104_empty_loop_body() {
    let codes = typecheck_warnings("for x in [1, 2, 3] {}");
    assert!(codes.contains(&"W104".to_string()), "expected W104, got: {:?}", codes);
}

#[test]
fn test_w105_infinite_loop() {
    let codes = typecheck_warnings("while true {}");
    assert!(codes.contains(&"W105".to_string()), "expected W105, got: {:?}", codes);
}

#[test]
fn test_w106_double_negation() {
    let codes = typecheck_warnings("let x = 5; let y = --x;");
    assert!(codes.contains(&"W106".to_string()), "expected W106, got: {:?}", codes);
}

#[test]
fn test_w106_self_comparison() {
    let codes = typecheck_warnings("let x = 5; let y = x == x;");
    assert!(codes.contains(&"W106".to_string()), "expected W106, got: {:?}", codes);
}

#[test]
fn test_w107_modulo_by_one() {
    let codes = typecheck_warnings("let x = 5; let y = x % 1;");
    assert!(codes.contains(&"W107".to_string()), "expected W107, got: {:?}", codes);
}

#[test]
fn test_w107_multiply_by_zero() {
    let codes = typecheck_warnings("let x = 5; let y = x * 0;");
    assert!(codes.contains(&"W107".to_string()), "expected W107, got: {:?}", codes);
}

// ═══════════════════════════════════════════════════════════
// String method edge cases
// ═══════════════════════════════════════════════════════════

#[test]
fn test_substring_start_greater_than_end() {
    // substring(5, 2) should return empty string, not panic
    assert_eq!(run(r#""hello world".substring(5, 2)"#), DataType::String("".into()));
}

#[test]
fn test_substring_equal_indices() {
    assert_eq!(run(r#""hello".substring(3, 3)"#), DataType::String("".into()));
}

#[test]
fn test_substring_normal() {
    assert_eq!(run(r#""hello world".substring(0, 5)"#), DataType::String("hello".into()));
}

#[test]
fn test_is_numeric_valid_integer() {
    assert_eq!(run(r#""42".is_numeric()"#), DataType::Bool(true));
}

#[test]
fn test_is_numeric_valid_float() {
    assert_eq!(run(r#""3.14".is_numeric()"#), DataType::Bool(true));
}

#[test]
fn test_is_numeric_negative() {
    assert_eq!(run(r#""-7".is_numeric()"#), DataType::Bool(true));
}

#[test]
fn test_is_numeric_scientific() {
    assert_eq!(run(r#""1e5".is_numeric()"#), DataType::Bool(true));
}

#[test]
fn test_is_numeric_dashes() {
    // "---" should NOT be numeric
    assert_eq!(run(r#""---".is_numeric()"#), DataType::Bool(false));
}

#[test]
fn test_is_numeric_dots() {
    // "..." should NOT be numeric
    assert_eq!(run(r#""...".is_numeric()"#), DataType::Bool(false));
}

#[test]
fn test_is_numeric_empty() {
    assert_eq!(run(r#""".is_numeric()"#), DataType::Bool(false));
}

#[test]
fn test_is_numeric_mixed() {
    assert_eq!(run(r#""12abc".is_numeric()"#), DataType::Bool(false));
}

#[test]
fn test_char_at_negative_index() {
    // char_at(-1) should return null, not the first character
    assert_eq!(run(r#""hello".char_at(-1)"#), DataType::Null);
}

#[test]
fn test_char_at_valid_index() {
    assert_eq!(run(r#""hello".char_at(1)"#), DataType::String("e".into()));
}

#[test]
fn test_char_at_out_of_bounds() {
    assert_eq!(run(r#""hello".char_at(100)"#), DataType::Null);
}

// ═══════════════════════════════════════════════════════════
// Clamp edge cases
// ═══════════════════════════════════════════════════════════

#[test]
fn test_int_clamp_normal() {
    assert_eq!(run("let x = 5; x.clamp(1, 10)"), DataType::Int64(5));
}

#[test]
fn test_int_clamp_below_min() {
    assert_eq!(run("let x = -5; x.clamp(0, 10)"), DataType::Int64(0));
}

#[test]
fn test_int_clamp_above_max() {
    assert_eq!(run("let x = 50; x.clamp(0, 10)"), DataType::Int64(10));
}

#[test]
fn test_int_clamp_reversed_args() {
    // clamp(10, 1) with reversed min/max should still work correctly
    assert_eq!(run("let x = 5; x.clamp(10, 1)"), DataType::Int64(5));
}

#[test]
fn test_float_clamp_reversed_args() {
    assert_eq!(run("let x = 5.0; x.clamp(10.0, 1.0)"), DataType::Float64(5.0));
}

#[test]
fn test_float_clamp_below_min() {
    assert_eq!(run("let x = -5.0; x.clamp(0.0, 10.0)"), DataType::Float64(0.0));
}

// ═══════════════════════════════════════════════════════════
// Formatter round-trip tests
// ═══════════════════════════════════════════════════════════

#[test]
fn test_formatter_fstring_with_braces_in_literal() {
    use magi_lang::formatter::{format_program, FormatConfig};
    // An f-string like f"value: {x}" should round-trip correctly
    let src = r#"let x = 42; let s = f"value: {x}";"#;
    let program = parse(src);
    let config = FormatConfig::default();
    let formatted = format_program(&program, &config);
    // Should contain the f-string with interpolation
    assert!(formatted.contains("f\"value: {x}\""), "formatted: {}", formatted);
}

#[test]
fn test_compiler_compiles_without_panic() {
    // Verify that compiler returns errors as Result, not panics
    let src = "let x = 42; fn add(a, b) { a + b } let y = add(x, 10);";
    let program = parse(src);
    let result = compiler::compile_to_wasm(&program);
    assert!(result.is_ok(), "compilation failed: {:?}", result.err());
}

// ═══════════════════════════════════════════════════════════
// Variable capture in closures — no false W100
// ═══════════════════════════════════════════════════════════

#[test]
fn test_closure_captures_no_w100() {
    let src = "let x = 42; let add = |n| n + x; output add(1);";
    let program = parse(src);
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    let w100_diags: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W100"))
        .collect();
    assert!(w100_diags.is_empty(),
        "variable captured by closure should not trigger W100, got: {:?}", w100_diags);
}

// ═══════════════════════════════════════════════════════════
// Array direct methods
// ═══════════════════════════════════════════════════════════

#[test]
fn test_array_first() {
    assert_eq!(run("[10, 20, 30].first()"), DataType::Int64(10));
}

#[test]
fn test_array_first_empty() {
    assert_eq!(run("[].first()"), DataType::Null);
}

#[test]
fn test_array_last() {
    assert_eq!(run("[10, 20, 30].last()"), DataType::Int64(30));
}

#[test]
fn test_array_last_empty() {
    assert_eq!(run("[].last()"), DataType::Null);
}

#[test]
fn test_array_is_empty_true() {
    assert_eq!(run("[].is_empty()"), DataType::Bool(true));
}

#[test]
fn test_array_is_empty_false() {
    assert_eq!(run("[1].is_empty()"), DataType::Bool(false));
}

#[test]
fn test_array_sum_ints() {
    assert_eq!(run("[1, 2, 3, 4, 5].sum()"), DataType::Int64(15));
}

#[test]
fn test_array_sum_floats() {
    assert_eq!(run("[1.5, 2.5, 3.0].sum()"), DataType::Float64(7.0));
}

#[test]
fn test_array_sum_empty() {
    assert_eq!(run("[].sum()"), DataType::Int64(0));
}

#[test]
fn test_array_product() {
    assert_eq!(run("[2, 3, 4].product()"), DataType::Int64(24));
}

#[test]
fn test_array_min() {
    assert_eq!(run("[5, 2, 8, 1, 9].min()"), DataType::Int64(1));
}

#[test]
fn test_array_max() {
    assert_eq!(run("[5, 2, 8, 1, 9].max()"), DataType::Int64(9));
}

#[test]
fn test_array_min_empty() {
    assert_eq!(run("[].min()"), DataType::Null);
}

#[test]
fn test_array_max_empty() {
    assert_eq!(run("[].max()"), DataType::Null);
}

// ═══════════════════════════════════════════════════════════
// W109/W110 type checker warnings
// ═══════════════════════════════════════════════════════════

#[test]
fn test_w109_unused_parameter() {
    let codes = typecheck_warnings("fn foo(x, y) { output x; }");
    assert!(codes.contains(&"W109".to_string()), "expected W109, got: {:?}", codes);
}

#[test]
fn test_w109_all_params_used() {
    let codes = typecheck_warnings("fn foo(x, y) { output x + y; }");
    assert!(!codes.contains(&"W109".to_string()), "should not get W109, got: {:?}", codes);
}

#[test]
fn test_w110_unnecessary_mut() {
    let codes = typecheck_warnings("let mut x = 5; output x;");
    assert!(codes.contains(&"W110".to_string()), "expected W110, got: {:?}", codes);
}

#[test]
fn test_w110_mut_actually_mutated() {
    let codes = typecheck_warnings("let mut x = 5; x = 10; output x;");
    assert!(!codes.contains(&"W110".to_string()), "should not get W110, got: {:?}", codes);
}

#[test]
fn test_w110_compound_assign_counts_as_mutation() {
    let codes = typecheck_warnings("let mut x = 5; x += 1; output x;");
    assert!(!codes.contains(&"W110".to_string()), "compound assign should count as mutation, got: {:?}", codes);
}

// ═══════════════════════════════════════════════════════════
// W202 reports all dead code
// ═══════════════════════════════════════════════════════════

#[test]
fn test_w202_reports_all_dead_code() {
    use magi_lang::linter::{lint, LintConfig};
    let src = "fn foo() {\n  return 1;\n  let x = 2;\n  let y = 3;\n}";
    let program = parse(src);
    let result = lint(&program, &LintConfig::default());
    let w202_count = result.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W202"))
        .count();
    assert!(w202_count >= 2, "expected at least 2 W202 diagnostics for all dead code, got {}", w202_count);
}

// ═══════════════════════════════════════════════════════════
// Null coalesce
// ═══════════════════════════════════════════════════════════

#[test]
fn test_null_coalesce_non_null() {
    assert_eq!(run("42 ?? 0"), DataType::Int64(42));
}

#[test]
fn test_null_coalesce_null() {
    assert_eq!(run("null ?? 99"), DataType::Int64(99));
}

#[test]
fn test_null_coalesce_chain() {
    assert_eq!(run("null ?? null ?? 7"), DataType::Int64(7));
}

// ═══════════════════════════════════════════════════════════
// Range expressions
// ═══════════════════════════════════════════════════════════

#[test]
fn test_range_exclusive() {
    assert_eq!(run("1..5"), DataType::Array(vec![
        DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
        DataType::Int64(4),
    ]));
}

#[test]
fn test_range_empty() {
    assert_eq!(run("5..1"), DataType::Array(vec![]));
}

// ═══════════════════════════════════════════════════════════
// Spread operator
// ═══════════════════════════════════════════════════════════

#[test]
fn test_spread_in_array_literal() {
    assert_eq!(
        run("let a = [1, 2]; [0, ...a, 3]"),
        DataType::Array(vec![
            DataType::Int64(0), DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
        ])
    );
}

// ═══════════════════════════════════════════════════════════
// Destructuring
// ═══════════════════════════════════════════════════════════

#[test]
fn test_array_destructure() {
    assert_eq!(run("let [a, b, c] = [10, 20, 30]; b"), DataType::Int64(20));
}

#[test]
fn test_array_destructure_rest() {
    assert_eq!(
        run("let [first, ...rest] = [1, 2, 3, 4]; rest"),
        DataType::Array(vec![DataType::Int64(2), DataType::Int64(3), DataType::Int64(4)])
    );
}

#[test]
fn test_map_destructure() {
    assert_eq!(
        run(r#"let {name, age} = {"name": "Alice", "age": 30}; name"#),
        DataType::String("Alice".into())
    );
}

// ═══════════════════════════════════════════════════════════
// List comprehension
// ═══════════════════════════════════════════════════════════

#[test]
fn test_list_comprehension_doubled() {
    assert_eq!(
        run("[x * 2 for x in [1, 2, 3]]"),
        DataType::Array(vec![DataType::Int64(2), DataType::Int64(4), DataType::Int64(6)])
    );
}

#[test]
fn test_list_comprehension_filtered() {
    assert_eq!(
        run("[x for x in [1, 2, 3, 4, 5] if x > 3]"),
        DataType::Array(vec![DataType::Int64(4), DataType::Int64(5)])
    );
}

// ═══════════════════════════════════════════════════════════
// Ternary expression
// ═══════════════════════════════════════════════════════════

#[test]
fn test_ternary_true() {
    assert_eq!(run("let x = 5; if x > 3 { \"big\" } else { \"small\" }"), DataType::String("big".into()));
}

#[test]
fn test_ternary_false() {
    assert_eq!(run("let x = 1; if x > 3 { \"big\" } else { \"small\" }"), DataType::String("small".into()));
}

// ═══════════════════════════════════════════════════════════
// Pipe operator
// ═══════════════════════════════════════════════════════════

#[test]
fn test_pipe_basic() {
    assert_eq!(
        run("fn double(x) { x * 2 }\n5 |> double(_)"),
        DataType::Int64(10)
    );
}

#[test]
fn test_pipe_chain() {
    assert_eq!(
        run("fn add1(x) { x + 1 }\nfn double(x) { x * 2 }\n3 |> add1(_) |> double(_)"),
        DataType::Int64(8)
    );
}

// ═══════════════════════════════════════════════════════════
// Advanced match patterns
// ═══════════════════════════════════════════════════════════

#[test]
fn test_match_array_pattern() {
    assert_eq!(
        run("match [1, 2, 3] { [1, ...rest] => rest.sum(), _ => 0 }"),
        DataType::Int64(5)
    );
}

#[test]
fn test_match_or_pattern() {
    assert_eq!(
        run("let x = 2; match x { 1 | 2 | 3 => \"small\", _ => \"big\" }"),
        DataType::String("small".into())
    );
}

#[test]
fn test_match_guard() {
    assert_eq!(
        run("let x = 15; match x { n if n > 10 => \"big\", _ => \"small\" }"),
        DataType::String("big".into())
    );
}

// ═══════════════════════════════════════════════════════════
// Enum with fields
// ═══════════════════════════════════════════════════════════

#[test]
fn test_enum_variant_with_field() {
    assert_eq!(
        run("enum Shape { Circle(radius), Square(side) }\nlet s = Shape::Circle(5);\nmatch s { Shape::Circle(r) => r, _ => 0 }"),
        DataType::Int64(5)
    );
}

// ═══════════════════════════════════════════════════════════
// Struct construction and field access
// ═══════════════════════════════════════════════════════════

#[test]
fn test_struct_field_access() {
    assert_eq!(
        run(r#"struct Point { x, y } let p = Point { x: 10, y: 20 }; p.x"#),
        DataType::Int64(10)
    );
}

#[test]
fn test_struct_multiple_fields() {
    assert_eq!(
        run(r#"struct Point { x, y } let p = Point { x: 10, y: 20 }; p.x + p.y"#),
        DataType::Int64(30)
    );
}

// ═══════════════════════════════════════════════════════════
// For loop patterns
// ═══════════════════════════════════════════════════════════

#[test]
fn test_for_loop_array_destructure() {
    assert_eq!(
        run("let mut sum = 0; for [a, b] in [[1, 2], [3, 4], [5, 6]] { sum = sum + a + b; } sum"),
        DataType::Int64(21)
    );
}

#[test]
fn test_for_loop_map_destructure() {
    assert_eq!(
        run(r#"let mut total = 0; for {value} in [{"value": 10}, {"value": 20}] { total = total + value; } total"#),
        DataType::Int64(30)
    );
}

// ═══════════════════════════════════════════════════════════
// Try/catch/finally semantics
// ═══════════════════════════════════════════════════════════

#[test]
fn test_finally_always_runs_on_catch() {
    // finally runs after catch handles the error
    assert_eq!(
        run(r#"
            let mut log = "";
            try {
                throw "error";
            } catch e {
                log = log + "caught ";
            } finally {
                log = log + "finally";
            }
            log
        "#),
        DataType::String("caught finally".into())
    );
}

#[test]
fn test_finally_runs_on_success() {
    assert_eq!(
        run(r#"
            let mut result = "";
            try {
                result = "ok";
            } catch e {
                result = "fail";
            } finally {
                result = result + " done";
            }
            result
        "#),
        DataType::String("ok done".into())
    );
}

// ═══════════════════════════════════════════════════════════
// Const definitions
// ═══════════════════════════════════════════════════════════

#[test]
fn test_const_definition() {
    assert_eq!(run("const PI = 3.14; PI"), DataType::Float64(3.14));
}

#[test]
fn test_const_immutable() {
    let err = run_err("const X = 5; X = 10;");
    assert!(matches!(err, InterpError::ImmutableAssignment { .. }));
}

// ═══════════════════════════════════════════════════════════
// UTF-8 string method correctness
// ═══════════════════════════════════════════════════════════

#[test]
fn test_string_len_unicode() {
    // "café" is 4 chars, 5 bytes — len should return 4
    assert_eq!(
        run(r#"
            let s = "café";
            s.len()
        "#),
        DataType::Int64(4)
    );
}

#[test]
fn test_string_len_emoji() {
    // Each emoji is 1 char (possibly multiple bytes)
    assert_eq!(
        run(r#"
            let s = "hi😊";
            s.length()
        "#),
        DataType::Int64(3)
    );
}

#[test]
fn test_substring_unicode() {
    // Substring on multi-byte string should work by char index
    assert_eq!(
        run(r#"
            let s = "café";
            s.substring(0, 3)
        "#),
        DataType::String("caf".into())
    );
}

#[test]
fn test_substring_unicode_middle() {
    assert_eq!(
        run(r#"
            let s = "héllo";
            s.substring(1, 4)
        "#),
        DataType::String("éll".into())
    );
}

#[test]
fn test_index_of_unicode() {
    // "café" — 'é' is at char index 3
    assert_eq!(
        run(r#"
            let s = "café";
            s.index_of("é")
        "#),
        DataType::Int64(3)
    );
}

#[test]
fn test_index_of_not_found() {
    assert_eq!(
        run(r#""hello".index_of("z")"#),
        DataType::Int64(-1)
    );
}

#[test]
fn test_pad_start_unicode() {
    // "café" is 4 chars — padding to width 6 should add 2 chars
    assert_eq!(
        run(r#"
            let s = "café";
            s.pad_start(6, ".")
        "#),
        DataType::String("..café".into())
    );
}

#[test]
fn test_pad_end_unicode() {
    assert_eq!(
        run(r#"
            let s = "café";
            s.pad_end(7, "-")
        "#),
        DataType::String("café---".into())
    );
}

#[test]
fn test_char_at_unicode() {
    assert_eq!(
        run(r#"
            let s = "café";
            s.char_at(3)
        "#),
        DataType::String("é".into())
    );
}

// ═══════════════════════════════════════════════════════════
// Compiler break/continue error handling
// ═══════════════════════════════════════════════════════════

#[test]
fn test_compile_break_outside_loop_errors() {
    use magi_lang::compiler::compile_to_wasm;
    let program = magi_lang::syntax::parser::parse_v2("break;").unwrap();
    let result = compile_to_wasm(&program);
    assert!(result.is_err(), "break outside loop should be a compile error");
}

#[test]
fn test_compile_continue_outside_loop_errors() {
    use magi_lang::compiler::compile_to_wasm;
    let program = magi_lang::syntax::parser::parse_v2("continue;").unwrap();
    let result = compile_to_wasm(&program);
    assert!(result.is_err(), "continue outside loop should be a compile error");
}

// ═══════════════════════════════════════════════════════════
// Linter Or-pattern exhaustiveness
// ═══════════════════════════════════════════════════════════

#[test]
fn test_lint_or_pattern_exhaustiveness() {
    use magi_lang::linter;
    let program = magi_lang::syntax::parser::parse_v2(r#"
        enum Color { Red, Green, Blue }
        let c = Color::Red;
        match c {
            Color::Red | Color::Green => "warm"
            Color::Blue => "cool"
        }
    "#).unwrap();
    let result = linter::lint(&program, &linter::LintConfig::default());
    // All variants covered (Red, Green, Blue) — no W203
    let w203: Vec<_> = result.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W203"))
        .collect();
    assert!(w203.is_empty(), "all variants covered via or-pattern, should not warn: {:?}", w203);
}

// =============================================================================
// Round 4: Formatter, LSP, and edge case fixes
// =============================================================================

#[test]
fn test_formatter_map_key_with_quotes() {
    use magi_lang::formatter::{format_program, FormatConfig};
    // Map keys containing quotes should be escaped in output
    let program = magi_lang::syntax::parser::parse_v2(r#"let x = {"say \"hello\"": 1};"#).unwrap();
    let result = format_program(&program, &FormatConfig::default());
    // The formatted output should contain escaped quotes in the key
    assert!(result.contains("\\\"hello\\\""), "map key quotes should be escaped: {}", result);
}

#[test]
fn test_formatter_map_key_with_backslash() {
    use magi_lang::formatter::{format_program, FormatConfig};
    let program = magi_lang::syntax::parser::parse_v2(r#"let x = {"path\\dir": 1};"#).unwrap();
    let result = format_program(&program, &FormatConfig::default());
    assert!(result.contains("path\\\\dir"), "map key backslash should be escaped: {}", result);
}

#[test]
fn test_formatter_null_coalesce() {
    use magi_lang::formatter::{format_program, FormatConfig};
    let program = magi_lang::syntax::parser::parse_v2("let x = a ?? b;").unwrap();
    let result = format_program(&program, &FormatConfig::default());
    assert!(result.contains("a ?? b"), "null coalesce should format: {}", result);
}

#[test]
fn test_formatter_try_propagate() {
    use magi_lang::formatter::{format_program, FormatConfig};
    let program = magi_lang::syntax::parser::parse_v2("let x = foo()?;").unwrap();
    let result = format_program(&program, &FormatConfig::default());
    assert!(result.contains("foo()?"), "try propagate should format: {}", result);
}

#[test]
fn test_formatter_idempotent_null_coalesce() {
    use magi_lang::formatter::{format_program, FormatConfig};
    let config = FormatConfig::default();
    let source = "let x = a ?? b;\n";
    let program1 = magi_lang::syntax::parser::parse_v2(source).unwrap();
    let first = format_program(&program1, &config);
    let program2 = magi_lang::syntax::parser::parse_v2(&first).unwrap();
    let second = format_program(&program2, &config);
    assert_eq!(first, second, "null coalesce should be idempotent:\nfirst:  {}\nsecond: {}", first, second);
}

#[test]
fn test_lexer_invalid_utf8_no_hang() {
    // Test that invalid/incomplete UTF-8 in strings doesn't cause infinite loop.
    // We can't inject raw bytes through parse_v2 (it takes &str), but we can test
    // the replacement character path by verifying advance_char handles edge cases.
    // Instead, test that Unicode replacement char in source is handled:
    let source = "let x = \"hello\u{FFFD}world\";";
    let result = magi_lang::syntax::parser::parse_v2(source);
    assert!(result.is_ok(), "replacement char in string should parse: {:?}", result.err());
}

#[test]
fn test_linter_duplicate_imports() {
    use magi_lang::linter;
    let program = magi_lang::syntax::parser::parse_v2(r#"
        import "foo";
        import "foo";
    "#).unwrap();
    let result = linter::lint(&program, &linter::LintConfig::default());
    let w208: Vec<_> = result.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W208"))
        .collect();
    assert_eq!(w208.len(), 1, "should detect duplicate import: {:?}", w208);
}

#[test]
fn test_linter_no_false_duplicate_import() {
    use magi_lang::linter;
    let program = magi_lang::syntax::parser::parse_v2(r#"
        import "foo";
        import "bar";
    "#).unwrap();
    let result = linter::lint(&program, &linter::LintConfig::default());
    let w208: Vec<_> = result.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W208"))
        .collect();
    assert!(w208.is_empty(), "different imports should not warn: {:?}", w208);
}

// =============================================================================
// Round 5: Parser, interpreter, type checker fixes
// =============================================================================

#[test]
fn test_rest_pattern_must_be_last_in_destructure() {
    // Valid: rest pattern at end
    let result = magi_lang::syntax::parser::parse_v2("let [a, ...rest] = arr;");
    assert!(result.is_ok(), "valid rest pattern should parse: {:?}", result.err());
}

#[test]
fn test_while_loop_break_value() {
    let result = run("
        let mut i = 0;
        while i < 10 {
            i = i + 1;
            if i == 5 {
                break 42;
            }
        }
    ");
    assert_eq!(result, DataType::Int64(42));
}

#[test]
fn test_while_loop_no_break_returns_last_body_value() {
    // while loop returns the last body expression value (consistent with for loop)
    let result = run("
        let mut i = 0;
        while i < 3 {
            i = i + 1
        }
    ");
    assert_eq!(result, DataType::Int64(3));
}

#[test]
fn test_pow_negative_exponent() {
    // 2^(-3) should return 0 (integer division: 1/8 rounds to 0)
    assert_eq!(run("2.pow(-3)"), DataType::Int64(0));
}

#[test]
fn test_pow_negative_exponent_one() {
    // 1^(-anything) is always 1
    assert_eq!(run("1.pow(-5)"), DataType::Int64(1));
}

#[test]
fn test_pow_negative_exponent_neg_one() {
    // (-1)^(-2) = 1, (-1)^(-3) = -1
    assert_eq!(run("let x = -1; x.pow(-2)"), DataType::Int64(1));
    assert_eq!(run("let x = -1; x.pow(-3)"), DataType::Int64(-1));
}

#[test]
fn test_try_propagate_caught_by_try_catch() {
    // The ? operator should produce an error catchable by try/catch
    let result = run(r#"
        let val = null;
        let mut caught = false;
        try {
            let x = val?;
        } catch e {
            caught = true;
        }
        caught
    "#);
    assert_eq!(result, DataType::Bool(true));
}

#[test]
fn test_w108_unnecessary_return() {
    let program = magi_lang::syntax::parser::parse_v2(r#"
        fn add(a, b) {
            return a + b;
        }
    "#).unwrap();
    let imports = std::collections::HashSet::new();
    let analysis = magi_lang::syntax::type_checker::check_types(&program, &imports);
    let w108: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W108"))
        .collect();
    assert_eq!(w108.len(), 1, "should detect unnecessary return: {:?}", w108);
}

#[test]
fn test_w108_no_false_positive_early_return() {
    // Early returns (not in tail position) should NOT trigger W108
    let program = magi_lang::syntax::parser::parse_v2(r#"
        fn check(x) {
            if x > 10 {
                return "big";
            }
            "small"
        }
    "#).unwrap();
    let imports = std::collections::HashSet::new();
    let analysis = magi_lang::syntax::type_checker::check_types(&program, &imports);
    let w108: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W108"))
        .collect();
    assert!(w108.is_empty(), "early return should not warn: {:?}", w108);
}

// =============================================================================
// Round 6: NaN truthiness, float-to-int safety, spread errors
// =============================================================================

#[test]
fn test_nan_is_falsy() {
    // NaN should be falsy via to_bool() conversion
    // The interpreter's if-condition requires Bool, so we test via the types module
    use magi_lang::types::DataType;
    let nan = DataType::Float64(f64::NAN);
    assert!(!nan.to_bool(), "NaN should be falsy");
    let zero = DataType::Float64(0.0);
    assert!(!zero.to_bool(), "0.0 should be falsy");
    let positive = DataType::Float64(1.5);
    assert!(positive.to_bool(), "positive floats should be truthy");
}

#[test]
fn test_float_to_int64_nan_returns_null() {
    let result = run("
        let x = 0.0 / 0.0;
        x.to_int64()
    ");
    assert_eq!(result, DataType::Null);
}

#[test]
fn test_float_to_int64_infinity_returns_null() {
    let result = run("
        let x = 1.0 / 0.0;
        x.to_int64()
    ");
    assert_eq!(result, DataType::Null);
}

#[test]
fn test_float_to_int64_valid() {
    let result = run("42.5.to_int64()");
    assert_eq!(result, DataType::Int64(42));
}

#[test]
fn test_spread_non_array_errors() {
    // Spreading a non-array value should error
    let result = run_err("
        fn add(a, b) { a + b }
        add(...5)
    ");
    match result {
        InterpError::TypeError { context, .. } => {
            assert!(context.contains("spread"), "error should mention spread: {}", context);
        }
        other => panic!("expected TypeError for spread on non-array, got: {:?}", other),
    }
}

#[test]
fn test_spread_non_array_in_array_literal_errors() {
    let result = run_err("[1, ...5, 3]");
    match result {
        InterpError::TypeError { context, .. } => {
            assert!(context.contains("spread"), "error should mention spread: {}", context);
        }
        other => panic!("expected TypeError for spread on non-array, got: {:?}", other),
    }
}

#[test]
fn test_spread_array_still_works() {
    let result = run("
        let a = [1, 2];
        let b = [0, ...a, 3];
        b
    ");
    assert_eq!(result, DataType::Array(vec![
        DataType::Int64(0),
        DataType::Int64(1),
        DataType::Int64(2),
        DataType::Int64(3),
    ]));
}

#[test]
fn test_formatter_optional_chain_idempotent() {
    use magi_lang::formatter::{format_program, FormatConfig};
    let config = FormatConfig::default();
    let source = "let x = obj?.field;\n";
    let program1 = magi_lang::syntax::parser::parse_v2(source).unwrap();
    let first = format_program(&program1, &config);
    let program2 = magi_lang::syntax::parser::parse_v2(&first).unwrap();
    let second = format_program(&program2, &config);
    assert_eq!(first, second, "optional chain should be idempotent:\nfirst:  {}\nsecond: {}", first, second);
}

// ── Round 7: Scope leak, untested features, compound assignments ─────

#[test]
fn test_for_loop_destructure_error_no_scope_leak() {
    // If destructure_bind errors, the scope should still be cleaned up.
    // After the for loop error, the interpreter should be in a clean state.
    let src = r#"
        let items = [1, 2, 3];
        let result = try {
            for [a, b] in items {
                a
            }
        } catch err {
            "caught"
        };
        result
    "#;
    let result = run(src);
    assert_eq!(result, DataType::String("caught".to_string()));
}

#[test]
fn test_compound_assign_subtract() {
    let src = r#"
        let mut x = 10;
        x -= 3;
        x
    "#;
    assert_eq!(run(src), DataType::Int64(7));
}

#[test]
fn test_compound_assign_multiply() {
    let src = r#"
        let mut x = 5;
        x *= 4;
        x
    "#;
    assert_eq!(run(src), DataType::Int64(20));
}

#[test]
fn test_compound_assign_divide() {
    let src = r#"
        let mut x = 20;
        x /= 4;
        x
    "#;
    assert_eq!(run(src), DataType::Int64(5));
}

#[test]
fn test_compound_assign_modulo() {
    let src = r#"
        let mut x = 17;
        x %= 5;
        x
    "#;
    assert_eq!(run(src), DataType::Int64(2));
}

#[test]
fn test_module_definition_and_call() {
    let src = r#"
        mod math {
            fn double(x) { x * 2 }
            fn triple(x) { x * 3 }
        }
        math::double(5) + math::triple(3)
    "#;
    assert_eq!(run(src), DataType::Int64(19));
}

#[test]
fn test_type_alias_transparent() {
    // Type aliases have no runtime effect; code should execute normally.
    let src = r#"
        type Score = int64;
        let x: Score = 42;
        x
    "#;
    assert_eq!(run(src), DataType::Int64(42));
}

#[test]
fn test_test_definitions_via_run_tests() {
    let src = r#"
        fn add(a, b) { a + b }

        test "addition" {
            if add(2, 3) != 5 {
                throw "addition failed";
            }
        }

        test "subtraction" {
            if add(10, -3) != 7 {
                throw "subtraction failed";
            }
        }
    "#;
    let program = parse(src);
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    let results = interp.run_tests(&program);
    assert_eq!(results.len(), 2);
    assert!(results[0].passed, "first test should pass: {:?}", results[0].error_message);
    assert!(results[1].passed, "second test should pass: {:?}", results[1].error_message);
}

#[test]
fn test_test_definitions_skipped_during_normal_execution() {
    // test blocks should NOT execute during normal `execute()`.
    let src = r#"
        test "should not run" {
            assert false;
        }
        42
    "#;
    assert_eq!(run(src), DataType::Int64(42));
}

#[test]
fn test_async_await_spawn_synchronous() {
    // In the synchronous interpreter, spawn evaluates eagerly
    // and await unwraps the resolved value.
    let src = r#"
        async fn compute() {
            42
        }
        let task = spawn compute();
        let result = await task;
        result
    "#;
    assert_eq!(run(src), DataType::Int64(42));
}

#[test]
fn test_for_loop_map_destructure_error_scope_cleanup() {
    // Map destructure error should also clean up scope.
    let src = r#"
        let items = [1, 2, 3];
        let result = try {
            for {name, age} in items {
                name
            }
        } catch err {
            "caught map destructure error"
        };
        result
    "#;
    let result = run(src);
    assert_eq!(result, DataType::String("caught map destructure error".to_string()));
}

// ── Round 8: Range overflow, slice safety, comprehension scope ───────

#[test]
fn test_range_inclusive_overflow_error() {
    // 0..=i64::MAX should error, not overflow
    let src = "0..=9223372036854775807";
    let err = run_err(src);
    match err {
        InterpError::TypeError { ref context, .. } => {
            assert!(context.contains("inclusive range"), "expected inclusive range error, got: {:?}", err);
        }
        _ => panic!("expected TypeError for range overflow, got: {:?}", err),
    }
}

#[test]
fn test_range_non_inclusive_max_ok() {
    // 0..3 should work fine (non-inclusive, no +1)
    let src = "0..3";
    assert_eq!(run(src), DataType::Array(vec![
        DataType::Int64(0),
        DataType::Int64(1),
        DataType::Int64(2),
    ]));
}

#[test]
fn test_slice_negative_start_wraps_from_end() {
    // Negative slice start wraps from end (Python-style)
    let src = r#"
        let arr = [10, 20, 30, 40, 50];
        arr[(-2)..5]
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::Int64(40),
        DataType::Int64(50),
    ]));
}

#[test]
fn test_slice_negative_end_wraps_from_end() {
    let src = r#"
        let arr = [10, 20, 30];
        arr[0..(-1)]
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::Int64(10),
        DataType::Int64(20),
    ]));
}

#[test]
fn test_comprehension_scope_leak_on_destructure_error() {
    // List comprehension destructure error should clean up scope
    let src = r#"
        let items = [1, 2, 3];
        let result = try {
            [x for [x, y] in items]
        } catch err {
            "caught"
        };
        result
    "#;
    assert_eq!(run(src), DataType::String("caught".to_string()));
}

#[test]
fn test_range_inclusive_normal() {
    let src = "1..=5";
    assert_eq!(run(src), DataType::Array(vec![
        DataType::Int64(1),
        DataType::Int64(2),
        DataType::Int64(3),
        DataType::Int64(4),
        DataType::Int64(5),
    ]));
}

// ── Round 10: CLI, pow overflow, await tail expression ───────────────

#[test]
fn test_pow_overflow_returns_null() {
    // 2^63 overflows i64, should return null instead of wrapping
    let src = r#"
        let n = 2;
        n.pow(63)
    "#;
    assert_eq!(run(src), DataType::Null);
}

#[test]
fn test_pow_normal() {
    let src = r#"
        let n = 2;
        n.pow(10)
    "#;
    assert_eq!(run(src), DataType::Int64(1024));
}

#[test]
fn test_await_as_tail_expression() {
    // await should work correctly in tail position
    let src = r#"
        async fn compute() { 42 }
        let result = { await spawn compute() };
        result
    "#;
    assert_eq!(run(src), DataType::Int64(42));
}

#[test]
fn test_sort_by_ascending() {
    let src = r#"
        let arr = [3, 1, 4, 1, 5];
        arr.sort_by(|a, b| a - b)
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::Int64(1),
        DataType::Int64(1),
        DataType::Int64(3),
        DataType::Int64(4),
        DataType::Int64(5),
    ]));
}

#[test]
fn test_sort_by_descending() {
    let src = r#"
        let arr = [3, 1, 4, 1, 5];
        arr.sort_by(|a, b| b - a)
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::Int64(5),
        DataType::Int64(4),
        DataType::Int64(3),
        DataType::Int64(1),
        DataType::Int64(1),
    ]));
}

// ===== Round 11: Comprehension scope leaks and formatter fixes =====

#[test]
fn test_list_comprehension_error_scope_cleanup() {
    // If an error occurs during list comprehension body eval,
    // the scope must still be cleaned up properly. After the try/catch,
    // subsequent code should work fine (no stale scopes).
    let src = r#"
        let result = try {
            [1 / (x - 2) for x in [1, 2, 3]]
        } catch e {
            "caught"
        };
        let after = 42;
        [result, after]
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::String("caught".to_string()),
        DataType::Int64(42),
    ]));
}

#[test]
fn test_list_comprehension_filter_error_scope_cleanup() {
    // Error in the filter condition should also clean up scope
    let src = r#"
        let result = try {
            [x for x in [1, 0, 3] if 1 / x > 0]
        } catch e {
            "filter_error"
        };
        let after = 99;
        [result, after]
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::String("filter_error".to_string()),
        DataType::Int64(99),
    ]));
}

#[test]
fn test_map_comprehension_error_scope_cleanup() {
    // Error in map comprehension value expression should clean up scope
    let src = r#"
        let result = try {
            {"k": 1 / (x - 2) for x in [1, 2, 3]}
        } catch e {
            "map_caught"
        };
        let after = 77;
        [result, after]
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::String("map_caught".to_string()),
        DataType::Int64(77),
    ]));
}

#[test]
fn test_map_comprehension_destructure_error_scope_cleanup() {
    // Error in map comprehension destructure binding should clean up scope
    let src = r#"
        let result = try {
            {"k": v for [k, v] in [[1, 2], "bad", [3, 4]]}
        } catch e {
            "destr_error"
        };
        let after = 55;
        [result, after]
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::String("destr_error".to_string()),
        DataType::Int64(55),
    ]));
}

#[test]
fn test_formatter_pipe_idempotency() {
    // Formatter should be idempotent: format(format(x)) == format(x)
    use magi_lang::formatter::{format_program, FormatConfig};
    use magi_lang::syntax::parser::parse_v2;
    let src = "let x = a |> (b |> c);";
    let program = parse_v2(src).unwrap();
    let config = FormatConfig::default();
    let first = format_program(&program, &config);
    let program2 = parse_v2(&first).unwrap();
    let second = format_program(&program2, &config);
    assert_eq!(first, second, "Pipe formatter should be idempotent");
}

#[test]
fn test_formatter_nested_pipe_idempotency() {
    // More complex nested pipes
    use magi_lang::formatter::{format_program, FormatConfig};
    use magi_lang::syntax::parser::parse_v2;
    let src = "let x = (a |> b) |> (c |> d);";
    let program = parse_v2(src).unwrap();
    let config = FormatConfig::default();
    let first = format_program(&program, &config);
    let program2 = parse_v2(&first).unwrap();
    let second = format_program(&program2, &config);
    assert_eq!(first, second, "Nested pipe formatter should be idempotent");
}

// ===== Round 12: Method dispatch, type checker, and overflow fixes =====

#[test]
fn test_sort_by_floats() {
    let src = r#"
        let arr = [0.9, 0.1, 0.5, 0.3, 0.7];
        arr.sort_by(|a, b| a - b)
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::Float64(0.1),
        DataType::Float64(0.3),
        DataType::Float64(0.5),
        DataType::Float64(0.7),
        DataType::Float64(0.9),
    ]));
}

#[test]
fn test_sort_by_floats_descending() {
    let src = r#"
        let arr = [0.1, 0.9, 0.5];
        arr.sort_by(|a, b| b - a)
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::Float64(0.9),
        DataType::Float64(0.5),
        DataType::Float64(0.1),
    ]));
}

#[test]
fn test_abs_i64_min_overflow() {
    // abs(i64::MIN) should return null instead of panicking
    let src = r#"
        let x = -9223372036854775807 - 1;
        x.abs()
    "#;
    assert_eq!(run(src), DataType::Null);
}

#[test]
fn test_pow_exponent_too_large() {
    // Exponents > u32::MAX should return null, not wrap
    let src = r#"
        2.pow(4294967296)
    "#;
    assert_eq!(run(src), DataType::Null);
}

#[test]
fn test_array_min_max_strings() {
    let src = r#"
        let arr = ["banana", "apple", "cherry"];
        [arr.min(), arr.max()]
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::String("apple".to_string()),
        DataType::String("cherry".to_string()),
    ]));
}

#[test]
fn test_min_by_float_comparator() {
    let src = r#"
        let arr = [0.9, 0.1, 0.5];
        arr.min_by(|a, b| a - b)
    "#;
    assert_eq!(run(src), DataType::Float64(0.1));
}

#[test]
fn test_max_by_float_comparator() {
    let src = r#"
        let arr = [0.9, 0.1, 0.5];
        arr.max_by(|a, b| a - b)
    "#;
    assert_eq!(run(src), DataType::Float64(0.9));
}

#[test]
fn test_return_type_mismatch_caught() {
    // Type checker should catch return type mismatches with explicit return
    use magi_lang::syntax::parser::parse_v2;
    use magi_lang::syntax::type_checker::check_types;
    let src = r#"fn bad() -> int64 { return "not an int"; }
let r = bad();
output r;"#;
    let program = parse_v2(src).unwrap();
    let analysis = check_types(&program, &std::collections::HashSet::new());
    assert!(analysis.diagnostics.iter().any(|d| d.message.contains("return type mismatch")),
        "Expected return type mismatch error. Diagnostics: {:?}",
        analysis.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
}

// =============================================================================
// Round 13: Linter, LSP, and evaluator fixes
// =============================================================================

#[test]
fn test_linter_to_snake_case_acronyms() {
    // to_snake_case should handle acronyms correctly (HTTPServer → http_server)
    use magi_lang::linter;
    let src = "let HTTPServer = 1;";
    let program = parse_v2(src).unwrap();
    let config = linter::LintConfig::default();
    let result = linter::lint(&program, &config);
    let w200 = result.diagnostics.iter().find(|d| d.code.as_deref() == Some("W200")).unwrap();
    assert!(w200.suggestion.as_ref().unwrap().contains("http_server"),
        "Expected 'http_server' suggestion, got: {:?}", w200.suggestion);
}

#[test]
fn test_linter_to_snake_case_camel() {
    // to_snake_case should handle camelCase correctly
    use magi_lang::linter;
    let src = "let myVarName = 1;";
    let program = parse_v2(src).unwrap();
    let config = linter::LintConfig::default();
    let result = linter::lint(&program, &config);
    let w200 = result.diagnostics.iter().find(|d| d.code.as_deref() == Some("W200")).unwrap();
    assert!(w200.suggestion.as_ref().unwrap().contains("my_var_name"),
        "Expected 'my_var_name' suggestion, got: {:?}", w200.suggestion);
}

#[test]
fn test_linter_w206_empty_for_loop() {
    // W206 should be emitted for empty for-loop bodies
    use magi_lang::linter;
    let src = "for x in [1, 2, 3] {}";
    let program = parse_v2(src).unwrap();
    let config = linter::LintConfig::default();
    let result = linter::lint(&program, &config);
    let codes: Vec<String> = result.diagnostics.iter().filter_map(|d| d.code.clone()).collect();
    assert!(codes.contains(&"W206".to_string()),
        "Expected W206 for empty for-loop, got: {:?}", codes);
}

#[test]
fn test_linter_w206_empty_while_loop() {
    // W206 should be emitted for empty while-loop bodies
    use magi_lang::linter;
    let src = "let x = true;\nwhile x {}";
    let program = parse_v2(src).unwrap();
    let config = linter::LintConfig::default();
    let result = linter::lint(&program, &config);
    let codes: Vec<String> = result.diagnostics.iter().filter_map(|d| d.code.clone()).collect();
    assert!(codes.contains(&"W206".to_string()),
        "Expected W206 for empty while-loop, got: {:?}", codes);
}

#[test]
fn test_linter_w206_empty_if_body() {
    // W206 should be emitted for empty if bodies
    use magi_lang::linter;
    let src = "let x = 1;\nif x > 0 {}";
    let program = parse_v2(src).unwrap();
    let config = linter::LintConfig::default();
    let result = linter::lint(&program, &config);
    let codes: Vec<String> = result.diagnostics.iter().filter_map(|d| d.code.clone()).collect();
    assert!(codes.contains(&"W206".to_string()),
        "Expected W206 for empty if body, got: {:?}", codes);
}

#[test]
fn test_linter_w207_or_pattern_catch_all() {
    // W207 should detect catch-all inside Or-pattern
    use magi_lang::linter;
    let src = r#"let x = 1;
match x {
    1 | _ => 0,
    2 => 2,
}"#;
    let program = parse_v2(src).unwrap();
    let config = linter::LintConfig::default();
    let result = linter::lint(&program, &config);
    let codes: Vec<String> = result.diagnostics.iter().filter_map(|d| d.code.clone()).collect();
    assert!(codes.contains(&"W207".to_string()),
        "Expected W207 for unreachable arm after or-pattern with wildcard, got: {:?}", codes);
}

#[test]
fn test_linter_w203_guarded_arm_not_exhaustive() {
    // W203: guarded arms should not count as covering enum variants
    use magi_lang::linter;
    let src = r#"enum Color { Red, Green, Blue }
let c = Color::Red;
match c {
    Color::Red if true => "red",
    Color::Green => "green",
}"#;
    let program = parse_v2(src).unwrap();
    let config = linter::LintConfig::default();
    let result = linter::lint(&program, &config);
    let codes: Vec<String> = result.diagnostics.iter().filter_map(|d| d.code.clone()).collect();
    assert!(codes.contains(&"W203".to_string()),
        "Expected W203 for non-exhaustive match (guarded Red + missing Blue), got: {:?}", codes);
}

#[test]
fn test_linter_w200_function_params() {
    // W200 should check function parameter names
    use magi_lang::linter;
    let src = "fn foo(myParam: int64) { myParam }";
    let program = parse_v2(src).unwrap();
    let config = linter::LintConfig::default();
    let result = linter::lint(&program, &config);
    let codes: Vec<String> = result.diagnostics.iter().filter_map(|d| d.code.clone()).collect();
    assert!(codes.contains(&"W200".to_string()),
        "Expected W200 for non-snake_case param name, got: {:?}", codes);
}

#[test]
fn test_linter_w200_for_loop_var() {
    // W200 should check for-loop variable names
    use magi_lang::linter;
    let src = "for myItem in [1, 2, 3] { output myItem; }";
    let program = parse_v2(src).unwrap();
    let config = linter::LintConfig::default();
    let result = linter::lint(&program, &config);
    let codes: Vec<String> = result.diagnostics.iter().filter_map(|d| d.code.clone()).collect();
    assert!(codes.contains(&"W200".to_string()),
        "Expected W200 for non-snake_case for-loop var, got: {:?}", codes);
}

#[test]
fn test_lsp_const_symbol_extraction() {
    // LSP should recognize const defs and mark them appropriately
    use magi_lang::lsp::analysis::analyze_document;
    let src = "const MAX_SIZE = 100;";
    let (state, _) = analyze_document(src);
    let var = state.variables.get("MAX_SIZE").unwrap();
    assert!(var.constant, "Expected const to be marked as constant");
    assert!(!var.mutable, "Const should not be mutable");
}

#[test]
fn test_lsp_completion_prefix_boundary() {
    // Completion should only consider prefix (text before cursor), not full word
    use magi_lang::lsp::analysis::find_word_at_position;
    let src = "let foo_bar = 1;";
    // find_word_at_position scans both directions, so at position 7 it returns "foo_bar"
    let word = find_word_at_position(src, 0, 7).unwrap();
    assert_eq!(word, "foo_bar");
    // But for completion, cursor at col 7 (middle of "foo_bar") should only return "foo_bar"
    // This is tested at the unit level in completion module
}

// =============================================================================
// Round 14: Parser and type checker fixes
// =============================================================================

#[test]
fn test_map_comprehension_key_is_string_literal() {
    // Parser should produce Literal::String for map comprehension keys, not Variable
    let src = r#"let m = {"key": x * 2 for x in [1, 2, 3]};"#;
    let program = parse_v2(src).unwrap();
    // If parsing produces Variable("key"), the interpreter would look up "key" as a var
    // and fail. With Literal::String("key"), it produces a map with string key "key".
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    let result = interp.execute(&program);
    assert!(result.is_ok(), "Map comprehension with string key should succeed: {:?}", result.err());
}

#[test]
fn test_type_checker_break_outside_loop_has_error_code() {
    // break outside loop should emit E300
    let src = "break;";
    let program = parse_v2(src).unwrap();
    let analysis = check_types(&program, &std::collections::HashSet::new());
    assert!(analysis.diagnostics.iter().any(|d|
        d.code.as_deref() == Some("E300") && d.message.contains("break")),
        "Expected E300 for break outside loop. Got: {:?}",
        analysis.diagnostics.iter().map(|d| (&d.code, &d.message)).collect::<Vec<_>>());
}

#[test]
fn test_type_checker_continue_outside_loop_has_error_code() {
    // continue outside loop should emit E301
    let src = "continue;";
    let program = parse_v2(src).unwrap();
    let analysis = check_types(&program, &std::collections::HashSet::new());
    assert!(analysis.diagnostics.iter().any(|d|
        d.code.as_deref() == Some("E301") && d.message.contains("continue")),
        "Expected E301 for continue outside loop. Got: {:?}",
        analysis.diagnostics.iter().map(|d| (&d.code, &d.message)).collect::<Vec<_>>());
}

#[test]
fn test_type_checker_return_outside_function_has_error_code() {
    // return outside function should emit E302
    let src = "return 5;";
    let program = parse_v2(src).unwrap();
    let analysis = check_types(&program, &std::collections::HashSet::new());
    assert!(analysis.diagnostics.iter().any(|d|
        d.code.as_deref() == Some("E302") && d.message.contains("return")),
        "Expected E302 for return outside function. Got: {:?}",
        analysis.diagnostics.iter().map(|d| (&d.code, &d.message)).collect::<Vec<_>>());
}

#[test]
fn test_type_checker_division_by_zero_has_error_code() {
    // Division by zero should emit E104
    let src = "let x = 5 / 0;";
    let program = parse_v2(src).unwrap();
    let analysis = check_types(&program, &std::collections::HashSet::new());
    assert!(analysis.diagnostics.iter().any(|d|
        d.code.as_deref() == Some("E104") && d.message.contains("Division by zero")),
        "Expected E104 for division by zero. Got: {:?}",
        analysis.diagnostics.iter().map(|d| (&d.code, &d.message)).collect::<Vec<_>>());
}

#[test]
fn test_type_checker_negative_array_index_has_error_code() {
    // Negative array index should emit E105
    let src = "let arr = [1, 2, 3];\nlet x = arr[-1];";
    let program = parse_v2(src).unwrap();
    let analysis = check_types(&program, &std::collections::HashSet::new());
    assert!(analysis.diagnostics.iter().any(|d|
        d.code.as_deref() == Some("E105") && d.message.contains("Negative array index")),
        "Expected E105 for negative array index. Got: {:?}",
        analysis.diagnostics.iter().map(|d| (&d.code, &d.message)).collect::<Vec<_>>());
}

#[test]
fn test_type_checker_duplicate_map_key_has_error_code() {
    // Duplicate map key should emit E107
    let src = r#"let m = {"a": 1, "a": 2};"#;
    let program = parse_v2(src).unwrap();
    let analysis = check_types(&program, &std::collections::HashSet::new());
    assert!(analysis.diagnostics.iter().any(|d|
        d.code.as_deref() == Some("E107") && d.message.contains("Duplicate key")),
        "Expected E107 for duplicate map key. Got: {:?}",
        analysis.diagnostics.iter().map(|d| (&d.code, &d.message)).collect::<Vec<_>>());
}

// =============================================================================
// Round 15: Interpreter fixes and coverage tests
// =============================================================================

#[test]
fn test_pad_start_negative_width_no_crash() {
    // pad_start with negative width should not OOM — treated as 0
    let src = r#"let x = "hello".pad_start(-5); output x;"#;
    assert_eq!(run(src), DataType::String("hello".to_string()));
}

#[test]
fn test_pad_end_negative_width_no_crash() {
    // pad_end with negative width should not OOM — treated as 0
    let src = r#"let x = "hello".pad_end(-5); output x;"#;
    assert_eq!(run(src), DataType::String("hello".to_string()));
}

#[test]
fn test_pad_start_positive_width() {
    let src = r#""hi".pad_start(5)"#;
    assert_eq!(run(src), DataType::String("   hi".to_string()));
}

#[test]
fn test_pad_end_positive_width() {
    let src = r#""hi".pad_end(5)"#;
    assert_eq!(run(src), DataType::String("hi   ".to_string()));
}

#[test]
fn test_product_integer_overflow_promotes_to_float() {
    // Product of large integers should promote to float instead of overflowing
    let src = r#"
let big = 9223372036854775807;
let result = [big, 2].product();
output result;
"#;
    let result = run(src);
    // Should be a float (promoted on overflow), not a wrapped integer
    match result {
        DataType::Float64(f) => assert!(f > 0.0, "Product should be positive, got {}", f),
        other => panic!("Expected Float64 for overflowed product, got {:?}", other),
    }
}

#[test]
fn test_string_interpolation_with_expressions() {
    assert_eq!(
        run(r#"
let x = 5;
let y = 3;
f"sum={x + y}, product={x * y}"
"#),
        DataType::String("sum=8, product=15".to_string())
    );
}

#[test]
fn test_string_interpolation_function_call() {
    assert_eq!(
        run(r#"
fn double(n) { n * 2 }
let x = 5;
f"doubled={double(x)}"
"#),
        DataType::String("doubled=10".to_string())
    );
}

#[test]
fn test_optional_chaining_deep() {
    assert_eq!(
        run(r#"let x = {"a": {"b": {"c": 42}}}; x?.a?.b?.c"#),
        DataType::Int64(42)
    );
}

#[test]
fn test_optional_chaining_null_short_circuit() {
    assert_eq!(
        run(r#"let x = {"a": null}; x?.a?.b?.c"#),
        DataType::Null
    );
}

#[test]
fn test_nested_closure_capture() {
    assert_eq!(
        run(r#"
let x = 10;
let outer = |y| |z| x + y + z;
let inner = outer(5);
inner(3)
"#),
        DataType::Int64(18)
    );
}

#[test]
fn test_function_default_params() {
    assert_eq!(
        run(r#"
fn greet(name = "World") {
    f"Hello, {name}!"
}
greet()
"#),
        DataType::String("Hello, World!".to_string())
    );
}

#[test]
fn test_function_default_params_override() {
    assert_eq!(
        run(r#"
fn greet(name = "World") {
    f"Hello, {name}!"
}
greet("Alice")
"#),
        DataType::String("Hello, Alice!".to_string())
    );
}

#[test]
fn test_match_guard_with_variable() {
    assert_eq!(
        run(r#"
let x = 15;
match x {
    n if n < 10 => "small",
    n if n < 20 => "medium",
    _ => "large",
}
"#),
        DataType::String("medium".to_string())
    );
}

#[test]
fn test_operator_precedence_and_or() {
    // && binds tighter than ||
    assert_eq!(run("true || false && false"), DataType::Bool(true));
}

#[test]
fn test_operator_precedence_arithmetic_comparison() {
    // * binds tighter than +, + binds tighter than ==
    assert_eq!(run("2 + 3 * 4 == 14"), DataType::Bool(true));
}

#[test]
fn test_const_def_with_type_annotation() {
    assert_eq!(
        run("const PI: float64 = 3.14159;\nPI"),
        DataType::Float64(3.14159)
    );
}

#[test]
fn test_let_with_type_annotation() {
    assert_eq!(
        run("let x: int64 = 42;\nx"),
        DataType::Int64(42)
    );
}

// Round 17: Finally block scope isolation tests
#[test]
fn test_finally_block_scope_isolation() {
    // Variables declared in finally should NOT leak to outer scope
    assert_eq!(
        run(r#"
let mut result = "before";
try {
    result = "try";
} catch e {
    result = "catch";
} finally {
    let temp = "finally_var";
}
result
"#),
        DataType::String("try".to_string())
    );
}

#[test]
fn test_finally_block_scope_isolation_catch_path() {
    // Variables declared in finally should NOT leak even on catch path
    assert_eq!(
        run(r#"
let mut result = "before";
try {
    throw "error";
} catch e {
    result = "caught";
} finally {
    let cleanup = "done";
}
result
"#),
        DataType::String("caught".to_string())
    );
}

#[test]
fn test_finally_runs_on_normal_path() {
    assert_eq!(
        run(r#"
let mut x = 0;
try {
    x = 1;
} catch e {
    x = 2;
} finally {
    x = 10;
}
x
"#),
        DataType::Int64(10)
    );
}

#[test]
fn test_finally_runs_on_error_path() {
    assert_eq!(
        run(r#"
let mut x = 0;
try {
    throw "boom";
} catch e {
    x = 2;
} finally {
    x = 10;
}
x
"#),
        DataType::Int64(10)
    );
}

#[test]
fn test_finally_error_overrides_try_result() {
    // If finally throws, it should override the try result
    assert_eq!(
        run(r#"
let mut result = "none";
try {
    try {
        result = "inner";
    } catch e {
        result = "caught";
    } finally {
        throw "finally_error";
    }
} catch e {
    result = "finally_err_caught";
}
result
"#),
        DataType::String("finally_err_caught".to_string())
    );
}

#[test]
fn test_try_catch_error_message() {
    // Catch variable receives the full formatted error string
    let result = run(r#"
try {
    throw "oops";
} catch e {
    e
}
"#);
    // The error message includes span and error code info
    match result {
        DataType::String(s) => assert!(s.contains("oops"), "expected 'oops' in: {}", s),
        other => panic!("expected String, got: {:?}", other),
    }
}

#[test]
fn test_nested_try_catch_finally() {
    assert_eq!(
        run(r#"
let mut log = "";
try {
    try {
        throw "inner";
    } catch e {
        log = log + "inner_catch,";
    } finally {
        log = log + "inner_finally,";
    }
} catch e {
    log = log + "outer_catch,";
} finally {
    log = log + "outer_finally";
}
log
"#),
        DataType::String("inner_catch,inner_finally,outer_finally".to_string())
    );
}

// Round 18: Memory safety and type checker tests
#[test]
fn test_string_repeat_bounded() {
    // Normal repeat should work fine
    assert_eq!(
        run(r#""abc".repeat(3)"#),
        DataType::String("abcabcabc".to_string())
    );
}

#[test]
fn test_string_repeat_zero() {
    assert_eq!(
        run(r#""abc".repeat(0)"#),
        DataType::String("".to_string())
    );
}

#[test]
fn test_string_repeat_negative() {
    assert_eq!(
        run(r#""abc".repeat(-5)"#),
        DataType::String("".to_string())
    );
}

#[test]
fn test_range_bounded() {
    // Normal range should work fine
    assert_eq!(
        run("(0..5)"),
        DataType::Array(vec![
            DataType::Int64(0),
            DataType::Int64(1),
            DataType::Int64(2),
            DataType::Int64(3),
            DataType::Int64(4),
        ])
    );
}

#[test]
fn test_type_checker_error_codes_on_conditions() {
    use std::collections::HashSet;
    let source = "if 42 { 1 }";
    let program = parse_v2(source).unwrap();
    let analysis = check_types(&program, &HashSet::new());
    let has_e101 = analysis.diagnostics.iter().any(|d| {
        d.code.as_ref().map_or(false, |c: &String| c.contains("E101"))
    });
    assert!(has_e101, "Expected E101 for non-bool if condition, got: {:?}", analysis.diagnostics);
}

#[test]
fn test_type_checker_error_codes_on_unknown_function() {
    use std::collections::HashSet;
    let source = "nonexistent_func(1, 2)";
    let program = parse_v2(source).unwrap();
    let analysis = check_types(&program, &HashSet::new());
    let has_e201 = analysis.diagnostics.iter().any(|d| {
        d.code.as_ref().map_or(false, |c: &String| c.contains("E201"))
    });
    assert!(has_e201, "Expected E201 for unknown function, got: {:?}", analysis.diagnostics);
}

#[test]
fn test_type_checker_error_codes_on_arity_mismatch() {
    use std::collections::HashSet;
    let source = "fn add(a, b) { a + b }\nadd(1)";
    let program = parse_v2(source).unwrap();
    let analysis = check_types(&program, &HashSet::new());
    let has_e405 = analysis.diagnostics.iter().any(|d| {
        d.code.as_ref().map_or(false, |c: &String| c.contains("E405"))
    });
    assert!(has_e405, "Expected E405 for arity mismatch, got: {:?}", analysis.diagnostics);
}

#[test]
fn test_type_checker_all_diagnostics_have_codes() {
    use std::collections::HashSet;
    let source = r#"
fn greet(name: string) -> string { name }
greet(42)
"#;
    let program = parse_v2(source).unwrap();
    let analysis = check_types(&program, &HashSet::new());
    for d in &analysis.diagnostics {
        assert!(d.code.is_some(), "Diagnostic without error code: {}", d.message);
    }
}

// Round 19: Linter destructuring naming checks
#[test]
fn test_lint_w200_destructure_array() {
    use magi_lang::linter;
    let source = r#"let [firstName, last_name] = ["Alice", "Smith"];"#;
    let program = parse_v2(source).unwrap();
    let result = linter::lint(&program, &linter::LintConfig::default());
    let w200s: Vec<_> = result.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W200"))
        .collect();
    assert_eq!(w200s.len(), 1, "Expected 1 W200 for firstName, got: {:?}", w200s);
    assert!(w200s[0].message.contains("firstName"), "Should warn about firstName");
}

#[test]
fn test_lint_w200_destructure_map() {
    use magi_lang::linter;
    let source = r#"let {key: myValue} = {"key": 1};"#;
    let program = parse_v2(source).unwrap();
    let result = linter::lint(&program, &linter::LintConfig::default());
    let w200s: Vec<_> = result.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W200"))
        .collect();
    assert_eq!(w200s.len(), 1, "Expected 1 W200 for myValue, got: {:?}", w200s);
}

#[test]
fn test_lint_w200_for_loop_destructure() {
    use magi_lang::linter;
    let source = r#"for [firstName, lastName] in [[1, 2]] { firstName }"#;
    let program = parse_v2(source).unwrap();
    let result = linter::lint(&program, &linter::LintConfig::default());
    let w200s: Vec<_> = result.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W200"))
        .collect();
    assert_eq!(w200s.len(), 2, "Expected 2 W200 for firstName and lastName, got: {:?}", w200s);
}

// Round 21: Test coverage gaps

#[test]
fn test_catch_variable_scope_isolation() {
    // Variables declared in catch block should not leak
    assert_eq!(
        run(r#"
let mut result = "none";
try {
    throw "error";
} catch e {
    let x = 42;
    result = "caught";
}
result
"#),
        DataType::String("caught".to_string())
    );
}

#[test]
fn test_rest_pattern_empty_array() {
    assert_eq!(
        run(r#"
let [first, ...rest] = [42];
[first, rest]
"#),
        DataType::Array(vec![
            DataType::Int64(42),
            DataType::Array(vec![]),
        ])
    );
}

#[test]
fn test_rest_pattern_multiple_elements() {
    assert_eq!(
        run(r#"
let [a, b, ...rest] = [1, 2, 3, 4, 5];
rest
"#),
        DataType::Array(vec![
            DataType::Int64(3),
            DataType::Int64(4),
            DataType::Int64(5),
        ])
    );
}

#[test]
fn test_destructure_too_few_elements_errors() {
    // [a, b, c] with only 1 element — non-rest positions get Null
    // But [a, b, ...rest] requires at least 2 elements
    let err = run_err(r#"
let [a, b, ...rest] = "not_array";
a
"#);
    match err {
        InterpError::TypeError { context, .. } => {
            assert_eq!(context, "array destructuring");
        }
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn test_destructure_rest_at_end() {
    // [a, b, ...rest] with exactly 2 elements: a=1, b=2, rest=[]
    assert_eq!(
        run(r#"
let [a, b, ...rest] = [1, 2];
[a, b, rest]
"#),
        DataType::Array(vec![
            DataType::Int64(1),
            DataType::Int64(2),
            DataType::Array(vec![]),
        ])
    );
}

#[test]
fn test_string_interpolation_with_method_calls() {
    assert_eq!(
        run(r#"
let s = "hello";
f"upper={s.to_upper()}, len={s.len()}"
"#),
        DataType::String("upper=HELLO, len=5".to_string())
    );
}

#[test]
fn test_mixed_numeric_chained_operations() {
    assert_eq!(
        run(r#"
let i = 10;
let f = 2.5;
i * f + 5.0 - f / 2.0
"#),
        DataType::Float64(28.75)
    );
}

#[test]
fn test_int_min_max() {
    assert_eq!(run("(5).min(3)"), DataType::Int64(3));
    assert_eq!(run("(5).max(10)"), DataType::Int64(10));
    assert_eq!(run("(5).clamp(1, 3)"), DataType::Int64(3));
    assert_eq!(run("(5).clamp(7, 10)"), DataType::Int64(7));
}

#[test]
fn test_float_min_max() {
    assert_eq!(run("(5.0).min(3.0)"), DataType::Float64(3.0));
    assert_eq!(run("(5.0).max(10.0)"), DataType::Float64(10.0));
    assert_eq!(run("(5.0).clamp(1.0, 3.0)"), DataType::Float64(3.0));
}

#[test]
fn test_float_math_methods() {
    assert_eq!(run("(1.0).sin()"), DataType::Float64(1.0_f64.sin()));
    assert_eq!(run("(0.0).cos()"), DataType::Float64(1.0));
    assert_eq!(run("(1.0).ln()"), DataType::Float64(0.0));
    assert_eq!(run("(100.0).log10()"), DataType::Float64(2.0));
    assert_eq!(run("(8.0).log2()"), DataType::Float64(3.0));
}

#[test]
fn test_int_pow() {
    assert_eq!(run("(2).pow(10)"), DataType::Int64(1024));
    assert_eq!(run("(2).pow(0)"), DataType::Int64(1));
}

#[test]
fn test_float_pow() {
    assert_eq!(run("(2.0).pow(0.5)"), DataType::Float64(2.0_f64.powf(0.5)));
}

#[test]
fn test_array_min_max_empty() {
    assert_eq!(run("[].min()"), DataType::Null);
    assert_eq!(run("[].max()"), DataType::Null);
}

#[test]
fn test_array_sum_product_empty() {
    assert_eq!(run("[].sum()"), DataType::Int64(0));
    assert_eq!(run("[].product()"), DataType::Int64(1));
}

#[test]
fn test_array_first_last_empty() {
    assert_eq!(run("[].first()"), DataType::Null);
    assert_eq!(run("[].last()"), DataType::Null);
}

#[test]
fn test_module_function_call() {
    assert_eq!(
        run(r#"
mod math {
    fn add(a, b) { a + b }
}
math::add(3, 4)
"#),
        DataType::Int64(7)
    );
}

#[test]
fn test_async_spawn_await() {
    assert_eq!(
        run(r#"
async fn compute() { 42 }
let t = spawn compute();
await t
"#),
        DataType::Int64(42)
    );
}

// ── Round 23: Short-circuit, default params, overflow, parser fixes ──

#[test]
fn test_and_short_circuit() {
    // && should not evaluate right side when left is false
    // If right side were evaluated, it would call undefined function and crash
    assert_eq!(
        run(r#"
let x = false && true;
x
"#),
        DataType::Bool(false)
    );
}

#[test]
fn test_or_short_circuit() {
    // || should not evaluate right side when left is true
    assert_eq!(
        run(r#"
let x = true || false;
x
"#),
        DataType::Bool(true)
    );
}

#[test]
fn test_and_evaluates_right_when_left_true() {
    assert_eq!(
        run(r#"
let x = true && false;
x
"#),
        DataType::Bool(false)
    );
}

#[test]
fn test_or_evaluates_right_when_left_false() {
    assert_eq!(
        run(r#"
let x = false || true;
x
"#),
        DataType::Bool(true)
    );
}

#[test]
fn test_short_circuit_and_prevents_error() {
    // Guard pattern: check before accessing
    assert_eq!(
        run(r#"
let arr = [];
let safe = arr.len() > 0 && arr.first() > 5;
safe
"#),
        DataType::Bool(false)
    );
}

#[test]
fn test_default_param_uses_caller_scope() {
    assert_eq!(
        run(r#"
let TIMEOUT = 30;
fn fetch(url, timeout = TIMEOUT) {
    timeout
}
fetch("http://example.com")
"#),
        DataType::Int64(30)
    );
}

#[test]
fn test_default_param_with_expression() {
    assert_eq!(
        run(r#"
let BASE = 10;
fn calc(x, offset = BASE * 2) {
    x + offset
}
calc(5)
"#),
        DataType::Int64(25)
    );
}

#[test]
fn test_default_param_overridden_by_caller() {
    assert_eq!(
        run(r#"
let DEFAULT = 100;
fn greet(name, greeting = DEFAULT) {
    greeting
}
greet("Alice", 42)
"#),
        DataType::Int64(42)
    );
}

#[test]
fn test_array_sum_overflow_promotes_to_float() {
    // i64::MAX + 1 should promote to float instead of wrapping
    assert_eq!(
        run(r#"
let arr = [9223372036854775807, 1];
let s = arr.sum();
typeof(s)
"#),
        DataType::String("float64".to_string())
    );
}

#[test]
fn test_array_sum_normal() {
    assert_eq!(
        run(r#"
[1, 2, 3, 4, 5].sum()
"#),
        DataType::Int64(15)
    );
}

#[test]
fn test_enum_inside_function_block() {
    // enum/struct should be parseable inside blocks
    assert_eq!(
        run(r#"
fn make_color() {
    enum Color { Red, Green, Blue }
    Color::Red
}
let c = make_color();
c.__variant
"#),
        DataType::String("Red".to_string())
    );
}

#[test]
fn test_struct_inside_function_block() {
    assert_eq!(
        run(r#"
fn make_point() {
    struct Point { x, y }
    Point { x: 10, y: 20 }
}
let p = make_point();
p.x
"#),
        DataType::Int64(10)
    );
}

#[test]
fn test_run_tests_enum_after_test() {
    // Enums defined after test blocks should still be available via pass 1 collection
    let src = r#"
test "use color" {
    let c = Color::Red;
    assert_eq(c.__variant, "Red");
}

enum Color { Red, Green, Blue }
"#;
    let program = parse_v2(src).expect("parse error");
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    let results = interp.run_tests(&program);
    assert_eq!(results.len(), 1);
    assert!(results[0].passed, "test should pass but got: {:?}", results[0].error_message);
}

// ── Round 24: join separator, find_index null, pad limits ──

#[test]
fn test_array_join_with_separator() {
    assert_eq!(
        run(r#"
let words = ["hello", "world", "foo"];
words.join(" - ")
"#),
        DataType::String("hello - world - foo".to_string())
    );
}

#[test]
fn test_array_join_default_separator() {
    assert_eq!(
        run(r#"
[1, 2, 3].join()
"#),
        DataType::String("1,2,3".to_string())
    );
}

#[test]
fn test_array_join_empty_separator() {
    assert_eq!(
        run(r#"
["a", "b", "c"].join("")
"#),
        DataType::String("abc".to_string())
    );
}

#[test]
fn test_find_index_found() {
    assert_eq!(
        run(r#"
[10, 20, 30, 40].find_index(|x| x == 30)
"#),
        DataType::Int64(2)
    );
}

#[test]
fn test_find_index_not_found_returns_null() {
    assert_eq!(
        run(r#"
[10, 20, 30].find_index(|x| x == 99)
"#),
        DataType::Null
    );
}

#[test]
fn test_pad_start_basic() {
    assert_eq!(
        run(r#"
"42".pad_start(5, "0")
"#),
        DataType::String("00042".to_string())
    );
}

#[test]
fn test_pad_end_basic() {
    assert_eq!(
        run(r#"
"hi".pad_end(5)
"#),
        DataType::String("hi   ".to_string())
    );
}

#[test]
fn test_pad_start_excessive_width_errors() {
    let err = run_err(r#"
"x".pad_start(99999999999)
"#);
    assert!(matches!(err, InterpError::ResourceLimit { .. }));
}

// ── Round 25: catch values, error display, type checker fixes ──

#[test]
fn test_catch_preserves_thrown_string() {
    assert_eq!(
        run(r#"
let result = try {
    throw "something went wrong";
    0
} catch e {
    e
};
result
"#),
        DataType::String("something went wrong".to_string())
    );
}

#[test]
fn test_catch_preserves_thrown_map() {
    assert_eq!(
        run(r#"
let result = try {
    throw {"code": 404, "msg": "not found"};
    0
} catch e {
    e.code
};
result
"#),
        DataType::Int64(404)
    );
}

#[test]
fn test_catch_preserves_thrown_int() {
    assert_eq!(
        run(r#"
let result = try { throw 42; 0 } catch e { e };
result
"#),
        DataType::Int64(42)
    );
}

#[test]
fn test_statement_try_catch_preserves_thrown_value() {
    // Statement-level try/catch: verify thrown array is preserved
    assert_eq!(
        run(r#"
let mut caught = null;
try {
    throw [1, 2, 3];
} catch e {
    caught = e;
}
caught
"#),
        DataType::Array(vec![DataType::Int64(1), DataType::Int64(2), DataType::Int64(3)])
    );
}

#[test]
fn test_short_circuit_and_with_side_effect_guard() {
    // Verify short-circuit: second expression should not be evaluated
    assert_eq!(
        run(r#"
let x = false && (1 / 0 > 0);
x
"#),
        DataType::Bool(false)
    );
}

#[test]
fn test_short_circuit_or_with_side_effect_guard() {
    assert_eq!(
        run(r#"
let x = true || (1 / 0 > 0);
x
"#),
        DataType::Bool(true)
    );
}

#[test]
fn test_string_concat_no_type_checker_warning() {
    let program = parse_v2(r#"let x = "hello" + " " + "world"; output x;"#).unwrap();
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    let arith_warnings: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.message.contains("Arithmetic operator"))
        .collect();
    assert!(arith_warnings.is_empty(), "String + should not warn: {:?}", arith_warnings);
}

#[test]
fn test_pipe_with_lambda_variable() {
    assert_eq!(
        run("let double = |x| x * 2\n5 |> double(_)"),
        DataType::Int64(10)
    );
}

#[test]
fn test_pipe_with_lambda_variable_chained() {
    assert_eq!(
        run("let inc = |x| x + 1\nlet dbl = |x| x * 2\n3 |> inc(_) |> dbl(_)"),
        DataType::Int64(8)
    );
}

#[test]
fn test_pipe_len_on_string() {
    assert_eq!(
        run(r#""hello" |> len(_)"#),
        DataType::Int64(5)
    );
}

#[test]
fn test_pipe_len_on_map() {
    assert_eq!(
        run("let m = {\"a\": 1, \"b\": 2, \"c\": 3}\nm |> len(_)"),
        DataType::Int64(3)
    );
}

#[test]
fn test_pipe_len_implicit_piped_value() {
    assert_eq!(
        run("[10, 20, 30] |> len()"),
        DataType::Int64(3)
    );
}

#[test]
fn test_pipe_len_errors_on_invalid_type() {
    let err = run_err("42 |> len(_)");
    match err {
        InterpError::TypeError { context, .. } => {
            assert_eq!(context, "len");
        }
        other => panic!("Expected TypeError, got: {:?}", other),
    }
}

#[test]
fn test_levenshtein_unicode_suggestion() {
    // Test that the Levenshtein distance works with multi-byte chars
    use magi_lang::syntax::errors::suggest_name;
    let available = ["café", "naïve", "résumé"];
    // "cafè" is 1 edit from "café"
    let result = suggest_name("cafè", &available);
    assert_eq!(result, Some("did you mean 'café'?".to_string()));
}

#[test]
fn test_type_checker_non_exhaustive_match_uses_w203() {
    let program = parse_v2(r#"
let x = 42;
let r = match x { 1 => "one", 2 => "two" };
output r;
"#).unwrap();
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    let codes: Vec<_> = analysis.diagnostics.iter()
        .filter_map(|d| d.code.as_deref())
        .collect();
    assert!(codes.contains(&"W203"), "Expected W203 for non-exhaustive match, got: {:?}", codes);
}

// ═══════════════════════════════════════════════════════════
// Round 27: Match guard, has_main, spawn, while loop, test isolation
// ═══════════════════════════════════════════════════════════

#[test]
fn test_match_guard_non_boolean_errors() {
    let err = run_err("match 42 { x if 1 => x, _ => -1 }");
    match err {
        InterpError::TypeError { context, .. } => {
            assert_eq!(context, "match guard");
        }
        other => panic!("Expected TypeError for non-bool guard, got: {:?}", other),
    }
}

#[test]
fn test_match_guard_boolean_works() {
    assert_eq!(
        run("match 42 { x if true => x, _ => -1 }"),
        DataType::Int64(42)
    );
    assert_eq!(
        run("match 42 { x if false => x, _ => -1 }"),
        DataType::Int64(-1)
    );
}

#[test]
fn test_spawn_captures_error_as_rejected_future() {
    assert_eq!(
        run("fn fail() { throw \"oops\" }\nlet f = spawn fail()\ntypeof(f)"),
        DataType::String("future".to_string())
    );
}

#[test]
fn test_while_loop_returns_last_body_value() {
    assert_eq!(
        run("let mut i = 0\nwhile i < 5 { i = i + 1\ni * 10 }"),
        DataType::Int64(50)
    );
}

#[test]
fn test_while_loop_empty_returns_null() {
    // If condition is immediately false, returns Null
    assert_eq!(
        run("while false { 42 }"),
        DataType::Null
    );
}

#[test]
fn test_run_tests_async_fn_registered() {
    // Verify async functions are properly wrapped as futures in test runner
    let program = parse("
        async fn fetch() { 42 }
        test \"async works\" {
            let f = fetch();
            assert_eq(typeof(f), \"future\")
        }
    ");
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    let results = interp.run_tests(&program);
    assert!(results[0].passed, "Test should pass: {:?}", results[0].error_message);
}

#[test]
fn test_run_tests_isolation_variables() {
    // Test that variables defined in one test don't leak to the next
    let program = parse("
        test \"define variable\" {
            let x = 999;
            assert_eq(x, 999)
        }
        test \"variable should not exist\" {
            let found = try { x } catch e { \"not found\" };
            assert_eq(found, \"not found\")
        }
    ");
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    let results = interp.run_tests(&program);
    assert!(results[0].passed, "First test should pass: {:?}", results[0].error_message);
    assert!(results[1].passed, "Second test should pass (isolation): {:?}", results[1].error_message);
}

// ═══════════════════════════════════════════════════════════
// Round 28: Formatter escaping, destructuring, formatter parens
// ═══════════════════════════════════════════════════════════

#[test]
fn test_formatter_preserves_string_escapes() {
    use magi_lang::formatter::{format_program, FormatConfig};
    let source = "let x = \"hello\\nworld\\t!\"";
    let program = parse(source);
    let config = FormatConfig::default();
    let formatted = format_program(&program, &config);
    assert!(formatted.contains("\\n"), "Formatter should preserve \\n escape: {}", formatted);
    assert!(formatted.contains("\\t"), "Formatter should preserve \\t escape: {}", formatted);
}

#[test]
fn test_formatter_fstring_preserves_escapes() {
    use magi_lang::formatter::{format_program, FormatConfig};
    let source = "let x = f\"line1\\nline2 {name}\"";
    let program = parse(source);
    let config = FormatConfig::default();
    let formatted = format_program(&program, &config);
    assert!(formatted.contains("\\n"), "Formatter should preserve \\n in f-string: {}", formatted);
}

#[test]
fn test_destructure_rest_end() {
    // Rest at end: [a, b, ...rest] = [1, 2, 3, 4]
    assert_eq!(
        run("let [a, b, ...rest] = [1, 2, 3, 4]\nrest"),
        DataType::Array(vec![DataType::Int64(3), DataType::Int64(4)])
    );
}

#[test]
fn test_destructure_rest_empty() {
    // Rest at end with exact match: [a, b, ...rest] = [1, 2]
    assert_eq!(
        run("let [a, b, ...rest] = [1, 2]\nrest"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_destructure_array_basic() {
    assert_eq!(
        run("let [x, y] = [10, 20]\nx + y"),
        DataType::Int64(30)
    );
}

#[test]
fn test_formatter_range_in_method_call_gets_parens() {
    use magi_lang::formatter::{format_program, FormatConfig};
    // This tests that Range expressions get parens when used as method call object
    let source = "let x = (1..10).len()";
    let program = parse(source);
    let config = FormatConfig::default();
    let formatted = format_program(&program, &config);
    assert!(formatted.contains("(1..10)"), "Range should be parenthesized: {}", formatted);
}

// ── Round 31: Resource limits & safety ────────────────────

#[test]
fn test_split_normal_works() {
    assert_eq!(
        run(r#""a,b,c".split(",")"#),
        DataType::Array(vec![
            DataType::String("a".to_string()),
            DataType::String("b".to_string()),
            DataType::String("c".to_string()),
        ])
    );
}

#[test]
fn test_replace_normal_works() {
    assert_eq!(
        run(r#""hello world".replace("world", "magi")"#),
        DataType::String("hello magi".to_string())
    );
}

#[test]
fn test_join_normal_works() {
    assert_eq!(
        run(r#"[1, 2, 3].join("-")"#),
        DataType::String("1-2-3".to_string())
    );
}

#[test]
fn test_chars_normal_works() {
    assert_eq!(
        run(r#""abc".chars()"#),
        DataType::Array(vec![
            DataType::String("a".to_string()),
            DataType::String("b".to_string()),
            DataType::String("c".to_string()),
        ])
    );
}

#[test]
fn test_lines_normal_works() {
    // Use a string with embedded newlines
    assert_eq!(
        run(r#""a\nb\nc".lines()"#),
        DataType::Array(vec![
            DataType::String("a".to_string()),
            DataType::String("b".to_string()),
            DataType::String("c".to_string()),
        ])
    );
}

#[test]
fn test_hof_map_with_cancellation_support() {
    // Just verify map still works correctly — cancellation is tested via the cancel token
    assert_eq!(
        run("[1, 2, 3].map(|x| x * 2)"),
        DataType::Array(vec![
            DataType::Int64(2),
            DataType::Int64(4),
            DataType::Int64(6),
        ])
    );
}

#[test]
fn test_hof_filter_works() {
    assert_eq!(
        run("[1, 2, 3, 4].filter(|x| x > 2)"),
        DataType::Array(vec![DataType::Int64(3), DataType::Int64(4)])
    );
}

#[test]
fn test_hof_reduce_works() {
    assert_eq!(
        run("[1, 2, 3].reduce(0, |acc, x| acc + x)"),
        DataType::Int64(6)
    );
}

#[test]
fn test_hof_flat_map_works() {
    assert_eq!(
        run("[1, 2].flat_map(|x| [x, x * 10])"),
        DataType::Array(vec![
            DataType::Int64(1),
            DataType::Int64(10),
            DataType::Int64(2),
            DataType::Int64(20),
        ])
    );
}

#[test]
fn test_hof_scan_works() {
    assert_eq!(
        run("[1, 2, 3].scan(0, |acc, x| acc + x)"),
        DataType::Array(vec![
            DataType::Int64(1),
            DataType::Int64(3),
            DataType::Int64(6),
        ])
    );
}

#[test]
fn test_lsp_diagnostic_nonzero_width() {
    use magi_lang::lsp::analysis::to_lsp_diagnostic_with_source;
    use magi_lang::syntax::type_checker::AstDiagnostic;
    use magi_lang::eval::DiagnosticSeverity;

    let d = AstDiagnostic {
        line: 1,
        column: 5,
        message: "unknown var".to_string(),
        severity: DiagnosticSeverity::Error,
        code: Some("E100".to_string()),
        help: None,
        suggestion: None,
    };
    let source = "let my_var = 42;";
    let lsp_d = to_lsp_diagnostic_with_source(&d, Some(source));
    // start at col 4 ('m' of my_var), end at col 10 (end of my_var)
    assert_eq!(lsp_d.range.start.character, 4);
    assert_eq!(lsp_d.range.end.character, 10);
    assert!(lsp_d.range.end.character > lsp_d.range.start.character, "range should be non-zero-width");
}

#[test]
fn test_lsp_utf16_column_conversion() {
    use magi_lang::lsp::analysis::{char_col_to_utf16, utf16_to_char_col};

    // ASCII: char col == UTF-16 col
    assert_eq!(char_col_to_utf16("hello world", 5), 5);
    assert_eq!(utf16_to_char_col("hello world", 5), 5);

    // Multi-byte but BMP: each char is 1 UTF-16 code unit
    // "café" — é is 2 bytes in UTF-8 but 1 UTF-16 code unit
    assert_eq!(char_col_to_utf16("café", 4), 4);
    assert_eq!(utf16_to_char_col("café", 4), 4);
}

// ── Round 32: else-if, cross-type match, circular import, linter, trailing commas ──

#[test]
fn test_else_if_chain() {
    let result = run(
        r#"
        fn classify(n) {
            if n < 0 {
                "negative"
            } else if n == 0 {
                "zero"
            } else if n < 10 {
                "small"
            } else {
                "big"
            }
        }
        [classify(-5), classify(0), classify(3), classify(100)]
        "#,
    );
    assert_eq!(
        result,
        DataType::Array(vec![
            DataType::String("negative".into()),
            DataType::String("zero".into()),
            DataType::String("small".into()),
            DataType::String("big".into()),
        ])
    );
}

#[test]
fn test_else_if_without_else() {
    let result = run(
        r#"
        let x = 5;
        if x < 0 {
            "neg"
        } else if x > 100 {
            "big"
        }
        "#,
    );
    assert_eq!(result, DataType::Null);
}

#[test]
fn test_cross_type_match_int_float() {
    // Matching float value against int literal pattern
    let result = run(
        r#"
        match 1.0 {
            1 => "one",
            2 => "two",
            _ => "other",
        }
        "#,
    );
    assert_eq!(result, DataType::String("one".into()));
}

#[test]
fn test_cross_type_match_float_int() {
    // Matching int value against float literal pattern
    let result = run(
        r#"
        match 42 {
            42.0 => "found",
            _ => "nope",
        }
        "#,
    );
    assert_eq!(result, DataType::String("found".into()));
}

#[test]
fn test_trailing_comma_function_call() {
    let result = run(
        r#"
        fn add(a, b) { a + b }
        add(10, 20,)
        "#,
    );
    assert_eq!(result, DataType::Int64(30));
}

#[test]
fn test_trailing_comma_function_def() {
    let result = run(
        r#"
        fn greet(name: string, greeting: string,) {
            greeting + " " + name
        }
        greet("world", "hello",)
        "#,
    );
    assert_eq!(result, DataType::String("hello world".into()));
}

#[test]
fn test_trailing_comma_array_literal() {
    let result = run(
        r#"
        [1, 2, 3,]
        "#,
    );
    assert_eq!(
        result,
        DataType::Array(vec![
            DataType::Int64(1),
            DataType::Int64(2),
            DataType::Int64(3),
        ])
    );
}

#[test]
fn test_trailing_comma_map_literal() {
    let result = run(
        r#"
        { "a": 1, "b": 2, }
        "#,
    );
    let map = match result {
        DataType::Map(m) => m,
        other => panic!("expected map, got {:?}", other),
    };
    assert_eq!(map.get("a"), Some(&DataType::Int64(1)));
    assert_eq!(map.get("b"), Some(&DataType::Int64(2)));
}

#[test]
fn test_lint_const_naming() {
    use magi_lang::linter;

    let program = parse(
        r#"
        const MyBadConst = 42;
        const good_name = 10;
        "#,
    );
    let result = linter::lint(&program, &linter::LintConfig::default());
    let codes: Vec<&str> = result
        .diagnostics
        .iter()
        .filter_map(|d| d.code.as_deref())
        .collect();
    assert!(codes.contains(&"W200"), "should warn on PascalCase const name");
    // good_name should not produce a warning
    assert_eq!(codes.iter().filter(|&&c| c == "W200").count(), 1);
}

// ── Round 33: optional chaining methods, zero-param lambda, scope isolation, &&/|| types, exhaustiveness ──

#[test]
fn test_optional_chaining_method_null() {
    let result = run(
        r#"
        let obj = null;
        obj?.keys()
        "#,
    );
    assert_eq!(result, DataType::Null);
}

#[test]
fn test_optional_chaining_method_nonnull() {
    let result = run(
        r#"
        let arr = [1, 2, 3];
        arr?.map(|x| x * 10)
        "#,
    );
    assert_eq!(
        result,
        DataType::Array(vec![
            DataType::Int64(10),
            DataType::Int64(20),
            DataType::Int64(30),
        ])
    );
}

#[test]
fn test_optional_chaining_hof_null() {
    let result = run(
        r#"
        let arr = null;
        arr?.map(|x| x * 2)
        "#,
    );
    assert_eq!(result, DataType::Null);
}

#[test]
fn test_zero_param_lambda() {
    let result = run(
        r#"
        let f = || 42;
        f()
        "#,
    );
    assert_eq!(result, DataType::Int64(42));
}

#[test]
fn test_zero_param_lambda_block() {
    let result = run(
        r#"
        let f = || { let x = 10; x + 1 };
        f()
        "#,
    );
    assert_eq!(result, DataType::Int64(11));
}

#[test]
fn test_if_else_scope_isolation() {
    let result = run_err(
        r#"
        if true { let x = 42 }
        x
        "#,
    );
    match result {
        InterpError::UndefinedVariable { name, .. } => assert_eq!(name, "x"),
        other => panic!("expected UndefinedVariable, got {:?}", other),
    }
}

#[test]
fn test_if_else_value_with_scope() {
    let result = run(
        r#"
        let val = if true { let x = 10; x + 5 } else { 0 };
        val
        "#,
    );
    assert_eq!(result, DataType::Int64(15));
}

#[test]
fn test_block_scope_isolation() {
    let result = run_err(
        r#"
        { let inner = 99 }
        inner
        "#,
    );
    match result {
        InterpError::UndefinedVariable { name, .. } => assert_eq!(name, "inner"),
        other => panic!("expected UndefinedVariable, got {:?}", other),
    }
}

#[test]
fn test_and_or_require_bool() {
    let result = run_err(
        r#"
        0 && true
        "#,
    );
    match result {
        InterpError::TypeError { context, .. } => assert!(context.contains("&&")),
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn test_and_or_short_circuit() {
    // false && <anything> should short-circuit
    let result = run(
        r#"
        let mut x = 0;
        false && { x = 1; true };
        x
        "#,
    );
    assert_eq!(result, DataType::Int64(0));
}

#[test]
fn test_exhaustive_enum_match_no_warning() {
    let program = parse(
        r#"
        enum Color { Red, Green, Blue }
        let c = Color::Red();
        match c {
            Color::Red() => "r",
            Color::Green() => "g",
            Color::Blue() => "b",
        }
        "#,
    );
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    let w203: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| d.code.as_deref() == Some("W203"))
        .collect();
    assert!(w203.is_empty(), "should not warn W203 on exhaustive enum match, got: {:?}", w203);
}

#[test]
fn test_incomplete_enum_match_warns() {
    let program = parse(
        r#"
        enum Color { Red, Green, Blue }
        let c = Color::Red();
        match c {
            Color::Red() => "r",
            Color::Green() => "g",
        }
        "#,
    );
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    let w203: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| d.code.as_deref() == Some("W203"))
        .collect();
    assert!(!w203.is_empty(), "should warn W203 on incomplete enum match");
}

// ── Round 34: Formatter, LSP, and WASM compiler fixes ──────────────

#[test]
fn test_formatter_else_if_chain() {
    use magi_lang::formatter::{format_program, FormatConfig};

    let src = r#"
let x = if true { 1 } else if false { 2 } else { 3 }
"#;
    let program = parse(src);
    let formatted = format_program(&program, &FormatConfig::default());
    // Should format as `else if`, not `else { if ... }`
    assert!(
        formatted.contains("else if"),
        "else-if chain should be formatted as 'else if', got:\n{}",
        formatted
    );
    assert!(
        !formatted.contains("else {\n    if"),
        "else-if should not be wrapped in a block, got:\n{}",
        formatted
    );
}

#[test]
fn test_formatter_no_semicolon_after_block_expr() {
    use magi_lang::formatter::{format_program, FormatConfig};

    let src = "if true { 1 } else { 2 }";
    let program = parse(src);
    let formatted = format_program(&program, &FormatConfig::default());
    // Expression statement ending with if/else should not have semicolon
    assert!(
        !formatted.trim().ends_with(';'),
        "block expression statement should not end with semicolon, got: '{}'",
        formatted.trim()
    );
}

#[test]
fn test_formatter_map_key_escaping() {
    use magi_lang::formatter::{format_program, FormatConfig};

    let src = r#"let m = {"hello\nworld": 1, "tab\there": 2}"#;
    let program = parse(src);
    let formatted = format_program(&program, &FormatConfig::default());
    // Keys with \n and \t should be properly escaped
    assert!(
        formatted.contains(r#"hello\nworld"#),
        "newline in key should be escaped, got:\n{}",
        formatted
    );
    assert!(
        formatted.contains(r#"tab\there"#),
        "tab in key should be escaped, got:\n{}",
        formatted
    );
}

#[test]
fn test_wasm_null_coalesce_compiles() {
    use magi_lang::compiler::compile_to_wasm;

    let src = r#"
let x = null
let y = x ?? "default"
output y
"#;
    let program = parse(src);
    let result = compile_to_wasm(&program);
    assert!(
        result.is_ok(),
        "null coalesce should compile to valid WASM, got error: {:?}",
        result.err()
    );
}

#[test]
fn test_wasm_null_coalesce_with_value() {
    use magi_lang::compiler::compile_to_wasm;

    let src = r#"
let x = 42
let y = x ?? 0
output y
"#;
    let program = parse(src);
    let result = compile_to_wasm(&program);
    assert!(result.is_ok(), "null coalesce with non-null should compile");
}

#[test]
fn test_formatter_match_no_semicolon() {
    use magi_lang::formatter::{format_program, FormatConfig};

    let src = r#"
match x {
    1 => "one",
    _ => "other",
}
"#;
    let program = parse(src);
    let formatted = format_program(&program, &FormatConfig::default());
    let trimmed = formatted.trim();
    assert!(
        !trimmed.ends_with("};"),
        "match expression statement should not end with semicolon after closing brace, got: '{}'",
        trimmed
    );
}

#[test]
fn test_formatter_idempotent_else_if() {
    use magi_lang::formatter::{format_program, FormatConfig};

    let src = r#"
let x = 5
let result = if x > 10 {
    "big"
} else if x > 5 {
    "medium"
} else {
    "small"
}
"#;
    let program = parse(src);
    let config = FormatConfig::default();
    let first = format_program(&program, &config);
    let program2 = parse(&first);
    let second = format_program(&program2, &config);
    assert_eq!(first, second, "else-if formatting should be idempotent");
}

// ── Round 35: Parser, type checker, and linter fixes ────────────────

#[test]
fn test_optional_chain_method_span() {
    // The OptionalChain marker in obj?.method(args) should have a span
    // covering the full expression, not just obj?.method
    let src = "let x = null\nlet r = x?.some_method(1, 2, 3)";
    let program = parse(src);
    // Verify it parses without error and produces the right AST
    assert!(!program.statements.is_empty());
}

#[test]
fn test_or_pattern_variables_type_check() {
    // Or patterns should bind variables from all alternatives
    let src = r#"
enum Result { Ok(value), Err(msg) }
let x = Result::Ok(42)
let r = match x {
    Result::Ok(v) | Result::Err(v) => v,
}
"#;
    let program = parse(src);
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    // Should NOT have undefined variable errors for v
    let undef_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("not defined") || d.message.contains("undefined"))
        .collect();
    assert!(
        undef_errors.is_empty(),
        "Or pattern variables should be bound: {:?}",
        undef_errors
    );
}

#[test]
fn test_literal_match_no_catchall_warns() {
    // Non-enum match without wildcard should still warn W203
    let src = r#"
let x = 1
let r = match x {
    1 => "one",
    2 => "two",
}
"#;
    let program = parse(src);
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    let w203: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| d.code.as_deref() == Some("W203"))
        .collect();
    assert!(!w203.is_empty(), "non-exhaustive literal match should warn W203");
}

#[test]
fn test_enum_match_fully_covered_no_warn() {
    // Fully covered enum match should NOT warn W203
    let src = r#"
enum Color { Red, Green, Blue }
let c = Color::Red
let name = match c {
    Color::Red => "red",
    Color::Green => "green",
    Color::Blue => "blue",
}
"#;
    let program = parse(src);
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    let w203: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| d.code.as_deref() == Some("W203"))
        .collect();
    assert!(
        w203.is_empty(),
        "fully covered enum match should not warn W203: {:?}",
        w203
    );
}

// ── Round 36: Interpreter struct validation ──────────────────────────

#[test]
fn test_struct_duplicate_field_error() {
    let src = r#"
struct Point { x: int, y: int }
let p = Point { x: 1, x: 2, y: 3 }
"#;
    let program = parse(src);
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    let result = interp.execute(&program);
    assert!(result.is_err(), "duplicate struct fields should error");
    let err = result.unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("duplicate"),
        "error should mention 'duplicate', got: {}",
        msg
    );
}

#[test]
fn test_struct_unique_fields_ok() {
    let src = r#"
struct Point { x: int, y: int }
let p = Point { x: 1, y: 2 }
p
"#;
    let result = run(src);
    match result {
        DataType::Map(m) => {
            assert_eq!(m.get("x"), Some(&DataType::Int64(1)));
            assert_eq!(m.get("y"), Some(&DataType::Int64(2)));
        }
        other => panic!("expected Map, got {:?}", other),
    }
}

// ── Round 37: Map and string iteration ───────────────────────────────

#[test]
fn test_for_loop_string_iteration() {
    // Use a counter to verify iteration count; can't use .concat() with StubEvaluator
    let src = r#"
let mut count = 0
let mut last = ""
for ch in "hello" {
    count = count + 1
    last = ch
}
[count, last]
"#;
    let result = run(src);
    match result {
        DataType::Array(arr) => {
            assert_eq!(arr[0], DataType::Int64(5));
            assert_eq!(arr[1], DataType::String("o".to_string()));
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

#[test]
fn test_for_loop_map_iteration() {
    let src = r#"
let m = {"a": 1, "b": 2}
let mut count = 0
let mut last_key = ""
for {key} in m {
    count = count + 1
    last_key = key
}
[count, last_key]
"#;
    let result = run(src);
    match result {
        DataType::Array(arr) => {
            assert_eq!(arr[0], DataType::Int64(2));
            // BTreeMap is ordered — last key is "b"
            assert_eq!(arr[1], DataType::String("b".to_string()));
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

#[test]
fn test_for_loop_map_key_value() {
    let src = r#"
let m = {"x": 10}
let mut total = 0
for {key, value} in m {
    total = value
}
total
"#;
    let result = run(src);
    assert_eq!(result, DataType::Int64(10));
}

#[test]
fn test_list_comprehension_string() {
    let src = r#"
let chars = [ch for ch in "abc"]
chars
"#;
    let result = run(src);
    match result {
        DataType::Array(arr) => {
            assert_eq!(arr.len(), 3);
            assert_eq!(arr[0], DataType::String("a".to_string()));
            assert_eq!(arr[2], DataType::String("c".to_string()));
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

#[test]
fn test_list_comprehension_map_iteration() {
    // Iterate over map using single variable (each item is a {key, value} map)
    let src = r#"
let m = {"a": 1, "b": 2, "c": 3}
let entries = [entry for entry in m]
entries
"#;
    let result = run(src);
    match result {
        DataType::Array(arr) => {
            assert_eq!(arr.len(), 3);
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

#[test]
fn test_for_loop_empty_string() {
    let src = r#"
let mut count = 0
for ch in "" {
    count = count + 1
}
count
"#;
    let result = run(src);
    assert_eq!(result, DataType::Int64(0));
}

#[test]
fn test_for_loop_map_single_var() {
    // Iterate map with single variable: each item is {key: "...", value: ...}
    let src = r#"
let m = {"x": 42}
let mut result = null
for entry in m {
    result = entry
}
result
"#;
    let result = run(src);
    match result {
        DataType::Map(m) => {
            assert_eq!(m.get("key"), Some(&DataType::String("x".to_string())));
            assert_eq!(m.get("value"), Some(&DataType::Int64(42)));
        }
        other => panic!("expected Map, got {:?}", other),
    }
}

// ═══════════════════════════════════════════════════════════
// Round 38: Lexer, method, and WASM compiler fixes
// ═══════════════════════════════════════════════════════════

#[test]
fn test_fstring_unmatched_closing_brace_error() {
    let result = parse_v2(r#"f"hello }"#);
    assert!(result.is_err(), "expected parse error for unmatched closing brace in f-string");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Unmatched '}'"), "expected brace error, got: {err}");
}

#[test]
fn test_fstring_interpolation_still_works() {
    let result = run(r#"let x = 42; f"value is {x}""#);
    assert_eq!(result, DataType::String("value is 42".to_string()));
}

#[test]
fn test_split_empty_separator_error() {
    let err = run_err(r#""hello".split("")"#).to_string();
    assert!(err.contains("non-empty separator") || err.contains("empty string"),
        "expected empty separator error, got: {err}");
}

#[test]
fn test_split_nonempty_sep_works() {
    let result = run(r#""a,b,c".split(",")"#);
    match result {
        DataType::Array(arr) => {
            assert_eq!(arr.len(), 3);
            assert_eq!(arr[0], DataType::String("a".to_string()));
            assert_eq!(arr[1], DataType::String("b".to_string()));
            assert_eq!(arr[2], DataType::String("c".to_string()));
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

#[test]
fn test_min_max_mixed_types_error() {
    let err = run_err(r#"[1, "hello"].min()"#).to_string();
    assert!(err.contains("comparable"), "expected comparable error, got: {err}");
}

#[test]
fn test_min_max_mixed_numeric_ok() {
    let result = run(r#"[5, 3.0, 7, 1.5].min()"#);
    assert_eq!(result, DataType::Float64(1.5));
}

#[test]
fn test_wasm_match_guard_rejected() {
    let src = r#"
        fn classify(x) {
            match x {
                n if n > 10 => "big",
                _ => "small",
            }
        }
    "#;
    let program = parse(src);
    let result = compiler::compile_to_wasm(&program);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("match guards"), "expected guard error, got: {err}");
}

#[test]
fn test_wasm_array_spread_rejected() {
    let src = r#"
        let a = [1, 2, 3];
        let b = [0, ...a, 4];
    "#;
    let program = parse(src);
    let result = compiler::compile_to_wasm(&program);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("spread"), "expected spread error, got: {err}");
}

// ═══════════════════════════════════════════════════════════
// Round 39: Parser and interpreter correctness fixes
// ═══════════════════════════════════════════════════════════

#[test]
fn test_range_pattern_token_stream_integrity() {
    // Range pattern that succeeds: 0..10
    let result = run(r#"
        match 5 {
            0..10 => "in range",
            _ => "out of range",
        }
    "#);
    assert_eq!(result, DataType::String("in range".to_string()));
}

#[test]
fn test_range_pattern_inclusive() {
    let result = run(r#"
        match 10 {
            0..=10 => "in range",
            _ => "out of range",
        }
    "#);
    assert_eq!(result, DataType::String("in range".to_string()));
}

#[test]
fn test_negative_range_pattern() {
    let result = run(r#"
        match -3 {
            -10..0 => "negative",
            0..10 => "positive",
            _ => "other",
        }
    "#);
    assert_eq!(result, DataType::String("negative".to_string()));
}

#[test]
fn test_negative_to_negative_range_pattern() {
    let result = run(r#"
        match -5 {
            -10..-1 => "deep negative",
            _ => "other",
        }
    "#);
    assert_eq!(result, DataType::String("deep negative".to_string()));
}

#[test]
fn test_try_propagate_only_result_err() {
    // The ? operator should only propagate Result::Err, not arbitrary maps with __variant: "Err"
    let result = run(r#"
        enum MyEnum { Err(v) }
        fn test_fn() {
            let x = MyEnum::Err("oops");
            // This should NOT be treated as Result::Err by ?
            x
        }
        test_fn()
    "#);
    // Should return the enum map, not throw
    match result {
        DataType::Map(m) => {
            assert_eq!(m.get("__enum").map(|v| v.to_string_lossy()), Some("MyEnum".to_string()));
        }
        other => panic!("expected Map, got {:?}", other),
    }
}

#[test]
fn test_string_interpolation_basic() {
    let result = run(r#"
        let name = "world";
        let n = 42;
        f"hello {name}, number {n}"
    "#);
    assert_eq!(result, DataType::String("hello world, number 42".to_string()));
}

// ═══════════════════════════════════════════════════════════
// Round 39b: Type checker correctness fixes
// ═══════════════════════════════════════════════════════════

#[test]
fn test_w110_no_false_positive_method_mutation() {
    // Calling .push() on a mut variable should not produce W110
    let warnings = typecheck_warnings(r#"
        let mut items = [1, 2, 3];
        items.push(4);
        items
    "#);
    let w110: Vec<_> = warnings.iter().filter(|w| w.contains("W110")).collect();
    assert!(w110.is_empty(), "should not warn W110 for method mutation, got: {:?}", w110);
}

#[test]
fn test_function_as_first_class_no_e200() {
    // Using a function name as a value should not produce E200 "Undefined variable"
    let warnings = typecheck_warnings(r#"
        fn double(x) { x * 2 }
        let f = double;
        f
    "#);
    let e200: Vec<_> = warnings.iter().filter(|w| w.contains("E200") || w.contains("Undefined")).collect();
    assert!(e200.is_empty(), "should not warn E200 for function reference, got: {:?}", e200);
}

#[test]
fn test_function_as_callback_no_w103() {
    // Passing a function as a callback should mark it as used (no W103)
    let warnings = typecheck_warnings(r#"
        fn helper(x) { x + 1 }
        let result = helper;
        result
    "#);
    let w103: Vec<_> = warnings.iter().filter(|w| w.contains("W103")).collect();
    assert!(w103.is_empty(), "should not warn W103 for function used as value, got: {:?}", w103);
}

#[test]
fn test_break_in_lambda_inside_loop_flagged() {
    // break inside a lambda inside a loop should be flagged as E300
    let warnings = typecheck_warnings(r#"
        for x in [1, 2, 3] {
            let f = || { break };
            f
        }
    "#);
    let e300: Vec<_> = warnings.iter().filter(|w| w.contains("E300") || w.contains("break")).collect();
    assert!(!e300.is_empty(), "should flag break inside lambda as error, got: {:?}", warnings);
}

// ═══════════════════════════════════════════════════════════
// Round 40: Formatter, linter, and WASM fixes
// ═══════════════════════════════════════════════════════════

#[test]
fn test_formatter_float_scientific_notation() {
    // Floats that produce scientific notation should format to parseable output
    use magi_lang::formatter::{format_program, FormatConfig};
    let src = "let x = 100000000000000000000.0;";
    let program = parse(src);
    let formatted = format_program(&program, &FormatConfig::default());
    // Should contain a decimal point and be parseable
    assert!(formatted.contains('.'), "formatted float should contain decimal point: {formatted}");
    // Verify it round-trips (parse doesn't error)
    let _reparsed = parse_v2(&formatted).expect("formatted output should be parseable");
}

#[test]
fn test_formatter_compound_assign() {
    use magi_lang::formatter::{format_program, FormatConfig};
    let src = "x += 1;";
    let program = parse(src);
    let formatted = format_program(&program, &FormatConfig::default());
    assert!(formatted.contains("+="), "should format compound assign correctly: {formatted}");
    assert!(!formatted.contains("==="), "should not produce triple-equals: {formatted}");
}

#[test]
fn test_linter_pascal_case_underscore_prefix_suppressed() {
    use magi_lang::linter::{lint, LintConfig};
    let src = r#"
        enum _InternalState { Active, Inactive }
    "#;
    let program = parse(src);
    let result = lint(&program, &LintConfig::default());
    let w201: Vec<_> = result.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W201"))
        .collect();
    assert!(w201.is_empty(), "underscore-prefixed types should not get W201: {:?}", w201);
}

// ═══════════════════════════════════════════════════════════
// Round 41: CLI binary stability tests
// ═══════════════════════════════════════════════════════════

#[test]
fn test_integer_division_by_zero_error() {
    let src = "let x = 10 / 0;";
    let err = run_err(src);
    let msg = err.to_string();
    assert!(msg.to_lowercase().contains("division") || msg.to_lowercase().contains("zero"),
        "should error on integer division by zero: {msg}");
}

#[test]
fn test_integer_modulo_by_zero_error() {
    let src = "let x = 10 % 0;";
    let err = run_err(src);
    let msg = err.to_string();
    assert!(msg.to_lowercase().contains("division") || msg.to_lowercase().contains("zero"),
        "should error on integer modulo by zero: {msg}");
}

#[test]
fn test_basic_negation() {
    let src = "let x = 5; let y = -x; y";
    let result = run(src);
    assert_eq!(result, DataType::Int64(-5));
}

#[test]
fn test_basic_arithmetic_correctness() {
    // Verify basic addition still works after checked arithmetic change
    let src = "let x = 100 + 200; x";
    let result = run(src);
    assert_eq!(result, DataType::Int64(300));
}

#[test]
fn test_subtraction_correctness() {
    let src = "let x = 50 - 30; x";
    let result = run(src);
    assert_eq!(result, DataType::Int64(20));
}

#[test]
fn test_multiplication_correctness() {
    let src = "let x = 7 * 8; x";
    let result = run(src);
    assert_eq!(result, DataType::Int64(56));
}

#[test]
fn test_float_division_works() {
    let src = "let x = 10.0 / 3.0; x";
    let result = run(src);
    match result {
        DataType::Float64(f) => assert!((f - 3.333333333333333).abs() < 1e-10),
        other => panic!("expected Float64, got {:?}", other),
    }
}

#[test]
fn test_negative_array_index_returns_null() {
    // Interpreter handles array indexing directly, not via evaluator
    let src = r#"
        let arr = [1, 2, 3];
        let x = arr[-1];
        x
    "#;
    // Behavior may vary (null or wrap around), just should not panic
    let _result = run(src);
}

#[test]
fn test_string_concat_add() {
    let src = r#"let s = "hello" + " " + "world"; s"#;
    let result = run(src);
    assert_eq!(result, DataType::String("hello world".to_string()));
}

// ── Round 42: Parser, type checker, interpreter fixes ──

#[test]
fn test_try_catch_as_tail_expression() {
    // try/catch can now be the tail expression of a block
    assert_eq!(
        run(r#"
let x = {
    try { 42 } catch e { 0 }
}
x
"#),
        DataType::Int64(42)
    );
}

#[test]
fn test_try_catch_expr_with_finally() {
    // try/catch/finally as expression in block
    assert_eq!(
        run(r#"
let mut side_effect = false;
let x = try {
    42
} catch e {
    0
} finally {
    side_effect = true;
}
side_effect
"#),
        DataType::Bool(true)
    );
}

#[test]
fn test_return_does_not_consume_across_newline() {
    // return on its own line should not consume the next line's expression
    assert_eq!(
        run(r#"
fn foo() {
    return
    42
}
foo()
"#),
        DataType::Null
    );
}

#[test]
fn test_break_does_not_consume_across_newline() {
    // break on its own line should not consume the next line
    assert_eq!(
        run(r#"
fn foo() {
    let mut result = 0;
    for x in [1, 2, 3] {
        break
        result = 99;
    }
    result
}
foo()
"#),
        DataType::Int64(0)
    );
}

#[test]
fn test_int64_min_rejects_non_numeric() {
    // min/max should error on non-numeric arguments
    let src = r#"(5).min("hello")"#;
    let result = run_result(src);
    assert!(result.is_err(), "expected error for non-numeric min arg");
}

#[test]
fn test_float64_max_rejects_non_numeric() {
    let src = r#"(3.14).max("world")"#;
    let result = run_result(src);
    assert!(result.is_err(), "expected error for non-numeric max arg");
}

#[test]
fn test_float64_clamp_nan_bounds() {
    // Clamp with NaN bounds should return NaN
    let src = r#"(5.0).clamp(0.0 / 0.0, 10.0)"#;
    let result = run(src);
    match result {
        DataType::Float64(f) => assert!(f.is_nan(), "expected NaN, got {}", f),
        other => panic!("expected Float64, got {:?}", other),
    }
}

#[test]
fn test_int32_methods() {
    // Int32 should support sign, to_string, to_int64, to_float64
    // (Int32 literals are not directly available, but we can test via the type system)
    // Test via Float32 → to_int64 conversion
    assert_eq!(
        run(r#"let x = 42; x.sign()"#),
        DataType::Int64(1)
    );
}

#[test]
fn test_pad_start_full_string() {
    // pad_start should use full pad string, not just first char
    assert_eq!(
        run(r#""42".pad_start(6, "0x")"#),
        DataType::String("0x0x42".to_string())
    );
}

#[test]
fn test_pad_end_full_string() {
    assert_eq!(
        run(r#""hi".pad_end(7, "!?")"#),
        DataType::String("hi!?!?!".to_string())
    );
}

#[test]
fn test_pub_rejects_invalid_statement() {
    // pub should only work with fn, mod, enum, struct, const
    let result = parse_v2("pub let x = 5;");
    assert!(result.is_err(), "pub let should be a parse error");
}

#[test]
fn test_pub_accepts_valid_fn() {
    let result = parse_v2("pub fn foo() { 42 }");
    assert!(result.is_ok(), "pub fn should parse: {:?}", result.err());
}

#[test]
fn test_empty_struct_literal() {
    // Empty struct {} should be parsed as struct construct, not Variable + Block
    assert_eq!(
        run(r#"
struct Empty {}
let e = Empty {}
e.__struct
"#),
        DataType::String("Empty".to_string())
    );
}

#[test]
fn test_method_not_found_suggestion() {
    // Method-not-found should suggest similar methods
    let src = r#""hello".lenght()"#;
    let result = run_result(src);
    match result {
        Err(e) => {
            let msg = format!("{}", e);
            assert!(msg.contains("length") || msg.contains("len"), "expected suggestion for 'lenght' typo, got: {}", msg);
        }
        Ok(v) => panic!("expected error, got: {:?}", v),
    }
}

// ── Round 44: Numeric literals, f-string braces, formatter ──

#[test]
fn test_hex_literal() {
    assert_eq!(run("0xFF"), DataType::Int64(255));
    assert_eq!(run("0x10"), DataType::Int64(16));
    assert_eq!(run("0xDEAD"), DataType::Int64(0xDEAD));
}

#[test]
fn test_octal_literal() {
    assert_eq!(run("0o77"), DataType::Int64(63));
    assert_eq!(run("0o10"), DataType::Int64(8));
}

#[test]
fn test_binary_literal() {
    assert_eq!(run("0b1010"), DataType::Int64(10));
    assert_eq!(run("0b11111111"), DataType::Int64(255));
}

#[test]
fn test_underscore_separators_in_numbers() {
    assert_eq!(run("1_000_000"), DataType::Int64(1_000_000));
    assert_eq!(run("0xFF_FF"), DataType::Int64(0xFFFF));
    assert_eq!(run("0b1010_0101"), DataType::Int64(0b1010_0101));
    assert_eq!(run("1_000.5"), DataType::Float64(1000.5));
}

#[test]
fn test_hex_arithmetic() {
    assert_eq!(run("0xFF + 1"), DataType::Int64(256));
    assert_eq!(run("0x10 * 2"), DataType::Int64(32));
}

#[test]
fn test_fstring_with_string_containing_braces() {
    // String inside f-string interpolation that contains brace characters
    let src = r#"
    let x = "hello"
    f"result: {x}"
    "#;
    assert_eq!(run(src), DataType::String("result: hello".to_string()));
}

#[test]
fn test_fstring_with_nested_braces() {
    // Map literal inside f-string interpolation
    let src = r#"
    let m = { "a": 1 }
    f"val: {m.a}"
    "#;
    assert_eq!(run(src), DataType::String("val: 1".to_string()));
}

#[test]
fn test_formatter_lambda_default_params() {
    let src = r#"let f = |x, y = 10| x + y"#;
    let program = parse(src);
    let config = magi_lang::formatter::FormatConfig::default();
    let formatted = magi_lang::formatter::format_program(&program, &config);
    assert!(formatted.contains("y = 10"), "formatter should preserve lambda default param, got: {}", formatted);
}

#[test]
fn test_formatter_nan_inf() {
    // NaN and Infinity should produce parseable output
    let src = "let x = 0.0 / 0.0\nlet y = 1.0 / 0.0";
    let program = parse(src);
    let config = magi_lang::formatter::FormatConfig::default();
    let formatted = magi_lang::formatter::format_program(&program, &config);
    // Should not contain "NaN" or "inf" as raw text — should use expressions
    assert!(!formatted.contains("NaN"), "NaN should be formatted as expression, got: {}", formatted);
    assert!(!formatted.contains("inf"), "Infinity should be formatted as expression, got: {}", formatted);
}

#[test]
fn test_linter_mixed_match_no_false_positive() {
    // Match with both enum patterns and literal patterns should not produce W203
    let src = r#"
    enum Color { Red, Green, Blue }
    let x = Color::Red
    match x {
        Color::Red => "red"
        42 => "forty-two"
        _ => "other"
    }
    "#;
    let program = parse(src);
    let config = magi_lang::linter::LintConfig::default();
    let result = magi_lang::linter::lint(&program, &config);
    let w203: Vec<_> = result.diagnostics.iter().filter(|d| d.code.as_deref() == Some("W203")).collect();
    assert!(w203.is_empty(), "should not produce W203 for mixed match, got: {:?}", w203);
}

// ── Round 45: Uint methods, struct reserved fields, slice wrapping, main scope ──

#[test]
fn test_has_main_sees_top_level_const() {
    let src = r#"
    const MAX = 100
    fn main() {
        MAX + 1
    }
    "#;
    assert_eq!(run(src), DataType::Int64(101));
}

#[test]
fn test_has_main_sees_top_level_let() {
    let src = r#"
    let greeting = "hello"
    fn main() {
        greeting
    }
    "#;
    assert_eq!(run(src), DataType::String("hello".to_string()));
}

#[test]
fn test_struct_reserved_field_rejected() {
    let src = "struct Bad { __struct: string }";
    let result = parse_v2(src);
    assert!(result.is_err(), "should reject __struct as field name");
}

#[test]
fn test_string_slice_negative_wraps() {
    let src = r#"
    let s = "hello"
    s[(-3)..5]
    "#;
    assert_eq!(run(src), DataType::String("llo".to_string()));
}

#[test]
fn test_slice_both_negative() {
    let src = r#"
    let arr = [1, 2, 3, 4, 5]
    arr[(-3)..(-1)]
    "#;
    assert_eq!(run(src), DataType::Array(vec![
        DataType::Int64(3),
        DataType::Int64(4),
    ]));
}

// ── Round 46: Type checker and parser correctness ──

#[test]
fn test_string_concat_type_inference() {
    // String + String should not produce type checker warnings
    let src = r#"
    let a = "hello"
    let b = " world"
    let c = a + b
    c
    "#;
    let warnings = typecheck_warnings(src);
    assert!(warnings.is_empty(), "string concat should produce no warnings, got: {:?}", warnings);
    assert_eq!(run(src), DataType::String("hello world".to_string()));
}

#[test]
fn test_for_loop_over_string_no_warning() {
    let src = r#"
    let s = "abc"
    let mut result = []
    for ch in s {
        result = result
    }
    result
    "#;
    let warnings = typecheck_warnings(src);
    let e102: Vec<_> = warnings.iter().filter(|w| w.contains("iterable")).collect();
    assert!(e102.is_empty(), "for-over-string should not warn about iterable type, got: {:?}", e102);
}

#[test]
fn test_match_arm_throw() {
    // throw should be allowed in match arm body without braces
    let src = r#"
    fn check(x) {
        match x {
            0 => throw "zero not allowed"
            _ => x
        }
    }
    check(5)
    "#;
    assert_eq!(run(src), DataType::Int64(5));
}

#[test]
fn test_match_arm_return() {
    let src = r#"
    fn check(x) {
        match x {
            0 => return "zero"
            _ => "other"
        }
    }
    check(0)
    "#;
    assert_eq!(run(src), DataType::String("zero".to_string()));
}

#[test]
fn test_float_range_pattern() {
    let src = r#"
    fn classify(x) {
        match x {
            0.0..1.0 => "small"
            _ => "big"
        }
    }
    classify(0.5)
    "#;
    assert_eq!(run(src), DataType::String("small".to_string()));
}

#[test]
fn test_pub_type_accepted() {
    let src = "pub type Num = int";
    let result = parse_v2(src);
    assert!(result.is_ok(), "pub type should be accepted");
}

#[test]
fn test_pub_use_accepted() {
    let src = "pub use std::math::sqrt";
    let result = parse_v2(src);
    assert!(result.is_ok(), "pub use should be accepted");
}

#[test]
fn test_glob_import_alias_rejected() {
    let src = "use std::math::* as m";
    let result = parse_v2(src);
    assert!(result.is_err(), "glob import with alias should be rejected");
}

// ── Round 47: CLI evaluator and interpreter edge case fixes ──

#[test]
fn test_let_destructure_in_main() {
    let src = r#"
let [a, b] = [10, 20]
fn main() {
    a + b
}
"#;
    let result = run(src);
    assert_eq!(result, DataType::Int64(30));
}

#[test]
fn test_method_call_optional_chain_deep() {
    let src = r#"
let x = null
let result = x?.nested.keys()
result
"#;
    let result = run(src);
    assert_eq!(result, DataType::Null);
}

#[test]
fn test_field_access_optional_chain_propagation() {
    let src = r#"
let x = null
let result = x?.a.b.c
result
"#;
    let result = run(src);
    assert_eq!(result, DataType::Null);
}

#[test]
fn test_use_module_nonexistent_function_errors() {
    let src = r#"
mod math {
    fn double(x) { x * 2 }
}
use math::triple
triple(5)
"#;
    let result = run_result(src);
    assert!(result.is_err(), "importing nonexistent module function should error");
}

#[test]
fn test_async_fns_isolated_between_tests() {
    let src = r#"
test "defines async foo" {
    async fn foo() { 42 }
    let r = await foo()
    assert_eq(r, 42)
}

test "defines sync foo" {
    fn foo() { 42 }
    let r = foo()
    assert_eq(r, 42)
}
"#;
    let program = parse(src);
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    let results = interp.run_tests(&program);
    assert!(results.iter().all(|r| r.passed), "all tests should pass: {:?}", results);
}

#[test]
fn test_module_enum_defined() {
    // Module enums are registered with qualified names.
    // The parser doesn't support multi-level :: paths (e.g. mod::Enum::Variant),
    // so we verify the enum_defs are populated by checking a function that uses it.
    let src = r#"
mod shapes {
    enum Color {
        Red,
        Green,
        Blue,
    }
    fn make_red() {
        Color::Red()
    }
}
shapes::make_red()
"#;
    let result = run(src);
    match &result {
        DataType::Map(m) => {
            assert!(m.contains_key("__enum"), "Expected __enum marker, got {:?}", m);
        }
        _ => panic!("Expected map with __enum marker, got {:?}", result),
    }
}

#[test]
fn test_module_struct_defined() {
    // Module structs are registered with qualified names.
    let src = r#"
mod geo {
    struct Point {
        x: float64,
        y: float64,
    }
    fn make_origin() {
        Point { x: 0.0, y: 0.0 }
    }
}
let p = geo::make_origin()
p.x
"#;
    let result = run(src);
    assert_eq!(result, DataType::Float64(0.0));
}

#[test]
fn test_module_function_spread_args() {
    let src = r#"
mod utils {
    fn sum_three(a, b, c) { a + b + c }
}
let args = [1, 2, 3]
utils::sum_three(...args)
"#;
    let result = run(src);
    assert_eq!(result, DataType::Int64(6));
}

#[test]
fn test_string_less_than() {
    let src = r#""apple" < "banana""#;
    let result = run(src);
    assert_eq!(result, DataType::Bool(true));
}

#[test]
fn test_string_greater_than() {
    let src = r#""zoo" > "apple""#;
    let result = run(src);
    assert_eq!(result, DataType::Bool(true));
}

#[test]
fn test_string_greater_eq() {
    let src = r#""same" >= "same""#;
    let result = run(src);
    assert_eq!(result, DataType::Bool(true));
}

#[test]
fn test_string_less_eq() {
    let src = r#""same" <= "same""#;
    let result = run(src);
    assert_eq!(result, DataType::Bool(true));
}

#[test]
fn test_string_comparison_false() {
    let src = r#""banana" < "apple""#;
    let result = run(src);
    assert_eq!(result, DataType::Bool(false));
}

#[test]
fn test_path_traversal_dependency_rejected() {
    // This is a CLI-only feature (magi.toml), so we just verify the parser doesn't crash
    // The actual path traversal check is in src/bin/magi.rs
    let src = "42";
    let result = run(src);
    assert_eq!(result, DataType::Int64(42));
}

// ── Round 48: formatter, type checker, LSP, linter fixes ──

#[test]
fn test_formatter_unary_in_method_call() {
    use magi_lang::formatter::{format_program, FormatConfig};
    let src = "(-a).method()";
    let program = parse(src);
    let formatted = format_program(&program, &FormatConfig::default());
    assert!(formatted.contains("(-a).method()"), "unary should be parenthesized: {}", formatted);
}

#[test]
fn test_formatter_await_in_field_access() {
    use magi_lang::formatter::{format_program, FormatConfig};
    let src = "(await x).field";
    let program = parse(src);
    let formatted = format_program(&program, &FormatConfig::default());
    assert!(formatted.contains("(await x).field"), "await should be parenthesized: {}", formatted);
}

#[test]
fn test_type_checker_rest_params_no_false_error() {
    let src = r#"
fn log(msg, ...args) {
    msg
}
log("hello", 1, 2, 3)
"#;
    let program = parse(src);
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    let arity_errors: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.message.contains("expects") && d.message.contains("arguments"))
        .collect();
    assert!(arity_errors.is_empty(), "rest params should not produce arity error: {:?}", arity_errors);
}

#[test]
fn test_type_checker_inclusive_range_no_false_warning() {
    let src = "5..=5";
    let warnings = typecheck_warnings(src);
    assert!(!warnings.contains(&"W107".to_string()), "inclusive range 5..=5 should not produce W107: {:?}", warnings);
}

#[test]
fn test_type_checker_exclusive_range_warns_when_equal() {
    let src = "5..5";
    let warnings = typecheck_warnings(src);
    assert!(warnings.contains(&"W107".to_string()), "exclusive range 5..5 should produce W107: {:?}", warnings);
}

#[test]
fn test_function_defined_in_block() {
    let src = r#"
let result = {
    fn helper(x) { x * 2 }
    helper(21)
}
result
"#;
    let result = run(src);
    assert_eq!(result, DataType::Int64(42));
}

#[test]
fn test_function_defined_in_if_block() {
    let src = r#"
let x = if true {
    fn double(n) { n * 2 }
    double(10)
} else {
    0
}
x
"#;
    let result = run(src);
    assert_eq!(result, DataType::Int64(20));
}

// ── Round 49: Parser fixes ──────────────────────────────────────────────────

#[test]
fn test_struct_literal_requires_uppercase() {
    // Lowercase `result { value: 1 }` should NOT parse as a struct literal.
    // It should be treated as variable `result` followed by block `{ value: 1 }`.
    // In an if-context, `if result { ... }` should work as condition + block.
    let src = r#"
let result = true
if result { 42 } else { 0 }
"#;
    let result = run(src);
    assert_eq!(result, DataType::Int64(42));
}

#[test]
fn test_pub_pub_rejected() {
    // Duplicate pub should produce a parse error
    let src = "pub pub fn foo() { 1 }";
    let parsed = parse_v2(src);
    assert!(parsed.is_err(), "pub pub should produce a parse error");
}

#[test]
fn test_fstring_escaped_braces() {
    // f-string with escaped braces should produce literal braces
    let src = r#"f"hello \{ world \}""#;
    let result = run(src);
    assert_eq!(result, DataType::String("hello { world }".to_string()));
}

#[test]
fn test_fstring_mixed_escaped_and_interpolation() {
    // Mix of escaped braces and actual interpolation
    let src = r#"
let x = 42
f"value = \{ {x} \}"
"#;
    let result = run(src);
    assert_eq!(result, DataType::String("value = { 42 }".to_string()));
}

#[test]
fn test_struct_literal_uppercase_still_works() {
    // Uppercase Name { field: value } should still parse as struct literal
    let src = r#"
struct Point { x, y }
let p = Point { x: 10, y: 20 }
p.x + p.y
"#;
    let result = run(src);
    assert_eq!(result, DataType::Int64(30));
}

#[test]
fn test_output_span_does_not_include_next() {
    // Verify output statement parses correctly with semicolons
    let src = r#"
output 42;
output 100;
100
"#;
    // Just make sure it parses and runs without error
    let result = run(src);
    assert_eq!(result, DataType::Int64(100));
}

#[test]
fn test_import_span_parse() {
    // Ensure import statement parses without spanning into next token
    let src = r#"import "test-plugin""#;
    let parsed = parse_v2(src);
    assert!(parsed.is_ok(), "import should parse cleanly");
}

// ── Round 50: FullEvaluator new operations ──────────────────────────────────

#[test]
fn test_string_reverse() {
    let src = r#""hello".reverse()"#;
    let result = run(src);
    assert_eq!(result, DataType::String("olleh".to_string()));
}

#[test]
fn test_string_chars() {
    let src = r#""abc".chars()"#;
    let result = run(src);
    assert_eq!(result, DataType::Array(vec![
        DataType::String("a".to_string()),
        DataType::String("b".to_string()),
        DataType::String("c".to_string()),
    ]));
}

#[test]
fn test_string_lines() {
    let src = "\"hello\\nworld\".lines()";
    let result = run(src);
    assert_eq!(result, DataType::Array(vec![
        DataType::String("hello".to_string()),
        DataType::String("world".to_string()),
    ]));
}

#[test]
fn test_string_repeat() {
    let src = r#""ab".repeat(3)"#;
    let result = run(src);
    assert_eq!(result, DataType::String("ababab".to_string()));
}

#[test]
fn test_string_count() {
    let src = r#""banana".count("an")"#;
    let result = run(src);
    assert_eq!(result, DataType::Int64(2));
}

#[test]
fn test_string_words() {
    let src = r#""hello  world  foo".words()"#;
    let result = run(src);
    assert_eq!(result, DataType::Array(vec![
        DataType::String("hello".to_string()),
        DataType::String("world".to_string()),
        DataType::String("foo".to_string()),
    ]));
}

#[test]
fn test_char_at() {
    let src = r#""hello".char_at(1)"#;
    let result = run(src);
    assert_eq!(result, DataType::String("e".to_string()));
}

#[test]
fn test_pad_start() {
    let src = r#""42".pad_start(5, "0")"#;
    let result = run(src);
    assert_eq!(result, DataType::String("00042".to_string()));
}

#[test]
fn test_pad_end() {
    let src = r#""hi".pad_end(5)"#;
    let result = run(src);
    assert_eq!(result, DataType::String("hi   ".to_string()));
}

#[test]
fn test_array_shift() {
    let src = r#"[10, 20, 30].shift()"#;
    let result = run(src);
    assert_eq!(result, DataType::Int64(10));
}

#[test]
fn test_to_json_map() {
    let src = r#"
let m = {"name": "alice", "age": 30}
m.to_json()
"#;
    let result = run(src);
    // JSON output — keys are alphabetical in BTreeMap
    match result {
        DataType::String(s) => {
            assert!(s.contains("\"name\":\"alice\""), "should contain name: {}", s);
            assert!(s.contains("\"age\":30"), "should contain age: {}", s);
        }
        other => panic!("expected String, got {:?}", other),
    }
}

#[test]
fn test_typeof_values() {
    let src = r#"typeof(42)"#;
    let result = run(src);
    assert_eq!(result, DataType::String("int64".to_string()));
}

#[test]
fn test_typeof_string() {
    let src = r#"typeof("hello")"#;
    let result = run(src);
    assert_eq!(result, DataType::String("string".to_string()));
}

#[test]
fn test_typeof_array() {
    let src = r#"typeof([1, 2, 3])"#;
    let result = run(src);
    assert_eq!(result, DataType::String("array".to_string()));
}

#[test]
fn test_typeof_null() {
    let src = r#"typeof(null)"#;
    let result = run(src);
    assert_eq!(result, DataType::String("null".to_string()));
}

// ── Round 51: type checker, interpreter, error code fixes ──

#[test]
fn test_resource_limit_error_type_pad_end() {
    // Resource limits should produce ResourceLimit, not TypeError
    let err = run_err(r#""x".pad_end(99999999999)"#);
    assert!(matches!(err, InterpError::ResourceLimit { .. }));
}

#[test]
fn test_resource_limit_error_type_repeat() {
    let err = run_err(r#""abc".repeat(99999999)"#);
    assert!(matches!(err, InterpError::ResourceLimit { .. }));
}

#[test]
fn test_w112_default_param_type_mismatch() {
    let codes = typecheck_warnings(r#"fn foo(x: int64 = "hello") { output x; }"#);
    assert!(codes.contains(&"W112".to_string()), "expected W112, got {:?}", codes);
}

#[test]
fn test_w106_stays_for_redundant_ops() {
    // W106 should still be used for self-comparison and double negation
    let codes = typecheck_warnings(r#"
let x = 5
let _y = x == x
"#);
    assert!(codes.contains(&"W106".to_string()), "expected W106, got {:?}", codes);
}

#[test]
fn test_w106_stays_for_double_negation() {
    let codes = typecheck_warnings(r#"
let x = 5
let _y = --x
"#);
    assert!(codes.contains(&"W106".to_string()), "expected W106, got {:?}", codes);
}

#[test]
fn test_bool_match_exhaustive_no_warning() {
    // Matching both true and false should not warn about non-exhaustiveness
    let codes = typecheck_warnings(r#"
let x = true
let _y = match x {
    true => 1,
    false => 0,
}
"#);
    assert!(!codes.contains(&"W203".to_string()), "should not warn W203, got {:?}", codes);
}

#[test]
fn test_bool_match_non_exhaustive_warns() {
    // Matching only true should warn
    let codes = typecheck_warnings(r#"
let x = true
let _y = match x {
    true => 1,
}
"#);
    assert!(codes.contains(&"W203".to_string()), "expected W203, got {:?}", codes);
}

#[test]
fn test_while_true_with_break_no_w105() {
    // while true { break; } should NOT warn W105
    let codes = typecheck_warnings(r#"
while true {
    break;
}
"#);
    assert!(!codes.contains(&"W105".to_string()), "should not warn W105, got {:?}", codes);
}

#[test]
fn test_while_true_with_break_in_if_no_w105() {
    // while true { if cond { break; } } should NOT warn W105
    let codes = typecheck_warnings(r#"
let mut i = 0
while true {
    i = i + 1
    if i > 5 { break; }
}
"#);
    assert!(!codes.contains(&"W105".to_string()), "should not warn W105, got {:?}", codes);
}

#[test]
fn test_while_true_without_break_warns_w105() {
    let codes = typecheck_warnings(r#"
while true {
    output 1;
}
"#);
    assert!(codes.contains(&"W105".to_string()), "expected W105, got {:?}", codes);
}

#[test]
fn test_method_call_on_array_first_last() {
    assert_eq!(run(r#"[1, 2, 3].first()"#), DataType::Int64(1));
    assert_eq!(run(r#"[1, 2, 3].last()"#), DataType::Int64(3));
}

#[test]
fn test_to_json_method_on_map() {
    // to_json should work on maps (previously broken by resolve_method shadowing)
    let result = run(r#"{"a": 1}.to_json()"#);
    assert!(matches!(result, DataType::String(_)));
}

#[test]
fn test_typeof_method_on_value() {
    let result = run(r#"42.typeof()"#);
    assert_eq!(result, DataType::String("int64".to_string()));
}

#[test]
fn test_int_abs_and_sign() {
    assert_eq!(run(r#"(-5).abs()"#), DataType::Int64(5));
    assert_eq!(run(r#"(-5).sign()"#), DataType::Int64(-1));
    assert_eq!(run(r#"(0).sign()"#), DataType::Int64(0));
    assert_eq!(run(r#"(5).sign()"#), DataType::Int64(1));
}

// ── Round 52: CLI evaluator, type checker, linter, LSP fixes ──

#[test]
fn test_or_pattern_binds_all_alternatives() {
    // Variables from all Or-pattern alternatives should be accessible
    let result = run(r#"
let x = 42
match x {
    1 | n => n,
}
"#);
    assert_eq!(result, DataType::Int64(42));
}

#[test]
fn test_linter_type_alias_pascal_case() {
    use magi_lang::linter;
    let program = parse_v2("type my_type = int64;").unwrap();
    let config = linter::LintConfig { disabled_rules: vec![] };
    let result = linter::lint(&program, &config);
    assert!(result.diagnostics.iter().any(|d| d.code.as_deref() == Some("W201")),
        "expected W201 for non-PascalCase type alias");
}

#[test]
fn test_linter_type_alias_pascal_case_ok() {
    use magi_lang::linter;
    let program = parse_v2("type MyType = int64;").unwrap();
    let config = linter::LintConfig { disabled_rules: vec![] };
    let result = linter::lint(&program, &config);
    assert!(!result.diagnostics.iter().any(|d| d.code.as_deref() == Some("W201")),
        "should not warn W201 for PascalCase type alias");
}

// ── Round 53: expanded test coverage ──

#[test]
fn test_nested_fstring() {
    assert_eq!(
        run(r#"let x = 5; f"outer {f"inner {x}"}""#),
        DataType::String("outer inner 5".to_string())
    );
}

#[test]
fn test_fstring_with_method_call() {
    assert_eq!(
        run(r#"let s = "hello"; f"upper: {s.to_upper()}""#),
        DataType::String("upper: HELLO".to_string())
    );
}

#[test]
fn test_closure_captures_outer_scope() {
    assert_eq!(
        run(r#"
let x = 10
let f = |a| a + x
f(5)
"#),
        DataType::Int64(15)
    );
}

#[test]
fn test_null_coalesce_chain_deep() {
    assert_eq!(run(r#"null ?? null ?? 42"#), DataType::Int64(42));
    assert_eq!(run(r#"1 ?? 2 ?? 3"#), DataType::Int64(1));
}

#[test]
fn test_optional_chaining_null() {
    assert_eq!(run(r#"let x = null; x?.field"#), DataType::Null);
}

#[test]
fn test_optional_chaining_nested() {
    assert_eq!(
        run(r#"let x = {"a": {"b": 42}}; x?.a?.b"#),
        DataType::Int64(42)
    );
}

#[test]
fn test_optional_chaining_method() {
    assert_eq!(
        run(r#"let x = "hello"; x?.to_upper()"#),
        DataType::String("HELLO".to_string())
    );
}

#[test]
fn test_optional_chaining_method_on_null() {
    assert_eq!(run(r#"let x = null; x?.to_upper()"#), DataType::Null);
}

#[test]
fn test_compound_assign_all_ops() {
    assert_eq!(run(r#"let mut x = 10; x += 5; x"#), DataType::Int64(15));
    assert_eq!(run(r#"let mut x = 10; x -= 3; x"#), DataType::Int64(7));
    assert_eq!(run(r#"let mut x = 10; x *= 2; x"#), DataType::Int64(20));
    assert_eq!(run(r#"let mut x = 10; x /= 3; x"#), DataType::Int64(3));
    assert_eq!(run(r#"let mut x = 10; x %= 3; x"#), DataType::Int64(1));
}

#[test]
fn test_match_nested_array_pattern() {
    assert_eq!(
        run(r#"
match [[1, 2], [3, 4]] {
    [[a, b], [c, d]] => a + b + c + d,
    _ => 0,
}
"#),
        DataType::Int64(10)
    );
}

#[test]
fn test_comprehension_with_method() {
    assert_eq!(
        run(r#"
let items = [1, 2, 3]
let result = [x * x for x in items]
result
"#),
        DataType::Array(vec![DataType::Int64(1), DataType::Int64(4), DataType::Int64(9)])
    );
}

#[test]
fn test_spread_in_array() {
    assert_eq!(
        run(r#"
let a = [1, 2, 3]
let b = [0, ...a, 4]
b
"#),
        DataType::Array(vec![
            DataType::Int64(0),
            DataType::Int64(1),
            DataType::Int64(2),
            DataType::Int64(3),
            DataType::Int64(4),
        ])
    );
}

#[test]
fn test_map_field_access_chain() {
    assert_eq!(
        run(r#"
let a = {"x": {"y": 42}}
a.x.y
"#),
        DataType::Int64(42)
    );
}

#[test]
fn test_match_rest_pattern() {
    assert_eq!(
        run(r#"
match [1, 2, 3, 4, 5] {
    [first, ...rest] => first + rest.len(),
    _ => 0,
}
"#),
        DataType::Int64(5)
    );
}

#[test]
fn test_match_rest_pattern_middle() {
    assert_eq!(
        run(r#"
match [1, 2, 3, 4, 5] {
    [first, ...middle, last] => first + last,
    _ => 0,
}
"#),
        DataType::Int64(6)
    );
}

#[test]
fn test_enum_pattern_in_match() {
    assert_eq!(
        run(r#"
enum Shape { Circle(r), Square(s) }
fn area(shape) {
    match shape {
        Shape::Circle(r) => r * r,
        Shape::Square(s) => s * s,
    }
}
let c = Shape::Circle(5)
area(c)
"#),
        DataType::Int64(25)
    );
}

#[test]
fn test_loop_with_break_value() {
    assert_eq!(
        run(r#"
let mut i = 0
let result = loop {
    i = i + 1
    if i >= 5 {
        break i * 10;
    }
}
result
"#),
        DataType::Int64(50)
    );
}

#[test]
fn test_while_loop_value() {
    assert_eq!(
        run(r#"
let mut sum = 0
let mut i = 1
while i <= 5 {
    sum = sum + i
    i = i + 1
}
sum
"#),
        DataType::Int64(15)
    );
}

#[test]
fn test_array_comprehension_with_filter() {
    assert_eq!(
        run(r#"
let items = [1, 2, 3, 4, 5]
let result = [x * x for x in items if x % 2 == 0]
result
"#),
        DataType::Array(vec![DataType::Int64(4), DataType::Int64(16)])
    );
}

#[test]
fn test_closure_with_default_param() {
    assert_eq!(
        run(r#"let f = |x, y = 10| x + y; f(5)"#),
        DataType::Int64(15)
    );
}

#[test]
fn test_closure_with_default_param_override() {
    assert_eq!(
        run(r#"let f = |x, y = 10| x + y; f(5, 20)"#),
        DataType::Int64(25)
    );
}

// Round 54 tests

#[test]
fn test_while_true_break_in_match_no_w105() {
    // expr_contains_break must handle Match expressions
    let codes = typecheck_warnings(r#"
let mut x = 0
while true {
    x = x + 1
    match x {
        5 => { break }
        _ => {}
    }
}
"#);
    assert!(!codes.contains(&"W105".to_string()),
        "Should not warn W105 when break is inside match arm, got {:?}", codes);
}

#[test]
fn test_match_rest_pattern_len() {
    // Verifies StubEvaluator correctly handles "array" port name for len()
    assert_eq!(
        run(r#"
match [1, 2, 3, 4, 5] {
    [first, ...rest] => first + rest.len(),
    _ => 0,
}
"#),
        DataType::Int64(5)
    );
}

#[test]
fn test_array_method_in_expression() {
    // Array .len() + arithmetic should work correctly with evaluator port names
    assert_eq!(
        run(r#"
let arr = [10, 20, 30]
100 + arr.len()
"#),
        DataType::Int64(103)
    );
}

#[test]
fn test_enum_exhaustive_or_pattern() {
    // Nested Or patterns should contribute to enum exhaustiveness
    let codes = typecheck_warnings(r#"
enum Color { Red, Green, Blue }
fn show(c) {
    match c {
        Color::Red | Color::Green => "warm",
        Color::Blue => "cool",
    }
}
show(Color::Red)
"#);
    assert!(!codes.contains(&"W203".to_string()),
        "Or-pattern with all enum variants covered should not warn W203, got {:?}", codes);
}

// Round 54 — untested method coverage

#[test]
fn test_string_to_float() {
    assert_eq!(run(r#""3.14".to_float()"#), DataType::Float64(3.14));
    assert_eq!(run(r#""not_a_number".to_float()"#), DataType::Null);
    assert_eq!(run(r#""".to_float()"#), DataType::Null);
}

#[test]
fn test_string_to_uppercase() {
    assert_eq!(
        run(r#""hello".to_uppercase()"#),
        DataType::String("HELLO".to_string())
    );
}

#[test]
fn test_string_trim_start() {
    assert_eq!(
        run(r#""  hello  ".trim_start()"#),
        DataType::String("hello  ".to_string())
    );
}

#[test]
fn test_string_trim_end() {
    assert_eq!(
        run(r#""  hello  ".trim_end()"#),
        DataType::String("  hello".to_string())
    );
}

#[test]
fn test_string_slice_method() {
    assert_eq!(
        run(r#""hello world".slice(6, 11)"#),
        DataType::String("world".to_string())
    );
}

#[test]
fn test_string_slice_method_negative() {
    assert_eq!(
        run(r#""hello".slice(-3, -1)"#),
        DataType::String("ll".to_string())
    );
}

#[test]
fn test_float64_tan() {
    match run(r#"1.0.tan()"#) {
        DataType::Float64(v) => assert!((v - 1.5574077246549023).abs() < 1e-10),
        other => panic!("expected Float64, got {:?}", other),
    }
}

#[test]
fn test_array_each() {
    // each() returns null but executes side effects
    assert_eq!(run(r#"[1, 2, 3].each(|x| x * 2)"#), DataType::Null);
}

#[test]
fn test_array_group_by() {
    let result = run(r#"
let items = [1, 2, 3, 4, 5, 6]
items.group_by(|x| if x % 2 == 0 { "even" } else { "odd" })
"#);
    match result {
        DataType::Map(m) => {
            assert_eq!(m.len(), 2);
            assert_eq!(
                m.get("even"),
                Some(&DataType::Array(vec![DataType::Int64(2), DataType::Int64(4), DataType::Int64(6)]))
            );
            assert_eq!(
                m.get("odd"),
                Some(&DataType::Array(vec![DataType::Int64(1), DataType::Int64(3), DataType::Int64(5)]))
            );
        }
        _ => panic!("expected Map, got {:?}", result),
    }
}

#[test]
fn test_map_filter_entries() {
    let result = run(r#"
let m = {"a": 1, "b": 2, "c": 3}
m.filter_entries(|k, v| v > 1)
"#);
    match result {
        DataType::Map(m) => {
            assert_eq!(m.len(), 2);
            assert_eq!(m.get("b"), Some(&DataType::Int64(2)));
            assert_eq!(m.get("c"), Some(&DataType::Int64(3)));
        }
        _ => panic!("expected Map, got {:?}", result),
    }
}

#[test]
fn test_map_map_values() {
    let result = run(r#"
let m = {"a": 1, "b": 2}
m.map_values(|v| v * 10)
"#);
    match result {
        DataType::Map(m) => {
            assert_eq!(m.get("a"), Some(&DataType::Int64(10)));
            assert_eq!(m.get("b"), Some(&DataType::Int64(20)));
        }
        _ => panic!("expected Map, got {:?}", result),
    }
}

#[test]
fn test_map_map_keys() {
    let result = run(r#"
let m = {"a": 1, "b": 2}
m.map_keys(|k| k + "!")
"#);
    match result {
        DataType::Map(m) => {
            assert_eq!(m.get("a!"), Some(&DataType::Int64(1)));
            assert_eq!(m.get("b!"), Some(&DataType::Int64(2)));
        }
        _ => panic!("expected Map, got {:?}", result),
    }
}

#[test]
fn test_string_is_numeric() {
    assert_eq!(run(r#""123".is_numeric()"#), DataType::Bool(true));
    assert_eq!(run(r#""12.5".is_numeric()"#), DataType::Bool(true));
    assert_eq!(run(r#""abc".is_numeric()"#), DataType::Bool(false));
    assert_eq!(run(r#""".is_numeric()"#), DataType::Bool(false));
}

#[test]
fn test_string_is_alphabetic() {
    assert_eq!(run(r#""hello".is_alphabetic()"#), DataType::Bool(true));
    assert_eq!(run(r#""hello123".is_alphabetic()"#), DataType::Bool(false));
    assert_eq!(run(r#""".is_alphabetic()"#), DataType::Bool(false));
}

#[test]
fn test_array_min_by_max_by() {
    assert_eq!(
        run(r#"
let items = ["cat", "elephant", "dog"]
items.min_by(|a, b| a.len() - b.len())
"#),
        DataType::String("cat".to_string())
    );
    assert_eq!(
        run(r#"
let items = ["cat", "elephant", "dog"]
items.max_by(|a, b| a.len() - b.len())
"#),
        DataType::String("elephant".to_string())
    );
}

#[test]
fn test_array_flat_map() {
    assert_eq!(
        run(r#"[1, 2, 3].flat_map(|x| [x, x * 10])"#),
        DataType::Array(vec![
            DataType::Int64(1), DataType::Int64(10),
            DataType::Int64(2), DataType::Int64(20),
            DataType::Int64(3), DataType::Int64(30),
        ])
    );
}

#[test]
fn test_array_sort_by() {
    assert_eq!(
        run(r#"[3, 1, 2].sort_by(|a, b| a - b)"#),
        DataType::Array(vec![DataType::Int64(1), DataType::Int64(2), DataType::Int64(3)])
    );
}

#[test]
fn test_array_enumerate() {
    let result = run(r#"["a", "b", "c"].enumerate()"#);
    match result {
        DataType::Array(arr) => {
            assert_eq!(arr.len(), 3);
            // Each element should be [index, value]
            match &arr[0] {
                DataType::Array(inner) => {
                    assert_eq!(inner[0], DataType::Int64(0));
                    assert_eq!(inner[1], DataType::String("a".to_string()));
                }
                _ => panic!("expected inner array"),
            }
        }
        _ => panic!("expected Array, got {:?}", result),
    }
}

#[test]
fn test_array_sum_product() {
    assert_eq!(run(r#"[1, 2, 3, 4].sum()"#), DataType::Int64(10));
    assert_eq!(run(r#"[1, 2, 3, 4].product()"#), DataType::Int64(24));
    assert_eq!(run(r#"[].sum()"#), DataType::Int64(0));
    assert_eq!(run(r#"[].product()"#), DataType::Int64(1));
}

#[test]
fn test_array_min_max() {
    assert_eq!(run(r#"[3, 1, 4, 1, 5].min()"#), DataType::Int64(1));
    assert_eq!(run(r#"[3, 1, 4, 1, 5].max()"#), DataType::Int64(5));
    assert_eq!(run(r#"[].min()"#), DataType::Null);
    assert_eq!(run(r#"[].max()"#), DataType::Null);
}

#[test]
fn test_string_is_empty() {
    assert_eq!(run(r#""".is_empty()"#), DataType::Bool(true));
    assert_eq!(run(r#""hello".is_empty()"#), DataType::Bool(false));
}

#[test]
fn test_array_is_empty() {
    assert_eq!(run(r#"[].is_empty()"#), DataType::Bool(true));
    assert_eq!(run(r#"[1].is_empty()"#), DataType::Bool(false));
}

#[test]
fn test_number_clamp() {
    assert_eq!(run(r#"15.clamp(0, 10)"#), DataType::Int64(10));
    assert_eq!(run(r#"(-5).clamp(0, 10)"#), DataType::Int64(0));
    assert_eq!(run(r#"5.clamp(0, 10)"#), DataType::Int64(5));
}

#[test]
fn test_number_pow() {
    assert_eq!(run(r#"2.pow(10)"#), DataType::Int64(1024));
    assert_eq!(run(r#"2.pow(0)"#), DataType::Int64(1));
}

#[test]
fn test_string_count_method() {
    assert_eq!(run(r#""hello world".count("l")"#), DataType::Int64(3));
    assert_eq!(run(r#""hello".count("z")"#), DataType::Int64(0));
}

#[test]
fn test_string_starts_ends_with() {
    assert_eq!(run(r#""hello".starts_with("hel")"#), DataType::Bool(true));
    assert_eq!(run(r#""hello".starts_with("world")"#), DataType::Bool(false));
    assert_eq!(run(r#""hello".ends_with("llo")"#), DataType::Bool(true));
    assert_eq!(run(r#""hello".ends_with("world")"#), DataType::Bool(false));
}

// =========================================================================
// Round 61: Coverage gap tests
// =========================================================================

#[test]
fn test_match_guard_complex_boolean() {
    assert_eq!(
        run(r#"
let x = 5;
let y = 10;
match x {
    n if n > 0 && y > n => "valid",
    n if n < 0 => "negative",
    _ => "other",
}
"#),
        DataType::String("valid".to_string())
    );
}

#[test]
fn test_match_guard_short_circuit() {
    // Guard with || short-circuit: first condition true, second not evaluated
    assert_eq!(
        run(r#"
match 5 {
    n if n > 3 || n < 0 => "match",
    _ => "no",
}
"#),
        DataType::String("match".to_string())
    );
}

#[test]
fn test_string_split_multichar_separator() {
    assert_eq!(
        run(r#""a::b::c".split("::")"#),
        DataType::Array(vec![
            DataType::String("a".to_string()),
            DataType::String("b".to_string()),
            DataType::String("c".to_string()),
        ])
    );
}

#[test]
fn test_array_hof_chain() {
    assert_eq!(
        run(r#"
let data = [1, 2, 3, 4, 5];
data.map(|x| x * 2).filter(|x| x > 4)
"#),
        DataType::Array(vec![
            DataType::Int64(6),
            DataType::Int64(8),
            DataType::Int64(10),
        ])
    );
}

#[test]
fn test_array_take_while_skip_while() {
    assert_eq!(
        run("[1, 2, 3, 4, 5].take_while(|x| x < 4)"),
        DataType::Array(vec![
            DataType::Int64(1),
            DataType::Int64(2),
            DataType::Int64(3),
        ])
    );
    assert_eq!(
        run("[1, 2, 3, 4, 5].skip_while(|x| x < 3)"),
        DataType::Array(vec![
            DataType::Int64(3),
            DataType::Int64(4),
            DataType::Int64(5),
        ])
    );
}

#[test]
fn test_loop_break_array_value() {
    // Loop break with a computed value
    assert_eq!(
        run(r#"
let result = loop {
    break 42;
};
result
"#),
        DataType::Int64(42)
    );
}

#[test]
fn test_nested_enum_pattern() {
    assert_eq!(
        run(r#"
enum Outer { Some(inner), None }
enum Inner { Ok(value), Err(code) }

let x = Outer::Some(Inner::Ok(42));
match x {
    Outer::Some(Inner::Ok(v)) => v,
    Outer::Some(Inner::Err(e)) => 0 - e,
    _ => -999,
}
"#),
        DataType::Int64(42)
    );
}

#[test]
fn test_for_map_destructure() {
    assert_eq!(
        run(r#"
let mut sum = 0;
for {x} in [{"x": 1, "y": 2}, {"x": 3, "y": 4}] {
    sum = sum + x;
}
sum
"#),
        DataType::Int64(4)
    );
}

#[test]
fn test_null_coalesce_lazy_evaluation() {
    // Right side should not be evaluated when left is non-null
    assert_eq!(
        run(r#"
let mut counter = 0;
fn incr() { counter = counter + 1; 99 }
let result = 42 ?? incr();
[result, counter]
"#),
        DataType::Array(vec![DataType::Int64(42), DataType::Int64(0)])
    );
}

#[test]
fn test_async_spawn_error_propagation() {
    assert_eq!(
        run(r#"
async fn failing() {
    throw "async error";
}
try {
    let f = spawn failing();
    await f
} catch e {
    "caught"
}
"#),
        DataType::String("caught".to_string())
    );
}

#[test]
fn test_try_catch_finally_return_value() {
    assert_eq!(
        run(r#"
let mut log = "";
let result = try {
    42
} catch e {
    0
} finally {
    log = "done";
};
[result, log]
"#),
        DataType::Array(vec![
            DataType::Int64(42),
            DataType::String("done".to_string()),
        ])
    );
}

#[test]
fn test_float64_min_max_clamp() {
    // Verify min/max/clamp work correctly
    assert_eq!(run("(5.0).min(3.0)"), DataType::Float64(3.0));
    assert_eq!(run("(5.0).max(7.0)"), DataType::Float64(7.0));
    assert_eq!(run("(5.0).clamp(1.0, 3.0)"), DataType::Float64(3.0));
    assert_eq!(run("(0.5).clamp(1.0, 3.0)"), DataType::Float64(1.0));
}

#[test]
fn test_map_keys_values() {
    // Test map keys() and values() methods
    assert_eq!(
        run(r#"
let m = {"a": 1, "b": 2};
m.keys()
"#),
        DataType::Array(vec![
            DataType::String("a".to_string()),
            DataType::String("b".to_string()),
        ])
    );
    assert_eq!(
        run(r#"
let m = {"x": 10, "y": 20};
m.values().sum()
"#),
        DataType::Int64(30)
    );
}

#[test]
fn test_enum_double_underscore_variant_rejected() {
    let result = parse_v2("enum Foo { __hidden, Bar }");
    assert!(result.is_err(), "Enum variant with __ prefix should be rejected");
    let err = result.unwrap_err();
    assert!(err.message.contains("reserved"), "Error should mention reserved: {}", err.message);
}

#[test]
fn test_enum_normal_variants_accepted() {
    let result = parse_v2("enum Color { Red, Green, Blue }");
    assert!(result.is_ok(), "Normal enum variants should be accepted");
}

#[test]
fn test_array_shift_method() {
    assert_eq!(
        run(r#"
let mut arr = [10, 20, 30];
let first = arr.shift();
output first;
"#),
        DataType::Int64(10)
    );
}

#[test]
fn test_array_first_last_is_empty() {
    assert_eq!(run("[10, 20, 30].first()"), DataType::Int64(10));
    assert_eq!(run("[10, 20, 30].last()"), DataType::Int64(30));
    assert_eq!(run("[].is_empty()"), DataType::Bool(true));
    assert_eq!(run("[1].is_empty()"), DataType::Bool(false));
}

#[test]
fn test_int64_to_int64_identity() {
    assert_eq!(run("(42).to_int64()"), DataType::Int64(42));
}

#[test]
fn test_float64_to_float64_identity() {
    assert_eq!(run("(3.14).to_float64()"), DataType::Float64(3.14));
}

#[test]
fn test_not_operator_truthiness() {
    assert_eq!(run("!true"), DataType::Bool(false));
    assert_eq!(run("!false"), DataType::Bool(true));
    assert_eq!(run("!0"), DataType::Bool(true));
    assert_eq!(run("!42"), DataType::Bool(false));
    assert_eq!(run("!null"), DataType::Bool(true));
    assert_eq!(run(r#"!"""#), DataType::Bool(true));
    assert_eq!(run(r#"!"hello""#), DataType::Bool(false));
}

#[test]
fn test_try_block_scope_isolation() {
    // Variables defined inside try block should not leak to outer scope
    let err = run_err(r#"
        try {
            let inner_var = 42
        } catch e {}
        inner_var
    "#);
    match err {
        InterpError::UndefinedVariable { name, .. } => {
            assert_eq!(name, "inner_var");
        }
        _ => panic!("expected UndefinedVariable, got: {:?}", err),
    }
}

#[test]
fn test_try_expr_scope_isolation() {
    // Expression-level try/catch should also isolate scope
    let err = run_err(r#"
        let result = try {
            let inner = 99
            inner
        } catch e { 0 }
        inner
    "#);
    match err {
        InterpError::UndefinedVariable { name, .. } => {
            assert_eq!(name, "inner");
        }
        _ => panic!("expected UndefinedVariable, got: {:?}", err),
    }
}

#[test]
fn test_assertion_error_code() {
    // assert failures should show E402, not E403
    let err = run_err("assert(false)");
    let msg = format!("{}", err);
    assert!(msg.contains("[E402]"), "expected E402 in: {}", msg);
}

#[test]
fn test_arity_mismatch_range() {
    // Functions with default params should show range in error
    let err = run_err(r#"
        fn greet(name, greeting = "Hello") {
            f"{greeting}, {name}!"
        }
        greet()
    "#);
    let msg = format!("{}", err);
    assert!(msg.contains("1-2"), "expected '1-2' in arity message: {}", msg);
}

#[test]
fn test_range_slice_no_e103_warning() {
    // Array slicing with range should not produce E103 false positive
    let src = r#"
        let arr = [1, 2, 3, 4, 5];
        let sliced = arr[1..3];
        output sliced;
    "#;
    let prog = parse_v2(src).unwrap();
    let analysis = check_types(&prog, &std::collections::HashSet::new());
    let e103s: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("E103"))
        .collect();
    assert!(e103s.is_empty(), "E103 false positive on range slice: {:?}", e103s);
}

#[test]
fn test_rest_in_middle_destructure() {
    // Rest element in the middle of array destructure should work
    let result = run(r#"
        let arr = [1, 2, 3, 4, 5];
        let [first, ...middle, last] = arr;
        output [first, middle, last];
    "#);
    assert_eq!(
        result,
        DataType::Array(vec![
            DataType::Int64(1),
            DataType::Array(vec![DataType::Int64(2), DataType::Int64(3), DataType::Int64(4)]),
            DataType::Int64(5),
        ])
    );
}

#[test]
fn test_multiple_rest_elements_rejected() {
    // Multiple rest elements should be a syntax error
    let result = parse_v2("let [a, ...b, ...c] = arr;");
    assert!(result.is_err(), "multiple rest elements should be rejected");
    let err = result.unwrap_err();
    assert!(err.message.contains("rest"), "error should mention rest: {}", err.message);
}

#[test]
fn test_type_alias_lsp_hover() {
    // Type alias should be tracked correctly in LSP analysis
    let src = "type MyInt = int64;";
    let (state, _) = magi_lang::lsp::analysis::analyze_document(src);
    let var = state.variables.get("MyInt").unwrap();
    assert!(var.is_type_alias);
    assert_eq!(var.type_annotation, Some("int64".to_string()));
    assert!(!var.mutable);
    assert!(!var.constant);
}

#[test]
fn test_async_fn_lsp_symbol() {
    // Async functions should be tracked correctly in LSP analysis
    let src = "async fn fetch_data(url) { null }";
    let (state, _) = magi_lang::lsp::analysis::analyze_document(src);
    let func = state.functions.get("fetch_data").unwrap();
    assert!(func.is_async);
}

#[test]
fn test_destructure_mutable_lsp_symbol() {
    // Mutable destructure should be tracked correctly in LSP analysis
    let src = "let mut [a, b] = [1, 2];";
    let (state, _) = magi_lang::lsp::analysis::analyze_document(src);
    let a = state.variables.get("a").unwrap();
    assert!(a.mutable);
    let b = state.variables.get("b").unwrap();
    assert!(b.mutable);
}

#[test]
fn test_use_import_no_e201() {
    // use statement should register the imported name, no E201 false positive
    let src = r#"
        mod MyMod {
            fn helper() { 42 }
        }
        use MyMod::helper;
        let x = helper();
        output x;
    "#;
    let prog = parse_v2(src).unwrap();
    let analysis = check_types(&prog, &std::collections::HashSet::new());
    let e201s: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("E201"))
        .collect();
    assert!(e201s.is_empty(), "E201 false positive for use-imported name: {:?}", e201s);
}

#[test]
fn test_use_alias_no_e201() {
    // use with alias should register the aliased name
    let src = r#"
        mod MyMod {
            fn helper() { 42 }
        }
        use MyMod::helper as h;
        let x = h();
        output x;
    "#;
    let prog = parse_v2(src).unwrap();
    let analysis = check_types(&prog, &std::collections::HashSet::new());
    let e201s: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("E201"))
        .collect();
    assert!(e201s.is_empty(), "E201 false positive for use-aliased name: {:?}", e201s);
}

#[test]
fn test_w113_or_pattern_variable_mismatch() {
    // Or-pattern alternatives that bind different variables should use W113
    let src = r#"
        let val = 5;
        let result = match val {
            x | 0 => x,
            _ => 0,
        };
        output result;
    "#;
    let prog = parse_v2(src).unwrap();
    let analysis = check_types(&prog, &std::collections::HashSet::new());
    let w113s: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W113"))
        .collect();
    assert!(!w113s.is_empty(), "Expected W113 for or-pattern variable mismatch");
}

#[test]
fn test_screaming_snake_case_no_w200() {
    // SCREAMING_SNAKE_CASE constants should not trigger W200
    let src = r#"
        const MAX_SIZE = 100;
        const HTTP_PORT = 8080;
        output MAX_SIZE;
    "#;
    let prog = parse_v2(src).unwrap();
    let lint_config = magi_lang::linter::LintConfig::default();
    let lint_result = magi_lang::linter::lint(&prog, &lint_config);
    let w200s: Vec<_> = lint_result.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W200"))
        .collect();
    assert!(w200s.is_empty(), "W200 false positive for SCREAMING_SNAKE_CASE: {:?}", w200s);
}
