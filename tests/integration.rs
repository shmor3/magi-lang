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
                        Ok(DataType::Int64(x / y))
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
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(x % y)),
                _ => Ok(DataType::Null),
            },

            // Comparison
            OperationType::Equal => Ok(DataType::Bool(a == b)),
            OperationType::NotEqual => Ok(DataType::Bool(a != b)),
            OperationType::Greater => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Bool(x > y)),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Bool(x > y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::Less => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Bool(x < y)),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Bool(x < y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::GreaterEq => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Bool(x >= y)),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Bool(x >= y)),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::LessEq => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Bool(x <= y)),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Bool(x <= y)),
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
fn test_compile_showcase() {
    let src = include_str!("../examples/showcase/main.magi");
    let program = parse(src);
    let mut compiler_inst = compiler::Compiler::new();
    let module = compiler_inst.compile(&program).unwrap();

    // Verify basic structure.
    assert!(module.functions.iter().any(|f| f.name == "__main"));
    assert!(module.functions.iter().any(|f| f.name == "distance"));
    assert!(module.functions.iter().any(|f| f.name == "area"));
    assert!(module.functions.iter().any(|f| f.name == "sum"));
    assert!(module.functions.iter().any(|f| f.name == "safe_divide"));
    assert!(module.functions.iter().any(|f| f.name == "classify"));
}

#[test]
fn test_compile_to_wasm_showcase() {
    let src = include_str!("../examples/showcase/main.magi");
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
        InterpError::ThrownError { .. } => {}
        _ => panic!("expected ThrownError (assertion), got: {:?}", err),
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
        InterpError::ThrownError { .. } => {}
        _ => panic!("expected ThrownError (assert_eq), got: {:?}", err),
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
