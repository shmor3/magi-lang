//! MAGI language CLI — interpret and compile .magi files.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::process;

use magi_lang::compiler;
use magi_lang::eval::{DiagnosticSeverity, EvalError, OperationEvaluator};
use magi_lang::syntax::interpreter::{resolve_package_from_source, Interpreter, ResolvedPackage};
use magi_lang::syntax::parser::parse_v2;
use magi_lang::types::{DataType, OperationType};

/// Maximum output string length (10 MB).
const MAX_STRING_OUTPUT: usize = 10_000_000;

/// Maximum array element count.
const MAX_ARRAY_ELEMENTS: usize = 10_000_000;

/// A full-featured operation evaluator for standalone execution.
struct FullEvaluator;

impl OperationEvaluator for FullEvaluator {
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
            .or(inputs.get("value"))
            .cloned()
            .unwrap_or(DataType::Null);
        let array = inputs.get("array").cloned().unwrap_or(DataType::Null);
        let value = inputs.get("value").cloned().unwrap_or(DataType::Null);
        let map = inputs.get("map").cloned().unwrap_or(DataType::Null);
        let key = inputs.get("key").cloned().unwrap_or(DataType::Null);

        match op {
            // Arithmetic
            OperationType::Add => {
                // String concatenation for Add only
                if let (DataType::String(x), DataType::String(y)) = (&a, &b) {
                    let result_len = x.len().saturating_add(y.len());
                    if result_len > MAX_STRING_OUTPUT {
                        return Err(EvalError::InvalidInput(format!("string concatenation would produce {} bytes (max {})", result_len, MAX_STRING_OUTPUT)));
                    }
                    return Ok(DataType::String(format!("{}{}", x, y)));
                }
                num_binop(&a, &b, i64::checked_add, |x, y| x + y)
            }
            OperationType::Subtract => num_binop(&a, &b, i64::checked_sub, |x, y| x - y),
            OperationType::Multiply => num_binop(&a, &b, i64::checked_mul, |x, y| x * y),
            OperationType::Divide => {
                match (&a, &b) {
                    (DataType::Int64(x), DataType::Int64(y)) => {
                        if *y == 0 { return Err(EvalError::DivisionByZero); }
                        match x.checked_div(*y) {
                            Some(v) => Ok(DataType::Int64(v)),
                            None => Err(EvalError::InvalidInput("integer overflow".to_string())),
                        }
                    }
                    (DataType::Float64(_), DataType::Float64(y)) if *y == 0.0 => Err(EvalError::DivisionByZero),
                    (DataType::Int64(_), DataType::Float64(y)) if *y == 0.0 => Err(EvalError::DivisionByZero),
                    (DataType::Float64(_), DataType::Int64(y)) if *y == 0 => Err(EvalError::DivisionByZero),
                    (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x / y)),
                    (DataType::Int64(x), DataType::Float64(y)) => Ok(DataType::Float64(*x as f64 / y)),
                    (DataType::Float64(x), DataType::Int64(y)) => Ok(DataType::Float64(x / *y as f64)),
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::Modulo => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => {
                    if *y == 0 { return Err(EvalError::DivisionByZero); }
                    match x.checked_rem(*y) {
                        Some(v) => Ok(DataType::Int64(v)),
                        None => Err(EvalError::InvalidInput("integer overflow".to_string())),
                    }
                }
                (DataType::Float64(_), DataType::Float64(y)) if *y == 0.0 => Err(EvalError::DivisionByZero),
                (DataType::Int64(_), DataType::Float64(y)) if *y == 0.0 => Err(EvalError::DivisionByZero),
                (DataType::Float64(_), DataType::Int64(y)) if *y == 0 => Err(EvalError::DivisionByZero),
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x % y)),
                (DataType::Int64(x), DataType::Float64(y)) => Ok(DataType::Float64(*x as f64 % y)),
                (DataType::Float64(x), DataType::Int64(y)) => Ok(DataType::Float64(x % *y as f64)),
                _ => Ok(DataType::Null),
            },

            // Comparison
            OperationType::Equal => Ok(DataType::Bool(a == b)),
            OperationType::NotEqual => Ok(DataType::Bool(a != b)),
            OperationType::Greater => num_cmp(&a, &b, |x, y| x > y, |x, y| x > y),
            OperationType::Less => num_cmp(&a, &b, |x, y| x < y, |x, y| x < y),
            OperationType::GreaterEq => num_cmp(&a, &b, |x, y| x >= y, |x, y| x >= y),
            OperationType::LessEq => num_cmp(&a, &b, |x, y| x <= y, |x, y| x <= y),

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
                DataType::Int64(x) => match x.checked_neg() {
                    Some(v) => Ok(DataType::Int64(v)),
                    None => Err(EvalError::InvalidInput("integer overflow".to_string())),
                },
                DataType::Float64(x) => Ok(DataType::Float64(-x)),
                _ => Ok(DataType::Null),
            },

            // String
            OperationType::Concat => {
                let (xs, ys) = match (&a, &b) {
                    (DataType::String(x), DataType::String(y)) => (x.as_str().to_string(), y.as_str().to_string()),
                    _ => (a.to_string_lossy(), b.to_string_lossy()),
                };
                let result_len = xs.len().saturating_add(ys.len());
                if result_len > MAX_STRING_OUTPUT {
                    return Err(EvalError::InvalidInput(format!("concat would produce {} bytes (max {})", result_len, MAX_STRING_OUTPUT)));
                }
                Ok(DataType::String(format!("{}{}", xs, ys)))
            },
            OperationType::ToString => Ok(DataType::String(input.to_string_lossy())),

            // Map access
            OperationType::MapGet => {
                match (&map, &key) {
                    (DataType::Map(m), DataType::String(k)) => {
                        Ok(m.get(k).cloned().unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::MapSet => {
                match (&map, &key) {
                    (DataType::Map(m), DataType::String(k)) => {
                        let mut new_map = m.clone();
                        new_map.insert(k.clone(), value.clone());
                        Ok(DataType::Map(new_map))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::MapKeys => match &map {
                DataType::Map(m) => Ok(DataType::Array(m.keys().map(|k| DataType::String(k.clone())).collect())),
                _ => Ok(DataType::Array(vec![])),
            },
            OperationType::MapValues => match &map {
                DataType::Map(m) => Ok(DataType::Array(m.values().cloned().collect())),
                _ => Ok(DataType::Array(vec![])),
            },

            // Array
            OperationType::ArrayLength => match &array {
                DataType::Array(arr) => Ok(DataType::Int64(arr.len() as i64)),
                _ => Ok(DataType::Int64(0)),
            },
            OperationType::ArrayPush => {
                let mut arr = match &array { DataType::Array(a) => a.clone(), _ => vec![] };
                if arr.len() >= MAX_ARRAY_ELEMENTS {
                    return Err(EvalError::InvalidInput(format!("array push would exceed {} elements", MAX_ARRAY_ELEMENTS)));
                }
                arr.push(value.clone());
                Ok(DataType::Array(arr))
            }
            OperationType::ArrayPop => match &array {
                DataType::Array(arr) if !arr.is_empty() => Ok(arr.last().cloned().unwrap_or(DataType::Null)),
                _ => Ok(DataType::Null),
            },
            OperationType::ArraySlice => Ok(DataType::Null),
            OperationType::ArraySort => match &array {
                DataType::Array(arr) => {
                    let mut sorted = arr.clone();
                    sorted.sort_by(|a, b| {
                        match (a, b) {
                            (DataType::Int64(x), DataType::Int64(y)) => x.cmp(y),
                            (DataType::Float64(x), DataType::Float64(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
                            (DataType::String(x), DataType::String(y)) => x.cmp(y),
                            _ => a.to_string_lossy().cmp(&b.to_string_lossy()),
                        }
                    });
                    Ok(DataType::Array(sorted))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayReverse => match &array {
                DataType::Array(arr) => { let mut r = arr.clone(); r.reverse(); Ok(DataType::Array(r)) }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayContains => match (&array, &value) {
                (DataType::Array(arr), val) => Ok(DataType::Bool(arr.contains(val))),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::ArrayJoin => match &array {
                DataType::Array(arr) => {
                    let s: Vec<String> = arr.iter().map(|v| v.to_string_lossy()).collect();
                    let estimated_len: usize = s.iter().map(|p| p.len()).sum::<usize>() + s.len().saturating_sub(1);
                    if estimated_len > MAX_STRING_OUTPUT {
                        return Err(EvalError::InvalidInput(format!("join result exceeds {} byte limit", MAX_STRING_OUTPUT)));
                    }
                    Ok(DataType::String(s.join(",")))
                }
                _ => Ok(DataType::String(String::new())),
            },

            // String ops
            OperationType::Length => match &input {
                DataType::String(s) => Ok(DataType::Int64(s.chars().count() as i64)),
                _ => Ok(DataType::Int64(0)),
            },
            OperationType::Split => {
                let delim = inputs.get("delimiter").cloned().unwrap_or(DataType::Null);
                match (&input, &delim) {
                    (DataType::String(s), DataType::String(sep)) => {
                        if sep.is_empty() {
                            return Err(EvalError::InvalidInput("split delimiter must not be empty".to_string()));
                        }
                        let parts: Vec<DataType> = s.split(sep.as_str()).take(MAX_ARRAY_ELEMENTS + 1).map(|p| DataType::String(p.to_string())).collect();
                        if parts.len() > MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!("split result exceeds {} element limit", MAX_ARRAY_ELEMENTS)));
                        }
                        Ok(DataType::Array(parts))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            },
            OperationType::Contains => {
                let search = inputs.get("search").cloned().unwrap_or(DataType::Null);
                match (&input, &search) {
                    (DataType::String(s), DataType::String(sub)) => Ok(DataType::Bool(s.contains(sub.as_str()))),
                    _ => Ok(DataType::Bool(false)),
                }
            },
            OperationType::Replace => {
                let search = inputs.get("search").cloned().unwrap_or(DataType::Null);
                let replace = inputs.get("replace").cloned().unwrap_or(DataType::Null);
                match (&input, &search, &replace) {
                    (DataType::String(s), DataType::String(from), DataType::String(to)) => {
                        if !from.is_empty() && to.len() > from.len() {
                            let match_count = s.matches(from.as_str()).count();
                            let growth = match_count.saturating_mul(to.len().saturating_sub(from.len()));
                            if s.len().saturating_add(growth) > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!("replace result exceeds {} byte limit", MAX_STRING_OUTPUT)));
                            }
                        }
                        Ok(DataType::String(s.replace(from.as_str(), to.as_str())))
                    }
                    _ => Ok(input.clone()),
                }
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
            OperationType::ToUpper => match &input {
                DataType::String(s) => Ok(DataType::String(s.to_uppercase())),
                _ => Ok(DataType::Null),
            },
            OperationType::ToLower => match &input {
                DataType::String(s) => Ok(DataType::String(s.to_lowercase())),
                _ => Ok(DataType::Null),
            },
            OperationType::StartsWith => {
                let prefix = inputs.get("prefix").cloned().unwrap_or(DataType::Null);
                match (&input, &prefix) {
                    (DataType::String(s), DataType::String(p)) => Ok(DataType::Bool(s.starts_with(p.as_str()))),
                    _ => Ok(DataType::Bool(false)),
                }
            },
            OperationType::EndsWith => {
                let suffix = inputs.get("suffix").cloned().unwrap_or(DataType::Null);
                match (&input, &suffix) {
                    (DataType::String(s), DataType::String(sfx)) => Ok(DataType::Bool(s.ends_with(sfx.as_str()))),
                    _ => Ok(DataType::Bool(false)),
                }
            },
            OperationType::Substring => {
                // "hello".substring(start, end) — character-based indices
                // input_1 = start index, input_2 = optional end index
                let start_val = inputs.get("input_1").or(inputs.get("start")).cloned().unwrap_or(DataType::Int64(0));
                let end_val = inputs.get("input_2").or(inputs.get("end")).cloned();
                match &input {
                    DataType::String(s) => {
                        let chars: Vec<char> = s.chars().collect();
                        let len = chars.len() as i64;
                        let start = match &start_val {
                            DataType::Int64(n) => {
                                let n = *n;
                                if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                            }
                            _ => 0,
                        };
                        let end = match &end_val {
                            Some(DataType::Int64(n)) => {
                                let n = *n;
                                if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                            }
                            _ => chars.len(),
                        };
                        if start >= end {
                            Ok(DataType::String(String::new()))
                        } else {
                            Ok(DataType::String(chars[start..end].iter().collect()))
                        }
                    }
                    _ => Ok(DataType::String(String::new())),
                }
            }
            OperationType::IndexOf => {
                let search = inputs.get("search").cloned().unwrap_or(DataType::Null);
                match (&input, &search) {
                    (DataType::String(s), DataType::String(sub)) => {
                        Ok(DataType::Int64(s.find(sub.as_str()).map(|i| i as i64).unwrap_or(-1)))
                    }
                    _ => Ok(DataType::Int64(-1)),
                }
            },

            // Map
            OperationType::MapSize => match &map {
                DataType::Map(m) => Ok(DataType::Int64(m.len() as i64)),
                _ => Ok(DataType::Int64(0)),
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
            OperationType::MapEntries => match &map {
                DataType::Map(m) => {
                    Ok(DataType::Array(m.iter().map(|(k, v)| {
                        DataType::Array(vec![DataType::String(k.clone()), v.clone()])
                    }).collect()))
                }
                _ => Ok(DataType::Array(vec![])),
            },
            OperationType::MapFromEntries => match &array {
                DataType::Array(arr) => {
                    let mut m = std::collections::BTreeMap::new();
                    for item in arr {
                        if let DataType::Array(pair) = item {
                            if pair.len() >= 2 {
                                if let DataType::String(k) = &pair[0] {
                                    m.insert(k.clone(), pair[1].clone());
                                }
                            }
                        }
                    }
                    Ok(DataType::Map(m))
                }
                _ => Ok(DataType::Map(std::collections::BTreeMap::new())),
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

            // Array extras
            OperationType::ArrayGet => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match (&array, &index) {
                    (DataType::Array(arr), DataType::Int64(i)) => {
                        if *i < 0 { return Ok(DataType::Null); }
                        Ok(arr.get(*i as usize).cloned().unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::ArraySet => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match (&array, &index) {
                    (DataType::Array(arr), DataType::Int64(i)) => {
                        if *i < 0 { return Ok(DataType::Array(arr.clone())); }
                        let idx = *i as usize;
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
                        if flat.len() > MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!("flatten would exceed {} elements", MAX_ARRAY_ELEMENTS)));
                        }
                    }
                    Ok(DataType::Array(flat))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayConcat => match (&a, &b) {
                (DataType::Array(a), DataType::Array(b)) => {
                    let total = a.len().saturating_add(b.len());
                    if total > MAX_ARRAY_ELEMENTS {
                        return Err(EvalError::InvalidInput(format!("array concat would produce {} elements (max {})", total, MAX_ARRAY_ELEMENTS)));
                    }
                    let mut result = a.clone();
                    result.extend(b.clone());
                    Ok(DataType::Array(result))
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
            OperationType::ArrayFilterNulls => match &array {
                DataType::Array(arr) => {
                    Ok(DataType::Array(arr.iter().filter(|v| !matches!(v, DataType::Null)).cloned().collect()))
                }
                _ => Ok(DataType::Null),
            },

            // Type conversions
            OperationType::ToInt64 => match &input {
                DataType::Int64(_) => Ok(input.clone()),
                DataType::Float64(f) => {
                    if f.is_nan() || f.is_infinite() {
                        Ok(DataType::Null)
                    } else {
                        Ok(DataType::Int64(*f as i64))
                    }
                }
                DataType::String(s) => Ok(s.parse::<i64>().map(DataType::Int64).unwrap_or(DataType::Null)),
                DataType::Bool(b) => Ok(DataType::Int64(if *b { 1 } else { 0 })),
                _ => Ok(DataType::Null),
            },
            OperationType::ToFloat64 => match &input {
                DataType::Float64(_) => Ok(input.clone()),
                DataType::Int64(n) => Ok(DataType::Float64(*n as f64)),
                DataType::String(s) => Ok(s.parse::<f64>().map(DataType::Float64).unwrap_or(DataType::Null)),
                _ => Ok(DataType::Null),
            },
            OperationType::ToBool => match &input {
                DataType::Bool(_) => Ok(input.clone()),
                DataType::Int64(n) => Ok(DataType::Bool(*n != 0)),
                DataType::Float64(f) => Ok(DataType::Bool(*f != 0.0)),
                DataType::String(s) => Ok(DataType::Bool(!s.is_empty())),
                DataType::Null => Ok(DataType::Bool(false)),
                _ => Ok(DataType::Bool(true)),
            },

            // Math
            OperationType::Abs => match &input {
                DataType::Int64(n) => Ok(match n.checked_abs() {
                    Some(v) => DataType::Int64(v),
                    None => DataType::Null, // i64::MIN overflow
                }),
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
            OperationType::Sqrt => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.sqrt())),
                DataType::Int64(n) => Ok(DataType::Float64((*n as f64).sqrt())),
                _ => Ok(DataType::Null),
            },
            OperationType::Power => {
                let a = inputs.get("a").unwrap_or(&DataType::Null);
                let b = inputs.get("b").unwrap_or(&DataType::Null);
                match (a, b) {
                    (DataType::Int64(base), DataType::Int64(exp)) => {
                        if *exp < 0 {
                            if *exp < i32::MIN as i64 || *exp > i32::MAX as i64 {
                                Ok(DataType::Float64(0.0)) // exponent out of i32 range
                            } else {
                                Ok(DataType::Float64((*base as f64).powi(*exp as i32)))
                            }
                        } else if *exp > u32::MAX as i64 {
                            Ok(DataType::Null) // exponent too large
                        } else {
                            match base.checked_pow(*exp as u32) {
                                Some(v) => Ok(DataType::Int64(v)),
                                None => Ok(DataType::Float64((*base as f64).powf(*exp as f64))),
                            }
                        }
                    }
                    (DataType::Float64(base), DataType::Float64(exp)) => Ok(DataType::Float64(base.powf(*exp))),
                    (DataType::Int64(base), DataType::Float64(exp)) => Ok(DataType::Float64((*base as f64).powf(*exp))),
                    (DataType::Float64(base), DataType::Int64(exp)) => {
                        if *exp < i32::MIN as i64 || *exp > i32::MAX as i64 {
                            Ok(DataType::Float64(base.powf(*exp as f64)))
                        } else {
                            Ok(DataType::Float64(base.powi(*exp as i32)))
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::Sin => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.sin())),
                DataType::Int64(n) => Ok(DataType::Float64((*n as f64).sin())),
                _ => Ok(DataType::Null),
            },
            OperationType::Cos => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.cos())),
                DataType::Int64(n) => Ok(DataType::Float64((*n as f64).cos())),
                _ => Ok(DataType::Null),
            },
            OperationType::Tan => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.tan())),
                DataType::Int64(n) => Ok(DataType::Float64((*n as f64).tan())),
                _ => Ok(DataType::Null),
            },
            OperationType::Ln => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.ln())),
                DataType::Int64(n) => Ok(DataType::Float64((*n as f64).ln())),
                _ => Ok(DataType::Null),
            },
            OperationType::Log2 => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.log2())),
                DataType::Int64(n) => Ok(DataType::Float64((*n as f64).log2())),
                _ => Ok(DataType::Null),
            },
            OperationType::Log10 => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.log10())),
                DataType::Int64(n) => Ok(DataType::Float64((*n as f64).log10())),
                _ => Ok(DataType::Null),
            },
            OperationType::Exp => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.exp())),
                DataType::Int64(n) => Ok(DataType::Float64((*n as f64).exp())),
                _ => Ok(DataType::Null),
            },
            OperationType::Sign => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.signum())),
                DataType::Int64(n) => Ok(DataType::Int64(n.signum())),
                _ => Ok(DataType::Null),
            },

            other => Err(EvalError::InvalidInput(format!(
                "operation '{:?}' is not implemented in the standalone evaluator",
                other,
            ))),
        }
    }
}

