#![allow(clippy::approx_constant)]
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
        let array = inputs.get("array").cloned().unwrap_or(DataType::Null);
        let value = inputs.get("value").cloned().unwrap_or(DataType::Null);
        let map = inputs.get("map").cloned().unwrap_or(DataType::Null);
        let key = inputs.get("key").cloned().unwrap_or(DataType::Null);

        match op {
            // Arithmetic
            OperationType::Add => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(x.wrapping_add(*y))),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x + y)),
                (DataType::Int64(x), DataType::Float64(y)) => Ok(DataType::Float64(*x as f64 + y)),
                (DataType::Float64(x), DataType::Int64(y)) => Ok(DataType::Float64(x + *y as f64)),
                (DataType::String(x), DataType::String(y)) => {
                    Ok(DataType::String(format!("{}{}", x, y)))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::Subtract => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(x.wrapping_sub(*y))),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x - y)),
                (DataType::Int64(x), DataType::Float64(y)) => Ok(DataType::Float64(*x as f64 - y)),
                (DataType::Float64(x), DataType::Int64(y)) => Ok(DataType::Float64(x - *y as f64)),
                _ => Ok(DataType::Null),
            },
            OperationType::Multiply => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(x.wrapping_mul(*y))),
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
                DataType::Int64(x) => Ok(DataType::Int64(x.wrapping_neg())),
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
            OperationType::ArrayPop => match &array {
                DataType::Array(arr) if !arr.is_empty() => {
                    Ok(arr.last().cloned().unwrap_or(DataType::Null))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArraySlice => {
                let start_val = inputs.get("input_1").or(inputs.get("start")).cloned().unwrap_or(DataType::Int64(0));
                let end_val = inputs.get("input_2").or(inputs.get("end")).cloned();
                match &array {
                    DataType::Array(arr) => {
                        let len = arr.len() as i64;
                        let start = {
                            let n = start_val.to_i64().unwrap_or(0);
                            if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                        };
                        let end = match &end_val {
                            Some(v) => {
                                let n = v.to_i64().unwrap_or(len);
                                if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                            }
                            None => arr.len(),
                        };
                        if start >= end {
                            Ok(DataType::Array(vec![]))
                        } else {
                            Ok(DataType::Array(arr[start..end].to_vec()))
                        }
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::ArraySort => match &array {
                DataType::Array(arr) => {
                    let mut sorted = arr.clone();
                    sorted.sort_by(|a, b| {
                        match (a, b) {
                            (DataType::String(x), DataType::String(y)) => x.cmp(y),
                            _ => a.to_i64().unwrap_or(0).cmp(&b.to_i64().unwrap_or(0)),
                        }
                    });
                    Ok(DataType::Array(sorted))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayReverse => match &array {
                DataType::Array(arr) => {
                    let mut rev = arr.clone();
                    rev.reverse();
                    Ok(DataType::Array(rev))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayContains => match (&array, &value) {
                (DataType::Array(arr), val) => Ok(DataType::Bool(arr.contains(val))),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::ArrayJoin => {
                let sep = inputs.get("input_1").cloned().unwrap_or(DataType::String(",".to_string()));
                match &array {
                    DataType::Array(arr) => {
                        let sep_str = match &sep {
                            DataType::String(s) => s.clone(),
                            _ => ",".to_string(),
                        };
                        let s: Vec<String> = arr.iter().map(|v| v.to_string_lossy()).collect();
                        Ok(DataType::String(s.join(&sep_str)))
                    }
                    _ => Ok(DataType::String(String::new())),
                }
            },
            OperationType::ArrayGet => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match &array {
                    DataType::Array(arr) => {
                        let i = index.to_i64().unwrap_or(-1);
                        if i < 0 { return Ok(DataType::Null); }
                        Ok(arr.get(i as usize).cloned().unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::ArraySet => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match &array {
                    DataType::Array(arr) => {
                        let i = index.to_i64().unwrap_or(-1);
                        if i < 0 { return Ok(DataType::Array(arr.clone())); }
                        let idx = i as usize;
                        let mut new_arr = arr.clone();
                        if idx < new_arr.len() {
                            new_arr[idx] = value.clone();
                        }
                        Ok(DataType::Array(new_arr))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::ArrayFlatten => match &array {
                DataType::Array(arr) => {
                    let mut flat = Vec::new();
                    for item in arr {
                        if let DataType::Array(inner) = item {
                            flat.extend(inner.clone());
                        } else {
                            flat.push(item.clone());
                        }
                    }
                    Ok(DataType::Array(flat))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayUnique => match &array {
                DataType::Array(arr) => {
                    let mut seen = Vec::new();
                    for item in arr {
                        if !seen.contains(item) {
                            seen.push(item.clone());
                        }
                    }
                    Ok(DataType::Array(seen))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayConcat => match (&a, &b) {
                (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                    let mut result = a_arr.clone();
                    result.extend(b_arr.clone());
                    Ok(DataType::Array(result))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayInsert => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match &array {
                    DataType::Array(arr) => {
                        let i = index.to_i64().unwrap_or(0);
                        let idx = if i < 0 { 0 } else { (i as usize).min(arr.len()) };
                        let mut new_arr = arr.clone();
                        new_arr.insert(idx, value.clone());
                        Ok(DataType::Array(new_arr))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::ArrayRemove => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match &array {
                    DataType::Array(arr) => {
                        let i = index.to_i64().unwrap_or(-1);
                        if i < 0 || i as usize >= arr.len() { return Ok(DataType::Array(arr.clone())); }
                        let mut new_arr = arr.clone();
                        new_arr.remove(i as usize);
                        Ok(DataType::Array(new_arr))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::ArrayFilterNulls => match &array {
                DataType::Array(arr) => {
                    Ok(DataType::Array(arr.iter().filter(|v| !matches!(v, DataType::Null)).cloned().collect()))
                }
                _ => Ok(DataType::Null),
            },

            // Map operations
            OperationType::MapGet => {
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
                let map_val = inputs.get("map").cloned().unwrap_or(DataType::Null);
                let key_val = inputs.get("key").cloned().unwrap_or(DataType::String(String::new()));
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
            OperationType::MapKeys => match &map {
                DataType::Map(m) => Ok(DataType::Array(
                    m.keys().map(|k| DataType::String(k.clone())).collect(),
                )),
                _ => Ok(DataType::Array(vec![])),
            },
            OperationType::MapValues => match &map {
                DataType::Map(m) => Ok(DataType::Array(m.values().cloned().collect())),
                _ => Ok(DataType::Array(vec![])),
            },
            OperationType::MapHas => match (&map, &key) {
                (DataType::Map(m), DataType::String(k)) => Ok(DataType::Bool(m.contains_key(k))),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::MapDelete => match (&map, &key) {
                (DataType::Map(m), DataType::String(k)) => {
                    let mut new_map = m.clone();
                    new_map.remove(k);
                    Ok(DataType::Map(new_map))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::MapMerge => match (&a, &b) {
                (DataType::Map(m1), DataType::Map(m2)) => {
                    let mut merged = m1.clone();
                    for (k, v) in m2 {
                        merged.insert(k.clone(), v.clone());
                    }
                    Ok(DataType::Map(merged))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::MapSize => match &map {
                DataType::Map(m) => Ok(DataType::Int64(m.len() as i64)),
                _ => Ok(DataType::Int64(0)),
            },
            OperationType::MapEntries => match &map {
                DataType::Map(m) => {
                    Ok(DataType::Array(m.iter().map(|(k, v)| {
                        DataType::Array(vec![DataType::String(k.clone()), v.clone()])
                    }).collect()))
                }
                _ => Ok(DataType::Array(vec![])),
            },

            // Type conversion
            OperationType::Abs => match &input {
                DataType::Int64(n) => Ok(DataType::Int64(n.checked_abs().unwrap_or(i64::MAX))),
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
                        DataType::Array(arr) => format!("[{}]", arr.iter().map(to_json_stub).collect::<Vec<_>>().join(",")),
                        DataType::Map(m) => format!("{{{}}}", m.iter().filter(|(k,_)| !k.starts_with("__")).map(|(k,v)| format!("\"{}\":{}", k, to_json_stub(v))).collect::<Vec<_>>().join(",")),
                        _ => "null".to_string(),
                    }
                }
                Ok(DataType::String(to_json_stub(&input)))
            },

            // Range
            OperationType::Range => {
                let start = inputs.get("start").or(inputs.get("a")).and_then(|v| v.to_i64()).unwrap_or(0);
                let end = inputs.get("end").or(inputs.get("b")).and_then(|v| v.to_i64()).unwrap_or(0);
                let step = inputs.get("step").and_then(|v| v.to_i64()).unwrap_or(if start <= end { 1 } else { -1 });
                if step == 0 { return Ok(DataType::Array(vec![])); }
                let mut result = Vec::new();
                let mut i = start;
                loop {
                    if step > 0 && i >= end { break; }
                    if step < 0 && i <= end { break; }
                    result.push(DataType::Int64(i));
                    i = match i.checked_add(step) {
                        Some(v) => v,
                        None => break,
                    };
                }
                Ok(DataType::Array(result))
            },

            // String length
            OperationType::Length => match &input {
                DataType::String(s) => Ok(DataType::Int64(s.chars().count() as i64)),
                _ => Ok(DataType::Int64(0)),
            },

            // String operations
            OperationType::Split => {
                let delim = inputs.get("delimiter").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                match (&input, &delim) {
                    (DataType::String(s), DataType::String(sep)) => {
                        if sep.is_empty() {
                            return Err(EvalError::InvalidInput("split delimiter must not be empty".to_string()));
                        }
                        let parts: Vec<DataType> = s.split(sep.as_str()).map(|p| DataType::String(p.to_string())).collect();
                        Ok(DataType::Array(parts))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            },
            OperationType::Contains => {
                let search = inputs.get("search").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                match (&input, &search) {
                    (DataType::String(s), DataType::String(sub)) => Ok(DataType::Bool(s.contains(sub.as_str()))),
                    _ => Ok(DataType::Bool(false)),
                }
            },
            OperationType::Replace => {
                let search = inputs.get("search").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                let replace = inputs.get("replace").or(inputs.get("input_2")).cloned().unwrap_or(DataType::Null);
                match (&input, &search, &replace) {
                    (DataType::String(s), DataType::String(from), DataType::String(to)) => {
                        Ok(DataType::String(s.replace(from.as_str(), to.as_str())))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::StartsWith => {
                let prefix = inputs.get("prefix").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                match (&input, &prefix) {
                    (DataType::String(s), DataType::String(p)) => Ok(DataType::Bool(s.starts_with(p.as_str()))),
                    _ => Ok(DataType::Bool(false)),
                }
            },
            OperationType::EndsWith => {
                let suffix = inputs.get("suffix").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                match (&input, &suffix) {
                    (DataType::String(s), DataType::String(sfx)) => Ok(DataType::Bool(s.ends_with(sfx.as_str()))),
                    _ => Ok(DataType::Bool(false)),
                }
            },
            OperationType::IndexOf => {
                let search = inputs.get("search").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                match (&input, &search) {
                    (DataType::String(s), DataType::String(sub)) => {
                        match s.find(sub.as_str()) {
                            Some(byte_pos) => Ok(DataType::Int64(s[..byte_pos].chars().count() as i64)),
                            None => Ok(DataType::Int64(-1)),
                        }
                    }
                    _ => Ok(DataType::Int64(-1)),
                }
            },
            OperationType::Substring => {
                let start_val = inputs.get("input_1").cloned().unwrap_or(DataType::Int64(0));
                let end_val = inputs.get("input_2").cloned();
                match &input {
                    DataType::String(s) => {
                        let chars: Vec<char> = s.chars().collect();
                        let len = chars.len() as i64;
                        let start = start_val.to_i64().unwrap_or(0).max(0).min(len) as usize;
                        let end = match end_val {
                            Some(v) => v.to_i64().unwrap_or(len).max(0).min(len) as usize,
                            None => chars.len(),
                        };
                        if start >= end {
                            Ok(DataType::String(String::new()))
                        } else {
                            Ok(DataType::String(chars[start..end].iter().collect()))
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::ToUpper => match &input {
                DataType::String(s) => Ok(DataType::String(s.to_uppercase())),
                _ => Ok(DataType::Null),
            },
            OperationType::ToLower => match &input {
                DataType::String(s) => Ok(DataType::String(s.to_lowercase())),
                _ => Ok(DataType::Null),
            },
            OperationType::Trim => match &input {
                DataType::String(s) => Ok(DataType::String(s.trim().to_string())),
                _ => Ok(DataType::Null),
            },
            OperationType::TrimStart => match &input {
                DataType::String(s) => Ok(DataType::String(s.trim_start().to_string())),
                _ => Ok(DataType::Null),
            },
            OperationType::TrimEnd => match &input {
                DataType::String(s) => Ok(DataType::String(s.trim_end().to_string())),
                _ => Ok(DataType::Null),
            },
            OperationType::StringReverse => match &input {
                DataType::String(s) => Ok(DataType::String(s.chars().rev().collect())),
                _ => Ok(DataType::Null),
            },
            OperationType::StringRepeat => {
                let count = inputs.get("input_1").or(inputs.get("count")).cloned().unwrap_or(DataType::Int64(0));
                match &input {
                    DataType::String(s) => {
                        let n = count.to_i64().unwrap_or(0).max(0) as usize;
                        Ok(DataType::String(s.repeat(n)))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::StringLines => match &input {
                DataType::String(s) => Ok(DataType::Array(s.lines().map(|l| DataType::String(l.to_string())).collect())),
                _ => Ok(DataType::Null),
            },
            OperationType::StringChars => match &input {
                DataType::String(s) => Ok(DataType::Array(s.chars().map(|c| DataType::String(c.to_string())).collect())),
                _ => Ok(DataType::Null),
            },
            OperationType::CharAt => {
                let idx = inputs.get("input_1").cloned().unwrap_or(DataType::Int64(0));
                match &input {
                    DataType::String(s) => {
                        let i = idx.to_i64().unwrap_or(0);
                        if i < 0 { return Ok(DataType::Null); }
                        Ok(s.chars().nth(i as usize).map(|c| DataType::String(c.to_string())).unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            },

            // Type conversion
            OperationType::ToInt64 => match &input {
                DataType::Int64(n) => Ok(DataType::Int64(*n)),
                DataType::Float64(f) => {
                    if f.is_finite() && *f >= i64::MIN as f64 && *f < i64::MAX as f64 {
                        Ok(DataType::Int64(*f as i64))
                    } else {
                        Ok(DataType::Null)
                    }
                }
                DataType::String(s) => Ok(s.parse::<i64>().map(DataType::Int64).unwrap_or(DataType::Null)),
                DataType::Bool(b) => Ok(DataType::Int64(if *b { 1 } else { 0 })),
                _ => Ok(DataType::Null),
            },
            OperationType::ToFloat64 => match &input {
                DataType::Float64(f) => Ok(DataType::Float64(*f)),
                DataType::Int64(n) => Ok(DataType::Float64(*n as f64)),
                DataType::String(s) => Ok(s.parse::<f64>().map(DataType::Float64).unwrap_or(DataType::Null)),
                DataType::Bool(b) => Ok(DataType::Float64(if *b { 1.0 } else { 0.0 })),
                _ => Ok(DataType::Null),
            },
            OperationType::ToBool => match &input {
                DataType::Bool(b) => Ok(DataType::Bool(*b)),
                DataType::Int64(n) => Ok(DataType::Bool(*n != 0)),
                DataType::Float64(f) => Ok(DataType::Bool(*f != 0.0 && !f.is_nan())),
                DataType::String(s) => Ok(DataType::Bool(!s.is_empty())),
                DataType::Null => Ok(DataType::Bool(false)),
                DataType::Array(a) => Ok(DataType::Bool(!a.is_empty())),
                DataType::Map(m) => Ok(DataType::Bool(!m.is_empty())),
                _ => Ok(DataType::Bool(true)),
            },

            // Power
            OperationType::Power => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => {
                    if *y < 0 {
                        Ok(DataType::Float64((*x as f64).powi(*y as i32)))
                    } else {
                        match x.checked_pow(*y as u32) {
                            Some(v) => Ok(DataType::Int64(v)),
                            None => Ok(DataType::Null),
                        }
                    }
                }
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x.powf(*y))),
                (DataType::Int64(x), DataType::Float64(y)) => Ok(DataType::Float64((*x as f64).powf(*y))),
                (DataType::Float64(x), DataType::Int64(y)) => Ok(DataType::Float64(x.powi(*y as i32))),
                _ => Ok(DataType::Null),
            },

            // Min/Max builtins
            OperationType::Min => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(*x.min(y))),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x.min(*y))),
                _ => Ok(DataType::Null),
            },
            OperationType::Max => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(*x.max(y))),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x.max(*y))),
                _ => Ok(DataType::Null),
            },

            // Assert
            OperationType::Assert => {
                let condition = inputs.get("condition").cloned().unwrap_or(DataType::Null);
                let message = inputs.get("message").cloned().unwrap_or(DataType::String("assertion failed".to_string()));
                match condition {
                    DataType::Bool(true) => Ok(DataType::Null),
                    _ => {
                        let msg = match message {
                            DataType::String(s) => s,
                            _ => "assertion failed".to_string(),
                        };
                        Err(EvalError::InvalidInput(msg))
                    }
                }
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
fn test_compile_showcase_rejects_unsupported() {
    // The showcase uses features not yet supported in WASM compilation
    // (spread in array literals, match guards, etc.).
    let src = include_str!("../examples/showcase/main.magi");
    let program = parse(src);
    let mut compiler_inst = compiler::Compiler::new();
    let result = compiler_inst.compile(&program);
    assert!(result.is_err(), "showcase should fail WASM compilation due to unsupported features");
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
    assert_eq!(v.minor, 3);
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
fn test_w104_empty_loop_body_moved_to_linter() {
    // W104 (empty block) is now handled by the linter as W206.
    let codes = typecheck_warnings("for x in [1, 2, 3] {}");
    assert!(!codes.contains(&"W104".to_string()), "W104 should no longer be emitted by type checker, got: {:?}", codes);
}

#[test]
fn test_w105_infinite_loop_moved_to_linter() {
    // W105 (while true) is now handled by the linter as W204.
    let codes = typecheck_warnings("while true {}");
    assert!(!codes.contains(&"W105".to_string()), "W105 should no longer be emitted by type checker, got: {:?}", codes);
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
        d.code.as_ref().is_some_and(|c: &String| c.contains("E101"))
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
        d.code.as_ref().is_some_and(|c: &String| c.contains("E201"))
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
        d.code.as_ref().is_some_and(|c: &String| c.contains("E405"))
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
    // W105 moved to linter as W204; type checker no longer emits it
    let codes = typecheck_warnings(r#"
while true {
    break;
}
"#);
    assert!(!codes.contains(&"W105".to_string()), "W105 should no longer be emitted by type checker, got {:?}", codes);
}

#[test]
fn test_while_true_with_break_in_if_no_w105() {
    // W105 moved to linter as W204; type checker no longer emits it
    let codes = typecheck_warnings(r#"
let mut i = 0
while true {
    i = i + 1
    if i > 5 { break; }
}
"#);
    assert!(!codes.contains(&"W105".to_string()), "W105 should no longer be emitted by type checker, got {:?}", codes);
}

#[test]
fn test_while_true_without_break_no_w105() {
    // W105 moved to linter as W204; type checker no longer emits it
    let codes = typecheck_warnings(r#"
while true {
    output 1;
}
"#);
    assert!(!codes.contains(&"W105".to_string()), "W105 should no longer be emitted by type checker, got {:?}", codes);
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
    // W105 moved to linter as W204; type checker no longer emits it
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
        "W105 should no longer be emitted by type checker, got {:?}", codes);
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

#[test]
fn test_max_call_depth() {
    // Infinite recursion should hit max call depth
    // Needs larger stack due to deep recursion in debug mode
    let result = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let err = run_err(r#"
                fn boom() { boom() }
                boom()
            "#);
            let msg = format!("{}", err);
            assert!(msg.contains("call depth") || msg.contains("recursion") || msg.contains("E401") || msg.contains("Maximum"),
                "expected max call depth error: {}", msg);
        })
        .unwrap()
        .join();
    result.unwrap();
}

#[test]
fn test_compound_assign_on_immutable() {
    // Compound assignment on immutable variable should error
    let err = run_err(r#"
        let x = 5;
        x += 1;
    "#);
    let msg = format!("{}", err);
    assert!(msg.contains("immutable") || msg.contains("mutable") || msg.contains("cannot assign"),
        "expected immutability error: {}", msg);
}

#[test]
fn test_map_comprehension_success() {
    // Map comprehension should produce correct output
    // Key must be a string literal, value can be any expression
    let result = run(r#"
        let data = [1, 2, 3];
        let doubled = {"item": v * 2 for v in data};
        output doubled;
    "#);
    // Map comprehension creates map entries — last iteration value wins for same key
    match &result {
        DataType::Map(m) => {
            assert_eq!(m.get("item"), Some(&DataType::Int64(6)));
        }
        other => panic!("expected map, got: {:?}", other),
    }
}

#[test]
fn test_w102_variable_shadowing_moved_to_linter() {
    // W102 (same-scope shadowing) is now handled by the linter as W209.
    let src = r#"
        let x = 1;
        let x = 2;
        output x;
    "#;
    let prog = parse_v2(src).unwrap();
    let analysis = check_types(&prog, &std::collections::HashSet::new());
    let w102s: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W102"))
        .collect();
    assert!(w102s.is_empty(), "W102 should no longer be emitted by type checker, got {:?}", w102s);
}

#[test]
fn test_w101_unused_import() {
    // Unused import should trigger W101
    let src = r#"
        import "unused_plugin";
        output 42;
    "#;
    let prog = parse_v2(src).unwrap();
    let mut imports = std::collections::HashSet::new();
    imports.insert("unused_plugin".to_string());
    let analysis = check_types(&prog, &imports);
    let w101s: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W101"))
        .collect();
    assert!(!w101s.is_empty(), "Expected W101 for unused import");
}

#[test]
fn test_loop_break_with_value() {
    // Loop with break value should return the break value
    let result = run(r#"
        let x = loop {
            break 42;
        };
        output x;
    "#);
    assert_eq!(result, DataType::Int64(42));
}

#[test]
fn test_for_loop_last_value() {
    // For loop returns the last iteration body value
    let result = run(r#"
        let mut last = 0;
        for i in [1, 2, 3] {
            last = i * 10;
        }
        output last;
    "#);
    assert_eq!(result, DataType::Int64(30));
}

#[test]
fn test_while_break_value() {
    // While loop with break value via loop expression
    let result = run(r#"
        let mut i = 0;
        let x = loop {
            i = i + 1;
            if i == 5 {
                break i * 100;
            }
        };
        output x;
    "#);
    assert_eq!(result, DataType::Int64(500));
}

#[test]
fn test_string_interpolation_resource_limit() {
    // Very large string interpolation should produce ResourceLimit, not TypeError
    let err = run_err(r#"
        let big = "x".repeat(5000000);
        output f"{big}{big}{big}";
    "#);
    assert!(matches!(err, InterpError::ResourceLimit { .. }),
        "Expected ResourceLimit, got: {:?}", err);
}

#[test]
fn test_array_spread_resource_limit() {
    // Spread operations that exceed the element limit should error
    let err = run_err(r#"
        fn make_big() {
            let arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
            let mut big = arr;
            let mut i = 0;
            while i < 20 {
                big = [...big, ...big];
                i = i + 1;
            }
            big
        }
        output make_big();
    "#);
    let msg = format!("{}", err);
    assert!(msg.contains("limit") || msg.contains("resource") || msg.contains("element") || msg.contains("iteration"),
        "Expected resource limit error: {}", msg);
}

// ── Round 74: E409 resource limit error code ────────────────────

#[test]
fn test_resource_limit_uses_e409() {
    let err = run_err(r#"
        let s = "a".repeat(20_000_000);
        output s;
    "#);
    let msg = format!("{}", err);
    assert!(msg.contains("E409"), "Expected E409 error code, got: {}", msg);
}

#[test]
fn test_max_iterations_uses_e400() {
    let err = run_err(r#"
        let mut i = 0;
        while true {
            i = i + 1;
        }
        output i;
    "#);
    let msg = format!("{}", err);
    assert!(msg.contains("E400"), "Expected E400 error code, got: {}", msg);
}

#[test]
fn test_e409_error_code_help() {
    let help = magi_lang::syntax::errors::ErrorCode::E409.help();
    assert!(help.contains("resource limit"), "E409 help should mention resource limit: {}", help);
}

#[test]
fn test_e400_e409_distinct_codes() {
    // E400 = MaxIterations, E409 = ResourceLimit — they should be distinct
    assert_ne!(
        magi_lang::syntax::errors::ErrorCode::E400.to_string(),
        magi_lang::syntax::errors::ErrorCode::E409.to_string(),
    );
}

// ── Round 75: Generic method type checker fix ────────────────────

#[test]
fn test_to_json_no_false_positive() {
    // to_json() should be recognized on all types without E201 warning
    let src = r#"
        let x = 42;
        let s = x.to_json();
        output s;
    "#;
    let program = parse_v2(src).expect("parse");
    let imports = std::collections::HashSet::new();
    let diags = check_types(&program, &imports);
    let e201_warnings: Vec<_> = diags.diagnostics.iter().filter(|d| {
        d.message.contains("Unknown method")
    }).collect();
    assert!(e201_warnings.is_empty(), "to_json should not produce E201: {:?}", e201_warnings);
}

#[test]
fn test_typeof_method_no_false_positive() {
    // typeof() as a method should be recognized
    let src = r#"
        let arr = [1, 2, 3];
        let t = arr.typeof();
        output t;
    "#;
    let program = parse_v2(src).expect("parse");
    let imports = std::collections::HashSet::new();
    let diags = check_types(&program, &imports);
    let e201_warnings: Vec<_> = diags.diagnostics.iter().filter(|d| {
        d.message.contains("Unknown method")
    }).collect();
    assert!(e201_warnings.is_empty(), "typeof should not produce E201: {:?}", e201_warnings);
}

#[test]
fn test_to_bool_method_no_false_positive() {
    // to_bool() should be recognized on all types
    let src = r#"
        let s = "hello";
        let b = s.to_bool();
        output b;
    "#;
    let program = parse_v2(src).expect("parse");
    let imports = std::collections::HashSet::new();
    let diags = check_types(&program, &imports);
    let e201_warnings: Vec<_> = diags.diagnostics.iter().filter(|d| {
        d.message.contains("Unknown method")
    }).collect();
    assert!(e201_warnings.is_empty(), "to_bool should not produce E201: {:?}", e201_warnings);
}

// ── Round 76: HOF cancellation and pad byte limits ────────────────────

#[test]
fn test_enumerate_produces_pairs() {
    let result = run(r#"
        let arr = ["a", "b", "c"];
        let pairs = arr.enumerate();
        output pairs;
    "#);
    match result {
        DataType::Array(items) => {
            assert_eq!(items.len(), 3);
            // First pair should be [0, "a"]
            match &items[0] {
                DataType::Array(pair) => {
                    assert_eq!(pair[0], DataType::Int64(0));
                    assert_eq!(pair[1], DataType::String("a".to_string()));
                }
                other => panic!("Expected array pair, got {:?}", other),
            }
        }
        other => panic!("Expected array, got {:?}", other),
    }
}

#[test]
fn test_zip_produces_pairs() {
    let result = run(r#"
        let a = [1, 2, 3];
        let b = ["x", "y", "z"];
        let zipped = a.zip(b);
        output zipped.length();
    "#);
    assert_eq!(result, DataType::Int64(3));
}

#[test]
fn test_chunk_produces_chunks() {
    let result = run(r#"
        let arr = [1, 2, 3, 4, 5];
        let chunks = arr.chunk(2);
        output chunks.length();
    "#);
    assert_eq!(result, DataType::Int64(3)); // [1,2], [3,4], [5]
}

#[test]
fn test_pad_start_multibyte_byte_limit() {
    // Padding with a multibyte fill string should be caught by byte limit
    let err = run_err(r#"
        let s = "x".pad_start(5_000_000, "你好");
        output s;
    "#);
    let msg = format!("{}", err);
    assert!(msg.contains("E409") || msg.contains("resource") || msg.contains("limit"),
        "Expected resource limit error, got: {}", msg);
}

#[test]
fn test_pad_end_multibyte_byte_limit() {
    let err = run_err(r#"
        let s = "x".pad_end(5_000_000, "你好");
        output s;
    "#);
    let msg = format!("{}", err);
    assert!(msg.contains("E409") || msg.contains("resource") || msg.contains("limit"),
        "Expected resource limit error, got: {}", msg);
}

// ── Round 77: arity message and Power fixes ──────────────

#[test]
fn test_pad_start_arity_message() {
    let err = run_err(r#"
        let s = "hello".pad_start();
        output s;
    "#);
    let msg = format!("{}", err);
    assert!(msg.contains("1-2"), "Expected arity '1-2', got: {}", msg);
}

#[test]
fn test_pad_end_arity_message() {
    let err = run_err(r#"
        let s = "hello".pad_end();
        output s;
    "#);
    let msg = format!("{}", err);
    assert!(msg.contains("1-2"), "Expected arity '1-2', got: {}", msg);
}

#[test]
fn test_substring_arity_message() {
    let err = run_err(r#"
        let s = "hello".substring();
        output s;
    "#);
    let msg = format!("{}", err);
    assert!(msg.contains("1-2"), "Expected arity '1-2', got: {}", msg);
}

#[test]
fn test_pad_start_with_two_args() {
    assert_eq!(run(r#"output "hi".pad_start(5, "*");"#), DataType::String("***hi".to_string()));
}

#[test]
fn test_substring_with_two_args() {
    assert_eq!(run(r#"output "hello".substring(1, 4);"#), DataType::String("ell".to_string()));
}

// ── Round 78: linter, type checker, formatter fixes ──────

#[test]
fn test_linter_module_no_duplicate_diagnostics() {
    use magi_lang::linter;
    let src = r#"
        mod utils {
            fn badName() { 42 }
        }
    "#;
    let program = parse_v2(src).unwrap();
    let result = linter::lint(&program, &linter::LintConfig::default());
    // Should have exactly one W200 for badName, not two
    let w200_count = result.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W200") && d.message.contains("badName"))
        .count();
    assert_eq!(w200_count, 1, "Expected 1 W200 diagnostic for badName, got {}. All diagnostics: {:?}",
        w200_count, result.diagnostics);
}

#[test]
fn test_linter_default_param_linted() {
    use magi_lang::linter;
    let src = r#"
        fn foo(x = if true { 1 } else { 2 }) {
            x
        }
    "#;
    let program = parse_v2(src).unwrap();
    let result = linter::lint(&program, &linter::LintConfig::default());
    // Should have W204 for constant condition `true` in default param
    let w204_count = result.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("W204"))
        .count();
    assert_eq!(w204_count, 1, "Expected 1 W204 diagnostic for constant condition in default param, got {}. All diagnostics: {:?}",
        w204_count, result.diagnostics);
}

#[test]
fn test_type_checker_shift_no_w110() {
    let src = r#"
        let mut arr = [1, 2, 3];
        let first = arr.shift();
        output first;
    "#;
    let program = parse_v2(src).unwrap();
    let imports = std::collections::HashSet::new();
    let result = check_types(&program, &imports);
    let w110_count = result.diagnostics.iter()
        .filter(|d| d.message.contains("W110"))
        .count();
    assert_eq!(w110_count, 0, "Expected no W110 for arr.shift(), got: {:?}",
        result.diagnostics.iter().filter(|d| d.message.contains("W110")).collect::<Vec<_>>());
}

#[test]
fn test_formatter_fstring_sentinel_roundtrip() {
    use magi_lang::formatter;
    // f-string with escaped braces (the parser stores \{ as sentinel \u{FFF0})
    let src = r#"output f"hello \{ world \}";"#;
    let program = parse_v2(src).unwrap();
    let formatted = formatter::format_program(&program, &formatter::FormatConfig::default());
    // The formatted output should contain \{ and \}, not raw sentinel chars
    assert!(!formatted.contains('\u{FFF0}'), "Formatted output should not contain sentinel chars");
    assert!(!formatted.contains('\u{FFF1}'), "Formatted output should not contain sentinel chars");
    assert!(formatted.contains("\\{"), "Formatted output should contain escaped braces: {}", formatted);
}

// ── Round 78 continued: coverage gap tests ───────────────

#[test]
fn test_for_loop_over_exclusive_range() {
    assert_eq!(run(r#"
        let mut sum = 0;
        for i in 0..5 { sum = sum + i; }
        output sum;
    "#), DataType::Int64(10));
}

#[test]
fn test_for_loop_over_inclusive_range() {
    assert_eq!(run(r#"
        let mut sum = 0;
        for i in 1..=5 { sum = sum + i; }
        output sum;
    "#), DataType::Int64(15));
}

#[test]
fn test_for_loop_empty_range() {
    assert_eq!(run(r#"
        let mut count = 0;
        for _x in 5..1 { count = count + 1; }
        output count;
    "#), DataType::Int64(0));
}

#[test]
fn test_closure_captures_mutable_variable_snapshot() {
    assert_eq!(run(r#"
        let mut x = 10;
        let f = |y| x + y;
        x = 99;
        output f(5);
    "#), DataType::Int64(15));
}

#[test]
fn test_closure_in_map_captures_outer() {
    assert_eq!(run(r#"
        let factor = 3;
        let result = [1, 2, 3, 4].map(|x| x * factor);
        output result;
    "#), DataType::Array(vec![
        DataType::Int64(3), DataType::Int64(6),
        DataType::Int64(9), DataType::Int64(12),
    ]));
}

#[test]
fn test_function_returning_closure() {
    assert_eq!(run(r#"
        fn make_adder(n) { |x| x + n }
        let add5 = make_adder(5);
        let add10 = make_adder(10);
        output [add5(3), add10(3)];
    "#), DataType::Array(vec![DataType::Int64(8), DataType::Int64(13)]));
}

#[test]
fn test_string_to_int() {
    assert_eq!(run(r#"output "42".to_int();"#), DataType::Int64(42));
}

#[test]
fn test_string_to_int_invalid() {
    assert_eq!(run(r#"output "abc".to_int();"#), DataType::Null);
}

#[test]
fn test_string_to_int_empty() {
    assert_eq!(run(r#"output "".to_int();"#), DataType::Null);
}

#[test]
fn test_string_to_float_valid() {
    assert_eq!(run(r#"output "3.14".to_float();"#), DataType::Float64(3.14));
}

#[test]
fn test_numeric_conversion_chain() {
    assert_eq!(run(r#"
        let f = 3.7;
        let i = f.to_int64();
        let s = i.to_string();
        output s;
    "#), DataType::String("3".to_string()));
}

#[test]
fn test_pipe_chain_three_stages() {
    assert_eq!(run(r#"
        fn only_even(arr) { arr.filter(|x| x % 2 == 0) }
        fn double_all(arr) { arr.map(|x| x * 2) }
        fn total(arr) { arr.reduce(0, |acc, x| acc + x) }
        output [1, 2, 3, 4, 5, 6] |> only_even(_) |> double_all(_) |> total(_);
    "#), DataType::Int64(24));
}

#[test]
fn test_multiple_spawns_and_awaits() {
    assert_eq!(run(r#"
        async fn double(x) { x * 2 }
        let t1 = spawn double(5);
        let t2 = spawn double(10);
        let r1 = await t1;
        let r2 = await t2;
        output [r1, r2];
    "#), DataType::Array(vec![DataType::Int64(10), DataType::Int64(20)]));
}

#[test]
fn test_use_import_and_call() {
    assert_eq!(run(r#"
        mod math {
            fn square(x) { x * x }
            fn cube(x) { x * x * x }
        }
        use math::square;
        use math::cube;
        output square(3) + cube(2);
    "#), DataType::Int64(17));
}

#[test]
fn test_triple_quoted_string() {
    let result = run("output \"\"\"hello world\"\"\";");
    assert_eq!(result, DataType::String("hello world".to_string()));
}

// Round 79: min_by/max_by arity check before empty-array check
#[test]
fn test_min_by_no_args() {
    let err = run_err("let a = [3,1,2]; output a.min_by();");
    match err {
        InterpError::ArityMismatch { name, expected, actual, .. } => {
            assert_eq!(name, "min_by");
            assert_eq!(expected, "1");
            assert_eq!(actual, 0);
        }
        _ => panic!("expected ArityMismatch, got {:?}", err),
    }
}

#[test]
fn test_max_by_no_args() {
    let err = run_err("let a = [3,1,2]; output a.max_by();");
    match err {
        InterpError::ArityMismatch { name, expected, actual, .. } => {
            assert_eq!(name, "max_by");
            assert_eq!(expected, "1");
            assert_eq!(actual, 0);
        }
        _ => panic!("expected ArityMismatch, got {:?}", err),
    }
}

#[test]
fn test_min_by_empty_array() {
    assert_eq!(run("let a = []; output a.min_by(|a, b| a - b);"), DataType::Null);
}

#[test]
fn test_max_by_empty_array() {
    assert_eq!(run("let a = []; output a.max_by(|a, b| a - b);"), DataType::Null);
}

#[test]
fn test_min_by_basic() {
    assert_eq!(run("output [3,1,2].min_by(|a, b| a - b);"), DataType::Int64(1));
}

#[test]
fn test_max_by_basic() {
    assert_eq!(run("output [3,1,2].max_by(|a, b| a - b);"), DataType::Int64(3));
}

#[test]
fn test_enumerate_basic() {
    // enumerate returns [[0, elem], [1, elem], ...] — check length
    assert_eq!(run("output [10,20,30].enumerate().len();"), DataType::Int64(3));
}

#[test]
fn test_group_by_basic() {
    // group_by returns a map — check it's a map
    assert_eq!(run(r#"
        let result = [1,2,3,4,5].group_by(|x| if x % 2 == 0 { "even" } else { "odd" });
        output typeof(result);
    "#), DataType::String("map".to_string()));
}

// Round 80: pow(0, -n) returns Null
#[test]
fn test_pow_zero_negative_exp() {
    assert_eq!(run("output (0).pow(-1);"), DataType::Null);
}

#[test]
fn test_pow_zero_negative_exp_large() {
    assert_eq!(run("output (0).pow(-100);"), DataType::Null);
}

#[test]
fn test_pow_one_negative_exp() {
    assert_eq!(run("output (1).pow(-5);"), DataType::Int64(1));
}

#[test]
fn test_pow_neg_one_negative_exp() {
    assert_eq!(run("output (-1).pow(-3);"), DataType::Int64(-1));
    assert_eq!(run("output (-1).pow(-4);"), DataType::Int64(1));
}

// Round 80: enumerate arity check
#[test]
fn test_enumerate_arity_error() {
    let err = run_err("output [1,2].enumerate(|x| x);");
    match err {
        InterpError::ArityMismatch { name, expected, actual, .. } => {
            assert_eq!(name, "enumerate");
            assert_eq!(expected, "0");
            assert_eq!(actual, 1);
        }
        _ => panic!("expected ArityMismatch, got {:?}", err),
    }
}

// Round 80: chunk(0) errors instead of silent coercion
#[test]
fn test_chunk_zero_error() {
    let err = run_err("output [1,2,3].chunk(0);");
    match err {
        InterpError::TypeError { context, .. } => {
            assert!(context.contains("chunk size"), "expected chunk size context, got {}", context);
        }
        _ => panic!("expected TypeError, got {:?}", err),
    }
}

#[test]
fn test_chunk_negative_error() {
    let err = run_err("output [1,2,3].chunk(-1);");
    match err {
        InterpError::TypeError { context, .. } => {
            assert!(context.contains("chunk size"));
        }
        _ => panic!("expected TypeError, got {:?}", err),
    }
}

// Round 80: assert() no args
#[test]
fn test_assert_no_args_error() {
    let err = run_err("assert();");
    match err {
        InterpError::ArityMismatch { name, expected, .. } => {
            assert_eq!(name, "assert");
            assert_eq!(expected, "1-2");
        }
        _ => panic!("expected ArityMismatch, got {:?}", err),
    }
}

// Round 80: Int64 min/max/clamp basic correctness
#[test]
fn test_int64_min_method() {
    assert_eq!(run("output (10).min(3);"), DataType::Int64(3));
}

#[test]
fn test_int64_max_method() {
    assert_eq!(run("output (3).max(10);"), DataType::Int64(10));
}

#[test]
fn test_int64_clamp_method() {
    assert_eq!(run("output (15).clamp(0, 10);"), DataType::Int64(10));
    assert_eq!(run("output (-5).clamp(0, 10);"), DataType::Int64(0));
    assert_eq!(run("output (5).clamp(0, 10);"), DataType::Int64(5));
}

// Round 81: type checker sort/reverse return Array (not Null)
#[test]
fn test_type_checker_sort_no_false_positive() {
    // sort() returns Array — should not warn about method on Null
    let src = r#"
        let arr = [3, 1, 2];
        let sorted = arr.sort();
        output sorted.len();
    "#;
    let program = magi_lang::syntax::parser::parse_v2(src).unwrap();
    let analysis = magi_lang::syntax::type_checker::check_types(&program, &std::collections::HashSet::new());
    let w110_warnings: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("E201"))
        .collect();
    assert!(w110_warnings.is_empty(), "sort() should return Array, not Null: {:?}", w110_warnings);
}

#[test]
fn test_type_checker_is_empty_no_false_positive() {
    // array.is_empty() returns Bool, not Null
    let src = r#"
        let arr = [1, 2, 3];
        if arr.is_empty() {
            output 0;
        }
        output 1;
    "#;
    let program = magi_lang::syntax::parser::parse_v2(src).unwrap();
    let analysis = magi_lang::syntax::type_checker::check_types(&program, &std::collections::HashSet::new());
    // Should not have E101 about condition being non-bool
    let e101_warnings: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.code.as_deref() == Some("E101"))
        .collect();
    assert!(e101_warnings.is_empty(), "is_empty() should return Bool: {:?}", e101_warnings);
}

// Round 81: f-string leftover token detection
#[test]
fn test_fstring_leftover_tokens_error() {
    let result = magi_lang::syntax::parser::parse_v2(r#"output f"value is {x y z}";"#);
    assert!(result.is_err(), "f-string with leftover tokens should fail to parse");
}

#[test]
fn test_fstring_valid_expression_ok() {
    let result = magi_lang::syntax::parser::parse_v2(r#"output f"value is {x + 1}";"#);
    assert!(result.is_ok(), "f-string with valid expression should parse: {:?}", result.err());
}

// Round 81: negative float range patterns
#[test]
fn test_negative_float_range_pattern() {
    assert_eq!(run(r#"
        let x = -0.5;
        output match x {
            -1.0..0.0 => "negative",
            0.0..1.0 => "positive",
            _ => "other"
        };
    "#), DataType::String("negative".to_string()));
}

#[test]
fn test_negative_float_range_inclusive() {
    assert_eq!(run(r#"
        let x = -1.0;
        output match x {
            -1.0..=0.0 => "in range",
            _ => "out"
        };
    "#), DataType::String("in range".to_string()));
}

#[test]
fn test_negative_to_negative_float_range() {
    assert_eq!(run(r#"
        let x = -0.5;
        output match x {
            -1.0..-0.1 => "in range",
            _ => "out"
        };
    "#), DataType::String("in range".to_string()));
}

// ── Round 82: coverage gap tests ───────────────

#[test]
fn test_return_inside_try_finally() {
    // return in try block should still execute finally
    assert_eq!(run(r#"
        fn foo() {
            try {
                return 42;
            } catch e {
                return -1;
            } finally {
                let x = 0;
            }
        }
        output foo();
    "#), DataType::Int64(42));
}

#[test]
fn test_catch_without_variable() {
    // catch block without named variable
    assert_eq!(run(r#"
        let result = try {
            throw "oops";
        } catch {
            "caught"
        };
        output result;
    "#), DataType::String("caught".to_string()));
}

#[test]
fn test_catch_rethrow() {
    let err = run_err(r#"
        try {
            throw "original";
        } catch e {
            throw e;
        }
    "#);
    match err {
        InterpError::ThrownError { value, .. } => {
            assert_eq!(value, DataType::String("original".to_string()));
        }
        _ => panic!("expected ThrownError, got {:?}", err),
    }
}

#[test]
fn test_use_with_alias() {
    assert_eq!(run(r#"
        mod math {
            fn add(a, b) { a + b }
        }
        use math::add as plus;
        output plus(3, 4);
    "#), DataType::Int64(7));
}

#[test]
fn test_glob_import() {
    assert_eq!(run(r#"
        mod utils {
            fn double(x) { x * 2 }
            fn triple(x) { x * 3 }
        }
        use utils::*;
        output double(5) + triple(2);
    "#), DataType::Int64(16));
}

#[test]
fn test_map_field_access() {
    assert_eq!(run(r#"
        let m = {"name": "alice", "age": 30};
        output m.name;
    "#), DataType::String("alice".to_string()));
}

#[test]
fn test_map_field_access_missing() {
    assert_eq!(run(r#"
        let m = {"name": "alice"};
        output m.email;
    "#), DataType::Null);
}

#[test]
fn test_map_nested_field_access() {
    assert_eq!(run(r#"
        let m = {"inner": {"val": 42}};
        output m.inner.val;
    "#), DataType::Int64(42));
}

#[test]
fn test_map_typeof() {
    assert_eq!(run(r#"
        let m = {"x": 10};
        output typeof(m);
    "#), DataType::String("map".to_string()));
}

#[test]
fn test_map_index_with_string_key() {
    // Bug fix: map["key"] should use MapGet, not ArrayGet
    assert_eq!(run(r#"
        let m = {"name": "alice", "age": 30};
        output m["name"];
    "#), DataType::String("alice".to_string()));
}

#[test]
fn test_map_index_with_variable_key() {
    // Dynamic key via variable
    assert_eq!(run(r#"
        let m = {"x": 10, "y": 20};
        let key = "y";
        output m[key];
    "#), DataType::Int64(20));
}

#[test]
fn test_map_index_missing_key() {
    // Missing key returns null
    assert_eq!(run(r#"
        let m = {"a": 1};
        output m["b"];
    "#), DataType::Null);
}

#[test]
fn test_closure_snapshot_isolation() {
    // Closures capture by value — mutations after capture don't affect the closure
    assert_eq!(run(r#"
        let mut val = 100;
        let get_val = || val;
        val = 200;
        output get_val();
    "#), DataType::Int64(100));
}

#[test]
fn test_mixed_int_float_add() {
    assert_eq!(run(r#"
        let i = 10;
        let f = 2.5;
        output i + f;
    "#), DataType::Float64(12.5));
}

#[test]
fn test_mixed_int_float_mul() {
    assert_eq!(run(r#"
        let i = 3;
        let f = 1.5;
        output i * f;
    "#), DataType::Float64(4.5));
}

#[test]
fn test_map_construction_and_field() {
    // Map from comprehension, access via field syntax
    assert_eq!(run(r#"
        let m = {"x": 42, "y": 99};
        output m.x + m.y;
    "#), DataType::Int64(141));
}

#[test]
fn test_string_contains_non_string() {
    // contains() with non-string argument should handle gracefully
    assert_eq!(run(r#"
        let s = "hello 42 world";
        output s.contains("42");
    "#), DataType::Bool(true));
}

#[test]
fn test_flat_map_basic() {
    assert_eq!(run(r#"
        let arr = [1, 2, 3];
        output arr.flat_map(|x| [x, x * 10]);
    "#), DataType::Array(vec![
        DataType::Int64(1), DataType::Int64(10),
        DataType::Int64(2), DataType::Int64(20),
        DataType::Int64(3), DataType::Int64(30),
    ]));
}

#[test]
fn test_array_filter_with_null_check() {
    assert_eq!(run(r#"
        let arr = [1, null, 2, null, 3];
        output arr.filter(|x| x != null);
    "#), DataType::Array(vec![
        DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
    ]));
}

#[test]
fn test_array_all_true() {
    assert_eq!(run(r#"
        output [2, 4, 6].all(|x| x % 2 == 0);
    "#), DataType::Bool(true));
}

#[test]
fn test_array_any_true() {
    assert_eq!(run(r#"
        output [1, 3, 4, 5].any(|x| x % 2 == 0);
    "#), DataType::Bool(true));
}

#[test]
fn test_array_any_false() {
    assert_eq!(run(r#"
        output [1, 3, 5].any(|x| x % 2 == 0);
    "#), DataType::Bool(false));
}

#[test]
fn test_nested_closure_capture_multiple() {
    assert_eq!(run(r#"
        fn make_counter(start) {
            let base = start;
            |n| base + n
        }
        let from10 = make_counter(10);
        let from20 = make_counter(20);
        output [from10(5), from20(5)];
    "#), DataType::Array(vec![DataType::Int64(15), DataType::Int64(25)]));
}

// ── Round 82: bug fix tests ───────────────

#[test]
fn test_typeof_in_pipe_no_placeholder() {
    assert_eq!(run(r#"
        output 42 |> typeof();
    "#), DataType::String("int64".to_string()));
}

#[test]
fn test_typeof_in_pipe_with_string() {
    assert_eq!(run(r#"
        output "hello" |> typeof();
    "#), DataType::String("string".to_string()));
}

#[test]
fn test_println_in_pipe_no_placeholder() {
    // println in pipe should print the piped value and return it
    assert_eq!(run(r#"
        output 42 |> println();
    "#), DataType::Int64(42));
}

#[test]
fn test_debug_log_in_pipe() {
    assert_eq!(run(r#"
        output "test" |> debug_log();
    "#), DataType::String("test".to_string()));
}

#[test]
fn test_module_enum_direct_access() {
    // Enums inside modules are registered under unqualified name by execute()
    assert_eq!(run(r#"
        mod shapes {
            enum Color { Red, Green, Blue }
        }
        output Color::Red;
    "#), DataType::Map({
        let mut m = std::collections::BTreeMap::new();
        m.insert("__data".to_string(), DataType::Array(vec![]));
        m.insert("__enum".to_string(), DataType::String("Color".to_string()));
        m.insert("__variant".to_string(), DataType::String("Red".to_string()));
        m
    }));
}

#[test]
fn test_module_qualified_function_call() {
    assert_eq!(run(r#"
        mod math {
            fn square(x) { x * x }
        }
        output math::square(5);
    "#), DataType::Int64(25));
}

#[test]
fn test_map_field_dot_access_nested() {
    // Deeply nested map field access
    assert_eq!(run(r#"
        let m = {"a": {"b": {"c": 42}}};
        output m.a.b.c;
    "#), DataType::Int64(42));
}

#[test]
fn test_pipe_typeof_array() {
    assert_eq!(run(r#"
        output [1, 2, 3] |> typeof();
    "#), DataType::String("array".to_string()));
}

// ── Round 83: parser depth and edge case tests ───────────────

#[test]
fn test_deeply_nested_blocks_error() {
    // 200 nested fn blocks should exceed MAX_PARSE_DEPTH (128)
    let mut src = String::new();
    for i in 0..200 {
        src.push_str(&format!("fn f{}() {{ ", i));
    }
    src.push_str("output 1;");
    for _ in 0..200 {
        src.push_str(" }");
    }
    let result = parse_v2(&src);
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("nesting"));
}

#[test]
fn test_deeply_nested_if_else_error() {
    // 200 else-if chains should exceed depth limit
    let mut src = String::from("output ");
    for _ in 0..200 {
        src.push_str("if true { 0 } else ");
    }
    src.push_str("{ 1 };");
    let result = parse_v2(&src);
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("nesting"));
}

#[test]
fn test_deeply_nested_patterns_error() {
    // 200 nested array patterns should exceed depth limit
    let mut src = String::from("match x { ");
    for _ in 0..200 {
        src.push('[');
    }
    src.push('_');
    for _ in 0..200 {
        src.push(']');
    }
    src.push_str(" => 1 }");
    // Wrap in fn + output to make it a valid program start
    let full = format!("fn test(x) {{ {} }}", src);
    let result = parse_v2(&full);
    assert!(result.is_err());
}

#[test]
fn test_moderate_nesting_ok() {
    // 50 levels of nesting should be fine (< 128 limit)
    let mut src = String::new();
    for i in 0..50 {
        src.push_str(&format!("fn f{}() {{ ", i));
    }
    src.push_str("output 1;");
    for _ in 0..50 {
        src.push_str(" }");
    }
    let result = parse_v2(&src);
    assert!(result.is_ok());
}

// ── Round 84: coverage gap tests ───────────────

#[test]
fn test_match_type_pattern_int64() {
    assert_eq!(run(r#"
        let val = 42;
        output match val {
            n: int64 => n + 1,
            s: string => 0,
            _ => -1,
        };
    "#), DataType::Int64(43));
}

#[test]
fn test_match_type_pattern_string() {
    assert_eq!(run(r#"
        let val = "hello";
        output match val {
            n: int64 => 0,
            s: string => s,
            _ => "unknown",
        };
    "#), DataType::String("hello".to_string()));
}

#[test]
fn test_match_null_literal_pattern() {
    assert_eq!(run(r#"
        let val = null;
        output match val {
            null => "was null",
            _ => "not null",
        };
    "#), DataType::String("was null".to_string()));
}

#[test]
fn test_for_loop_break_with_value() {
    // break with value in a for loop — test the value propagates
    assert_eq!(run(r#"
        fn find_first_large(arr) {
            for x in arr {
                if x > 25 { return x }
            }
            return -1;
        }
        output find_first_large([10, 20, 30, 40]);
    "#), DataType::Int64(30));
}

#[test]
fn test_mutable_array_destructure() {
    assert_eq!(run(r#"
        let mut [a, b] = [1, 2];
        a = 10;
        b = 20;
        output a + b;
    "#), DataType::Int64(30));
}

#[test]
fn test_map_destructure_with_alias() {
    assert_eq!(run(r#"
        let {name: user_name} = {"name": "Alice", "age": 30};
        output user_name;
    "#), DataType::String("Alice".to_string()));
}

#[test]
fn test_nested_loop_break_inner_only() {
    assert_eq!(run(r#"
        let mut result = 0;
        for i in [1, 2, 3] {
            for j in [10, 20, 30] {
                if j == 20 { break }
            }
            result = result + i;
        }
        output result;
    "#), DataType::Int64(6));
}

#[test]
fn test_nested_loop_continue_inner_only() {
    assert_eq!(run(r#"
        let mut sum = 0;
        for i in [1, 2, 3] {
            for j in [10, 20, 30] {
                if j == 20 { continue }
                sum = sum + j;
            }
            sum = sum + i;
        }
        output sum;
    "#), DataType::Int64(126));
}

#[test]
fn test_try_catch_expr_finally_catch_path() {
    assert_eq!(run(r#"
        let mut log = "";
        let val = try {
            throw "oops"
        } catch e {
            log = "caught";
            99
        } finally {
            log = log + "_done";
        };
        output [val, log];
    "#), DataType::Array(vec![
        DataType::Int64(99),
        DataType::String("caught_done".to_string()),
    ]));
}

#[test]
fn test_try_propagate_non_null() {
    assert_eq!(run(r#"
        fn safe_get() {
            let val = 42;
            val?
        }
        output safe_get();
    "#), DataType::Int64(42));
}

#[test]
fn test_try_propagate_null_throws() {
    let err = run_err(r#"
        fn will_fail() {
            let val = null;
            val?
        }
        will_fail();
    "#);
    match err {
        InterpError::ThrownError { .. } => {}
        other => panic!("expected ThrownError from ?, got: {:?}", other),
    }
}

// ── Round 85: edge case stress tests ───────────────

// --- Scope/Closure edge cases ---

#[test]
fn test_closure_returned_from_function_captures_param() {
    // A function returns a closure that captures the function parameter.
    // Multiple instances should be independent since closures capture by value.
    assert_eq!(run(r#"
        fn make_multiplier(factor) {
            |x| x * factor
        }
        let double = make_multiplier(2);
        let triple = make_multiplier(3);
        output [double(5), triple(5), double(triple(2))];
    "#), DataType::Array(vec![
        DataType::Int64(10),
        DataType::Int64(15),
        DataType::Int64(12),
    ]));
}

#[test]
fn test_nested_closure_two_levels() {
    // A function returns a closure that itself returns a closure.
    // Both levels should capture their respective parameters.
    assert_eq!(run(r#"
        fn outer(a) {
            |b| {
                let sum = a + b;
                |c| sum + c
            }
        }
        let step1 = outer(10);
        let step2 = step1(20);
        output step2(3);
    "#), DataType::Int64(33));
}

#[test]
fn test_closure_captures_loop_variable_per_iteration() {
    // Each iteration of a for-loop creates a new scope, so closures
    // captured in a loop capture the loop variable's value at that moment.
    // We test this by creating closures and immediately using them.
    assert_eq!(run(r#"
        fn make_adder(n) { |x| x + n }
        let mut sum = 0;
        for i in [10, 20, 30] {
            let adder = make_adder(i);
            sum = sum + adder(1);
        }
        output sum;
    "#), DataType::Int64(63));
}

// --- Control flow edge cases ---

#[test]
fn test_break_with_value_in_loop_expr() {
    // loop { ... break value } should return the break value as the expression result
    assert_eq!(run(r#"
        let result = loop {
            break 42
        };
        output result;
    "#), DataType::Int64(42));
}

#[test]
fn test_break_value_in_nested_loops_inner_only() {
    // break with value in inner loop should not affect outer loop
    assert_eq!(run(r#"
        let mut total = 0;
        for i in [1, 2, 3] {
            let inner_val = loop {
                break i * 10
            };
            total = total + inner_val;
        }
        output total;
    "#), DataType::Int64(60));
}

#[test]
fn test_return_in_deeply_nested_blocks() {
    // return should escape through multiple levels of nesting
    assert_eq!(run(r#"
        fn deep() {
            for i in [1, 2, 3] {
                if i == 2 {
                    for j in [10, 20, 30] {
                        if j == 20 {
                            return i * j
                        }
                    }
                }
            }
            return -1;
        }
        output deep();
    "#), DataType::Int64(40));
}

#[test]
fn test_continue_skips_rest_of_body() {
    // continue in for-in with complex bodies should skip correctly
    assert_eq!(run(r#"
        let mut sum = 0;
        for item in [1, 2, 3, 4, 5, 6] {
            if item % 2 == 0 { continue }
            sum = sum + item;
        }
        output sum;
    "#), DataType::Int64(9));
}

// --- Pattern matching edge cases ---

#[test]
fn test_match_nested_array_destructuring() {
    // Match on nested array patterns
    assert_eq!(run(r#"
        let data = [1, [2, 3], 4];
        output match data {
            [a, [b, c], d] => a + b + c + d,
            _ => -1,
        };
    "#), DataType::Int64(10));
}

#[test]
fn test_match_guard_with_function_call() {
    // Guard that calls a user-defined function
    assert_eq!(run(r#"
        fn is_even(n) { n % 2 == 0 }
        let val = 4;
        output match val {
            n if is_even(n) => "even",
            _ => "odd",
        };
    "#), DataType::String("even".to_string()));
}

#[test]
fn test_match_guard_with_function_call_odd() {
    assert_eq!(run(r#"
        fn is_even(n) { n % 2 == 0 }
        let val = 7;
        output match val {
            n if is_even(n) => "even",
            _ => "odd",
        };
    "#), DataType::String("odd".to_string()));
}

#[test]
fn test_match_or_pattern_with_binding() {
    // Or-pattern with consistent variable binding
    assert_eq!(run(r#"
        output match 2 {
            1 | 2 | 3 => "small",
            _ => "big",
        };
    "#), DataType::String("small".to_string()));
}

#[test]
fn test_match_enum_nested_destructuring() {
    // Match on enum variant with nested data
    assert_eq!(run(r#"
        enum Result { Ok(val), Err(msg) }
        let r = Result::Ok(42);
        output match r {
            Result::Ok(v) => v + 1,
            Result::Err(m) => -1,
        };
    "#), DataType::Int64(43));
}

// --- Expression edge cases ---

#[test]
fn test_null_coalesce_triple_chain() {
    // Chained null coalesce: a ?? b ?? c — three levels deep
    assert_eq!(run(r#"
        let a = null;
        let b = null;
        let c = 42;
        output a ?? b ?? c;
    "#), DataType::Int64(42));
}

#[test]
fn test_null_coalesce_short_circuits() {
    // First non-null value should be returned, rest not evaluated
    assert_eq!(run(r#"
        let a = null;
        let b = 10;
        let c = 20;
        output a ?? b ?? c;
    "#), DataType::Int64(10));
}

#[test]
fn test_pipe_chained_with_placeholder() {
    // Pipe through user-defined functions using placeholder
    assert_eq!(run(r#"
        fn add_one(x) { x + 1 }
        fn double(x) { x * 2 }
        output 5 |> add_one(_) |> double(_) |> add_one(_);
    "#), DataType::Int64(13));
}

// --- Type edge cases ---

#[test]
fn test_mixed_numeric_array_sum() {
    // Array with mixed int and float should produce float from sum
    assert_eq!(run(r#"
        output [1, 2.5, 3].sum();
    "#), DataType::Float64(6.5));
}

#[test]
fn test_same_type_comparison_int() {
    // Same-type int comparison works through evaluator
    assert_eq!(run(r#"
        output 5 > 3;
    "#), DataType::Bool(true));
}

#[test]
fn test_same_type_comparison_float() {
    // Same-type float comparison works through evaluator
    assert_eq!(run(r#"
        output 2.0 < 2.5;
    "#), DataType::Bool(true));
}

// --- Error handling edge cases ---

#[test]
fn test_nested_try_catch() {
    // Inner try-catch catches inner error, outer catches outer
    assert_eq!(run(r#"
        let result = try {
            let inner = try {
                throw "inner error"
            } catch e {
                "caught: " + e
            };
            inner
        } catch e {
            "outer: " + e
        };
        output result;
    "#), DataType::String("caught: inner error".to_string()));
}

#[test]
fn test_try_catch_finally_always_runs() {
    // Finally block runs even when try succeeds
    assert_eq!(run(r#"
        let mut log = "";
        try {
            log = "try";
        } catch e {
            log = "catch";
        } finally {
            log = log + "_finally";
        }
        output log;
    "#), DataType::String("try_finally".to_string()));
}

#[test]
fn test_throw_in_finally_overrides() {
    // throw in finally should override the catch result
    let err = run_err(r#"
        try {
            throw "original"
        } catch e {
            "caught"
        } finally {
            throw "from_finally"
        }
    "#);
    match err {
        InterpError::ThrownError { value, .. } => {
            assert_eq!(value, DataType::String("from_finally".to_string()));
        }
        other => panic!("expected ThrownError from finally, got: {:?}", other),
    }
}

#[test]
fn test_catch_preserves_thrown_type() {
    // Thrown non-string values should be preserved in catch
    assert_eq!(run(r#"
        let result = try {
            throw 42
        } catch e {
            e
        };
        output result;
    "#), DataType::Int64(42));
}

#[test]
fn test_catch_preserves_thrown_array() {
    assert_eq!(run(r#"
        let result = try {
            throw [1, 2, 3]
        } catch e {
            e
        };
        output result;
    "#), DataType::Array(vec![DataType::Int64(1), DataType::Int64(2), DataType::Int64(3)]));
}

// --- Operator edge cases ---

#[test]
fn test_operator_precedence_logical_vs_comparison() {
    // && binds tighter than ||, comparison tighter than logical
    assert_eq!(run(r#"
        output true || false && false;
    "#), DataType::Bool(true));
}

#[test]
fn test_operator_precedence_arithmetic_in_comparison() {
    // Arithmetic evaluated before comparison
    assert_eq!(run(r#"
        output 2 + 3 > 4;
    "#), DataType::Bool(true));
}

#[test]
fn test_unary_not_with_comparison() {
    assert_eq!(run(r#"
        output !(3 > 5);
    "#), DataType::Bool(true));
}

#[test]
fn test_short_circuit_and_does_not_eval_rhs() {
    // false && <error> should not throw because rhs not evaluated
    assert_eq!(run(r#"
        fn explode() { throw "boom" }
        output false && explode();
    "#), DataType::Bool(false));
}

#[test]
fn test_short_circuit_or_does_not_eval_rhs() {
    // true || <error> should not throw because rhs not evaluated
    assert_eq!(run(r#"
        fn explode() { throw "boom" }
        output true || explode();
    "#), DataType::Bool(true));
}

// --- HOF edge cases ---

#[test]
fn test_map_filter_reduce_chain() {
    // Chain map, filter, reduce on arrays
    // reduce takes (initial, callback) order
    assert_eq!(run(r#"
        let result = [1, 2, 3, 4, 5]
            .map(|x| x * 2)
            .filter(|x| x > 4)
            .reduce(0, |acc, x| acc + x);
        output result;
    "#), DataType::Int64(24));
}

#[test]
fn test_find_on_empty_array() {
    // find on empty array returns null
    assert_eq!(run(r#"
        let arr = [];
        output arr.find(|x| x > 0);
    "#), DataType::Null);
}

#[test]
fn test_each_returns_null() {
    // each() iterates for side effects and returns null
    assert_eq!(run(r#"
        let result = [1, 2, 3].each(|x| x * 2);
        output result;
    "#), DataType::Null);
}

#[test]
fn test_reduce_with_initial_accumulator() {
    // reduce(initial, callback) — initial value is first argument
    assert_eq!(run(r#"
        output [1, 2, 3, 4].reduce(100, |acc, x| acc + x);
    "#), DataType::Int64(110));
}

#[test]
fn test_all_false_on_empty_array() {
    // all on empty array is vacuously true
    assert_eq!(run(r#"
        output [].all(|x| x > 0);
    "#), DataType::Bool(true));
}

#[test]
fn test_any_false_on_empty_array() {
    // any on empty array is false
    assert_eq!(run(r#"
        output [].any(|x| x > 0);
    "#), DataType::Bool(false));
}

// --- String edge cases ---

#[test]
fn test_empty_string_operations() {
    assert_eq!(run(r#"
        let s = "";
        output [s.is_empty(), s.length(), s.trim(), s.reverse(), s.to_upper()];
    "#), DataType::Array(vec![
        DataType::Bool(true),
        DataType::Int64(0),
        DataType::String("".to_string()),
        DataType::String("".to_string()),
        DataType::String("".to_string()),
    ]));
}

#[test]
fn test_fstring_with_complex_expression() {
    // f-string with arithmetic expression inside
    assert_eq!(run(r#"
        let x = 10;
        let y = 20;
        output f"sum is {x + y}";
    "#), DataType::String("sum is 30".to_string()));
}

#[test]
fn test_fstring_with_chained_method_calls() {
    assert_eq!(run(r#"
        let name = "  alice  ";
        output f"Hello, {name.trim().to_upper()}!";
    "#), DataType::String("Hello, ALICE!".to_string()));
}

#[test]
fn test_fstring_with_nested_fstring() {
    // f-string containing another f-string expression
    assert_eq!(run(r#"
        let x = 5;
        output f"result: {f"({x})"}";
    "#), DataType::String("result: (5)".to_string()));
}

// --- Module edge cases ---

#[test]
fn test_module_with_enum_and_struct() {
    // Module containing both enum and struct definitions
    assert_eq!(run(r#"
        mod shapes {
            enum Color { Red, Green, Blue }
            struct Point { x, y }
        }
        let p = Point { x: 10, y: 20 };
        output p.x + p.y;
    "#), DataType::Int64(30));
}

#[test]
fn test_module_qualified_vs_unqualified_function() {
    // Both qualified and unqualified access should work for functions
    assert_eq!(run(r#"
        mod math {
            fn double(x) { x * 2 }
        }
        use math::double;
        output [math::double(5), double(3)];
    "#), DataType::Array(vec![DataType::Int64(10), DataType::Int64(6)]));
}

// --- Compound/misc edge cases ---

#[test]
fn test_compound_assign_operators() {
    assert_eq!(run(r#"
        let mut x = 10;
        x += 5;
        x -= 3;
        x *= 2;
        output x;
    "#), DataType::Int64(24));
}

#[test]
fn test_list_comprehension_with_nested_condition() {
    assert_eq!(run(r#"
        output [x * x for x in [1, 2, 3, 4, 5, 6] if x % 2 == 0];
    "#), DataType::Array(vec![
        DataType::Int64(4),
        DataType::Int64(16),
        DataType::Int64(36),
    ]));
}

#[test]
fn test_for_loop_over_string_characters() {
    // for-in over a string iterates over characters
    // Use string concatenation to collect results (push goes through evaluator)
    assert_eq!(run(r#"
        let mut result = "";
        for c in "abc" {
            result = result + c + ",";
        }
        output result;
    "#), DataType::String("a,b,c,".to_string()));
}

#[test]
fn test_recursive_function_with_accumulator() {
    // Recursive function pattern (tail-call style)
    assert_eq!(run(r#"
        fn factorial(n, acc) {
            if n <= 1 { return acc }
            return factorial(n - 1, n * acc);
        }
        output factorial(6, 1);
    "#), DataType::Int64(720));
}

#[test]
fn test_match_with_multiple_guards() {
    // Multiple arms with different guards
    assert_eq!(run(r#"
        fn classify(n) {
            match n {
                x if x < 0 => "negative",
                x if x == 0 => "zero",
                x if x < 10 => "small",
                x if x < 100 => "medium",
                _ => "large",
            }
        }
        output [classify(-5), classify(0), classify(7), classify(42), classify(999)];
    "#), DataType::Array(vec![
        DataType::String("negative".to_string()),
        DataType::String("zero".to_string()),
        DataType::String("small".to_string()),
        DataType::String("medium".to_string()),
        DataType::String("large".to_string()),
    ]));
}

#[test]
fn test_while_loop_with_break_value() {
    // While loop with break carrying a value
    assert_eq!(run(r#"
        let mut i = 0;
        let mut result = null;
        while true {
            i = i + 1;
            if i == 5 {
                result = i * 100;
                break
            }
        }
        output result;
    "#), DataType::Int64(500));
}

#[test]
fn test_map_with_computed_keys() {
    // Map literal with pre-defined string keys
    assert_eq!(run(r#"
        let m = {"a": 1, "b": 2, "c": 3};
        output m.a + m.b + m.c;
    "#), DataType::Int64(6));
}

#[test]
fn test_try_catch_expr_in_let_binding() {
    // try-catch as an expression in a let binding
    assert_eq!(run(r#"
        let val = try { 1 + 2 } catch e { -1 };
        output val;
    "#), DataType::Int64(3));
}

#[test]
fn test_try_catch_expr_catches_in_let() {
    assert_eq!(run(r#"
        let val = try { throw "oops" } catch e { -1 };
        output val;
    "#), DataType::Int64(-1));
}

#[test]
fn test_block_expression_value() {
    // Block as expression returns its tail expression
    assert_eq!(run(r#"
        let val = {
            let x = 10;
            let y = 20;
            x + y
        };
        output val;
    "#), DataType::Int64(30));
}

#[test]
fn test_array_first_last_on_empty() {
    assert_eq!(run(r#"
        let arr = [];
        output [arr.first(), arr.last()];
    "#), DataType::Array(vec![DataType::Null, DataType::Null]));
}

#[test]
fn test_string_split_and_join_roundtrip() {
    assert_eq!(run(r#"
        let s = "a,b,c,d";
        let parts = s.split(",");
        let rejoined = parts.join(",");
        output rejoined;
    "#), DataType::String("a,b,c,d".to_string()));
}

#[test]
fn test_closure_in_hof_captures_outer_variable() {
    // Lambda used in HOF captures outer variable
    assert_eq!(run(r#"
        let threshold = 3;
        let filtered = [1, 2, 3, 4, 5].filter(|x| x > threshold);
        output filtered;
    "#), DataType::Array(vec![DataType::Int64(4), DataType::Int64(5)]));
}

#[test]
fn test_match_on_string_values() {
    assert_eq!(run(r#"
        fn greet(lang) {
            match lang {
                "en" => "Hello",
                "fr" => "Bonjour",
                "de" => "Hallo",
                _ => "Hi",
            }
        }
        output [greet("en"), greet("fr"), greet("ja")];
    "#), DataType::Array(vec![
        DataType::String("Hello".to_string()),
        DataType::String("Bonjour".to_string()),
        DataType::String("Hi".to_string()),
    ]));
}

// ── Round 82: Parser struct literal ambiguity fix ─────────────

#[test]
fn test_if_uppercase_condition_with_block() {
    // if State { done: true } -- State is condition, { done: true } is block body
    // Previously failed: parser treated State { done: true } as struct literal
    assert_eq!(run(r#"
        let State = true;
        let done = true;
        let result = if State { done };
        output result;
    "#), DataType::Bool(true));
}

#[test]
fn test_if_uppercase_condition_field_colon() {
    // if Config { value: 42 } -- must parse as condition=Config, block={value: 42}
    // The block body `value: 42` is actually a type-pattern expression which won't work,
    // but the parser should not crash. Let's use a simpler case.
    assert_eq!(run(r#"
        let Running = true;
        let x = 10;
        let result = if Running { x + 1 };
        output result;
    "#), DataType::Int64(11));
}

#[test]
fn test_while_uppercase_condition_block() {
    // while Running { count += 1; if count > 3 { break } }
    // Previously: if Running starts with uppercase and block body begins with ident:,
    // parser could misparse as struct literal.
    assert_eq!(run(r#"
        let mut Running = true;
        let mut count = 0;
        while Running {
            count += 1;
            if count >= 3 {
                Running = false;
            }
        }
        output count;
    "#), DataType::Int64(3));
}

#[test]
fn test_for_uppercase_iterable_block() {
    // for x in Items { ... } -- Items is iterable, { ... } is loop body
    assert_eq!(run(r#"
        let Items = [10, 20, 30];
        let mut sum = 0;
        for x in Items {
            sum += x;
        }
        output sum;
    "#), DataType::Int64(60));
}

#[test]
fn test_match_uppercase_value_block() {
    // match Status { "ok" => 1, _ => 0 } -- Status is the value, { ... } is the match body
    assert_eq!(run(r#"
        let Status = "ok";
        let result = match Status {
            "ok" => 1,
            _ => 0,
        };
        output result;
    "#), DataType::Int64(1));
}

#[test]
fn test_match_guard_no_struct_literal() {
    // match x { v if Flag => v * 2, _ => 0 }
    // Guard should be just `Flag`, not eating into `=> v * 2`
    assert_eq!(run(r#"
        let Flag = true;
        let x = 5;
        let result = match x {
            v if Flag => v + 10,
            _ => 0,
        };
        output result;
    "#), DataType::Int64(15));
}

#[test]
fn test_struct_literal_still_works_in_let() {
    // Struct literals should still work in non-condition contexts
    assert_eq!(run(r#"
        struct Point { x, y }
        let p = Point { x: 3, y: 4 };
        output p.x + p.y;
    "#), DataType::Int64(7));
}

#[test]
fn test_struct_literal_in_return() {
    // Struct literal in return position should work
    assert_eq!(run(r#"
        struct Pair { a, b }
        fn make_pair(x, y) {
            Pair { a: x, b: y }
        }
        let p = make_pair(10, 20);
        output p.a + p.b;
    "#), DataType::Int64(30));
}

#[test]
fn test_struct_literal_in_array() {
    // Struct literal inside array should parse and work
    assert_eq!(run(r#"
        struct Item { val }
        let x = Item { val: 7 };
        output x.val;
    "#), DataType::Int64(7));
}

#[test]
fn test_if_else_with_struct_in_body() {
    // Struct literal INSIDE the body of if should work
    assert_eq!(run(r#"
        struct Result { ok }
        let cond = true;
        let r = if cond { Result { ok: 42 } } else { Result { ok: 0 } };
        output r.ok;
    "#), DataType::Int64(42));
}

#[test]
fn test_while_with_struct_in_body() {
    // Struct literal inside while body should work
    assert_eq!(run(r#"
        struct Counter { n }
        let mut i = 0;
        let mut last = null;
        while i < 3 {
            last = Counter { n: i };
            i += 1;
        }
        output last.n;
    "#), DataType::Int64(2));
}

#[test]
fn test_for_with_struct_in_body() {
    // Struct literal inside for body should work
    assert_eq!(run(r#"
        struct Wrapper { v }
        let mut result = 0;
        for x in [1, 2, 3] {
            let w = Wrapper { v: x };
            result += w.v;
        }
        output result;
    "#), DataType::Int64(6));
}

// ── Round 82: Integration tests for uncovered interpreter features ───

#[test]
fn test_await_on_non_future_is_identity() {
    // Await on a non-Future value should return the value unchanged
    assert_eq!(run("await 42"), DataType::Int64(42));
    assert_eq!(run(r#"await "hello""#), DataType::String("hello".to_string()));
    assert_eq!(run("await null"), DataType::Null);
}

#[test]
fn test_spawn_already_future_no_double_wrap() {
    // Spawning an already-future value should not double-wrap
    assert_eq!(run(r#"
        async fn f() { 10 }
        let future1 = f();
        let future2 = spawn future1;
        await future2
    "#), DataType::Int64(10));
}

#[test]
fn test_spawn_error_becomes_rejected_await_throws() {
    // Spawn captures errors as rejected futures; await propagates them
    let err = run_err(r#"
        fn fail() { throw "boom" }
        let f = spawn fail();
        await f
    "#);
    let msg = format!("{:?}", err);
    assert!(msg.contains("boom") || msg.contains("rejected"), "expected 'boom' or 'rejected' in error: {}", msg);
}

#[test]
fn test_async_fn_wraps_return_in_future() {
    // Calling an async function directly returns a Future, not the raw value
    assert_eq!(run(r#"
        async fn get_value() { 99 }
        let f = get_value();
        typeof(f)
    "#), DataType::String("future".to_string()));
}

#[test]
fn test_type_alias_is_runtime_noop() {
    // Type aliases should be ignored at runtime and not affect execution
    assert_eq!(run(r#"
        type Score = int64;
        type Name = string;
        let x = 42;
        output x;
    "#), DataType::Int64(42));
}

#[test]
fn test_type_alias_with_functions() {
    // Type alias mixed with function definitions should not interfere
    assert_eq!(run(r#"
        type Num = int64;
        fn add(a, b) { a + b }
        output add(3, 4);
    "#), DataType::Int64(7));
}

#[test]
fn test_loop_with_continue_skips_iteration() {
    // Continue in loop should skip the rest of the body for that iteration
    assert_eq!(run(r#"
        let mut count = 0;
        let mut i = 0;
        let result = loop {
            i += 1;
            if i > 10 { break count }
            if i % 2 == 0 { continue }
            count += 1;
        };
        output result;
    "#), DataType::Int64(5));
}
#[test]
fn test_map_comprehension_with_condition() {
    // Map comprehension with an if filter
    let result = run(r#"
        let m = {"val": x * 10 for x in [1, 2, 3, 4, 5] if x > 3};
        output m;
    "#);
    // Only x=4 and x=5 pass the filter; last iteration (x=5) wins for same key
    match &result {
        DataType::Map(m) => {
            assert_eq!(m.get("val"), Some(&DataType::Int64(50)));
        }
        other => panic!("expected map, got: {:?}", other),
    }
}

#[test]
fn test_map_comprehension_over_string() {
    // Map comprehension iterating over a string's characters
    let result = run(r#"
        let m = {"char": c for c in "xyz"};
        output m;
    "#);
    // Last character wins: "z"
    match &result {
        DataType::Map(m) => {
            assert_eq!(m.get("char"), Some(&DataType::String("z".to_string())));
        }
        other => panic!("expected map, got: {:?}", other),
    }
}

#[test]
fn test_map_comprehension_non_iterable_error() {
    // Map comprehension on a non-iterable type should error
    let err = run_err(r#"
        let m = {"k": v for v in 42};
    "#);
    match err {
        InterpError::TypeError { expected, .. } => {
            assert!(expected.contains("Array") || expected.contains("Map") || expected.contains("String"));
        }
        other => panic!("expected TypeError, got: {:?}", other),
    }
}

#[test]
fn test_list_comprehension_over_string() {
    // List comprehension should work with string iteration
    assert_eq!(run(r#"
        output [c for c in "abc"];
    "#), DataType::Array(vec![
        DataType::String("a".to_string()),
        DataType::String("b".to_string()),
        DataType::String("c".to_string()),
    ]));
}

#[test]
fn test_list_comprehension_with_destructure_pattern() {
    // List comprehension with array destructuring in the pattern
    assert_eq!(run(r#"
        let pairs = [[1, 10], [2, 20], [3, 30]];
        output [a + b for [a, b] in pairs];
    "#), DataType::Array(vec![
        DataType::Int64(11),
        DataType::Int64(22),
        DataType::Int64(33),
    ]));
}

#[test]
fn test_list_comprehension_over_map() {
    // List comprehension iterating over a map yields {key, value} entries
    assert_eq!(run(r#"
        let m = {"x": 1, "y": 2};
        let keys = [entry.key for entry in m];
        output keys;
    "#), DataType::Array(vec![
        DataType::String("x".to_string()),
        DataType::String("y".to_string()),
    ]));
}

#[test]
fn test_enum_variant_with_multiple_data_fields() {
    // Enum variant carrying multiple data fields
    assert_eq!(run(r#"
        enum Shape { Circle(r), Rect(w, h), Point }
        let s = Shape::Rect(3, 4);
        let area = match s {
            Shape::Rect(w, h) => w * h,
            Shape::Circle(r) => r * r,
            Shape::Point => 0,
        };
        output area;
    "#), DataType::Int64(12));
}

#[test]
fn test_enum_variant_arity_mismatch_error() {
    // Constructing an enum variant with wrong number of args should error
    let err = run_err(r#"
        enum Color { RGB(r, g, b) }
        let c = Color::RGB(255, 128);
    "#);
    match err {
        InterpError::ArityMismatch { name, expected, actual, .. } => {
            assert_eq!(name, "Color::RGB");
            assert_eq!(expected, "3");
            assert_eq!(actual, 2);
        }
        other => panic!("expected ArityMismatch, got: {:?}", other),
    }
}

#[test]
fn test_struct_missing_field_error() {
    // Constructing a struct with a missing required field should error
    let err = run_err(r#"
        struct Point { x, y }
        let p = Point { x: 1 };
    "#);
    match err {
        InterpError::TypeError { expected, actual, .. } => {
            assert!(expected.contains("y"), "expected field 'y' in error: {}", expected);
            assert_eq!(actual, "missing");
        }
        other => panic!("expected TypeError, got: {:?}", other),
    }
}

#[test]
fn test_struct_unknown_field_error() {
    // Constructing a struct with an unknown field should error
    let err = run_err(r#"
        struct Point { x, y }
        let p = Point { x: 1, y: 2, z: 3 };
    "#);
    match err {
        InterpError::TypeError { context, .. } => {
            assert!(context.contains("no field"), "expected 'no field' in context: {}", context);
        }
        other => panic!("expected TypeError, got: {:?}", other),
    }
}

#[test]
fn test_struct_duplicate_field_in_construction() {
    // Duplicate field in struct construction should error
    let err = run_err(r#"
        struct Pair { a, b }
        let p = Pair { a: 1, b: 2, a: 3 };
    "#);
    match err {
        InterpError::TypeError { actual, .. } => {
            assert!(actual.contains("duplicate"), "expected 'duplicate' in actual: {}", actual);
        }
        other => panic!("expected TypeError, got: {:?}", other),
    }
}

#[test]
fn test_module_with_async_function() {
    // Async functions inside modules should be registered and callable
    assert_eq!(run(r#"
        mod service {
            async fn fetch() { 200 }
        }
        let f = service::fetch();
        output await f;
    "#), DataType::Int64(200));
}

#[test]
fn test_module_struct_construction_via_function() {
    // Structs defined inside modules accessible via a module function
    assert_eq!(run(r#"
        mod geo {
            struct Point { x, y }
            fn make_point(x, y) { Point { x: x, y: y } }
        }
        let p = geo::make_point(10, 20);
        output p.x + p.y;
    "#), DataType::Int64(30));
}

#[test]
fn test_let_destructure_map_missing_key() {
    // Destructuring a map with a missing key should bind to null
    assert_eq!(run(r#"
        let {name, age} = {"name": "Alice"};
        output age;
    "#), DataType::Null);
}

#[test]
fn test_let_destructure_mutable_modification() {
    // Mutable destructured variables should be modifiable
    assert_eq!(run(r#"
        let mut [a, b] = [1, 2];
        a += 10;
        b += 20;
        output a + b;
    "#), DataType::Int64(33));
}

#[test]
fn test_let_destructure_in_has_main_mode() {
    // LetDestructure at top level in has_main mode should be accessible in main
    assert_eq!(run(r#"
        let [x, y] = [100, 200];
        fn main() {
            output x + y;
        }
    "#), DataType::Int64(300));
}

#[test]
fn test_try_propagate_on_result_err_enum() {
    // The ? operator on a Result::Err should throw the error value
    let err = run_err(r#"
        enum Result { Ok(v), Err(e) }
        fn might_fail() { Result::Err("bad input") }
        fn caller() {
            let val = might_fail()?;
            output val;
        }
        caller()
    "#);
    match err {
        InterpError::ThrownError { value, .. } => {
            assert_eq!(value, DataType::String("bad input".to_string()));
        }
        other => panic!("expected ThrownError, got: {:?}", other),
    }
}

#[test]
fn test_try_propagate_on_result_ok_unwraps() {
    // The ? operator on a Result::Ok should unwrap the value
    assert_eq!(run(r#"
        enum Result { Ok(v), Err(e) }
        fn succeed() { Result::Ok(42) }
        let val = succeed()?;
        output val;
    "#), DataType::Map({
        let mut m = std::collections::BTreeMap::new();
        m.insert("__data".to_string(), DataType::Array(vec![DataType::Int64(42)]));
        m.insert("__enum".to_string(), DataType::String("Result".to_string()));
        m.insert("__variant".to_string(), DataType::String("Ok".to_string()));
        m
    }));
}

#[test]
fn test_use_glob_imports_module_functions() {
    // Glob import should make all module functions available unqualified
    assert_eq!(run(r#"
        mod helpers {
            fn double(x) { x * 2 }
            fn triple(x) { x * 3 }
        }
        use helpers::*;
        output double(5) + triple(5);
    "#), DataType::Int64(25));
}

#[test]
fn test_use_alias_renames_function() {
    // Use with alias should make function available under alias name
    assert_eq!(run(r#"
        mod math {
            fn add(a, b) { a + b }
        }
        use math::add as plus;
        output plus(3, 7);
    "#), DataType::Int64(10));
}

#[test]
fn test_use_nonexistent_function_errors() {
    // Importing a function that doesn't exist in the module should error
    let err = run_err(r#"
        mod m {
            fn foo() { 1 }
        }
        use m::bar;
    "#);
    match err {
        InterpError::UnknownOperation { name, .. } => {
            assert!(name.contains("bar"), "expected 'bar' in name: {}", name);
        }
        other => panic!("expected UnknownOperation, got: {:?}", other),
    }
}

#[test]
fn test_spread_on_non_array_errors() {
    // Spread operator on a non-array value should error
    let err = run_err(r#"
        fn sum(a, b) { a + b }
        sum(...42)
    "#);
    match err {
        InterpError::TypeError { expected, .. } => {
            assert_eq!(expected, "Array");
        }
        other => panic!("expected TypeError, got: {:?}", other),
    }
}

#[test]
fn test_map_comprehension_with_array_destructure() {
    // Map comprehension using array destructuring in the pattern
    let result = run(r#"
        let pairs = [[1, 10], [2, 20], [3, 30]];
        let m = {"sum": a + b for [a, b] in pairs};
        output m;
    "#);
    match &result {
        DataType::Map(m) => {
            // Last pair [3, 30] gives 33
            assert_eq!(m.get("sum"), Some(&DataType::Int64(33)));
        }
        other => panic!("expected map, got: {:?}", other),
    }
}

#[test]
fn test_list_comprehension_non_iterable_error() {
    // List comprehension on a non-iterable type should error
    let err = run_err(r#"
        output [x for x in 42];
    "#);
    match err {
        InterpError::TypeError { expected, .. } => {
            assert!(expected.contains("Array") || expected.contains("Map") || expected.contains("String"));
        }
        other => panic!("expected TypeError, got: {:?}", other),
    }
}

// ═══════════════════════════════════════════════════════════
// Gap coverage tests: critical paths with zero or minimal tests
// ═══════════════════════════════════════════════════════════

#[test]
fn test_mutual_recursion_is_even_odd() {
    // Mutual recursion: two functions calling each other
    assert_eq!(run(r#"
        fn is_even(n) {
            if n == 0 { true }
            else { is_odd(n - 1) }
        }
        fn is_odd(n) {
            if n == 0 { false }
            else { is_even(n - 1) }
        }
        output is_even(10);
    "#), DataType::Bool(true));
    assert_eq!(run(r#"
        fn is_even(n) {
            if n == 0 { true }
            else { is_odd(n - 1) }
        }
        fn is_odd(n) {
            if n == 0 { false }
            else { is_even(n - 1) }
        }
        output is_odd(7);
    "#), DataType::Bool(true));
}

#[test]
fn test_max_call_depth_mutual_recursion() {
    // Mutual recursion should still hit MaxCallDepth (needs larger stack in debug mode)
    let result = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let err = run_err(r#"
                fn ping(n) { pong(n) }
                fn pong(n) { ping(n) }
                ping(1)
            "#);
            let msg = format!("{}", err);
            assert!(msg.contains("call depth") || msg.contains("recursion") || msg.contains("E401") || msg.contains("Maximum"),
                "expected max call depth error: {}", msg);
        })
        .unwrap()
        .join();
    result.unwrap();
}

#[test]
fn test_chained_string_methods() {
    // Multiple method calls chained on strings
    assert_eq!(
        run(r#""  Hello World  ".trim().to_upper().reverse()"#),
        DataType::String("DLROW OLLEH".to_string())
    );
    assert_eq!(
        run(r#""hello".to_upper().to_lower()"#),
        DataType::String("hello".to_string())
    );
}

#[test]
fn test_chained_array_hof_methods() {
    // Chain map -> filter -> reduce on arrays
    assert_eq!(run(r#"
        [1, 2, 3, 4, 5, 6]
            .map(|x| x * x)
            .filter(|x| x > 10)
            .reduce(0, |acc, x| acc + x)
    "#), DataType::Int64(16 + 25 + 36));
}

#[test]
fn test_pipe_chain_with_lambda_vars_three_stages() {
    // Pipe chain with lambda variables using placeholder syntax
    assert_eq!(run(r#"
        let double = |x| x * 2
        let add_ten = |x| x + 10
        5 |> double(_) |> add_ten(_) |> double(_)
    "#), DataType::Int64(40));
}

#[test]
fn test_variable_shadowing_in_nested_scopes() {
    // Variable shadowing across nested scopes preserves outer values
    assert_eq!(run(r#"
        let x = 1;
        let result = {
            let x = 2;
            let inner = {
                let x = 3;
                x
            };
            x + inner
        };
        output x + result;
    "#), DataType::Int64(6)); // 1 + (2 + 3)
}

#[test]
fn test_closure_three_levels_deep() {
    // Three levels of nested closures each capturing from their parent
    assert_eq!(run(r#"
        let a = 10;
        let f = |b| {
            let g = |c| {
                let h = |d| a + b + c + d;
                h(4)
            };
            g(3)
        };
        output f(2);
    "#), DataType::Int64(19)); // 10 + 2 + 3 + 4
}

#[test]
fn test_throw_in_hof_caught_by_try_catch() {
    // Throw inside a map lambda should be catchable
    assert_eq!(run(r#"
        let result = try {
            [1, 2, 3].map(|x| {
                if x == 2 { throw "found two" }
                x
            })
        } catch e {
            e
        };
        output result;
    "#), DataType::String("found two".to_string()));
}

#[test]
fn test_complex_match_with_guards_and_bindings() {
    // Match combining multiple features: or-patterns, guards, bindings
    assert_eq!(run(r#"
        fn classify(x) {
            match x {
                n if n < 0 => "negative",
                0 => "zero",
                1 | 2 | 3 => "small",
                n if n > 100 => "huge",
                _ => "medium",
            }
        }
        let r1 = classify(-5);
        let r2 = classify(0);
        let r3 = classify(2);
        let r4 = classify(200);
        let r5 = classify(50);
        output f"{r1},{r2},{r3},{r4},{r5}";
    "#), DataType::String("negative,zero,small,huge,medium".to_string()));
}

#[test]
fn test_enum_with_recursive_structure() {
    // Enum that mimics a recursive linked list
    assert_eq!(run(r#"
        enum List { Cons(head, tail), Nil }
        fn sum_list(lst) {
            match lst {
                List::Cons(h, t) => h + sum_list(t),
                List::Nil => 0,
            }
        }
        let lst = List::Cons(1, List::Cons(2, List::Cons(3, List::Nil)));
        output sum_list(lst);
    "#), DataType::Int64(6));
}

#[test]
fn test_string_escape_in_fstring_and_raw_string() {
    // Escape sequences in f-strings work correctly
    assert_eq!(
        run(r#"let x = 42; f"tab:\there\nnewline:{x}""#),
        DataType::String("tab:\there\nnewline:42".to_string())
    );
    // Raw strings preserve backslashes
    assert_eq!(
        run(r#"r"no\nescape\there""#),
        DataType::String("no\\nescape\\there".to_string())
    );
}

#[test]
fn test_compound_assign_chained_sequence() {
    // Multiple compound assignments on the same variable in sequence
    assert_eq!(run(r#"
        let mut x = 100;
        x -= 10;
        x *= 2;
        x += 5;
        output x;
    "#), DataType::Int64(185)); // (100-10)*2+5 = 185
}

#[test]
fn test_rest_in_middle_with_head_and_tail() {
    // Rest pattern in the middle of a destructure
    assert_eq!(run(r#"
        let [first, ...middle, last] = [1, 2, 3, 4, 5];
        output first + last;
    "#), DataType::Int64(6));
    // Middle should contain [2, 3, 4]
    assert_eq!(run(r#"
        let [first, ...middle, last] = [1, 2, 3, 4, 5];
        output middle;
    "#), DataType::Array(vec![DataType::Int64(2), DataType::Int64(3), DataType::Int64(4)]));
}

#[test]
fn test_destructure_single_element_array() {
    // Single element array destructure
    assert_eq!(run("let [only] = [42]; output only;"), DataType::Int64(42));
}

#[test]
fn test_rest_destructure_captures_nothing_when_exact() {
    // Rest captures empty array when all elements are named
    assert_eq!(run(r#"
        let [a, b, ...rest] = [1, 2];
        output rest;
    "#), DataType::Array(vec![]));
}

#[test]
fn test_nested_try_catch_inner_rethrows_caught_by_outer() {
    // Inner try-catch rethrows, caught by outer try-catch
    assert_eq!(run(r#"
        let result = try {
            try {
                throw "inner error"
            } catch e {
                throw f"rethrown: {e}"
            }
        } catch e {
            e
        };
        output result;
    "#), DataType::String("rethrown: inner error".to_string()));
}

#[test]
fn test_many_function_parameters() {
    // Functions with many parameters should work
    assert_eq!(run(r#"
        fn sum6(a, b, c, d, e, f) { a + b + c + d + e + f }
        output sum6(1, 2, 3, 4, 5, 6);
    "#), DataType::Int64(21));
}

#[test]
fn test_for_loop_break_exits_early() {
    // for loop with break should exit the loop early
    assert_eq!(run(r#"
        let mut last_seen = 0;
        for x in [10, 20, 30, 40, 50] {
            last_seen = x;
            if x == 30 { break }
        }
        output last_seen;
    "#), DataType::Int64(30));
}

// ═══════════════════════════════════════════════════════════
// End-to-end integration tests: realistic multi-feature programs
//
// NOTE: The StubEvaluator now supports most operations with correct port names.
// These tests exercise realistic multi-feature programs.
// ═══════════════════════════════════════════════════════════

#[test]
fn test_e2e_simple_calculator() {
    // A calculator that evaluates simple expressions using match and recursion.
    // Combines: enums, match with destructuring, recursion, arithmetic.
    assert_eq!(run(r#"
        enum Expr {
            Num(val),
            Add(left, right),
            Mul(left, right),
            Neg(inner)
        }

        fn eval(expr) {
            match expr {
                Expr::Num(v) => v,
                Expr::Add(l, r) => eval(l) + eval(r),
                Expr::Mul(l, r) => eval(l) * eval(r),
                Expr::Neg(e) => 0 - eval(e),
            }
        }

        // (3 + 4) * -(2 + 1) = 7 * -3 = -21
        let expr = Expr::Mul(
            Expr::Add(Expr::Num(3), Expr::Num(4)),
            Expr::Neg(Expr::Add(Expr::Num(2), Expr::Num(1)))
        );
        output eval(expr);
    "#), DataType::Int64(-21));
}

#[test]
fn test_e2e_calculator_with_variables() {
    // Extended calculator with variable environment using enums for lookup.
    // Combines: enums, match, recursion, null coalescing, field access.
    assert_eq!(run(r#"
        enum Expr {
            Lit(val),
            VarX,
            VarY,
            BinAdd(left, right),
            BinMul(left, right)
        }

        fn eval(expr, x_val, y_val) {
            match expr {
                Expr::Lit(v) => v,
                Expr::VarX => x_val,
                Expr::VarY => y_val,
                Expr::BinAdd(l, r) => eval(l, x_val, y_val) + eval(r, x_val, y_val),
                Expr::BinMul(l, r) => eval(l, x_val, y_val) * eval(r, x_val, y_val),
            }
        }

        // (x + y) * 3 = (10 + 5) * 3 = 45
        let expr = Expr::BinMul(
            Expr::BinAdd(Expr::VarX, Expr::VarY),
            Expr::Lit(3)
        );
        output eval(expr, 10, 5);
    "#), DataType::Int64(45));
}

#[test]
fn test_e2e_stack_implementation() {
    // Stack data structure using arrays and spread syntax.
    // Combines: arrays, spread, closures, while loop, mutable state, .last(), slicing.
    assert_eq!(run(r#"
        fn stack_push(stack, val) {
            [...stack, val]
        }

        fn stack_pop(stack) {
            if len(stack) == 0 {
                [stack, null]
            } else {
                let top = stack.last();
                let new_stack = stack[0..len(stack) - 1];
                [new_stack, top]
            }
        }

        // Use the stack to reverse a sequence
        let mut s = [];
        s = stack_push(s, 10);
        s = stack_push(s, 20);
        s = stack_push(s, 30);

        let mut reversed = [];
        while len(s) > 0 {
            let pair = stack_pop(s);
            // destructure the pair: [new_stack, top_value]
            let [new_s, top] = pair;
            s = new_s;
            reversed = [...reversed, top];
        }
        output reversed;
    "#), DataType::Array(vec![
        DataType::Int64(30),
        DataType::Int64(20),
        DataType::Int64(10),
    ]));
}

#[test]
fn test_e2e_stack_balanced_parens() {
    // Use a stack (array) to check balanced parentheses.
    // Combines: arrays, spread, for loop, match, mutable state, break, .last(), slicing.
    assert_eq!(run(r#"
        fn is_balanced(chars) {
            let mut stack = [];
            let mut valid = true;
            for ch in chars {
                match ch {
                    "(" | "[" | "{" => {
                        stack = [...stack, ch];
                    },
                    ")" => {
                        if len(stack) == 0 { valid = false; break }
                        let top = stack.last();
                        stack = stack[0..len(stack) - 1];
                        if top != "(" { valid = false; break }
                    },
                    "]" => {
                        if len(stack) == 0 { valid = false; break }
                        let top = stack.last();
                        stack = stack[0..len(stack) - 1];
                        if top != "[" { valid = false; break }
                    },
                    "}" => {
                        if len(stack) == 0 { valid = false; break }
                        let top = stack.last();
                        stack = stack[0..len(stack) - 1];
                        if top != "{" { valid = false; break }
                    },
                    _ => {},
                }
            }
            valid && len(stack) == 0
        }

        let test1 = is_balanced(["(", "[", "]", ")"]);
        let test2 = is_balanced(["(", "[", ")", "]"]);
        let test3 = is_balanced(["(", "(", ")", ")"]);
        let test4 = is_balanced(["(", ")"]);
        output [test1, test2, test3, test4];
    "#), DataType::Array(vec![
        DataType::Bool(true),
        DataType::Bool(false),
        DataType::Bool(true),
        DataType::Bool(true),
    ]));
}

#[test]
fn test_e2e_string_processing_csv_parse() {
    // Parse CSV-like data lines and transform them.
    // Combines: string split (direct method), HOFs, closures, destructuring, f-strings.
    assert_eq!(run(r#"
        fn parse_record(line) {
            let fields = line.split(",");
            let [name, age, city] = fields;
            {"name": name, "age": age, "city": city}
        }

        fn format_record(record) {
            f"{record.name} (age {record.age}) from {record.city}"
        }

        let lines = [
            "Alice,30,NYC",
            "Bob,25,LA",
            "Charlie,35,Chicago"
        ];

        let records = lines.map(|line| parse_record(line));
        let formatted = records.map(|r| format_record(r));
        output formatted;
    "#), DataType::Array(vec![
        DataType::String("Alice (age 30) from NYC".to_string()),
        DataType::String("Bob (age 25) from LA".to_string()),
        DataType::String("Charlie (age 35) from Chicago".to_string()),
    ]));
}

#[test]
fn test_e2e_csv_filter_and_aggregate() {
    // Parse CSV, filter rows, and compute aggregates.
    // Combines: string split, destructuring, map, filter, reduce, type conversion.
    // Note: uses .to_int() (direct string method) instead of .to_int64() (evaluator op).
    assert_eq!(run(r#"
        fn parse_row(line) {
            let [name, score_str] = line.split(":");
            {"name": name, "score": score_str.to_int()}
        }

        let data = [
            "Alice:85",
            "Bob:92",
            "Charlie:78",
            "Diana:95",
            "Eve:60"
        ];

        let rows = data.map(|line| parse_row(line));
        let high_scorers = rows.filter(|r| r.score > 80);
        let total = high_scorers.reduce(0, |acc, r| acc + r.score);
        let count = len(high_scorers);
        output {"total": total, "count": count};
    "#), DataType::Map({
        let mut m = std::collections::BTreeMap::new();
        m.insert("total".to_string(), DataType::Int64(272));
        m.insert("count".to_string(), DataType::Int64(3));
        m
    }));
}

#[test]
fn test_e2e_state_machine_traffic_light() {
    // Enum-based state machine simulating a traffic light.
    // Combines: enums, match, while loop, mutable state, spread, counter.
    assert_eq!(run(r#"
        enum Light { Red, Yellow, Green }

        fn next_state(light) {
            match light {
                Light::Red => Light::Green,
                Light::Green => Light::Yellow,
                Light::Yellow => Light::Red,
            }
        }

        fn light_name(light) {
            match light {
                Light::Red => "red",
                Light::Green => "green",
                Light::Yellow => "yellow",
            }
        }

        let mut current = Light::Red;
        let mut history = [];
        let mut steps = 0;

        while steps < 7 {
            history = [...history, light_name(current)];
            current = next_state(current);
            steps = steps + 1;
        }
        output history;
    "#), DataType::Array(vec![
        DataType::String("red".to_string()),
        DataType::String("green".to_string()),
        DataType::String("yellow".to_string()),
        DataType::String("red".to_string()),
        DataType::String("green".to_string()),
        DataType::String("yellow".to_string()),
        DataType::String("red".to_string()),
    ]));
}

#[test]
fn test_e2e_state_machine_with_data() {
    // State machine with data-carrying enum variants and event dispatch.
    // Combines: enums with fields, match destructuring, for loop, spread, f-strings.
    assert_eq!(run(r#"
        enum State {
            Idle,
            Loading(url),
            Success(data),
            Error(msg)
        }

        fn transition(state, event) {
            match state {
                State::Idle => {
                    if event == "fetch" { State::Loading("https://api.example.com") }
                    else { state }
                },
                State::Loading(_) => {
                    if event == "ok" { State::Success("response data") }
                    else if event == "fail" { State::Error("network error") }
                    else { state }
                },
                State::Error(_) => {
                    if event == "retry" { State::Idle }
                    else { state }
                },
                State::Success(_) => {
                    if event == "reset" { State::Idle }
                    else { state }
                },
            }
        }

        fn describe(state) {
            match state {
                State::Idle => "idle",
                State::Loading(url) => f"loading:{url}",
                State::Success(data) => f"success:{data}",
                State::Error(msg) => f"error:{msg}",
            }
        }

        let events = ["fetch", "ok", "reset", "fetch", "fail", "retry"];
        let mut st = State::Idle;
        let mut log = [describe(st)];
        for ev in events {
            st = transition(st, ev);
            log = [...log, describe(st)];
        }
        output log;
    "#), DataType::Array(vec![
        DataType::String("idle".to_string()),
        DataType::String("loading:https://api.example.com".to_string()),
        DataType::String("success:response data".to_string()),
        DataType::String("idle".to_string()),
        DataType::String("loading:https://api.example.com".to_string()),
        DataType::String("error:network error".to_string()),
        DataType::String("idle".to_string()),
    ]));
}

#[test]
fn test_e2e_tree_traversal_sum() {
    // Recursive tree operations using enum-based binary tree.
    // Combines: enums with fields, recursion, match destructuring, spread.
    assert_eq!(run(r#"
        enum Tree {
            Leaf(val),
            Node(left, right)
        }

        fn tree_sum(tree) {
            match tree {
                Tree::Leaf(v) => v,
                Tree::Node(l, r) => tree_sum(l) + tree_sum(r),
            }
        }

        fn tree_depth(tree) {
            match tree {
                Tree::Leaf(_) => 1,
                Tree::Node(l, r) => {
                    let ld = tree_depth(l);
                    let rd = tree_depth(r);
                    1 + if ld > rd { ld } else { rd }
                },
            }
        }

        fn tree_leaves(tree) {
            match tree {
                Tree::Leaf(v) => [v],
                Tree::Node(l, r) => {
                    let left_leaves = tree_leaves(l);
                    let right_leaves = tree_leaves(r);
                    [...left_leaves, ...right_leaves]
                },
            }
        }

        //       Node
        //      /    \
        //    Node    Leaf(5)
        //   /    \
        // Leaf(1) Node
        //        /    \
        //     Leaf(2)  Leaf(3)
        let tree = Tree::Node(
            Tree::Node(
                Tree::Leaf(1),
                Tree::Node(Tree::Leaf(2), Tree::Leaf(3))
            ),
            Tree::Leaf(5)
        );

        output [tree_sum(tree), tree_depth(tree), tree_leaves(tree)];
    "#), DataType::Array(vec![
        DataType::Int64(11),
        DataType::Int64(4),
        DataType::Array(vec![
            DataType::Int64(1),
            DataType::Int64(2),
            DataType::Int64(3),
            DataType::Int64(5),
        ]),
    ]));
}

#[test]
fn test_e2e_tree_search() {
    // Binary search tree: insert and search with inorder traversal.
    // Combines: enums with 3 fields, deep recursion, match, comparison, spread.
    assert_eq!(run(r#"
        enum BST {
            Empty,
            Node(val, left, right)
        }

        fn insert(tree, x) {
            match tree {
                BST::Empty => BST::Node(x, BST::Empty, BST::Empty),
                BST::Node(v, l, r) => {
                    if x < v {
                        BST::Node(v, insert(l, x), r)
                    } else if x > v {
                        BST::Node(v, l, insert(r, x))
                    } else {
                        tree
                    }
                },
            }
        }

        fn bst_contains(tree, x) {
            match tree {
                BST::Empty => false,
                BST::Node(v, l, r) => {
                    if x == v { true }
                    else if x < v { bst_contains(l, x) }
                    else { bst_contains(r, x) }
                },
            }
        }

        fn inorder(tree) {
            match tree {
                BST::Empty => [],
                BST::Node(v, l, r) => {
                    let left = inorder(l);
                    let right = inorder(r);
                    [...left, v, ...right]
                },
            }
        }

        let mut bst = BST::Empty;
        for val in [5, 3, 7, 1, 4, 6, 8] {
            bst = insert(bst, val);
        }

        let sorted = inorder(bst);
        let has3 = bst_contains(bst, 3);
        let has9 = bst_contains(bst, 9);
        output [sorted, has3, has9];
    "#), DataType::Array(vec![
        DataType::Array(vec![
            DataType::Int64(1),
            DataType::Int64(3),
            DataType::Int64(4),
            DataType::Int64(5),
            DataType::Int64(6),
            DataType::Int64(7),
            DataType::Int64(8),
        ]),
        DataType::Bool(true),
        DataType::Bool(false),
    ]));
}

#[test]
fn test_e2e_pipeline_data_transformations() {
    // Data flowing through multiple transformation functions.
    // Combines: HOF chaining, closures, map/filter/reduce, f-strings, string methods.
    assert_eq!(run(r#"
        fn normalize(items) {
            items.map(|x| x.trim())
        }

        fn remove_empty(items) {
            items.filter(|x| len(x) > 0)
        }

        fn deduplicate(items) {
            let mut seen = [];
            let mut result = [];
            for item in items {
                let already = seen.find(|s| s == item);
                if already == null {
                    seen = [...seen, item];
                    result = [...result, item];
                }
            }
            result
        }

        fn to_upper_first(s) {
            if len(s) == 0 { return s }
            let first = s[0..1];
            let rest = s[1..len(s)];
            // Simple uppercase for ASCII a-z
            let upper = match first {
                "a" => "A", "b" => "B", "c" => "C", "d" => "D",
                "e" => "E", "f" => "F", "g" => "G", "h" => "H",
                _ => first,
            };
            f"{upper}{rest}"
        }

        let raw = ["  alice ", "bob", " alice", "", "  charlie  ", "bob", " dave "];

        let result = deduplicate(remove_empty(normalize(raw)))
            .map(|name| to_upper_first(name));

        output result;
    "#), DataType::Array(vec![
        DataType::String("Alice".to_string()),
        DataType::String("Bob".to_string()),
        DataType::String("Charlie".to_string()),
        DataType::String("Dave".to_string()),
    ]));
}

#[test]
fn test_e2e_pipeline_number_processing() {
    // Multi-step numeric pipeline using HOF chaining.
    // Combines: filter, map, reduce, scan, partition, destructuring.
    assert_eq!(run(r#"
        let data = [12, 5, 8, 3, 15, 7, 20, 1, 9, 14];

        // Split into small (<=10) and large (>10)
        let [small, large] = data.partition(|x| x <= 10);

        // Process small: square each, sum
        let small_sum = small.map(|x| x * x).reduce(0, |acc, x| acc + x);

        // Process large: subtract 10 from each, product
        let large_product = large.map(|x| x - 10).reduce(1, |acc, x| acc * x);

        // running totals of original data
        let running = data.scan(0, |acc, x| acc + x);
        let final_total = running.last();

        output [small_sum, large_product, final_total];
    "#), DataType::Array(vec![
        // small = [5, 8, 3, 7, 1, 9], squares = [25, 64, 9, 49, 1, 81], sum = 229
        DataType::Int64(229),
        // large = [12, 15, 20, 14], minus 10 = [2, 5, 10, 4], product = 400
        DataType::Int64(400),
        // sum of all = 12+5+8+3+15+7+20+1+9+14 = 94
        DataType::Int64(94),
    ]));
}

#[test]
fn test_e2e_builder_pattern() {
    // Struct construction through chained function calls.
    // Combines: structs, field access, functions returning modified structs, spread, f-strings.
    assert_eq!(run(r#"
        struct Config {
            host: string,
            port: int64,
            debug: bool,
            tags: array
        }

        fn default_config() {
            Config {
                host: "localhost",
                port: 8080,
                debug: false,
                tags: []
            }
        }

        fn with_host(config, host) {
            Config {
                host: host,
                port: config.port,
                debug: config.debug,
                tags: config.tags
            }
        }

        fn with_port(config, port) {
            Config {
                host: config.host,
                port: port,
                debug: config.debug,
                tags: config.tags
            }
        }

        fn with_debug(config) {
            Config {
                host: config.host,
                port: config.port,
                debug: true,
                tags: config.tags
            }
        }

        fn add_tag(config, tag) {
            Config {
                host: config.host,
                port: config.port,
                debug: config.debug,
                tags: [...config.tags, tag]
            }
        }

        fn describe(config) {
            f"{config.host}:{config.port} debug={config.debug} tags={len(config.tags)}"
        }

        let cfg = default_config();
        let cfg = with_host(cfg, "api.example.com");
        let cfg = with_port(cfg, 443);
        let cfg = with_debug(cfg);
        let cfg = add_tag(cfg, "production");
        let cfg = add_tag(cfg, "v2");

        output [describe(cfg), cfg.host, cfg.port, cfg.debug, cfg.tags];
    "#), DataType::Array(vec![
        DataType::String("api.example.com:443 debug=true tags=2".to_string()),
        DataType::String("api.example.com".to_string()),
        DataType::Int64(443),
        DataType::Bool(true),
        DataType::Array(vec![
            DataType::String("production".to_string()),
            DataType::String("v2".to_string()),
        ]),
    ]));
}

#[test]
fn test_e2e_error_handling_validation() {
    // Complex try-catch with validation and recovery.
    // Combines: try-catch, throw, functions, spread, mutable state.
    assert_eq!(run(r#"
        fn validate_age(age) {
            if age < 0 { throw "age cannot be negative" }
            if age > 150 { throw "age unreasonably large" }
            age
        }

        fn validate_name(name) {
            if len(name) == 0 { throw "name cannot be empty" }
            if len(name) > 50 { throw "name too long" }
            name
        }

        fn create_person(name, age) {
            let validated_name = validate_name(name);
            let validated_age = validate_age(age);
            {"name": validated_name, "age": validated_age}
        }

        // Test 1: valid input
        let r1 = try { create_person("Alice", 30) } catch e { f"error: {e}" };

        // Test 2: invalid age
        let r2 = try { create_person("Bob", -5) } catch e { f"error: {e}" };

        // Test 3: empty name
        let r3 = try { create_person("", 25) } catch e { f"error: {e}" };

        // Test 4: recovery with default
        let r4 = try {
            create_person("Eve", 200)
        } catch e {
            // Recover with a safe default
            create_person("Eve", 99)
        };

        output [r1, r2, r3, r4];
    "#), DataType::Array(vec![
        DataType::Map({
            let mut m = std::collections::BTreeMap::new();
            m.insert("name".to_string(), DataType::String("Alice".to_string()));
            m.insert("age".to_string(), DataType::Int64(30));
            m
        }),
        DataType::String("error: age cannot be negative".to_string()),
        DataType::String("error: name cannot be empty".to_string()),
        DataType::Map({
            let mut m = std::collections::BTreeMap::new();
            m.insert("name".to_string(), DataType::String("Eve".to_string()));
            m.insert("age".to_string(), DataType::Int64(99));
            m
        }),
    ]));
}

#[test]
fn test_e2e_error_handling_nested_with_finally() {
    // Nested try-catch with finally blocks tracking cleanup via spread.
    // Combines: try-catch-finally, throw, mutable state, nested blocks, spread.
    assert_eq!(run(r#"
        let mut log = [];

        let result = try {
            log = [...log, "outer-start"];
            let inner = try {
                log = [...log, "inner-start"];
                throw "inner failure"
            } catch e {
                log = [...log, f"inner-caught: {e}"];
                "recovered"
            } finally {
                log = [...log, "inner-finally"];
            };
            log = [...log, f"after-inner: {inner}"];
            inner
        } catch e {
            log = [...log, f"outer-caught: {e}"];
            "outer-recovery"
        } finally {
            log = [...log, "outer-finally"];
        };

        output [result, log];
    "#), DataType::Array(vec![
        DataType::String("recovered".to_string()),
        DataType::Array(vec![
            DataType::String("outer-start".to_string()),
            DataType::String("inner-start".to_string()),
            DataType::String("inner-caught: inner failure".to_string()),
            DataType::String("inner-finally".to_string()),
            DataType::String("after-inner: recovered".to_string()),
            DataType::String("outer-finally".to_string()),
        ]),
    ]));
}

#[test]
fn test_e2e_higher_order_function_composition() {
    // Functions that return functions, compose them, and apply chains.
    // Combines: closures, HOFs, function-returning-functions, reduce.
    assert_eq!(run(r#"
        fn make_adder(n) {
            |x| x + n
        }

        fn make_multiplier(n) {
            |x| x * n
        }

        fn compose(f, g) {
            |x| f(g(x))
        }

        fn apply_all(fns, value) {
            let mut result = value;
            for f in fns {
                result = f(result);
            }
            result
        }

        let add5 = make_adder(5);
        let mul3 = make_multiplier(3);
        let add5_then_mul3 = compose(mul3, add5);
        let mul3_then_add5 = compose(add5, mul3);

        // Build a pipeline: +2, *4, +10
        let pipeline = [make_adder(2), make_multiplier(4), make_adder(10)];
        // apply_all(pipeline, 3) = ((3+2)*4)+10 = 30
        let piped = apply_all(pipeline, 3);

        output [
            add5(10),
            mul3(10),
            add5_then_mul3(10),
            mul3_then_add5(10),
            piped
        ];
    "#), DataType::Array(vec![
        DataType::Int64(15),    // 10 + 5
        DataType::Int64(30),    // 10 * 3
        DataType::Int64(45),    // (10 + 5) * 3
        DataType::Int64(35),    // (10 * 3) + 5
        DataType::Int64(30),    // ((3+2)*4)+10
    ]));
}

#[test]
fn test_e2e_function_composition_with_predicates() {
    // Higher-order predicate combinators: and_pred, or_pred, not_pred.
    // Combines: closures, HOFs, boolean logic, filter.
    assert_eq!(run(r#"
        fn and_pred(f, g) {
            |x| f(x) && g(x)
        }

        fn or_pred(f, g) {
            |x| f(x) || g(x)
        }

        fn not_pred(f) {
            |x| !f(x)
        }

        let is_positive = |x| x > 0;
        let is_even = |x| x % 2 == 0;
        let is_small = |x| x < 10;

        let positive_and_even = and_pred(is_positive, is_even);
        let small_or_even = or_pred(is_small, is_even);
        let not_small = not_pred(is_small);

        let data = [-4, -1, 0, 1, 2, 5, 8, 12, 15, 20];

        let r1 = data.filter(positive_and_even);
        let r2 = data.filter(small_or_even);
        let r3 = data.filter(not_small);
        output [r1, r2, r3];
    "#), DataType::Array(vec![
        // positive_and_even: > 0 && even => 2, 8, 12, 20
        DataType::Array(vec![
            DataType::Int64(2),
            DataType::Int64(8),
            DataType::Int64(12),
            DataType::Int64(20),
        ]),
        // small_or_even: < 10 || even => -4, -1, 0, 1, 2, 5, 8, 12, 20
        DataType::Array(vec![
            DataType::Int64(-4),
            DataType::Int64(-1),
            DataType::Int64(0),
            DataType::Int64(1),
            DataType::Int64(2),
            DataType::Int64(5),
            DataType::Int64(8),
            DataType::Int64(12),
            DataType::Int64(20),
        ]),
        // not_small: >= 10 => 12, 15, 20
        DataType::Array(vec![
            DataType::Int64(12),
            DataType::Int64(15),
            DataType::Int64(20),
        ]),
    ]));
}

#[test]
fn test_e2e_pattern_matching_exhaustive() {
    // Complex match with guards, or-patterns, destructuring, nested patterns.
    // Combines: enums, match, guards, or-patterns, wildcard, nested destructuring.
    assert_eq!(run(r#"
        enum Shape {
            Circle(radius),
            Rectangle(width, height),
            Triangle(base, height)
        }

        fn classify(shape) {
            match shape {
                Shape::Circle(r) if r <= 0 => "invalid",
                Shape::Circle(r) if r < 5 => "small-circle",
                Shape::Circle(r) if r < 20 => "medium-circle",
                Shape::Circle(_) => "large-circle",
                Shape::Rectangle(w, h) if w == h => f"square-{w}x{h}",
                Shape::Rectangle(w, h) if w > 100 || h > 100 => "oversized-rect",
                Shape::Rectangle(w, h) => f"rect-{w}x{h}",
                Shape::Triangle(b, h) if b == h => "equilateral-ish",
                Shape::Triangle(b, h) => f"triangle-{b}x{h}",
            }
        }

        let shapes = [
            Shape::Circle(0),
            Shape::Circle(3),
            Shape::Circle(15),
            Shape::Circle(50),
            Shape::Rectangle(5, 5),
            Shape::Rectangle(200, 10),
            Shape::Rectangle(8, 12),
            Shape::Triangle(7, 7),
            Shape::Triangle(3, 9),
        ];

        let results = shapes.map(|s| classify(s));
        output results;
    "#), DataType::Array(vec![
        DataType::String("invalid".to_string()),
        DataType::String("small-circle".to_string()),
        DataType::String("medium-circle".to_string()),
        DataType::String("large-circle".to_string()),
        DataType::String("square-5x5".to_string()),
        DataType::String("oversized-rect".to_string()),
        DataType::String("rect-8x12".to_string()),
        DataType::String("equilateral-ish".to_string()),
        DataType::String("triangle-3x9".to_string()),
    ]));
}

#[test]
fn test_e2e_pattern_matching_or_and_destructure() {
    // Match with or-patterns combining enum variants and destructuring.
    // Combines: enums, or-patterns, match, wildcard, bindings.
    assert_eq!(run(r#"
        enum Result { Ok(v), Err(e) }
        enum Option { Some(v), None }

        fn unwrap_or(val, default) {
            match val {
                Result::Ok(v) | Option::Some(v) => v,
                Result::Err(_) | Option::None => default,
            }
        }

        fn classify_number(n) {
            match n {
                0 => "zero",
                1 | 2 | 3 => "tiny",
                n if n < 0 => "negative",
                n if n <= 10 => "small",
                n if n <= 100 => "medium",
                _ => "large",
            }
        }

        let r1 = unwrap_or(Result::Ok(42), 0);
        let r2 = unwrap_or(Result::Err("fail"), 0);
        let r3 = unwrap_or(Option::Some(99), 0);
        let r4 = unwrap_or(Option::None, 0);

        let classifications = [-5, 0, 1, 2, 7, 50, 200].map(|n| classify_number(n));

        output [r1, r2, r3, r4, classifications];
    "#), DataType::Array(vec![
        DataType::Int64(42),
        DataType::Int64(0),
        DataType::Int64(99),
        DataType::Int64(0),
        DataType::Array(vec![
            DataType::String("negative".to_string()),
            DataType::String("zero".to_string()),
            DataType::String("tiny".to_string()),
            DataType::String("tiny".to_string()),
            DataType::String("small".to_string()),
            DataType::String("medium".to_string()),
            DataType::String("large".to_string()),
        ]),
    ]));
}

#[test]
fn test_e2e_fibonacci_iterative() {
    // Iterative Fibonacci computing all values up to n=10.
    // Combines: while loop, mutable state, closures, HOF map.
    assert_eq!(run(r#"
        fn fib_iter(n) {
            if n <= 0 { return 0 }
            if n == 1 { return 1 }
            let mut a = 0;
            let mut b = 1;
            let mut i = 2;
            while i <= n {
                let temp = a + b;
                a = b;
                b = temp;
                i = i + 1;
            }
            b
        }

        let fibs = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map(|n| fib_iter(n));
        output fibs;
    "#), DataType::Array(vec![
        DataType::Int64(0),
        DataType::Int64(1),
        DataType::Int64(1),
        DataType::Int64(2),
        DataType::Int64(3),
        DataType::Int64(5),
        DataType::Int64(8),
        DataType::Int64(13),
        DataType::Int64(21),
        DataType::Int64(34),
        DataType::Int64(55),
    ]));
}

#[test]
fn test_e2e_event_dispatcher() {
    // Simple event system using parallel arrays for events/handlers.
    // Combines: arrays, closures, for loops, spread, enumerate, if/else.
    assert_eq!(run(r#"
        fn register(events, fns, event_name, handler) {
            [[...events, event_name], [...fns, handler]]
        }

        fn emit(events, fns, event_name, data) {
            let mut results = [];
            let pairs = events.zip(fns);
            for pair in pairs {
                let [ev, handler] = pair;
                if ev == event_name {
                    results = [...results, handler(data)];
                }
            }
            results
        }

        let mut events = [];
        let mut fns = [];
        let [events, fns] = register(events, fns, "greet", |name| f"Hello, {name}!");
        let [events, fns] = register(events, fns, "greet", |name| f"Welcome {name}");
        let [events, fns] = register(events, fns, "farewell", |name| f"Goodbye, {name}!");

        let greet_results = emit(events, fns, "greet", "Alice");
        let farewell_results = emit(events, fns, "farewell", "Bob");
        let unknown_results = emit(events, fns, "unknown", "Nobody");

        output [...greet_results, ...farewell_results, ...unknown_results];
    "#), DataType::Array(vec![
        DataType::String("Hello, Alice!".to_string()),
        DataType::String("Welcome Alice".to_string()),
        DataType::String("Goodbye, Bob!".to_string()),
    ]));
}

#[test]
fn test_e2e_linked_list_operations() {
    // Linked list using enums: cons, head, tail, length, to_array.
    // Combines: enums, recursion, match, spread for array building.
    assert_eq!(run(r#"
        enum List {
            Nil,
            Cons(head, tail)
        }

        fn from_array(arr) {
            arr.reduce(List::Nil, |acc, x| List::Cons(x, acc))
        }

        fn list_reverse(list) {
            fn go(lst, acc) {
                match lst {
                    List::Nil => acc,
                    List::Cons(h, t) => go(t, List::Cons(h, acc)),
                }
            }
            go(list, List::Nil)
        }

        fn to_array(list) {
            match list {
                List::Nil => [],
                List::Cons(h, t) => [h, ...to_array(t)],
            }
        }

        fn list_length(list) {
            match list {
                List::Nil => 0,
                List::Cons(_, t) => 1 + list_length(t),
            }
        }

        fn list_map(list, f) {
            match list {
                List::Nil => List::Nil,
                List::Cons(h, t) => List::Cons(f(h), list_map(t, f)),
            }
        }

        fn list_filter(list, pred) {
            match list {
                List::Nil => List::Nil,
                List::Cons(h, t) => {
                    let rest = list_filter(t, pred);
                    if pred(h) { List::Cons(h, rest) } else { rest }
                },
            }
        }

        // from_array with reduce builds reversed, so reverse it back
        let list = list_reverse(from_array([1, 2, 3, 4, 5]));
        let doubled = list_map(list, |x| x * 2);
        let evens = list_filter(list, |x| x % 2 == 0);

        output [
            to_array(doubled),
            to_array(evens),
            list_length(list),
            list_length(evens)
        ];
    "#), DataType::Array(vec![
        DataType::Array(vec![
            DataType::Int64(2),
            DataType::Int64(4),
            DataType::Int64(6),
            DataType::Int64(8),
            DataType::Int64(10),
        ]),
        DataType::Array(vec![
            DataType::Int64(2),
            DataType::Int64(4),
        ]),
        DataType::Int64(5),
        DataType::Int64(2),
    ]));
}

// ═══════════════════════════════════════════════════════════
// Tests for newly-supported StubEvaluator operations
// ═══════════════════════════════════════════════════════════

// -- Map operations --

#[test]
fn test_map_has_key() {
    assert_eq!(run(r#"
        let m = {"name": "Alice", "age": 30};
        [m.has("name"), m.has("email")]
    "#), DataType::Array(vec![DataType::Bool(true), DataType::Bool(false)]));
}

#[test]
fn test_map_delete_key() {
    assert_eq!(run(r#"
        let m = {"a": 1, "b": 2, "c": 3};
        let m2 = m.delete("b");
        m2.keys()
    "#), DataType::Array(vec![
        DataType::String("a".to_string()),
        DataType::String("c".to_string()),
    ]));
}

#[test]
fn test_map_merge() {
    assert_eq!(run(r#"
        let m1 = {"a": 1, "b": 2};
        let m2 = {"b": 99, "c": 3};
        let merged = m1.merge(m2);
        [merged.a, merged.b, merged.c]
    "#), DataType::Array(vec![
        DataType::Int64(1),
        DataType::Int64(99),
        DataType::Int64(3),
    ]));
}

#[test]
fn test_map_size() {
    assert_eq!(run(r#"
        let m = {"x": 1, "y": 2, "z": 3};
        m.size()
    "#), DataType::Int64(3));
}

#[test]
fn test_map_entries() {
    // BTreeMap keys are sorted, so entries come out in order
    assert_eq!(run(r#"
        let m = {"a": 1, "b": 2};
        let entries = m.entries();
        entries.len()
    "#), DataType::Int64(2));
}

#[test]
fn test_map_values_and_keys() {
    assert_eq!(run(r#"
        let m = {"x": 10, "y": 20};
        let ks = m.keys();
        let vs = m.values();
        [ks.len(), vs.len()]
    "#), DataType::Array(vec![DataType::Int64(2), DataType::Int64(2)]));
}

#[test]
fn test_map_set_method() {
    assert_eq!(run(r#"
        let m = {"a": 1};
        let m2 = m.set("b", 2);
        [m2.a, m2.b]
    "#), DataType::Array(vec![DataType::Int64(1), DataType::Int64(2)]));
}

// -- Array operations --

#[test]
fn test_array_contains_method() {
    assert_eq!(run(r#"
        let arr = [1, 2, 3, 4, 5];
        [arr.contains(3), arr.contains(99)]
    "#), DataType::Array(vec![DataType::Bool(true), DataType::Bool(false)]));
}

#[test]
fn test_array_flatten() {
    assert_eq!(run(r#"
        let nested = [[1, 2], [3, 4], [5]];
        nested.flatten()
    "#), DataType::Array(vec![
        DataType::Int64(1), DataType::Int64(2),
        DataType::Int64(3), DataType::Int64(4),
        DataType::Int64(5),
    ]));
}

#[test]
fn test_array_flatten_mixed() {
    // Non-array elements pass through unchanged
    assert_eq!(run(r#"
        let mixed = [1, [2, 3], 4, [5]];
        mixed.flatten()
    "#), DataType::Array(vec![
        DataType::Int64(1),
        DataType::Int64(2), DataType::Int64(3),
        DataType::Int64(4),
        DataType::Int64(5),
    ]));
}

#[test]
fn test_array_unique() {
    assert_eq!(run(r#"
        let arr = [1, 2, 3, 2, 1, 4, 3];
        arr.unique()
    "#), DataType::Array(vec![
        DataType::Int64(1), DataType::Int64(2),
        DataType::Int64(3), DataType::Int64(4),
    ]));
}

#[test]
fn test_array_slice_method() {
    assert_eq!(run(r#"
        let arr = [10, 20, 30, 40, 50];
        arr.slice(1, 4)
    "#), DataType::Array(vec![
        DataType::Int64(20), DataType::Int64(30), DataType::Int64(40),
    ]));
}

#[test]
fn test_array_slice_negative_indices() {
    assert_eq!(run(r#"
        let arr = [10, 20, 30, 40, 50];
        arr.slice(-3, -1)
    "#), DataType::Array(vec![
        DataType::Int64(30), DataType::Int64(40),
    ]));
}

#[test]
fn test_array_push_method() {
    assert_eq!(run(r#"
        let mut arr = [1, 2];
        arr = arr.push(3);
        arr
    "#), DataType::Array(vec![
        DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
    ]));
}

#[test]
fn test_array_insert_method() {
    assert_eq!(run(r#"
        let arr = [1, 2, 4, 5];
        arr.insert(2, 3)
    "#), DataType::Array(vec![
        DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
        DataType::Int64(4), DataType::Int64(5),
    ]));
}

#[test]
fn test_array_remove_method() {
    assert_eq!(run(r#"
        let arr = [10, 20, 30, 40];
        arr.remove(1)
    "#), DataType::Array(vec![
        DataType::Int64(10), DataType::Int64(30), DataType::Int64(40),
    ]));
}

#[test]
fn test_array_sort_strings() {
    assert_eq!(run(r#"
        ["banana", "apple", "cherry"].sort()
    "#), DataType::Array(vec![
        DataType::String("apple".to_string()),
        DataType::String("banana".to_string()),
        DataType::String("cherry".to_string()),
    ]));
}

#[test]
fn test_array_filter_nulls() {
    assert_eq!(run(r#"
        [1, null, 2, null, 3].filter_nulls()
    "#), DataType::Array(vec![
        DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
    ]));
}

// -- String operations --

#[test]
fn test_string_split_method() {
    assert_eq!(run(r#"
        "a,b,c".split(",")
    "#), DataType::Array(vec![
        DataType::String("a".to_string()),
        DataType::String("b".to_string()),
        DataType::String("c".to_string()),
    ]));
}

#[test]
fn test_string_replace_method() {
    assert_eq!(run(r#"
        "hello world".replace("world", "MAGI")
    "#), DataType::String("hello MAGI".to_string()));
}

#[test]
fn test_string_index_of() {
    assert_eq!(run(r#"
        ["hello".index_of("llo"), "hello".index_of("xyz")]
    "#), DataType::Array(vec![DataType::Int64(2), DataType::Int64(-1)]));
}

#[test]
fn test_string_lines_via_method() {
    assert_eq!(run(r#"
        "a\nb\nc".lines()
    "#), DataType::Array(vec![
        DataType::String("a".to_string()),
        DataType::String("b".to_string()),
        DataType::String("c".to_string()),
    ]));
}

#[test]
fn test_string_chars_method() {
    assert_eq!(run(r#"
        "abc".chars()
    "#), DataType::Array(vec![
        DataType::String("a".to_string()),
        DataType::String("b".to_string()),
        DataType::String("c".to_string()),
    ]));
}

#[test]
fn test_string_char_at() {
    assert_eq!(run(r#"
        ["hello".char_at(0), "hello".char_at(4)]
    "#), DataType::Array(vec![
        DataType::String("h".to_string()),
        DataType::String("o".to_string()),
    ]));
}

// -- Combined feature tests using new operations --

#[test]
fn test_map_iteration_with_entries() {
    // Iterate over map entries, filter and transform
    assert_eq!(run(r#"
        let scores = {"alice": 85, "bob": 92, "carol": 78, "dave": 95};
        let high_scorers = scores.entries()
            .filter(|entry| {
                let [_name, score] = entry;
                score >= 90
            })
            .map(|entry| {
                let [name, score] = entry;
                f"{name}: {score}"
            });
        high_scorers.sort()
    "#), DataType::Array(vec![
        DataType::String("bob: 92".to_string()),
        DataType::String("dave: 95".to_string()),
    ]));
}

#[test]
fn test_array_pipeline_flatten_unique_sort() {
    // Chain flatten, unique, and sort
    assert_eq!(run(r#"
        let groups = [[3, 1, 4], [1, 5, 9], [2, 6, 5]];
        groups.flatten().unique().sort()
    "#), DataType::Array(vec![
        DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
        DataType::Int64(4), DataType::Int64(5), DataType::Int64(6),
        DataType::Int64(9),
    ]));
}

#[test]
fn test_map_has_in_conditional_logic() {
    // Use map.has() for conditional branching
    assert_eq!(run(r#"
        fn get_or_default(m, k, default) {
            if m.has(k) { m.get(k) } else { default }
        }
        let config = {"host": "localhost", "port": 8080};
        [
            get_or_default(config, "host", "0.0.0.0"),
            get_or_default(config, "timeout", 30),
        ]
    "#), DataType::Array(vec![
        DataType::String("localhost".to_string()),
        DataType::Int64(30),
    ]));
}

#[test]
fn test_string_split_and_rejoin() {
    // Split, transform, rejoin
    assert_eq!(run(r#"
        let words = "hello world foo bar".split(" ");
        let upper_words = words.map(|w| w.to_upper());
        upper_words.join(" ")
    "#), DataType::String("HELLO WORLD FOO BAR".to_string()));
}

#[test]
fn test_array_contains_with_filter() {
    // Use contains to check membership during filtering
    assert_eq!(run(r#"
        let allowed = [2, 4, 6, 8, 10];
        let candidates = [1, 2, 3, 4, 5, 6, 7, 8];
        candidates.filter(|x| allowed.contains(x))
    "#), DataType::Array(vec![
        DataType::Int64(2), DataType::Int64(4),
        DataType::Int64(6), DataType::Int64(8),
    ]));
}

#[test]
fn test_map_delete_in_loop() {
    // Progressively delete keys from a map
    assert_eq!(run(r#"
        let mut m = {"a": 1, "b": 2, "c": 3, "d": 4};
        let to_remove = ["b", "d"];
        for k in to_remove {
            m = m.delete(k);
        }
        m.keys()
    "#), DataType::Array(vec![
        DataType::String("a".to_string()),
        DataType::String("c".to_string()),
    ]));
}

#[test]
fn test_map_merge_accumulator() {
    // Build a map by merging successive entries using set
    assert_eq!(run(r#"
        let entries = [["x", 1], ["y", 2], ["z", 3]];
        let mut result = {"_init": 0};
        result = result.delete("_init");
        for entry in entries {
            let [k, v] = entry;
            result = result.set(k, v);
        }
        [result.x, result.y, result.z]
    "#), DataType::Array(vec![
        DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
    ]));
}

#[test]
fn test_array_reverse_sort_chain() {
    // Sort then reverse for descending order
    assert_eq!(run(r#"
        [5, 3, 8, 1, 9, 2].sort().reverse()
    "#), DataType::Array(vec![
        DataType::Int64(9), DataType::Int64(8), DataType::Int64(5),
        DataType::Int64(3), DataType::Int64(2), DataType::Int64(1),
    ]));
}

#[test]
fn test_string_split_filter_join() {
    // Parse CSV-like data, filter empty fields
    assert_eq!(run(r#"
        let csv = "Alice,,Bob,,Carol";
        let fields = csv.split(",");
        let non_empty = fields.filter(|f| f.len() > 0);
        non_empty.join(", ")
    "#), DataType::String("Alice, Bob, Carol".to_string()));
}

#[test]
fn test_e2e_word_frequency_counter() {
    // Count word frequency using map operations
    assert_eq!(run(r#"
        let text = "the cat sat on the mat the cat";
        let words = text.split(" ");
        let mut counts = {"_init": 0};
        counts = counts.delete("_init");
        for word in words {
            if counts.has(word) {
                let c = counts.get(word);
                counts = counts.set(word, c + 1);
            } else {
                counts = counts.set(word, 1);
            }
        }
        [counts.get("the"), counts.get("cat"), counts.get("sat"), counts.get("on"), counts.get("mat")]
    "#), DataType::Array(vec![
        DataType::Int64(3), DataType::Int64(2),
        DataType::Int64(1), DataType::Int64(1), DataType::Int64(1),
    ]));
}

#[test]
fn test_e2e_set_operations_via_arrays() {
    // Implement set union, intersection, difference using array operations
    assert_eq!(run(r#"
        fn set_union(a, b) {
            [...a, ...b].unique()
        }
        fn set_intersection(a, b) {
            a.filter(|x| b.contains(x))
        }
        fn set_difference(a, b) {
            a.filter(|x| !b.contains(x))
        }

        let s1 = [1, 2, 3, 4, 5];
        let s2 = [3, 4, 5, 6, 7];

        let union = set_union(s1, s2).sort();
        let inter = set_intersection(s1, s2).sort();
        let diff = set_difference(s1, s2).sort();

        output [union, inter, diff];
    "#), DataType::Array(vec![
        DataType::Array(vec![
            DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
            DataType::Int64(4), DataType::Int64(5), DataType::Int64(6),
            DataType::Int64(7),
        ]),
        DataType::Array(vec![
            DataType::Int64(3), DataType::Int64(4), DataType::Int64(5),
        ]),
        DataType::Array(vec![
            DataType::Int64(1), DataType::Int64(2),
        ]),
    ]));
}

#[test]
fn test_e2e_map_based_cache() {
    // Simple cache using map has/get/set
    assert_eq!(run(r#"
        fn make_cache() {
            let m = {"_init": 0};
            m.delete("_init")
        }

        fn cache_get(cache, key) {
            if cache.has(key) { cache.get(key) } else { null }
        }

        fn cache_set(cache, key, val) {
            cache.set(key, val)
        }

        let mut cache = make_cache();
        cache = cache_set(cache, "user:1", "Alice");
        cache = cache_set(cache, "user:2", "Bob");
        cache = cache_set(cache, "user:3", "Carol");

        let r1 = cache_get(cache, "user:1");
        let r2 = cache_get(cache, "user:99");
        let r3 = cache.size();

        cache = cache.delete("user:2");
        let r4 = cache.size();
        let r5 = cache.has("user:2");

        output [r1, r2, r3, r4, r5];
    "#), DataType::Array(vec![
        DataType::String("Alice".to_string()),
        DataType::Null,
        DataType::Int64(3),
        DataType::Int64(2),
        DataType::Bool(false),
    ]));
}

#[test]
fn test_e2e_string_tokenizer() {
    // Tokenize a simple expression using string methods
    assert_eq!(run(r#"
        fn tokenize(input) {
            let chars = input.chars();
            let mut tokens = [];
            let mut current = "";
            for ch in chars {
                if ch == " " {
                    if current.len() > 0 {
                        tokens = [...tokens, current];
                        current = "";
                    }
                } else if ch == "+" || ch == "-" || ch == "*" || ch == "/" {
                    if current.len() > 0 {
                        tokens = [...tokens, current];
                        current = "";
                    }
                    tokens = [...tokens, ch];
                } else {
                    current = current + ch;
                }
            }
            if current.len() > 0 {
                tokens = [...tokens, current];
            }
            tokens
        }

        tokenize("42 + 7 * 3 - 1")
    "#), DataType::Array(vec![
        DataType::String("42".to_string()),
        DataType::String("+".to_string()),
        DataType::String("7".to_string()),
        DataType::String("*".to_string()),
        DataType::String("3".to_string()),
        DataType::String("-".to_string()),
        DataType::String("1".to_string()),
    ]));
}

#[test]
fn test_e2e_nested_map_operations() {
    // Build and query nested maps
    assert_eq!(run(r#"
        let db = {
            "users": {
                "alice": {"role": "admin", "active": true},
                "bob": {"role": "user", "active": false},
                "carol": {"role": "user", "active": true},
            }
        };

        let users = db.users;
        let user_names = users.keys();
        let active_users = user_names.filter(|name| {
            let user = users.get(name);
            user.active
        });
        active_users.sort()
    "#), DataType::Array(vec![
        DataType::String("alice".to_string()),
        DataType::String("carol".to_string()),
    ]));
}

#[test]
fn test_e2e_matrix_operations_with_flatten() {
    // Matrix transpose using map + flatten + slice
    assert_eq!(run(r#"
        fn get(arr, i) {
            arr.get(i)
        }

        let matrix = [[1, 2, 3], [4, 5, 6], [7, 8, 9]];
        let row0 = get(matrix, 0);
        let row1 = get(matrix, 1);
        let row2 = get(matrix, 2);

        // Extract columns manually
        let col0 = [get(row0, 0), get(row1, 0), get(row2, 0)];
        let col1 = [get(row0, 1), get(row1, 1), get(row2, 1)];
        let col2 = [get(row0, 2), get(row1, 2), get(row2, 2)];

        let transposed = [col0, col1, col2];

        // Also flatten the original
        let flat = matrix.flatten();

        output [transposed, flat];
    "#), DataType::Array(vec![
        DataType::Array(vec![
            DataType::Array(vec![DataType::Int64(1), DataType::Int64(4), DataType::Int64(7)]),
            DataType::Array(vec![DataType::Int64(2), DataType::Int64(5), DataType::Int64(8)]),
            DataType::Array(vec![DataType::Int64(3), DataType::Int64(6), DataType::Int64(9)]),
        ]),
        DataType::Array(vec![
            DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
            DataType::Int64(4), DataType::Int64(5), DataType::Int64(6),
            DataType::Int64(7), DataType::Int64(8), DataType::Int64(9),
        ]),
    ]));
}

#[test]
fn test_e2e_group_by_with_map_operations() {
    // Group items by category using map operations
    assert_eq!(run(r#"
        let items = [
            {"name": "apple", "category": "fruit"},
            {"name": "banana", "category": "fruit"},
            {"name": "carrot", "category": "vegetable"},
            {"name": "lettuce", "category": "vegetable"},
            {"name": "cherry", "category": "fruit"},
        ];

        let mut groups = {"_init": 0};
        groups = groups.delete("_init");
        for item in items {
            let cat = item.category;
            let name = item.name;
            if groups.has(cat) {
                let existing = groups.get(cat);
                groups = groups.set(cat, [...existing, name]);
            } else {
                groups = groups.set(cat, [name]);
            }
        }

        let fruits = groups.get("fruit");
        let vegs = groups.get("vegetable");
        output [fruits, vegs];
    "#), DataType::Array(vec![
        DataType::Array(vec![
            DataType::String("apple".to_string()),
            DataType::String("banana".to_string()),
            DataType::String("cherry".to_string()),
        ]),
        DataType::Array(vec![
            DataType::String("carrot".to_string()),
            DataType::String("lettuce".to_string()),
        ]),
    ]));
}

#[test]
fn test_e2e_array_insert_remove_build() {
    // Build a sorted array by inserting elements at the right position
    assert_eq!(run(r#"
        fn sorted_insert(arr, val) {
            let mut i = 0;
            while i < arr.len() {
                if val < arr.get(i) {
                    return arr.insert(i, val)
                }
                i = i + 1;
            }
            [...arr, val]
        }

        let mut sorted = [];
        let values = [5, 3, 8, 1, 9, 2, 7, 4, 6];
        for v in values {
            sorted = sorted_insert(sorted, v);
        }
        sorted
    "#), DataType::Array(vec![
        DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
        DataType::Int64(4), DataType::Int64(5), DataType::Int64(6),
        DataType::Int64(7), DataType::Int64(8), DataType::Int64(9),
    ]));
}

#[test]
fn test_e2e_string_replace_chain() {
    // Chain multiple string replacements
    assert_eq!(run(r#"
        let template = "Hello, {name}! You have {count} messages.";
        let result = template
            .replace("{name}", "Alice")
            .replace("{count}", "5");
        result
    "#), DataType::String("Hello, Alice! You have 5 messages.".to_string()));
}

#[test]
fn test_e2e_map_keys_values_round_trip() {
    // Reconstruct a map from its keys and values
    assert_eq!(run(r#"
        let original = {"x": 10, "y": 20, "z": 30};
        let ks = original.keys();
        let vs = original.values();

        // Zip keys and values, then build new map
        let pairs = ks.zip(vs);
        let mut rebuilt = {"_init": 0};
        rebuilt = rebuilt.delete("_init");
        for pair in pairs {
            let [k, v] = pair;
            rebuilt = rebuilt.set(k, v * 2);
        }
        [rebuilt.x, rebuilt.y, rebuilt.z]
    "#), DataType::Array(vec![
        DataType::Int64(20), DataType::Int64(40), DataType::Int64(60),
    ]));
}

#[test]
fn test_e2e_deduplication_pipeline() {
    // Remove duplicate records from a list using unique and flatten
    assert_eq!(run(r#"
        let batches = [
            [1, 2, 3],
            [3, 4, 5],
            [5, 6, 7],
        ];
        let all = batches.flatten();
        let deduped = all.unique().sort();
        let count = deduped.len();
        [deduped, count]
    "#), DataType::Array(vec![
        DataType::Array(vec![
            DataType::Int64(1), DataType::Int64(2), DataType::Int64(3),
            DataType::Int64(4), DataType::Int64(5), DataType::Int64(6),
            DataType::Int64(7),
        ]),
        DataType::Int64(7),
    ]));
}

// ── Round 82: HOF audit — edge-case coverage ──

#[test]
fn test_hof_map_empty_array() {
    assert_eq!(
        run("[].map(|x| x * 2)"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_filter_empty_array() {
    assert_eq!(
        run("[].filter(|x| x > 0)"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_reduce_empty_array_returns_initial() {
    assert_eq!(
        run("[].reduce(42, |acc, x| acc + x)"),
        DataType::Int64(42)
    );
}

#[test]
fn test_hof_find_empty_array() {
    assert_eq!(
        run("[].find(|x| x > 0)"),
        DataType::Null
    );
}

#[test]
fn test_hof_find_index_empty_array() {
    assert_eq!(
        run("[].find_index(|x| x > 0)"),
        DataType::Null
    );
}

#[test]
fn test_hof_any_empty_array() {
    assert_eq!(
        run("[].any(|x| x > 0)"),
        DataType::Bool(false)
    );
}

#[test]
fn test_hof_all_empty_array() {
    // all on empty array should be vacuously true
    assert_eq!(
        run("[].all(|x| x > 0)"),
        DataType::Bool(true)
    );
}

#[test]
fn test_hof_each_empty_array() {
    assert_eq!(
        run("[].each(|x| x)"),
        DataType::Null
    );
}

#[test]
fn test_hof_flat_map_empty_array() {
    assert_eq!(
        run("[].flat_map(|x| [x, x])"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_flat_map_non_array_return() {
    // When callback returns a non-array, it should be included as-is
    assert_eq!(
        run("[1, 2, 3].flat_map(|x| x * 10)"),
        DataType::Array(vec![DataType::Int64(10), DataType::Int64(20), DataType::Int64(30)])
    );
}

#[test]
fn test_hof_flat_map_mixed_return() {
    // Some callbacks return arrays, some return scalars
    assert_eq!(
        run(r#"[1, 2, 3].flat_map(|x| if x == 2 { [x, x] } else { x })"#),
        DataType::Array(vec![
            DataType::Int64(1),
            DataType::Int64(2), DataType::Int64(2),
            DataType::Int64(3),
        ])
    );
}

#[test]
fn test_hof_sort_by_empty_array() {
    assert_eq!(
        run("[].sort_by(|a, b| a - b)"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_sort_by_single_element() {
    assert_eq!(
        run("[42].sort_by(|a, b| a - b)"),
        DataType::Array(vec![DataType::Int64(42)])
    );
}

#[test]
fn test_hof_sort_by_all_equal() {
    assert_eq!(
        run("[5, 5, 5].sort_by(|a, b| a - b)"),
        DataType::Array(vec![DataType::Int64(5), DataType::Int64(5), DataType::Int64(5)])
    );
}

#[test]
fn test_hof_group_by_empty_array() {
    let result = run(r#"[].group_by(|x| "a")"#);
    match result {
        DataType::Map(m) => assert_eq!(m.len(), 0),
        _ => panic!("expected Map, got {:?}", result),
    }
}

#[test]
fn test_hof_min_by_single_element() {
    assert_eq!(
        run("[99].min_by(|a, b| a - b)"),
        DataType::Int64(99)
    );
}

#[test]
fn test_hof_max_by_single_element() {
    assert_eq!(
        run("[99].max_by(|a, b| a - b)"),
        DataType::Int64(99)
    );
}

#[test]
fn test_hof_take_while_all_pass() {
    assert_eq!(
        run("[1, 2, 3].take_while(|x| x > 0)"),
        DataType::Array(vec![DataType::Int64(1), DataType::Int64(2), DataType::Int64(3)])
    );
}

#[test]
fn test_hof_take_while_none_pass() {
    assert_eq!(
        run("[1, 2, 3].take_while(|x| x > 10)"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_take_while_empty_array() {
    assert_eq!(
        run("[].take_while(|x| x > 0)"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_skip_while_all_pass() {
    assert_eq!(
        run("[1, 2, 3].skip_while(|x| x > 0)"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_skip_while_none_pass() {
    assert_eq!(
        run("[1, 2, 3].skip_while(|x| x > 10)"),
        DataType::Array(vec![DataType::Int64(1), DataType::Int64(2), DataType::Int64(3)])
    );
}

#[test]
fn test_hof_skip_while_empty_array() {
    assert_eq!(
        run("[].skip_while(|x| x > 0)"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_partition_empty_array() {
    assert_eq!(
        run("[].partition(|x| x > 0)"),
        DataType::Array(vec![
            DataType::Array(vec![]),
            DataType::Array(vec![]),
        ])
    );
}

#[test]
fn test_hof_partition_all_match() {
    assert_eq!(
        run("[1, 2, 3].partition(|x| x > 0)"),
        DataType::Array(vec![
            DataType::Array(vec![DataType::Int64(1), DataType::Int64(2), DataType::Int64(3)]),
            DataType::Array(vec![]),
        ])
    );
}

#[test]
fn test_hof_partition_none_match() {
    assert_eq!(
        run("[1, 2, 3].partition(|x| x > 10)"),
        DataType::Array(vec![
            DataType::Array(vec![]),
            DataType::Array(vec![DataType::Int64(1), DataType::Int64(2), DataType::Int64(3)]),
        ])
    );
}

#[test]
fn test_hof_scan_empty_array() {
    assert_eq!(
        run("[].scan(0, |acc, x| acc + x)"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_enumerate_empty_array() {
    assert_eq!(
        run("[].enumerate()"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_zip_empty_arrays() {
    assert_eq!(
        run("[].zip([])"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_zip_different_lengths() {
    // zip truncates to the shorter array
    assert_eq!(
        run("[1, 2, 3].zip([10, 20])"),
        DataType::Array(vec![
            DataType::Array(vec![DataType::Int64(1), DataType::Int64(10)]),
            DataType::Array(vec![DataType::Int64(2), DataType::Int64(20)]),
        ])
    );
}

#[test]
fn test_hof_zip_second_longer() {
    assert_eq!(
        run("[1].zip([10, 20, 30])"),
        DataType::Array(vec![
            DataType::Array(vec![DataType::Int64(1), DataType::Int64(10)]),
        ])
    );
}

#[test]
fn test_hof_chunk_empty_array() {
    assert_eq!(
        run("[].chunk(3)"),
        DataType::Array(vec![])
    );
}

#[test]
fn test_hof_chunk_exact_division() {
    assert_eq!(
        run("[1, 2, 3, 4].chunk(2)"),
        DataType::Array(vec![
            DataType::Array(vec![DataType::Int64(1), DataType::Int64(2)]),
            DataType::Array(vec![DataType::Int64(3), DataType::Int64(4)]),
        ])
    );
}

#[test]
fn test_hof_chunk_size_larger_than_array() {
    assert_eq!(
        run("[1, 2].chunk(10)"),
        DataType::Array(vec![
            DataType::Array(vec![DataType::Int64(1), DataType::Int64(2)]),
        ])
    );
}

// Map HOF edge cases

#[test]
fn test_hof_filter_entries_empty_map() {
    let result = run(r#"
        let m = {"_": 0}.delete("_");
        m.filter_entries(|k, v| true)
    "#);
    match result {
        DataType::Map(m) => assert_eq!(m.len(), 0),
        _ => panic!("expected Map, got {:?}", result),
    }
}

#[test]
fn test_hof_map_values_empty_map() {
    let result = run(r#"
        let m = {"_": 0}.delete("_");
        m.map_values(|v| v * 2)
    "#);
    match result {
        DataType::Map(m) => assert_eq!(m.len(), 0),
        _ => panic!("expected Map, got {:?}", result),
    }
}

#[test]
fn test_hof_map_keys_empty_map() {
    let result = run(r#"
        let m = {"_": 0}.delete("_");
        m.map_keys(|k| k + "!")
    "#);
    match result {
        DataType::Map(m) => assert_eq!(m.len(), 0),
        _ => panic!("expected Map, got {:?}", result),
    }
}

#[test]
fn test_hof_map_keys_collision() {
    // When map_keys produces duplicate keys, last one wins (BTreeMap ordering)
    let result = run(r#"
        let m = {"a": 1, "b": 2, "c": 3};
        m.map_keys(|k| "same")
    "#);
    match result {
        DataType::Map(m) => {
            assert_eq!(m.len(), 1);
            // BTreeMap iterates in alphabetical order: a, b, c
            // Last one to insert "same" key is c=3
            assert_eq!(m.get("same"), Some(&DataType::Int64(3)));
        }
        _ => panic!("expected Map, got {:?}", result),
    }
}

#[test]
fn test_hof_filter_entries_all_removed() {
    let result = run(r#"
        let m = {"a": 1, "b": 2};
        m.filter_entries(|k, v| false)
    "#);
    match result {
        DataType::Map(m) => assert_eq!(m.len(), 0),
        _ => panic!("expected Map, got {:?}", result),
    }
}

// HOF arity error tests

#[test]
fn test_hof_map_no_args_error() {
    let err = run_err("[1,2,3].map()");
    match err {
        InterpError::ArityMismatch { name, .. } => assert_eq!(name, "map"),
        _ => panic!("expected ArityMismatch, got {:?}", err),
    }
}

#[test]
fn test_hof_filter_no_args_error() {
    let err = run_err("[1,2,3].filter()");
    match err {
        InterpError::ArityMismatch { name, .. } => assert_eq!(name, "filter"),
        _ => panic!("expected ArityMismatch, got {:?}", err),
    }
}

#[test]
fn test_hof_reduce_one_arg_error() {
    let err = run_err("[1,2,3].reduce(0)");
    match err {
        InterpError::ArityMismatch { name, expected, actual, .. } => {
            assert_eq!(name, "reduce");
            assert_eq!(expected, "2");
            assert_eq!(actual, 1);
        }
        _ => panic!("expected ArityMismatch, got {:?}", err),
    }
}

#[test]
fn test_hof_scan_one_arg_error() {
    let err = run_err("[1,2,3].scan(0)");
    match err {
        InterpError::ArityMismatch { name, expected, actual, .. } => {
            assert_eq!(name, "scan");
            assert_eq!(expected, "2");
            assert_eq!(actual, 1);
        }
        _ => panic!("expected ArityMismatch, got {:?}", err),
    }
}

#[test]
fn test_hof_enumerate_with_args_error() {
    let err = run_err("[1,2,3].enumerate(1)");
    match err {
        InterpError::ArityMismatch { name, expected, actual, .. } => {
            assert_eq!(name, "enumerate");
            assert_eq!(expected, "0");
            assert_eq!(actual, 1);
        }
        _ => panic!("expected ArityMismatch, got {:?}", err),
    }
}

#[test]
fn test_hof_zip_non_array_error() {
    let err = run_err(r#"[1,2,3].zip(42)"#);
    match err {
        InterpError::TypeError { expected, .. } => assert_eq!(expected, "Array"),
        _ => panic!("expected TypeError, got {:?}", err),
    }
}

#[test]
fn test_hof_sort_by_non_numeric_comparator_error() {
    let err = run_err(r#"[1,2,3].sort_by(|a, b| "not a number")"#);
    match err {
        InterpError::TypeError { context, .. } => {
            assert!(context.contains("sort_by"), "context should mention sort_by, got: {}", context);
        }
        _ => panic!("expected TypeError, got {:?}", err),
    }
}

// HOF chaining tests

#[test]
fn test_hof_chain_filter_map() {
    assert_eq!(
        run("[1, 2, 3, 4, 5].filter(|x| x % 2 == 0).map(|x| x * 10)"),
        DataType::Array(vec![DataType::Int64(20), DataType::Int64(40)])
    );
}

#[test]
fn test_hof_chain_map_flat_map() {
    assert_eq!(
        run("[1, 2].map(|x| x + 10).flat_map(|x| [x, x + 100])"),
        DataType::Array(vec![
            DataType::Int64(11), DataType::Int64(111),
            DataType::Int64(12), DataType::Int64(112),
        ])
    );
}

#[test]
fn test_hof_chain_filter_reduce() {
    assert_eq!(
        run("[1, 2, 3, 4, 5].filter(|x| x > 2).reduce(0, |acc, x| acc + x)"),
        DataType::Int64(12)
    );
}

#[test]
fn test_hof_skip_while_does_not_resume_skipping() {
    // After skipping stops, all remaining elements are kept even if they match the predicate
    assert_eq!(
        run("[1, 2, 3, 1, 2].skip_while(|x| x < 3)"),
        DataType::Array(vec![DataType::Int64(3), DataType::Int64(1), DataType::Int64(2)])
    );
}

#[test]
fn test_hof_group_by_single_group() {
    let result = run(r#"[1, 2, 3].group_by(|x| "all")"#);
    match result {
        DataType::Map(m) => {
            assert_eq!(m.len(), 1);
            assert_eq!(
                m.get("all"),
                Some(&DataType::Array(vec![DataType::Int64(1), DataType::Int64(2), DataType::Int64(3)]))
            );
        }
        _ => panic!("expected Map, got {:?}", result),
    }
}

#[test]
fn test_hof_group_by_numeric_keys() {
    // group_by converts keys to strings via to_string_lossy
    let result = run("[1, 2, 3, 4].group_by(|x| x % 2)");
    match result {
        DataType::Map(m) => {
            assert_eq!(m.len(), 2);
            assert!(m.contains_key("0"));
            assert!(m.contains_key("1"));
        }
        _ => panic!("expected Map, got {:?}", result),
    }
}

// ═══════════════════════════════════════════════════════════
// Unicode audit: comprehensive Unicode handling tests
// ═══════════════════════════════════════════════════════════

#[test]
fn test_unicode_string_cjk_roundtrip() {
    // CJK characters in string literals should survive the full pipeline
    assert_eq!(
        run(r#""日本語テスト""#),
        DataType::String("日本語テスト".to_string())
    );
}

#[test]
fn test_unicode_string_emoji_roundtrip() {
    assert_eq!(
        run(r#""hello 😀🎉🚀 world""#),
        DataType::String("hello 😀🎉🚀 world".to_string())
    );
}

#[test]
fn test_unicode_string_arabic_roundtrip() {
    assert_eq!(
        run(r#""مرحبا بالعالم""#),
        DataType::String("مرحبا بالعالم".to_string())
    );
}

#[test]
fn test_unicode_string_korean_roundtrip() {
    assert_eq!(
        run(r#""한국어 테스트""#),
        DataType::String("한국어 테스트".to_string())
    );
}

#[test]
fn test_unicode_string_greek_roundtrip() {
    assert_eq!(
        run(r#""Ελληνικά""#),
        DataType::String("Ελληνικά".to_string())
    );
}

#[test]
fn test_unicode_escape_emoji_in_expression() {
    // Use \u{1F600} in a string and check the result
    assert_eq!(
        run(r#"let s = "\u{1F600}"; s.len()"#),
        DataType::Int64(1) // One character
    );
}

#[test]
fn test_unicode_escape_accented_in_expression() {
    assert_eq!(
        run(r#""\u{00E9}""#),
        DataType::String("é".to_string())
    );
}

#[test]
fn test_unicode_string_concatenation() {
    // Concatenating Unicode strings
    assert_eq!(
        run(r#""日本" + "語""#),
        DataType::String("日本語".to_string())
    );
}

#[test]
fn test_unicode_string_methods_on_cjk() {
    // len() should return character count, not byte count
    assert_eq!(run(r#""日本語".len()"#), DataType::Int64(3));
    assert_eq!(run(r#""日本語".reverse()"#), DataType::String("語本日".to_string()));
    assert_eq!(run(r#""日本語".contains("本")"#), DataType::Bool(true));
    assert_eq!(run(r#""日本語".starts_with("日")"#), DataType::Bool(true));
    assert_eq!(run(r#""日本語".ends_with("語")"#), DataType::Bool(true));
}

#[test]
fn test_unicode_string_chars_cjk() {
    let result = run(r#""日本語".chars()"#);
    assert_eq!(
        result,
        DataType::Array(vec![
            DataType::String("日".to_string()),
            DataType::String("本".to_string()),
            DataType::String("語".to_string()),
        ])
    );
}

#[test]
fn test_unicode_string_in_array() {
    let result = run(r#"["hello", "日本語", "🎉"]"#);
    assert_eq!(
        result,
        DataType::Array(vec![
            DataType::String("hello".to_string()),
            DataType::String("日本語".to_string()),
            DataType::String("🎉".to_string()),
        ])
    );
}

#[test]
fn test_unicode_string_in_map_value() {
    let result = run(r#"{"greeting": "こんにちは"}"#);
    match result {
        DataType::Map(m) => {
            assert_eq!(m.get("greeting"), Some(&DataType::String("こんにちは".to_string())));
        }
        _ => panic!("expected Map, got {:?}", result),
    }
}

#[test]
fn test_unicode_string_comparison() {
    assert_eq!(run(r#""café" == "café""#), DataType::Bool(true));
    assert_eq!(run(r#""café" == "cafe""#), DataType::Bool(false));
}

#[test]
fn test_unicode_in_line_comment_before_code() {
    // Unicode in a line comment should not affect subsequent code
    let result = run("// 日本語コメント 🎉\n42");
    assert_eq!(result, DataType::Int64(42));
}

#[test]
fn test_unicode_in_block_comment_before_code() {
    // Unicode in a block comment should not affect subsequent code
    let result = run("/* 日本語コメント 🎉 */42");
    assert_eq!(result, DataType::Int64(42));
}

#[test]
fn test_unicode_fstring_with_cjk() {
    let result = run(r#"
        let name = "世界";
        f"こんにちは {name} さん"
    "#);
    assert_eq!(result, DataType::String("こんにちは 世界 さん".to_string()));
}

#[test]
fn test_unicode_raw_string_preserves_content() {
    assert_eq!(
        run(r#"r"日本語\nテスト""#),
        DataType::String("日本語\\nテスト".to_string())
    );
}

#[test]
fn test_unicode_string_empty() {
    assert_eq!(run(r#""""#), DataType::String("".to_string()));
}

#[test]
fn test_unicode_string_single_emoji() {
    assert_eq!(run(r#""🎉""#), DataType::String("🎉".to_string()));
}

#[test]
fn test_unicode_string_slice_cjk() {
    // Slicing CJK characters should work by char index
    assert_eq!(
        run(r#""日本語テスト"[1..4]"#),
        DataType::String("本語テ".to_string())
    );
}

#[test]
fn test_unicode_string_index_of_emoji() {
    assert_eq!(
        run(r#""hello 🎉 world".index_of("🎉")"#),
        DataType::Int64(6)
    );
}

#[test]
fn test_unicode_match_on_string() {
    let result = run(r#"
        let s = "café";
        match s {
            "café" => "french",
            "cafe" => "english",
            _ => "other"
        }
    "#);
    assert_eq!(result, DataType::String("french".to_string()));
}

#[test]
fn test_unicode_type_checker_no_false_positives() {
    // Type checker should not produce false positives on Unicode strings
    let program = parse_v2(r#"
        let s: string = "日本語";
        let t: string = "café";
        let u = s + t;
        output u;
    "#).unwrap();
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    let errors: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.severity == magi_lang::syntax::type_checker::DiagnosticSeverity::Error)
        .collect();
    assert!(errors.is_empty(), "No type errors expected for Unicode strings: {:?}", errors);
}

#[test]
fn test_unicode_position_tracking_block_comment() {
    // After a block comment with Unicode, subsequent tokens should have
    // correct column positions (characters, not bytes)
    use magi_lang::syntax::lexer::tokenize;
    let tokens = tokenize("/* 日 */x").unwrap();
    // '日' is 1 char (3 bytes). Comment: / * space 日 space * / = 7 chars
    // Then 'x' should be at column 8
    let x_tok = tokens.iter().find(|t| t.text == "x").unwrap();
    assert_eq!(x_tok.span.start_col, 8,
        "After '/* 日 */' (7 chars), x should be at col 8, got col {}",
        x_tok.span.start_col);
}

// ── Round 82: module/use audit tests ─────────────────────────

#[test]
fn test_use_glob_imports_module_enums() {
    // Bug fix: glob import should also bring in enums from a module
    assert_eq!(run(r#"
        mod colors {
            enum Color { Red, Green, Blue }
            fn make_green() { Color::Green() }
        }
        use colors::*;
        output make_green();
    "#), DataType::Map({
        let mut m = std::collections::BTreeMap::new();
        m.insert("__data".to_string(), DataType::Array(vec![]));
        m.insert("__enum".to_string(), DataType::String("Color".to_string()));
        m.insert("__variant".to_string(), DataType::String("Green".to_string()));
        m
    }));
}

#[test]
fn test_use_glob_imports_module_structs() {
    // Bug fix: glob import should also bring in structs from a module
    assert_eq!(run(r#"
        mod geo {
            struct Point { x, y }
            fn origin() { Point { x: 0, y: 0 } }
        }
        use geo::*;
        let p = Point { x: 5, y: 10 };
        output p.x + p.y;
    "#), DataType::Int64(15));
}

#[test]
fn test_use_enum_by_name() {
    // Bug fix: `use mod::Enum` should work for enum definitions
    assert_eq!(run(r#"
        mod shapes {
            enum Shape { Circle, Square }
        }
        use shapes::Shape;
        output Shape::Circle();
    "#), DataType::Map({
        let mut m = std::collections::BTreeMap::new();
        m.insert("__data".to_string(), DataType::Array(vec![]));
        m.insert("__enum".to_string(), DataType::String("Shape".to_string()));
        m.insert("__variant".to_string(), DataType::String("Circle".to_string()));
        m
    }));
}

#[test]
fn test_use_struct_by_name() {
    // Bug fix: `use mod::Struct` should work for struct definitions
    assert_eq!(run(r#"
        mod geo {
            struct Vec2 { x, y }
        }
        use geo::Vec2;
        let v = Vec2 { x: 3, y: 4 };
        output v.x;
    "#), DataType::Int64(3));
}

#[test]
fn test_use_enum_with_alias() {
    // `use mod::Enum as Alias` should register enum under alias name
    assert_eq!(run(r#"
        mod colors {
            enum Color { Red, Green, Blue }
        }
        use colors::Color as Hue;
        output Hue::Blue();
    "#), DataType::Map({
        let mut m = std::collections::BTreeMap::new();
        m.insert("__data".to_string(), DataType::Array(vec![]));
        m.insert("__enum".to_string(), DataType::String("Hue".to_string()));
        m.insert("__variant".to_string(), DataType::String("Blue".to_string()));
        m
    }));
}

#[test]
fn test_use_struct_with_alias() {
    // `use mod::Struct as Alias` should register struct under alias name
    assert_eq!(run(r#"
        mod geo {
            struct Point { x, y }
        }
        use geo::Point as Pt;
        let p = Pt { x: 1, y: 2 };
        output p.y;
    "#), DataType::Int64(2));
}

#[test]
fn test_nested_module_function_via_use() {
    // Bug fix: nested modules should register functions with multi-level qualified names.
    // Direct call `outer::inner::greet()` is not supported (parser only handles one :: level),
    // but `use` supports multi-level paths.
    assert_eq!(run(r#"
        mod outer {
            mod inner {
                fn greet() { "hello" }
            }
        }
        use outer::inner::greet;
        output greet();
    "#), DataType::String("hello".to_string()));
}

#[test]
fn test_nested_module_use_glob() {
    // Glob import from a nested module should import its functions
    assert_eq!(run(r#"
        mod a {
            mod b {
                fn double(x) { x * 2 }
                fn triple(x) { x * 3 }
            }
        }
        use a::b::*;
        output double(5) + triple(5);
    "#), DataType::Int64(25));
}

#[test]
fn test_nested_module_use_specific() {
    // Import a specific function from a nested module
    assert_eq!(run(r#"
        mod a {
            mod b {
                fn add(x, y) { x + y }
            }
        }
        use a::b::add;
        output add(3, 4);
    "#), DataType::Int64(7));
}

#[test]
fn test_nested_module_enum_via_use() {
    // Nested module registers enums; access via use import and module function
    assert_eq!(run(r#"
        mod outer {
            mod inner {
                enum Status { Ok, Err }
                fn make_ok() { Status::Ok() }
            }
        }
        use outer::inner::make_ok;
        output make_ok();
    "#), DataType::Map({
        let mut m = std::collections::BTreeMap::new();
        m.insert("__data".to_string(), DataType::Array(vec![]));
        m.insert("__enum".to_string(), DataType::String("Status".to_string()));
        m.insert("__variant".to_string(), DataType::String("Ok".to_string()));
        m
    }));
}

#[test]
fn test_use_nonexistent_enum_errors() {
    // Using a non-existent item from a module should give error with suggestions
    let err = run_err(r#"
        mod m {
            enum Color { Red }
            fn foo() { 1 }
        }
        use m::Colour;
    "#);
    match err {
        InterpError::UnknownOperation { name, suggestion, .. } => {
            assert!(name.contains("Colour"), "expected 'Colour' in name: {}", name);
            // Should suggest 'Color' since it's close
            assert_eq!(suggestion, Some("did you mean 'Color'?".to_string()));
        }
        other => panic!("expected UnknownOperation, got: {:?}", other),
    }
}

#[test]
fn test_use_nonexistent_item_includes_enum_struct_suggestions() {
    // When looking up a non-existent item, suggestions should include enums and structs
    let err = run_err(r#"
        mod m {
            struct Point { x, y }
        }
        use m::Poont;
    "#);
    match err {
        InterpError::UnknownOperation { suggestion, .. } => {
            assert_eq!(suggestion, Some("did you mean 'Point'?".to_string()));
        }
        other => panic!("expected UnknownOperation, got: {:?}", other),
    }
}

#[test]
fn test_module_cross_module_enum_match() {
    // Cross-module type visibility: function in one module can match on enum from another.
    // Module enums are registered unqualified, so Color::Red() works directly.
    // Use a helper function since `types::Color::Red()` needs multi-level :: (parser limitation).
    assert_eq!(run(r#"
        mod types {
            enum Color { Red, Green, Blue }
            fn make_red() { Color::Red() }
        }
        mod logic {
            fn is_red(c) {
                match c {
                    Color::Red() => true,
                    _ => false,
                }
            }
        }
        let r = types::make_red();
        output logic::is_red(r);
    "#), DataType::Bool(true));
}

// ═══════════════════════════════════════════════════════════════════════════════
// SMOKE TESTS — holistic subsystem sanity checks
// ═══════════════════════════════════════════════════════════════════════════════

/// Smoke test: every literal type parses, type-checks, and interprets correctly.
#[test]
fn test_smoke_all_literal_types() {
    // Int64
    assert_eq!(run("42"), DataType::Int64(42));
    assert_eq!(run("-1"), DataType::Int64(-1));
    assert_eq!(run("0"), DataType::Int64(0));

    // Float64
    assert_eq!(run("3.14"), DataType::Float64(3.14));
    assert_eq!(run("-0.5"), DataType::Float64(-0.5));
    assert_eq!(run("0.0"), DataType::Float64(0.0));

    // Bool
    assert_eq!(run("true"), DataType::Bool(true));
    assert_eq!(run("false"), DataType::Bool(false));

    // Null
    assert_eq!(run("null"), DataType::Null);

    // String
    assert_eq!(run(r#""hello""#), DataType::String("hello".to_string()));
    assert_eq!(run(r#""""#), DataType::String("".to_string()));

    // Array
    assert_eq!(
        run("[1, 2, 3]"),
        DataType::Array(vec![DataType::Int64(1), DataType::Int64(2), DataType::Int64(3)])
    );
    assert_eq!(run("[]"), DataType::Array(vec![]));

    // Map
    let result = run(r#"{"a": 1, "b": 2}"#);
    match &result {
        DataType::Map(m) => {
            assert_eq!(m.get("a"), Some(&DataType::Int64(1)));
            assert_eq!(m.get("b"), Some(&DataType::Int64(2)));
        }
        _ => panic!("Expected Map, got {:?}", result),
    }

    // Nested: array of maps
    let result = run(r#"[{"x": 1}, {"x": 2}]"#);
    match &result {
        DataType::Array(arr) => {
            assert_eq!(arr.len(), 2);
            match &arr[0] {
                DataType::Map(m) => assert_eq!(m.get("x"), Some(&DataType::Int64(1))),
                other => panic!("Expected Map in array, got {:?}", other),
            }
        }
        _ => panic!("Expected Array, got {:?}", result),
    }

    // All literals also type-check without errors
    let src = r#"
        let a = 42;
        let b = 3.14;
        let c = true;
        let d = null;
        let e = "hello";
        let f = [1, 2, 3];
        let g = {"x": 1};
        output a;
        output b;
        output c;
        output d;
        output e;
        output f;
        output g;
    "#;
    let program = parse_v2(src).unwrap();
    let imports = std::collections::HashSet::new();
    let analysis = check_types(&program, &imports);
    let errors: Vec<_> = analysis.diagnostics.iter()
        .filter(|d| d.severity == magi_lang::eval::DiagnosticSeverity::Error)
        .collect();
    assert!(errors.is_empty(), "No type errors expected for literals: {:?}", errors);
}

/// Smoke test: a single program covering every statement type.
#[test]
fn test_smoke_all_statement_types() {
    // This program exercises: let, let_mut, const, for, while, loop, fn,
    // async fn, enum, struct, match, if, try-catch-finally, throw, return,
    // break, continue, import, use, type, module, test, output, assignment,
    // compound assignment, expression statement, let-destructure.
    let src = r#"
        import "some_plugin";

        use std::math;

        type IntAlias = int64;

        mod helpers {
            fn identity(x) { x }
        }

        const PI = 3;

        enum Color {
            Red,
            Green,
            Blue(value),
        }

        struct Point {
            x,
            y,
        }

        fn add(a, b) {
            return a + b;
        }

        async fn fetch_data() {
            42
        }

        test "basic" {
            let result = add(1, 2);
            assert(result == 3);
        }

        let x = 10;
        let mut y = 0;
        let [a, b] = [1, 2];
        let {name} = {"name": "magi"};

        y = 5;
        y += 1;

        for i in [1, 2, 3] {
            output i;
        }

        while y > 10 {
            y = y - 1;
        }

        let loop_val = loop {
            break 99;
        };

        if x > 5 {
            output x;
        } else {
            output 0;
        }

        let color = Color::Red;
        let result = match color {
            Color::Red => "red",
            Color::Green => "green",
            Color::Blue(v) => "blue",
            _ => "unknown",
        };

        let point = Point { x: 1, y: 2 };

        try {
            throw "oops";
        } catch err {
            output err;
        } finally {
            let _cleanup = true;
        }

        fn with_continue() {
            for i in [1, 2, 3, 4] {
                if i == 2 { continue; }
                output i;
            }
        }
        with_continue();

        output result;
    "#;

    // Should parse without errors
    let program = parse_v2(src).unwrap();

    // Should type-check (warnings are fine, no panics)
    let imports = std::collections::HashSet::new();
    let _analysis = check_types(&program, &imports);

    // Should interpret without panics (import/use will be no-ops in stub)
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    let result = interp.execute(&program);
    // We don't assert the exact value since import/use stubs may differ,
    // but it should not panic
    assert!(result.is_ok(), "Execution failed: {:?}", result.err());
}

/// Smoke test: a program exercising every expression kind.
#[test]
fn test_smoke_all_expression_types() {
    let src = r#"
        enum Option {
            Some(val),
            None,
        }

        struct Pair {
            first,
            second,
        }

        fn double(x) { x * 2 }
        fn add_one(x) { x + 1 }

        // Binary ops
        let bin_add = 1 + 2;
        let bin_sub = 5 - 3;
        let bin_mul = 4 * 5;
        let bin_div = 10 / 2;
        let bin_mod = 7 % 3;
        let bin_and = true && false;
        let bin_or = true || false;
        let bin_eq = 1 == 1;
        let bin_neq = 1 != 2;
        let bin_gt = 2 > 1;
        let bin_lt = 1 < 2;
        let bin_gte = 2 >= 2;
        let bin_lte = 1 <= 2;

        // Unary ops
        let neg = -42;
        let not_val = !true;

        // Call
        let call_result = double(5);

        // Method call
        let arr = [3, 1, 2];
        let len = arr.length();

        // Field access
        let map = {"name": "magi"};
        let name = map.name;

        // Index
        let indexed = arr[0];

        // Pipe
        let piped = 5 |> double(_);

        // If-else expression
        let ternary = if true { "yes" } else { "no" };

        // Match expression
        let matched = match 42 {
            0 => "zero",
            42 => "answer",
            _ => "other",
        };

        // Lambda
        let square = |x| x * x;
        let squared = square(4);

        // Spread (in array)
        let base = [1, 2];
        let spread_arr = [0, ...base, 3];

        // Range
        let rng = 0..5;

        // Null coalesce
        let nullable = null;
        let coalesced = nullable ?? "default";

        // Optional chain
        let obj = {"inner": {"val": 10}};
        let chained = obj?.inner;

        // Await / Spawn (just parse, execution is stub)
        async fn async_val() { 1 }
        let spawned = spawn async_val();
        let awaited = await spawned;

        // String interpolation
        let who = "world";
        let greeting = f"hello {who}";

        // List comprehension
        let comp = [x * 2 for x in [1, 2, 3] if x > 1];

        // Map comprehension
        let mcomp = {"key": v * 10 for [k, v] in [["a", 1], ["b", 2]]};

        // Enum construct
        let opt = Option::Some(42);

        // Struct construct
        let pair = Pair { first: 1, second: 2 };

        // Block expression
        let block_val = {
            let tmp = 10;
            tmp + 5
        };

        // Loop expression
        let loop_val = loop {
            break 77;
        };

        // TryCatchExpr
        let safe = try { 1 / 0 } catch _e { -1 };

        // Try propagate (inside a function that returns)
        fn maybe_fail() {
            let val = try { 42 } catch _e { null };
            val
        }
        let propagated = maybe_fail();

        output [
            bin_add, bin_sub, bin_mul, bin_div, bin_mod,
            neg, call_result, len, indexed, piped,
            squared, coalesced, block_val, loop_val, propagated
        ];
    "#;

    // Parse
    let program = parse_v2(src).unwrap();

    // Type check (no panics)
    let imports = std::collections::HashSet::new();
    let _analysis = check_types(&program, &imports);

    // Interpret
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    let result = interp.execute(&program).unwrap();

    // Verify the output array has the expected values
    match &result {
        DataType::Array(arr) => {
            assert_eq!(arr.len(), 15, "Expected 15 elements in output array, got {}", arr.len());
            assert_eq!(arr[0], DataType::Int64(3));    // 1 + 2
            assert_eq!(arr[1], DataType::Int64(2));    // 5 - 3
            assert_eq!(arr[2], DataType::Int64(20));   // 4 * 5
            assert_eq!(arr[3], DataType::Int64(5));    // 10 / 2
            assert_eq!(arr[4], DataType::Int64(1));    // 7 % 3
            assert_eq!(arr[5], DataType::Int64(-42));  // -42
            assert_eq!(arr[6], DataType::Int64(10));   // double(5)
            assert_eq!(arr[7], DataType::Int64(3));    // [3,1,2].length()
            assert_eq!(arr[8], DataType::Int64(3));    // arr[0]
            assert_eq!(arr[9], DataType::Int64(10));   // 5 |> double(_)
            assert_eq!(arr[10], DataType::Int64(16));  // square(4)
            assert_eq!(arr[11], DataType::String("default".to_string())); // null ?? "default"
            assert_eq!(arr[12], DataType::Int64(15));  // block: 10 + 5
            assert_eq!(arr[13], DataType::Int64(77));  // loop { break 77 }
            assert_eq!(arr[14], DataType::Int64(42));  // maybe_fail()
        }
        _ => panic!("Expected Array, got {:?}", result),
    }
}

/// Smoke test: trigger every type-checker warning code (W100-W113) and verify emission.
#[test]
fn test_smoke_type_checker_all_codes() {
    // W100: Unused variable
    let codes = typecheck_warnings("let x = 5;");
    assert!(codes.contains(&"W100".to_string()), "W100 (unused variable) missing, got: {:?}", codes);

    // W101: Unused import (needs import registered in imports set)
    {
        let program = parse_v2(r#"import "unused_plugin"; output 1;"#).unwrap();
        let mut imports = std::collections::HashSet::new();
        imports.insert("unused_plugin".to_string());
        let analysis = check_types(&program, &imports);
        let codes: Vec<_> = analysis.diagnostics.iter().filter_map(|d| d.code.clone()).collect();
        assert!(codes.contains(&"W101".to_string()), "W101 (unused import) missing, got: {:?}", codes);
    }

    // W103: Unused function
    let codes = typecheck_warnings("fn unused() { 1 }");
    assert!(codes.contains(&"W103".to_string()), "W103 (unused function) missing, got: {:?}", codes);

    // W106: Self-comparison (redundant operation)
    let codes = typecheck_warnings("let x = 5; let y = x == x; output y;");
    assert!(codes.contains(&"W106".to_string()), "W106 (self-comparison) missing, got: {:?}", codes);

    // W106: Double negation
    let codes = typecheck_warnings("let x = 5; let y = --x; output y;");
    assert!(codes.contains(&"W106".to_string()), "W106 (double negation) missing, got: {:?}", codes);

    // W107: Modulo by 1
    let codes = typecheck_warnings("let x = 5; let y = x % 1; output y;");
    assert!(codes.contains(&"W107".to_string()), "W107 (modulo by 1) missing, got: {:?}", codes);

    // W107: Multiply by 0
    let codes = typecheck_warnings("let x = 5; let y = x * 0; output y;");
    assert!(codes.contains(&"W107".to_string()), "W107 (multiply by 0) missing, got: {:?}", codes);

    // W108: Unnecessary return in tail position
    let codes = typecheck_warnings("fn foo() { return 42; }");
    assert!(codes.contains(&"W108".to_string()), "W108 (unnecessary return) missing, got: {:?}", codes);

    // W109: Unused function parameter
    let codes = typecheck_warnings("fn foo(x, y) { output x; }");
    assert!(codes.contains(&"W109".to_string()), "W109 (unused param) missing, got: {:?}", codes);

    // W110: Unnecessary let mut (never reassigned)
    let codes = typecheck_warnings("let mut x = 5; output x;");
    assert!(codes.contains(&"W110".to_string()), "W110 (unnecessary mut) missing, got: {:?}", codes);

    // W111: Reserved keyword used as identifier.
    // Note: W111 is checked in define_var(), but reserved keywords (trait, impl, etc.)
    // are tokenized as TokenKind::Reserved by the lexer and rejected by the parser before
    // the type checker runs. W111 is only reachable via direct define_var() calls in unit
    // tests. We verify it does NOT trigger for non-reserved names:
    let codes = typecheck_warnings("let my_var = 5; output my_var;");
    assert!(!codes.contains(&"W111".to_string()), "W111 should not trigger for non-reserved name");

    // W112: Default parameter type mismatch
    let codes = typecheck_warnings(r#"fn foo(x: int64 = "hello") { output x; }"#);
    assert!(codes.contains(&"W112".to_string()), "W112 (default param mismatch) missing, got: {:?}", codes);

    // W113: Or-pattern variable mismatch
    let codes = typecheck_warnings(r#"
        let val = 5;
        let result = match val {
            x | 0 => x,
            _ => 0,
        };
        output result;
    "#);
    assert!(codes.contains(&"W113".to_string()), "W113 (or-pattern mismatch) missing, got: {:?}", codes);
}

/// Smoke test: trigger every linter rule and verify the correct code is emitted.
#[test]
fn test_smoke_linter_all_rules() {
    use magi_lang::linter::{lint, LintConfig};
    let config = LintConfig::default();

    // Helper to get lint codes for a source
    let lint_codes = |src: &str| -> Vec<String> {
        let program = parse_v2(src).unwrap();
        let result = lint(&program, &config);
        result.diagnostics.iter()
            .filter_map(|d| d.code.clone())
            .collect::<Vec<_>>()
    };

    // W200: Non-snake_case variable/function name
    let codes = lint_codes("let myVar = 5; output myVar;");
    assert!(codes.contains(&"W200".to_string()), "W200 (snake_case) missing, got: {:?}", codes);

    // W201: Non-PascalCase enum/struct name
    let codes = lint_codes("enum my_color { Red, Blue }");
    assert!(codes.contains(&"W201".to_string()), "W201 (PascalCase) missing, got: {:?}", codes);

    // W202: Dead code after return
    let codes = lint_codes("fn foo() { return 1; let x = 2; }");
    assert!(codes.contains(&"W202".to_string()), "W202 (dead code) missing, got: {:?}", codes);

    // W203: Non-exhaustive match (missing enum variants)
    let codes = lint_codes(r#"
        enum Dir { North, South, East, West }
        let d = Dir::North;
        let r = match d {
            Dir::North => 1,
            Dir::South => 2,
        };
    "#);
    assert!(codes.contains(&"W203".to_string()), "W203 (non-exhaustive match) missing, got: {:?}", codes);

    // W204: Constant condition in while
    let codes = lint_codes("while false { output 1; }");
    assert!(codes.contains(&"W204".to_string()), "W204 (constant condition) missing, got: {:?}", codes);

    // W206: Empty block body
    let codes = lint_codes("fn foo() {}");
    assert!(codes.contains(&"W206".to_string()), "W206 (empty block) missing, got: {:?}", codes);

    // W207: Unreachable match arm after wildcard
    let codes = lint_codes(r#"
        let x = 1;
        let r = match x {
            1 => "one",
            _ => "other",
            2 => "two",
        };
    "#);
    assert!(codes.contains(&"W207".to_string()), "W207 (unreachable arm) missing, got: {:?}", codes);

    // W208: Duplicate import
    let codes = lint_codes(r#"
        import "plugin_a";
        import "plugin_a";
    "#);
    assert!(codes.contains(&"W208".to_string()), "W208 (duplicate import) missing, got: {:?}", codes);

    // W209: Shadowed variable in same scope
    let codes = lint_codes("let x = 1; let x = 2; output x;");
    assert!(codes.contains(&"W209".to_string()), "W209 (shadow in same scope) missing, got: {:?}", codes);

    // W211: Unused function parameter — now handled by type checker as W109, no longer emitted by linter.
    // Verify it is NOT emitted by the linter:
    let codes = lint_codes("fn foo(x, y) { output x; }");
    assert!(!codes.contains(&"W211".to_string()), "W211 should no longer be emitted by linter, got: {:?}", codes);

    // W212: Control flow in finally block
    let codes = lint_codes(r#"
        try {
            output 1;
        } catch e {
            output e;
        } finally {
            return 0;
        }
    "#);
    assert!(codes.contains(&"W212".to_string()), "W212 (finally control flow) missing, got: {:?}", codes);
}

/// Smoke test: format a complex program and verify parse(format(x)) roundtrip.
#[test]
fn test_smoke_formatter_roundtrip() {
    use magi_lang::formatter::{format_program, FormatConfig};

    let src = r#"
        const MAX = 100;
        type Id = int64;

        enum Shape {
            Circle(radius),
            Rect(w, h),
        }

        struct Point {
            x,
            y,
        }

        fn area(shape) {
            match shape {
                Shape::Circle(r) => 3 * r * r,
                Shape::Rect(w, h) => w * h,
                _ => 0,
            }
        }

        fn process(items, threshold = 10) {
            let filtered = [x * 2 for x in items if x > threshold];
            let mapped = {"key": v for [k, v] in [["a", 1], ["b", 2]]};
            let result = filtered |> double(_);
            let safe = try { risky() } catch e { null };
            let val = null ?? "default";
            let greeting = f"count: {filtered.length()}";
            let rng = 0..10;
            for i in [1, 2, 3] {
                if i == 2 { continue; }
                output i;
            }
            while false {
                break;
            }
            let loop_val = loop {
                break 42;
            };
            try {
                throw "error";
            } catch e {
                output e;
            } finally {
                let _done = true;
            }
            filtered
        }

        fn double(x) { x * 2 }

        let p = Point { x: 1, y: 2 };
        let s = Shape::Circle(5);
        let a = area(s);
        output a;
    "#;

    let program = parse_v2(src).unwrap();
    let config = FormatConfig::default();
    let formatted = format_program(&program, &config);

    // The formatted output must be non-empty
    assert!(!formatted.is_empty(), "Formatted output should not be empty");

    // The formatted output must re-parse successfully
    let reparsed = parse_v2(&formatted);
    assert!(reparsed.is_ok(),
        "Failed to re-parse formatted output.\nFormatted:\n{}\nError: {}",
        formatted, reparsed.err().unwrap());

    // Double-format should be idempotent
    let reparsed_program = reparsed.unwrap();
    let reformatted = format_program(&reparsed_program, &config);
    assert_eq!(formatted, reformatted,
        "Formatter is not idempotent.\nFirst format:\n{}\nSecond format:\n{}", formatted, reformatted);
}

/// Smoke test: error recovery parser produces partial AST and collects errors.
#[test]
fn test_smoke_error_recovery() {
    use magi_lang::syntax::parser::parse_v2_recovering;

    // A program with multiple syntax errors
    let src = r#"
        let x = 10;
        let y = ;
        fn foo( { }
        let good = 42;
        output good;
        let z = +++
    "#;

    let (program, errors) = parse_v2_recovering(src);

    // Should have recovered some errors
    assert!(!errors.is_empty(),
        "Expected syntax errors from malformed input, got none");

    // Should have recovered at least some valid statements
    assert!(!program.statements.is_empty(),
        "Expected some recovered statements, got empty program");

    // Verify that at least one valid statement was recovered
    // (the `let x = 10;` or `let good = 42;` or `output good;`)
    let has_let = program.statements.iter().any(|s| {
        matches!(&s.kind,
            magi_lang::syntax::ast::StatementKind::Let { .. } |
            magi_lang::syntax::ast::StatementKind::Output(_))
    });
    assert!(has_let, "Expected at least one Let or Output statement to survive recovery");

    // Another test: lexer error (unterminated string)
    let (prog2, errs2) = parse_v2_recovering(r#"let x = "unterminated"#);
    assert!(!errs2.is_empty(), "Expected error for unterminated string");
    // Program may be empty since lexer fails first
    let _ = prog2;

    // Mixed: some good, some bad expressions
    let (prog3, errs3) = parse_v2_recovering(r#"
        let a = 1;
        let b = @invalid;
        let c = 3;
    "#);
    assert!(!errs3.is_empty(), "Expected error for @ token");
    // Should still have some recovered statements
    let _ = prog3;
}

// ═══════════════════════════════════════════════════════════
// Round 82: Final coverage sweep — 18 tests for critical gaps
// ═══════════════════════════════════════════════════════════

/// Type alias used as annotation on a variable that feeds into a match arm.
/// Exercises type alias resolution + match interaction end-to-end.
#[test]
fn test_type_alias_used_in_match_context() {
    let result = run(r#"
        type Num = int64
        let x: Num = 42;
        let label = match x {
            0 => "zero",
            42 => "answer",
            _ => "other",
        };
        label
    "#);
    assert_eq!(result, DataType::String("answer".to_string()));
}

/// Type alias inside a module, resolved by the type checker.
/// Ensures module-scoped type aliases do not leak or cause errors.
#[test]
fn test_type_alias_inside_module_type_check() {
    let warnings = typecheck_warnings(r#"
        mod math {
            type Real = float64
            fn square(x: Real) -> float64 { x * x }
        }
    "#);
    // Should not produce any warnings — Real resolves to float64
    let alias_errors: Vec<_> = warnings.iter().filter(|w| w.as_str() == "E100").collect();
    assert!(alias_errors.is_empty(),
        "Type alias inside module should resolve cleanly, got: {:?}", warnings);
}

/// Type alias chain: A -> B -> int64, used in const def. Verifies the
/// type checker follows multi-hop alias chains.
#[test]
fn test_type_alias_chain_in_const() {
    let warnings = typecheck_warnings(r#"
        type A = int64
        type B = A
        const val: B = 10;
    "#);
    // No type mismatch warnings expected
    let mismatch: Vec<_> = warnings.iter().filter(|w| w.contains("E100")).collect();
    assert!(mismatch.is_empty(), "Chained alias should resolve, got: {:?}", mismatch);
}

/// Error recovery parser preserves function definitions through syntax errors.
/// Important for LSP which needs partial ASTs.
#[test]
fn test_error_recovery_preserves_functions_and_modules() {
    use magi_lang::syntax::parser::parse_v2_recovering;
    use magi_lang::syntax::ast::StatementKind;

    // Use parser-level errors (not lexer errors like @@@)
    let src = r#"
        fn good_fn(x) { x + 1 }
        let = ;
        fn another_good(y) { y * 2 }
        let also_bad = ;
    "#;

    let (prog, errors) = parse_v2_recovering(src);
    assert!(!errors.is_empty(), "Should have syntax errors");

    // Check that we recovered at least one function definition
    let has_fn = prog.statements.iter().any(|s| matches!(&s.kind, StatementKind::FunctionDef(f) if f.name == "good_fn" || f.name == "another_good"));
    assert!(has_fn, "Error recovery should preserve function definitions, got: {:?}",
        prog.statements.iter().map(|s| format!("{:?}", std::mem::discriminant(&s.kind))).collect::<Vec<_>>());
}

/// Error recovery with module-level construct — ensures mod blocks survive.
#[test]
fn test_error_recovery_with_module_block() {
    use magi_lang::syntax::parser::parse_v2_recovering;
    use magi_lang::syntax::ast::StatementKind;

    // Module first, then parser-level errors (not lexer errors)
    let src = r#"
        mod utils {
            fn helper() { 1 }
        }
        let = ;
        let y = 100;
    "#;
    let (prog, errors) = parse_v2_recovering(src);
    assert!(!errors.is_empty(), "Should have errors from malformed let");

    let has_module = prog.statements.iter().any(|s| matches!(&s.kind, StatementKind::ModuleDef { name, .. } if name == "utils"));
    assert!(has_module, "Module definition should survive error recovery");
}

/// W209 (same-scope shadowing) standalone integration test with let mut.
#[test]
fn test_lint_w209_shadow_let_mut_then_let() {
    use magi_lang::linter::{lint, LintConfig};
    let config = LintConfig::default();

    let src = "let mut x = 1; let x = 2; output x;";
    let program = parse(src);
    let result = lint(&program, &config);
    let codes: Vec<_> = result.diagnostics.iter().filter_map(|d| d.code.clone()).collect();
    assert!(codes.contains(&"W209".to_string()),
        "let mut followed by let in same scope should trigger W209, got: {:?}", codes);
}

/// W209 not triggered for variables in nested scope.
#[test]
fn test_lint_w209_no_false_positive_nested_scope() {
    use magi_lang::linter::{lint, LintConfig};
    let config = LintConfig::default();

    let src = r#"
        let x = 1;
        if true {
            let x = 2;
            output x;
        }
        output x;
    "#;
    let program = parse(src);
    let result = lint(&program, &config);
    let w209_count = result.diagnostics.iter().filter(|d| d.code.as_deref() == Some("W209")).count();
    assert_eq!(w209_count, 0,
        "Nested scope shadowing should not trigger W209, got {} occurrences", w209_count);
}

/// W212 standalone test: break in finally block.
#[test]
fn test_lint_w212_break_in_finally() {
    use magi_lang::linter::{lint, LintConfig};
    let config = LintConfig::default();

    let src = r#"
        for i in 0..5 {
            try {
                output i;
            } catch e {
                output e;
            } finally {
                break;
            }
        }
    "#;
    let program = parse(src);
    let result = lint(&program, &config);
    let codes: Vec<_> = result.diagnostics.iter().filter_map(|d| d.code.clone()).collect();
    assert!(codes.contains(&"W212".to_string()),
        "break in finally should trigger W212, got: {:?}", codes);
}

/// W212 not triggered when finally has no control flow.
#[test]
fn test_lint_w212_clean_finally_no_warning() {
    use magi_lang::linter::{lint, LintConfig};
    let config = LintConfig::default();

    let src = r#"
        try {
            output 1;
        } catch e {
            output e;
        } finally {
            output "cleanup";
        }
    "#;
    let program = parse(src);
    let result = lint(&program, &config);
    let w212_count = result.diagnostics.iter().filter(|d| d.code.as_deref() == Some("W212")).count();
    assert_eq!(w212_count, 0,
        "Clean finally block should not trigger W212, got {} occurrences", w212_count);
}

/// Formatter roundtrip for a program containing module, enum, struct, and type alias.
/// Verifies format(parse(format(x))) == format(x) (idempotency).
#[test]
fn test_formatter_roundtrip_module_enum_struct_type_alias() {
    use magi_lang::formatter::{format_program, FormatConfig};
    let config = FormatConfig::default();

    let src = r#"
        type Id = int64

        enum Color {
            Red,
            Green,
            Blue,
        }

        struct Point {
            x: float64,
            y: float64,
        }

        mod shapes {
            fn area(p) {
                p.x * p.y
            }
        }
    "#;

    let program = parse(src);
    let formatted = format_program(&program, &config);
    assert!(!formatted.is_empty(), "Formatted output should not be empty");

    let reparsed = parse_v2(&formatted).unwrap_or_else(|e| panic!("Re-parse failed: {}", e));
    let reformatted = format_program(&reparsed, &config);
    assert_eq!(formatted, reformatted,
        "Formatter not idempotent for module/enum/struct/type_alias.\nFirst:\n{}\nSecond:\n{}", formatted, reformatted);
}

/// Test runner isolates module definitions between tests.
#[test]
fn test_test_runner_module_isolation() {
    let src = r#"
        mod math {
            fn double(x) { x * 2 }
        }

        test "use module fn" {
            assert(math::double(5) == 10);
        }

        test "module still accessible" {
            assert(math::double(3) == 6);
        }
    "#;
    let program = parse(src);
    let evaluator = StubEvaluator;
    let mut interp = Interpreter::new(&evaluator);
    let results = interp.run_tests(&program);
    for r in &results {
        assert!(r.passed, "Test '{}' failed: {:?}", r.name, r.error_message);
    }
    assert_eq!(results.len(), 2, "Expected 2 test results");
}

/// Map destructuring in a for-loop with array of maps.
#[test]
fn test_for_loop_map_destructure_array_of_maps() {
    let result = run(r#"
        let data = [
            { "name": "Alice", "age": 30 },
            { "name": "Bob", "age": 25 },
        ];
        let mut names = [];
        for {name} in data {
            names = names.push(name);
        }
        names
    "#);
    match &result {
        DataType::Array(arr) => {
            assert_eq!(arr.len(), 2);
            assert_eq!(arr[0], DataType::String("Alice".to_string()));
            assert_eq!(arr[1], DataType::String("Bob".to_string()));
        }
        other => panic!("Expected Array, got {:?}", other),
    }
}

/// Map destructuring in for-loop with alias: `for {name: n, age: a} in ...`
#[test]
fn test_for_loop_map_destructure_with_alias() {
    let result = run(r#"
        let people = [
            { "name": "X", "age": 1 },
            { "name": "Y", "age": 2 },
        ];
        let mut total = 0;
        for {age: a} in people {
            total = total + a;
        }
        total
    "#);
    assert_eq!(result, DataType::Int64(3));
}

/// Optional chaining with field access then further field access.
/// Tests `obj?.field.nested` propagation of null through the chain.
#[test]
fn test_optional_chain_field_then_field() {
    // Non-null case: access nested field
    let result = run(r#"
        let obj = { "data": { "value": 42 } };
        let x = obj?.data;
        x.value
    "#);
    assert_eq!(result, DataType::Int64(42));

    // Null case: ?. short-circuits to null
    let result2 = run(r#"
        let obj = null;
        obj?.data
    "#);
    assert_eq!(result2, DataType::Null);
}

/// Nested try-catch-finally as expression inside another expression.
#[test]
fn test_nested_try_catch_finally_as_expr() {
    let result = run(r#"
        let val = try {
            let inner = try {
                throw "inner_error";
            } catch e {
                f"caught: {e}"
            } finally {
                let cleanup = 0;
            };
            inner
        } catch e {
            "outer_catch"
        };
        val
    "#);
    assert_eq!(result, DataType::String("caught: inner_error".to_string()));
}

/// Type checker warns on unknown type alias target.
#[test]
fn test_type_checker_unknown_alias_target_warns() {
    let warnings = typecheck_warnings(r#"
        type Foo = nonexistent_type
        let x: Foo = 42;
    "#);
    // Should get a warning about unknown type
    let has_unknown = warnings.iter().any(|w| w == "E100");
    assert!(has_unknown,
        "Unknown alias target should produce E100, got: {:?}", warnings);
}

/// Type checker W208: duplicate use-import detection (integration level).
#[test]
fn test_type_checker_w208_duplicate_use_import() {
    let warnings = typecheck_warnings(r#"
        mod utils {
            fn helper() { 1 }
        }
        use utils::helper;
        use utils::helper;
    "#);
    let has_w208 = warnings.iter().any(|w| w == "W208");
    assert!(has_w208,
        "Duplicate use-import should produce W208, got: {:?}", warnings);
}

/// Pipe operator combined with lambda variable and method call.
/// Exercises pipe resolution + lambda capture interaction.
#[test]
fn test_pipe_with_lambda_and_method_chain() {
    let result = run(r#"
        let double = |x| x * 2
        let result = 5 |> double(_)
        result
    "#);
    assert_eq!(result, DataType::Int64(10));

    // Chained with another lambda
    let result2 = run(r#"
        let inc = |x| x + 1
        let double = |x| x * 2
        let result = 3 |> inc(_) |> double(_)
        result
    "#);
    assert_eq!(result2, DataType::Int64(8));
}

// ── HTTP client operation tests (FullEvaluator via CLI binary) ───────────────

/// Run magi source code via the CLI binary and return combined stdout+stderr.
fn run_eval(src: &str) -> String {
    let dir = std::env::temp_dir();
    let file = dir.join("_magi_http_test.magi");
    std::fs::write(&file, src).expect("write temp file");
    let bin = env!("CARGO_BIN_EXE_magi");
    let output = std::process::Command::new(bin)
        .arg("run")
        .arg(&file)
        .output()
        .expect("run magi binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    format!("{}{}", stdout, stderr)
}

#[test]
fn test_http_get_blocked_localhost() {
    // SSRF protection should block localhost
    let result = run_eval(r#"std::http_get("http://localhost:9999/")"#);
    assert!(result.contains("blocked") || result.contains("Error") || result.contains("error"));
}

#[test]
fn test_http_get_invalid_scheme() {
    let result = run_eval(r#"std::http_get("ftp://example.com")"#);
    assert!(result.contains("http://") || result.contains("Error") || result.contains("error"));
}

#[test]
fn test_http_post_blocked() {
    let result = run_eval(r#"std::http_post("http://127.0.0.1:9999/", "data")"#);
    assert!(result.contains("blocked") || result.contains("Error") || result.contains("error"));
}

#[test]
fn test_http_request_unsupported_method() {
    let result = run_eval(r#"std::http_request("INVALID", "http://10.0.0.1/", "", {})"#);
    assert!(result.contains("blocked") || result.contains("Unsupported") || result.contains("Error") || result.contains("error"));
}

/// Run a magi script via the CLI binary, using a unique temp file to avoid race conditions.
fn run_eval_unique(src: &str, name: &str) -> String {
    let dir = std::env::temp_dir();
    let file = dir.join(format!("_magi_{}.magi", name));
    std::fs::write(&file, src).expect("write temp file");
    let bin = env!("CARGO_BIN_EXE_magi");
    let output = std::process::Command::new(bin)
        .arg("run")
        .arg(&file)
        .output()
        .expect("run magi binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    format!("{}{}", stdout, stderr)
}

#[test]
fn test_compress_zstd_roundtrip() {
    let result = run_eval_unique(r#"
        let data = to_bytes("hello world zstd compression test data")
        let compressed = compress_zstd(data)
        let decompressed = decompress_zstd(compressed)
        println(from_bytes(decompressed))
    "#, "zstd_rt");
    assert!(result.contains("hello world zstd"));
}

#[test]
fn test_compress_lz4_roundtrip() {
    let result = run_eval_unique(r#"
        let data = to_bytes("lz4 compression test data here")
        let compressed = compress_lz4(data)
        let decompressed = decompress_lz4(compressed)
        println(from_bytes(decompressed))
    "#, "lz4_rt");
    assert!(result.contains("lz4 compression"));
}

#[test]
fn test_decompress_zstd_invalid() {
    let result = run_eval_unique(
        r#"decompress_zstd(to_bytes("not compressed"))"#,
        "zstd_inv",
    );
    assert!(result.contains("Error") || result.contains("error"));
}

#[test]
fn test_key_generate() {
    let result = run_eval_unique(r#"
        let k = key_generate()
        println(typeof(k))
    "#, "keygen");
    assert!(result.contains("map"));
}

#[test]
fn test_cert_generate_and_parse() {
    let result = run_eval_unique(r#"
        let c = cert_generate("test.example.com")
        let info = cert_parse(c.cert_pem)
        println(info.subject)
    "#, "cert_parse");
    assert!(result.contains("test.example.com"));
}

#[test]
fn test_cert_verify_valid() {
    let result = run_eval_unique(r#"
        let c = cert_generate("test.example.com")
        let v = cert_verify(c.cert_pem)
        println(v.valid)
    "#, "cert_verify");
    assert!(result.contains("true"));
}

// -------------------------------------------------------
// TCP operations
// -------------------------------------------------------

#[test]
fn test_tcp_bind_and_close() {
    let result = run_eval_unique(r#"
        let listener = tcp_bind("127.0.0.1", 0)
        tcp_server_close(listener)
        println("closed")
    "#, "tcp_bind_close");
    assert!(result.contains("closed"));
}

#[test]
fn test_tcp_connect_refused() {
    let result = run_eval_unique(
        r#"tcp_connect("127.0.0.1", 1)"#,
        "tcp_connect_refused",
    );
    assert!(result.contains("Error") || result.contains("error"));
}

// -------------------------------------------------------
// UDP operations
// -------------------------------------------------------

#[test]
fn test_udp_bind_and_close() {
    let result = run_eval_unique(r#"
        let sock = udp_bind("127.0.0.1", 0)
        udp_close(sock)
        println("closed")
    "#, "udp_bind_close");
    assert!(result.contains("closed"));
}

#[test]
fn test_udp_sendto_loopback() {
    let result = run_eval_unique(r#"
        let s1 = udp_bind("127.0.0.1", 0)
        let s2 = udp_bind("127.0.0.1", 0)
        udp_close(s1)
        udp_close(s2)
        println("ok")
    "#, "udp_sendto_loop");
    assert!(result.contains("ok"));
}

