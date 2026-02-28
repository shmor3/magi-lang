//! MAGI language CLI — interpret and compile .magi files.

use std::any::Any;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read as _;
use std::net::IpAddr;
use std::process;
use std::sync::{LazyLock, Mutex};

use rand::Rng;

use magi_lang::compiler;
use magi_lang::eval::{DiagnosticSeverity, EvalError, OperationEvaluator};
use magi_lang::syntax::interpreter::{resolve_package_from_source, Interpreter, ResolvedPackage};
use magi_lang::syntax::parser::parse_v2;
use magi_lang::types::{DataType, OperationType};

/// Maximum output string length (10 MB).
const MAX_STRING_OUTPUT: usize = 10_000_000;

/// Maximum array element count.
const MAX_ARRAY_ELEMENTS: usize = 10_000_000;

/// UTF-8 BOM (byte order mark).
const UTF8_BOM: &str = "\u{FEFF}";

// ---------------------------------------------------------------------------
// Connection registry — global storage for open connections (HTTP clients,
// WebSocket handles, TLS sessions, etc.) keyed by UUID-based connection IDs.
// ---------------------------------------------------------------------------

/// Global connection registry.
static CONNECTIONS: LazyLock<Mutex<HashMap<String, Box<dyn Any + Send>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Store a connection in the global registry.
#[allow(dead_code)]
fn conn_store<T: Send + 'static>(id: &str, conn: T) {
    let mut map = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    map.insert(id.to_string(), Box::new(conn));
}

/// Execute a closure with mutable access to a typed connection.
#[allow(dead_code)]
fn conn_with<T: Send + 'static, R>(
    id: &str,
    f: impl FnOnce(&mut T) -> Result<R, EvalError>,
) -> Result<R, EvalError> {
    let mut map = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let entry = map
        .get_mut(id)
        .ok_or_else(|| EvalError::InvalidInput(format!("Connection not found: {}", id)))?;
    let typed = entry
        .downcast_mut::<T>()
        .ok_or_else(|| EvalError::InvalidInput(format!("Connection type mismatch: {}", id)))?;
    f(typed)
}

/// Remove a connection from the global registry.
#[allow(dead_code)]
fn conn_remove(id: &str) -> Result<(), EvalError> {
    let mut map = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    map.remove(id)
        .ok_or_else(|| EvalError::InvalidInput(format!("Connection not found: {}", id)))?;
    Ok(())
}

/// Generate a UUID-based connection ID with the given prefix.
#[allow(dead_code)]
fn conn_id(prefix: &str) -> String {
    format!("{}:{}", prefix, uuid::Uuid::new_v4())
}

// ---------------------------------------------------------------------------
// SSRF protection helpers
// ---------------------------------------------------------------------------

/// Check whether an IP address is in a private / loopback / link-local /
/// CGNAT range that should be blocked for outbound requests.
#[allow(dead_code)]
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()                           // 127.0.0.0/8
                || v4.is_private()                     // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local()                  // 169.254/16
                || v4.octets()[0] == 0                 // 0.0.0.0/8
                || v4.is_broadcast()                   // 255.255.255.255
                || v4.is_multicast()                   // 224/4
                || (v4.octets()[0] == 100              // CGNAT 100.64/10
                    && (v4.octets()[1] & 0xC0) == 64)
                || (v4.octets()[0] == 198              // benchmarking 198.18/15
                    && (v4.octets()[1] & 0xFE) == 18)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_multicast() || {
                let seg = v6.segments();
                // link-local fe80::/10
                (seg[0] & 0xFFC0) == 0xFE80
                    // unique local fc00::/7
                    || (seg[0] & 0xFE00) == 0xFC00
                    // ::ffff:0:0/96 mapped IPv4 — check inner
                    || (seg[0] == 0
                        && seg[1] == 0
                        && seg[2] == 0
                        && seg[3] == 0
                        && seg[4] == 0
                        && seg[5] == 0xFFFF
                        && is_blocked_ip(IpAddr::V4(std::net::Ipv4Addr::new(
                            (seg[6] >> 8) as u8,
                            seg[6] as u8,
                            (seg[7] >> 8) as u8,
                            seg[7] as u8,
                        ))))
            }
        }
    }
}

/// Validate that a URL uses an allowed scheme (http/https/ws/wss) and does
/// not target a blocked host.
#[allow(dead_code)]
fn validate_url(url_str: &str) -> Result<(), EvalError> {
    let parsed = url::Url::parse(url_str)
        .map_err(|e| EvalError::InvalidInput(format!("Invalid URL: {}", e)))?;

    match parsed.scheme() {
        "http" | "https" | "ws" | "wss" => {}
        scheme => return Err(EvalError::InvalidInput(
            format!("URL scheme must be http, https, ws, or wss, got: {}", scheme)
        )),
    }

    let host = parsed.host_str()
        .ok_or_else(|| EvalError::InvalidInput("URL has empty host".to_string()))?;

    validate_host(host)
}

/// Validate that a hostname is not a blocked internal name.
#[allow(dead_code)]
fn validate_host(host: &str) -> Result<(), EvalError> {
    let lower = host.to_ascii_lowercase();

    // Block well-known internal hostnames.
    if lower == "localhost"
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
        || lower == "metadata.google.internal"
        || lower == "[::1]"
    {
        return Err(EvalError::InvalidInput(format!(
            "Blocked host: {}",
            host
        )));
    }

    // If the host parses as an IP, apply IP-level checks.
    let ip_str = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = ip_str.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(EvalError::InvalidInput(format!(
                "Blocked IP address: {}",
                ip
            )));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Utility helpers for FullEvaluator operation implementations
// ---------------------------------------------------------------------------

/// Extract a port number from an input map.
#[allow(dead_code)]
fn get_port(inputs: &HashMap<String, DataType>, key: &str) -> Result<u16, EvalError> {
    match inputs.get(key) {
        Some(DataType::Int64(n)) => {
            let n = *n;
            if (1..=65535).contains(&n) {
                Ok(n as u16)
            } else {
                Err(EvalError::InvalidInput(format!(
                    "Port out of range (1-65535): {}",
                    n
                )))
            }
        }
        Some(DataType::Int32(n)) => {
            let n = *n as i64;
            if (1..=65535).contains(&n) {
                Ok(n as u16)
            } else {
                Err(EvalError::InvalidInput(format!(
                    "Port out of range (1-65535): {}",
                    n
                )))
            }
        }
        Some(DataType::Uint32(n)) => {
            let n = *n as u64;
            if (1..=65535).contains(&n) {
                Ok(n as u16)
            } else {
                Err(EvalError::InvalidInput(format!(
                    "Port out of range (1-65535): {}",
                    n
                )))
            }
        }
        Some(DataType::Uint64(n)) => {
            let n = *n;
            if (1..=65535).contains(&n) {
                Ok(n as u16)
            } else {
                Err(EvalError::InvalidInput(format!(
                    "Port out of range (1-65535): {}",
                    n
                )))
            }
        }
        Some(other) => Err(EvalError::InvalidInput(format!(
            "Expected numeric port for '{}', got: {:?}",
            key, other
        ))),
        None => Err(EvalError::InvalidInput(format!(
            "Missing required input: {}",
            key
        ))),
    }
}

/// Extract a port number from an input map, allowing port 0 (OS-assigned).
#[allow(dead_code)]
fn get_bind_port(inputs: &HashMap<String, DataType>, key: &str) -> Result<u16, EvalError> {
    match inputs.get(key) {
        Some(DataType::Int64(n)) => {
            let n = *n;
            if (0..=65535).contains(&n) {
                Ok(n as u16)
            } else {
                Err(EvalError::InvalidInput(format!(
                    "Port out of range (0-65535): {}",
                    n
                )))
            }
        }
        Some(DataType::Int32(n)) => {
            let n = *n as i64;
            if (0..=65535).contains(&n) {
                Ok(n as u16)
            } else {
                Err(EvalError::InvalidInput(format!(
                    "Port out of range (0-65535): {}",
                    n
                )))
            }
        }
        Some(DataType::Uint32(n)) => {
            let n = *n as u64;
            if n <= 65535 {
                Ok(n as u16)
            } else {
                Err(EvalError::InvalidInput(format!(
                    "Port out of range (0-65535): {}",
                    n
                )))
            }
        }
        Some(DataType::Uint64(n)) => {
            let n = *n;
            if n <= 65535 {
                Ok(n as u16)
            } else {
                Err(EvalError::InvalidInput(format!(
                    "Port out of range (0-65535): {}",
                    n
                )))
            }
        }
        Some(other) => Err(EvalError::InvalidInput(format!(
            "Expected numeric port for '{}', got: {:?}",
            key, other
        ))),
        None => Err(EvalError::InvalidInput(format!(
            "Missing required input: {}",
            key
        ))),
    }
}

/// Extract a string reference from an input map.
#[allow(dead_code)]
fn get_string<'a>(inputs: &'a HashMap<String, DataType>, key: &str) -> Result<&'a str, EvalError> {
    match inputs.get(key) {
        Some(DataType::String(s)) => Ok(s.as_str()),
        Some(other) => Err(EvalError::InvalidInput(format!(
            "Expected string for '{}', got: {:?}",
            key, other
        ))),
        None => Err(EvalError::InvalidInput(format!(
            "Missing required input: {}",
            key
        ))),
    }
}

/// Convert a `DataType` value to a byte vector.
#[allow(dead_code)]
fn data_to_bytes(data: &DataType) -> Vec<u8> {
    match data {
        DataType::Bytes(b) => b.clone(),
        DataType::String(s) => s.as_bytes().to_vec(),
        other => other.to_string().into_bytes(),
    }
}