fn num_binop(
    a: &DataType, b: &DataType,
    int_op: fn(i64, i64) -> Option<i64>,
    float_op: fn(f64, f64) -> f64,
) -> Result<DataType, EvalError> {
    match (a, b) {
        (DataType::Int64(x), DataType::Int64(y)) => match int_op(*x, *y) {
            Some(v) => Ok(DataType::Int64(v)),
            None => Err(EvalError::InvalidInput("integer overflow".to_string())),
        },
        (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(float_op(*x, *y))),
        (DataType::Int64(x), DataType::Float64(y)) => Ok(DataType::Float64(float_op(*x as f64, *y))),
        (DataType::Float64(x), DataType::Int64(y)) => Ok(DataType::Float64(float_op(*x, *y as f64))),
        _ => Ok(DataType::Null),
    }
}

fn num_cmp(
    a: &DataType, b: &DataType,
    int_op: fn(&i64, &i64) -> bool,
    float_op: fn(&f64, &f64) -> bool,
) -> Result<DataType, EvalError> {
    match (a, b) {
        (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Bool(int_op(x, y))),
        (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Bool(float_op(x, y))),
        (DataType::Int64(x), DataType::Float64(y)) => Ok(DataType::Bool(float_op(&(*x as f64), y))),
        (DataType::Float64(x), DataType::Int64(y)) => Ok(DataType::Bool(float_op(x, &(*y as f64)))),
        _ => Ok(DataType::Bool(false)),
    }
}

