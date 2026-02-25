//! MAGI language CLI — interpret and compile .magi files.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::process;

use magi_lang::compiler;
use magi_lang::eval::{EvalError, OperationEvaluator};
use magi_lang::syntax::interpreter::Interpreter;
use magi_lang::syntax::parser::parse_v2;
use magi_lang::types::{DataType, OperationType};

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

        match op {
            // Arithmetic
            OperationType::Add => num_binop(&a, &b, |x, y| x + y, |x, y| x + y),
            OperationType::Subtract => num_binop(&a, &b, |x, y| x - y, |x, y| x - y),
            OperationType::Multiply => num_binop(&a, &b, |x, y| x * y, |x, y| x * y),
            OperationType::Divide => {
                match (&a, &b) {
                    (DataType::Int64(x), DataType::Int64(y)) => {
                        if *y == 0 { return Err(EvalError::DivisionByZero); }
                        Ok(DataType::Int64(x / y))
                    }
                    _ => num_binop(&a, &b, |x, y| x / y, |x, y| x / y),
                }
            }
            OperationType::Modulo => match (&a, &b) {
                (DataType::Int64(x), DataType::Int64(y)) => {
                    if *y == 0 { return Err(EvalError::DivisionByZero); }
                    Ok(DataType::Int64(x % y))
                }
                (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(x % y)),
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
                DataType::Int64(x) => Ok(DataType::Int64(-x)),
                DataType::Float64(x) => Ok(DataType::Float64(-x)),
                _ => Ok(DataType::Null),
            },

            // String
            OperationType::Concat => match (&a, &b) {
                (DataType::String(x), DataType::String(y)) => Ok(DataType::String(format!("{}{}", x, y))),
                _ => Ok(DataType::String(format!("{}{}", a.to_string_lossy(), b.to_string_lossy()))),
            },
            OperationType::ToString => Ok(DataType::String(input.to_string_lossy())),

            // Map access (used by FieldAccess and Index)
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
                DataType::Map(map) => Ok(DataType::Array(map.keys().map(|k| DataType::String(k.clone())).collect())),
                _ => Ok(DataType::Array(vec![])),
            },
            OperationType::MapValues => match &input {
                DataType::Map(map) => Ok(DataType::Array(map.values().cloned().collect())),
                _ => Ok(DataType::Array(vec![])),
            },

            // Array
            OperationType::ArrayLength => match &input {
                DataType::Array(arr) => Ok(DataType::Int64(arr.len() as i64)),
                _ => Ok(DataType::Int64(0)),
            },
            OperationType::ArrayPush => {
                let mut arr = match &a { DataType::Array(a) => a.clone(), _ => vec![] };
                arr.push(b);
                Ok(DataType::Array(arr))
            }
            OperationType::ArrayPop => match &input {
                DataType::Array(arr) if !arr.is_empty() => Ok(arr.last().cloned().unwrap_or(DataType::Null)),
                _ => Ok(DataType::Null),
            },
            OperationType::ArraySlice => Ok(DataType::Null),
            OperationType::ArraySort => match &input {
                DataType::Array(arr) => {
                    let mut sorted = arr.clone();
                    sorted.sort_by(|a, b| a.to_i64().unwrap_or(0).cmp(&b.to_i64().unwrap_or(0)));
                    Ok(DataType::Array(sorted))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayReverse => match &input {
                DataType::Array(arr) => { let mut r = arr.clone(); r.reverse(); Ok(DataType::Array(r)) }
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

            // String ops
            OperationType::Length => match &input {
                DataType::String(s) => Ok(DataType::Int64(s.chars().count() as i64)),
                _ => Ok(DataType::Int64(0)),
            },
            OperationType::Split => match (&a, &b) {
                (DataType::String(s), DataType::String(sep)) => {
                    Ok(DataType::Array(s.split(sep.as_str()).map(|p| DataType::String(p.to_string())).collect()))
                }
                _ => Ok(DataType::Array(vec![])),
            },
            OperationType::Contains => match (&a, &b) {
                (DataType::String(s), DataType::String(sub)) => Ok(DataType::Bool(s.contains(sub.as_str()))),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::Replace => {
                let c = inputs.get("c").cloned().unwrap_or(DataType::Null);
                match (&a, &b, &c) {
                    (DataType::String(s), DataType::String(from), DataType::String(to)) => {
                        Ok(DataType::String(s.replacen(from.as_str(), to.as_str(), 1)))
                    }
                    _ => Ok(a.clone()),
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
            OperationType::StartsWith => match (&a, &b) {
                (DataType::String(s), DataType::String(prefix)) => Ok(DataType::Bool(s.starts_with(prefix.as_str()))),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::EndsWith => match (&a, &b) {
                (DataType::String(s), DataType::String(suffix)) => Ok(DataType::Bool(s.ends_with(suffix.as_str()))),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::Substring => {
                let c = inputs.get("c").cloned().unwrap_or(DataType::Null);
                match (&a, &b, &c) {
                    (DataType::String(s), DataType::Int64(start), DataType::Int64(end)) => {
                        let chars: Vec<char> = s.chars().collect();
                        let start = (*start).max(0) as usize;
                        let end = (*end).min(chars.len() as i64) as usize;
                        if start <= end {
                            Ok(DataType::String(chars[start..end].iter().collect()))
                        } else {
                            Ok(DataType::String(String::new()))
                        }
                    }
                    _ => Ok(DataType::String(String::new())),
                }
            },
            OperationType::IndexOf => match (&a, &b) {
                (DataType::String(s), DataType::String(sub)) => {
                    Ok(DataType::Int64(s.find(sub.as_str()).map(|i| i as i64).unwrap_or(-1)))
                }
                _ => Ok(DataType::Int64(-1)),
            },

            // Map
            OperationType::MapSize => match &input {
                DataType::Map(m) => Ok(DataType::Int64(m.len() as i64)),
                _ => Ok(DataType::Int64(0)),
            },
            OperationType::MapHas => match (&a, &b) {
                (DataType::Map(m), DataType::String(k)) => Ok(DataType::Bool(m.contains_key(k))),
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::MapDelete => {
                let map_val = inputs.get("map").or(inputs.get("a")).cloned().unwrap_or(DataType::Null);
                let key_val = inputs.get("key").or(inputs.get("b")).cloned().unwrap_or(DataType::Null);
                match (&map_val, &key_val) {
                    (DataType::Map(m), DataType::String(k)) => {
                        let mut new_map = m.clone();
                        new_map.remove(k);
                        Ok(DataType::Map(new_map))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::MapEntries => match &input {
                DataType::Map(m) => {
                    Ok(DataType::Array(m.iter().map(|(k, v)| {
                        DataType::Array(vec![DataType::String(k.clone()), v.clone()])
                    }).collect()))
                }
                _ => Ok(DataType::Array(vec![])),
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
            OperationType::ArrayGet => match (&a, &b) {
                (DataType::Array(arr), DataType::Int64(i)) => {
                    let idx = *i as usize;
                    Ok(arr.get(idx).cloned().unwrap_or(DataType::Null))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArraySet => {
                let c = inputs.get("c").cloned().unwrap_or(DataType::Null);
                match (&a, &b) {
                    (DataType::Array(arr), DataType::Int64(i)) => {
                        let mut new_arr = arr.clone();
                        let idx = *i as usize;
                        if idx < new_arr.len() {
                            new_arr[idx] = c;
                        }
                        Ok(DataType::Array(new_arr))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::ArrayFlatten => match &input {
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
            OperationType::ArrayConcat => match (&a, &b) {
                (DataType::Array(a), DataType::Array(b)) => {
                    let mut result = a.clone();
                    result.extend(b.clone());
                    Ok(DataType::Array(result))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayUnique => match &input {
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
            OperationType::ArrayFilterNulls => match &input {
                DataType::Array(arr) => {
                    Ok(DataType::Array(arr.iter().filter(|v| !matches!(v, DataType::Null)).cloned().collect()))
                }
                _ => Ok(DataType::Null),
            },

            // Type conversions
            OperationType::ToInt64 => match &input {
                DataType::Int64(_) => Ok(input.clone()),
                DataType::Float64(f) => Ok(DataType::Int64(*f as i64)),
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
            OperationType::Sqrt => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.sqrt())),
                DataType::Int64(n) => Ok(DataType::Float64((*n as f64).sqrt())),
                _ => Ok(DataType::Null),
            },

            _ => Ok(DataType::Null),
        }
    }
}

fn num_binop(
    a: &DataType, b: &DataType,
    int_op: fn(i64, i64) -> i64,
    float_op: fn(f64, f64) -> f64,
) -> Result<DataType, EvalError> {
    match (a, b) {
        (DataType::Int64(x), DataType::Int64(y)) => Ok(DataType::Int64(int_op(*x, *y))),
        (DataType::Float64(x), DataType::Float64(y)) => Ok(DataType::Float64(float_op(*x, *y))),
        (DataType::Int64(x), DataType::Float64(y)) => Ok(DataType::Float64(float_op(*x as f64, *y))),
        (DataType::Float64(x), DataType::Int64(y)) => Ok(DataType::Float64(float_op(*x, *y as f64))),
        (DataType::String(x), DataType::String(y)) => Ok(DataType::String(format!("{}{}", x, y))),
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
            eprintln!("Parse error: {}", e);
            process::exit(1);
        }
    };

    let evaluator = FullEvaluator;
    let mut interp = Interpreter::new(&evaluator);

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

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Parse error: {}", e);
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
            // Decode tagged value for printing.
            let tag = (val >> 56) as u8;
            let payload = val & 0x00FFFFFFFFFFFFFF;
            match tag {
                0 => println!("null"),
                1 => println!("{}", payload != 0),
                2 => {
                    // Sign-extend from 56 bits.
                    let n = if payload & (1 << 55) != 0 {
                        payload | !0x00FFFFFFFFFFFFFF
                    } else {
                        payload
                    };
                    println!("{}", n);
                }
                3 => println!("<float64>"),
                4 => {
                    // String: payload is memory offset. Read length-prefixed string data.
                    let offset = payload as usize;
                    if let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let data = memory.data(&caller);
                        if offset + 4 <= data.len() {
                            let len = u32::from_le_bytes([
                                data[offset], data[offset + 1],
                                data[offset + 2], data[offset + 3],
                            ]) as usize;
                            if offset + 4 + len <= data.len() {
                                let s = String::from_utf8_lossy(&data[offset + 4..offset + 4 + len]);
                                println!("{}", s);
                            } else {
                                println!("<string@{}>", offset);
                            }
                        } else {
                            println!("<string@{}>", offset);
                        }
                    } else {
                        println!("<string@{}>", offset);
                    }
                }
                _ => println!("<tagged:{}:{}>", tag, payload),
            }
        })
        .expect("failed to define print");

    linker
        .func_wrap("env", "runtime_call", |_caller: wasmtime::Caller<'_, ()>, _name: i32, _argc: i32| -> i64 {
            // Stub runtime call — return null.
            0i64
        })
        .expect("failed to define runtime_call");

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