/// Read a .magi source file, stripping BOM and validating the contents.
/// Prints an error message and exits with code 1 on failure.
fn read_source(path: &str) -> String {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read '{}': {}", path, e);
            process::exit(1);
        }
    };
    // Strip UTF-8 BOM if present (common with Windows-edited files).
    let source = if let Some(stripped) = source.strip_prefix(UTF8_BOM) {
        stripped.to_string()
    } else {
        source
    };
    // Reject files that contain null bytes (likely binary).
    if source.contains('\0') {
        eprintln!("error: '{}' appears to be a binary file", path);
        process::exit(1);
    }
    source
}

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
                match (promote_numeric(&a), promote_numeric(&b)) {
                    (Some(Ok(x)), Some(Ok(y))) => {
                        if y == 0 { return Err(EvalError::DivisionByZero); }
                        match x.checked_div(y) {
                            Some(v) => Ok(DataType::Int64(v)),
                            None => Err(EvalError::InvalidInput("integer overflow".to_string())),
                        }
                    }
                    (Some(av), Some(bv)) => {
                        let fb = match bv { Ok(i) => i as f64, Err(f) => f };
                        if fb == 0.0 { return Err(EvalError::DivisionByZero); }
                        let fa = match av { Ok(i) => i as f64, Err(f) => f };
                        Ok(DataType::Float64(fa / fb))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::Modulo => {
                match (promote_numeric(&a), promote_numeric(&b)) {
                    (Some(Ok(x)), Some(Ok(y))) => {
                        if y == 0 { return Err(EvalError::DivisionByZero); }
                        match x.checked_rem(y) {
                            Some(v) => Ok(DataType::Int64(v)),
                            None => Err(EvalError::InvalidInput("integer overflow".to_string())),
                        }
                    }
                    (Some(av), Some(bv)) => {
                        let fb = match bv { Ok(i) => i as f64, Err(f) => f };
                        if fb == 0.0 { return Err(EvalError::DivisionByZero); }
                        let fa = match av { Ok(i) => i as f64, Err(f) => f };
                        Ok(DataType::Float64(fa % fb))
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // Comparison
            OperationType::Equal => {
                if a == b { return Ok(DataType::Bool(true)); }
                // Cross-type numeric equality (e.g. Float32(1.0) == Float64(1.0))
                match (promote_numeric(&a), promote_numeric(&b)) {
                    (Some(av), Some(bv)) => {
                        let fa = match av { Ok(i) => i as f64, Err(f) => f };
                        let fb = match bv { Ok(i) => i as f64, Err(f) => f };
                        Ok(DataType::Bool(fa == fb))
                    }
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::NotEqual => {
                if a == b { return Ok(DataType::Bool(false)); }
                match (promote_numeric(&a), promote_numeric(&b)) {
                    (Some(av), Some(bv)) => {
                        let fa = match av { Ok(i) => i as f64, Err(f) => f };
                        let fb = match bv { Ok(i) => i as f64, Err(f) => f };
                        Ok(DataType::Bool(fa != fb))
                    }
                    _ => Ok(DataType::Bool(true)),
                }
            }
            OperationType::Greater => num_cmp(&a, &b, |x, y| x > y, |x, y| x > y, |x, y| x > y),
            OperationType::Less => num_cmp(&a, &b, |x, y| x < y, |x, y| x < y, |x, y| x < y),
            OperationType::GreaterEq => num_cmp(&a, &b, |x, y| x >= y, |x, y| x >= y, |x, y| x >= y),
            OperationType::LessEq => num_cmp(&a, &b, |x, y| x <= y, |x, y| x <= y, |x, y| x <= y),

            // Logical
            OperationType::And => {
                let ta = is_truthy(&a);
                let tb = is_truthy(&b);
                Ok(DataType::Bool(ta && tb))
            },
            OperationType::Or => {
                let ta = is_truthy(&a);
                let tb = is_truthy(&b);
                Ok(DataType::Bool(ta || tb))
            },
            OperationType::Not => {
                let truthy = match &input {
                    DataType::Bool(b) => *b,
                    DataType::Int64(n) => *n != 0,
                    DataType::Float64(f) => *f != 0.0 && !f.is_nan(),
                    DataType::String(s) => !s.is_empty(),
                    DataType::Null => false,
                    DataType::Array(a) => !a.is_empty(),
                    DataType::Map(m) => !m.is_empty(),
                    _ => true,
                };
                Ok(DataType::Bool(!truthy))
            },
            OperationType::Negate => match &input {
                DataType::Int64(x) => match x.checked_neg() {
                    Some(v) => Ok(DataType::Int64(v)),
                    None => Err(EvalError::InvalidInput("integer overflow".to_string())),
                },
                DataType::Int32(x) => match x.checked_neg() {
                    Some(v) => Ok(DataType::Int32(v)),
                    None => Err(EvalError::InvalidInput("integer overflow".to_string())),
                },
                DataType::Float64(x) => Ok(DataType::Float64(-x)),
                DataType::Float32(x) => Ok(DataType::Float32(-x)),
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
                        // Try numeric comparison first (handles cross-type: Int32, Int64, Float32, Float64, etc.)
                        if let (Some(pa), Some(pb)) = (promote_numeric(a), promote_numeric(b)) {
                            let fa = match pa { Ok(i) => i as f64, Err(f) => f };
                            let fb = match pb { Ok(i) => i as f64, Err(f) => f };
                            return fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal);
                        }
                        match (a, b) {
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
                (DataType::Array(arr), val) => {
                    let found = arr.iter().any(|item| {
                        if item == val { return true; }
                        // Cross-type numeric equality
                        match (promote_numeric(item), promote_numeric(val)) {
                            (Some(av), Some(bv)) => {
                                let fa = match av { Ok(i) => i as f64, Err(f) => f };
                                let fb = match bv { Ok(i) => i as f64, Err(f) => f };
                                fa == fb
                            }
                            _ => false,
                        }
                    });
                    Ok(DataType::Bool(found))
                }
                _ => Ok(DataType::Bool(false)),
            },
            OperationType::ArrayJoin => match &array {
                DataType::Array(arr) => {
                    let sep = match inputs.get("delimiter").or(inputs.get("separator")).or(inputs.get("input_1")) {
                        Some(DataType::String(s)) => s.clone(),
                        _ => ",".to_string(),
                    };
                    let s: Vec<String> = arr.iter().map(|v| v.to_string_lossy()).collect();
                    let estimated_len: usize = s.iter().map(|p| p.len()).sum::<usize>() + s.len().saturating_sub(1) * sep.len();
                    if estimated_len > MAX_STRING_OUTPUT {
                        return Err(EvalError::InvalidInput(format!("join result exceeds {} byte limit", MAX_STRING_OUTPUT)));
                    }
                    Ok(DataType::String(s.join(&sep)))
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
                        if from.is_empty() {
                            // Empty search: inserts between every char + start/end
                            let result_len = (s.chars().count() + 1).saturating_mul(to.len()).saturating_add(s.len());
                            if result_len > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!("replace result exceeds {} byte limit", MAX_STRING_OUTPUT)));
                            }
                        } else if to.len() > from.len() {
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
                        let start = match start_val.to_i64() {
                            Some(n) => {
                                if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                            }
                            None => 0,
                        };
                        let end = match end_val.as_ref().and_then(|v| v.to_i64()) {
                            Some(n) => {
                                if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                            }
                            None => chars.len(),
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
                        Ok(DataType::Int64(s.find(sub.as_str()).map(|byte_idx| {
                            s[..byte_idx].chars().count() as i64
                        }).unwrap_or(-1)))
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
                        let already = seen.iter().any(|s: &DataType| {
                            if s == item { return true; }
                            match (promote_numeric(s), promote_numeric(item)) {
                                (Some(av), Some(bv)) => {
                                    let fa = match av { Ok(i) => i as f64, Err(f) => f };
                                    let fb = match bv { Ok(i) => i as f64, Err(f) => f };
                                    fa == fb
                                }
                                _ => false,
                            }
                        });
                        if !already {
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
                DataType::Int32(n) => Ok(DataType::Int64(*n as i64)),
                DataType::Uint32(n) => Ok(DataType::Int64(*n as i64)),
                DataType::Uint64(n) => {
                    if *n > i64::MAX as u64 { Ok(DataType::Null) }
                    else { Ok(DataType::Int64(*n as i64)) }
                }
                DataType::Float32(f) => {
                    let f = *f as f64;
                    if f.is_nan() || f.is_infinite() || f < (i64::MIN as f64) || f >= (i64::MAX as f64 + 1.0) { Ok(DataType::Null) }
                    else { Ok(DataType::Int64(f as i64)) }
                }
                DataType::Float64(f) => {
                    if f.is_nan() || f.is_infinite() || *f < (i64::MIN as f64) || *f >= (i64::MAX as f64 + 1.0) {
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
                DataType::Int32(n) => Ok(DataType::Float64(*n as f64)),
                DataType::Uint32(n) => Ok(DataType::Float64(*n as f64)),
                DataType::Uint64(n) => Ok(DataType::Float64(*n as f64)),
                DataType::Float32(f) => Ok(DataType::Float64(*f as f64)),
                DataType::String(s) => Ok(s.parse::<f64>().map(DataType::Float64).unwrap_or(DataType::Null)),
                DataType::Bool(b) => Ok(DataType::Float64(if *b { 1.0 } else { 0.0 })),
                _ => Ok(DataType::Null),
            },
            OperationType::ToBool => match &input {
                DataType::Bool(_) => Ok(input.clone()),
                DataType::Int64(n) => Ok(DataType::Bool(*n != 0)),
                DataType::Int32(n) => Ok(DataType::Bool(*n != 0)),
                DataType::Uint32(n) => Ok(DataType::Bool(*n != 0)),
                DataType::Uint64(n) => Ok(DataType::Bool(*n != 0)),
                DataType::Float64(f) => Ok(DataType::Bool(*f != 0.0 && !f.is_nan())),
                DataType::Float32(f) => Ok(DataType::Bool(*f != 0.0 && !f.is_nan())),
                DataType::String(s) => Ok(DataType::Bool(!s.is_empty())),
                DataType::Null => Ok(DataType::Bool(false)),
                DataType::Array(a) => Ok(DataType::Bool(!a.is_empty())),
                DataType::Map(m) => Ok(DataType::Bool(!m.is_empty())),
                _ => Ok(DataType::Bool(true)),
            },

            // Math
            OperationType::Abs => match &input {
                DataType::Int64(n) => Ok(match n.checked_abs() {
                    Some(v) => DataType::Int64(v),
                    None => DataType::Null,
                }),
                DataType::Int32(n) => Ok(match n.checked_abs() {
                    Some(v) => DataType::Int32(v),
                    None => DataType::Null,
                }),
                DataType::Uint32(_) | DataType::Uint64(_) => Ok(input.clone()),
                DataType::Float64(f) => Ok(DataType::Float64(f.abs())),
                DataType::Float32(f) => Ok(DataType::Float32(f.abs())),
                _ => Ok(DataType::Null),
            },
            OperationType::Round => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.round())),
                DataType::Float32(n) => Ok(DataType::Float32(n.round())),
                other => Ok(other.clone()),
            },
            OperationType::Floor => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.floor())),
                DataType::Float32(n) => Ok(DataType::Float32(n.floor())),
                other => Ok(other.clone()),
            },
            OperationType::Ceil => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.ceil())),
                DataType::Float32(n) => Ok(DataType::Float32(n.ceil())),
                other => Ok(other.clone()),
            },
            OperationType::Sqrt => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.sqrt()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).sqrt())),
                    Some(Err(f)) => Ok(DataType::Float64(f.sqrt())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Power => {
                let a = inputs.get("a").unwrap_or(&DataType::Null);
                let b = inputs.get("b").unwrap_or(&DataType::Null);
                match (promote_numeric(a), promote_numeric(b)) {
                    (Some(Ok(base)), Some(Ok(exp))) => {
                        if exp < 0 {
                            Ok(DataType::Float64((base as f64).powf(exp as f64)))
                        } else if exp > u32::MAX as i64 {
                            Ok(DataType::Null)
                        } else {
                            match base.checked_pow(exp as u32) {
                                Some(v) => Ok(DataType::Int64(v)),
                                None => Ok(DataType::Float64((base as f64).powf(exp as f64))),
                            }
                        }
                    }
                    (Some(Ok(base)), Some(Err(exp))) => Ok(DataType::Float64((base as f64).powf(exp))),
                    (Some(Err(base)), Some(Ok(exp))) => {
                        if exp < i32::MIN as i64 || exp > i32::MAX as i64 {
                            Ok(DataType::Float64(base.powf(exp as f64)))
                        } else {
                            Ok(DataType::Float64(base.powi(exp as i32)))
                        }
                    }
                    (Some(Err(base)), Some(Err(exp))) => Ok(DataType::Float64(base.powf(exp))),
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::Sin => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.sin()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).sin())),
                    Some(Err(f)) => Ok(DataType::Float64(f.sin())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Cos => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.cos()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).cos())),
                    Some(Err(f)) => Ok(DataType::Float64(f.cos())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Tan => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.tan()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).tan())),
                    Some(Err(f)) => Ok(DataType::Float64(f.tan())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Ln => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.ln()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).ln())),
                    Some(Err(f)) => Ok(DataType::Float64(f.ln())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Log2 => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.log2()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).log2())),
                    Some(Err(f)) => Ok(DataType::Float64(f.log2())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Log10 => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.log10()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).log10())),
                    Some(Err(f)) => Ok(DataType::Float64(f.log10())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Exp => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.exp()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).exp())),
                    Some(Err(f)) => Ok(DataType::Float64(f.exp())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Sign => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.signum()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Int64(n.signum())),
                    Some(Err(f)) => Ok(DataType::Float64(f.signum())),
                    None => Ok(DataType::Null),
                }
            },

            // Array mutation operations
            OperationType::ArrayShift => match &array {
                DataType::Array(arr) => Ok(arr.first().cloned().unwrap_or(DataType::Null)),
                _ => Ok(DataType::Null),
            },
            OperationType::ArrayInsert => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match &array {
                    DataType::Array(arr) => {
                        if arr.len() >= MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!("array exceeds maximum size ({})", MAX_ARRAY_ELEMENTS)));
                        }
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

            // String methods
            OperationType::StringChars => match &input {
                DataType::String(s) => {
                    if s.chars().count() > MAX_ARRAY_ELEMENTS {
                        return Err(EvalError::InvalidInput(format!("chars() would produce {} elements (max {})", s.chars().count(), MAX_ARRAY_ELEMENTS)));
                    }
                    Ok(DataType::Array(s.chars().map(|c| DataType::String(c.to_string())).collect()))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::StringRepeat => {
                let count = inputs.get("count").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Int64(0));
                match &input {
                    DataType::String(s) => {
                        let n = count.to_i64().unwrap_or(0).max(0) as usize;
                        let result_len = s.len().saturating_mul(n);
                        if result_len > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!("repeat result exceeds {} byte limit", MAX_STRING_OUTPUT)));
                        }
                        Ok(DataType::String(s.repeat(n)))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::StringLines => match &input {
                DataType::String(s) => {
                    let lines: Vec<DataType> = s.lines().take(MAX_ARRAY_ELEMENTS + 1).map(|l| DataType::String(l.to_string())).collect();
                    if lines.len() > MAX_ARRAY_ELEMENTS {
                        return Err(EvalError::InvalidInput(format!("lines() would produce more than {} elements", MAX_ARRAY_ELEMENTS)));
                    }
                    Ok(DataType::Array(lines))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::StringWords => match &input {
                DataType::String(s) => {
                    let words: Vec<DataType> = s.split_whitespace().take(MAX_ARRAY_ELEMENTS + 1).map(|w| DataType::String(w.to_string())).collect();
                    if words.len() > MAX_ARRAY_ELEMENTS {
                        return Err(EvalError::InvalidInput(format!("words() would produce more than {} elements", MAX_ARRAY_ELEMENTS)));
                    }
                    Ok(DataType::Array(words))
                }
                _ => Ok(DataType::Null),
            },
            OperationType::StringReverse => match &input {
                DataType::String(s) => Ok(DataType::String(s.chars().rev().collect())),
                _ => Ok(DataType::Null),
            },
            OperationType::StringCount => {
                let search = inputs.get("search").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                match (&input, &search) {
                    (DataType::String(s), DataType::String(sub)) => {
                        Ok(DataType::Int64(s.matches(sub.as_str()).count() as i64))
                    }
                    _ => Ok(DataType::Int64(0)),
                }
            },
            OperationType::CharAt => {
                let index = inputs.get("index").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Int64(0));
                match &input {
                    DataType::String(s) => {
                        let i = index.to_i64().unwrap_or(-1);
                        if i < 0 { return Ok(DataType::Null); }
                        Ok(s.chars().nth(i as usize).map(|c| DataType::String(c.to_string())).unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::PadStart => {
                let width = inputs.get("width").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Int64(0));
                let fill = inputs.get("fill").or(inputs.get("input_2")).cloned();
                match &input {
                    DataType::String(s) => {
                        let w = width.to_i64().unwrap_or(0).max(0) as usize;
                        let pad_str = match &fill {
                            Some(DataType::String(f)) if !f.is_empty() => f.clone(),
                            _ => " ".to_string(),
                        };
                        let char_count = s.chars().count();
                        if char_count >= w {
                            Ok(DataType::String(s.clone()))
                        } else {
                            let pad_chars = w - char_count;
                            // Check estimated byte size (pad chars * max bytes per fill char)
                            let max_pad_bytes = pad_chars.saturating_mul(pad_str.len());
                            if s.len().saturating_add(max_pad_bytes) > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!("pad_start result exceeds {} byte limit", MAX_STRING_OUTPUT)));
                            }
                            let padding: String = pad_str.chars().cycle().take(pad_chars).collect();
                            Ok(DataType::String(format!("{}{}", padding, s)))
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::PadEnd => {
                let width = inputs.get("width").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Int64(0));
                let fill = inputs.get("fill").or(inputs.get("input_2")).cloned();
                match &input {
                    DataType::String(s) => {
                        let w = width.to_i64().unwrap_or(0).max(0) as usize;
                        let pad_str = match &fill {
                            Some(DataType::String(f)) if !f.is_empty() => f.clone(),
                            _ => " ".to_string(),
                        };
                        let char_count = s.chars().count();
                        if char_count >= w {
                            Ok(DataType::String(s.clone()))
                        } else {
                            let pad_chars = w - char_count;
                            let max_pad_bytes = pad_chars.saturating_mul(pad_str.len());
                            if s.len().saturating_add(max_pad_bytes) > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!("pad_end result exceeds {} byte limit", MAX_STRING_OUTPUT)));
                            }
                            let padding: String = pad_str.chars().cycle().take(pad_chars).collect();
                            Ok(DataType::String(format!("{}{}", s, padding)))
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            },

            // Type inspection
            OperationType::Typeof => {
                let type_name = match &input {
                    DataType::Null => "null",
                    DataType::Bool(_) => "bool",
                    DataType::Int32(_) => "int32",
                    DataType::Int64(_) => "int64",
                    DataType::Uint32(_) => "uint32",
                    DataType::Uint64(_) => "uint64",
                    DataType::Float32(_) => "float32",
                    DataType::Float64(_) => "float64",
                    DataType::String(_) => "string",
                    DataType::Array(_) => "array",
                    DataType::Map(m) => {
                        if m.contains_key("__enum") { "enum" }
                        else if m.contains_key("__struct") { "struct" }
                        else { "map" }
                    }
                    DataType::Bytes(_) => "bytes",
                    DataType::Future(_) => "future",
                };
                Ok(DataType::String(type_name.to_string()))
            },

            // Min/Max
            OperationType::Min => {
                match (promote_numeric(&a), promote_numeric(&b)) {
                    (Some(Ok(x)), Some(Ok(y))) => Ok(DataType::Int64(x.min(y))),
                    (Some(pa), Some(pb)) => {
                        let fa = match pa { Ok(i) => i as f64, Err(f) => f };
                        let fb = match pb { Ok(i) => i as f64, Err(f) => f };
                        Ok(DataType::Float64(fa.min(fb)))
                    }
                    _ => Ok(DataType::Null),
                }
            },
            OperationType::Max => {
                match (promote_numeric(&a), promote_numeric(&b)) {
                    (Some(Ok(x)), Some(Ok(y))) => Ok(DataType::Int64(x.max(y))),
                    (Some(pa), Some(pb)) => {
                        let fa = match pa { Ok(i) => i as f64, Err(f) => f };
                        let fb = match pb { Ok(i) => i as f64, Err(f) => f };
                        Ok(DataType::Float64(fa.max(fb)))
                    }
                    _ => Ok(DataType::Null),
                }
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
                    if result.len() >= MAX_ARRAY_ELEMENTS {
                        return Err(EvalError::InvalidInput(format!("range would produce more than {} elements", MAX_ARRAY_ELEMENTS)));
                    }
                    result.push(DataType::Int64(i));
                    i = match i.checked_add(step) {
                        Some(v) => v,
                        None => break,
                    };
                }
                Ok(DataType::Array(result))
            },

            // ToJson
            OperationType::ToJson => {
                let json_val = datatype_to_serde_json(&input);
                Ok(DataType::String(serde_json::to_string(&json_val).unwrap_or_else(|_| "null".to_string())))
            },

            // Bitwise operations
            OperationType::BitAnd => match (a.to_i64(), b.to_i64()) {
                (Some(x), Some(y)) => Ok(DataType::Int64(x & y)),
                _ => Ok(DataType::Null),
            },
            OperationType::BitOr => match (a.to_i64(), b.to_i64()) {
                (Some(x), Some(y)) => Ok(DataType::Int64(x | y)),
                _ => Ok(DataType::Null),
            },
            OperationType::BitXor => match (a.to_i64(), b.to_i64()) {
                (Some(x), Some(y)) => Ok(DataType::Int64(x ^ y)),
                _ => Ok(DataType::Null),
            },
            OperationType::BitNot => match input.to_i64() {
                Some(x) => Ok(DataType::Int64(!x)),
                None => Ok(DataType::Null),
            },
            OperationType::BitShiftLeft => match (a.to_i64(), b.to_i64()) {
                (Some(x), Some(y)) if (0..64).contains(&y) => Ok(DataType::Int64(x << y)),
                _ => Ok(DataType::Null),
            },
            OperationType::BitShiftRight => match (a.to_i64(), b.to_i64()) {
                (Some(x), Some(y)) if (0..64).contains(&y) => Ok(DataType::Int64(x >> y)),
                _ => Ok(DataType::Null),
            },

            // Type checking predicates
            OperationType::IsNull => Ok(DataType::Bool(matches!(&input, DataType::Null))),
            OperationType::IsString => Ok(DataType::Bool(matches!(&input, DataType::String(_)))),
            OperationType::IsNumber => Ok(DataType::Bool(promote_numeric(&input).is_some())),
            OperationType::IsArray => Ok(DataType::Bool(matches!(&input, DataType::Array(_)))),
            OperationType::IsMap => Ok(DataType::Bool(matches!(&input, DataType::Map(_)))),
            OperationType::IsBool => Ok(DataType::Bool(matches!(&input, DataType::Bool(_)))),
            OperationType::IsBytes => Ok(DataType::Bool(matches!(&input, DataType::Bytes(_)))),

            // Assert / DebugLog
            OperationType::Assert => {
                match &input {
                    DataType::Bool(true) => Ok(DataType::Null),
                    DataType::Bool(false) => Err(EvalError::InvalidInput("Assertion failed".to_string())),
                    _ => Err(EvalError::InvalidInput(format!("Assert expects bool, got {:?}", input))),
                }
            },
            OperationType::DebugLog => {
                eprintln!("[debug] {}", input.to_string_lossy());
                Ok(DataType::Null)
            },

            // Bytes operations
            OperationType::BytesLength => {
                match &input {
                    DataType::Bytes(b) => Ok(DataType::Int64(b.len() as i64)),
                    _ => Err(EvalError::InvalidInput(format!("BytesLength expects Bytes, got {:?}", input))),
                }
            },
            OperationType::BytesSlice => {
                match &input {
                    DataType::Bytes(b) => {
                        let len = b.len() as i64;
                        let raw_start = inputs.get("input_1").or(inputs.get("start")).and_then(|v| v.to_i64()).unwrap_or(0);
                        let raw_end = inputs.get("input_2").or(inputs.get("end")).and_then(|v| v.to_i64()).unwrap_or(len);
                        let start = if raw_start < 0 { (len + raw_start).max(0) as usize } else { (raw_start as usize).min(b.len()) };
                        let end = if raw_end < 0 { (len + raw_end).max(0) as usize } else { (raw_end as usize).min(b.len()) };
                        if start > end {
                            Ok(DataType::Bytes(vec![]))
                        } else {
                            Ok(DataType::Bytes(b[start..end].to_vec()))
                        }
                    }
                    _ => Err(EvalError::InvalidInput(format!("BytesSlice expects Bytes, got {:?}", input))),
                }
            },
            OperationType::BytesConcat => {
                let a_val = inputs.get("a").cloned().unwrap_or(DataType::Null);
                let b_val = inputs.get("b").cloned().unwrap_or(DataType::Null);
                match (&a_val, &b_val) {
                    (DataType::Bytes(a), DataType::Bytes(b)) => {
                        let mut result = a.clone();
                        result.extend_from_slice(b);
                        Ok(DataType::Bytes(result))
                    }
                    _ => Err(EvalError::InvalidInput("BytesConcat expects two Bytes arguments".to_string())),
                }
            },
            OperationType::BytesContains => {
                let search = inputs.get("search").cloned().unwrap_or(DataType::Null);
                match (&input, &search) {
                    (DataType::Bytes(haystack), DataType::Bytes(needle)) => {
                        if needle.is_empty() {
                            return Ok(DataType::Bool(true));
                        }
                        let found = haystack.windows(needle.len()).any(|w| w == needle.as_slice());
                        Ok(DataType::Bool(found))
                    }
                    _ => Err(EvalError::InvalidInput("BytesContains expects Bytes input and search".to_string())),
                }
            },
            OperationType::Base64Encode => {
                use base64::Engine;
                match &input {
                    DataType::Bytes(b) => {
                        Ok(DataType::String(base64::engine::general_purpose::STANDARD.encode(b)))
                    }
                    _ => Err(EvalError::InvalidInput(format!("Base64Encode expects Bytes, got {:?}", input))),
                }
            },
            OperationType::Base64Decode => {
                use base64::Engine;
                match &input {
                    DataType::String(s) => {
                        match base64::engine::general_purpose::STANDARD.decode(s) {
                            Ok(bytes) => Ok(DataType::Bytes(bytes)),
                            Err(e) => Err(EvalError::InvalidInput(format!("Base64Decode failed: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput(format!("Base64Decode expects String, got {:?}", input))),
                }
            },

            // Logical Xor
            OperationType::Xor => {
                let a_bool = is_truthy(&a);
                let b_bool = is_truthy(&b);
                Ok(DataType::Bool(a_bool ^ b_bool))
            }

            // Clamp: clamp(value, min, max)
            OperationType::Clamp => {
                let min_val = inputs.get("min").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                let max_val = inputs.get("max").or(inputs.get("input_2")).cloned().unwrap_or(DataType::Null);
                match (promote_numeric(&input), promote_numeric(&min_val), promote_numeric(&max_val)) {
                    (Some(v), Some(lo), Some(hi)) => {
                        let fv = match v { Ok(i) => i as f64, Err(f) => f };
                        let flo = match lo { Ok(i) => i as f64, Err(f) => f };
                        let fhi = match hi { Ok(i) => i as f64, Err(f) => f };
                        let clamped = fv.max(flo).min(fhi);
                        if v.is_ok() && lo.is_ok() && hi.is_ok() {
                            Ok(DataType::Int64(clamped as i64))
                        } else {
                            Ok(DataType::Float64(clamped))
                        }
                    }
                    _ => Err(EvalError::InvalidInput("Clamp requires numeric arguments".to_string())),
                }
            }

            // Float checks
            OperationType::IsNan => {
                match &input {
                    DataType::Float64(f) => Ok(DataType::Bool(f.is_nan())),
                    DataType::Float32(f) => Ok(DataType::Bool(f.is_nan())),
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::IsInfinite => {
                match &input {
                    DataType::Float64(f) => Ok(DataType::Bool(f.is_infinite())),
                    DataType::Float32(f) => Ok(DataType::Bool(f.is_infinite())),
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::IsFinite => {
                match &input {
                    DataType::Float64(f) => Ok(DataType::Bool(f.is_finite())),
                    DataType::Float32(f) => Ok(DataType::Bool(f.is_finite())),
                    DataType::Int64(_) | DataType::Int32(_) | DataType::Uint32(_) | DataType::Uint64(_) => Ok(DataType::Bool(true)),
                    _ => Ok(DataType::Bool(false)),
                }
            }

            // Parse functions
            OperationType::ParseJson => {
                match &input {
                    DataType::String(s) => {
                        match serde_json::from_str::<serde_json::Value>(s) {
                            Ok(val) => Ok(json_value_to_datatype(&val)),
                            Err(e) => Err(EvalError::InvalidInput(format!("Invalid JSON: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput(format!("ParseJson expects String, got {:?}", input))),
                }
            }
            OperationType::ParseInt => {
                match &input {
                    DataType::String(s) => {
                        let trimmed = s.trim();
                        match trimmed.parse::<i64>() {
                            Ok(n) => Ok(DataType::Int64(n)),
                            Err(_) => Ok(DataType::Null),
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::ParseFloat => {
                match &input {
                    DataType::String(s) => {
                        let trimmed = s.trim();
                        match trimmed.parse::<f64>() {
                            Ok(f) => Ok(DataType::Float64(f)),
                            Err(_) => Ok(DataType::Null),
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // Inverse trigonometric
            OperationType::Asin => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.asin()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).asin())),
                    Some(Err(f)) => Ok(DataType::Float64(f.asin())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Acos => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.acos()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).acos())),
                    Some(Err(f)) => Ok(DataType::Float64(f.acos())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Atan => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.atan()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).atan())),
                    Some(Err(f)) => Ok(DataType::Float64(f.atan())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Atan2 => {
                match (promote_numeric(&a), promote_numeric(&b)) {
                    (Some(av), Some(bv)) => {
                        let y = match av { Ok(i) => i as f64, Err(f) => f };
                        let x = match bv { Ok(i) => i as f64, Err(f) => f };
                        Ok(DataType::Float64(y.atan2(x)))
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // Hyperbolic
            OperationType::Sinh => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.sinh()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).sinh())),
                    Some(Err(f)) => Ok(DataType::Float64(f.sinh())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Cosh => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.cosh()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).cosh())),
                    Some(Err(f)) => Ok(DataType::Float64(f.cosh())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::Tanh => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.tanh()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).tanh())),
                    Some(Err(f)) => Ok(DataType::Float64(f.tanh())),
                    None => Ok(DataType::Null),
                }
            },

            // Arbitrary base logarithm: log(value, base) = ln(value) / ln(base)
            OperationType::Log => {
                let base_val = inputs.get("base").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                match (promote_numeric(&input), promote_numeric(&base_val)) {
                    (Some(vv), Some(bv)) => {
                        let val = match vv { Ok(i) => i as f64, Err(f) => f };
                        let base = match bv { Ok(i) => i as f64, Err(f) => f };
                        if base <= 0.0 || base == 1.0 {
                            Ok(DataType::Float64(f64::NAN))
                        } else {
                            Ok(DataType::Float64(val.ln() / base.ln()))
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // Angle conversion
            OperationType::ToRadians => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.to_radians()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).to_radians())),
                    Some(Err(f)) => Ok(DataType::Float64(f.to_radians())),
                    None => Ok(DataType::Null),
                }
            },
            OperationType::ToDegrees => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.to_degrees()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).to_degrees())),
                    Some(Err(f)) => Ok(DataType::Float64(f.to_degrees())),
                    None => Ok(DataType::Null),
                }
            },

            // Linear interpolation: lerp(a, b, t) = a + (b - a) * t
            OperationType::Lerp => {
                let t_val = inputs.get("t").or(inputs.get("input_2")).cloned().unwrap_or(DataType::Null);
                match (promote_numeric(&a), promote_numeric(&b), promote_numeric(&t_val)) {
                    (Some(av), Some(bv), Some(tv)) => {
                        let fa = match av { Ok(i) => i as f64, Err(f) => f };
                        let fb = match bv { Ok(i) => i as f64, Err(f) => f };
                        let ft = match tv { Ok(i) => i as f64, Err(f) => f };
                        Ok(DataType::Float64(fa + (fb - fa) * ft))
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // Approximate equality: approx_eq(a, b) with optional epsilon
            OperationType::ApproxEq => {
                let epsilon = inputs.get("epsilon").or(inputs.get("input_2"))
                    .and_then(|v| match promote_numeric(v) {
                        Some(Ok(i)) => Some(i as f64),
                        Some(Err(f)) => Some(f),
                        None => None,
                    })
                    .unwrap_or(1e-10);
                match (promote_numeric(&a), promote_numeric(&b)) {
                    (Some(av), Some(bv)) => {
                        let fa = match av { Ok(i) => i as f64, Err(f) => f };
                        let fb = match bv { Ok(i) => i as f64, Err(f) => f };
                        Ok(DataType::Bool((fa - fb).abs() <= epsilon))
                    }
                    _ => Ok(DataType::Bool(false)),
                }
            }

            // Greatest common divisor (Euclidean algorithm)
            OperationType::Gcd => {
                match (a.to_i64(), b.to_i64()) {
                    (Some(mut x), Some(mut y)) => {
                        x = x.checked_abs().unwrap_or(0);
                        y = y.checked_abs().unwrap_or(0);
                        while y != 0 {
                            let t = y;
                            y = x % y;
                            x = t;
                        }
                        Ok(DataType::Int64(x))
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // Least common multiple: lcm(a, b) = |a * b| / gcd(a, b)
            OperationType::Lcm => {
                match (a.to_i64(), b.to_i64()) {
                    (Some(x), Some(y)) => {
                        if x == 0 || y == 0 {
                            return Ok(DataType::Int64(0));
                        }
                        let mut gx = x.checked_abs().unwrap_or(0);
                        let mut gy = y.checked_abs().unwrap_or(0);
                        while gy != 0 {
                            let t = gy;
                            gy = gx % gy;
                            gx = t;
                        }
                        // gx is now gcd
                        // lcm = |x| / gcd * |y| to avoid overflow
                        match (x.checked_abs().unwrap_or(0) / gx).checked_mul(y.checked_abs().unwrap_or(0)) {
                            Some(v) => Ok(DataType::Int64(v)),
                            None => Err(EvalError::InvalidInput("integer overflow in lcm".to_string())),
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Coalesce: return a if non-null, else b
            // ================================================================
            OperationType::Coalesce => {
                if !matches!(a, DataType::Null) {
                    Ok(a)
                } else {
                    Ok(b)
                }
            }

            // ================================================================
            // Default: return input if non-null, else fallback
            // ================================================================
            OperationType::Default => {
                let fallback = inputs.get("fallback").cloned().unwrap_or(DataType::Null);
                if !matches!(input, DataType::Null) {
                    Ok(input)
                } else {
                    Ok(fallback)
                }
            }

            // ================================================================
            // Error: create an error
            // ================================================================
            OperationType::Error => {
                let message = inputs.get("message").cloned().unwrap_or(DataType::String("error".to_string()));
                Err(EvalError::InvalidInput(message.to_string_lossy()))
            }

            // ================================================================
            // StringJoin: join array elements with separator
            // ================================================================
            OperationType::StringJoin => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let sep = inputs.get("separator").or(inputs.get("delimiter")).or(inputs.get("input_1"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                match arr_val {
                    DataType::Array(arr) => {
                        let parts: Vec<String> = arr.iter().map(|v| v.to_string_lossy()).collect();
                        let estimated_len: usize = parts.iter().map(|p| p.len()).sum::<usize>()
                            + parts.len().saturating_sub(1) * sep.len();
                        if estimated_len > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "string_join result exceeds {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(parts.join(&sep)))
                    }
                    _ => Ok(DataType::String(String::new())),
                }
            }

            // ================================================================
            // StringTemplate: simple template with {key} substitution
            // ================================================================
            OperationType::StringTemplate => {
                let template = inputs.get("template").cloned().unwrap_or(DataType::Null);
                let values = inputs.get("values").cloned().unwrap_or(DataType::Null);
                match (&template, &values) {
                    (DataType::String(tmpl), DataType::Map(vals)) => {
                        let mut result = tmpl.clone();
                        for (k, v) in vals {
                            result = result.replace(&format!("{{{}}}", k), &v.to_string_lossy());
                        }
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "string_template result exceeds {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Ok(template),
                }
            }

            // ================================================================
            // StringFormat: same as StringTemplate
            // ================================================================
            OperationType::StringFormat => {
                let template = inputs.get("template").cloned().unwrap_or(DataType::Null);
                let values = inputs.get("values").cloned().unwrap_or(DataType::Null);
                match (&template, &values) {
                    (DataType::String(tmpl), DataType::Map(vals)) => {
                        let mut result = tmpl.clone();
                        for (k, v) in vals {
                            result = result.replace(&format!("{{{}}}", k), &v.to_string_lossy());
                        }
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "string_format result exceeds {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(result))
                    }
                    (DataType::String(tmpl), DataType::Array(vals)) => {
                        let mut result = tmpl.clone();
                        for (i, v) in vals.iter().enumerate() {
                            result = result.replace(&format!("{{{}}}", i), &v.to_string_lossy());
                        }
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "string_format result exceeds {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Ok(template),
                }
            }

            // ================================================================
            // ToBytes / FromBytes
            // ================================================================
            OperationType::ToBytes => {
                match &input {
                    DataType::String(s) => Ok(DataType::Bytes(s.as_bytes().to_vec())),
                    DataType::Bytes(_) => Ok(input.clone()),
                    DataType::Array(arr) => {
                        let mut bytes = Vec::with_capacity(arr.len());
                        for item in arr {
                            match item.to_i64() {
                                Some(n) if (0..=255).contains(&n) => bytes.push(n as u8),
                                _ => return Err(EvalError::InvalidInput("to_bytes: array elements must be 0-255".to_string())),
                            }
                        }
                        Ok(DataType::Bytes(bytes))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::FromBytes => {
                match &input {
                    DataType::Bytes(b) => {
                        match String::from_utf8(b.clone()) {
                            Ok(s) => Ok(DataType::String(s)),
                            Err(_) => Err(EvalError::InvalidInput("from_bytes: invalid UTF-8".to_string())),
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // ArrayFromMap: convert map to array of [key, value] pairs
            // ================================================================
            OperationType::ArrayFromMap => {
                let map_val = inputs.get("map").cloned().unwrap_or(DataType::Null);
                match map_val {
                    DataType::Map(m) => {
                        Ok(DataType::Array(m.into_iter().map(|(k, v)| {
                            DataType::Array(vec![DataType::String(k), v])
                        }).collect()))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }

            // ================================================================
            // MapUpdate: update a map key with a value
            // ================================================================
            OperationType::MapUpdate => {
                match (&map, &key) {
                    (DataType::Map(m), DataType::String(k)) => {
                        let mut new_map = m.clone();
                        new_map.insert(k.clone(), value.clone());
                        Ok(DataType::Map(new_map))
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Math Aggregates
            // ================================================================
            OperationType::MathSum => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let mut int_sum: i64 = 0;
                        let mut has_float = false;
                        let mut float_sum: f64 = 0.0;
                        for item in &arr {
                            match promote_numeric(item) {
                                Some(Ok(i)) => {
                                    if has_float {
                                        float_sum += i as f64;
                                    } else {
                                        match int_sum.checked_add(i) {
                                            Some(v) => int_sum = v,
                                            None => {
                                                has_float = true;
                                                float_sum = int_sum as f64 + i as f64;
                                            }
                                        }
                                    }
                                }
                                Some(Err(f)) => {
                                    if !has_float {
                                        has_float = true;
                                        float_sum = int_sum as f64;
                                    }
                                    float_sum += f;
                                }
                                None => {} // skip non-numeric
                            }
                        }
                        if has_float {
                            Ok(DataType::Float64(float_sum))
                        } else {
                            Ok(DataType::Int64(int_sum))
                        }
                    }
                    _ => Ok(DataType::Int64(0)),
                }
            }
            OperationType::MathProduct => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let mut int_prod: i64 = 1;
                        let mut has_float = false;
                        let mut float_prod: f64 = 1.0;
                        for item in &arr {
                            match promote_numeric(item) {
                                Some(Ok(i)) => {
                                    if has_float {
                                        float_prod *= i as f64;
                                    } else {
                                        match int_prod.checked_mul(i) {
                                            Some(v) => int_prod = v,
                                            None => {
                                                has_float = true;
                                                float_prod = int_prod as f64 * i as f64;
                                            }
                                        }
                                    }
                                }
                                Some(Err(f)) => {
                                    if !has_float {
                                        has_float = true;
                                        float_prod = int_prod as f64;
                                    }
                                    float_prod *= f;
                                }
                                None => {} // skip non-numeric
                            }
                        }
                        if has_float {
                            Ok(DataType::Float64(float_prod))
                        } else {
                            Ok(DataType::Int64(int_prod))
                        }
                    }
                    _ => Ok(DataType::Int64(1)),
                }
            }
            OperationType::MathAverage => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let mut sum = 0.0f64;
                        let mut count = 0usize;
                        for item in &arr {
                            match promote_numeric(item) {
                                Some(Ok(i)) => { sum += i as f64; count += 1; }
                                Some(Err(f)) => { sum += f; count += 1; }
                                None => {}
                            }
                        }
                        if count == 0 {
                            Ok(DataType::Float64(f64::NAN))
                        } else {
                            Ok(DataType::Float64(sum / count as f64))
                        }
                    }
                    _ => Ok(DataType::Float64(f64::NAN)),
                }
            }
            OperationType::MathMinOf => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let mut min_val: Option<f64> = None;
                        for item in &arr {
                            let f = match promote_numeric(item) {
                                Some(Ok(i)) => i as f64,
                                Some(Err(f)) => f,
                                None => continue,
                            };
                            min_val = Some(match min_val {
                                Some(cur) => cur.min(f),
                                None => f,
                            });
                        }
                        Ok(min_val.map(DataType::Float64).unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::MathMaxOf => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let mut max_val: Option<f64> = None;
                        for item in &arr {
                            let f = match promote_numeric(item) {
                                Some(Ok(i)) => i as f64,
                                Some(Err(f)) => f,
                                None => continue,
                            };
                            max_val = Some(match max_val {
                                Some(cur) => cur.max(f),
                                None => f,
                            });
                        }
                        Ok(max_val.map(DataType::Float64).unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::MathCount => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => Ok(DataType::Int64(arr.len() as i64)),
                    _ => Ok(DataType::Int64(0)),
                }
            }

            // ================================================================
            // Remap: remap value from [in_min, in_max] to [out_min, out_max]
            // ================================================================
            OperationType::Remap => {
                let in_min = inputs.get("in_min").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                let in_max = inputs.get("in_max").or(inputs.get("input_2")).cloned().unwrap_or(DataType::Null);
                let out_min = inputs.get("out_min").or(inputs.get("input_3")).cloned().unwrap_or(DataType::Null);
                let out_max = inputs.get("out_max").or(inputs.get("input_4")).cloned().unwrap_or(DataType::Null);
                match (promote_numeric(&input), promote_numeric(&in_min), promote_numeric(&in_max),
                       promote_numeric(&out_min), promote_numeric(&out_max)) {
                    (Some(v), Some(imin), Some(imax), Some(omin), Some(omax)) => {
                        let fv = match v { Ok(i) => i as f64, Err(f) => f };
                        let fimin = match imin { Ok(i) => i as f64, Err(f) => f };
                        let fimax = match imax { Ok(i) => i as f64, Err(f) => f };
                        let fomin = match omin { Ok(i) => i as f64, Err(f) => f };
                        let fomax = match omax { Ok(i) => i as f64, Err(f) => f };
                        if (fimax - fimin).abs() < f64::EPSILON {
                            Ok(DataType::Float64(fomin))
                        } else {
                            let t = (fv - fimin) / (fimax - fimin);
                            Ok(DataType::Float64(fomin + t * (fomax - fomin)))
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // NowTimestamp: current time in milliseconds
            // ================================================================
            OperationType::NowTimestamp => {
                Ok(DataType::Int64(chrono::Utc::now().timestamp_millis()))
            }

            // ================================================================
            // FormatTimestamp: format a timestamp as ISO 8601 string
            // ================================================================
            OperationType::FormatTimestamp => {
                match promote_numeric(&input) {
                    Some(v) => {
                        let ms = match v { Ok(i) => i, Err(f) => f as i64 };
                        match chrono::DateTime::from_timestamp_millis(ms) {
                            Some(dt) => Ok(DataType::String(dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())),
                            None => Err(EvalError::InvalidInput(format!("format_timestamp: invalid timestamp {}", ms))),
                        }
                    }
                    None => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Sleep: sleep for duration ms (no-op in sync evaluator, just returns null)
            // ================================================================
            OperationType::Sleep => {
                let duration = inputs.get("duration").cloned().unwrap_or(DataType::Null);
                if let Some(ms) = duration.to_i64() {
                    if ms > 0 && ms <= 30000 {
                        std::thread::sleep(std::time::Duration::from_millis(ms as u64));
                    }
                }
                Ok(DataType::Null)
            }

            // ================================================================
            // TimestampDiff: difference between two timestamps in ms
            // ================================================================
            OperationType::TimestampDiff => {
                match (promote_numeric(&a), promote_numeric(&b)) {
                    (Some(av), Some(bv)) => {
                        let fa = match av { Ok(i) => i, Err(f) => f as i64 };
                        let fb = match bv { Ok(i) => i, Err(f) => f as i64 };
                        Ok(DataType::Int64(fa - fb))
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // TimestampAdd: add ms to a timestamp
            // ================================================================
            OperationType::TimestampAdd => {
                let amount = inputs.get("amount").cloned().unwrap_or(DataType::Null);
                match (promote_numeric(&input), promote_numeric(&amount)) {
                    (Some(tv), Some(av)) => {
                        let ft = match tv { Ok(i) => i, Err(f) => f as i64 };
                        let fa = match av { Ok(i) => i, Err(f) => f as i64 };
                        Ok(DataType::Int64(ft.saturating_add(fa)))
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // ParseTimestamp: parse ISO timestamp string to millis
            // ================================================================
            OperationType::ParseTimestamp => {
                match &input {
                    DataType::String(s) => {
                        let s = s.trim();
                        // Try RFC 3339 (with timezone)
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                            return Ok(DataType::Int64(dt.timestamp_millis()));
                        }
                        // Try ISO 8601 without timezone (assume UTC)
                        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
                            return Ok(DataType::Int64(dt.and_utc().timestamp_millis()));
                        }
                        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
                            return Ok(DataType::Int64(dt.and_utc().timestamp_millis()));
                        }
                        // Try space-separated datetime
                        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
                            return Ok(DataType::Int64(dt.and_utc().timestamp_millis()));
                        }
                        // Try date-only
                        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                            if let Some(dt) = d.and_hms_opt(0, 0, 0) {
                                return Ok(DataType::Int64(dt.and_utc().timestamp_millis()));
                            }
                        }
                        Ok(DataType::Null)
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // HexEncode / HexDecode
            // ================================================================
            OperationType::HexEncode => {
                match &input {
                    DataType::Bytes(b) => {
                        Ok(DataType::String(hex::encode(b)))
                    }
                    DataType::String(s) => {
                        Ok(DataType::String(hex::encode(s.as_bytes())))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::HexDecode => {
                match &input {
                    DataType::String(s) => {
                        let s = s.trim();
                        let s = s.strip_prefix("0x").or(s.strip_prefix("0X")).unwrap_or(s);
                        match hex::decode(s) {
                            Ok(bytes) => Ok(DataType::Bytes(bytes)),
                            Err(e) => Err(EvalError::InvalidInput(format!("hex_decode: {}", e))),
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // UrlEncode / UrlDecode
            // ================================================================
            OperationType::UrlEncode => {
                match &input {
                    DataType::String(s) => {
                        use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
                        // RFC 3986 unreserved characters: A-Z a-z 0-9 - _ . ~
                        const RFC3986_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
                            .remove(b'-').remove(b'_').remove(b'.').remove(b'~');
                        Ok(DataType::String(utf8_percent_encode(s, RFC3986_ENCODE_SET).to_string()))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::UrlDecode => {
                match &input {
                    DataType::String(s) => {
                        use percent_encoding::percent_decode_str;
                        let s = s.replace('+', " ");
                        match percent_decode_str(&s).decode_utf8() {
                            Ok(decoded) => Ok(DataType::String(decoded.into_owned())),
                            Err(_) => Err(EvalError::InvalidInput("url_decode: invalid UTF-8".to_string())),
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // HashSha256: SHA-256 hash
            // ================================================================
            OperationType::HashSha256 => {
                use sha2::{Sha256, Digest};
                let data = data_to_bytes(&input);
                if data.is_empty() && matches!(input, DataType::Null) {
                    return Ok(DataType::Null);
                }
                let hash = Sha256::digest(&data);
                Ok(DataType::String(hex::encode(hash)))
            }

            // ================================================================
            // HashMd5: MD5 hash
            // ================================================================
            OperationType::HashMd5 => {
                use md5::Md5;
                use md5::Digest;
                let data = data_to_bytes(&input);
                if data.is_empty() && matches!(input, DataType::Null) {
                    return Ok(DataType::Null);
                }
                let hash = Md5::digest(&data);
                Ok(DataType::String(hex::encode(hash)))
            }

            // ================================================================
            // JSON operations
            // ================================================================
            OperationType::JsonGet => {
                let json_val = inputs.get("value").cloned().unwrap_or(DataType::Null);
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => {
                        let parts: Vec<&str> = p.split('.').filter(|s| !s.is_empty()).collect();
                        let mut current = json_val;
                        for part in parts {
                            match &current {
                                DataType::Map(m) => {
                                    current = m.get(part).cloned().unwrap_or(DataType::Null);
                                }
                                DataType::Array(arr) => {
                                    if let Ok(idx) = part.parse::<usize>() {
                                        current = arr.get(idx).cloned().unwrap_or(DataType::Null);
                                    } else {
                                        return Ok(DataType::Null);
                                    }
                                }
                                _ => return Ok(DataType::Null),
                            }
                        }
                        Ok(current)
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::JsonSet => {
                let json_val = inputs.get("value").cloned().unwrap_or(DataType::Null);
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                let item = inputs.get("item").cloned().unwrap_or(DataType::Null);
                match (&json_val, &path) {
                    (DataType::Map(m), DataType::String(key)) => {
                        let mut new_map = m.clone();
                        new_map.insert(key.clone(), item);
                        Ok(DataType::Map(new_map))
                    }
                    _ => Ok(json_val),
                }
            }
            OperationType::JsonDelete => {
                let json_val = inputs.get("value").cloned().unwrap_or(DataType::Null);
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match (&json_val, &path) {
                    (DataType::Map(m), DataType::String(key)) => {
                        let mut new_map = m.clone();
                        new_map.remove(key);
                        Ok(DataType::Map(new_map))
                    }
                    _ => Ok(json_val),
                }
            }
            OperationType::JsonType => {
                Ok(DataType::String(match &input {
                    DataType::Null => "null",
                    DataType::Bool(_) => "boolean",
                    DataType::Int32(_) | DataType::Int64(_) | DataType::Uint32(_)
                    | DataType::Uint64(_) | DataType::Float32(_) | DataType::Float64(_) => "number",
                    DataType::String(_) => "string",
                    DataType::Array(_) => "array",
                    DataType::Map(_) => "object",
                    _ => "unknown",
                }.to_string()))
            }
            OperationType::JsonMerge => {
                match (&a, &b) {
                    (DataType::Map(m1), DataType::Map(m2)) => {
                        let mut merged = m1.clone();
                        for (k, v) in m2 {
                            merged.insert(k.clone(), v.clone());
                        }
                        Ok(DataType::Map(merged))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::JsonPrettyPrint => {
                let json_val = datatype_to_serde_json(&input);
                Ok(DataType::String(serde_json::to_string_pretty(&json_val).unwrap_or_else(|_| "null".to_string())))
            }
            OperationType::JsonCompact => {
                let json_val = datatype_to_serde_json(&input);
                Ok(DataType::String(serde_json::to_string(&json_val).unwrap_or_else(|_| "null".to_string())))
            }
            OperationType::JsonValidate => {
                match &input {
                    DataType::String(s) => {
                        // Try parsing as JSON
                        Ok(DataType::Bool(serde_json::from_str::<serde_json::Value>(s).is_ok()))
                    }
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::JsonFlatten => {
                fn json_flatten(val: &DataType, prefix: &str, result: &mut std::collections::BTreeMap<String, DataType>) {
                    match val {
                        DataType::Map(m) => {
                            for (k, v) in m {
                                if k.starts_with("__") { continue; }
                                let new_key = if prefix.is_empty() { k.clone() } else { format!("{}.{}", prefix, k) };
                                json_flatten(v, &new_key, result);
                            }
                        }
                        DataType::Array(arr) => {
                            for (i, v) in arr.iter().enumerate() {
                                let new_key = if prefix.is_empty() { format!("{}", i) } else { format!("{}.{}", prefix, i) };
                                json_flatten(v, &new_key, result);
                            }
                        }
                        _ => {
                            let key = if prefix.is_empty() { "value".to_string() } else { prefix.to_string() };
                            result.insert(key, val.clone());
                        }
                    }
                }
                let mut result = std::collections::BTreeMap::new();
                json_flatten(&input, "", &mut result);
                Ok(DataType::Map(result))
            }
            OperationType::JsonQuery => {
                // Same as JsonGet with dot-path
                let json_val = inputs.get("value").cloned().unwrap_or(DataType::Null);
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => {
                        let parts: Vec<&str> = p.split('.').filter(|s| !s.is_empty()).collect();
                        let mut current = json_val;
                        for part in parts {
                            match &current {
                                DataType::Map(m) => {
                                    current = m.get(part).cloned().unwrap_or(DataType::Null);
                                }
                                DataType::Array(arr) => {
                                    if let Ok(idx) = part.parse::<usize>() {
                                        current = arr.get(idx).cloned().unwrap_or(DataType::Null);
                                    } else {
                                        return Ok(DataType::Null);
                                    }
                                }
                                _ => return Ok(DataType::Null),
                            }
                        }
                        Ok(current)
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Random operations
            // ================================================================
            OperationType::RandomInt => {
                let val: i64 = rand::rng().random();
                Ok(DataType::Int64(val))
            }
            OperationType::RandomFloat => {
                let val: f64 = rand::rng().random_range(0.0..1.0);
                Ok(DataType::Float64(val))
            }
            OperationType::RandomBool => {
                Ok(DataType::Bool(rand::rng().random::<bool>()))
            }
            OperationType::RandomRange => {
                match (a.to_i64(), b.to_i64()) {
                    (Some(lo), Some(hi)) if lo < hi => {
                        let result = rand::rng().random_range(lo..hi);
                        Ok(DataType::Int64(result))
                    }
                    (Some(lo), Some(hi)) if lo == hi => Ok(DataType::Int64(lo)),
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::RandomChoice => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) if !arr.is_empty() => {
                        let idx = rand::rng().random_range(0..arr.len());
                        Ok(arr[idx].clone())
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::RandomShuffle => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(mut arr) => {
                        use rand::seq::SliceRandom;
                        arr.shuffle(&mut rand::rng());
                        Ok(DataType::Array(arr))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::RandomUuid => {
                Ok(DataType::String(uuid::Uuid::new_v4().to_string()))
            }

            // ================================================================
            // Regex operations (regex crate)
            // ================================================================
            OperationType::RegexMatch => {
                let pattern = inputs.get("input_1").or(inputs.get("pattern")).cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        match regex::Regex::new(pat) {
                            Ok(re) => Ok(DataType::Bool(re.is_match(s))),
                            Err(e) => Err(EvalError::InvalidInput(format!("regex_match: {}", e))),
                        }
                    }
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::RegexTest => {
                let pattern = inputs.get("pattern").cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        match regex::Regex::new(pat) {
                            Ok(re) => Ok(DataType::Bool(re.is_match(s))),
                            Err(e) => Err(EvalError::InvalidInput(format!("regex_test: {}", e))),
                        }
                    }
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::RegexReplace => {
                let replacement = inputs.get("replacement").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                let pattern = inputs.get("pattern").or(inputs.get("input_2")).cloned().unwrap_or(DataType::Null);
                match (&input, &pattern, &replacement) {
                    (DataType::String(s), DataType::String(pat), DataType::String(rep)) => {
                        match regex::Regex::new(pat) {
                            Ok(re) => {
                                let result = re.replace_all(s, rep.as_str()).to_string();
                                if result.len() > MAX_STRING_OUTPUT {
                                    return Err(EvalError::InvalidInput(format!(
                                        "regex_replace result exceeds {} byte limit", MAX_STRING_OUTPUT
                                    )));
                                }
                                Ok(DataType::String(result))
                            }
                            Err(e) => Err(EvalError::InvalidInput(format!("regex_replace: {}", e))),
                        }
                    }
                    _ => Ok(input.clone()),
                }
            }
            OperationType::RegexExtract => {
                let pattern = inputs.get("pattern").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        match regex::Regex::new(pat) {
                            Ok(re) => match re.find(s) {
                                Some(m) => Ok(DataType::String(m.as_str().to_string())),
                                None => Ok(DataType::Null),
                            },
                            Err(e) => Err(EvalError::InvalidInput(format!("regex_extract: {}", e))),
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::RegexSplit => {
                let pattern = inputs.get("pattern").cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        match regex::Regex::new(pat) {
                            Ok(re) => {
                                let parts: Vec<DataType> = re.split(s)
                                    .take(MAX_ARRAY_ELEMENTS + 1)
                                    .map(|p| DataType::String(p.to_string()))
                                    .collect();
                                if parts.len() > MAX_ARRAY_ELEMENTS {
                                    return Err(EvalError::InvalidInput(format!(
                                        "regex_split result exceeds {} element limit", MAX_ARRAY_ELEMENTS
                                    )));
                                }
                                Ok(DataType::Array(parts))
                            }
                            Err(e) => Err(EvalError::InvalidInput(format!("regex_split: {}", e))),
                        }
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::RegexEscape => {
                match &input {
                    DataType::String(s) => Ok(DataType::String(regex::escape(s))),
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::RegexCaptures => {
                let pattern = inputs.get("pattern").cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        match regex::Regex::new(pat) {
                            Ok(re) => match re.captures(s) {
                                Some(caps) => {
                                    let groups: Vec<DataType> = caps.iter()
                                        .map(|m| match m {
                                            Some(m) => DataType::String(m.as_str().to_string()),
                                            None => DataType::Null,
                                        })
                                        .collect();
                                    Ok(DataType::Array(groups))
                                }
                                None => Ok(DataType::Array(vec![])),
                            },
                            Err(e) => Err(EvalError::InvalidInput(format!("regex_captures: {}", e))),
                        }
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::RegexFindAll => {
                let pattern = inputs.get("pattern").cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        match regex::Regex::new(pat) {
                            Ok(re) => {
                                let matches: Vec<DataType> = re.find_iter(s)
                                    .take(MAX_ARRAY_ELEMENTS + 1)
                                    .map(|m| DataType::String(m.as_str().to_string()))
                                    .collect();
                                if matches.len() > MAX_ARRAY_ELEMENTS {
                                    return Err(EvalError::InvalidInput(format!(
                                        "regex_find_all result exceeds {} element limit", MAX_ARRAY_ELEMENTS
                                    )));
                                }
                                Ok(DataType::Array(matches))
                            }
                            Err(e) => Err(EvalError::InvalidInput(format!("regex_find_all: {}", e))),
                        }
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }

            // ================================================================
            // Filesystem operations
            // ================================================================
            OperationType::FsRead => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => {
                        match fs::read_to_string(p) {
                            Ok(content) => Ok(DataType::String(content)),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_read: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("fs_read: path must be a string".to_string())),
                }
            }
            OperationType::FsWrite => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                let content = inputs.get("content").cloned().unwrap_or(DataType::Null);
                match (&path, &content) {
                    (DataType::String(p), DataType::String(c)) => {
                        match fs::write(p, c) {
                            Ok(_) => Ok(DataType::Bool(true)),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_write: {}", e))),
                        }
                    }
                    (DataType::String(p), DataType::Bytes(b)) => {
                        match fs::write(p, b) {
                            Ok(_) => Ok(DataType::Bool(true)),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_write: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("fs_write: path and content must be provided".to_string())),
                }
            }
            OperationType::FsAppend => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                let content = inputs.get("content").cloned().unwrap_or(DataType::Null);
                match (&path, &content) {
                    (DataType::String(p), DataType::String(c)) => {
                        use std::io::Write;
                        match std::fs::OpenOptions::new().append(true).create(true).open(p) {
                            Ok(mut file) => {
                                match file.write_all(c.as_bytes()) {
                                    Ok(_) => Ok(DataType::Bool(true)),
                                    Err(e) => Err(EvalError::InvalidInput(format!("fs_append: {}", e))),
                                }
                            }
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_append: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("fs_append: path and content must be strings".to_string())),
                }
            }
            OperationType::FsExists => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => Ok(DataType::Bool(std::path::Path::new(p).exists())),
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::FsList => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => {
                        match fs::read_dir(p) {
                            Ok(entries) => {
                                let mut results = Vec::new();
                                for entry in entries {
                                    if let Ok(e) = entry {
                                        results.push(DataType::String(
                                            e.file_name().to_string_lossy().to_string()
                                        ));
                                    }
                                    if results.len() >= MAX_ARRAY_ELEMENTS {
                                        break;
                                    }
                                }
                                Ok(DataType::Array(results))
                            }
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_list: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("fs_list: path must be a string".to_string())),
                }
            }
            OperationType::FsMkdir => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => {
                        match fs::create_dir_all(p) {
                            Ok(_) => Ok(DataType::Bool(true)),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_mkdir: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("fs_mkdir: path must be a string".to_string())),
                }
            }
            OperationType::FsRemove => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => {
                        let pb = std::path::Path::new(p);
                        let result = if pb.is_dir() {
                            fs::remove_dir_all(p)
                        } else {
                            fs::remove_file(p)
                        };
                        match result {
                            Ok(_) => Ok(DataType::Bool(true)),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_remove: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("fs_remove: path must be a string".to_string())),
                }
            }
            OperationType::FsIsFile => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => Ok(DataType::Bool(std::path::Path::new(p).is_file())),
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::FsIsDir => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => Ok(DataType::Bool(std::path::Path::new(p).is_dir())),
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::FsSize => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => {
                        match fs::metadata(p) {
                            Ok(meta) => Ok(DataType::Int64(meta.len() as i64)),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_size: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("fs_size: path must be a string".to_string())),
                }
            }
            OperationType::FsCopy => {
                let source = inputs.get("source").cloned().unwrap_or(DataType::Null);
                let dest = inputs.get("destination").cloned().unwrap_or(DataType::Null);
                match (&source, &dest) {
                    (DataType::String(src), DataType::String(dst)) => {
                        match fs::copy(src, dst) {
                            Ok(bytes) => Ok(DataType::Int64(bytes as i64)),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_copy: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("fs_copy: source and destination must be strings".to_string())),
                }
            }
            OperationType::FsMove => {
                let source = inputs.get("source").cloned().unwrap_or(DataType::Null);
                let dest = inputs.get("destination").cloned().unwrap_or(DataType::Null);
                match (&source, &dest) {
                    (DataType::String(src), DataType::String(dst)) => {
                        match fs::rename(src, dst) {
                            Ok(_) => Ok(DataType::Bool(true)),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_move: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("fs_move: source and destination must be strings".to_string())),
                }
            }

            // ================================================================
            // Environment operations
            // ================================================================
            OperationType::EnvGet => {
                let key_val = inputs.get("key").cloned().unwrap_or(DataType::Null);
                match &key_val {
                    DataType::String(k) => {
                        match env::var(k) {
                            Ok(v) => Ok(DataType::String(v)),
                            Err(_) => Ok(DataType::Null),
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::EnvHas => {
                let key_val = inputs.get("key").cloned().unwrap_or(DataType::Null);
                match &key_val {
                    DataType::String(k) => Ok(DataType::Bool(env::var(k).is_ok())),
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::EnvKeys => {
                let keys: Vec<DataType> = env::vars()
                    .take(MAX_ARRAY_ELEMENTS)
                    .map(|(k, _)| DataType::String(k))
                    .collect();
                Ok(DataType::Array(keys))
            }
            OperationType::OsName => {
                Ok(DataType::String(std::env::consts::OS.to_string()))
            }
            OperationType::OsArch => {
                Ok(DataType::String(std::env::consts::ARCH.to_string()))
            }
            OperationType::ProcessPid => {
                Ok(DataType::Int64(process::id() as i64))
            }
            OperationType::CurrentDir => {
                match env::current_dir() {
                    Ok(p) => Ok(DataType::String(p.to_string_lossy().to_string())),
                    Err(e) => Err(EvalError::InvalidInput(format!("current_dir: {}", e))),
                }
            }

            // ================================================================
            // Path operations
            // ================================================================
            OperationType::PathJoin => {
                match (&a, &b) {
                    (DataType::String(p1), DataType::String(p2)) => {
                        let joined = std::path::Path::new(p1).join(p2);
                        Ok(DataType::String(joined.to_string_lossy().to_string()))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::PathBasename => {
                match &input {
                    DataType::String(p) => {
                        let path = std::path::Path::new(p);
                        Ok(path.file_name()
                            .map(|n| DataType::String(n.to_string_lossy().to_string()))
                            .unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::PathDirname => {
                match &input {
                    DataType::String(p) => {
                        let path = std::path::Path::new(p);
                        Ok(path.parent()
                            .map(|n| DataType::String(n.to_string_lossy().to_string()))
                            .unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::PathExtension => {
                match &input {
                    DataType::String(p) => {
                        let path = std::path::Path::new(p);
                        Ok(path.extension()
                            .map(|n| DataType::String(n.to_string_lossy().to_string()))
                            .unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::PathStem => {
                match &input {
                    DataType::String(p) => {
                        let path = std::path::Path::new(p);
                        Ok(path.file_stem()
                            .map(|n| DataType::String(n.to_string_lossy().to_string()))
                            .unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::PathIsAbsolute => {
                match &input {
                    DataType::String(p) => Ok(DataType::Bool(std::path::Path::new(p).is_absolute())),
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::PathParent => {
                match &input {
                    DataType::String(p) => {
                        let path = std::path::Path::new(p);
                        Ok(path.parent()
                            .map(|n| DataType::String(n.to_string_lossy().to_string()))
                            .unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::PathNormalize => {
                match &input {
                    DataType::String(p) => {
                        // Simple normalization: remove . and .. components
                        let path = std::path::Path::new(p);
                        let mut components = Vec::new();
                        for comp in path.components() {
                            match comp {
                                std::path::Component::ParentDir => { components.pop(); }
                                std::path::Component::CurDir => {}
                                other => components.push(other),
                            }
                        }
                        let normalized: std::path::PathBuf = components.into_iter().collect();
                        Ok(DataType::String(normalized.to_string_lossy().to_string()))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::PathSplit => {
                match &input {
                    DataType::String(p) => {
                        let path = std::path::Path::new(p);
                        let parts: Vec<DataType> = path.components()
                            .map(|c| DataType::String(c.as_os_str().to_string_lossy().to_string()))
                            .collect();
                        Ok(DataType::Array(parts))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::PathWithExtension => {
                let extension = inputs.get("extension").cloned().unwrap_or(DataType::Null);
                match (&input, &extension) {
                    (DataType::String(p), DataType::String(ext)) => {
                        let path = std::path::Path::new(p).with_extension(ext);
                        Ok(DataType::String(path.to_string_lossy().to_string()))
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Reduce (array fold with initial value)
            // ================================================================
            OperationType::Reduce => {
                // Reduce is mostly handled by the interpreter's HOF method,
                // but as a standalone op, we treat initial as the seed and return it
                // (the real reduce uses lambda callbacks handled by the interpreter)
                let initial = inputs.get("initial").cloned().unwrap_or(DataType::Null);
                Ok(initial)
            }

            // ================================================================
            // Formatting operations
            // ================================================================
            OperationType::FmtNumber => {
                match promote_numeric(&value) {
                    Some(Ok(n)) => Ok(DataType::String(format!("{}", n))),
                    Some(Err(f)) => Ok(DataType::String(format!("{}", f))),
                    None => Ok(DataType::String(value.to_string_lossy())),
                }
            }
            OperationType::FmtHex => {
                match value.to_i64() {
                    Some(n) => Ok(DataType::String(format!("{:x}", n))),
                    None => Ok(DataType::Null),
                }
            }
            OperationType::FmtBinary => {
                match value.to_i64() {
                    Some(n) => Ok(DataType::String(format!("{:b}", n))),
                    None => Ok(DataType::Null),
                }
            }
            OperationType::FmtPercent => {
                match promote_numeric(&value) {
                    Some(Ok(n)) => Ok(DataType::String(format!("{}%", n))),
                    Some(Err(f)) => Ok(DataType::String(format!("{:.1}%", f * 100.0))),
                    None => Ok(DataType::Null),
                }
            }
            OperationType::FmtBytes => {
                match value.to_i64() {
                    Some(n) => {
                        let abs = (n as f64).abs();
                        let result = if abs < 1024.0 {
                            format!("{} B", n)
                        } else if abs < 1024.0 * 1024.0 {
                            format!("{:.1} KB", n as f64 / 1024.0)
                        } else if abs < 1024.0 * 1024.0 * 1024.0 {
                            format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
                        } else {
                            format!("{:.1} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
                        };
                        Ok(DataType::String(result))
                    }
                    None => Ok(DataType::Null),
                }
            }
            OperationType::FmtDuration => {
                match value.to_i64() {
                    Some(ms) => {
                        let abs = ms.unsigned_abs();
                        let secs = abs / 1000;
                        let mins = secs / 60;
                        let hours = mins / 60;
                        let result = if hours > 0 {
                            format!("{}h {}m {}s", hours, mins % 60, secs % 60)
                        } else if mins > 0 {
                            format!("{}m {}s", mins, secs % 60)
                        } else if secs > 0 {
                            format!("{}.{:03}s", secs, abs % 1000)
                        } else {
                            format!("{}ms", abs)
                        };
                        Ok(DataType::String(if ms < 0 { format!("-{}", result) } else { result }))
                    }
                    None => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Text operations
            // ================================================================
            OperationType::TextSlug => {
                match &input {
                    DataType::String(s) => {
                        Ok(DataType::String(slug::slugify(s)))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::TextCamelCase => {
                match &input {
                    DataType::String(s) => {
                        use heck::ToLowerCamelCase;
                        Ok(DataType::String(s.to_lower_camel_case()))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::TextSnakeCase => {
                match &input {
                    DataType::String(s) => {
                        use heck::ToSnakeCase;
                        Ok(DataType::String(s.to_snake_case()))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::TextTitleCase => {
                match &input {
                    DataType::String(s) => {
                        use heck::ToTitleCase;
                        Ok(DataType::String(s.to_title_case()))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::TextWrap => {
                match &input {
                    DataType::String(s) => {
                        let width = inputs.get("input_1").and_then(|v| v.to_i64()).unwrap_or(80) as usize;
                        Ok(DataType::String(textwrap::fill(s, width)))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::TextTruncate => {
                match &input {
                    DataType::String(s) => {
                        let max_len = inputs.get("input_1").and_then(|v| v.to_i64()).unwrap_or(80) as usize;
                        if s.chars().count() <= max_len {
                            Ok(DataType::String(s.clone()))
                        } else {
                            let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
                            Ok(DataType::String(format!("{}...", truncated)))
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Encode/Decode extended
            // ================================================================
            OperationType::HtmlEscape => {
                match &input {
                    DataType::String(s) => {
                        Ok(DataType::String(html_escape::encode_text(s).into_owned()))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::HtmlUnescape => {
                match &input {
                    DataType::String(s) => {
                        Ok(DataType::String(html_escape::decode_html_entities(s).into_owned()))
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Reflect operations
            // ================================================================
            OperationType::ReflectTypeOf | OperationType::ReflectTypeName => {
                Ok(DataType::String(input.type_name().to_string()))
            }
            OperationType::ReflectIsType => {
                let type_name = inputs.get("type_name").cloned().unwrap_or(DataType::Null);
                match &type_name {
                    DataType::String(t) => {
                        Ok(DataType::Bool(input.type_name() == t.as_str()))
                    }
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::ReflectFields => {
                match &input {
                    DataType::Map(m) => {
                        Ok(DataType::Array(m.keys()
                            .filter(|k| !k.starts_with("__"))
                            .map(|k| DataType::String(k.clone()))
                            .collect()))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::ReflectHasField => {
                let field = inputs.get("field").cloned().unwrap_or(DataType::Null);
                match (&input, &field) {
                    (DataType::Map(m), DataType::String(f)) => Ok(DataType::Bool(m.contains_key(f))),
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::ReflectCallable => {
                // In the evaluator, we can't know if something is callable
                Ok(DataType::Bool(matches!(&input, DataType::String(_))))
            }
            OperationType::ReflectArity => {
                // Can't determine arity from evaluator
                Ok(DataType::Null)
            }
            OperationType::ReflectInspect => {
                Ok(DataType::String(format!("{:?}", input)))
            }

            // ================================================================
            // IfElse: conditional
            // ================================================================
            OperationType::IfElse => {
                let condition = inputs.get("condition").cloned().unwrap_or(DataType::Null);
                let then_val = inputs.get("then").cloned().unwrap_or(DataType::Null);
                let else_val = inputs.get("else").cloned().unwrap_or(DataType::Null);
                if is_truthy(&condition) {
                    Ok(then_val)
                } else {
                    Ok(else_val)
                }
            }

            // ================================================================
            // Switch: match value against cases
            // ================================================================
            OperationType::Switch => {
                let switch_val = inputs.get("value").cloned().unwrap_or(DataType::Null);
                let default_val = inputs.get("default").cloned().unwrap_or(DataType::Null);
                // Check numbered cases: case_0, value_0, case_1, value_1, ...
                for i in 0..100 {
                    let case_key = format!("case_{}", i);
                    let value_key = format!("value_{}", i);
                    match (inputs.get(&case_key), inputs.get(&value_key)) {
                        (Some(case), Some(result)) if *case == switch_val => {
                            return Ok(result.clone());
                        }
                        (None, _) => break,
                        _ => continue,
                    }
                }
                Ok(default_val)
            }

            // ================================================================
            // TryCatch: error handling
            // ================================================================
            OperationType::TryCatch => {
                // As a standalone operation, just return the input (or fallback if input is null)
                let fallback = inputs.get("fallback").cloned().unwrap_or(DataType::Null);
                if matches!(input, DataType::Null) {
                    Ok(fallback)
                } else {
                    Ok(input)
                }
            }

            // ================================================================
            // UUID operations
            // ================================================================
            OperationType::UuidV4 => {
                Ok(DataType::String(uuid::Uuid::new_v4().to_string()))
            }
            OperationType::UuidNil => {
                Ok(DataType::String("00000000-0000-0000-0000-000000000000".to_string()))
            }
            OperationType::UuidIsValid => {
                match &input {
                    DataType::String(s) => {
                        Ok(DataType::Bool(uuid::Uuid::parse_str(s.trim()).is_ok()))
                    }
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::UuidParse => {
                match &input {
                    DataType::String(s) => {
                        match uuid::Uuid::parse_str(s.trim()) {
                            Ok(parsed) => {
                                let mut m = std::collections::BTreeMap::new();
                                m.insert("full".to_string(), DataType::String(parsed.hyphenated().to_string()));
                                m.insert("version".to_string(), DataType::Int64(
                                    parsed.get_version_num() as i64
                                ));
                                Ok(DataType::Map(m))
                            }
                            Err(_) => Ok(DataType::Null),
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Sort operations
            // ================================================================
            OperationType::SortAsc => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(mut arr) => {
                        arr.sort_by(|a, b| {
                            match (promote_numeric(a), promote_numeric(b)) {
                                (Some(pa), Some(pb)) => {
                                    let fa = match pa { Ok(i) => i as f64, Err(f) => f };
                                    let fb = match pb { Ok(i) => i as f64, Err(f) => f };
                                    fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
                                }
                                _ => a.to_string_lossy().cmp(&b.to_string_lossy()),
                            }
                        });
                        Ok(DataType::Array(arr))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::SortDesc => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(mut arr) => {
                        arr.sort_by(|a, b| {
                            match (promote_numeric(a), promote_numeric(b)) {
                                (Some(pa), Some(pb)) => {
                                    let fa = match pa { Ok(i) => i as f64, Err(f) => f };
                                    let fb = match pb { Ok(i) => i as f64, Err(f) => f };
                                    fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
                                }
                                _ => b.to_string_lossy().cmp(&a.to_string_lossy()),
                            }
                        });
                        Ok(DataType::Array(arr))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::SortReverse => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(mut arr) => {
                        arr.reverse();
                        Ok(DataType::Array(arr))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::StableSort => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(mut arr) => {
                        arr.sort_by(|a, b| {
                            match (promote_numeric(a), promote_numeric(b)) {
                                (Some(pa), Some(pb)) => {
                                    let fa = match pa { Ok(i) => i as f64, Err(f) => f };
                                    let fb = match pb { Ok(i) => i as f64, Err(f) => f };
                                    fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
                                }
                                _ => a.to_string_lossy().cmp(&b.to_string_lossy()),
                            }
                        });
                        Ok(DataType::Array(arr))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::IsSorted => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let sorted = arr.windows(2).all(|w| {
                            match (promote_numeric(&w[0]), promote_numeric(&w[1])) {
                                (Some(pa), Some(pb)) => {
                                    let fa = match pa { Ok(i) => i as f64, Err(f) => f };
                                    let fb = match pb { Ok(i) => i as f64, Err(f) => f };
                                    fa <= fb
                                }
                                _ => w[0].to_string_lossy() <= w[1].to_string_lossy(),
                            }
                        });
                        Ok(DataType::Bool(sorted))
                    }
                    _ => Ok(DataType::Bool(true)),
                }
            }
            OperationType::BinarySearch => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match (&arr_val, &value) {
                    (DataType::Array(arr), target) => {
                        let idx = arr.iter().position(|item| {
                            if item == target { return true; }
                            match (promote_numeric(item), promote_numeric(target)) {
                                (Some(av), Some(bv)) => {
                                    let fa = match av { Ok(i) => i as f64, Err(f) => f };
                                    let fb = match bv { Ok(i) => i as f64, Err(f) => f };
                                    fa == fb
                                }
                                _ => false,
                            }
                        });
                        Ok(idx.map(|i| DataType::Int64(i as i64)).unwrap_or(DataType::Int64(-1)))
                    }
                    _ => Ok(DataType::Int64(-1)),
                }
            }
            // SortBy and SortByKey require lambda callbacks, handled by interpreter
            OperationType::SortBy | OperationType::SortByKey => {
                // Return input array unchanged (actual sorting done by interpreter HOF)
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                Ok(arr_val)
            }

            // ================================================================
            // Collection operations
            // ================================================================
            OperationType::SetFrom => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let mut seen = Vec::new();
                        for item in arr {
                            let already = seen.contains(&item);
                            if !already {
                                seen.push(item);
                            }
                        }
                        Ok(DataType::Array(seen))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::SetUnion => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        let mut result = a_arr.clone();
                        for item in b_arr {
                            if !result.iter().any(|s| s == item) {
                                result.push(item.clone());
                            }
                        }
                        Ok(DataType::Array(result))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::SetIntersection => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        let result: Vec<DataType> = a_arr.iter()
                            .filter(|item| b_arr.iter().any(|s| s == *item))
                            .cloned().collect();
                        Ok(DataType::Array(result))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::SetDifference => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        let result: Vec<DataType> = a_arr.iter()
                            .filter(|item| !b_arr.iter().any(|s| s == *item))
                            .cloned().collect();
                        Ok(DataType::Array(result))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::SetSymmetricDifference => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        let mut result = Vec::new();
                        for item in a_arr {
                            if !b_arr.iter().any(|s| s == item) {
                                result.push(item.clone());
                            }
                        }
                        for item in b_arr {
                            if !a_arr.iter().any(|s| s == item) {
                                result.push(item.clone());
                            }
                        }
                        Ok(DataType::Array(result))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::Counter => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let mut counts = std::collections::BTreeMap::new();
                        for item in &arr {
                            let key = item.to_string_lossy();
                            let count = counts.entry(key).or_insert(DataType::Int64(0));
                            if let DataType::Int64(n) = count {
                                *n += 1;
                            }
                        }
                        Ok(DataType::Map(counts))
                    }
                    _ => Ok(DataType::Map(std::collections::BTreeMap::new())),
                }
            }
            OperationType::MostCommon => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let mut counts: std::collections::HashMap<String, (DataType, usize)> = std::collections::HashMap::new();
                        for item in &arr {
                            let key = item.to_string_lossy();
                            counts.entry(key).and_modify(|(_, c)| *c += 1).or_insert((item.clone(), 1));
                        }
                        let max_count = counts.values().map(|(_, c)| *c).max().unwrap_or(0);
                        let most_common: Vec<DataType> = counts.into_values()
                            .filter(|(_, c)| *c == max_count)
                            .map(|(v, _)| v)
                            .collect();
                        if most_common.len() == 1 {
                            Ok(most_common.into_iter().next().unwrap_or(DataType::Null))
                        } else {
                            Ok(DataType::Array(most_common))
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::OrderedMap => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
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
                }
            }

            // ================================================================
            // Stats operations
            // ================================================================
            OperationType::StatsSum | OperationType::StatsMean | OperationType::StatsMedian
            | OperationType::StatsMode | OperationType::StatsVariance | OperationType::StatsStdDev => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let nums: Vec<f64> = arr.iter().filter_map(|item| {
                            match promote_numeric(item) {
                                Some(Ok(i)) => Some(i as f64),
                                Some(Err(f)) => Some(f),
                                None => None,
                            }
                        }).collect();

                        if nums.is_empty() { return Ok(DataType::Null); }

                        match op {
                            OperationType::StatsSum => {
                                Ok(DataType::Float64(nums.iter().sum()))
                            }
                            OperationType::StatsMean => {
                                Ok(DataType::Float64(nums.iter().sum::<f64>() / nums.len() as f64))
                            }
                            OperationType::StatsMedian => {
                                let mut sorted = nums.clone();
                                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                                let mid = sorted.len() / 2;
                                if sorted.len().is_multiple_of(2) {
                                    Ok(DataType::Float64((sorted[mid - 1] + sorted[mid]) / 2.0))
                                } else {
                                    Ok(DataType::Float64(sorted[mid]))
                                }
                            }
                            OperationType::StatsMode => {
                                use ordered_float::OrderedFloat;
                                let mut counts: std::collections::HashMap<OrderedFloat<f64>, usize> = std::collections::HashMap::new();
                                for n in &nums {
                                    *counts.entry(OrderedFloat(*n)).or_insert(0) += 1;
                                }
                                let max_count = counts.values().max().copied().unwrap_or(0);
                                let mode = counts.into_iter()
                                    .find(|(_, c)| *c == max_count)
                                    .map(|(of, _)| of.into_inner())
                                    .unwrap_or(f64::NAN);
                                Ok(DataType::Float64(mode))
                            }
                            OperationType::StatsVariance => {
                                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                                let variance = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64;
                                Ok(DataType::Float64(variance))
                            }
                            OperationType::StatsStdDev => {
                                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                                let variance = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64;
                                Ok(DataType::Float64(variance.sqrt()))
                            }
                            _ => unreachable!(),
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::StatsPercentile => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let pct = inputs.get("percentile").and_then(|v| v.to_f64()).unwrap_or(50.0);
                match arr_val {
                    DataType::Array(arr) => {
                        let mut nums: Vec<f64> = arr.iter().filter_map(|item| {
                            match promote_numeric(item) {
                                Some(Ok(i)) => Some(i as f64),
                                Some(Err(f)) => Some(f),
                                None => None,
                            }
                        }).collect();
                        if nums.is_empty() { return Ok(DataType::Null); }
                        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                        let k = (pct / 100.0 * (nums.len() - 1) as f64).clamp(0.0, (nums.len() - 1) as f64);
                        let lower = k.floor() as usize;
                        let upper = k.ceil() as usize;
                        let frac = k - lower as f64;
                        Ok(DataType::Float64(nums[lower] * (1.0 - frac) + nums[upper] * frac))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::StatsQuantile => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let q = inputs.get("quantile").and_then(|v| v.to_f64()).unwrap_or(0.5);
                match arr_val {
                    DataType::Array(arr) => {
                        let mut nums: Vec<f64> = arr.iter().filter_map(|item| {
                            match promote_numeric(item) {
                                Some(Ok(i)) => Some(i as f64),
                                Some(Err(f)) => Some(f),
                                None => None,
                            }
                        }).collect();
                        if nums.is_empty() { return Ok(DataType::Null); }
                        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                        let k = (q * (nums.len() - 1) as f64).clamp(0.0, (nums.len() - 1) as f64);
                        let lower = k.floor() as usize;
                        let upper = k.ceil() as usize;
                        let frac = k - lower as f64;
                        Ok(DataType::Float64(nums[lower] * (1.0 - frac) + nums[upper] * frac))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::StatsMinBy | OperationType::StatsMaxBy => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let key_name = inputs.get("key").cloned().unwrap_or(DataType::Null);
                match (&arr_val, &key_name) {
                    (DataType::Array(arr), DataType::String(k)) => {
                        let mut best: Option<&DataType> = None;
                        let mut best_val: Option<f64> = None;
                        for item in arr {
                            if let DataType::Map(m) = item {
                                if let Some(v) = m.get(k) {
                                    let fv = match promote_numeric(v) {
                                        Some(Ok(i)) => i as f64,
                                        Some(Err(f)) => f,
                                        None => continue,
                                    };
                                    let is_better = match (best_val, op) {
                                        (None, _) => true,
                                        (Some(cur), OperationType::StatsMinBy) => fv < cur,
                                        (Some(cur), _) => fv > cur,
                                    };
                                    if is_better {
                                        best = Some(item);
                                        best_val = Some(fv);
                                    }
                                }
                            }
                        }
                        Ok(best.cloned().unwrap_or(DataType::Null))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::StatsCovariance | OperationType::StatsCorrelation => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        let a_nums: Vec<f64> = a_arr.iter().filter_map(|v| v.to_f64()).collect();
                        let b_nums: Vec<f64> = b_arr.iter().filter_map(|v| v.to_f64()).collect();
                        let n = a_nums.len().min(b_nums.len());
                        if n == 0 { return Ok(DataType::Null); }

                        let a_mean = a_nums[..n].iter().sum::<f64>() / n as f64;
                        let b_mean = b_nums[..n].iter().sum::<f64>() / n as f64;
                        let cov = (0..n).map(|i| (a_nums[i] - a_mean) * (b_nums[i] - b_mean)).sum::<f64>() / n as f64;

                        if matches!(op, OperationType::StatsCovariance) {
                            Ok(DataType::Float64(cov))
                        } else {
                            let a_std = ((0..n).map(|i| (a_nums[i] - a_mean).powi(2)).sum::<f64>() / n as f64).sqrt();
                            let b_std = ((0..n).map(|i| (b_nums[i] - b_mean).powi(2)).sum::<f64>() / n as f64).sqrt();
                            if a_std == 0.0 || b_std == 0.0 {
                                Ok(DataType::Float64(0.0))
                            } else {
                                Ok(DataType::Float64(cov / (a_std * b_std)))
                            }
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Array HOF operations: These are normally handled by the
            // interpreter directly. When called as standalone ops, return the
            // input array unchanged (the actual transformation requires lambdas).
            // ================================================================
            OperationType::ArrayMap | OperationType::ArrayFilter | OperationType::ArrayFlatMap
            | OperationType::ArrayFind | OperationType::ArrayFindIndex | OperationType::ArrayEvery
            | OperationType::ArraySome | OperationType::ArrayTakeWhile | OperationType::ArraySkipWhile
            | OperationType::ArrayGroupBy | OperationType::ArraySortBy | OperationType::ArrayPartition
            | OperationType::ArrayScan | OperationType::MapMapValues | OperationType::MapFilterEntries => {
                let arr_val = inputs.get("array").or(inputs.get("map")).cloned().unwrap_or(DataType::Null);
                Ok(arr_val)
            }

            // ================================================================
            // ArrayZip, ArrayEnumerate, ArrayTake, ArraySkip, ArrayChunk, ArrayWindow
            // ================================================================
            OperationType::ArrayZip => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        let len = a_arr.len().min(b_arr.len());
                        let result: Vec<DataType> = (0..len)
                            .map(|i| DataType::Array(vec![a_arr[i].clone(), b_arr[i].clone()]))
                            .collect();
                        Ok(DataType::Array(result))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::ArrayEnumerate => {
                match &array {
                    DataType::Array(arr) => {
                        let result: Vec<DataType> = arr.iter().enumerate()
                            .map(|(i, v)| DataType::Array(vec![DataType::Int64(i as i64), v.clone()]))
                            .collect();
                        Ok(DataType::Array(result))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::ArrayTake => {
                let count = inputs.get("input_1").or(inputs.get("count")).cloned().unwrap_or(DataType::Int64(0));
                match &array {
                    DataType::Array(arr) => {
                        let n = count.to_i64().unwrap_or(0).max(0) as usize;
                        Ok(DataType::Array(arr[..n.min(arr.len())].to_vec()))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::ArraySkip => {
                let count = inputs.get("input_1").or(inputs.get("count")).cloned().unwrap_or(DataType::Int64(0));
                match &array {
                    DataType::Array(arr) => {
                        let n = count.to_i64().unwrap_or(0).max(0) as usize;
                        Ok(DataType::Array(arr[n.min(arr.len())..].to_vec()))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::ArrayChunk => {
                let size = inputs.get("input_1").or(inputs.get("size")).cloned().unwrap_or(DataType::Int64(1));
                match &array {
                    DataType::Array(arr) => {
                        let n = size.to_i64().unwrap_or(1).max(1) as usize;
                        let result: Vec<DataType> = arr.chunks(n)
                            .map(|chunk| DataType::Array(chunk.to_vec()))
                            .collect();
                        Ok(DataType::Array(result))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }
            OperationType::ArrayWindow => {
                let size = inputs.get("input_1").or(inputs.get("size")).cloned().unwrap_or(DataType::Int64(1));
                match &array {
                    DataType::Array(arr) => {
                        let n = size.to_i64().unwrap_or(1).max(1) as usize;
                        if n > arr.len() {
                            return Ok(DataType::Array(vec![]));
                        }
                        let result: Vec<DataType> = arr.windows(n)
                            .map(|window| DataType::Array(window.to_vec()))
                            .collect();
                        Ok(DataType::Array(result))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }

            // ================================================================
            // MapUpdate: same as MapSet but named differently
            // (already handled above, this is for std::map::map_update)
            // ================================================================

            // ================================================================
            // Language constructs handled by interpreter, not evaluator
            // ================================================================
            OperationType::FunctionDef | OperationType::FunctionCall
            | OperationType::AsyncSpawn | OperationType::AsyncAwait
            | OperationType::LoopGroup => {
                Ok(DataType::Null)
            }

            // ================================================================
            // Text operations (remaining)
            // ================================================================
            OperationType::TextIndent => {
                match &input {
                    DataType::String(s) => {
                        let indent = inputs.get("input_1").and_then(|v| v.to_i64()).unwrap_or(4) as usize;
                        let pad = " ".repeat(indent);
                        let result: String = s.lines()
                            .map(|line| format!("{}{}", pad, line))
                            .collect::<Vec<_>>()
                            .join("\n");
                        Ok(DataType::String(result))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::TextDedent => {
                match &input {
                    DataType::String(s) => {
                        let lines: Vec<&str> = s.lines().collect();
                        let min_indent = lines.iter()
                            .filter(|l| !l.trim().is_empty())
                            .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
                            .min()
                            .unwrap_or(0);
                        let result: String = lines.iter()
                            .map(|l| {
                                let skip: usize = l.chars().take(min_indent)
                                    .take_while(|c| c.is_whitespace())
                                    .map(|c| c.len_utf8())
                                    .sum();
                                &l[skip..]
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        Ok(DataType::String(result))
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::TextPadLeft => {
                match &input {
                    DataType::String(s) => {
                        let width = inputs.get("input_1").and_then(|v| v.to_i64()).unwrap_or(0) as usize;
                        let char_count = s.chars().count();
                        if char_count >= width {
                            Ok(DataType::String(s.clone()))
                        } else {
                            let padding = " ".repeat(width - char_count);
                            Ok(DataType::String(format!("{}{}", padding, s)))
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::TextPadRight => {
                match &input {
                    DataType::String(s) => {
                        let width = inputs.get("input_1").and_then(|v| v.to_i64()).unwrap_or(0) as usize;
                        let char_count = s.chars().count();
                        if char_count >= width {
                            Ok(DataType::String(s.clone()))
                        } else {
                            let padding = " ".repeat(width - char_count);
                            Ok(DataType::String(format!("{}{}", s, padding)))
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Time operations (remaining)
            // ================================================================
            OperationType::Duration => {
                // Return current time as duration in ms
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                Ok(DataType::Int64(now))
            }
            OperationType::Elapsed => {
                let timestamp = inputs.get("timestamp").cloned().unwrap_or(DataType::Null);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                match timestamp.to_i64() {
                    Some(ts) => Ok(DataType::Int64(now - ts)),
                    None => Ok(DataType::Null),
                }
            }
            OperationType::TimeSleep => {
                let duration = inputs.get("duration").cloned().unwrap_or(DataType::Null);
                if let Some(ms) = duration.to_i64() {
                    if ms > 0 && ms <= 30000 {
                        std::thread::sleep(std::time::Duration::from_millis(ms as u64));
                    }
                }
                Ok(DataType::Null)
            }
            OperationType::AddDuration | OperationType::SubDuration => {
                let timestamp = inputs.get("timestamp").cloned().unwrap_or(DataType::Null);
                let duration = inputs.get("duration").cloned().unwrap_or(DataType::Null);
                match (timestamp.to_i64(), duration.to_i64()) {
                    (Some(ts), Some(dur)) => {
                        if matches!(op, OperationType::AddDuration) {
                            Ok(DataType::Int64(ts.saturating_add(dur)))
                        } else {
                            Ok(DataType::Int64(ts.saturating_sub(dur)))
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::TimeDiff => {
                match (a.to_i64(), b.to_i64()) {
                    (Some(t1), Some(t2)) => Ok(DataType::Int64((t1 - t2).abs())),
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::StartOf | OperationType::EndOf => {
                // Simple: truncate to day boundary
                match input.to_i64() {
                    Some(ms) => {
                        let day_ms = 86400 * 1000i64;
                        let day_start = ms.div_euclid(day_ms) * day_ms;
                        if matches!(op, OperationType::StartOf) {
                            Ok(DataType::Int64(day_start))
                        } else {
                            Ok(DataType::Int64(day_start + day_ms - 1))
                        }
                    }
                    None => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Random remaining
            // ================================================================
            OperationType::RandomBytes => {
                let count = inputs.get("input_1").or(inputs.get("count"))
                    .and_then(|v| v.to_i64()).unwrap_or(16) as usize;
                let count = count.min(1_000_000);
                let mut bytes = vec![0u8; count];
                rand::rng().fill(&mut bytes[..]);
                Ok(DataType::Bytes(bytes))
            }
            OperationType::RandomString => {
                let length = inputs.get("input_1").or(inputs.get("length"))
                    .and_then(|v| v.to_i64()).unwrap_or(16) as usize;
                let length = length.min(MAX_STRING_OUTPUT);
                let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
                let mut rng = rand::rng();
                let mut result = String::with_capacity(length);
                for _ in 0..length {
                    result.push(chars[rng.random_range(0..chars.len())] as char);
                }
                Ok(DataType::String(result))
            }
            OperationType::RandomSample => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let count = inputs.get("input_1").or(inputs.get("count"))
                    .and_then(|v| v.to_i64()).unwrap_or(1) as usize;
                match arr_val {
                    DataType::Array(mut arr) => {
                        use rand::seq::SliceRandom;
                        let count = count.min(arr.len());
                        let (shuffled, _) = arr.partial_shuffle(&mut rand::rng(), count);
                        Ok(DataType::Array(shuffled.to_vec()))
                    }
                    _ => Ok(DataType::Array(vec![])),
                }
            }

            // ================================================================
            // URL operations
            // ================================================================
            OperationType::UrlParse => {
                match &input {
                    DataType::String(url_str) => {
                        match url::Url::parse(url_str) {
                            Ok(parsed) => {
                                let mut m = std::collections::BTreeMap::new();
                                m.insert("raw".into(), DataType::String(url_str.clone()));
                                m.insert("protocol".into(), DataType::String(parsed.scheme().to_string()));
                                m.insert("host".into(), DataType::String(parsed.host_str().unwrap_or("").to_string()));
                                if let Some(port) = parsed.port() {
                                    m.insert("port".into(), DataType::Int64(port as i64));
                                }
                                m.insert("path".into(), DataType::String(parsed.path().to_string()));
                                if let Some(q) = parsed.query() {
                                    m.insert("query".into(), DataType::String(q.to_string()));
                                }
                                if let Some(f) = parsed.fragment() {
                                    m.insert("fragment".into(), DataType::String(f.to_string()));
                                }
                                if !parsed.username().is_empty() {
                                    m.insert("username".into(), DataType::String(parsed.username().to_string()));
                                }
                                if let Some(pw) = parsed.password() {
                                    m.insert("password".into(), DataType::String(pw.to_string()));
                                }
                                Ok(DataType::Map(m))
                            }
                            Err(_) => {
                                let mut m = std::collections::BTreeMap::new();
                                m.insert("raw".into(), DataType::String(url_str.clone()));
                                Ok(DataType::Map(m))
                            }
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::UrlJoin => {
                let base_val = inputs.get("base").cloned().unwrap_or(DataType::Null);
                let path_val = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match (&base_val, &path_val) {
                    (DataType::String(b), DataType::String(p)) => {
                        match url::Url::parse(b) {
                            Ok(base_url) => match base_url.join(p) {
                                Ok(joined) => Ok(DataType::String(joined.to_string())),
                                Err(_) => {
                                    let bt = b.trim_end_matches('/');
                                    let pt = p.trim_start_matches('/');
                                    Ok(DataType::String(format!("{}/{}", bt, pt)))
                                }
                            },
                            Err(_) => {
                                let bt = b.trim_end_matches('/');
                                let pt = p.trim_start_matches('/');
                                Ok(DataType::String(format!("{}/{}", bt, pt)))
                            }
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // Hash extended
            // ================================================================
            OperationType::HashSha512 => {
                use sha2::{Sha512, Digest};
                let data = data_to_bytes(&input);
                if data.is_empty() && matches!(input, DataType::Null) {
                    return Ok(DataType::Null);
                }
                let hash = Sha512::digest(&data);
                Ok(DataType::String(hex::encode(hash)))
            }
            OperationType::HashCrc32 => {
                let data = data_to_bytes(&input);
                if data.is_empty() && matches!(input, DataType::Null) {
                    return Ok(DataType::Null);
                }
                let crc = crc32fast::hash(&data);
                Ok(DataType::Int64(crc as i64))
            }
            OperationType::HmacSha256 => {
                use hmac::{Hmac, Mac};
                use sha2::Sha256;
                type HmacSha256 = Hmac<Sha256>;
                let key_val = inputs.get("key").cloned().unwrap_or(DataType::Null);
                let data = match &input {
                    DataType::String(s) => s.as_bytes().to_vec(),
                    DataType::Bytes(b) => b.clone(),
                    _ => return Ok(DataType::Null),
                };
                let key = match &key_val {
                    DataType::String(s) => s.as_bytes().to_vec(),
                    DataType::Bytes(b) => b.clone(),
                    _ => return Ok(DataType::Null),
                };
                let mut mac = HmacSha256::new_from_slice(&key)
                    .map_err(|e| EvalError::InvalidInput(format!("hmac_sha256: {}", e)))?;
                mac.update(&data);
                let result = mac.finalize();
                Ok(DataType::String(hex::encode(result.into_bytes())))
            }
            OperationType::ConstantTimeEq => {
                use subtle::ConstantTimeEq;
                match (&a, &b) {
                    (DataType::String(s1), DataType::String(s2)) => {
                        Ok(DataType::Bool(s1.as_bytes().ct_eq(s2.as_bytes()).into()))
                    }
                    (DataType::Bytes(b1), DataType::Bytes(b2)) => {
                        Ok(DataType::Bool(b1.ct_eq(b2).into()))
                    }
                    _ => Ok(DataType::Bool(false)),
                }
            }

            // ================================================================
            // Base32 encode/decode
            // ================================================================
            OperationType::Base32Encode => {
                let data = match &input {
                    DataType::Bytes(b) => b.clone(),
                    DataType::String(s) => s.as_bytes().to_vec(),
                    _ => return Ok(DataType::Null),
                };
                Ok(DataType::String(data_encoding::BASE32.encode(&data)))
            }
            OperationType::Base32Decode => {
                match &input {
                    DataType::String(s) => {
                        match data_encoding::BASE32.decode(s.as_bytes()) {
                            Ok(decoded) => Ok(DataType::Bytes(decoded)),
                            Err(_) => Ok(DataType::Null),
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }

            // ================================================================
            // HashBlake3
            // ================================================================
            OperationType::HashBlake3 => {
                let data = data_to_bytes(&input);
                if data.is_empty() && matches!(input, DataType::Null) {
                    return Ok(DataType::Null);
                }
                let hash = blake3::hash(&data);
                Ok(DataType::String(hash.to_hex().to_string()))
            }

            // ================================================================
            // TOML operations (we have the toml crate)
            // ================================================================
            OperationType::TomlParse => {
                match &input {
                    DataType::String(s) => {
                        match s.parse::<toml::Table>() {
                            Ok(table) => Ok(toml_value_to_datatype(&toml::Value::Table(table))),
                            Err(e) => Err(EvalError::InvalidInput(format!("toml_parse: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("toml_parse: input must be a string".to_string())),
                }
            }
            OperationType::TomlStringify => {
                fn datatype_to_toml(val: &DataType) -> toml::Value {
                    match val {
                        DataType::Null => toml::Value::String("null".to_string()),
                        DataType::Bool(b) => toml::Value::Boolean(*b),
                        DataType::Int32(n) => toml::Value::Integer(*n as i64),
                        DataType::Int64(n) => toml::Value::Integer(*n),
                        DataType::Uint32(n) => toml::Value::Integer(*n as i64),
                        DataType::Uint64(n) => {
                            if *n > i64::MAX as u64 {
                                toml::Value::String(n.to_string())
                            } else {
                                toml::Value::Integer(*n as i64)
                            }
                        }
                        DataType::Float32(f) => toml::Value::Float(*f as f64),
                        DataType::Float64(f) => toml::Value::Float(*f),
                        DataType::String(s) => toml::Value::String(s.clone()),
                        DataType::Array(arr) => {
                            toml::Value::Array(arr.iter().map(datatype_to_toml).collect())
                        }
                        DataType::Map(m) => {
                            let table: toml::map::Map<String, toml::Value> = m.iter()
                                .filter(|(k, _)| !k.starts_with("__"))
                                .map(|(k, v)| (k.clone(), datatype_to_toml(v)))
                                .collect();
                            toml::Value::Table(table)
                        }
                        _ => toml::Value::String(val.to_string_lossy()),
                    }
                }
                let toml_val = datatype_to_toml(&input);
                match toml::to_string_pretty(&toml_val) {
                    Ok(s) => Ok(DataType::String(s)),
                    Err(e) => Err(EvalError::InvalidInput(format!("toml_stringify: {}", e))),
                }
            }

            // ================================================================
            // CSV operations (pure string parsing)
            // ================================================================
            OperationType::CsvParse => {
                match &input {
                    DataType::String(s) => {
                        let mut reader = csv::ReaderBuilder::new()
                            .has_headers(true)
                            .from_reader(s.as_bytes());
                        let headers: Vec<String> = reader.headers()
                            .map_err(|e| EvalError::InvalidInput(format!("csv_parse: {}", e)))?
                            .iter()
                            .map(|h| h.to_string())
                            .collect();
                        let mut rows = Vec::new();
                        for result in reader.records() {
                            let record = result.map_err(|e| EvalError::InvalidInput(format!("csv_parse: {}", e)))?;
                            let mut row = std::collections::BTreeMap::new();
                            for (i, field) in record.iter().enumerate() {
                                let key = headers.get(i).cloned().unwrap_or_else(|| format!("col{}", i));
                                row.insert(key, DataType::String(field.to_string()));
                            }
                            rows.push(DataType::Map(row));
                        }
                        Ok(DataType::Array(rows))
                    }
                    _ => Err(EvalError::InvalidInput("csv_parse: expected string input".into())),
                }
            }
            OperationType::CsvStringify => {
                match &input {
                    DataType::Array(rows) if !rows.is_empty() => {
                        let mut wtr = csv::Writer::from_writer(vec![]);
                        if let DataType::Map(first) = &rows[0] {
                            let headers: Vec<&str> = first.keys().map(|k| k.as_str()).collect();
                            wtr.write_record(&headers)
                                .map_err(|e| EvalError::InvalidInput(format!("csv_stringify: {}", e)))?;
                            let vals: Vec<String> = first.values().map(|v| v.to_string()).collect();
                            wtr.write_record(&vals)
                                .map_err(|e| EvalError::InvalidInput(format!("csv_stringify: {}", e)))?;
                            for row in &rows[1..] {
                                if let DataType::Map(m) = row {
                                    let vals: Vec<String> = headers.iter()
                                        .map(|&h| m.get(h).map(|v| v.to_string()).unwrap_or_default())
                                        .collect();
                                    wtr.write_record(&vals)
                                        .map_err(|e| EvalError::InvalidInput(format!("csv_stringify: {}", e)))?;
                                }
                            }
                        }
                        let bytes = wtr.into_inner()
                            .map_err(|e| EvalError::InvalidInput(format!("csv_stringify: {}", e)))?;
                        Ok(DataType::String(String::from_utf8_lossy(&bytes).to_string()))
                    }
                    DataType::Array(_) => Ok(DataType::String(String::new())),
                    _ => Err(EvalError::InvalidInput("csv_stringify: expected array input".into())),
                }
            }
            OperationType::CsvHeaders => {
                match &input {
                    DataType::String(s) => {
                        let mut reader = csv::ReaderBuilder::new()
                            .has_headers(true)
                            .from_reader(s.as_bytes());
                        let headers = reader.headers()
                            .map_err(|e| EvalError::InvalidInput(format!("csv_headers: {}", e)))?;
                        let arr: Vec<DataType> = headers.iter()
                            .map(|h| DataType::String(h.to_string()))
                            .collect();
                        Ok(DataType::Array(arr))
                    }
                    _ => Err(EvalError::InvalidInput("csv_headers: expected string input".into())),
                }
            }
            OperationType::CsvParseRows => {
                match &input {
                    DataType::String(s) => {
                        let mut reader = csv::ReaderBuilder::new()
                            .has_headers(false)
                            .from_reader(s.as_bytes());
                        let mut rows = Vec::new();
                        for result in reader.records() {
                            let record = result.map_err(|e| EvalError::InvalidInput(format!("csv_parse_rows: {}", e)))?;
                            let row: Vec<DataType> = record.iter()
                                .map(|f| DataType::String(f.to_string()))
                                .collect();
                            rows.push(DataType::Array(row));
                        }
                        Ok(DataType::Array(rows))
                    }
                    _ => Err(EvalError::InvalidInput("csv_parse_rows: expected string input".into())),
                }
            }

            // ================================================================
            // YAML operations (serde_yaml)
            // ================================================================
            OperationType::YamlParse => {
                match &input {
                    DataType::String(s) => {
                        let yaml_val: serde_yaml::Value = serde_yaml::from_str(s)
                            .map_err(|e| EvalError::InvalidInput(format!("yaml_parse: {}", e)))?;
                        Ok(yaml_value_to_datatype(&yaml_val))
                    }
                    _ => Err(EvalError::InvalidInput("yaml_parse: input must be a string".to_string())),
                }
            }
            OperationType::YamlStringify => {
                let yaml_val = datatype_to_yaml_value(&input);
                let s = serde_yaml::to_string(&yaml_val)
                    .map_err(|e| EvalError::InvalidInput(format!("yaml_stringify: {}", e)))?;
                Ok(DataType::String(s))
            }
            OperationType::YamlValidate => {
                match &input {
                    DataType::String(s) => {
                        let valid = serde_yaml::from_str::<serde_yaml::Value>(s).is_ok();
                        Ok(DataType::Bool(valid))
                    }
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::YamlToJson => {
                match &input {
                    DataType::String(s) => {
                        let yaml_val: serde_yaml::Value = serde_yaml::from_str(s)
                            .map_err(|e| EvalError::InvalidInput(format!("yaml_to_json: {}", e)))?;
                        let data = yaml_value_to_datatype(&yaml_val);
                        Ok(DataType::String(datatype_to_json_string(&data)))
                    }
                    _ => Err(EvalError::InvalidInput("yaml_to_json: input must be a YAML string".to_string())),
                }
            }
            OperationType::YamlFromJson => {
                match &input {
                    DataType::String(s) => {
                        match serde_json::from_str::<serde_json::Value>(s) {
                            Ok(json_val) => {
                                let data = json_value_to_datatype(&json_val);
                                let yaml_val = datatype_to_yaml_value(&data);
                                let yaml_str = serde_yaml::to_string(&yaml_val)
                                    .map_err(|e| EvalError::InvalidInput(format!("yaml_from_json: {}", e)))?;
                                Ok(DataType::String(yaml_str))
                            }
                            Err(e) => Err(EvalError::InvalidInput(format!("yaml_from_json: invalid JSON: {}", e))),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("yaml_from_json: input must be a JSON string".to_string())),
                }
            }
            OperationType::YamlMerge => {
                match (&a, &b) {
                    (DataType::Map(m1), DataType::Map(m2)) => {
                        let mut merged = m1.clone();
                        for (k, v) in m2 { merged.insert(k.clone(), v.clone()); }
                        Ok(DataType::Map(merged))
                    }
                    (DataType::String(s1), DataType::String(s2)) => {
                        let v1: serde_yaml::Value = serde_yaml::from_str(s1)
                            .map_err(|e| EvalError::InvalidInput(format!("yaml_merge: {}", e)))?;
                        let v2: serde_yaml::Value = serde_yaml::from_str(s2)
                            .map_err(|e| EvalError::InvalidInput(format!("yaml_merge: {}", e)))?;
                        let d1 = yaml_value_to_datatype(&v1);
                        let d2 = yaml_value_to_datatype(&v2);
                        match (d1, d2) {
                            (DataType::Map(m1), DataType::Map(m2)) => {
                                let mut merged = m1;
                                for (k, v) in m2 { merged.insert(k, v); }
                                Ok(DataType::Map(merged))
                            }
                            _ => Err(EvalError::InvalidInput("yaml_merge: both inputs must be YAML maps".to_string())),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("yaml_merge: inputs must be maps or YAML strings".to_string())),
                }
            }

            // ================================================================
            // HTTP client operations (ureq)
            // ================================================================

            OperationType::HttpGet => {
                let url = get_string(inputs, "url")?;
                validate_url(url)?;
                let body: String = ureq::get(url)
                    .call()
                    .map_err(|e| EvalError::InvalidInput(format!("http_get: {}", e)))?
                    .into_body()
                    .read_to_string()
                    .map_err(|e| EvalError::InvalidInput(format!("http_get read: {}", e)))?;
                Ok(DataType::String(body))
            }

            OperationType::HttpPost => {
                let url = get_string(inputs, "url")?;
                validate_url(url)?;
                let payload = inputs.get("body").map(|d| d.to_string());
                let body: String = ureq::post(url)
                    .header("Content-Type", "application/json")
                    .send(payload.as_deref().unwrap_or("").as_bytes())
                    .map_err(|e| EvalError::InvalidInput(format!("http_post: {}", e)))?
                    .into_body()
                    .read_to_string()
                    .map_err(|e| EvalError::InvalidInput(format!("http_post read: {}", e)))?;
                Ok(DataType::String(body))
            }

            OperationType::HttpPut => {
                let url = get_string(inputs, "url")?;
                validate_url(url)?;
                let payload = inputs.get("body").map(|d| d.to_string());
                let body: String = ureq::put(url)
                    .header("Content-Type", "application/json")
                    .send(payload.as_deref().unwrap_or("").as_bytes())
                    .map_err(|e| EvalError::InvalidInput(format!("http_put: {}", e)))?
                    .into_body()
                    .read_to_string()
                    .map_err(|e| EvalError::InvalidInput(format!("http_put read: {}", e)))?;
                Ok(DataType::String(body))
            }

            OperationType::HttpDelete => {
                let url = get_string(inputs, "url")?;
                validate_url(url)?;
                let body: String = ureq::delete(url)
                    .call()
                    .map_err(|e| EvalError::InvalidInput(format!("http_delete: {}", e)))?
                    .into_body()
                    .read_to_string()
                    .map_err(|e| EvalError::InvalidInput(format!("http_delete read: {}", e)))?;
                Ok(DataType::String(body))
            }

            OperationType::HttpRequest => {
                let method = get_string(inputs, "method")?;
                let url = get_string(inputs, "url")?;
                validate_url(url)?;
                let headers = inputs.get("headers").and_then(|d| d.as_map()).cloned();
                let payload = inputs.get("body").map(|d| d.to_string());
                let method_upper = method.to_uppercase();

                let resp = match method_upper.as_str() {
                    "POST" | "PUT" | "PATCH" => {
                        let req = headers.iter().flat_map(|h| h.iter()).fold(
                            match method_upper.as_str() {
                                "POST" => ureq::post(url),
                                "PUT" => ureq::put(url),
                                _ => ureq::patch(url),
                            },
                            |r, (k, v)| r.header(k.as_str(), &v.to_string()),
                        );
                        req.send(payload.as_deref().unwrap_or("").as_bytes())
                            .map_err(|e| EvalError::InvalidInput(format!("http_request: {}", e)))?
                    }
                    "GET" | "DELETE" | "HEAD" => {
                        let req = headers.iter().flat_map(|h| h.iter()).fold(
                            match method_upper.as_str() {
                                "DELETE" => ureq::delete(url),
                                "HEAD" => ureq::head(url),
                                _ => ureq::get(url),
                            },
                            |r, (k, v)| r.header(k.as_str(), &v.to_string()),
                        );
                        req.call()
                            .map_err(|e| EvalError::InvalidInput(format!("http_request: {}", e)))?
                    }
                    other => {
                        return Err(EvalError::InvalidInput(format!(
                            "Unsupported HTTP method: {}",
                            other
                        )));
                    }
                };
                let status = resp.status().as_u16();
                let body: String = resp
                    .into_body()
                    .read_to_string()
                    .map_err(|e| EvalError::InvalidInput(format!("http_request read: {}", e)))?;
                Ok(DataType::Map(std::collections::BTreeMap::from([
                    ("status".into(), DataType::Int64(status as i64)),
                    ("body".into(), DataType::String(body)),
                ])))
            }

            OperationType::HttpHead => {
                let url = get_string(inputs, "url")?;
                validate_url(url)?;
                let resp = ureq::head(url)
                    .call()
                    .map_err(|e| EvalError::InvalidInput(format!("http_head: {}", e)))?;
                let status = resp.status().as_u16();
                let headers: std::collections::BTreeMap<String, DataType> = resp
                    .headers()
                    .keys()
                    .map(|name| {
                        let value = resp
                            .headers()
                            .get(name)
                            .map(|v| v.to_str().unwrap_or("").to_string())
                            .unwrap_or_default();
                        (name.as_str().to_string(), DataType::String(value))
                    })
                    .collect();
                Ok(DataType::Map(std::collections::BTreeMap::from([
                    ("status".into(), DataType::Int64(status as i64)),
                    ("headers".into(), DataType::Map(headers)),
                ])))
            }

            OperationType::HttpOptions => {
                let url = get_string(inputs, "url")?;
                validate_url(url)?;
                let agent = ureq::Agent::new_with_defaults();
                let resp = agent
                    .options(url)
                    .call()
                    .map_err(|e| EvalError::InvalidInput(format!("http_options: {}", e)))?;
                let status = resp.status().as_u16();
                let headers: std::collections::BTreeMap<String, DataType> = resp
                    .headers()
                    .keys()
                    .map(|name| {
                        let value = resp
                            .headers()
                            .get(name)
                            .map(|v| v.to_str().unwrap_or("").to_string())
                            .unwrap_or_default();
                        (name.as_str().to_string(), DataType::String(value))
                    })
                    .collect();
                let allow = headers
                    .get("allow")
                    .cloned()
                    .unwrap_or(DataType::String(String::new()));
                Ok(DataType::Map(std::collections::BTreeMap::from([
                    ("status".into(), DataType::Int64(status as i64)),
                    ("headers".into(), DataType::Map(headers)),
                    ("allow".into(), allow),
                ])))
            }

            OperationType::HttpPatch => {
                let url = get_string(inputs, "url")?;
                validate_url(url)?;
                let payload = inputs.get("body").map(|d| d.to_string());
                let body: String = ureq::patch(url)
                    .header("Content-Type", "application/json")
                    .send(payload.as_deref().unwrap_or("").as_bytes())
                    .map_err(|e| EvalError::InvalidInput(format!("http_patch: {}", e)))?
                    .into_body()
                    .read_to_string()
                    .map_err(|e| EvalError::InvalidInput(format!("http_patch read: {}", e)))?;
                Ok(DataType::String(body))
            }

            // ================================================================
            // Compression operations
            // ================================================================
            OperationType::CompressZstd => {
                let bytes = data_to_bytes(&input);
                let compressed = zstd::encode_all(bytes.as_slice(), 3)
                    .map_err(|e| EvalError::InvalidInput(format!("compress_zstd: {}", e)))?;
                Ok(DataType::Bytes(compressed))
            }
            OperationType::DecompressZstd => {
                let bytes = match &input {
                    DataType::Bytes(b) => b.as_slice(),
                    _ => return Err(EvalError::InvalidInput("decompress_zstd: expected bytes input".into())),
                };
                const MAX_DECOMPRESS: usize = 64 * 1024 * 1024;
                let mut decoder = zstd::Decoder::new(bytes)
                    .map_err(|e| EvalError::InvalidInput(format!("decompress_zstd: {}", e)))?;
                let mut output = Vec::with_capacity(bytes.len().min(1024 * 1024));
                let mut buf = [0u8; 8192];
                loop {
                    let n = decoder.read(&mut buf)
                        .map_err(|e| EvalError::InvalidInput(format!("decompress_zstd: {}", e)))?;
                    if n == 0 { break; }
                    if output.len() + n > MAX_DECOMPRESS {
                        return Err(EvalError::InvalidInput(format!(
                            "Decompressed output exceeds {} byte limit", MAX_DECOMPRESS
                        )));
                    }
                    output.extend_from_slice(&buf[..n]);
                }
                Ok(DataType::Bytes(output))
            }
            OperationType::CompressLz4 => {
                let bytes = data_to_bytes(&input);
                let compressed = lz4_flex::compress_prepend_size(&bytes);
                Ok(DataType::Bytes(compressed))
            }
            OperationType::DecompressLz4 => {
                let bytes = match &input {
                    DataType::Bytes(b) => b.as_slice(),
                    _ => return Err(EvalError::InvalidInput("decompress_lz4: expected bytes input".into())),
                };
                const MAX_DECOMPRESS: usize = 64 * 1024 * 1024;
                if bytes.len() >= 4 {
                    let claimed = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
                    if claimed > MAX_DECOMPRESS {
                        return Err(EvalError::InvalidInput(format!(
                            "LZ4 claimed size {} exceeds {} byte limit", claimed, MAX_DECOMPRESS
                        )));
                    }
                }
                let decompressed = lz4_flex::decompress_size_prepended(bytes)
                    .map_err(|e| EvalError::InvalidInput(format!("decompress_lz4: {}", e)))?;
                if decompressed.len() > MAX_DECOMPRESS {
                    return Err(EvalError::InvalidInput(format!(
                        "Decompressed output exceeds {} byte limit", MAX_DECOMPRESS
                    )));
                }
                Ok(DataType::Bytes(decompressed))
            }

            // ================================================================
            // Certificate / TLS operations
            // ================================================================
            OperationType::CertGenerate | OperationType::CertSelfSigned => {
                let cn = get_string(inputs, "cn")?;
                let mut params = rcgen::CertificateParams::new(vec![cn.to_string()])
                    .map_err(|e| EvalError::InvalidInput(format!("cert_generate: {}", e)))?;
                let mut dn = rcgen::DistinguishedName::new();
                dn.push(rcgen::DnType::CommonName, cn);
                params.distinguished_name = dn;
                let key_pair = rcgen::KeyPair::generate()
                    .map_err(|e| EvalError::InvalidInput(format!("cert_generate key: {}", e)))?;
                let cert = params.self_signed(&key_pair)
                    .map_err(|e| EvalError::InvalidInput(format!("cert_generate: {}", e)))?;
                Ok(DataType::Map(std::collections::BTreeMap::from([
                    ("cert_pem".into(), DataType::String(cert.pem())),
                    ("key_pem".into(), DataType::String(key_pair.serialize_pem())),
                ])))
            }
            OperationType::CertParse | OperationType::CertInfo => {
                let pem = get_string(inputs, "pem")?;
                let (_, pem_block) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
                    .map_err(|e| EvalError::InvalidInput(format!("cert_parse pem: {}", e)))?;
                let cert = pem_block.parse_x509()
                    .map_err(|e| EvalError::InvalidInput(format!("cert_parse x509: {}", e)))?;
                let mut m = std::collections::BTreeMap::new();
                m.insert("subject".into(), DataType::String(cert.subject().to_string()));
                m.insert("issuer".into(), DataType::String(cert.issuer().to_string()));
                m.insert("serial".into(), DataType::String(cert.tbs_certificate.raw_serial_as_string()));
                m.insert("not_before".into(), DataType::String(
                    cert.validity().not_before.to_rfc2822().unwrap_or_default()));
                m.insert("not_after".into(), DataType::String(
                    cert.validity().not_after.to_rfc2822().unwrap_or_default()));
                m.insert("version".into(), DataType::Int64(cert.version().0 as i64));
                if op == OperationType::CertParse {
                    m.insert("signature_algorithm".into(), DataType::String(
                        cert.signature_algorithm.algorithm.to_string()));
                    m.insert("is_ca".into(), DataType::Bool(cert.is_ca()));
                }
                Ok(DataType::Map(m))
            }
            OperationType::CertVerify => {
                let pem = get_string(inputs, "pem")?;
                let result = match x509_parser::pem::parse_x509_pem(pem.as_bytes()) {
                    Ok((_, pem_block)) => match pem_block.parse_x509() {
                        Ok(cert) => {
                            let now = chrono::Utc::now().timestamp();
                            let not_before = cert.validity().not_before.timestamp();
                            let not_after = cert.validity().not_after.timestamp();
                            if now < not_before {
                                std::collections::BTreeMap::from([
                                    ("valid".into(), DataType::Bool(false)),
                                    ("error".into(), DataType::String("Certificate not yet valid".into())),
                                ])
                            } else if now > not_after {
                                std::collections::BTreeMap::from([
                                    ("valid".into(), DataType::Bool(false)),
                                    ("error".into(), DataType::String("Certificate has expired".into())),
                                ])
                            } else {
                                std::collections::BTreeMap::from([("valid".into(), DataType::Bool(true))])
                            }
                        }
                        Err(e) => std::collections::BTreeMap::from([
                            ("valid".into(), DataType::Bool(false)),
                            ("error".into(), DataType::String(format!("Failed to parse X509: {}", e))),
                        ]),
                    },
                    Err(e) => std::collections::BTreeMap::from([
                        ("valid".into(), DataType::Bool(false)),
                        ("error".into(), DataType::String(format!("Failed to parse PEM: {}", e))),
                    ]),
                };
                Ok(DataType::Map(result))
            }
            OperationType::KeyGenerate => {
                let key_pair = rcgen::KeyPair::generate()
                    .map_err(|e| EvalError::InvalidInput(format!("key_generate: {}", e)))?;
                Ok(DataType::Map(std::collections::BTreeMap::from([
                    ("private_pem".into(), DataType::String(key_pair.serialize_pem())),
                    ("public_pem".into(), DataType::String(key_pair.public_key_pem())),
                ])))
            }

            // ================================================================
            // TCP operations
            // ================================================================
            OperationType::TcpConnect => {
                let host = get_string(inputs, "host")?;
                validate_host(host)?;
                let port = get_port(inputs, "port")?;
                let addr = format!("{}:{}", host, port);
                let sock_addr: std::net::SocketAddr = addr
                    .parse()
                    .map_err(|e| EvalError::InvalidInput(format!("tcp_connect: invalid address: {}", e)))?;
                let stream = std::net::TcpStream::connect_timeout(
                    &sock_addr,
                    std::time::Duration::from_millis(5000),
                )
                .map_err(|e| EvalError::InvalidInput(format!("tcp_connect: {}", e)))?;
                let id = conn_id("tcp");
                conn_store(&id, Mutex::new(stream));
                Ok(DataType::String(id))
            }
            OperationType::TcpWrite => {
                let cid = get_string(inputs, "conn_id")?;
                let data = inputs.get("data").cloned().unwrap_or(DataType::Null);
                let bytes = data_to_bytes(&data);
                conn_with::<Mutex<std::net::TcpStream>, _>(cid, |mtx| {
                    use std::io::Write;
                    let stream = mtx
                        .get_mut()
                        .map_err(|_| EvalError::InvalidInput("tcp lock poisoned".into()))?;
                    let written = stream
                        .write(&bytes)
                        .map_err(|e| EvalError::InvalidInput(format!("tcp_write: {}", e)))?;
                    stream
                        .flush()
                        .map_err(|e| EvalError::InvalidInput(format!("tcp_write flush: {}", e)))?;
                    Ok(DataType::Int64(written as i64))
                })
            }
            OperationType::TcpRead => {
                let cid = get_string(inputs, "conn_id")?;
                conn_with::<Mutex<std::net::TcpStream>, _>(cid, |mtx| {
                    let stream = mtx
                        .get_mut()
                        .map_err(|_| EvalError::InvalidInput("tcp lock poisoned".into()))?;
                    let mut buf = vec![0u8; 4096];
                    let n = stream
                        .read(&mut buf)
                        .map_err(|e| EvalError::InvalidInput(format!("tcp_read: {}", e)))?;
                    buf.truncate(n);
                    Ok(DataType::Bytes(buf))
                })
            }
            OperationType::TcpClose => {
                let cid = get_string(inputs, "conn_id")?;
                conn_remove(cid)?;
                Ok(DataType::Null)
            }
            OperationType::TcpBind => {
                let address = get_string(inputs, "address")?;
                let port = get_bind_port(inputs, "port")?;
                let addr = format!("{}:{}", address, port);
                let listener = std::net::TcpListener::bind(&addr)
                    .map_err(|e| EvalError::InvalidInput(format!("tcp_bind: {}", e)))?;
                let id = conn_id("tcp-listener");
                conn_store(&id, Mutex::new(listener));
                Ok(DataType::String(id))
            }
            OperationType::TcpAccept => {
                let lid = get_string(inputs, "listener_id")?;
                // Accept inside conn_with, return stream+addr; store stream outside to avoid deadlock.
                let (stream, addr) =
                    conn_with::<Mutex<std::net::TcpListener>, _>(lid, |mtx| {
                        let listener = mtx.get_mut().map_err(|_| {
                            EvalError::InvalidInput("tcp listener lock poisoned".into())
                        })?;
                        listener.set_nonblocking(true).map_err(|e| {
                            EvalError::InvalidInput(format!("tcp_accept: {}", e))
                        })?;
                        let deadline = std::time::Instant::now()
                            + std::time::Duration::from_millis(30000);
                        let result = loop {
                            match listener.accept() {
                                Ok(r) => break Ok(r),
                                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                    if std::time::Instant::now() >= deadline {
                                        listener.set_nonblocking(false).ok();
                                        break Err(EvalError::InvalidInput(
                                            "tcp_accept: timed out".into(),
                                        ));
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(10));
                                }
                                Err(e) => {
                                    listener.set_nonblocking(false).ok();
                                    break Err(EvalError::InvalidInput(format!(
                                        "tcp_accept: {}",
                                        e
                                    )));
                                }
                            }
                        };
                        listener.set_nonblocking(false).ok();
                        result
                    })?;
                stream.set_nonblocking(false).ok();
                let id = conn_id("tcp");
                conn_store(&id, Mutex::new(stream));
                Ok(DataType::Map(std::collections::BTreeMap::from([
                    ("conn_id".into(), DataType::String(id)),
                    ("address".into(), DataType::String(addr.to_string())),
                ])))
            }
            OperationType::TcpServerClose => {
                let lid = get_string(inputs, "listener_id")?;
                conn_remove(lid)?;
                Ok(DataType::Null)
            }

            // ================================================================
            // UDP operations
            // ================================================================
            OperationType::UdpBind => {
                let address = get_string(inputs, "address")?;
                let port = get_bind_port(inputs, "port")?;
                let addr = format!("{}:{}", address, port);
                let socket = std::net::UdpSocket::bind(&addr)
                    .map_err(|e| EvalError::InvalidInput(format!("udp_bind: {}", e)))?;
                let id = conn_id("udp");
                conn_store(&id, Mutex::new(socket));
                Ok(DataType::String(id))
            }
            OperationType::UdpSendTo => {
                let sid = get_string(inputs, "socket_id")?;
                let data = inputs.get("data").cloned().unwrap_or(DataType::Null);
                let address = get_string(inputs, "address")?;
                let port = get_port(inputs, "port")?;
                let target = format!("{}:{}", address, port);
                let bytes = data_to_bytes(&data);
                conn_with::<Mutex<std::net::UdpSocket>, _>(sid, |mtx| {
                    let socket = mtx
                        .get_mut()
                        .map_err(|_| EvalError::InvalidInput("udp lock poisoned".into()))?;
                    let sent = socket
                        .send_to(&bytes, &target)
                        .map_err(|e| EvalError::InvalidInput(format!("udp_send_to: {}", e)))?;
                    Ok(DataType::Int64(sent as i64))
                })
            }
            OperationType::UdpRecvFrom => {
                let sid = get_string(inputs, "socket_id")?;
                conn_with::<Mutex<std::net::UdpSocket>, _>(sid, |mtx| {
                    let socket = mtx
                        .get_mut()
                        .map_err(|_| EvalError::InvalidInput("udp lock poisoned".into()))?;
                    socket
                        .set_read_timeout(Some(std::time::Duration::from_millis(30000)))
                        .map_err(|e| {
                            EvalError::InvalidInput(format!("udp set_read_timeout: {}", e))
                        })?;
                    let mut buf = vec![0u8; 4096];
                    let (n, addr) = socket.recv_from(&mut buf).map_err(|e| {
                        EvalError::InvalidInput(format!("udp_recv_from: {}", e))
                    })?;
                    buf.truncate(n);
                    Ok(DataType::Map(std::collections::BTreeMap::from([
                        ("data".into(), DataType::Bytes(buf)),
                        ("address".into(), DataType::String(addr.ip().to_string())),
                        ("port".into(), DataType::Int64(addr.port() as i64)),
                    ])))
                })
            }
            OperationType::UdpClose => {
                let sid = get_string(inputs, "socket_id")?;
                conn_remove(sid)?;
                Ok(DataType::Null)
            }

            // ================================================================
            // WebSocket operations
            // ================================================================
            OperationType::WsConnect => {
                let url = get_string(inputs, "url")?;
                validate_url(url)?;
                let (socket, _resp) = tungstenite::connect(url)
                    .map_err(|e| EvalError::InvalidInput(format!("ws_connect: {}", e)))?;
                let id = conn_id("ws");
                conn_store(&id, Mutex::new(socket));
                Ok(DataType::String(id))
            }
            OperationType::WsSend => {
                let cid = get_string(inputs, "conn_id")?;
                let message = inputs.get("message").cloned().unwrap_or(DataType::Null);
                let msg = match &message {
                    DataType::Bytes(b) => tungstenite::Message::Binary(b.clone().into()),
                    other => tungstenite::Message::Text(other.to_string().into()),
                };
                type WsStream = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;
                conn_with::<Mutex<WsStream>, _>(cid, |mtx| {
                    let ws = mtx.get_mut().unwrap_or_else(|e| e.into_inner());
                    ws.send(msg).map_err(|e| EvalError::InvalidInput(format!("ws_send: {}", e)))?;
                    Ok(DataType::Null)
                })
            }
            OperationType::WsReceive => {
                let cid = get_string(inputs, "conn_id")?;
                type WsStream = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;
                conn_with::<Mutex<WsStream>, _>(cid, |mtx| {
                    let ws = mtx.get_mut().unwrap_or_else(|e| e.into_inner());
                    let msg = ws.read().map_err(|e| EvalError::InvalidInput(format!("ws_receive: {}", e)))?;
                    match msg {
                        tungstenite::Message::Text(t) => Ok(DataType::String(t.to_string())),
                        tungstenite::Message::Binary(b) => Ok(DataType::Bytes(b.to_vec())),
                        tungstenite::Message::Close(_) => Ok(DataType::Null),
                        _ => Ok(DataType::Null),
                    }
                })
            }
            OperationType::WsClose => {
                let cid = get_string(inputs, "conn_id")?;
                type WsStream = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;
                let _ = conn_with::<Mutex<WsStream>, _>(cid, |mtx| {
                    let ws = mtx.get_mut().unwrap_or_else(|e| e.into_inner());
                    let _ = ws.close(None);
                    Ok(())
                });
                conn_remove(cid)?;
                Ok(DataType::Null)
            }

            // ================================================================
            // SSE (Server-Sent Events) operations
            // ================================================================
            OperationType::SseConnect => {
                let url = get_string(inputs, "url")?;
                validate_url(url)?;
                let resp = ureq::get(url)
                    .header("Accept", "text/event-stream")
                    .call()
                    .map_err(|e| EvalError::InvalidInput(format!("sse_connect: {}", e)))?;
                let reader = resp.into_body().into_reader();
                let buffered: Box<dyn std::io::BufRead + Send> = Box::new(std::io::BufReader::new(reader));
                let id = conn_id("sse");
                conn_store(&id, Mutex::new(buffered));
                Ok(DataType::String(id))
            }
            OperationType::SseReadEvent => {
                let cid = get_string(inputs, "conn_id")?;
                conn_with::<Mutex<Box<dyn std::io::BufRead + Send>>, _>(cid, |mtx| {
                    let reader = mtx.get_mut().unwrap_or_else(|e| e.into_inner());
                    let mut event_type = String::new();
                    let mut data_lines = Vec::new();
                    let mut event_id = String::new();
                    loop {
                        let mut line = String::new();
                        use std::io::BufRead;
                        let n = reader.read_line(&mut line)
                            .map_err(|e| EvalError::InvalidInput(format!("sse_read_event: {}", e)))?;
                        if n == 0 { return Ok(DataType::Null); }
                        let trimmed = line.trim_end();
                        if trimmed.is_empty() {
                            if !data_lines.is_empty() {
                                let mut m = std::collections::BTreeMap::new();
                                if !event_type.is_empty() {
                                    m.insert("event".into(), DataType::String(event_type));
                                }
                                m.insert("data".into(), DataType::String(data_lines.join("\n")));
                                if !event_id.is_empty() {
                                    m.insert("id".into(), DataType::String(event_id));
                                }
                                return Ok(DataType::Map(m));
                            }
                            continue;
                        }
                        if let Some(rest) = trimmed.strip_prefix("data:") {
                            data_lines.push(rest.trim_start().to_string());
                        } else if let Some(rest) = trimmed.strip_prefix("event:") {
                            event_type = rest.trim_start().to_string();
                        } else if let Some(rest) = trimmed.strip_prefix("id:") {
                            event_id = rest.trim_start().to_string();
                        }
                    }
                })
            }
            OperationType::SseClose => {
                let cid = get_string(inputs, "conn_id")?;
                conn_remove(cid)?;
                Ok(DataType::Null)
            }

            // ================================================================
            // HTTP Server operations
            // ================================================================
            OperationType::HttpServerStart => {
                let address = get_string(inputs, "address")?;
                let port = get_bind_port(inputs, "port")?;
                let addr = format!("{}:{}", address, port);
                let listener = std::net::TcpListener::bind(&addr)
                    .map_err(|e| EvalError::InvalidInput(format!("http_server_start: {}", e)))?;
                let id = conn_id("http-server");
                conn_store(&id, Mutex::new(listener));
                Ok(DataType::String(id))
            }
            OperationType::HttpServerReceive => {
                let sid = get_string(inputs, "server_id")?;
                // Accept and parse outside conn_with to avoid deadlock when storing client
                let (stream, addr) = conn_with::<Mutex<std::net::TcpListener>, _>(sid, |mtx| {
                    let listener = mtx.get_mut().unwrap_or_else(|e| e.into_inner());
                    listener.accept().map_err(|e| EvalError::InvalidInput(format!("http_server_receive: {}", e)))
                })?;
                // Parse HTTP request from the accepted stream
                use std::io::BufRead;
                let mut reader = std::io::BufReader::new(&stream);
                let mut request_line = String::new();
                reader.read_line(&mut request_line)
                    .map_err(|e| EvalError::InvalidInput(format!("http_server_receive: {}", e)))?;
                let parts: Vec<&str> = request_line.trim().splitn(3, ' ').collect();
                let method = parts.first().unwrap_or(&"GET").to_string();
                let path = parts.get(1).unwrap_or(&"/").to_string();
                let mut headers = std::collections::BTreeMap::new();
                let mut content_length: usize = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line)
                        .map_err(|e| EvalError::InvalidInput(format!("http_server_receive: {}", e)))?;
                    let trimmed = line.trim().to_string();
                    if trimmed.is_empty() { break; }
                    if let Some((key, value)) = trimmed.split_once(':') {
                        let key = key.trim().to_lowercase();
                        let value = value.trim().to_string();
                        if key == "content-length" {
                            content_length = value.parse().unwrap_or(0);
                        }
                        headers.insert(key, DataType::String(value));
                    }
                }
                const MAX_BODY: usize = 16 * 1024 * 1024;
                let body = if content_length > MAX_BODY {
                    return Err(EvalError::InvalidInput(format!(
                        "http_server_receive: Content-Length {} exceeds max {}", content_length, MAX_BODY
                    )));
                } else if content_length > 0 {
                    let mut buf = vec![0u8; content_length];
                    reader.read_exact(&mut buf)
                        .map_err(|e| EvalError::InvalidInput(format!("http_server_receive: {}", e)))?;
                    String::from_utf8_lossy(&buf).to_string()
                } else { String::new() };
                let client_id = conn_id("http-client");
                conn_store(&client_id, Mutex::new(stream));
                Ok(DataType::Map(std::collections::BTreeMap::from([
                    ("method".into(), DataType::String(method)),
                    ("path".into(), DataType::String(path)),
                    ("headers".into(), DataType::Map(headers)),
                    ("body".into(), DataType::String(body)),
                    ("client_id".into(), DataType::String(client_id)),
                    ("address".into(), DataType::String(addr.to_string())),
                ])))
            }
            OperationType::HttpServerRespond => {
                let cid = get_string(inputs, "client_id")?;
                let status = match inputs.get("status") {
                    Some(DataType::Int64(n)) => *n,
                    Some(DataType::Int32(n)) => *n as i64,
                    _ => 200,
                };
                let body = inputs.get("body").map(|d| d.to_string()).unwrap_or_default();
                let reason = match status {
                    200 => "OK", 201 => "Created", 204 => "No Content",
                    301 => "Moved Permanently", 302 => "Found",
                    400 => "Bad Request", 401 => "Unauthorized", 403 => "Forbidden",
                    404 => "Not Found", 500 => "Internal Server Error", _ => "OK",
                };
                let response = format!(
                    "HTTP/1.1 {} {}\r\nContent-Length: {}\r\n\r\n{}",
                    status, reason, body.len(), body
                );
                conn_with::<Mutex<std::net::TcpStream>, _>(cid, |mtx| {
                    use std::io::Write;
                    let stream = mtx.get_mut().unwrap_or_else(|e| e.into_inner());
                    stream.write_all(response.as_bytes())
                        .map_err(|e| EvalError::InvalidInput(format!("http_server_respond: {}", e)))?;
                    stream.flush()
                        .map_err(|e| EvalError::InvalidInput(format!("http_server_respond: {}", e)))?;
                    Ok(())
                })?;
                conn_remove(cid)?;
                Ok(DataType::Null)
            }
            OperationType::HttpServerStop => {
                let sid = get_string(inputs, "server_id")?;
                conn_remove(sid)?;
                Ok(DataType::Null)
            }

            // All OperationType variants are now handled above.
        }
    }
}

/// Promote a DataType to either i64 or f64 for arithmetic.
fn is_truthy(val: &DataType) -> bool {
    match val {
        DataType::Bool(b) => *b,
        DataType::Int64(n) => *n != 0,
        DataType::Int32(n) => *n != 0,
        DataType::Uint32(n) => *n != 0,
        DataType::Uint64(n) => *n != 0,
        DataType::Float64(f) => *f != 0.0 && !f.is_nan(),
        DataType::Float32(f) => *f != 0.0 && !f.is_nan(),
        DataType::String(s) => !s.is_empty(),
        DataType::Null => false,
        DataType::Array(a) => !a.is_empty(),
        DataType::Map(m) => !m.is_empty(),
        _ => true,
    }
}


fn promote_numeric(val: &DataType) -> Option<Result<i64, f64>> {
    match val {
        DataType::Int64(x) => Some(Ok(*x)),
        DataType::Int32(x) => Some(Ok(*x as i64)),
        DataType::Uint32(x) => Some(Ok(*x as i64)),
        DataType::Uint64(x) => {
            if *x <= i64::MAX as u64 { Some(Ok(*x as i64)) } else { Some(Err(*x as f64)) }
        }
        DataType::Float64(x) => Some(Err(*x)),
        DataType::Float32(x) => Some(Err(*x as f64)),
        _ => None,
    }
}

fn toml_value_to_datatype(val: &toml::Value) -> DataType {
    match val {
        toml::Value::String(s) => DataType::String(s.clone()),
        toml::Value::Integer(n) => DataType::Int64(*n),
        toml::Value::Float(f) => DataType::Float64(*f),
        toml::Value::Boolean(b) => DataType::Bool(*b),
        toml::Value::Array(arr) => DataType::Array(arr.iter().map(toml_value_to_datatype).collect()),
        toml::Value::Table(t) => {
            let m: std::collections::BTreeMap<String, DataType> = t.iter()
                .map(|(k, v)| (k.clone(), toml_value_to_datatype(v)))
                .collect();
            DataType::Map(m)
        }
        toml::Value::Datetime(dt) => DataType::String(dt.to_string()),
    }
}

fn num_binop(
    a: &DataType, b: &DataType,
    int_op: fn(i64, i64) -> Option<i64>,
    float_op: fn(f64, f64) -> f64,
) -> Result<DataType, EvalError> {
    match (promote_numeric(a), promote_numeric(b)) {
        (Some(Ok(x)), Some(Ok(y))) => match int_op(x, y) {
            Some(v) => Ok(DataType::Int64(v)),
            None => Err(EvalError::InvalidInput("integer overflow".to_string())),
        },
        (Some(av), Some(bv)) => {
            let fa = match av { Ok(i) => i as f64, Err(f) => f };
            let fb = match bv { Ok(i) => i as f64, Err(f) => f };
            Ok(DataType::Float64(float_op(fa, fb)))
        }
        _ => Ok(DataType::Null),
    }
}

fn num_cmp(
    a: &DataType, b: &DataType,
    int_op: fn(&i64, &i64) -> bool,
    float_op: fn(&f64, &f64) -> bool,
    str_op: fn(&str, &str) -> bool,
) -> Result<DataType, EvalError> {
    // String comparison
    if let (DataType::String(x), DataType::String(y)) = (a, b) {
        return Ok(DataType::Bool(str_op(x, y)));
    }
    match (promote_numeric(a), promote_numeric(b)) {
        (Some(Ok(x)), Some(Ok(y))) => Ok(DataType::Bool(int_op(&x, &y))),
        (Some(av), Some(bv)) => {
            let fa = match av { Ok(i) => i as f64, Err(f) => f };
            let fb = match bv { Ok(i) => i as f64, Err(f) => f };
            Ok(DataType::Bool(float_op(&fa, &fb)))
        }
        _ => Ok(DataType::Bool(false)),
    }
}


// =============================================================================
// YAML helpers (serde_yaml conversion)
// =============================================================================

fn yaml_value_to_datatype(val: &serde_yaml::Value) -> DataType {
    match val {
        serde_yaml::Value::Null => DataType::Null,
        serde_yaml::Value::Bool(b) => DataType::Bool(*b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                DataType::Int64(i)
            } else if let Some(f) = n.as_f64() {
                DataType::Float64(f)
            } else {
                DataType::Null
            }
        }
        serde_yaml::Value::String(s) => DataType::String(s.clone()),
        serde_yaml::Value::Sequence(arr) => {
            DataType::Array(arr.iter().map(yaml_value_to_datatype).collect())
        }
        serde_yaml::Value::Mapping(map) => {
            let m: std::collections::BTreeMap<String, DataType> = map.iter()
                .map(|(k, v)| {
                    let key = match k {
                        serde_yaml::Value::String(s) => s.clone(),
                        serde_yaml::Value::Number(n) => format!("{}", n),
                        serde_yaml::Value::Bool(b) => format!("{}", b),
                        serde_yaml::Value::Null => "null".to_string(),
                        other => format!("{:?}", other),
                    };
                    (key, yaml_value_to_datatype(v))
                })
                .collect();
            DataType::Map(m)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_value_to_datatype(&tagged.value),
    }
}

fn datatype_to_yaml_value(data: &DataType) -> serde_yaml::Value {
    match data {
        DataType::Null => serde_yaml::Value::Null,
        DataType::Bool(b) => serde_yaml::Value::Bool(*b),
        DataType::Int64(n) => serde_yaml::Value::Number(serde_yaml::Number::from(*n)),
        DataType::Int32(n) => serde_yaml::Value::Number(serde_yaml::Number::from(*n as i64)),
        DataType::Uint32(n) => serde_yaml::Value::Number(serde_yaml::Number::from(*n as i64)),
        DataType::Uint64(n) => {
            if *n > i64::MAX as u64 {
                serde_yaml::Value::String(n.to_string())
            } else {
                serde_yaml::Value::Number(serde_yaml::Number::from(*n as i64))
            }
        }
        DataType::Float64(f) => {
            if f.is_nan() || f.is_infinite() {
                serde_yaml::Value::String(format!("{}", f))
            } else {
                serde_yaml::Value::Number(serde_yaml::Number::from(*f))
            }
        }
        DataType::Float32(f) => {
            if f.is_nan() || f.is_infinite() {
                serde_yaml::Value::String(format!("{}", f))
            } else {
                serde_yaml::Value::Number(serde_yaml::Number::from(*f as f64))
            }
        }
        DataType::String(s) => serde_yaml::Value::String(s.clone()),
        DataType::Array(arr) => {
            serde_yaml::Value::Sequence(arr.iter().map(datatype_to_yaml_value).collect())
        }
        DataType::Map(m) => {
            let mapping: serde_yaml::Mapping = m.iter()
                .filter(|(k, _)| !k.starts_with("__"))
                .map(|(k, v)| (serde_yaml::Value::String(k.clone()), datatype_to_yaml_value(v)))
                .collect();
            serde_yaml::Value::Mapping(mapping)
        }
        DataType::Bytes(b) => serde_yaml::Value::String(format!("<bytes:{}>", b.len())),
        DataType::Future(_) => serde_yaml::Value::Null,
    }
}

fn datatype_to_serde_json(val: &DataType) -> serde_json::Value {
    match val {
        DataType::Null | DataType::Future(_) => serde_json::Value::Null,
        DataType::Bool(b) => serde_json::Value::Bool(*b),
        DataType::Int64(n) => serde_json::json!(*n),
        DataType::Int32(n) => serde_json::json!(*n),
        DataType::Uint32(n) => serde_json::json!(*n),
        DataType::Uint64(n) => serde_json::json!(*n),
        DataType::Float64(f) => {
            if f.is_finite() { serde_json::json!(*f) } else { serde_json::Value::Null }
        }
        DataType::Float32(f) => {
            if f.is_finite() { serde_json::json!(*f as f64) } else { serde_json::Value::Null }
        }
        DataType::String(s) => serde_json::Value::String(s.clone()),
        DataType::Array(arr) => serde_json::Value::Array(arr.iter().map(datatype_to_serde_json).collect()),
        DataType::Map(m) => {
            let obj: serde_json::Map<String, serde_json::Value> = m.iter()
                .filter(|(k, _)| !k.starts_with("__"))
                .map(|(k, v)| (k.clone(), datatype_to_serde_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        DataType::Bytes(b) => {
            use base64::Engine;
            serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(b))
        }
    }
}

fn datatype_to_json_string(val: &DataType) -> String {
    serde_json::to_string(&datatype_to_serde_json(val)).unwrap_or_else(|_| "null".to_string())
}

fn json_value_to_datatype(val: &serde_json::Value) -> DataType {
    match val {
        serde_json::Value::Null => DataType::Null,
        serde_json::Value::Bool(b) => DataType::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() { DataType::Int64(i) }
            else if let Some(f) = n.as_f64() { DataType::Float64(f) }
            else { DataType::Null }
        }
        serde_json::Value::String(s) => DataType::String(s.clone()),
        serde_json::Value::Array(arr) => DataType::Array(arr.iter().map(json_value_to_datatype).collect()),
        serde_json::Value::Object(obj) => {
            let m: std::collections::BTreeMap<String, DataType> = obj.iter()
                .map(|(k, v)| (k.clone(), json_value_to_datatype(v)))
                .collect();
            DataType::Map(m)
        }
    }
}

fn print_usage() {
    let version = magi_lang::version::version_string();
    eprintln!("MAGI Language v{}", version);
    eprintln!();
    eprintln!("Usage: magi <command> [options] [file]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  run <file.magi>             Interpret and execute a .magi file");
    eprintln!("  check <file.magi>           Type-check and lint (exit 1 on errors)");
    eprintln!("  lint <file.magi>            Lint for style issues");
    eprintln!("  fmt [options] <file.magi>   Format source code");
    eprintln!("  compile <file.magi>         Compile to WebAssembly (.wasm)");
    eprintln!("  run-wasm <file.wasm>        Execute a compiled .wasm file");
    eprintln!("  lsp                         Start the Language Server Protocol server");
    eprintln!("  version                     Show version information");
    eprintln!();
    eprintln!("Format options:");
    eprintln!("  --write, -w                 Write formatted output back to file");
    eprintln!("  --check, -c                 Check formatting without modifying (exit 1 if unformatted)");
    eprintln!();
    eprintln!("Flags:");
    eprintln!("  --help, -h                  Show this help message");
    eprintln!("  --version, -V               Show version");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  magi run main.magi          Run a program");
    eprintln!("  magi main.magi              Shorthand for 'magi run main.magi'");
    eprintln!("  magi check main.magi        Type-check before deploying");
    eprintln!("  magi fmt --write main.magi  Format a file in-place");
    eprintln!("  magi compile main.magi      Compile to dist/main.wasm");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "--help" | "-h" | "help" => {
            print_usage();
        }
        "--version" | "-V" | "version" => {
            println!("MAGI Language v{}", magi_lang::version::version_string());
        }
        "run" => {
            if args.len() < 3 {
                eprintln!("error: missing file argument");
                eprintln!("Usage: magi run <file.magi>");
                process::exit(1);
            }
            cmd_run(&args[2]);
        }
        "compile" => {
            if args.len() < 3 {
                eprintln!("error: missing file argument");
                eprintln!("Usage: magi compile <file.magi>");
                process::exit(1);
            }
            cmd_compile(&args[2]);
        }
        "run-wasm" => {
            if args.len() < 3 {
                eprintln!("error: missing file argument");
                eprintln!("Usage: magi run-wasm <file.wasm>");
                process::exit(1);
            }
            cmd_run_wasm(&args[2]);
        }
        "check" => {
            if args.len() < 3 {
                eprintln!("error: missing file argument");
                eprintln!("Usage: magi check <file.magi>");
                process::exit(1);
            }
            cmd_check(&args[2]);
        }
        "lint" => {
            if args.len() < 3 {
                eprintln!("error: missing file argument");
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
                eprintln!("error: --write and --check are mutually exclusive");
                process::exit(1);
            }

            match file_path {
                Some(path) => cmd_fmt(path, write_in_place, check_only),
                None => {
                    eprintln!("error: missing file argument");
                    eprintln!("Usage: magi fmt [--write] [--check] <file.magi>");
                    process::exit(1);
                }
            }
        }
        "lsp" => {
            cmd_lsp();
        }
        _ => {
            // If first arg is a .magi file, run it directly.
            if args[1].ends_with(".magi") {
                cmd_run(&args[1]);
            } else {
                eprintln!("error: unknown command '{}'", args[1]);
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

        // Security: reject absolute paths and path traversal that escapes the project
        if std::path::Path::new(rel_path).is_absolute() {
            eprintln!("Warning: dependency '{}' uses an absolute path, skipping", id);
            continue;
        }
        // Check if resolved path escapes the project root
        let dep_resolved = dir.join(rel_path);
        if let (Ok(project_canonical), Ok(dep_canonical)) = (dir.canonicalize(), dep_resolved.canonicalize()) {
            // Find the common ancestor: the project root's parent (to allow sibling dirs)
            let project_root = project_canonical.parent().unwrap_or(&project_canonical);
            if !dep_canonical.starts_with(project_root) {
                eprintln!("Warning: dependency '{}' escapes project root, skipping", id);
                continue;
            }
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

        // Check if resolved path escapes the project root
        let dep_resolved = dir.join(rel_path);
        if let (Ok(project_canonical), Ok(dep_canonical)) = (dir.canonicalize(), dep_resolved.canonicalize()) {
            let project_root = project_canonical.parent().unwrap_or(&project_canonical);
            if !dep_canonical.starts_with(project_root) {
                eprintln!("Warning: dependency '{}' escapes project root, skipping", id);
                continue;
            }
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
    let source = read_source(path);

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}:{}:{}: error: {}", path, e.line, e.column, e.message);
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
    let source = read_source(path);

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}:{}:{}: error: {}", path, e.line, e.column, e.message);
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
    }
}

fn cmd_fmt(path: &str, write_in_place: bool, check_only: bool) {
    let source = read_source(path);

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}:{}:{}: error: {}", path, e.line, e.column, e.message);
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
                eprintln!("error: cannot write '{}': {}", path, e);
                process::exit(1);
            }
        }
    } else {
        print!("{}", formatted);
    }
}

fn cmd_lsp() {
    match tokio::runtime::Runtime::new() {
        Ok(rt) => rt.block_on(magi_lang::lsp::run_server()),
        Err(e) => {
            eprintln!("error: failed to create tokio runtime: {}", e);
            process::exit(1);
        }
    }
}

fn cmd_run(path: &str) {
    let source = read_source(path);

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}:{}:{}: error: {}", path, e.line, e.column, e.message);
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
            eprintln!("{}: runtime error: {}", path, e);
            process::exit(1);
        }
    }

    // Print all output/log messages
    for log in &interp.logs {
        println!("{}", log.message);
    }
}

fn cmd_compile(path: &str) {
    let source = read_source(path);

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
            eprintln!("{}:{}:{}: error: {}", path, e.line, e.column, e.message);
            process::exit(1);
        }
    };

    // Type check before compiling
    let imports = std::collections::HashSet::new();
    let analysis = magi_lang::syntax::type_checker::check_types(&program, &imports);
    let mut has_errors = false;
    for d in &analysis.diagnostics {
        let severity = match d.severity {
            DiagnosticSeverity::Error => { has_errors = true; "error" }
            DiagnosticSeverity::Warning => "warning",
            DiagnosticSeverity::Info => "info",
        };
        let code = d.code.as_deref().unwrap_or("");
        eprintln!("{}:{}:{}: {} [{}]: {}", path, d.line, d.column, severity, code, d.message);
    }
    if has_errors {
        eprintln!("Type errors found; aborting compilation.");
        process::exit(1);
    }

    let wasm_bytes = match compiler::compile_to_wasm(&program) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("{}: compile error: {}", path, e);
            process::exit(1);
        }
    };

    let src_path = std::path::Path::new(path);
    let dir = src_path.parent().unwrap_or(std::path::Path::new("."));
    let dist_dir = dir.join("dist");
    if let Err(e) = fs::create_dir_all(&dist_dir) {
        eprintln!("error: cannot create dist directory: {}", e);
        process::exit(1);
    }

    let stem = src_path.file_stem().unwrap_or_default();
    let out_path = dist_dir.join(format!("{}.wasm", stem.to_string_lossy()));
    match fs::write(&out_path, &wasm_bytes) {
        Ok(_) => {
            println!("Compiled {} -> {} ({} bytes)", path, out_path.display(), wasm_bytes.len());
        }
        Err(e) => {
            eprintln!("error: cannot write '{}': {}", out_path.display(), e);
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
        3 => {
            // F64: payload is lower 56 bits of IEEE 754. Top 8 bits are lost.
            // Reconstruct with zeroed top 8 bits (precision loss for negative/large floats).
            let bits = payload & 0x00FFFFFFFFFFFFFF;
            let f = f64::from_bits(bits as u64);
            if f == (f as i64 as f64) && !f.is_nan() && f.abs() < 1e15 {
                format!("{}.0", f as i64)  // show as integer-like if exact
            } else {
                format!("{}", f)
            }
        }
        4 => {
            // String: payload is memory offset.
            let offset = payload as usize;
            if offset.checked_add(4).is_none_or(|end| end > data.len()) {
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
            if ptr.checked_add(4).is_none_or(|end| end > data.len()) {
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
                if elem_offset.checked_add(8).is_none_or(|end| end > data.len()) {
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
            if ptr.checked_add(4).is_none_or(|end| end > data.len()) {
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
                if val_offset.checked_add(8).is_none_or(|end| end > data.len()) {
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
        7 => {
            // I32: payload is a 32-bit signed integer (sign-extended in 56 bits)
            let n = if payload & (1 << 31) != 0 {
                (payload | !0xFFFFFFFF) as i32
            } else {
                payload as i32
            };
            format!("{}", n)
        }
        8 => {
            // F32: payload is lower 32 bits of IEEE 754 f32
            let bits = (payload & 0xFFFFFFFF) as u32;
            let f = f32::from_bits(bits);
            format!("{}", f)
        }
        _ => format!("<tagged:{}:{}>", tag, payload),
    }
}

fn cmd_run_wasm(path: &str) {
    let wasm_bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read '{}': {}", path, e);
            process::exit(1);
        }
    };

    // Validate WASM magic.
    if wasm_bytes.len() < 8 || &wasm_bytes[0..4] != b"\0asm" {
        eprintln!("error: '{}' is not a valid WASM file", path);
        process::exit(1);
    }

    let engine = wasmtime::Engine::default();
    let module = match wasmtime::Module::new(&engine, &wasm_bytes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: cannot load '{}': {}", path, e);
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
            eprintln!("error: WASM instantiation failed: {}", e);
            process::exit(1);
        }
    };

    // Call __main.
    let main_fn = match instance.get_typed_func::<(), i64>(&mut store, "__main") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: no __main export found in '{}': {}", path, e);
            process::exit(1);
        }
    };

    match main_fn.call(&mut store, ()) {
        Ok(result) => {
            if result != 0 {
                let tag = (result >> 56) as u8;
                if tag != 0 {
                    // Use format_tagged_value which handles all tag types
                    let mem = instance.get_memory(&mut store, "memory")
                        .map(|m| m.data(&store).to_vec())
                        .unwrap_or_default();
                    println!("Result: {}", format_tagged_value(result, &mem));
                }
            }
        }
        Err(e) => {
            eprintln!("{}: WASM execution error: {}", path, e);
            process::exit(1);
        }
    }
}