fn print_usage() {
    eprintln!("MAGI Language v{}", magi_lang::version::version_string());
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  magi run <file.magi>           Interpret a .magi file");
    eprintln!("  magi compile <file.magi>       Compile to .wasm");
    eprintln!("  magi run-wasm <file.wasm>      Run a compiled .wasm file");
    eprintln!("  magi check <file.magi>         Type-check and lint a file");
    eprintln!("  magi lint <file.magi>          Lint a file for style issues");
    eprintln!("  magi fmt <file.magi>           Format a file");
    eprintln!("  magi fmt --write <file.magi>   Format a file in-place");
    eprintln!("  magi fmt --check <file.magi>   Check if a file is formatted");
    eprintln!("  magi lsp                       Start the LSP server");
    eprintln!("  magi version                   Show version info");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "run" => {
            if args.len() < 3 {
                eprintln!("Error: missing file argument");
                eprintln!("Usage: magi run <file.magi>");
                process::exit(1);
            }
            cmd_run(&args[2]);
        }
        "compile" => {
            if args.len() < 3 {
                eprintln!("Error: missing file argument");
                eprintln!("Usage: magi compile <file.magi>");
                process::exit(1);
            }
            cmd_compile(&args[2]);
        }
        "run-wasm" => {
            if args.len() < 3 {
                eprintln!("Error: missing file argument");
                eprintln!("Usage: magi run-wasm <file.wasm>");
                process::exit(1);
            }
            cmd_run_wasm(&args[2]);
        }
        "check" => {
            if args.len() < 3 {
                eprintln!("Error: missing file argument");
                eprintln!("Usage: magi check <file.magi>");
                process::exit(1);
            }
            cmd_check(&args[2]);
        }
        "lint" => {
            if args.len() < 3 {
                eprintln!("Error: missing file argument");
                eprintln!("Usage: magi lint <file.magi>");
                process::exit(1);
            }
            cmd_lint(&args[2]);
        }
        "fmt" => {
            // Parse flags: --write, --check
            let mut write_in_place = false;
            let mut check_only = false;
            let mut file_path = None;

            for arg in &args[2..] {
                match arg.as_str() {
                    "--write" | "-w" => write_in_place = true,
                    "--check" | "-c" => check_only = true,
                    _ => file_path = Some(arg.as_str()),
                }
            }

            if write_in_place && check_only {
                eprintln!("Error: --write and --check are mutually exclusive");
                process::exit(1);
            }

            match file_path {
                Some(path) => cmd_fmt(path, write_in_place, check_only),
                None => {
                    eprintln!("Error: missing file argument");
                    eprintln!("Usage: magi fmt [--write] [--check] <file.magi>");
                    process::exit(1);
                }
            }
        }
        "lsp" => {
            cmd_lsp();
        }
        "version" => {
            println!("MAGI Language v{}", magi_lang::version::version_string());
            let features = magi_lang::version::available_features();
            println!("Features: {}", features.len());
        }
        _ => {
            // If first arg is a .magi file, run it directly.
            if args[1].ends_with(".magi") {
                cmd_run(&args[1]);
            } else {
                eprintln!("Unknown command: {}", args[1]);
                print_usage();
                process::exit(1);
            }
        }
    }
}

/// Resolve package dependencies by reading magi.toml next to the source file.
fn resolve_dependencies(magi_file_path: &std::path::Path) -> Vec<ResolvedPackage> {
    let dir = magi_file_path.parent().unwrap_or(std::path::Path::new("."));
    let toml_path = dir.join("magi.toml");

    let toml_str = match fs::read_to_string(&toml_path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let table: toml::Table = match toml_str.parse() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Warning: failed to parse {}: {}", toml_path.display(), e);
            return Vec::new();
        }
    };

    let deps = match table.get("dependencies").and_then(|d| d.as_table()) {
        Some(d) => d,
        None => return Vec::new(),
    };

    let mut packages = Vec::new();
    for (id, value) in deps {
        let rel_path = match value.as_table().and_then(|t| t.get("path")).and_then(|p| p.as_str()) {
            Some(p) => p,
            None => continue,
        };

        // Security: reject absolute paths and paths with suspicious components
        if std::path::Path::new(rel_path).is_absolute() {
            eprintln!("Warning: dependency '{}' uses an absolute path, skipping", id);
            continue;
        }

        let dep_dir = dir.join(rel_path);
        let source_path = dep_dir.join("source.magi");
        let source = match fs::read_to_string(&source_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Warning: could not read dependency '{}' at {}: {}", id, source_path.display(), e);
                continue;
            }
        };

        match resolve_package_from_source(id, &source) {
            Ok(pkg) => packages.push(pkg),
            Err(e) => {
                eprintln!("Warning: could not parse dependency '{}': {}", id, e);
            }
        }
    }

    packages
}

/// Resolve package dependency sources (raw source strings) for compilation.
fn resolve_dependency_sources(magi_file_path: &std::path::Path) -> Vec<String> {
    let dir = magi_file_path.parent().unwrap_or(std::path::Path::new("."));
    let toml_path = dir.join("magi.toml");

    let toml_str = match fs::read_to_string(&toml_path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let table: toml::Table = match toml_str.parse() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Warning: failed to parse {}: {}", toml_path.display(), e);
            return Vec::new();
        }
    };

    let deps = match table.get("dependencies").and_then(|d| d.as_table()) {
        Some(d) => d,
        None => return Vec::new(),
    };

    let mut sources = Vec::new();
    for (id, value) in deps {
        let rel_path = match value.as_table().and_then(|t| t.get("path")).and_then(|p| p.as_str()) {
            Some(p) => p,
            None => continue,
        };

        // Security: reject absolute paths
        if std::path::Path::new(rel_path).is_absolute() {
            eprintln!("Warning: dependency '{}' uses an absolute path, skipping", id);
            continue;
        }

        let dep_dir = dir.join(rel_path);
        let source_path = dep_dir.join("source.magi");
        match fs::read_to_string(&source_path) {
            Ok(s) => sources.push(s),
            Err(e) => {
                eprintln!("Warning: could not read dependency '{}' at {}: {}", id, source_path.display(), e);
            }
        }
    }

    sources
}

fn cmd_check(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading {}: {}", path, e);
            process::exit(1);
        }
    };

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}:{}: error: {}", path, e.line, e);
            process::exit(1);
        }
    };

    // Type check
    let imports = std::collections::HashSet::new();
    let analysis = magi_lang::syntax::type_checker::check_types(&program, &imports);

    // Lint
    let lint_config = magi_lang::linter::LintConfig::default();
    let lint_result = magi_lang::linter::lint(&program, &lint_config);

    let mut has_errors = false;
    let mut count = 0;

    for d in analysis.diagnostics.iter().chain(lint_result.diagnostics.iter()) {
        let severity = match d.severity {
            DiagnosticSeverity::Error => { has_errors = true; "error" }
            DiagnosticSeverity::Warning => "warning",
            DiagnosticSeverity::Info => "info",
        };
        let code = d.code.as_deref().unwrap_or("");
        eprintln!("{}:{}:{}: {} [{}]: {}", path, d.line, d.column, severity, code, d.message);
        if let Some(ref help) = d.help {
            eprintln!("  help: {}", help);
        }
        count += 1;
    }

    if count == 0 {
        println!("No issues found.");
    } else {
        eprintln!("{} diagnostic(s) emitted.", count);
    }

    if has_errors {
        process::exit(1);
    }
}

fn cmd_lint(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading {}: {}", path, e);
            process::exit(1);
        }
    };

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}:{}: parse error: {}", path, e.line, e);
            process::exit(1);
        }
    };

    let config = magi_lang::linter::LintConfig::default();
    let result = magi_lang::linter::lint(&program, &config);

    if result.diagnostics.is_empty() {
        println!("No lint warnings.");
    } else {
        for d in &result.diagnostics {
            let code = d.code.as_deref().unwrap_or("");
            eprintln!("{}:{}:{}: warning [{}]: {}", path, d.line, d.column, code, d.message);
            if let Some(ref help) = d.help {
                eprintln!("  help: {}", help);
            }
        }
        eprintln!("{} warning(s) emitted.", result.diagnostics.len());
        process::exit(1);
    }
}

fn cmd_fmt(path: &str, write_in_place: bool, check_only: bool) {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading {}: {}", path, e);
            process::exit(1);
        }
    };

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}:{}: parse error: {}", path, e.line, e);
            process::exit(1);
        }
    };

    let config = magi_lang::formatter::FormatConfig::default();
    let formatted = magi_lang::formatter::format_program(&program, &config);

    if check_only {
        if formatted == source {
            println!("{} is formatted.", path);
        } else {
            eprintln!("{} is not formatted.", path);
            process::exit(1);
        }
    } else if write_in_place {
        match fs::write(path, &formatted) {
            Ok(_) => println!("Formatted {}.", path),
            Err(e) => {
                eprintln!("Error writing {}: {}", path, e);
                process::exit(1);
            }
        }
    } else {
        print!("{}", formatted);
    }
}

fn cmd_lsp() {
    tokio::runtime::Runtime::new()
        .expect("failed to create tokio runtime")
        .block_on(magi_lang::lsp::run_server());
}

fn cmd_run(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading {}: {}", path, e);
            process::exit(1);
        }
    };

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}:{}: parse error: {}", path, e.line, e);
            process::exit(1);
        }
    };

    let evaluator = FullEvaluator;
    let file_path = std::path::Path::new(path);
    let packages = resolve_dependencies(file_path);
    let mut interp = Interpreter::new(&evaluator).with_packages(packages);

    match interp.execute(&program) {
        Ok(_) => {}
        Err(e) => {
            // Print any logs collected before the error
            for log in &interp.logs {
                println!("{}", log.message);
            }
            eprintln!("Runtime error: {}", e);
            process::exit(1);
        }
    }

    // Print all output/log messages
    for log in &interp.logs {
        println!("{}", log.message);
    }
}

fn cmd_compile(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading {}: {}", path, e);
            process::exit(1);
        }
    };

    // Resolve dependencies and prepend their source to create a single compilation unit.
    let file_path = std::path::Path::new(path);
    let mut combined_source = String::new();
    let dep_sources = resolve_dependency_sources(file_path);
    for dep_src in &dep_sources {
        combined_source.push_str(dep_src);
        combined_source.push('\n');
    }
    // Strip `use pkg::*` imports from the main source (they're inlined now).
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("use pkg::") {
            continue;
        }
        combined_source.push_str(line);
        combined_source.push('\n');
    }

    let program = match parse_v2(&combined_source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}:{}: parse error: {}", path, e.line, e);
            process::exit(1);
        }
    };

    let wasm_bytes = match compiler::compile_to_wasm(&program) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("Compile error: {}", e);
            process::exit(1);
        }
    };

    let src_path = std::path::Path::new(path);
    let dir = src_path.parent().unwrap_or(std::path::Path::new("."));
    let dist_dir = dir.join("dist");
    if let Err(e) = fs::create_dir_all(&dist_dir) {
        eprintln!("Error creating dist directory: {}", e);
        process::exit(1);
    }

    let stem = src_path.file_stem().unwrap_or_default();
    let out_path = dist_dir.join(format!("{}.wasm", stem.to_string_lossy()));
    match fs::write(&out_path, &wasm_bytes) {
        Ok(_) => {
            println!("Compiled {} -> {} ({} bytes)", path, out_path.display(), wasm_bytes.len());
        }
        Err(e) => {
            eprintln!("Error writing {}: {}", out_path.display(), e);
            process::exit(1);
        }
    }
}

/// Format a tagged WASM value into a human-readable string.
fn format_tagged_value(val: i64, data: &[u8]) -> String {
    let tag = (val >> 56) as u8;
    let payload = val & 0x00FFFFFFFFFFFFFF;
    match tag {
        0 => "null".to_string(),
        1 => format!("{}", payload != 0),
        2 => {
            // Sign-extend from 56 bits.
            let n = if payload & (1 << 55) != 0 {
                payload | !0x00FFFFFFFFFFFFFF
            } else {
                payload
            };
            format!("{}", n)
        }
        3 => "<float64>".to_string(),
        4 => {
            // String: payload is memory offset.
            let offset = payload as usize;
            if offset.checked_add(4).map_or(true, |end| end > data.len()) {
                return format!("<string@{}>", offset);
            }
            let len = u32::from_le_bytes([
                data[offset], data[offset + 1],
                data[offset + 2], data[offset + 3],
            ]) as usize;
            match offset.checked_add(4).and_then(|o| o.checked_add(len)) {
                Some(end) if end <= data.len() => {
                    String::from_utf8_lossy(&data[offset + 4..end]).to_string()
                }
                _ => format!("<string@{}>", offset),
            }
        }
        5 => {
            // Array: payload is memory offset.
            // Layout: [i32 length][i32 capacity][i64 elem0][i64 elem1]...
            const MAX_DISPLAY_ELEMENTS: usize = 10_000;
            let ptr = payload as usize;
            if ptr.checked_add(4).map_or(true, |end| end > data.len()) {
                return format!("<array@{}>", ptr);
            }
            let raw_len = u32::from_le_bytes([
                data[ptr], data[ptr + 1], data[ptr + 2], data[ptr + 3],
            ]) as usize;
            let len = raw_len.min(MAX_DISPLAY_ELEMENTS);
            let mut parts = Vec::with_capacity(len);
            for i in 0..len {
                let elem_offset = match (i.checked_mul(8)).and_then(|o| ptr.checked_add(8)?.checked_add(o)) {
                    Some(o) => o,
                    None => break,
                };
                if elem_offset.checked_add(8).map_or(true, |end| end > data.len()) {
                    break;
                }
                let elem = i64::from_le_bytes([
                    data[elem_offset], data[elem_offset + 1],
                    data[elem_offset + 2], data[elem_offset + 3],
                    data[elem_offset + 4], data[elem_offset + 5],
                    data[elem_offset + 6], data[elem_offset + 7],
                ]);
                parts.push(format_tagged_value(elem, data));
            }
            if raw_len > MAX_DISPLAY_ELEMENTS {
                parts.push(format!("...({} more)", raw_len - MAX_DISPLAY_ELEMENTS));
            }
            format!("[{}]", parts.join(", "))
        }
        6 => {
            // Map: payload is memory offset.
            // Layout: [i32 count][i32 capacity][i64 key0][i64 val0]...
            const MAX_DISPLAY_ENTRIES: usize = 10_000;
            let ptr = payload as usize;
            if ptr.checked_add(4).map_or(true, |end| end > data.len()) {
                return format!("<map@{}>", ptr);
            }
            let raw_count = u32::from_le_bytes([
                data[ptr], data[ptr + 1], data[ptr + 2], data[ptr + 3],
            ]) as usize;
            let count = raw_count.min(MAX_DISPLAY_ENTRIES);
            let mut parts = Vec::with_capacity(count);
            for i in 0..count {
                let key_offset = match (i.checked_mul(16)).and_then(|o| ptr.checked_add(8)?.checked_add(o)) {
                    Some(o) => o,
                    None => break,
                };
                let val_offset = match key_offset.checked_add(8) {
                    Some(o) => o,
                    None => break,
                };
                if val_offset.checked_add(8).map_or(true, |end| end > data.len()) {
                    break;
                }
                let key = i64::from_le_bytes([
                    data[key_offset], data[key_offset + 1],
                    data[key_offset + 2], data[key_offset + 3],
                    data[key_offset + 4], data[key_offset + 5],
                    data[key_offset + 6], data[key_offset + 7],
                ]);
                let value = i64::from_le_bytes([
                    data[val_offset], data[val_offset + 1],
                    data[val_offset + 2], data[val_offset + 3],
                    data[val_offset + 4], data[val_offset + 5],
                    data[val_offset + 6], data[val_offset + 7],
                ]);
                parts.push(format!("{}: {}", format_tagged_value(key, data), format_tagged_value(value, data)));
            }
            if raw_count > MAX_DISPLAY_ENTRIES {
                parts.push(format!("...({} more)", raw_count - MAX_DISPLAY_ENTRIES));
            }
            format!("{{{}}}", parts.join(", "))
        }
        _ => format!("<tagged:{}:{}>", tag, payload),
    }
}

fn cmd_run_wasm(path: &str) {
    let wasm_bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error reading {}: {}", path, e);
            process::exit(1);
        }
    };

    // Validate WASM magic.
    if wasm_bytes.len() < 8 || &wasm_bytes[0..4] != b"\0asm" {
        eprintln!("Error: {} is not a valid WASM file", path);
        process::exit(1);
    }

    let engine = wasmtime::Engine::default();
    let module = match wasmtime::Module::new(&engine, &wasm_bytes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("WASM load error: {:?}", e);
            process::exit(1);
        }
    };

    let mut store = wasmtime::Store::new(&engine, ());
    let mut linker = wasmtime::Linker::new(&engine);

    // Provide host functions that the MAGI runtime expects.
    linker
        .func_wrap("env", "print", |mut caller: wasmtime::Caller<'_, ()>, val: i64| {
            if let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = memory.data(&caller);
                let s = format_tagged_value(val, data);
                println!("{}", s);
            } else {
                println!("<no-memory>");
            }
        })
        .expect("failed to define print");

    linker
        .func_wrap("env", "runtime_call", |_caller: wasmtime::Caller<'_, ()>, _name: i32, _argc: i32| -> i64 {
            // Stub runtime call — return null.
            0i64
        })
        .expect("failed to define runtime_call");

    linker
        .func_wrap("env", "__to_string", |mut caller: wasmtime::Caller<'_, ()>, val: i64| -> i64 {
            let tag = (val >> 56) as u8;
            // If already a string, return as-is.
            if tag == 4 {
                return val;
            }

            let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m,
                None => return 0, // null tagged value
            };
            let heap_global = match caller.get_export("__heap_ptr").and_then(|e| e.into_global()) {
                Some(g) => g,
                None => return 0,
            };

            let formatted = {
                let data = memory.data(&caller);
                format_tagged_value(val, data)
            };
            let bytes = formatted.as_bytes();
            let total = 4 + bytes.len();

            // Read current heap pointer from exported global.
            let ptr = match heap_global.get(&mut caller).i32() {
                Some(v) => v as u32,
                None => return 0,
            };

            // Write string: [u32 len][bytes...]
            let str_offset = ptr as usize;
            {
                let data = memory.data_mut(&mut caller);
                if str_offset + 4 + bytes.len() > data.len() {
                    return 0; // out of memory
                }
                let len_bytes = (bytes.len() as u32).to_le_bytes();
                data[str_offset..str_offset + 4].copy_from_slice(&len_bytes);
                data[str_offset + 4..str_offset + 4 + bytes.len()].copy_from_slice(bytes);
            }

            // Update heap pointer.
            let new_ptr = match ptr.checked_add(total as u32) {
                Some(v) => v,
                None => return 0,
            };
            let _ = heap_global.set(&mut caller, wasmtime::Val::I32(new_ptr as i32));

            // Return tagged string: (STRING_TAG << 56) | offset
            ((4i64) << 56) | (str_offset as i64)
        })
        .expect("failed to define __to_string");

    let instance = match linker.instantiate(&mut store, &module) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("WASM instantiation error: {}", e);
            process::exit(1);
        }
    };

    // Call __main.
    let main_fn = match instance.get_typed_func::<(), i64>(&mut store, "__main") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error: no __main export found: {}", e);
            process::exit(1);
        }
    };

    match main_fn.call(&mut store, ()) {
        Ok(result) => {
            if result != 0 {
                let tag = (result >> 56) as u8;
                let payload = result & 0x00FFFFFFFFFFFFFF;
                match tag {
                    0 => {} // null — no output
                    1 => println!("Result: {}", payload != 0),
                    2 => {
                        let n = if payload & (1 << 55) != 0 {
                            payload | !0x00FFFFFFFFFFFFFF
                        } else {
                            payload
                        };
                        println!("Result: {}", n);
                    }
                    _ => println!("Result: <tagged:{}:{}>", tag, payload),
                }
            }
        }
        Err(e) => {
            eprintln!("WASM execution error: {}", e);
            process::exit(1);
        }
    }
}
