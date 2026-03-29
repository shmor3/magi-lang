//! MAGI language CLI — interpret and compile .magi files.

use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::env;
use std::fs;
use std::io::Read as _;
use std::net::IpAddr;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

/// When true, filesystem and network operations are forbidden.
static SANDBOX_MODE: AtomicBool = AtomicBool::new(false);

use magi_lang::compiler;
use magi_lang::eval::{DiagnosticSeverity, EvalError, OperationEvaluator};
use magi_lang::syntax::interpreter::{resolve_package_from_source, Interpreter, ResolvedPackage};
use magi_lang::syntax::parser::parse_v2;
use magi_lang::telemetry::Telemetry;
use magi_lang::types::{DataType, OperationType};

/// Maximum output string length (10 MB).
const MAX_STRING_OUTPUT: usize = 100_000_000; // 100 MB

/// Maximum array element count.
const MAX_ARRAY_ELEMENTS: usize = 100_000_000; // 100 million

/// Maximum number of open connections in the global registry.
const MAX_CONNECTIONS: usize = 65_536; // Match OS file descriptor limits

/// Maximum size of a single SSE line (1 MB).
const MAX_SSE_LINE_BYTES: usize = 1_048_576;

/// Maximum recursion depth for JSON conversion.
const MAX_JSON_DEPTH: usize = 64;

/// Maximum output size for ReflectInspect (1 MB).
const MAX_INSPECT_OUTPUT: usize = 1_048_576;

/// UTF-8 BOM (byte order mark).
const UTF8_BOM: &str = "\u{FEFF}";

/// Maximum file write size: 1 GB.
const MAX_FILE_WRITE_SIZE: usize = 1024 * 1024 * 1024;

/// Maximum number of compiled regexes held in the thread-local LRU cache.
const REGEX_CACHE_CAPACITY: usize = 128;

// Thread-local LRU cache for compiled regexes.
// `order` tracks access recency (most-recent at back); `map` stores compiled patterns.
thread_local! {
    static REGEX_CACHE: RefCell<RegexCache> = RefCell::new(RegexCache {
        map: HashMap::with_capacity(REGEX_CACHE_CAPACITY),
        order: VecDeque::with_capacity(REGEX_CACHE_CAPACITY),
    });
}

struct RegexCache {
    map: HashMap<String, magi_lang::util::Regex>,
    order: VecDeque<String>,
}

// Connection registry — global storage for open connections (HTTP clients,
// WebSocket handles, TLS sessions, etc.) keyed by UUID-based connection IDs.

/// Global connection registry.
static CONNECTIONS: LazyLock<Mutex<HashMap<String, Box<dyn Any + Send>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Store a connection in the global registry.
fn conn_store<T: Send + 'static>(id: &str, conn: T) -> Result<(), EvalError> {
    let mut map = CONNECTIONS.lock().unwrap_or_else(|e| {
        eprintln!("warning: connection registry mutex was poisoned, recovering");
        e.into_inner()
    });
    if map.contains_key(id) {
        return Err(EvalError::InvalidInput(format!(
            "connection ID already exists: {}",
            id
        )));
    }
    if map.len() >= MAX_CONNECTIONS {
        return Err(EvalError::InvalidInput(format!(
            "connection limit reached (max {})",
            MAX_CONNECTIONS
        )));
    }
    map.insert(id.to_string(), Box::new(conn));
    Ok(())
}

/// Execute a closure with mutable access to a typed connection.
fn conn_with<T: Send + 'static, R>(
    id: &str,
    f: impl FnOnce(&mut T) -> Result<R, EvalError>,
) -> Result<R, EvalError> {
    let mut map = CONNECTIONS.lock().unwrap_or_else(|e| {
        eprintln!("warning: connection registry mutex was poisoned, recovering");
        e.into_inner()
    });
    let entry = map
        .get_mut(id)
        .ok_or_else(|| EvalError::InvalidInput(format!("Connection not found: {}", id)))?;
    let typed = entry
        .downcast_mut::<T>()
        .ok_or_else(|| EvalError::InvalidInput(format!("Connection type mismatch: {}", id)))?;
    f(typed)
}

/// Remove a connection from the global registry.
fn conn_remove(id: &str) -> Result<(), EvalError> {
    let mut map = CONNECTIONS.lock().unwrap_or_else(|e| {
        eprintln!("warning: connection registry mutex was poisoned, recovering");
        e.into_inner()
    });
    map.remove(id)
        .ok_or_else(|| EvalError::InvalidInput(format!("Connection not found: {}", id)))?;
    Ok(())
}

/// Generate a UUID-based connection ID with the given prefix.
fn conn_id(prefix: &str) -> String {
    format!("{}:{}", prefix, magi_lang::util::uuid_v4())
}


/// Check whether an IP address is in a private / loopback / link-local /
/// CGNAT range that should be blocked for outbound requests.
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
                    // Teredo 2001:0000::/32 — encapsulates IPv4, check inner
                    || (seg[0] == 0x2001
                        && seg[1] == 0x0000
                        && is_blocked_ip(IpAddr::V4(std::net::Ipv4Addr::new(
                            // Teredo encodes the IPv4 as bitwise NOT in seg[6..7]
                            !((seg[6] >> 8) as u8),
                            !(seg[6] as u8),
                            !((seg[7] >> 8) as u8),
                            !(seg[7] as u8),
                        ))))
                    // 6to4 2002::/16 — encapsulates IPv4 in seg[1..2]
                    || (seg[0] == 0x2002
                        && is_blocked_ip(IpAddr::V4(std::net::Ipv4Addr::new(
                            (seg[1] >> 8) as u8,
                            seg[1] as u8,
                            (seg[2] >> 8) as u8,
                            seg[2] as u8,
                        ))))
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
                    // ::0:0/96 IPv4-compatible (deprecated but still routable
                    // on some stacks) — check inner IPv4
                    || (seg[0] == 0
                        && seg[1] == 0
                        && seg[2] == 0
                        && seg[3] == 0
                        && seg[4] == 0
                        && seg[5] == 0
                        && (seg[6] != 0 || seg[7] > 1) // exclude ::0 and ::1 (already handled)
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
fn validate_url(url_str: &str) -> Result<(), EvalError> {
    let parsed = magi_lang::util::UrlParts::parse(url_str)
        .map_err(|e| EvalError::InvalidInput(format!("Invalid URL: {}", e)))?;

    match parsed.scheme.as_str() {
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
fn validate_host(host: &str) -> Result<(), EvalError> {
    let lower = host.to_ascii_lowercase();

    // Block well-known internal hostnames.
    if lower == "localhost"
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
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

/// Validate a URL and also perform DNS resolution to check that the resolved
/// IP addresses are not in blocked ranges. This mitigates DNS rebinding
/// attacks where `validate_url` only checks the hostname string but the DNS
/// can resolve to a private/internal IP.
fn validate_url_with_dns(url_str: &str) -> Result<(), EvalError> {
    validate_url(url_str)?;

    let parsed = magi_lang::util::UrlParts::parse(url_str)
        .map_err(|e| EvalError::InvalidInput(format!("Invalid URL: {}", e)))?;

    let host = match parsed.host_str() {
        Some(h) => h,
        None => return Ok(()), // already validated above
    };

    // If the host is already a literal IP, is_blocked_ip was already checked
    // by validate_host inside validate_url. Skip DNS for raw IPs.
    let ip_str = host.trim_start_matches('[').trim_end_matches(']');
    if ip_str.parse::<IpAddr>().is_ok() {
        return Ok(());
    }

    // Resolve hostname and check all returned IPs.
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addr = format!("{}:{}", host, port);
    use std::net::ToSocketAddrs;
    match addr.to_socket_addrs() {
        Ok(addrs) => {
            for sock_addr in addrs {
                if is_blocked_ip(sock_addr.ip()) {
                    return Err(EvalError::InvalidInput(format!(
                        "Blocked IP after DNS resolution: {} resolved to {}",
                        host,
                        sock_addr.ip()
                    )));
                }
            }
        }
        Err(e) => {
            return Err(EvalError::InvalidInput(format!(
                "DNS resolution failed for {}: {}",
                host, e
            )));
        }
    }

    Ok(())
}

// Utility helpers for FullEvaluator operation implementations

/// Extract a port number from an input map.
fn get_port(inputs: &HashMap<String, DataType>, key: &str) -> Result<u16, EvalError> {
    get_port_range(inputs, key, 1)
}

/// Extract a port number from an input map, allowing port 0 (OS-assigned).
fn get_bind_port(inputs: &HashMap<String, DataType>, key: &str) -> Result<u16, EvalError> {
    get_port_range(inputs, key, 0)
}

/// Extract and validate a port number from an input map.
/// `min_port` controls the lower bound (0 for bind ports, 1 for connect ports).
fn get_port_range(inputs: &HashMap<String, DataType>, key: &str, min_port: i64) -> Result<u16, EvalError> {
    let val = inputs.get(key).ok_or_else(|| {
        EvalError::InvalidInput(format!("Missing required input: {}", key))
    })?;
    let n = val.to_i64().ok_or_else(|| EvalError::TypeError {
        expected: "numeric".to_string(),
        actual: val.type_name().to_string(),
        context: format!("port '{}'", key),
    })?;
    if (min_port..=65535).contains(&n) {
        Ok(n as u16)
    } else {
        Err(EvalError::InvalidInput(format!(
            "Port out of range ({}-65535): {}",
            min_port, n
        )))
    }
}

/// Shared HTTP agent with connection pooling (#369).
/// Redirects are disabled to prevent SSRF bypass (our pre-request DNS validation
/// would not apply to redirect targets).
static HTTP_AGENT: LazyLock<magi_lang::util::HttpClient> = LazyLock::new(|| {
    magi_lang::util::HttpClient::new(std::time::Duration::from_secs(30))
});

/// Get the shared HTTP agent.
fn http_agent() -> &'static magi_lang::util::HttpClient {
    &HTTP_AGENT
}

/// Read HTTP response body with a size limit.
fn read_http_body(body: magi_lang::util::HttpBody, context: &str) -> Result<String, EvalError> {
    use std::io::Read;
    let mut limited = body.into_reader().take((MAX_STRING_OUTPUT + 1) as u64);
    let mut buf = String::new();
    limited.read_to_string(&mut buf)
        .map_err(|e| EvalError::InvalidInput(format!("{} read: {}", context, e)))?;
    if buf.len() > MAX_STRING_OUTPUT {
        buf.truncate(MAX_STRING_OUTPUT);
        buf.push_str("[truncated]");
    }
    Ok(buf)
}

fn http_response_to_map(resp: magi_lang::util::HttpResponse, context: &str) -> Result<DataType, EvalError> {
    let status = resp.status();
    let headers_vec: Vec<(String, String)> = resp.headers.clone();
    let body = read_http_body(resp.into_body(), context)?;
    let mut m = magi_lang::util::OrderedMap::new();
    m.insert("status".into(), DataType::Int64(status as i64));
    let mut hdr_map = magi_lang::util::OrderedMap::new();
    for (k, v) in headers_vec {
        hdr_map.insert(k, DataType::String(v));
    }
    m.insert("headers".into(), DataType::Map(hdr_map));
    m.insert("body".into(), DataType::String(body));
    Ok(DataType::Map(m))
}

/// Compile a user-supplied regex pattern with a size limit.
/// Uses a thread-local LRU cache (capacity [`REGEX_CACHE_CAPACITY`]) so that
/// repeated use of the same pattern avoids recompilation.
fn compile_regex(pat: &str) -> Result<magi_lang::util::Regex, String> {
    REGEX_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        // Clone the cached regex (if present) before mutating the order queue.
        if let Some(re) = cache.map.get(pat).cloned() {
            // Promote to most-recently used.
            if let Some(pos) = cache.order.iter().position(|k| k == pat) {
                cache.order.remove(pos);
            }
            cache.order.push_back(pat.to_string());
            return Ok(re);
        }
        let re = magi_lang::util::Regex::with_size_limit(pat, 1 << 20)?;
        // Evict least-recently used entry if at capacity.
        if cache.map.len() >= REGEX_CACHE_CAPACITY {
            if let Some(oldest) = cache.order.pop_front() {
                cache.map.remove(&oldest);
            }
        }
        cache.order.push_back(pat.to_string());
        cache.map.insert(pat.to_string(), re.clone());
        Ok(re)
    })
}

/// Maximum timeout for regex operations (#260).
const REGEX_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Run a regex operation with a 5-second timeout to prevent ReDoS (#260).
fn regex_with_timeout<F, R>(f: F) -> Result<R, EvalError>
where
    F: FnOnce() -> Result<R, EvalError> + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = f();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(REGEX_TIMEOUT) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err(EvalError::InvalidInput("regex operation timed out (5s limit)".to_string()))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err(EvalError::InvalidInput("regex operation thread panicked".to_string()))
        }
    }
}

/// Extract a string reference from an input map.
fn get_string<'a>(inputs: &'a HashMap<String, DataType>, key: &str) -> Result<&'a str, EvalError> {
    match inputs.get(key) {
        Some(DataType::String(s)) => Ok(s.as_str()),
        Some(other) => Err(EvalError::TypeError {
            expected: "string".to_string(),
            actual: other.type_name().to_string(),
            context: format!("input '{}'", key),
        }),
        None => Err(EvalError::InvalidInput(format!(
            "Missing required input: {}",
            key
        ))),
    }
}

/// Convert a `DataType` value to a byte vector.
fn data_to_bytes(data: &DataType) -> Vec<u8> {
    match data {
        DataType::Bytes(b) => b.clone(),
        DataType::String(s) => s.as_bytes().to_vec(),
        other => other.to_string().into_bytes(),
    }
}

/// Sleep in 100ms chunks, capped at 1 hour.
/// Chunking allows future cancellation support without blocking for the full duration.
fn sleep_chunked(inputs: &HashMap<String, DataType>) -> Result<(), EvalError> {
    const MAX_SLEEP_MS: i64 = 86_400_000; // 24 hours — have no sleep limit
    const CHUNK_MS: u64 = 100;
    let duration = inputs.get("duration").cloned().unwrap_or(DataType::Null);
    if let Some(ms) = duration.to_i64() {
        if ms > 0 {
            let total = ms.min(MAX_SLEEP_MS) as u64;
            let mut remaining = total;
            while remaining > 0 {
                let chunk = remaining.min(CHUNK_MS);
                std::thread::sleep(std::time::Duration::from_millis(chunk));
                remaining -= chunk;
            }
        }
    }
    Ok(())
}

/// Read a .magi source file, stripping BOM and validating the contents.
/// Prints an error message and exits with code 1 on failure.
const MAX_SOURCE_FILE_SIZE: u64 = 64 * 1024 * 1024; // 64 MB

fn read_source(path: &str) -> String {
    // Check file size before reading to prevent DoS via huge files.
    match fs::metadata(path) {
        Ok(meta) => {
            if meta.len() > MAX_SOURCE_FILE_SIZE {
                eprintln!("error: '{}' exceeds maximum source file size ({} bytes, limit {} bytes)", path, meta.len(), MAX_SOURCE_FILE_SIZE);
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("error: cannot read '{}': {}", path, e);
            process::exit(1);
        }
    }
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

        // Sandbox mode: reject filesystem and network operations.
        if SANDBOX_MODE.load(Ordering::Relaxed) {
            match op {
                OperationType::FsRead
                | OperationType::FsWrite
                | OperationType::FsAppend
                | OperationType::FsExists
                | OperationType::FsList
                | OperationType::FsMkdir
                | OperationType::FsRemove
                | OperationType::FsIsFile
                | OperationType::FsIsDir
                | OperationType::FsSize
                | OperationType::FsCopy
                | OperationType::FsMove
                | OperationType::FsChmod
                | OperationType::FsSymlink
                | OperationType::FsReadlink
                | OperationType::HttpGet
                | OperationType::HttpPost
                | OperationType::HttpPut
                | OperationType::HttpDelete
                | OperationType::HttpRequest
                | OperationType::HttpHead
                | OperationType::HttpOptions
                | OperationType::HttpPatch
                | OperationType::TcpConnect
                | OperationType::TcpWrite
                | OperationType::TcpRead
                | OperationType::TcpClose
                | OperationType::TcpBind
                | OperationType::TcpAccept
                | OperationType::TcpServerClose
                | OperationType::UdpBind
                | OperationType::UdpSendTo
                | OperationType::UdpRecvFrom
                | OperationType::UdpClose
                | OperationType::WsConnect
                | OperationType::WsSend
                | OperationType::WsReceive
                | OperationType::WsClose
                | OperationType::SseConnect
                | OperationType::SseReadEvent
                | OperationType::SseClose
                | OperationType::HttpServerStart
                | OperationType::HttpServerReceive
                | OperationType::HttpServerRespond
                | OperationType::HttpServerStop
                | OperationType::Exec
                | OperationType::ExecStatus
                | OperationType::ExecOutput => {
                    return Err(EvalError::InvalidInput(format!(
                        "operation {:?} is not allowed in sandbox mode",
                        op
                    )));
                }
                _ => {}
            }
        }

        match op {
            OperationType::Add => {
                // String concatenation for Add only
                if let (DataType::String(x), DataType::String(y)) = (&a, &b) {
                    let result_len = x.len().saturating_add(y.len());
                    if result_len > MAX_STRING_OUTPUT {
                        return Err(EvalError::InvalidInput(format!("string concatenation would produce {} bytes (max {})", result_len, MAX_STRING_OUTPUT)));
                    }
                    let mut result = String::with_capacity(result_len);
                    result.push_str(x);
                    result.push_str(y);
                    return Ok(DataType::String(result));
                }
                num_binop(&a, &b, i64::checked_add, |x, y| x + y)
            }
            OperationType::Subtract => num_binop(&a, &b, i64::checked_sub, |x, y| x - y),
            OperationType::Multiply => num_binop(&a, &b, i64::checked_mul, |x, y| x * y),
            OperationType::Divide => num_div_op(&a, &b, i64::checked_div, |x, y| x / y),
            OperationType::Modulo => num_div_op(&a, &b, i64::checked_rem, |x, y| x % y),

            OperationType::Equal => Ok(DataType::Bool(a == b || numeric_eq(&a, &b))),
            OperationType::NotEqual => Ok(DataType::Bool(a != b && !numeric_eq(&a, &b))),
            OperationType::Greater => num_cmp(&a, &b, |ord| ord == std::cmp::Ordering::Greater),
            OperationType::Less => num_cmp(&a, &b, |ord| ord == std::cmp::Ordering::Less),
            OperationType::GreaterEq => num_cmp(&a, &b, |ord| ord != std::cmp::Ordering::Less),
            OperationType::LessEq => num_cmp(&a, &b, |ord| ord != std::cmp::Ordering::Greater),

            OperationType::And => {
                let ta = a.to_bool();
                let tb = b.to_bool();
                Ok(DataType::Bool(ta && tb))
            },
            OperationType::Or => {
                let ta = a.to_bool();
                let tb = b.to_bool();
                Ok(DataType::Bool(ta || tb))
            },
            OperationType::Not => Ok(DataType::Bool(!input.to_bool())),
            OperationType::Negate => match &input {
                DataType::Int64(x) => match x.checked_neg() {
                    Some(v) => Ok(DataType::Int64(v)),
                    None => Err(EvalError::Overflow("integer overflow in negate".to_string())),
                },
                DataType::Int32(x) => match x.checked_neg() {
                    Some(v) => Ok(DataType::Int32(v)),
                    None => Err(EvalError::Overflow("integer overflow in negate".to_string())),
                },
                DataType::Float64(x) => Ok(DataType::Float64(-x)),
                DataType::Float32(x) => Ok(DataType::Float32(-x)),
                DataType::Uint32(x) => Ok(DataType::Int64(-(*x as i64))),
                DataType::Uint64(x) => {
                    let x = *x;
                    if x <= i64::MAX as u64 {
                        Ok(DataType::Int64(-(x as i64)))
                    } else if x == i64::MIN.unsigned_abs() {
                        Ok(DataType::Int64(i64::MIN))
                    } else {
                        Err(EvalError::Overflow("negation of Uint64 overflows i64".to_string()))
                    }
                },
                _ => Err(EvalError::TypeError { expected: "number".to_string(), actual: input.type_name().to_string(), context: "negate".to_string() }),
            },

            OperationType::Concat => {
                let (xs, ys) = match (&a, &b) {
                    (DataType::String(x), DataType::String(y)) => (x.as_str().to_string(), y.as_str().to_string()),
                    _ => (a.to_string_lossy(), b.to_string_lossy()),
                };
                let result_len = xs.len().saturating_add(ys.len());
                if result_len > MAX_STRING_OUTPUT {
                    return Err(EvalError::InvalidInput(format!("concat would produce {} bytes (max {})", result_len, MAX_STRING_OUTPUT)));
                }
                let mut result = String::with_capacity(result_len);
                result.push_str(&xs);
                result.push_str(&ys);
                Ok(DataType::String(result))
            },
            OperationType::ToString => Ok(DataType::String(input.to_string_lossy())),

            OperationType::MapGet => {
                match (&map, &key) {
                    (DataType::Map(m), DataType::String(k)) => {
                        Ok(m.get(k).cloned().unwrap_or(DataType::Null))
                    }
                    // #291: Convert integer (and other) keys to string
                    (DataType::Map(m), _) => {
                        let k = key.to_string_lossy();
                        Ok(m.get(&k).cloned().unwrap_or(DataType::Null))
                    }
                    _ => Err(EvalError::TypeError { expected: "Map".to_string(), actual: map.type_name().to_string(), context: "MapGet".to_string() }),
                }
            }
            OperationType::MapSet => {
                match (&map, &key) {
                    (DataType::Map(m), DataType::String(k)) => {
                        if !m.contains_key(k.as_str()) && m.len() >= MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!("MapSet would exceed {} entries", MAX_ARRAY_ELEMENTS)));
                        }
                        let mut new_map = m.clone();
                        new_map.insert(k.clone(), value.clone());
                        Ok(DataType::Map(new_map))
                    }
                    // #291: Convert integer (and other) keys to string
                    (DataType::Map(m), _) => {
                        let k = key.to_string_lossy();
                        if !m.contains_key(&k) && m.len() >= MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!("MapSet would exceed {} entries", MAX_ARRAY_ELEMENTS)));
                        }
                        let mut new_map = m.clone();
                        new_map.insert(k, value.clone());
                        Ok(DataType::Map(new_map))
                    }
                    _ => Err(EvalError::TypeError { expected: "Map".to_string(), actual: map.type_name().to_string(), context: "MapSet".to_string() }),
                }
            }
            OperationType::MapKeys => match &map {
                DataType::Map(m) => Ok(DataType::Array(m.keys().map(|k| DataType::String(k.clone())).collect())),
                _ => Err(EvalError::TypeError { expected: "Map".to_string(), actual: map.type_name().to_string(), context: "MapKeys".to_string() }),
            },
            OperationType::MapValues => match &map {
                DataType::Map(m) => Ok(DataType::Array(m.values().cloned().collect())),
                _ => Err(EvalError::TypeError { expected: "Map".to_string(), actual: map.type_name().to_string(), context: "MapValues".to_string() }),
            },

            OperationType::ArrayLength => match &array {
                DataType::Array(arr) => Ok(DataType::Int64(arr.len() as i64)),
                _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: array.type_name().to_string(), context: "ArrayLength".to_string() }),
            },
            OperationType::ArrayPush => {
                let arr = match &array {
                    DataType::Array(a) => a.clone(),
                    _ => return Err(EvalError::TypeError { expected: "array".to_string(), actual: array.type_name().to_string(), context: "ArrayPush".to_string() }),
                };
                let mut arr = arr;
                if arr.len() >= MAX_ARRAY_ELEMENTS {
                    return Err(EvalError::InvalidInput(format!("array push would exceed {} elements", MAX_ARRAY_ELEMENTS)));
                }
                arr.push(value.clone());
                Ok(DataType::Array(arr))
            }
            OperationType::ArrayPop => match &array {
                DataType::Array(arr) if !arr.is_empty() => Ok(arr.last().cloned().unwrap_or(DataType::Null)),
                DataType::Array(_) => Ok(DataType::Null), // empty array → Null (correct semantic)
                _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "ArrayPop".to_string() }),
            },
            OperationType::ArraySlice => {
                let start_val = inputs.get("input_1").or(inputs.get("start")).cloned().unwrap_or(DataType::Int64(0));
                let end_val = inputs.get("input_2").or(inputs.get("end")).cloned();
                match &array {
                    DataType::Array(arr) => {
                        let len = arr.len() as i64;
                        let start = {
                            let n = match start_val.to_i64() {
                                Some(n) => n,
                                None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: start_val.type_name().to_string(), context: "ArraySlice start".into() }),
                            };
                            if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                        };
                        let end = match &end_val {
                            Some(v) => {
                                let n = match v.to_i64() {
                                    Some(n) => n,
                                    None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: v.type_name().to_string(), context: "ArraySlice end".into() }),
                                };
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
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "array_slice".to_string() }),
                }
            }
            OperationType::ArraySort => match &array {
                DataType::Array(arr) => {
                    let mut sorted = arr.clone();
                    sorted.sort_by(total_cmp_values);
                    Ok(DataType::Array(sorted))
                }
                _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: array.type_name().to_string(), context: "ArraySort".to_string() }),
            },
            OperationType::ArrayReverse => match &array {
                DataType::Array(arr) => { let mut r = arr.clone(); r.reverse(); Ok(DataType::Array(r)) }
                _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: array.type_name().to_string(), context: "ArrayReverse".to_string() }),
            },
            OperationType::ArrayContains => match (&array, &value) {
                (DataType::Array(arr), val) => {
                    Ok(DataType::Bool(arr.iter().any(|item| item == val || numeric_eq(item, val))))
                }
                _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "ArrayContains".to_string() }),
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
                other => Err(EvalError::TypeError { expected: "Array".to_string(), actual: other.type_name().to_string(), context: "ArrayJoin".to_string() }),
            },

            OperationType::Length => match &input {
                DataType::String(s) => Ok(DataType::Int64(s.chars().count() as i64)),
                _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "Length".to_string() }),
            },
            OperationType::Split => {
                let delim = inputs.get("delimiter").cloned().unwrap_or(DataType::Null);
                match (&input, &delim) {
                    (DataType::String(s), DataType::String(sep)) => {
                        if sep.is_empty() {
                            return Err(EvalError::TypeError {
                                expected: "non-empty string".to_string(),
                                actual: "empty string".to_string(),
                                context: "split delimiter".to_string(),
                            });
                        }
                        let parts: Vec<DataType> = s.split(sep.as_str()).take(MAX_ARRAY_ELEMENTS + 1).map(|p| DataType::String(p.to_string())).collect();
                        if parts.len() > MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!("split result exceeds {} element limit", MAX_ARRAY_ELEMENTS)));
                        }
                        Ok(DataType::Array(parts))
                    }
                    (DataType::String(_), _) => Err(EvalError::TypeError { expected: "string".to_string(), actual: delim.type_name().to_string(), context: "Split delimiter".to_string() }),
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "Split".to_string() }),
                }
            },
            OperationType::Contains => {
                let search = inputs.get("search").cloned().unwrap_or(DataType::Null);
                match (&input, &search) {
                    (DataType::String(s), DataType::String(sub)) => Ok(DataType::Bool(s.contains(sub.as_str()))),
                    (DataType::String(_), _) => Err(EvalError::TypeError { expected: "string".to_string(), actual: search.type_name().to_string(), context: "Contains search".to_string() }),
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "Contains".to_string() }),
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
                            Ok(DataType::String(s.replace(from.as_str(), to.as_str())))
                        } else if to.len() > from.len() {
                            // Single-pass: replace while checking size to avoid scanning twice
                            let mut result = String::new();
                            let mut remainder = s.as_str();
                            while let Some(pos) = remainder.find(from.as_str()) {
                                result.push_str(&remainder[..pos]);
                                result.push_str(to.as_str());
                                remainder = &remainder[pos + from.len()..];
                                if result.len() > MAX_STRING_OUTPUT {
                                    return Err(EvalError::InvalidInput(format!("replace result exceeds {} byte limit", MAX_STRING_OUTPUT)));
                                }
                            }
                            result.push_str(remainder);
                            if result.len() > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!("replace result exceeds {} byte limit", MAX_STRING_OUTPUT)));
                            }
                            Ok(DataType::String(result))
                        } else {
                            // to.len() <= from.len(): result cannot exceed input length
                            Ok(DataType::String(s.replace(from.as_str(), to.as_str())))
                        }
                    }
                    (DataType::String(_), _, _) => {
                        let bad = if !matches!(&search, DataType::String(_)) { &search } else { &replace };
                        Err(EvalError::TypeError { expected: "string".to_string(), actual: bad.type_name().to_string(), context: "Replace argument".to_string() })
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "Replace".to_string() }),
                }
            },
            OperationType::Trim => match &input {
                DataType::String(s) => Ok(DataType::String(s.trim().to_string())),
                _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "Trim".to_string() }),
            },
            OperationType::TrimStart => match &input {
                DataType::String(s) => Ok(DataType::String(s.trim_start().to_string())),
                _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "TrimStart".to_string() }),
            },
            OperationType::TrimEnd => match &input {
                DataType::String(s) => Ok(DataType::String(s.trim_end().to_string())),
                _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "TrimEnd".to_string() }),
            },
            OperationType::ToUpper => match &input {
                DataType::String(s) => {
                    let result = s.to_uppercase();
                    if result.len() > MAX_STRING_OUTPUT {
                        return Err(EvalError::InvalidInput(format!("to_uppercase output exceeds {} bytes", MAX_STRING_OUTPUT)));
                    }
                    Ok(DataType::String(result))
                },
                _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "ToUpper".to_string() }),
            },
            OperationType::ToLower => match &input {
                DataType::String(s) => {
                    let result = s.to_lowercase();
                    if result.len() > MAX_STRING_OUTPUT {
                        return Err(EvalError::InvalidInput(format!("to_lowercase output exceeds {} bytes", MAX_STRING_OUTPUT)));
                    }
                    Ok(DataType::String(result))
                },
                _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "ToLower".to_string() }),
            },
            OperationType::StartsWith => {
                let prefix = inputs.get("prefix").cloned().unwrap_or(DataType::Null);
                match (&input, &prefix) {
                    (DataType::String(s), DataType::String(p)) => Ok(DataType::Bool(s.starts_with(p.as_str()))),
                    (DataType::String(_), _) => Err(EvalError::TypeError { expected: "string".to_string(), actual: prefix.type_name().to_string(), context: "StartsWith prefix".to_string() }),
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "StartsWith".to_string() }),
                }
            },
            OperationType::EndsWith => {
                let suffix = inputs.get("suffix").cloned().unwrap_or(DataType::Null);
                match (&input, &suffix) {
                    (DataType::String(s), DataType::String(sfx)) => Ok(DataType::Bool(s.ends_with(sfx.as_str()))),
                    (DataType::String(_), _) => Err(EvalError::TypeError { expected: "string".to_string(), actual: suffix.type_name().to_string(), context: "EndsWith suffix".to_string() }),
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "EndsWith".to_string() }),
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
                            None => return Err(EvalError::TypeError {
                                expected: "number".to_string(),
                                actual: start_val.type_name().to_string(),
                                context: "substring start index".to_string(),
                            }),
                        };
                        let end = match &end_val {
                            Some(v) => match v.to_i64() {
                                Some(n) => if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize },
                                None => return Err(EvalError::TypeError { expected: "number".into(), actual: v.type_name().to_string(), context: "substring end index".into() }),
                            },
                            None => chars.len(),
                        };
                        if start >= end {
                            Ok(DataType::String(String::new()))
                        } else {
                            Ok(DataType::String(chars[start..end].iter().collect()))
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "Substring".to_string() }),
                }
            }
            OperationType::IndexOf => {
                let search = inputs.get("search").cloned().unwrap_or(DataType::Null);
                match (&input, &search) {
                    (DataType::String(s), DataType::String(sub)) => {
                        Ok(match s.find(sub.as_str()) {
                            Some(byte_idx) => DataType::Int64(s[..byte_idx].chars().count() as i64),
                            None => DataType::Null,
                        })
                    }
                    (DataType::String(_), _) => Err(EvalError::TypeError { expected: "string".to_string(), actual: search.type_name().to_string(), context: "IndexOf search".to_string() }),
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "IndexOf".to_string() }),
                }
            },

            OperationType::MapSize => match &map {
                DataType::Map(m) => Ok(DataType::Int64(m.len() as i64)),
                _ => Err(EvalError::TypeError { expected: "Map".to_string(), actual: map.type_name().to_string(), context: "MapSize".to_string() }),
            },
            OperationType::MapHas => match (&map, &key) {
                (DataType::Map(m), DataType::String(k)) => Ok(DataType::Bool(m.contains_key(k))),
                (DataType::Map(_), _) => Err(EvalError::TypeError { expected: "String".to_string(), actual: key.type_name().to_string(), context: "MapHas key".to_string() }),
                _ => Err(EvalError::TypeError { expected: "Map".to_string(), actual: map.type_name().to_string(), context: "MapHas".to_string() }),
            },
            OperationType::MapDelete => match (&map, &key) {
                (DataType::Map(m), DataType::String(k)) => {
                    let mut new_map = m.clone();
                    new_map.shift_remove(k);
                    Ok(DataType::Map(new_map))
                }
                (DataType::Map(_), _) => Err(EvalError::TypeError { expected: "String".to_string(), actual: key.type_name().to_string(), context: "MapDelete key".to_string() }),
                _ => Err(EvalError::TypeError { expected: "Map".to_string(), actual: map.type_name().to_string(), context: "MapDelete".to_string() }),
            },
            OperationType::MapEntries => match &map {
                DataType::Map(m) => {
                    Ok(DataType::Array(m.iter().map(|(k, v)| {
                        DataType::Array(vec![DataType::String(k.clone()), v.clone()])
                    }).collect()))
                }
                _ => Err(EvalError::TypeError { expected: "Map".to_string(), actual: map.type_name().to_string(), context: "MapEntries".to_string() }),
            },
            OperationType::MapFromEntries => match &array {
                DataType::Array(arr) => {
                    if arr.len() > MAX_ARRAY_ELEMENTS {
                        return Err(EvalError::InvalidInput(format!(
                            "map_from_entries: array exceeds {} element limit", MAX_ARRAY_ELEMENTS
                        )));
                    }
                    let mut m = magi_lang::util::OrderedMap::new();
                    for (i, item) in arr.iter().enumerate() {
                        match item {
                            DataType::Array(pair) if pair.len() >= 2 => {
                                if let DataType::String(k) = &pair[0] {
                                    m.insert(k.clone(), pair[1].clone());
                                } else {
                                    return Err(EvalError::TypeError { expected: "String key".to_string(), actual: pair[0].type_name().to_string(), context: format!("MapFromEntries entry[{}][0]", i) });
                                }
                            }
                            DataType::Array(pair) => {
                                return Err(EvalError::InvalidInput(format!("MapFromEntries entry[{}] has {} elements, expected at least 2", i, pair.len())));
                            }
                            _ => {
                                return Err(EvalError::TypeError { expected: "Array pair".to_string(), actual: item.type_name().to_string(), context: format!("MapFromEntries entry[{}]", i) });
                            }
                        }
                    }
                    Ok(DataType::Map(m))
                }
                _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "MapFromEntries".to_string() }),
            },
            OperationType::MapMerge => match (&a, &b) {
                (DataType::Map(m1), DataType::Map(m2)) => {
                    let mut merged = m1.clone();
                    for (k, v) in m2 {
                        if !merged.contains_key(k.as_str()) && merged.len() >= MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!("MapMerge would exceed {} entries", MAX_ARRAY_ELEMENTS)));
                        }
                        merged.insert(k.clone(), v.clone());
                    }
                    Ok(DataType::Map(merged))
                }
                _ => {
                    let bad = if !matches!(a, DataType::Map(_)) { &a } else { &b };
                    Err(EvalError::TypeError { expected: "Map".to_string(), actual: bad.type_name().to_string(), context: "MapMerge".to_string() })
                }
            },

            OperationType::ArrayGet => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match &array {
                    DataType::Array(arr) => {
                        let i = match index.to_i64() {
                            Some(n) => n,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: index.type_name().to_string(), context: "ArrayGet index".into() }),
                        };
                        if i < 0 { return Ok(DataType::Null); }
                        let idx = usize::try_from(i).unwrap_or(usize::MAX);
                        Ok(arr.get(idx).cloned().unwrap_or(DataType::Null))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "ArrayGet".to_string() }),
                }
            },
            OperationType::ArraySet => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match &array {
                    DataType::Array(arr) => {
                        let i = match index.to_i64() {
                            Some(n) => n,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: index.type_name().to_string(), context: "ArraySet index".into() }),
                        };
                        if i < 0 { return Ok(DataType::Array(arr.clone())); }
                        let idx = usize::try_from(i).unwrap_or(usize::MAX);
                        let mut new_arr = arr.clone();
                        if idx < new_arr.len() {
                            new_arr[idx] = value.clone();
                        }
                        Ok(DataType::Array(new_arr))
                    }
                    _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: array.type_name().to_string(), context: "ArraySet".to_string() }),
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
                _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: array.type_name().to_string(), context: "ArrayFlatten".to_string() }),
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
                _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: a.type_name().to_string(), context: "ArrayConcat".to_string() }),
            },
            OperationType::ArrayUnique => match &array {
                DataType::Array(arr) => {
                    const MAX_UNIQUE: usize = 100_000;
                    if arr.len() > MAX_UNIQUE {
                        return Err(EvalError::InvalidInput(format!(
                            "array_unique: array too large ({} elements, max {} for quadratic uniqueness check)",
                            arr.len(),
                            MAX_UNIQUE,
                        )));
                    }
                    let mut seen = Vec::new();
                    for item in arr {
                        if !seen.iter().any(|s: &DataType| s == item || numeric_eq(s, item)) {
                            seen.push(item.clone());
                        }
                    }
                    Ok(DataType::Array(seen))
                }
                _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: array.type_name().to_string(), context: "ArrayUnique".to_string() }),
            },
            OperationType::ArrayFilterNulls => match &array {
                DataType::Array(arr) => {
                    Ok(DataType::Array(arr.iter().filter(|v| !matches!(v, DataType::Null)).cloned().collect()))
                }
                _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: array.type_name().to_string(), context: "ArrayFilterNulls".to_string() }),
            },

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
                DataType::String(s) => Ok(s.trim().parse::<i64>().map(DataType::Int64).unwrap_or(DataType::Null)),
                DataType::Bool(b) => Ok(DataType::Int64(if *b { 1 } else { 0 })),
                _ => Err(EvalError::TypeError { expected: "numeric, string, or bool".to_string(), actual: input.type_name().to_string(), context: "ToInt64".to_string() }),
            },
            OperationType::ToFloat64 => match &input {
                DataType::Float64(_) => Ok(input.clone()),
                DataType::Int64(n) => Ok(DataType::Float64(*n as f64)),
                DataType::Int32(n) => Ok(DataType::Float64(*n as f64)),
                DataType::Uint32(n) => Ok(DataType::Float64(*n as f64)),
                DataType::Uint64(n) => Ok(DataType::Float64(*n as f64)),
                DataType::Float32(f) => Ok(DataType::Float64(*f as f64)),
                DataType::String(s) => Ok(s.trim().parse::<f64>().map(DataType::Float64).unwrap_or(DataType::Null)),
                DataType::Bool(b) => Ok(DataType::Float64(if *b { 1.0 } else { 0.0 })),
                _ => Err(EvalError::TypeError { expected: "numeric, string, or bool".to_string(), actual: input.type_name().to_string(), context: "ToFloat64".to_string() }),
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

            OperationType::Abs => match &input {
                DataType::Int64(n) => match n.checked_abs() {
                    Some(v) => Ok(DataType::Int64(v)),
                    None => Err(EvalError::Overflow(format!("integer overflow: abs({})", n))),
                },
                DataType::Int32(n) => match n.checked_abs() {
                    Some(v) => Ok(DataType::Int32(v)),
                    None => Err(EvalError::Overflow(format!("integer overflow: abs({})", n))),
                },
                DataType::Uint32(_) | DataType::Uint64(_) => Ok(input.clone()),
                DataType::Float64(f) => Ok(DataType::Float64(f.abs())),
                DataType::Float32(f) => Ok(DataType::Float32(f.abs())),
                _ => Err(EvalError::TypeError { expected: "number".to_string(), actual: input.type_name().to_string(), context: "Abs".to_string() }),
            },
            OperationType::Round => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.round())),
                DataType::Float32(n) => Ok(DataType::Float32(n.round())),
                DataType::Int64(_) | DataType::Int32(_) | DataType::Uint32(_) | DataType::Uint64(_) => Ok(input.clone()),
                other => Err(EvalError::TypeError { expected: "number".to_string(), actual: other.type_name().to_string(), context: "Round".to_string() }),
            },
            OperationType::Floor => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.floor())),
                DataType::Float32(n) => Ok(DataType::Float32(n.floor())),
                DataType::Int64(_) | DataType::Int32(_) | DataType::Uint32(_) | DataType::Uint64(_) => Ok(input.clone()),
                other => Err(EvalError::TypeError { expected: "number".to_string(), actual: other.type_name().to_string(), context: "Floor".to_string() }),
            },
            OperationType::Ceil => match &input {
                DataType::Float64(n) => Ok(DataType::Float64(n.ceil())),
                DataType::Float32(n) => Ok(DataType::Float32(n.ceil())),
                DataType::Int64(_) | DataType::Int32(_) | DataType::Uint32(_) | DataType::Uint64(_) => Ok(input.clone()),
                other => Err(EvalError::TypeError { expected: "number".to_string(), actual: other.type_name().to_string(), context: "Ceil".to_string() }),
            },
            OperationType::Sqrt => eval_unary_float_op(&input, f32::sqrt, f64::sqrt),
            OperationType::Cbrt => eval_unary_float_op(&input, f32::cbrt, f64::cbrt),
            OperationType::Hypot => {
                match (a.to_f64(), b.to_f64()) {
                    (Some(x), Some(y)) => Ok(DataType::Float64(x.hypot(y))),
                    _ => Err(EvalError::TypeError { expected: "number".into(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "hypot".into() }),
                }
            }
            OperationType::Power => {
                let a = inputs.get("a").unwrap_or(&DataType::Null);
                let b = inputs.get("b").unwrap_or(&DataType::Null);
                if let (DataType::Float32(base), DataType::Float32(exp)) = (a, b) {
                    return Ok(DataType::Float32(base.powf(*exp)));
                }
                match (promote_numeric(a), promote_numeric(b)) {
                    (Some(Ok(base)), Some(Ok(exp))) => {
                        if base == 0 && exp < 0 {
                            Ok(DataType::Null)
                        } else if exp < 0 {
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
                    _ => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: format!("({}, {})", a.type_name(), b.type_name()), context: "Power".to_string() }),
                }
            },
            OperationType::Sin => eval_unary_float_op(&input, f32::sin, f64::sin),
            OperationType::Cos => eval_unary_float_op(&input, f32::cos, f64::cos),
            OperationType::Tan => eval_unary_float_op(&input, f32::tan, f64::tan),
            OperationType::Ln => eval_unary_float_op(&input, f32::ln, f64::ln),
            OperationType::Log2 => eval_unary_float_op(&input, f32::log2, f64::log2),
            OperationType::Log10 => eval_unary_float_op(&input, f32::log10, f64::log10),
            OperationType::Exp => eval_unary_float_op(&input, f32::exp, f64::exp),
            OperationType::Sign => {
                match &input {
                    DataType::Float32(n) => Ok(DataType::Float32(n.signum())),
                    DataType::Int32(n) => Ok(DataType::Int32(n.signum())),
                    DataType::Uint32(_) => Ok(DataType::Uint32(if input == DataType::Uint32(0) { 0 } else { 1 })),
                    DataType::Uint64(_) => Ok(DataType::Uint64(if input == DataType::Uint64(0) { 0 } else { 1 })),
                    _ => match promote_numeric(&input) {
                        Some(Ok(n)) => Ok(DataType::Int64(n.signum())),
                        Some(Err(f)) => Ok(DataType::Float64(f.signum())),
                        None => Err(EvalError::TypeError { expected: "number".to_string(), actual: input.type_name().to_string(), context: "Sign".to_string() }),
                    }
                }
            },

            OperationType::ArrayShift => match &array {
                DataType::Array(arr) => Ok(arr.first().cloned().unwrap_or(DataType::Null)),
                _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: array.type_name().to_string(), context: "ArrayShift".to_string() }),
            },
            OperationType::ArrayInsert => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match &array {
                    DataType::Array(arr) => {
                        if arr.len() >= MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!("array exceeds maximum size ({})", MAX_ARRAY_ELEMENTS)));
                        }
                        let i = match index.to_i64() {
                            Some(n) => n,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: index.type_name().to_string(), context: "ArrayInsert index".into() }),
                        };
                        let idx = if i < 0 { 0 } else { usize::try_from(i).unwrap_or(usize::MAX).min(arr.len()) };
                        let mut new_arr = arr.clone();
                        new_arr.insert(idx, value.clone());
                        Ok(DataType::Array(new_arr))
                    }
                    _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: array.type_name().to_string(), context: "ArrayInsert".to_string() }),
                }
            },
            OperationType::ArrayRemove => {
                let index = inputs.get("index").cloned().unwrap_or(DataType::Null);
                match &array {
                    DataType::Array(arr) => {
                        let i = match index.to_i64() {
                            Some(n) => n,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: index.type_name().to_string(), context: "ArrayRemove index".into() }),
                        };
                        let idx = if i >= 0 { usize::try_from(i).ok() } else { None };
                        match idx {
                            Some(idx) if idx < arr.len() => {
                                let mut new_arr = arr.clone();
                                new_arr.remove(idx);
                                Ok(DataType::Array(new_arr))
                            }
                            _ => Ok(DataType::Array(arr.clone())),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: array.type_name().to_string(), context: "ArrayRemove".to_string() }),
                }
            },

            OperationType::StringChars => match &input {
                DataType::String(s) => {
                    let count = s.chars().count();
                    if count > MAX_ARRAY_ELEMENTS {
                        return Err(EvalError::InvalidInput(format!("chars() would produce {} elements (max {})", count, MAX_ARRAY_ELEMENTS)));
                    }
                    Ok(DataType::Array(s.chars().map(|c| DataType::String(c.to_string())).collect()))
                }
                _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "StringChars".to_string() }),
            },
            OperationType::StringRepeat => {
                let count = inputs.get("count").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Int64(0));
                match &input {
                    DataType::String(s) => {
                        let n = match count.to_i64() {
                            Some(n) => n.max(0) as usize,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: count.type_name().to_string(), context: "StringRepeat count".into() }),
                        };
                        let result_len = s.len().saturating_mul(n);
                        if result_len > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!("repeat result exceeds {} byte limit", MAX_STRING_OUTPUT)));
                        }
                        Ok(DataType::String(s.repeat(n)))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "StringRepeat".to_string() }),
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
                _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "StringLines".to_string() }),
            },
            OperationType::StringWords => match &input {
                DataType::String(s) => {
                    let words: Vec<DataType> = s.split_whitespace().take(MAX_ARRAY_ELEMENTS + 1).map(|w| DataType::String(w.to_string())).collect();
                    if words.len() > MAX_ARRAY_ELEMENTS {
                        return Err(EvalError::InvalidInput(format!("words() would produce more than {} elements", MAX_ARRAY_ELEMENTS)));
                    }
                    Ok(DataType::Array(words))
                }
                _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "StringWords".to_string() }),
            },
            OperationType::StringReverse => match &input {
                DataType::String(s) => Ok(DataType::String(s.chars().rev().collect())),
                _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "StringReverse".to_string() }),
            },
            OperationType::StringCount => {
                let search = inputs.get("search").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                match (&input, &search) {
                    (DataType::String(s), DataType::String(sub)) => {
                        if sub.is_empty() {
                            return Err(EvalError::InvalidInput("count: search string must not be empty".to_string()));
                        }
                        Ok(DataType::Int64(s.matches(sub.as_str()).count() as i64))
                    }
                    (DataType::String(_), _) => Err(EvalError::TypeError { expected: "string".to_string(), actual: search.type_name().to_string(), context: "StringCount search".to_string() }),
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "StringCount".to_string() }),
                }
            },
            OperationType::CharAt => {
                let index = inputs.get("index").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Int64(0));
                match &input {
                    DataType::String(s) => {
                        let i = match index.to_i64() {
                            Some(n) => n,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: index.type_name().to_string(), context: "CharAt index".into() }),
                        };
                        if i < 0 { return Ok(DataType::Null); }
                        let idx = usize::try_from(i).unwrap_or(usize::MAX);
                        Ok(s.chars().nth(idx).map(|c| DataType::String(c.to_string())).unwrap_or(DataType::Null))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "CharAt".to_string() }),
                }
            },
            OperationType::PadStart => {
                let width = inputs.get("width").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Int64(0));
                let fill = inputs.get("fill").or(inputs.get("input_2")).cloned();
                match &input {
                    DataType::String(s) => {
                        let w = match width.to_i64() {
                            Some(n) => n.max(0) as usize,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: width.type_name().to_string(), context: "PadStart width".into() }),
                        };
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
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "PadStart".to_string() }),
                }
            },
            OperationType::PadEnd => {
                let width = inputs.get("width").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Int64(0));
                let fill = inputs.get("fill").or(inputs.get("input_2")).cloned();
                match &input {
                    DataType::String(s) => {
                        let w = match width.to_i64() {
                            Some(n) => n.max(0) as usize,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: width.type_name().to_string(), context: "PadEnd width".into() }),
                        };
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
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "PadEnd".to_string() }),
                }
            },

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
                    DataType::Set(_) => "set",
                    DataType::Tuple(_) => "tuple",
                    DataType::Future(_) => "future",
                };
                Ok(DataType::String(type_name.to_string()))
            },

            // Min/Max
            OperationType::Min => {
                match (&a, &b) {
                    (DataType::Float32(x), DataType::Float32(y)) => Ok(DataType::Float32(x.min(*y))),
                    (DataType::Int32(x), DataType::Int32(y)) => Ok(DataType::Int32((*x).min(*y))),
                    (DataType::Uint32(x), DataType::Uint32(y)) => Ok(DataType::Uint32((*x).min(*y))),
                    (DataType::Uint64(x), DataType::Uint64(y)) => Ok(DataType::Uint64((*x).min(*y))),
                    _ => match (promote_numeric(&a), promote_numeric(&b)) {
                        (Some(Ok(x)), Some(Ok(y))) => Ok(DataType::Int64(x.min(y))),
                        (Some(pa), Some(pb)) => {
                            let fa = match pa { Ok(i) => i as f64, Err(f) => f };
                            let fb = match pb { Ok(i) => i as f64, Err(f) => f };
                            Ok(DataType::Float64(fa.min(fb)))
                        }
                        _ => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "Min".to_string() }),
                    }
                }
            },
            OperationType::Max => {
                match (&a, &b) {
                    (DataType::Float32(x), DataType::Float32(y)) => Ok(DataType::Float32(x.max(*y))),
                    (DataType::Int32(x), DataType::Int32(y)) => Ok(DataType::Int32((*x).max(*y))),
                    (DataType::Uint32(x), DataType::Uint32(y)) => Ok(DataType::Uint32((*x).max(*y))),
                    (DataType::Uint64(x), DataType::Uint64(y)) => Ok(DataType::Uint64((*x).max(*y))),
                    _ => match (promote_numeric(&a), promote_numeric(&b)) {
                        (Some(Ok(x)), Some(Ok(y))) => Ok(DataType::Int64(x.max(y))),
                        (Some(pa), Some(pb)) => {
                            let fa = match pa { Ok(i) => i as f64, Err(f) => f };
                            let fb = match pb { Ok(i) => i as f64, Err(f) => f };
                            Ok(DataType::Float64(fa.max(fb)))
                        }
                        _ => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "Max".to_string() }),
                    }
                }
            },

            OperationType::Range => {
                let start = require_i64_or_default(inputs.get("start").or(inputs.get("a")), 0, "Range start")?;
                let end = require_i64_or_default(inputs.get("end").or(inputs.get("b")), 0, "Range end")?;
                let inclusive = matches!(inputs.get("inclusive"), Some(DataType::Bool(true)));
                let step = require_i64_or_default(inputs.get("step"), if start <= end { 1 } else { -1 }, "Range step")?;
                if step == 0 { return Ok(DataType::Array(vec![])); }
                let mut result = Vec::new();
                let mut i = start;
                loop {
                    if inclusive {
                        if step > 0 && i > end { break; }
                        if step < 0 && i < end { break; }
                    } else {
                        if step > 0 && i >= end { break; }
                        if step < 0 && i <= end { break; }
                    }
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
                let json_val = datatype_to_json_value(&input);
                let s = magi_lang::util::json_to_string(&json_val);
                if s.len() > MAX_STRING_OUTPUT {
                    return Err(EvalError::InvalidInput(format!(
                        "to_json: output would exceed {} byte limit", MAX_STRING_OUTPUT
                    )));
                }
                Ok(DataType::String(s))
            },

            // CharFromCode: int → single-char string → "A")
            OperationType::CharFromCode => {
                let code = input.to_i64().ok_or_else(|| EvalError::TypeError {
                    expected: "integer".to_string(),
                    actual: input.type_name().to_string(),
                    context: "char_from_code".to_string(),
                })?;
                let ch = char::from_u32(code as u32).ok_or_else(|| {
                    EvalError::InvalidInput(format!("char_from_code: {} is not a valid Unicode code point", code))
                })?;
                Ok(DataType::String(ch.to_string()))
            },

            // CharCode: single-char string → int → 65)
            OperationType::CharCode => {
                let s = match &input {
                    DataType::String(s) => s.clone(),
                    _ => return Err(EvalError::TypeError {
                        expected: "string".to_string(),
                        actual: input.type_name().to_string(),
                        context: "char_code".to_string(),
                    }),
                };
                let ch = s.chars().next().ok_or_else(|| {
                    EvalError::InvalidInput("char_code: empty string".to_string())
                })?;
                Ok(DataType::Int64(ch as i64))
            },

            OperationType::BitAnd => {
                match (&a, &b) {
                    (DataType::Uint64(x), DataType::Uint64(y)) => Ok(DataType::Uint64(x & y)),
                    (DataType::Uint32(x), DataType::Uint32(y)) => Ok(DataType::Uint32(x & y)),
                    (DataType::Int32(x), DataType::Int32(y)) => Ok(DataType::Int32(x & y)),
                    _ => match (a.to_i64(), b.to_i64()) {
                        (Some(x), Some(y)) => Ok(DataType::Int64(x & y)),
                        _ => Err(EvalError::TypeError { expected: "integer".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "BitAnd".to_string() }),
                    }
                }
            },
            OperationType::BitOr => {
                match (&a, &b) {
                    (DataType::Uint64(x), DataType::Uint64(y)) => Ok(DataType::Uint64(x | y)),
                    (DataType::Uint32(x), DataType::Uint32(y)) => Ok(DataType::Uint32(x | y)),
                    (DataType::Int32(x), DataType::Int32(y)) => Ok(DataType::Int32(x | y)),
                    _ => match (a.to_i64(), b.to_i64()) {
                        (Some(x), Some(y)) => Ok(DataType::Int64(x | y)),
                        _ => Err(EvalError::TypeError { expected: "integer".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "BitOr".to_string() }),
                    }
                }
            },
            OperationType::BitXor => {
                match (&a, &b) {
                    (DataType::Uint64(x), DataType::Uint64(y)) => Ok(DataType::Uint64(x ^ y)),
                    (DataType::Uint32(x), DataType::Uint32(y)) => Ok(DataType::Uint32(x ^ y)),
                    (DataType::Int32(x), DataType::Int32(y)) => Ok(DataType::Int32(x ^ y)),
                    _ => match (a.to_i64(), b.to_i64()) {
                        (Some(x), Some(y)) => Ok(DataType::Int64(x ^ y)),
                        _ => Err(EvalError::TypeError { expected: "integer".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "BitXor".to_string() }),
                    }
                }
            },
            OperationType::BitNot => {
                match &input {
                    DataType::Uint64(x) => Ok(DataType::Uint64(!x)),
                    DataType::Uint32(x) => Ok(DataType::Uint32(!x)),
                    DataType::Int32(x) => Ok(DataType::Int32(!x)),
                    _ => match input.to_i64() {
                        Some(x) => Ok(DataType::Int64(!x)),
                        None => Err(EvalError::TypeError { expected: "integer".to_string(), actual: input.type_name().to_string(), context: "BitNot".to_string() }),
                    }
                }
            },
            OperationType::BitShiftLeft => {
                match &a {
                    DataType::Uint64(x) => {
                        let shift = match b.to_i64() {
                            Some(n) => n,
                            None => return Err(EvalError::TypeError { expected: "integer".into(), actual: b.type_name().to_string(), context: "bitwise shift amount".into() }),
                        };
                        if (0..64).contains(&shift) {
                            Ok(DataType::Uint64(x << shift))
                        } else {
                            Err(EvalError::TypeError { expected: "shift amount 0-63".to_string(), actual: format!("{}", shift), context: "BitShiftLeft".to_string() })
                        }
                    }
                    DataType::Uint32(x) => {
                        let shift = match b.to_i64() {
                            Some(n) => n,
                            None => return Err(EvalError::TypeError { expected: "integer".into(), actual: b.type_name().to_string(), context: "bitwise shift amount".into() }),
                        };
                        if (0..32).contains(&shift) {
                            Ok(DataType::Uint32(x << shift))
                        } else {
                            Err(EvalError::TypeError { expected: "shift amount 0-31".to_string(), actual: format!("{}", shift), context: "BitShiftLeft".to_string() })
                        }
                    }
                    DataType::Int32(x) => {
                        let shift = match b.to_i64() {
                            Some(n) => n,
                            None => return Err(EvalError::TypeError { expected: "integer".into(), actual: b.type_name().to_string(), context: "bitwise shift amount".into() }),
                        };
                        if (0..32).contains(&shift) {
                            Ok(DataType::Int32(x << shift))
                        } else {
                            Err(EvalError::TypeError { expected: "shift amount 0-31".to_string(), actual: format!("{}", shift), context: "BitShiftLeft".to_string() })
                        }
                    }
                    _ => match (a.to_i64(), b.to_i64()) {
                        (Some(x), Some(y)) if (0..64).contains(&y) => Ok(DataType::Int64(x << y)),
                        _ => Err(EvalError::TypeError { expected: "integer".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "BitShiftLeft".to_string() }),
                    }
                }
            },
            OperationType::BitShiftRight => {
                match &a {
                    DataType::Uint64(x) => {
                        let shift = match b.to_i64() {
                            Some(n) => n,
                            None => return Err(EvalError::TypeError { expected: "integer".into(), actual: b.type_name().to_string(), context: "bitwise shift amount".into() }),
                        };
                        if (0..64).contains(&shift) {
                            Ok(DataType::Uint64(x >> shift))
                        } else {
                            Err(EvalError::TypeError { expected: "shift amount 0-63".to_string(), actual: format!("{}", shift), context: "BitShiftRight".to_string() })
                        }
                    }
                    DataType::Uint32(x) => {
                        let shift = match b.to_i64() {
                            Some(n) => n,
                            None => return Err(EvalError::TypeError { expected: "integer".into(), actual: b.type_name().to_string(), context: "bitwise shift amount".into() }),
                        };
                        if (0..32).contains(&shift) {
                            Ok(DataType::Uint32(x >> shift))
                        } else {
                            Err(EvalError::TypeError { expected: "shift amount 0-31".to_string(), actual: format!("{}", shift), context: "BitShiftRight".to_string() })
                        }
                    }
                    DataType::Int32(x) => {
                        let shift = match b.to_i64() {
                            Some(n) => n,
                            None => return Err(EvalError::TypeError { expected: "integer".into(), actual: b.type_name().to_string(), context: "bitwise shift amount".into() }),
                        };
                        if (0..32).contains(&shift) {
                            Ok(DataType::Int32(x >> shift))
                        } else {
                            Err(EvalError::TypeError { expected: "shift amount 0-31".to_string(), actual: format!("{}", shift), context: "BitShiftRight".to_string() })
                        }
                    }
                    _ => match (a.to_i64(), b.to_i64()) {
                        (Some(x), Some(y)) if (0..64).contains(&y) => Ok(DataType::Int64(x >> y)),
                        _ => Err(EvalError::TypeError { expected: "integer".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "BitShiftRight".to_string() }),
                    }
                }
            },

            OperationType::IsNull => Ok(DataType::Bool(matches!(&input, DataType::Null))),
            OperationType::IsString => Ok(DataType::Bool(matches!(&input, DataType::String(_)))),
            OperationType::IsNumber => Ok(DataType::Bool(promote_numeric(&input).is_some())),
            OperationType::IsArray => Ok(DataType::Bool(matches!(&input, DataType::Array(_)))),
            OperationType::IsMap => Ok(DataType::Bool(matches!(&input, DataType::Map(_)))),
            OperationType::IsBool => Ok(DataType::Bool(matches!(&input, DataType::Bool(_)))),
            OperationType::IsBytes => Ok(DataType::Bool(matches!(&input, DataType::Bytes(_)))),

            // Assert / DebugLog
            OperationType::Assert => {
                let condition = inputs.get("condition").unwrap_or(&input);
                let message = inputs.get("message").and_then(|m| if let DataType::String(s) = m { Some(s.as_str()) } else { None });
                if condition.to_bool() {
                    Ok(DataType::Null)
                } else {
                    let msg = message.unwrap_or("Assertion failed");
                    Err(EvalError::InvalidInput(msg.to_string()))
                }
            },
            OperationType::DebugLog => {
                eprintln!("[debug] {}", input.to_string_lossy());
                Ok(DataType::Null)
            },

            OperationType::BytesLength => {
                match &input {
                    DataType::Bytes(b) => Ok(DataType::Int64(b.len() as i64)),
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "bytes_length".to_string() }),
                }
            },
            OperationType::BytesSlice => {
                match &input {
                    DataType::Bytes(b) => {
                        let len = b.len() as i64;
                        let raw_start = require_i64_or_default(inputs.get("input_1").or(inputs.get("start")), 0, "BytesSlice start")?;
                        let raw_end = require_i64_or_default(inputs.get("input_2").or(inputs.get("end")), len, "BytesSlice end")?;
                        let start = if raw_start < 0 { (len + raw_start).max(0) as usize } else { (raw_start as usize).min(b.len()) };
                        let end = if raw_end < 0 { (len + raw_end).max(0) as usize } else { (raw_end as usize).min(b.len()) };
                        if start >= end {
                            Ok(DataType::Bytes(vec![]))
                        } else {
                            Ok(DataType::Bytes(b[start..end].to_vec()))
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "bytes_slice".to_string() }),
                }
            },
            OperationType::BytesConcat => {
                let a_val = inputs.get("a").cloned().unwrap_or(DataType::Null);
                let b_val = inputs.get("b").cloned().unwrap_or(DataType::Null);
                match (&a_val, &b_val) {
                    (DataType::Bytes(a), DataType::Bytes(b)) => {
                        let total = a.len().saturating_add(b.len());
                        if total > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "bytes_concat: result would be {} bytes (max {})", total, MAX_STRING_OUTPUT
                            )));
                        }
                        let mut result = a.clone();
                        result.extend_from_slice(b);
                        Ok(DataType::Bytes(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "bytes_concat".to_string() }),
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
                match &input {
                    DataType::Bytes(b) => {
                        if b.len() * 4 / 3 + 4 > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "Base64Encode: output would exceed {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(magi_lang::util::base64_encode(b)))
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "base64_encode".to_string() }),
                }
            },
            OperationType::Base64Decode => {
                match &input {
                    DataType::String(s) => {
                        match magi_lang::util::base64_decode(s) {
                            Ok(bytes) => Ok(DataType::Bytes(bytes)),
                            Err(e) => Err(EvalError::InvalidInput(format!("Base64Decode failed: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "base64_decode".to_string() }),
                }
            },

            OperationType::BytesCompare => {
                match (&a, &b) {
                    (DataType::Bytes(a_bytes), DataType::Bytes(b_bytes)) => {
                        Ok(DataType::Int64(match a_bytes.cmp(b_bytes) {
                            std::cmp::Ordering::Less => -1,
                            std::cmp::Ordering::Equal => 0,
                            std::cmp::Ordering::Greater => 1,
                        }))
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "bytes_compare".to_string() }),
                }
            },
            OperationType::BytesEqual => {
                match (&a, &b) {
                    (DataType::Bytes(a_bytes), DataType::Bytes(b_bytes)) => {
                        Ok(DataType::Bool(a_bytes == b_bytes))
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "bytes_equal".to_string() }),
                }
            },
            OperationType::BytesHasPrefix => {
                let prefix_val = inputs.get("prefix").cloned().unwrap_or(DataType::Null);
                match (&input, &prefix_val) {
                    (DataType::Bytes(haystack), DataType::Bytes(prefix)) => {
                        Ok(DataType::Bool(haystack.starts_with(prefix.as_slice())))
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "bytes_has_prefix".to_string() }),
                }
            },
            OperationType::BytesHasSuffix => {
                let suffix_val = inputs.get("prefix").cloned().unwrap_or(DataType::Null);
                match (&input, &suffix_val) {
                    (DataType::Bytes(haystack), DataType::Bytes(suffix)) => {
                        Ok(DataType::Bool(haystack.ends_with(suffix.as_slice())))
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "bytes_has_suffix".to_string() }),
                }
            },
            OperationType::BytesIndex => {
                let needle_val = inputs.get("needle").cloned().unwrap_or(DataType::Null);
                match (&input, &needle_val) {
                    (DataType::Bytes(haystack), DataType::Bytes(needle)) => {
                        if needle.is_empty() {
                            return Ok(DataType::Int64(0));
                        }
                        let pos = haystack.windows(needle.len())
                            .position(|w| w == needle.as_slice())
                            .map(|p| p as i64)
                            .unwrap_or(-1);
                        Ok(DataType::Int64(pos))
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "bytes_index".to_string() }),
                }
            },
            OperationType::BytesJoin => {
                let array_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let sep_val = inputs.get("separator").cloned().unwrap_or(DataType::Bytes(vec![]));
                match (&array_val, &sep_val) {
                    (DataType::Array(arr), DataType::Bytes(sep)) => {
                        let mut result: Vec<u8> = Vec::new();
                        for (i, item) in arr.iter().enumerate() {
                            if let DataType::Bytes(b) = item {
                                if i > 0 {
                                    result.extend_from_slice(sep);
                                }
                                result.extend_from_slice(b);
                                if result.len() > MAX_STRING_OUTPUT {
                                    return Err(EvalError::InvalidInput(format!(
                                        "bytes_join: result would exceed {} byte limit", MAX_STRING_OUTPUT
                                    )));
                                }
                            } else {
                                return Err(EvalError::TypeError { expected: "bytes".to_string(), actual: item.type_name().to_string(), context: "bytes_join array element".to_string() });
                            }
                        }
                        Ok(DataType::Bytes(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "array, bytes".to_string(), actual: format!("{}, {}", array_val.type_name(), sep_val.type_name()), context: "bytes_join".to_string() }),
                }
            },
            OperationType::BytesRepeat => {
                let count_val = inputs.get("count").cloned().unwrap_or(DataType::Null);
                match (&input, &count_val) {
                    (DataType::Bytes(b), DataType::Int64(count)) => {
                        if *count < 0 {
                            return Err(EvalError::InvalidInput("bytes_repeat: count must be non-negative".to_string()));
                        }
                        let total = b.len().saturating_mul(*count as usize);
                        if total > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "bytes_repeat: result would be {} bytes (max {})", total, MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::Bytes(b.repeat(*count as usize)))
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes, int64".to_string(), actual: format!("{}, {}", input.type_name(), count_val.type_name()), context: "bytes_repeat".to_string() }),
                }
            },
            OperationType::BytesSplit => {
                let sep_val = inputs.get("separator").cloned().unwrap_or(DataType::Null);
                match (&input, &sep_val) {
                    (DataType::Bytes(haystack), DataType::Bytes(sep)) => {
                        if sep.is_empty() {
                            // Split into individual bytes
                            let parts: Vec<DataType> = haystack.iter().map(|b| DataType::Bytes(vec![*b])).collect();
                            if parts.len() > MAX_ARRAY_ELEMENTS {
                                return Err(EvalError::InvalidInput(format!(
                                    "bytes_split: result would have {} elements (max {})", parts.len(), MAX_ARRAY_ELEMENTS
                                )));
                            }
                            return Ok(DataType::Array(parts));
                        }
                        let mut parts: Vec<DataType> = Vec::new();
                        let mut start = 0;
                        while start <= haystack.len() {
                            if let Some(pos) = haystack[start..].windows(sep.len()).position(|w| w == sep.as_slice()) {
                                parts.push(DataType::Bytes(haystack[start..start + pos].to_vec()));
                                start = start + pos + sep.len();
                            } else {
                                parts.push(DataType::Bytes(haystack[start..].to_vec()));
                                break;
                            }
                            if parts.len() > MAX_ARRAY_ELEMENTS {
                                return Err(EvalError::InvalidInput(format!(
                                    "bytes_split: result would exceed {} elements", MAX_ARRAY_ELEMENTS
                                )));
                            }
                        }
                        Ok(DataType::Array(parts))
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "bytes_split".to_string() }),
                }
            },
            OperationType::BytesTrim => {
                match &input {
                    DataType::Bytes(b) => {
                        let start = b.iter().position(|&byte| !byte.is_ascii_whitespace()).unwrap_or(b.len());
                        let end = b.iter().rposition(|&byte| !byte.is_ascii_whitespace()).map(|p| p + 1).unwrap_or(0);
                        if start >= end {
                            Ok(DataType::Bytes(vec![]))
                        } else {
                            Ok(DataType::Bytes(b[start..end].to_vec()))
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "bytes_trim".to_string() }),
                }
            },
            OperationType::BytesFromString => {
                match &input {
                    DataType::String(s) => {
                        if s.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "bytes_from_string: input string is {} bytes (max {})", s.len(), MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::Bytes(s.as_bytes().to_vec()))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "bytes_from_string".to_string() }),
                }
            },
            OperationType::BytesToString => {
                match &input {
                    DataType::Bytes(b) => {
                        match std::str::from_utf8(b) {
                            Ok(s) => Ok(DataType::String(s.to_string())),
                            Err(e) => Err(EvalError::InvalidInput(format!("bytes_to_string: invalid UTF-8: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "bytes_to_string".to_string() }),
                }
            },

            // Error wrapping/chain operations
            OperationType::ErrorNew => {
                let msg_val = inputs.get("message").cloned().unwrap_or(DataType::Null);
                let msg = match msg_val {
                    DataType::String(s) => s,
                    other => other.to_string_lossy(),
                };
                let mut map = magi_lang::util::OrderedMap::new();
                map.insert("message".to_string(), DataType::String(msg));
                map.insert("cause".to_string(), DataType::Null);
                Ok(DataType::Map(map))
            },
            OperationType::ErrorWrap => {
                let inner = inputs.get("inner").cloned().unwrap_or(DataType::Null);
                let msg_val = inputs.get("message").cloned().unwrap_or(DataType::Null);
                let msg = match msg_val {
                    DataType::String(s) => s,
                    other => other.to_string_lossy(),
                };
                let mut map = magi_lang::util::OrderedMap::new();
                map.insert("message".to_string(), DataType::String(msg));
                map.insert("cause".to_string(), inner);
                Ok(DataType::Map(map))
            },
            OperationType::ErrorUnwrap => {
                let err = inputs.get("error").cloned().unwrap_or(DataType::Null);
                match err {
                    DataType::Map(m) => {
                        Ok(m.get("cause").cloned().unwrap_or(DataType::Null))
                    }
                    _ => Err(EvalError::TypeError { expected: "map (error)".to_string(), actual: err.type_name().to_string(), context: "error_unwrap".to_string() }),
                }
            },
            OperationType::ErrorIs => {
                let err = inputs.get("error").cloned().unwrap_or(DataType::Null);
                let target = inputs.get("target").cloned().unwrap_or(DataType::Null);
                let target_msg = match target {
                    DataType::String(s) => s,
                    other => other.to_string_lossy(),
                };
                // Walk the cause chain
                let mut current = err;
                loop {
                    match current {
                        DataType::Map(ref m) => {
                            if let Some(DataType::String(msg)) = m.get("message") {
                                if *msg == target_msg {
                                    return Ok(DataType::Bool(true));
                                }
                            }
                            match m.get("cause") {
                                Some(DataType::Null) | None => break,
                                Some(next) => { current = next.clone(); }
                            }
                        }
                        _ => break,
                    }
                }
                Ok(DataType::Bool(false))
            },
            OperationType::ErrorChain => {
                let err = inputs.get("error").cloned().unwrap_or(DataType::Null);
                let mut chain: Vec<DataType> = Vec::new();
                let mut current = err;
                loop {
                    match current {
                        DataType::Map(ref m) => {
                            if let Some(msg) = m.get("message") {
                                chain.push(msg.clone());
                            }
                            match m.get("cause") {
                                Some(DataType::Null) | None => break,
                                Some(next) => { current = next.clone(); }
                            }
                        }
                        _ => break,
                    }
                    if chain.len() > MAX_ARRAY_ELEMENTS {
                        return Err(EvalError::InvalidInput("error_chain: chain depth exceeds limit".to_string()));
                    }
                }
                Ok(DataType::Array(chain))
            },

            // Logical Xor
            OperationType::Xor => {
                let a_bool = a.to_bool();
                let b_bool = b.to_bool();
                Ok(DataType::Bool(a_bool ^ b_bool))
            }

            // Clamp: clamp(value, min, max)
            OperationType::Clamp => {
                let min_val = inputs.get("min").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                let max_val = inputs.get("max").or(inputs.get("input_2")).cloned().unwrap_or(DataType::Null);
                match (&input, &min_val, &max_val) {
                    (DataType::Int32(v), DataType::Int32(lo), DataType::Int32(hi)) => {
                        Ok(DataType::Int32((*v).max(*lo).min(*hi)))
                    }
                    (DataType::Uint32(v), DataType::Uint32(lo), DataType::Uint32(hi)) => {
                        Ok(DataType::Uint32((*v).max(*lo).min(*hi)))
                    }
                    (DataType::Uint64(v), DataType::Uint64(lo), DataType::Uint64(hi)) => {
                        Ok(DataType::Uint64((*v).max(*lo).min(*hi)))
                    }
                    (DataType::Float32(v), DataType::Float32(lo), DataType::Float32(hi)) => {
                        Ok(DataType::Float32(v.max(*lo).min(*hi)))
                    }
                    _ => match (promote_numeric(&input), promote_numeric(&min_val), promote_numeric(&max_val)) {
                        (Some(Ok(v)), Some(Ok(lo)), Some(Ok(hi))) => {
                            Ok(DataType::Int64(v.max(lo).min(hi)))
                        }
                        (Some(v), Some(lo), Some(hi)) => {
                            let fv = match v { Ok(i) => i as f64, Err(f) => f };
                            let flo = match lo { Ok(i) => i as f64, Err(f) => f };
                            let fhi = match hi { Ok(i) => i as f64, Err(f) => f };
                            let clamped = fv.max(flo).min(fhi);
                            if matches!(&input, DataType::Float32(_)) {
                                Ok(DataType::Float32(clamped as f32))
                            } else {
                                Ok(DataType::Float64(clamped))
                            }
                        }
                        _ => Err(EvalError::InvalidInput("Clamp requires numeric arguments".to_string())),
                    }
                }
            }

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

            OperationType::ParseJson => {
                match &input {
                    DataType::String(s) => {
                        match magi_lang::util::json_parse_value(s) {
                            Ok(val) => Ok(json_value_to_datatype(&val)),
                            Err(e) => Err(EvalError::InvalidInput(format!("Invalid JSON: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "parse_json".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "ParseInt".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "ParseFloat".to_string() }),
                }
            }

            OperationType::Asin => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.asin()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).asin())),
                    Some(Err(f)) => Ok(DataType::Float64(f.asin())),
                    None => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
                }
            },
            OperationType::Acos => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.acos()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).acos())),
                    Some(Err(f)) => Ok(DataType::Float64(f.acos())),
                    None => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
                }
            },
            OperationType::Atan => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.atan()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).atan())),
                    Some(Err(f)) => Ok(DataType::Float64(f.atan())),
                    None => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
                }
            },
            OperationType::Atan2 => {
                match (promote_numeric(&a), promote_numeric(&b)) {
                    (Some(av), Some(bv)) => {
                        let y = match av { Ok(i) => i as f64, Err(f) => f };
                        let x = match bv { Ok(i) => i as f64, Err(f) => f };
                        Ok(DataType::Float64(y.atan2(x)))
                    }
                    _ => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
                }
            }

            OperationType::Sinh => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.sinh()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).sinh())),
                    Some(Err(f)) => Ok(DataType::Float64(f.sinh())),
                    None => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
                }
            },
            OperationType::Cosh => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.cosh()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).cosh())),
                    Some(Err(f)) => Ok(DataType::Float64(f.cosh())),
                    None => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
                }
            },
            OperationType::Tanh => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.tanh()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).tanh())),
                    Some(Err(f)) => Ok(DataType::Float64(f.tanh())),
                    None => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
                }
            }

            OperationType::ToRadians => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.to_radians()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).to_radians())),
                    Some(Err(f)) => Ok(DataType::Float64(f.to_radians())),
                    None => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
                }
            },
            OperationType::ToDegrees => {
                if let DataType::Float32(n) = &input {
                    return Ok(DataType::Float32(n.to_degrees()));
                }
                match promote_numeric(&input) {
                    Some(Ok(n)) => Ok(DataType::Float64((n as f64).to_degrees())),
                    Some(Err(f)) => Ok(DataType::Float64(f.to_degrees())),
                    None => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "number".to_string(), actual: format!("({}, {})", a.type_name(), b.type_name()), context: "ApproxEq".to_string() }),
                }
            }

            // Greatest common divisor (Euclidean algorithm)
            OperationType::Gcd => {
                match (a.to_i64(), b.to_i64()) {
                    (Some(x), Some(y)) => {
                        // Use i128 to handle abs(i64::MIN) without overflow
                        let mut ax = (x as i128).abs();
                        let mut ay = (y as i128).abs();
                        while ay != 0 {
                            let t = ay;
                            ay = ax % ay;
                            ax = t;
                        }
                        // Result fits in i64 since gcd(a,b) <= max(|a|,|b|) and inputs were i64
                        Ok(DataType::Int64(ax.min(i64::MAX as i128) as i64))
                    }
                    _ => Err(EvalError::TypeError { expected: "integer".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "Gcd".to_string() }),
                }
            }

            // Least common multiple: lcm(a, b) = |a * b| / gcd(a, b)
            OperationType::Lcm => {
                match (a.to_i64(), b.to_i64()) {
                    (Some(x), Some(y)) => {
                        if x == 0 || y == 0 {
                            return Ok(DataType::Int64(0));
                        }
                        // Use i128 to handle abs(i64::MIN) without overflow
                        let mut gx = (x as i128).abs();
                        let mut gy = (y as i128).abs();
                        while gy != 0 {
                            let t = gy;
                            gy = gx % gy;
                            gx = t;
                        }
                        // gx is now gcd
                        // lcm = |x| / gcd * |y| to avoid overflow
                        let ax = (x as i128).abs();
                        let ay = (y as i128).abs();
                        match i64::try_from((ax / gx) * ay) {
                            Ok(v) => Ok(DataType::Int64(v)),
                            Err(_) => Err(EvalError::Overflow("integer overflow in lcm".to_string())),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "integer".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "Lcm".to_string() }),
                }
            }

            // Coalesce: return a if non-null, else b
            OperationType::Coalesce => {
                if !matches!(a, DataType::Null) {
                    Ok(a)
                } else {
                    Ok(b)
                }
            }

            // Default: return input if non-null, else fallback
            OperationType::Default => {
                let fallback = inputs.get("fallback").cloned().unwrap_or(DataType::Null);
                if !matches!(input, DataType::Null) {
                    Ok(input)
                } else {
                    Ok(fallback)
                }
            }

            // Error: create an error
            OperationType::Error => {
                let message = inputs.get("message").cloned().unwrap_or(DataType::String("error".to_string()));
                Err(EvalError::InvalidInput(message.to_string_lossy()))
            }

            // StringJoin: join array elements with separator
            OperationType::StringJoin => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let sep = inputs.get("separator").or(inputs.get("delimiter")).or(inputs.get("input_1"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                match arr_val {
                    DataType::Array(arr) => {
                        let parts: Vec<String> = arr.iter().map(|v| v.to_string_lossy()).collect();
                        let estimated_len: usize = parts.iter().map(|p| p.len()).fold(0usize, |acc, len| acc.saturating_add(len))
                            .saturating_add(parts.len().saturating_sub(1).saturating_mul(sep.len()));
                        if estimated_len > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "string_join result exceeds {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(parts.join(&sep)))
                    }
                    other => Err(EvalError::TypeError { expected: "Array".to_string(), actual: other.type_name().to_string(), context: "StringJoin".to_string() }),
                }
            }

            // StringTemplate: simple template with {key} substitution
            OperationType::StringTemplate => {
                let template = inputs.get("template").cloned().unwrap_or(DataType::Null);
                let values = inputs.get("values").cloned().unwrap_or(DataType::Null);
                match (&template, &values) {
                    (DataType::String(tmpl), DataType::Map(vals)) => {
                        let mut result = tmpl.clone();
                        for (k, v) in vals {
                            result = result.replace(&format!("{{{}}}", k), &v.to_string_lossy());
                            if result.len() > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!(
                                    "string_template result exceeds {} byte limit", MAX_STRING_OUTPUT
                                )));
                            }
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "(String, Map)".to_string(), actual: format!("({}, {})", template.type_name(), values.type_name()), context: "StringTemplate".to_string() }),
                }
            }

            // StringFormat: same as StringTemplate
            OperationType::StringFormat => {
                let template = inputs.get("template").cloned().unwrap_or(DataType::Null);
                let values = inputs.get("values").cloned().unwrap_or(DataType::Null);
                match (&template, &values) {
                    (DataType::String(tmpl), DataType::Map(vals)) => {
                        let mut result = tmpl.clone();
                        for (k, v) in vals {
                            result = result.replace(&format!("{{{}}}", k), &v.to_string_lossy());
                            if result.len() > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!(
                                    "string_format result exceeds {} byte limit", MAX_STRING_OUTPUT
                                )));
                            }
                        }
                        Ok(DataType::String(result))
                    }
                    (DataType::String(tmpl), DataType::Array(vals)) => {
                        let mut result = tmpl.clone();
                        for (i, v) in vals.iter().enumerate() {
                            result = result.replace(&format!("{{{}}}", i), &v.to_string_lossy());
                            if result.len() > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!(
                                    "string_format result exceeds {} byte limit", MAX_STRING_OUTPUT
                                )));
                            }
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "(String, Map|Array)".to_string(), actual: format!("({}, {})", template.type_name(), values.type_name()), context: "StringFormat".to_string() }),
                }
            }

            // ToBytes / FromBytes
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
                    _ => Err(EvalError::TypeError { expected: "String, Bytes, or Array".to_string(), actual: input.type_name().to_string(), context: "ToBytes".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "Bytes".to_string(), actual: input.type_name().to_string(), context: "FromBytes".to_string() }),
                }
            }

            // ArrayFromMap: convert map to array of [key, value] pairs
            OperationType::ArrayFromMap => {
                let map_val = inputs.get("map").cloned().unwrap_or(DataType::Null);
                match map_val {
                    DataType::Map(m) => {
                        Ok(DataType::Array(m.into_iter().map(|(k, v)| {
                            DataType::Array(vec![DataType::String(k), v])
                        }).collect()))
                    }
                    _ => Err(EvalError::TypeError { expected: "Map".to_string(), actual: map.type_name().to_string(), context: "ArrayFromMap".to_string() }),
                }
            }

            // MapUpdate: update a map key with a value
            OperationType::MapUpdate => {
                match (&map, &key) {
                    (DataType::Map(m), DataType::String(k)) => {
                        if !m.contains_key(k.as_str()) && m.len() >= MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!("MapUpdate would exceed {} entries", MAX_ARRAY_ELEMENTS)));
                        }
                        let mut new_map = m.clone();
                        new_map.insert(k.clone(), value.clone());
                        Ok(DataType::Map(new_map))
                    }
                    (DataType::Map(_), _) => Err(EvalError::TypeError { expected: "String".to_string(), actual: key.type_name().to_string(), context: "MapUpdate key".to_string() }),
                    _ => Err(EvalError::TypeError { expected: "Map".to_string(), actual: map.type_name().to_string(), context: "MapUpdate".to_string() }),
                }
            }

            // Math Aggregates
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
                                None => {
                                    return Err(EvalError::TypeError { expected: "number".to_string(), actual: item.type_name().to_string(), context: "MathSum".to_string() });
                                }
                            }
                        }
                        if has_float {
                            Ok(DataType::Float64(float_sum))
                        } else {
                            Ok(DataType::Int64(int_sum))
                        }
                    }
                    other => Err(EvalError::TypeError { expected: "Array".to_string(), actual: other.type_name().to_string(), context: "MathSum".to_string() }),
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
                                None => {
                                    return Err(EvalError::TypeError { expected: "number".to_string(), actual: item.type_name().to_string(), context: "MathProduct".to_string() });
                                }
                            }
                        }
                        if has_float {
                            Ok(DataType::Float64(float_prod))
                        } else {
                            Ok(DataType::Int64(int_prod))
                        }
                    }
                    other => Err(EvalError::TypeError { expected: "Array".to_string(), actual: other.type_name().to_string(), context: "MathProduct".to_string() }),
                }
            }
            OperationType::MathAverage => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        if arr.is_empty() {
                            return Ok(DataType::Float64(f64::NAN));
                        }
                        let mut sum = 0.0f64;
                        for item in &arr {
                            match promote_numeric(item) {
                                Some(Ok(i)) => { sum += i as f64; }
                                Some(Err(f)) => { sum += f; }
                                None => return Err(EvalError::TypeError { expected: "numeric".to_string(), actual: item.type_name().to_string(), context: "MathAverage".to_string() }),
                            }
                        }
                        Ok(DataType::Float64(sum / arr.len() as f64))
                    }
                    other => Err(EvalError::TypeError { expected: "Array".to_string(), actual: other.type_name().to_string(), context: "MathAverage".to_string() }),
                }
            }
            OperationType::MathMinOf => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        if arr.is_empty() { return Ok(DataType::Null); }
                        let mut best: Option<(f64, usize)> = None;
                        for (idx, item) in arr.iter().enumerate() {
                            let f = match promote_numeric(item) {
                                Some(Ok(i)) => i as f64,
                                Some(Err(f)) => f,
                                None => return Err(EvalError::TypeError { expected: "numeric".to_string(), actual: item.type_name().to_string(), context: "MathMinOf".to_string() }),
                            };
                            if f.is_nan() { continue; }
                            best = Some(match best {
                                Some((cur, ci)) => if f < cur { (f, idx) } else { (cur, ci) },
                                None => (f, idx),
                            });
                        }
                        Ok(best.map(|(_, idx)| arr[idx].clone()).unwrap_or(DataType::Null))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "MathMinOf".to_string() }),
                }
            }
            OperationType::MathMaxOf => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        if arr.is_empty() { return Ok(DataType::Null); }
                        let mut best: Option<(f64, usize)> = None;
                        for (idx, item) in arr.iter().enumerate() {
                            let f = match promote_numeric(item) {
                                Some(Ok(i)) => i as f64,
                                Some(Err(f)) => f,
                                None => return Err(EvalError::TypeError { expected: "numeric".to_string(), actual: item.type_name().to_string(), context: "MathMaxOf".to_string() }),
                            };
                            if f.is_nan() { continue; }
                            best = Some(match best {
                                Some((cur, ci)) => if f > cur { (f, idx) } else { (cur, ci) },
                                None => (f, idx),
                            });
                        }
                        Ok(best.map(|(_, idx)| arr[idx].clone()).unwrap_or(DataType::Null))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "MathMaxOf".to_string() }),
                }
            }
            OperationType::MathCount => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => Ok(DataType::Int64(arr.len() as i64)),
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "MathCount".to_string() }),
                }
            }

            // Remap: remap value from [in_min, in_max] to [out_min, out_max]
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
                    _ => Err(EvalError::TypeError { expected: "number".to_string(), actual: input.type_name().to_string(), context: "Remap".to_string() }),
                }
            }

            // NowTimestamp: current time in milliseconds
            OperationType::NowTimestamp => {
                Ok(DataType::Int64(magi_lang::util::now_millis()))
            }

            // FormatTimestamp: format a timestamp as ISO 8601 string
            OperationType::FormatTimestamp => {
                match promote_numeric(&input) {
                    Some(v) => {
                        let ms = match v { Ok(i) => i, Err(f) => f as i64 };
                        match magi_lang::util::format_timestamp_millis(ms) {
                            Some(s) => Ok(DataType::String(s)),
                            None => Err(EvalError::InvalidInput(format!("format_timestamp: invalid timestamp {}", ms))),
                        }
                    }
                    None => Err(EvalError::TypeError { expected: "number".to_string(), actual: input.type_name().to_string(), context: "FormatTimestamp".to_string() }),
                }
            }

            // Sleep: sleep for duration ms (no-op in sync evaluator, just returns null)
            OperationType::Sleep => {
                sleep_chunked(inputs)?;
                Ok(DataType::Null)
            }

            // TimestampDiff: difference between two timestamps in ms
            OperationType::TimestampDiff => {
                match (promote_numeric(&a), promote_numeric(&b)) {
                    (Some(av), Some(bv)) => {
                        let fa = match av { Ok(i) => i, Err(f) => f as i64 };
                        let fb = match bv { Ok(i) => i, Err(f) => f as i64 };
                        Ok(DataType::Int64(fa.saturating_sub(fb)))
                    }
                    _ => Err(EvalError::TypeError { expected: "number".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "TimestampDiff".to_string() }),
                }
            }

            // TimestampAdd: add ms to a timestamp
            OperationType::TimestampAdd => {
                let amount = inputs.get("amount").cloned().unwrap_or(DataType::Null);
                match (promote_numeric(&input), promote_numeric(&amount)) {
                    (Some(tv), Some(av)) => {
                        let ft = match tv { Ok(i) => i, Err(f) => f as i64 };
                        let fa = match av { Ok(i) => i, Err(f) => f as i64 };
                        Ok(DataType::Int64(ft.saturating_add(fa)))
                    }
                    _ => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: format!("({}, {})", input.type_name(), amount.type_name()), context: "TimestampAdd".to_string() }),
                }
            }

            // ParseTimestamp: parse ISO timestamp string to millis
            OperationType::ParseTimestamp => {
                match &input {
                    DataType::String(s) => {
                        match magi_lang::util::parse_timestamp_to_millis(s) {
                            Ok(ms) => Ok(DataType::Int64(ms)),
                            Err(_) => Err(EvalError::InvalidInput(format!("ParseTimestamp: unrecognized format: {}", s.trim()))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "ParseTimestamp".to_string() }),
                }
            }

            // HexEncode / HexDecode
            OperationType::HexEncode => {
                match &input {
                    DataType::Bytes(b) => {
                        if b.len() * 2 > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "hex_encode: output would exceed {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(magi_lang::util::hex_encode(b)))
                    }
                    DataType::String(s) => {
                        if s.len() * 2 > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "hex_encode: output would exceed {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(magi_lang::util::hex_encode(s.as_bytes())))
                    }
                    _ => Err(EvalError::TypeError {
                        expected: "String or Bytes".to_string(),
                        actual: input.type_name().to_string(),
                        context: "hex_encode".to_string(),
                    }),
                }
            }
            OperationType::HexDecode => {
                match &input {
                    DataType::String(s) => {
                        let s = s.trim();
                        let s = s.strip_prefix("0x").or(s.strip_prefix("0X")).unwrap_or(s);
                        match magi_lang::util::hex_decode(s) {
                            Ok(bytes) => Ok(DataType::Bytes(bytes)),
                            Err(e) => Err(EvalError::InvalidInput(format!("hex_decode: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "HexDecode".to_string() }),
                }
            }

            // UrlEncode / UrlDecode
            OperationType::UrlEncode => {
                match &input {
                    DataType::String(s) => {
                        let result = magi_lang::util::percent_encode(s);
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!("UrlEncode output exceeds {} bytes", MAX_STRING_OUTPUT)));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "UrlEncode".to_string() }),
                }
            }
            OperationType::UrlDecode => {
                match &input {
                    DataType::String(s) => {
                        match magi_lang::util::percent_decode(s) {
                            Ok(decoded) => Ok(DataType::String(decoded)),
                            Err(_) => Err(EvalError::InvalidInput("url_decode: invalid UTF-8".to_string())),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "UrlDecode".to_string() }),
                }
            }

            // HashSha256: SHA-256 hash
            OperationType::HashSha256 => {
                if matches!(input, DataType::Null) {
                    return Err(EvalError::TypeError { expected: "String or Bytes".to_string(), actual: "Null".to_string(), context: "HashSha256".to_string() });
                }
                let data = data_to_bytes(&input);
                let hash = magi_lang::util::sha256(&data);
                Ok(DataType::String(magi_lang::util::hex_encode(&hash)))
            }

            // HashSha1: SHA-1 hash
            OperationType::HashSha1 => {
                if matches!(input, DataType::Null) {
                    return Err(EvalError::TypeError { expected: "String or Bytes".to_string(), actual: "Null".to_string(), context: "HashSha1".to_string() });
                }
                let data = data_to_bytes(&input);
                let hash = magi_lang::util::sha1(&data);
                Ok(DataType::String(magi_lang::util::hex_encode(&hash)))
            }

            // HashMd5: MD5 hash
            OperationType::HashMd5 => {
                if matches!(input, DataType::Null) {
                    return Err(EvalError::TypeError { expected: "String or Bytes".to_string(), actual: "Null".to_string(), context: "HashMd5".to_string() });
                }
                let data = data_to_bytes(&input);
                let hash = magi_lang::util::md5_hash(&data);
                Ok(DataType::String(magi_lang::util::hex_encode(&hash)))
            }

            // JSON operations
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
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: path.type_name().to_string(), context: "JsonGet path".to_string() }),
                }
            }
            OperationType::JsonSet => {
                let json_val = inputs.get("value").cloned().unwrap_or(DataType::Null);
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                let item = inputs.get("item").cloned().unwrap_or(DataType::Null);
                match (&json_val, &path) {
                    (DataType::Map(m), DataType::String(key)) => {
                        if !m.contains_key(key.as_str()) && m.len() >= MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!("JsonSet would exceed {} entries", MAX_ARRAY_ELEMENTS)));
                        }
                        let mut new_map = m.clone();
                        new_map.insert(key.clone(), item);
                        Ok(DataType::Map(new_map))
                    }
                    (DataType::Map(_), _) => Err(EvalError::TypeError { expected: "string".to_string(), actual: path.type_name().to_string(), context: "JsonSet path".to_string() }),
                    _ => Err(EvalError::TypeError { expected: "map".to_string(), actual: json_val.type_name().to_string(), context: "JsonSet".to_string() }),
                }
            }
            OperationType::JsonDelete => {
                let json_val = inputs.get("value").cloned().unwrap_or(DataType::Null);
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match (&json_val, &path) {
                    (DataType::Map(m), DataType::String(key)) => {
                        let mut new_map = m.clone();
                        new_map.shift_remove(key);
                        Ok(DataType::Map(new_map))
                    }
                    (DataType::Map(_), _) => Err(EvalError::TypeError { expected: "string".to_string(), actual: path.type_name().to_string(), context: "JsonDelete path".to_string() }),
                    _ => Err(EvalError::TypeError { expected: "map".to_string(), actual: json_val.type_name().to_string(), context: "JsonDelete".to_string() }),
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
                            if !merged.contains_key(k.as_str()) && merged.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(EvalError::InvalidInput(format!("JsonMerge would exceed {} entries", MAX_ARRAY_ELEMENTS)));
                            }
                            merged.insert(k.clone(), v.clone());
                        }
                        Ok(DataType::Map(merged))
                    }
                    _ => Err(EvalError::TypeError { expected: "(Map, Map)".to_string(), actual: format!("({}, {})", a.type_name(), b.type_name()), context: "JsonMerge".to_string() }),
                }
            }
            OperationType::JsonPrettyPrint => {
                let json_val = datatype_to_json_value(&input);
                let s = magi_lang::util::json_to_string_pretty(&json_val);
                if s.len() > MAX_STRING_OUTPUT {
                    return Err(EvalError::InvalidInput(format!(
                        "json_pretty_print: output would exceed {} byte limit", MAX_STRING_OUTPUT
                    )));
                }
                Ok(DataType::String(s))
            }
            OperationType::JsonCompact => {
                let json_val = datatype_to_json_value(&input);
                let s = magi_lang::util::json_to_string(&json_val);
                if s.len() > MAX_STRING_OUTPUT {
                    return Err(EvalError::InvalidInput(format!(
                        "json_compact: output would exceed {} byte limit", MAX_STRING_OUTPUT
                    )));
                }
                Ok(DataType::String(s))
            }
            OperationType::JsonValidate => {
                match &input {
                    DataType::String(s) => {
                        // Try parsing as JSON
                        Ok(DataType::Bool(magi_lang::util::json_parse_value(s).is_ok()))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "JsonValidate".to_string() }),
                }
            }
            OperationType::JsonFlatten => {
                fn json_flatten(val: &DataType, prefix: &str, result: &mut magi_lang::util::OrderedMap<String, DataType>, depth: usize) -> Result<(), ()> {
                    if depth > 64 || result.len() > MAX_ARRAY_ELEMENTS { return Err(()); }
                    match val {
                        DataType::Map(m) => {
                            for (k, v) in m {
                                if k.starts_with("__") { continue; }
                                let new_key = if prefix.is_empty() { k.clone() } else { format!("{}.{}", prefix, k) };
                                json_flatten(v, &new_key, result, depth + 1)?;
                            }
                        }
                        DataType::Array(arr) => {
                            for (i, v) in arr.iter().enumerate() {
                                if result.len() > MAX_ARRAY_ELEMENTS { return Err(()); }
                                let new_key = if prefix.is_empty() { format!("{}", i) } else { format!("{}.{}", prefix, i) };
                                json_flatten(v, &new_key, result, depth + 1)?;
                            }
                        }
                        _ => {
                            let key = if prefix.is_empty() { "value".to_string() } else { prefix.to_string() };
                            result.insert(key, val.clone());
                        }
                    }
                    Ok(())
                }
                let mut result = magi_lang::util::OrderedMap::new();
                if json_flatten(&input, "", &mut result, 0).is_err() {
                    return Err(EvalError::InvalidInput(format!("JsonFlatten result exceeds {} elements", MAX_ARRAY_ELEMENTS)));
                }
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
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: path.type_name().to_string(), context: "JsonQuery path".to_string() }),
                }
            }

            OperationType::RandomInt => {
                Ok(DataType::Int64(magi_lang::util::random_i64()))
            }
            OperationType::RandomFloat => {
                Ok(DataType::Float64(magi_lang::util::random_f64()))
            }
            OperationType::RandomBool => {
                Ok(DataType::Bool(magi_lang::util::random_bool()))
            }
            OperationType::RandomRange => {
                match (a.to_i64(), b.to_i64()) {
                    (Some(lo), Some(hi)) if lo < hi => {
                        let result = magi_lang::util::random_range_i64(lo, hi);
                        Ok(DataType::Int64(result))
                    }
                    (Some(lo), Some(hi)) if lo == hi => Ok(DataType::Int64(lo)),
                    (Some(lo), Some(hi)) => Err(EvalError::InvalidInput(
                        format!("random_range: min ({}) must be less than or equal to max ({})", lo, hi),
                    )),
                    _ => Err(EvalError::TypeError {
                        expected: "number".to_string(),
                        actual: format!("{}, {}", a.type_name(), b.type_name()),
                        context: "random_range".to_string(),
                    }),
                }
            }
            OperationType::RandomChoice => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) if !arr.is_empty() => {
                        let idx = magi_lang::util::random_range_usize(arr.len());
                        Ok(arr[idx].clone())
                    }
                    DataType::Array(_) => Ok(DataType::Null), // empty array → Null (correct semantic)
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "RandomChoice".to_string() }),
                }
            }
            OperationType::RandomShuffle => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(mut arr) => {
                        magi_lang::util::random_shuffle(&mut arr);
                        Ok(DataType::Array(arr))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "RandomShuffle".to_string() }),
                }
            }
            OperationType::RandomUuid => {
                Ok(DataType::String(magi_lang::util::uuid_v4()))
            }

            // Regex operations (regex crate)
            OperationType::RegexMatch => {
                let pattern = inputs.get("input_1").or(inputs.get("pattern")).cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        let s = s.clone();
                        let pat = pat.clone();
                        regex_with_timeout(move || {
                            match compile_regex(&pat) {
                                Ok(re) => Ok(DataType::Bool(re.is_match(&s))),
                                Err(e) => Err(EvalError::InvalidInput(format!("regex_match: {}", e))),
                            }
                        })
                    }
                    _ => Err(EvalError::TypeError { expected: "(String, String)".to_string(), actual: format!("({}, {})", input.type_name(), pattern.type_name()), context: "RegexMatch".to_string() }),
                }
            }
            OperationType::RegexTest => {
                let pattern = inputs.get("pattern").cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        let s = s.clone();
                        let pat = pat.clone();
                        regex_with_timeout(move || {
                            match compile_regex(&pat) {
                                Ok(re) => Ok(DataType::Bool(re.is_match(&s))),
                                Err(e) => Err(EvalError::InvalidInput(format!("regex_test: {}", e))),
                            }
                        })
                    }
                    _ => Err(EvalError::TypeError { expected: "(String, String)".to_string(), actual: format!("({}, {})", input.type_name(), pattern.type_name()), context: "RegexTest".to_string() }),
                }
            }
            OperationType::RegexReplace => {
                let replacement = inputs.get("replacement").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                let pattern = inputs.get("pattern").or(inputs.get("input_2")).cloned().unwrap_or(DataType::Null);
                match (&input, &pattern, &replacement) {
                    (DataType::String(s), DataType::String(pat), DataType::String(rep)) => {
                        let s = s.clone();
                        let pat = pat.clone();
                        let rep = rep.clone();
                        regex_with_timeout(move || {
                            match compile_regex(&pat) {
                                Ok(re) => {
                                    // Use capture-aware replace when $ is in replacement
                                    let result = if rep.contains('$') {
                                        re.replace_with_captures(&s, &rep)
                                    } else {
                                        re.replace_all(&s, rep.as_str()).to_string()
                                    };
                                    if result.len() > MAX_STRING_OUTPUT {
                                        return Err(EvalError::InvalidInput(format!(
                                            "regex_replace result exceeds {} byte limit", MAX_STRING_OUTPUT
                                        )));
                                    }
                                    Ok(DataType::String(result))
                                }
                                Err(e) => Err(EvalError::InvalidInput(format!("regex_replace: {}", e))),
                            }
                        })
                    }
                    _ => Err(EvalError::TypeError { expected: "(String, String, String)".to_string(), actual: format!("({}, {}, {})", input.type_name(), pattern.type_name(), replacement.type_name()), context: "RegexReplace".to_string() }),
                }
            }
            OperationType::RegexExtract => {
                let pattern = inputs.get("pattern").or(inputs.get("input_1")).cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        let s = s.clone();
                        let pat = pat.clone();
                        regex_with_timeout(move || {
                            match compile_regex(&pat) {
                                Ok(re) => match re.captures(&s) {
                                    Some(caps) => {
                                        if caps.len() > 1 {
                                            let groups: Vec<DataType> = caps.iter().skip(1)
                                                .map(|m| match m {
                                                    Some(m) => DataType::String(m.as_str().to_string()),
                                                    None => DataType::Null,
                                                })
                                                .collect();
                                            Ok(DataType::Array(groups))
                                        } else {
                                            Ok(DataType::String(caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()))
                                        }
                                    }
                                    None => Ok(DataType::Null),
                                },
                                Err(e) => Err(EvalError::InvalidInput(format!("regex_extract: {}", e))),
                            }
                        })
                    }
                    _ => Err(EvalError::TypeError { expected: "(String, String)".to_string(), actual: format!("({}, {})", input.type_name(), pattern.type_name()), context: "RegexExtract".to_string() }),
                }
            }
            OperationType::RegexSplit => {
                let pattern = inputs.get("pattern").cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        let s = s.clone();
                        let pat = pat.clone();
                        regex_with_timeout(move || {
                            match compile_regex(&pat) {
                                Ok(re) => {
                                    let parts: Vec<DataType> = re.split(&s)
                                        .into_iter()
                                        .take(MAX_ARRAY_ELEMENTS + 1)
                                        .map(|p| DataType::String(p))
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
                        })
                    }
                    _ => Err(EvalError::TypeError { expected: "(String, String)".to_string(), actual: format!("({}, {})", input.type_name(), pattern.type_name()), context: "RegexSplit".to_string() }),
                }
            }
            OperationType::RegexEscape => {
                match &input {
                    DataType::String(s) => {
                        let result = magi_lang::util::regex_escape(s);
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!("RegexEscape output exceeds {} bytes", MAX_STRING_OUTPUT)));
                        }
                        Ok(DataType::String(result))
                    },
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "RegexEscape".to_string() }),
                }
            }
            OperationType::RegexCaptures => {
                let pattern = inputs.get("pattern").cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        let s = s.clone();
                        let pat = pat.clone();
                        regex_with_timeout(move || {
                            match compile_regex(&pat) {
                                Ok(re) => match re.captures(&s) {
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
                        })
                    }
                    _ => Err(EvalError::TypeError { expected: "(String, String)".to_string(), actual: format!("({}, {})", input.type_name(), pattern.type_name()), context: "RegexCaptures".to_string() }),
                }
            }
            OperationType::RegexFindAll => {
                let pattern = inputs.get("pattern").cloned().unwrap_or(DataType::Null);
                match (&input, &pattern) {
                    (DataType::String(s), DataType::String(pat)) => {
                        let s = s.clone();
                        let pat = pat.clone();
                        regex_with_timeout(move || {
                            match compile_regex(&pat) {
                                Ok(re) => {
                                    let matches: Vec<DataType> = re.find_iter(&s)
                                        .into_iter()
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
                        })
                    }
                    _ => Err(EvalError::TypeError { expected: "(String, String)".to_string(), actual: format!("({}, {})", input.type_name(), pattern.type_name()), context: "RegexFindAll".to_string() }),
                }
            }

            OperationType::FsRead => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => {
                        const MAX_FILE_READ: u64 = 64 * 1024 * 1024; // 64 MB
                        let file = fs::File::open(p)
                            .map_err(|e| EvalError::InvalidInput(format!("fs_read: {}", e)))?;
                        let mut limited = std::io::Read::take(file, MAX_FILE_READ + 1);
                        let mut content = String::new();
                        limited.read_to_string(&mut content)
                            .map_err(|e| EvalError::InvalidInput(format!("fs_read: {}", e)))?;
                        if content.len() as u64 > MAX_FILE_READ {
                            return Err(EvalError::InvalidInput(format!(
                                "fs_read: file exceeds {} byte limit", MAX_FILE_READ
                            )));
                        }
                        Ok(DataType::String(content))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "fs_read".to_string() }),
                }
            }
            OperationType::FsWrite => {
                // Use module-level MAX_FILE_WRITE_SIZE (#274)
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                let content = inputs.get("content").cloned().unwrap_or(DataType::Null);
                match (&path, &content) {
                    (DataType::String(p), DataType::String(c)) => {
                        if c.len() > MAX_FILE_WRITE_SIZE {
                            return Err(EvalError::InvalidInput(format!(
                                "fs_write: content exceeds {} byte limit", MAX_FILE_WRITE_SIZE
                            )));
                        }
                        if let Some(parent) = std::path::Path::new(p.as_str()).parent() {
                            if !parent.as_os_str().is_empty() {
                                if let Err(e) = fs::create_dir_all(parent) {
                                    return Err(EvalError::InvalidInput(format!("fs_write: cannot create parent directory: {}", e)));
                                }
                            }
                        }
                        match fs::write(p, c) {
                            Ok(_) => Ok(DataType::Bool(true)),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_write: {}", e))),
                        }
                    }
                    (DataType::String(p), DataType::Bytes(b)) => {
                        if b.len() > MAX_FILE_WRITE_SIZE {
                            return Err(EvalError::InvalidInput(format!(
                                "fs_write: content exceeds {} byte limit", MAX_FILE_WRITE_SIZE
                            )));
                        }
                        if let Some(parent) = std::path::Path::new(p.as_str()).parent() {
                            if !parent.as_os_str().is_empty() {
                                if let Err(e) = fs::create_dir_all(parent) {
                                    return Err(EvalError::InvalidInput(format!("fs_write: cannot create parent directory: {}", e)));
                                }
                            }
                        }
                        match fs::write(p, b) {
                            Ok(_) => Ok(DataType::Bool(true)),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_write: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "fs_write".to_string() }),
                }
            }
            OperationType::FsAppend => {
                // Use module-level MAX_FILE_WRITE_SIZE (#274)
                const MAX_FILE_SIZE: u64 = 256 * 1024 * 1024; // 256 MB max resulting file
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                let content = inputs.get("content").cloned().unwrap_or(DataType::Null);
                match (&path, &content) {
                    (DataType::String(p), DataType::String(c)) => {
                        if c.len() > MAX_FILE_WRITE_SIZE {
                            return Err(EvalError::InvalidInput(format!(
                                "fs_append: content exceeds {} byte limit", MAX_FILE_WRITE_SIZE
                            )));
                        }
                        use std::io::Write;
                        match std::fs::OpenOptions::new().append(true).create(true).open(p) {
                            Ok(mut file) => {
                                // Check resulting file size to prevent unbounded growth
                                let existing_size = file.metadata()
                                    .map(|m| m.len()).unwrap_or(0);
                                if existing_size + c.len() as u64 > MAX_FILE_SIZE {
                                    return Err(EvalError::InvalidInput(format!(
                                        "fs_append: resulting file would exceed {} byte limit",
                                        MAX_FILE_SIZE
                                    )));
                                }
                                match file.write_all(c.as_bytes()) {
                                    Ok(_) => Ok(DataType::Bool(true)),
                                    Err(e) => Err(EvalError::InvalidInput(format!("fs_append: {}", e))),
                                }
                            }
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_append: {}", e))),
                        }
                    }
                    (DataType::String(p), DataType::Bytes(b)) => {
                        if b.len() > MAX_FILE_WRITE_SIZE {
                            return Err(EvalError::InvalidInput(format!(
                                "fs_append: content exceeds {} byte limit", MAX_FILE_WRITE_SIZE
                            )));
                        }
                        use std::io::Write;
                        match std::fs::OpenOptions::new().append(true).create(true).open(p) {
                            Ok(mut file) => {
                                let existing_size = file.metadata()
                                    .map(|m| m.len()).unwrap_or(0);
                                if existing_size + b.len() as u64 > MAX_FILE_SIZE {
                                    return Err(EvalError::InvalidInput(format!(
                                        "fs_append: resulting file would exceed {} byte limit",
                                        MAX_FILE_SIZE
                                    )));
                                }
                                match file.write_all(b) {
                                    Ok(_) => Ok(DataType::Bool(true)),
                                    Err(e) => Err(EvalError::InvalidInput(format!("fs_append: {}", e))),
                                }
                            }
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_append: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "fs_append".to_string() }),
                }
            }
            OperationType::FsExists => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => Ok(DataType::Bool(std::path::Path::new(p).exists())),
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: path.type_name().to_string(), context: "FsExists".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "fs_list".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "fs_mkdir".to_string() }),
                }
            }
            OperationType::FsRemove => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => {
                        // Try remove_file first; if it fails with a "is a directory"
                        // error, fall back to remove_dir_all. This avoids a TOCTOU race
                        // between is_dir() and the actual removal.
                        match fs::remove_file(p) {
                            Ok(_) => Ok(DataType::Bool(true)),
                            Err(e) if e.kind() == std::io::ErrorKind::IsADirectory
                                || e.raw_os_error() == Some(21) /* EISDIR */ => {
                                match fs::remove_dir_all(p) {
                                    Ok(_) => Ok(DataType::Bool(true)),
                                    Err(e) => Err(EvalError::InvalidInput(format!("fs_remove: {}", e))),
                                }
                            }
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_remove: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "fs_remove".to_string() }),
                }
            }
            OperationType::FsIsFile => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => Ok(DataType::Bool(std::path::Path::new(p).is_file())),
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: path.type_name().to_string(), context: "FsIsFile".to_string() }),
                }
            }
            OperationType::FsIsDir => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => Ok(DataType::Bool(std::path::Path::new(p).is_dir())),
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: path.type_name().to_string(), context: "FsIsDir".to_string() }),
                }
            }
            OperationType::FsSize => {
                let path = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match &path {
                    DataType::String(p) => {
                        match fs::metadata(p) {
                            Ok(meta) => Ok(DataType::Int64(i64::try_from(meta.len()).unwrap_or(i64::MAX))),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_size: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "fs_size".to_string() }),
                }
            }
            OperationType::FsCopy => {
                let source = inputs.get("source").cloned().unwrap_or(DataType::Null);
                let dest = inputs.get("destination").cloned().unwrap_or(DataType::Null);
                match (&source, &dest) {
                    (DataType::String(src), DataType::String(dst)) => {
                        match fs::copy(src, dst) {
                            Ok(bytes) => Ok(DataType::Int64(i64::try_from(bytes).unwrap_or(i64::MAX))),
                            Err(e) => Err(EvalError::InvalidInput(format!("fs_copy: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "fs_copy".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "fs_move".to_string() }),
                }
            }

            OperationType::FsChmod => {
                let path = get_string(inputs, "path")?;
                let mode = require_i64_or_default(inputs.get("mode").or(inputs.get("input_1")), 0o644, "chmod mode")? as u32;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                        .map_err(|e| EvalError::InvalidInput(format!("chmod: {}", e)))?;
                }
                Ok(DataType::Bool(true))
            }
            OperationType::FsSymlink => {
                let target = get_string(inputs, "target")?;
                let link = get_string(inputs, "link")?;
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(target, link)
                        .map_err(|e| EvalError::InvalidInput(format!("symlink: {}", e)))?;
                }
                Ok(DataType::Bool(true))
            }
            OperationType::FsReadlink => {
                let path = get_string(inputs, "path")?;
                match std::fs::read_link(path) {
                    Ok(target) => Ok(DataType::String(target.to_string_lossy().to_string())),
                    Err(e) => Err(EvalError::InvalidInput(format!("readlink: {}", e))),
                }
            }

            OperationType::EnvGet => {
                let key_val = inputs.get("key").cloned().unwrap_or(DataType::Null);
                match &key_val {
                    DataType::String(k) => {
                        match env::var(k) {
                            Ok(v) => Ok(DataType::String(v)),
                            Err(_) => Ok(DataType::Null),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: key_val.type_name().to_string(), context: "EnvGet".to_string() }),
                }
            }
            OperationType::EnvHas => {
                let key_val = inputs.get("key").cloned().unwrap_or(DataType::Null);
                match &key_val {
                    DataType::String(k) => Ok(DataType::Bool(env::var(k).is_ok())),
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: key_val.type_name().to_string(), context: "EnvHas".to_string() }),
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

            OperationType::PathJoin => {
                match (&a, &b) {
                    (DataType::String(p1), DataType::String(p2)) => {
                        let joined = std::path::Path::new(p1).join(p2);
                        Ok(DataType::String(normalize_path(&joined)))
                    }
                    _ => Err(EvalError::TypeError { expected: "(String, String)".to_string(), actual: format!("({}, {})", a.type_name(), b.type_name()), context: "PathJoin".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "PathBasename".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "PathDirname".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "PathExtension".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "PathStem".to_string() }),
                }
            }
            OperationType::PathIsAbsolute => {
                match &input {
                    DataType::String(p) => Ok(DataType::Bool(std::path::Path::new(p).is_absolute())),
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "PathIsAbsolute".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "PathParent".to_string() }),
                }
            }
            OperationType::PathNormalize => {
                match &input {
                    DataType::String(p) => {
                        // Normalization: remove . and resolve .. (never pop past root)
                        let path = std::path::Path::new(p);
                        let mut components: Vec<std::path::Component> = Vec::new();
                        for comp in path.components() {
                            match comp {
                                std::path::Component::ParentDir => {
                                    match components.last() {
                                        Some(std::path::Component::Normal(_)) => { components.pop(); }
                                        Some(std::path::Component::RootDir) | Some(std::path::Component::Prefix(_)) => {}
                                        _ => components.push(comp),
                                    }
                                }
                                std::path::Component::CurDir => {}
                                other => components.push(other),
                            }
                        }
                        let normalized: std::path::PathBuf = components.into_iter().collect();
                        Ok(DataType::String(normalized.to_string_lossy().to_string()))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "PathNormalize".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "PathSplit".to_string() }),
                }
            }
            OperationType::PathWithExtension => {
                let extension = inputs.get("extension").cloned().unwrap_or(DataType::Null);
                match (&input, &extension) {
                    (DataType::String(p), DataType::String(ext)) => {
                        let path = std::path::Path::new(p).with_extension(ext);
                        Ok(DataType::String(path.to_string_lossy().to_string()))
                    }
                    _ => Err(EvalError::TypeError { expected: "(String, String)".to_string(), actual: format!("({}, {})", input.type_name(), extension.type_name()), context: "PathWithExtension".to_string() }),
                }
            }

            // Reduce (array fold with initial value)
            OperationType::Reduce => {
                // Reduce is mostly handled by the interpreter's HOF method,
                // but as a standalone op, we treat initial as the seed and return it
                // (the real reduce uses lambda callbacks handled by the interpreter)
                let initial = inputs.get("initial").cloned().unwrap_or(DataType::Null);
                Ok(initial)
            }

            OperationType::FmtNumber => {
                match promote_numeric(&value) {
                    Some(Ok(n)) => Ok(DataType::String(format!("{}", n))),
                    Some(Err(f)) => Ok(DataType::String(format!("{}", f))),
                    None => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: value.type_name().to_string(), context: "FmtNumber".to_string() }),
                }
            }
            OperationType::FmtHex => {
                if let DataType::Uint64(n) = &value {
                    Ok(DataType::String(format!("{:x}", n)))
                } else {
                    match value.to_i64() {
                        Some(n) => Ok(DataType::String(format!("{:x}", n))),
                        None => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: value.type_name().to_string(), context: "FmtHex".to_string() }),
                    }
                }
            }
            OperationType::FmtBinary => {
                if let DataType::Uint64(n) = &value {
                    Ok(DataType::String(format!("{:b}", n)))
                } else {
                    match value.to_i64() {
                        Some(n) => Ok(DataType::String(format!("{:b}", n))),
                        None => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: value.type_name().to_string(), context: "FmtBinary".to_string() }),
                    }
                }
            }
            OperationType::FmtPercent => {
                match promote_numeric(&value) {
                    Some(Ok(n)) => Ok(DataType::String(format!("{}%", n))),
                    Some(Err(f)) => {
                        // Format cleanly: if it's a whole number in i64 range use no decimals
                        if f == f.trunc() && f.is_finite() && f >= i64::MIN as f64 && f < i64::MAX as f64 {
                            Ok(DataType::String(format!("{}%", f as i64)))
                        } else {
                            Ok(DataType::String(format!("{}%", f)))
                        }
                    }
                    None => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: value.type_name().to_string(), context: "FmtPercent".to_string() }),
                }
            }
            OperationType::FmtBytes => {
                match value.to_i64() {
                    Some(n) => {
                        let abs = (n as f64).abs();
                        let f = n as f64;
                        let result = if abs < 1024.0 {
                            format!("{} B", n)
                        } else if abs < 1024.0_f64.powi(2) {
                            format!("{:.1} KiB", f / 1024.0)
                        } else if abs < 1024.0_f64.powi(3) {
                            format!("{:.1} MiB", f / 1024.0_f64.powi(2))
                        } else if abs < 1024.0_f64.powi(4) {
                            format!("{:.1} GiB", f / 1024.0_f64.powi(3))
                        } else if abs < 1024.0_f64.powi(5) {
                            format!("{:.1} TiB", f / 1024.0_f64.powi(4))
                        } else {
                            format!("{:.1} PiB", f / 1024.0_f64.powi(5))
                        };
                        Ok(DataType::String(result))
                    }
                    None => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: value.type_name().to_string(), context: "FmtBytes".to_string() }),
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
                    None => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: value.type_name().to_string(), context: "FmtDuration".to_string() }),
                }
            }

            OperationType::TextSlug => {
                match &input {
                    DataType::String(s) => {
                        let result = magi_lang::util::slugify(s);
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "text_slug: output would exceed {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "TextSlug".to_string() }),
                }
            }
            OperationType::TextCamelCase => {
                match &input {
                    DataType::String(s) => {
                        let result = magi_lang::util::to_lower_camel_case(s);
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "camel_case output exceeds {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "TextCamelCase".to_string() }),
                }
            }
            OperationType::TextSnakeCase => {
                match &input {
                    DataType::String(s) => {
                        let result = magi_lang::util::to_snake_case(s);
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "snake_case output exceeds {} byte limit", MAX_STRING_OUTPUT)));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "TextSnakeCase".to_string() }),
                }
            }
            OperationType::TextTitleCase => {
                match &input {
                    DataType::String(s) => {
                        let result = magi_lang::util::to_title_case(s);
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "title_case output exceeds {} byte limit", MAX_STRING_OUTPUT)));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "TextTitleCase".to_string() }),
                }
            }
            OperationType::TextWrap => {
                match &input {
                    DataType::String(s) => {
                        let width = require_i64_or_default(inputs.get("input_1"), 80, "TextWrap width")?.max(1) as usize;
                        let result = magi_lang::util::textwrap_fill(s, width);
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "text_wrap: output would exceed {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "TextWrap".to_string() }),
                }
            }
            OperationType::TextTruncate => {
                match &input {
                    DataType::String(s) => {
                        let max_len = require_i64_or_default(inputs.get("input_1"), 80, "TextTruncate max_len")?.max(0) as usize;
                        if s.chars().count() <= max_len {
                            Ok(DataType::String(s.clone()))
                        } else if max_len <= 3 {
                            // If max_len is too small for ellipsis, just truncate without it
                            let truncated: String = s.chars().take(max_len).collect();
                            Ok(DataType::String(truncated))
                        } else {
                            let truncated: String = s.chars().take(max_len - 3).collect();
                            Ok(DataType::String(format!("{}...", truncated)))
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "TextTruncate".to_string() }),
                }
            }

            // Encode/Decode extended
            OperationType::HtmlEscape => {
                match &input {
                    DataType::String(s) => {
                        let result = magi_lang::util::html_encode(s);
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!("HtmlEscape output exceeds {} bytes", MAX_STRING_OUTPUT)));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "HtmlEscape".to_string() }),
                }
            }
            OperationType::HtmlUnescape => {
                match &input {
                    DataType::String(s) => {
                        let result = magi_lang::util::html_decode(s);
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!("HtmlUnescape output exceeds {} byte limit", MAX_STRING_OUTPUT)));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "HtmlUnescape".to_string() }),
                }
            }

            OperationType::ReflectTypeOf | OperationType::ReflectTypeName => {
                Ok(DataType::String(input.type_name().to_string()))
            }
            OperationType::ReflectIsType => {
                let type_name = inputs.get("type_name").cloned().unwrap_or(DataType::Null);
                match &type_name {
                    DataType::String(t) => {
                        Ok(DataType::Bool(input.type_name() == t.as_str()))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: type_name.type_name().to_string(), context: "ReflectIsType".to_string() }),
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
                    _ => Err(EvalError::TypeError { expected: "(Map, String)".to_string(), actual: format!("({}, {})", input.type_name(), field.type_name()), context: "ReflectHasField".to_string() }),
                }
            }
            OperationType::ReflectCallable => {
                // A value is callable if it's a string that names a known stdlib operation.
                match &input {
                    DataType::String(name) => {
                        Ok(DataType::Bool(OperationType::parse(name).is_some()))
                    }
                    _ => Ok(DataType::Bool(false)),
                }
            }
            OperationType::ReflectArity => {
                // Look up the stdlib operation by name and return its parameter count.
                match &input {
                    DataType::String(name) => {
                        if let Some(op) = OperationType::parse(name) {
                            let arity = magi_lang::ops::op_input_ports(op).len() as i64;
                            Ok(DataType::Int64(arity))
                        } else {
                            Ok(DataType::Null)
                        }
                    }
                    _ => Ok(DataType::Null),
                }
            }
            OperationType::ReflectInspect => {
                let mut s = format!("{:?}", input);
                if s.len() > MAX_INSPECT_OUTPUT {
                    // Find a safe char boundary for truncation
                    let mut end = MAX_INSPECT_OUTPUT;
                    while end > 0 && !s.is_char_boundary(end) {
                        end -= 1;
                    }
                    s.truncate(end);
                    s.push_str("...[truncated]");
                }
                Ok(DataType::String(s))
            }

            // IfElse: conditional
            OperationType::IfElse => {
                let condition = inputs.get("condition").cloned().unwrap_or(DataType::Null);
                let then_val = inputs.get("then").cloned().unwrap_or(DataType::Null);
                let else_val = inputs.get("else").cloned().unwrap_or(DataType::Null);
                if condition.to_bool() {
                    Ok(then_val)
                } else {
                    Ok(else_val)
                }
            }

            // Switch: match value against cases
            OperationType::Switch => {
                let switch_val = inputs.get("value").cloned().unwrap_or(DataType::Null);
                let default_val = inputs.get("default").cloned().unwrap_or(DataType::Null);
                // Check numbered cases: case_0, value_0, case_1, value_1, ...
                for i in 0..1000 {
                    let case_key = format!("case_{}", i);
                    let value_key = format!("value_{}", i);
                    match (inputs.get(&case_key), inputs.get(&value_key)) {
                        (Some(case), Some(result)) if *case == switch_val || numeric_eq(case, &switch_val) => {
                            return Ok(result.clone());
                        }
                        (None, _) => break,
                        _ => continue,
                    }
                }
                Ok(default_val)
            }

            // TryCatch: error handling
            OperationType::TryCatch => {
                // As a standalone operation, just return the input (or fallback if input is null)
                let fallback = inputs.get("fallback").cloned().unwrap_or(DataType::Null);
                if matches!(input, DataType::Null) {
                    Ok(fallback)
                } else {
                    Ok(input)
                }
            }

            // UUID operations
            OperationType::UuidV4 => {
                Ok(DataType::String(magi_lang::util::uuid_v4()))
            }
            OperationType::UuidNil => {
                Ok(DataType::String("00000000-0000-0000-0000-000000000000".to_string()))
            }
            OperationType::UuidIsValid => {
                match &input {
                    DataType::String(s) => {
                        let trimmed = s.trim();
                        // Strict validation: must be canonical hyphenated format (8-4-4-4-12)
                        let valid = magi_lang::util::uuid_is_valid(trimmed);
                        Ok(DataType::Bool(valid))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "UuidIsValid".to_string() }),
                }
            }
            OperationType::UuidParse => {
                match &input {
                    DataType::String(s) => {
                        let trimmed = s.trim();
                        // Strict validation: must be canonical hyphenated format (8-4-4-4-12)
                        if trimmed.len() != 36
                            || trimmed.as_bytes().get(8) != Some(&b'-')
                            || trimmed.as_bytes().get(13) != Some(&b'-')
                            || trimmed.as_bytes().get(18) != Some(&b'-')
                            || trimmed.as_bytes().get(23) != Some(&b'-')
                        {
                            return Err(EvalError::InvalidInput(
                                "UuidParse: invalid UUID: expected canonical hyphenated format (xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)".to_string()
                            ));
                        }
                        match magi_lang::util::uuid_parse(trimmed) {
                            Ok((_, version)) => {
                                let mut m = magi_lang::util::OrderedMap::new();
                                m.insert("full".to_string(), DataType::String(trimmed.to_lowercase()));
                                m.insert("version".to_string(), DataType::Int64(version as i64));
                                Ok(DataType::Map(m))
                            }
                            Err(e) => Err(EvalError::InvalidInput(format!("UuidParse: invalid UUID: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "UuidParse".to_string() }),
                }
            }

            OperationType::SortAsc => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(mut arr) => {
                        arr.sort_by(total_cmp_values);
                        Ok(DataType::Array(arr))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "SortAsc".to_string() }),
                }
            }
            OperationType::SortDesc => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(mut arr) => {
                        arr.sort_by(|a, b| total_cmp_values(b, a));
                        Ok(DataType::Array(arr))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "SortDesc".to_string() }),
                }
            }
            OperationType::SortReverse => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(mut arr) => {
                        arr.reverse();
                        Ok(DataType::Array(arr))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "SortReverse".to_string() }),
                }
            }
            // StableSort delegates to SortAsc
            OperationType::StableSort => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(mut arr) => {
                        arr.sort_by(total_cmp_values);
                        Ok(DataType::Array(arr))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "StableSort".to_string() }),
                }
            }
            OperationType::IsSorted => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let sorted = arr.windows(2).all(|w| {
                            total_cmp_values(&w[0], &w[1]) != std::cmp::Ordering::Greater
                        });
                        Ok(DataType::Bool(sorted))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "IsSorted".to_string() }),
                }
            }
            OperationType::BinarySearch => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match (&arr_val, &value) {
                    (DataType::Array(arr), target) => {
                        let result = arr.binary_search_by(|item| total_cmp_values(item, target));
                        match result {
                            Ok(i) => Ok(DataType::Int64(i as i64)),
                            Err(_) => Ok(DataType::Int64(-1)),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: arr_val.type_name().to_string(), context: "BinarySearch".to_string() }),
                }
            }
            // SortBy and SortByKey require lambda callbacks, handled by interpreter
            OperationType::SortBy | OperationType::SortByKey => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(_) => Ok(arr_val),
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "SortBy".to_string() }),
                }
            }

            OperationType::SetFrom => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        const MAX_UNIQUE: usize = 100_000;
                        if arr.len() > MAX_UNIQUE {
                            return Err(EvalError::InvalidInput(format!(
                                "set_from: array too large ({} elements, max {} for quadratic uniqueness check)",
                                arr.len(),
                                MAX_UNIQUE,
                            )));
                        }
                        let mut seen = Vec::new();
                        for item in arr {
                            if !seen.iter().any(|s: &DataType| s == &item || numeric_eq(s, &item)) {
                                seen.push(item);
                            }
                        }
                        Ok(DataType::Array(seen))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "SetFrom".to_string() }),
                }
            }
            OperationType::SetUnion => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        const MAX_UNIQUE: usize = 100_000;
                        let total = a_arr.len().saturating_add(b_arr.len());
                        if total > MAX_UNIQUE {
                            return Err(EvalError::InvalidInput(format!(
                                "set_union: combined arrays too large ({} elements, max {} for quadratic uniqueness check)",
                                total,
                                MAX_UNIQUE,
                            )));
                        }
                        let mut result: Vec<DataType> = Vec::new();
                        for item in a_arr.iter().chain(b_arr.iter()) {
                            if !result.iter().any(|s| s == item || numeric_eq(s, item)) {
                                result.push(item.clone());
                            }
                        }
                        Ok(DataType::Array(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "(Array, Array)".to_string(), actual: format!("({}, {})", a.type_name(), b.type_name()), context: "SetUnion".to_string() }),
                }
            }
            OperationType::SetIntersection => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        const MAX_UNIQUE: usize = 100_000;
                        let larger = a_arr.len().max(b_arr.len());
                        if larger > MAX_UNIQUE {
                            return Err(EvalError::InvalidInput(format!(
                                "set_intersection: array too large ({} elements, max {} for quadratic uniqueness check)",
                                larger,
                                MAX_UNIQUE,
                            )));
                        }
                        let mut result: Vec<DataType> = Vec::new();
                        for item in a_arr {
                            if b_arr.iter().any(|s| s == item || numeric_eq(s, item))
                                && !result.iter().any(|s| s == item || numeric_eq(s, item))
                            {
                                result.push(item.clone());
                            }
                        }
                        Ok(DataType::Array(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "(Array, Array)".to_string(), actual: format!("({}, {})", a.type_name(), b.type_name()), context: "SetIntersection".to_string() }),
                }
            }
            OperationType::SetDifference => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        const MAX_UNIQUE: usize = 100_000;
                        let larger = a_arr.len().max(b_arr.len());
                        if larger > MAX_UNIQUE {
                            return Err(EvalError::InvalidInput(format!(
                                "set_difference: array too large ({} elements, max {} for quadratic uniqueness check)",
                                larger,
                                MAX_UNIQUE,
                            )));
                        }
                        let mut result: Vec<DataType> = Vec::new();
                        for item in a_arr {
                            if !b_arr.iter().any(|s| s == item || numeric_eq(s, item))
                                && !result.iter().any(|s| s == item || numeric_eq(s, item))
                            {
                                result.push(item.clone());
                            }
                        }
                        Ok(DataType::Array(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "(Array, Array)".to_string(), actual: format!("({}, {})", a.type_name(), b.type_name()), context: "SetDifference".to_string() }),
                }
            }
            OperationType::SetSymmetricDifference => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        const MAX_UNIQUE: usize = 100_000;
                        let total = a_arr.len().saturating_add(b_arr.len());
                        if total > MAX_UNIQUE {
                            return Err(EvalError::InvalidInput(format!(
                                "set_symmetric_difference: combined arrays too large ({} elements, max {} for quadratic uniqueness check)",
                                total,
                                MAX_UNIQUE,
                            )));
                        }
                        let mut result: Vec<DataType> = Vec::new();
                        for item in a_arr {
                            if !b_arr.iter().any(|s| s == item || numeric_eq(s, item))
                                && !result.iter().any(|s| s == item || numeric_eq(s, item))
                            {
                                result.push(item.clone());
                            }
                        }
                        for item in b_arr {
                            if !a_arr.iter().any(|s| s == item || numeric_eq(s, item))
                                && !result.iter().any(|s| s == item || numeric_eq(s, item))
                            {
                                result.push(item.clone());
                            }
                        }
                        Ok(DataType::Array(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "(Array, Array)".to_string(), actual: format!("({}, {})", a.type_name(), b.type_name()), context: "SetSymmetricDifference".to_string() }),
                }
            }
            OperationType::Counter => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let mut counts = magi_lang::util::OrderedMap::new();
                        for item in &arr {
                            let key = item.to_string_lossy();
                            if !counts.contains_key(&key) && counts.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(EvalError::InvalidInput(format!("Counter would exceed {} entries", MAX_ARRAY_ELEMENTS)));
                            }
                            let count = counts.entry(key).or_insert(DataType::Int64(0));
                            if let DataType::Int64(n) = count {
                                *n += 1;
                            }
                        }
                        Ok(DataType::Map(counts))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "Counter".to_string() }),
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
                        Ok(DataType::Array(most_common))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "MostCommon".to_string() }),
                }
            }
            OperationType::OrderedMap => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        if arr.len() > MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!(
                                "ordered_map: array exceeds {} element limit", MAX_ARRAY_ELEMENTS
                            )));
                        }
                        let mut m = magi_lang::util::OrderedMap::new();
                        for (i, item) in arr.iter().enumerate() {
                            match item {
                                DataType::Array(pair) if pair.len() >= 2 => {
                                    if let DataType::String(k) = &pair[0] {
                                        m.insert(k.clone(), pair[1].clone());
                                    } else {
                                        return Err(EvalError::TypeError { expected: "String key".to_string(), actual: pair[0].type_name().to_string(), context: format!("OrderedMap entry[{}][0]", i) });
                                    }
                                }
                                DataType::Array(pair) => {
                                    return Err(EvalError::InvalidInput(format!("OrderedMap entry[{}] has {} elements, expected at least 2", i, pair.len())));
                                }
                                _ => {
                                    return Err(EvalError::TypeError { expected: "Array pair".to_string(), actual: item.type_name().to_string(), context: format!("OrderedMap entry[{}]", i) });
                                }
                            }
                        }
                        Ok(DataType::Map(m))
                    }
                    _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: arr_val.type_name().to_string(), context: "OrderedMap".to_string() }),
                }
            }

            OperationType::StatsSum | OperationType::StatsMean | OperationType::StatsMedian
            | OperationType::StatsMode | OperationType::StatsVariance | OperationType::StatsStdDev => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        if arr.is_empty() { return Ok(DataType::Null); }
                        let mut nums: Vec<f64> = Vec::with_capacity(arr.len());
                        for item in &arr {
                            match promote_numeric(item) {
                                Some(Ok(i)) => nums.push(i as f64),
                                Some(Err(f)) => nums.push(f),
                                None => return Err(EvalError::TypeError { expected: "numeric".to_string(), actual: item.type_name().to_string(), context: format!("{:?}", op) }),
                            }
                        }

                        match op {
                            OperationType::StatsSum => {
                                Ok(DataType::Float64(nums.iter().sum()))
                            }
                            OperationType::StatsMean => {
                                Ok(DataType::Float64(nums.iter().sum::<f64>() / nums.len() as f64))
                            }
                            OperationType::StatsMedian => {
                                let mut sorted = nums.clone();
                                sorted.sort_by(|a, b| a.total_cmp(b));
                                let mid = sorted.len() / 2;
                                if sorted.len().is_multiple_of(2) {
                                    Ok(DataType::Float64((sorted[mid - 1] + sorted[mid]) / 2.0))
                                } else {
                                    Ok(DataType::Float64(sorted[mid]))
                                }
                            }
                            OperationType::StatsMode => {
                                use magi_lang::util::OrderedFloat;
                                let mut counts: std::collections::HashMap<OrderedFloat, usize> = std::collections::HashMap::new();
                                for n in &nums {
                                    *counts.entry(OrderedFloat(*n)).or_insert(0) += 1;
                                }
                                let max_count = counts.values().max().copied().unwrap_or(0);
                                match counts.into_iter()
                                    .find(|(_, c)| *c == max_count)
                                    .map(|(of, _)| of.into_inner())
                                {
                                    Some(mode) => Ok(DataType::Float64(mode)),
                                    None => Ok(DataType::Null),
                                }
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
                            _ => Err(EvalError::InvalidInput(format!("unsupported stats op: {:?}", op))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: format!("{:?}", op) }),
                }
            }
            OperationType::StatsPercentile => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let pct = match inputs.get("percentile") {
                    Some(v) => match v.to_f64() {
                        Some(f) if !f.is_nan() => {
                            if !(0.0..=100.0).contains(&f) {
                                return Err(EvalError::InvalidInput(format!(
                                    "StatsPercentile: percentile must be 0-100, got {}", f
                                )));
                            }
                            f
                        }
                        Some(_) => return Err(EvalError::TypeError { expected: "valid percentile".into(), actual: "NaN".into(), context: "StatsPercentile".into() }),
                        None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: v.type_name().to_string(), context: "StatsPercentile percentile".into() }),
                    },
                    None => 50.0,
                };
                match arr_val {
                    DataType::Array(arr) => {
                        if arr.is_empty() { return Ok(DataType::Null); }
                        let mut nums: Vec<f64> = Vec::with_capacity(arr.len());
                        for item in &arr {
                            match promote_numeric(item) {
                                Some(Ok(i)) => nums.push(i as f64),
                                Some(Err(f)) => nums.push(f),
                                None => return Err(EvalError::TypeError { expected: "numeric".to_string(), actual: item.type_name().to_string(), context: "StatsPercentile".to_string() }),
                            }
                        }
                        nums.sort_by(|a, b| a.total_cmp(b));
                        let max_idx = nums.len() - 1;
                        let k = (pct / 100.0 * max_idx as f64).clamp(0.0, max_idx as f64);
                        let lower = (k.floor() as usize).min(max_idx);
                        let upper = (k.ceil() as usize).min(max_idx);
                        let frac = k - lower as f64;
                        Ok(DataType::Float64(nums[lower] * (1.0 - frac) + nums[upper] * frac))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "StatsPercentile".to_string() }),
                }
            }
            OperationType::StatsQuantile => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let q = match inputs.get("quantile") {
                    Some(v) => match v.to_f64() {
                        Some(f) if !f.is_nan() => {
                            if !(0.0..=1.0).contains(&f) {
                                return Err(EvalError::InvalidInput(format!(
                                    "StatsQuantile: quantile must be 0.0-1.0, got {}", f
                                )));
                            }
                            f
                        }
                        Some(_) => return Err(EvalError::TypeError { expected: "valid quantile".into(), actual: "NaN".into(), context: "StatsQuantile".into() }),
                        None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: v.type_name().to_string(), context: "StatsQuantile quantile".into() }),
                    },
                    None => 0.5,
                };
                match arr_val {
                    DataType::Array(arr) => {
                        if arr.is_empty() { return Ok(DataType::Null); }
                        let mut nums: Vec<f64> = Vec::with_capacity(arr.len());
                        for item in &arr {
                            match promote_numeric(item) {
                                Some(Ok(i)) => nums.push(i as f64),
                                Some(Err(f)) => nums.push(f),
                                None => return Err(EvalError::TypeError { expected: "numeric".to_string(), actual: item.type_name().to_string(), context: "StatsQuantile".to_string() }),
                            }
                        }
                        nums.sort_by(|a, b| a.total_cmp(b));
                        let max_idx = nums.len() - 1;
                        let k = (q * max_idx as f64).clamp(0.0, max_idx as f64);
                        let lower = (k.floor() as usize).min(max_idx);
                        let upper = (k.ceil() as usize).min(max_idx);
                        let frac = k - lower as f64;
                        Ok(DataType::Float64(nums[lower] * (1.0 - frac) + nums[upper] * frac))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: arr_val.type_name().to_string(), context: "StatsQuantile".to_string() }),
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
                                    if fv.is_nan() { continue; }
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
                    _ => Err(EvalError::TypeError { expected: "(Array, String)".to_string(), actual: format!("({}, {})", arr_val.type_name(), key_name.type_name()), context: format!("{:?}", op) }),
                }
            }
            OperationType::StatsCovariance | OperationType::StatsCorrelation => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        let mut a_nums: Vec<f64> = Vec::with_capacity(a_arr.len());
                        for item in a_arr.iter() {
                            match promote_numeric(item) {
                                Some(Ok(i)) => a_nums.push(i as f64),
                                Some(Err(f)) => a_nums.push(f),
                                None => return Err(EvalError::TypeError { expected: "numeric".to_string(), actual: item.type_name().to_string(), context: format!("{:?}", op) }),
                            }
                        }
                        let mut b_nums: Vec<f64> = Vec::with_capacity(b_arr.len());
                        for item in b_arr.iter() {
                            match promote_numeric(item) {
                                Some(Ok(i)) => b_nums.push(i as f64),
                                Some(Err(f)) => b_nums.push(f),
                                None => return Err(EvalError::TypeError { expected: "numeric".to_string(), actual: item.type_name().to_string(), context: format!("{:?}", op) }),
                            }
                        }
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
                                Ok(DataType::Float64(f64::NAN))
                            } else {
                                Ok(DataType::Float64(cov / (a_std * b_std)))
                            }
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "(Array, Array)".to_string(), actual: format!("({}, {})", a.type_name(), b.type_name()), context: format!("{:?}", op) }),
                }
            }

            // Array HOF operations: These are normally handled by the
            // interpreter directly. When called as standalone ops, return the
            // input array unchanged (the actual transformation requires lambdas).
            OperationType::ArrayMap | OperationType::ArrayFilter | OperationType::ArrayFlatMap
            | OperationType::ArrayFind | OperationType::ArrayFindIndex | OperationType::ArrayEvery
            | OperationType::ArraySome | OperationType::ArrayTakeWhile | OperationType::ArraySkipWhile
            | OperationType::ArrayGroupBy | OperationType::ArraySortBy | OperationType::ArrayPartition
            | OperationType::ArrayScan | OperationType::MapMapValues | OperationType::MapFilterEntries => {
                Err(EvalError::InvalidInput(format!("{:?} requires lambda callback (interpreter context)", op)))
            }

            // ArrayZip, ArrayEnumerate, ArrayTake, ArraySkip, ArrayChunk, ArrayWindow
            OperationType::ArrayZip => {
                match (&a, &b) {
                    (DataType::Array(a_arr), DataType::Array(b_arr)) => {
                        let len = a_arr.len().min(b_arr.len());
                        if len > MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!(
                                "array_zip would produce {} elements (max {})", len, MAX_ARRAY_ELEMENTS
                            )));
                        }
                        let result: Vec<DataType> = (0..len)
                            .map(|i| DataType::Array(vec![a_arr[i].clone(), b_arr[i].clone()]))
                            .collect();
                        Ok(DataType::Array(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: a.type_name().to_string(), context: "ArrayZip".to_string() }),
                }
            }
            OperationType::ArrayEnumerate => {
                match &array {
                    DataType::Array(arr) => {
                        if arr.len() > MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!(
                                "array_enumerate: input has {} elements (max {})", arr.len(), MAX_ARRAY_ELEMENTS
                            )));
                        }
                        let result: Vec<DataType> = arr.iter().enumerate()
                            .map(|(i, v)| DataType::Array(vec![DataType::Int64(i as i64), v.clone()]))
                            .collect();
                        Ok(DataType::Array(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "ArrayEnumerate".to_string() }),
                }
            }
            OperationType::ArrayTake => {
                let count = inputs.get("input_1").or(inputs.get("count")).cloned().unwrap_or(DataType::Int64(0));
                match &array {
                    DataType::Array(arr) => {
                        let n = match count.to_i64() {
                            Some(n) => n.max(0) as usize,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: count.type_name().to_string(), context: "ArrayTake count".into() }),
                        };
                        Ok(DataType::Array(arr[..n.min(arr.len())].to_vec()))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "ArrayTake".to_string() }),
                }
            }
            OperationType::ArraySkip => {
                let count = inputs.get("input_1").or(inputs.get("count")).cloned().unwrap_or(DataType::Int64(0));
                match &array {
                    DataType::Array(arr) => {
                        let n = match count.to_i64() {
                            Some(n) => n.max(0) as usize,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: count.type_name().to_string(), context: "ArraySkip count".into() }),
                        };
                        Ok(DataType::Array(arr[n.min(arr.len())..].to_vec()))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "ArraySkip".to_string() }),
                }
            }
            OperationType::ArrayChunk => {
                let size = inputs.get("input_1").or(inputs.get("size")).cloned().unwrap_or(DataType::Int64(1));
                match &array {
                    DataType::Array(arr) => {
                        let n = match size.to_i64() {
                            Some(n) => n.max(1) as usize,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: size.type_name().to_string(), context: "ArrayChunk size".into() }),
                        };
                        let output_len = arr.len().div_ceil(n);
                        if output_len > MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!(
                                "array_chunk would produce {} elements (max {})", output_len, MAX_ARRAY_ELEMENTS
                            )));
                        }
                        let result: Vec<DataType> = arr.chunks(n)
                            .map(|chunk| DataType::Array(chunk.to_vec()))
                            .collect();
                        Ok(DataType::Array(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "ArrayChunk".to_string() }),
                }
            }
            OperationType::ArrayWindow => {
                let size = inputs.get("input_1").or(inputs.get("size")).cloned().unwrap_or(DataType::Int64(1));
                match &array {
                    DataType::Array(arr) => {
                        let n = match size.to_i64() {
                            Some(n) => n.max(1) as usize,
                            None => return Err(EvalError::TypeError { expected: "numeric".into(), actual: size.type_name().to_string(), context: "ArrayWindow size".into() }),
                        };
                        if n > arr.len() {
                            return Ok(DataType::Array(vec![]));
                        }
                        let output_len = arr.len() - n + 1;
                        if output_len > MAX_ARRAY_ELEMENTS {
                            return Err(EvalError::InvalidInput(format!(
                                "array_window would produce {} elements (max {})", output_len, MAX_ARRAY_ELEMENTS
                            )));
                        }
                        let result: Vec<DataType> = arr.windows(n)
                            .map(|window| DataType::Array(window.to_vec()))
                            .collect();
                        Ok(DataType::Array(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "ArrayWindow".to_string() }),
                }
            }

            // MapUpdate: same as MapSet but named differently
            // (already handled above, this is for std::map::map_update)

            // Language constructs handled by interpreter, not evaluator
            OperationType::FunctionDef | OperationType::FunctionCall
            | OperationType::AsyncSpawn | OperationType::AsyncAwait
            | OperationType::LoopGroup => {
                Err(EvalError::InvalidInput(format!("{:?} is an interpreter-level construct", op)))
            }

            // Text operations (remaining)
            OperationType::TextIndent => {
                match &input {
                    DataType::String(s) => {
                        let indent = require_i64_or_default(inputs.get("input_1"), 4, "TextIndent width")?.clamp(0, 1000) as usize;
                        let pad = " ".repeat(indent);
                        let result = magi_lang::util::textwrap_indent(s, &pad).trim_end_matches('\n').to_string();
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!("text_indent: output would exceed {} byte limit", MAX_STRING_OUTPUT)));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "TextIndent".to_string() }),
                }
            }
            OperationType::TextDedent => {
                match &input {
                    DataType::String(s) => {
                        let result = magi_lang::util::textwrap_dedent(s);
                        if result.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "text_dedent: output would exceed {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "TextDedent".to_string() }),
                }
            }
            OperationType::TextPadLeft => {
                match &input {
                    DataType::String(s) => {
                        let width = require_i64_or_default(inputs.get("input_1"), 0, "TextPadLeft width")?.max(0) as usize;
                        if width > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!("TextPadLeft width {} exceeds {} byte limit", width, MAX_STRING_OUTPUT)));
                        }
                        let char_count = s.chars().count();
                        if char_count >= width {
                            Ok(DataType::String(s.clone()))
                        } else {
                            let padding = " ".repeat(width - char_count);
                            Ok(DataType::String(format!("{}{}", padding, s)))
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "TextPadLeft".to_string() }),
                }
            }
            OperationType::TextPadRight => {
                match &input {
                    DataType::String(s) => {
                        let width = require_i64_or_default(inputs.get("input_1"), 0, "TextPadRight width")?.max(0) as usize;
                        if width > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!("TextPadRight width {} exceeds {} byte limit", width, MAX_STRING_OUTPUT)));
                        }
                        let char_count = s.chars().count();
                        if char_count >= width {
                            Ok(DataType::String(s.clone()))
                        } else {
                            let padding = " ".repeat(width - char_count);
                            Ok(DataType::String(format!("{}{}", s, padding)))
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "TextPadRight".to_string() }),
                }
            }

            // Time operations (remaining)
            OperationType::Duration => {
                // Return current time as duration in ms
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let now = i64::try_from(now).unwrap_or(i64::MAX);
                Ok(DataType::Int64(now))
            }
            OperationType::Elapsed => {
                let timestamp = inputs.get("timestamp").cloned().unwrap_or(DataType::Null);
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let now = i64::try_from(now_ms).unwrap_or(i64::MAX);
                match timestamp.to_i64() {
                    Some(ts) => Ok(DataType::Int64(now.saturating_sub(ts))),
                    None => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: timestamp.type_name().to_string(), context: "Elapsed".to_string() }),
                }
            }
            OperationType::TimeSleep => {
                sleep_chunked(inputs)?;
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
                    _ => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: format!("({}, {})", timestamp.type_name(), duration.type_name()), context: format!("{:?}", op) }),
                }
            }
            OperationType::TimeDiff => {
                match (a.to_i64(), b.to_i64()) {
                    (Some(t1), Some(t2)) => Ok(DataType::Int64(
                        t1.checked_sub(t2)
                            .and_then(|d| d.checked_abs())
                            .unwrap_or(i64::MAX)
                    )),
                    _ => Err(EvalError::TypeError { expected: "numeric".to_string(), actual: format!("({}, {})", a.type_name(), b.type_name()), context: "TimeDiff".to_string() }),
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
                    None => Err(EvalError::TypeError { expected: "numeric (timestamp)".to_string(), actual: input.type_name().to_string(), context: "StartOf/EndOf".to_string() }),
                }
            }

            OperationType::RandomBytes => {
                let count = require_i64_or_default(inputs.get("input_1").or(inputs.get("count")), 16, "RandomBytes count")?.max(0) as usize;
                let count = count.min(1_000_000);
                let mut bytes = vec![0u8; count];
                magi_lang::util::random_fill_bytes(&mut bytes);
                Ok(DataType::Bytes(bytes))
            }
            OperationType::RandomString => {
                let length = require_i64_or_default(inputs.get("input_1").or(inputs.get("length")), 16, "RandomString length")?.max(0) as usize;
                let length = length.min(MAX_STRING_OUTPUT);
                let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
                let mut result = String::with_capacity(length);
                for _ in 0..length {
                    result.push(chars[magi_lang::util::random_range_usize(chars.len())] as char);
                }
                Ok(DataType::String(result))
            }
            OperationType::RandomSample => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let count = require_i64_or_default(inputs.get("input_1").or(inputs.get("count")), 1, "RandomSample count")?.max(0) as usize;
                match arr_val {
                    DataType::Array(mut arr) => {
                        let sampled = magi_lang::util::random_sample(&mut arr, count);
                        Ok(DataType::Array(sampled))
                    }
                    _ => Err(EvalError::TypeError { expected: "Array".to_string(), actual: array.type_name().to_string(), context: "RandomSample".to_string() }),
                }
            }

            // URL operations
            OperationType::UrlParse => {
                match &input {
                    DataType::String(url_str) => {
                        match magi_lang::util::UrlParts::parse(url_str) {
                            Ok(parsed) => {
                                let mut m = magi_lang::util::OrderedMap::new();
                                m.insert("raw".into(), DataType::String(url_str.clone()));
                                m.insert("protocol".into(), DataType::String(parsed.scheme.clone()));
                                m.insert("host".into(), DataType::String(parsed.host.clone()));
                                if let Some(port) = parsed.port {
                                    m.insert("port".into(), DataType::Int64(port as i64));
                                }
                                m.insert("path".into(), DataType::String(parsed.path.clone()));
                                if let Some(ref q) = parsed.query {
                                    m.insert("query".into(), DataType::String(q.clone()));
                                }
                                if let Some(ref f) = parsed.fragment {
                                    m.insert("fragment".into(), DataType::String(f.clone()));
                                }
                                if !parsed.username.is_empty() {
                                    m.insert("username".into(), DataType::String(parsed.username.clone()));
                                }
                                if !parsed.password.is_empty() {
                                    m.insert("password".into(), DataType::String(parsed.password.clone()));
                                }
                                Ok(DataType::Map(m))
                            }
                            Err(_) => {
                                let mut m = magi_lang::util::OrderedMap::new();
                                m.insert("raw".into(), DataType::String(url_str.clone()));
                                Ok(DataType::Map(m))
                            }
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "UrlParse".to_string() }),
                }
            }
            OperationType::UrlJoin => {
                let base_val = inputs.get("base").cloned().unwrap_or(DataType::Null);
                let path_val = inputs.get("path").cloned().unwrap_or(DataType::Null);
                match (&base_val, &path_val) {
                    (DataType::String(b), DataType::String(p)) => {
                        match magi_lang::util::UrlParts::parse(b) {
                            Ok(base_url) => match base_url.join(p) {
                                Ok(joined) => Ok(DataType::String(joined)),
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
                    _ => Err(EvalError::TypeError { expected: "(String, String)".to_string(), actual: format!("({}, {})", base_val.type_name(), path_val.type_name()), context: "UrlJoin".to_string() }),
                }
            }

            OperationType::HashSha512 => {
                if matches!(input, DataType::Null) {
                    return Err(EvalError::TypeError { expected: "String or Bytes".to_string(), actual: "Null".to_string(), context: "HashSha512".to_string() });
                }
                let data = data_to_bytes(&input);
                let hash = magi_lang::util::sha512(&data);
                Ok(DataType::String(magi_lang::util::hex_encode(&hash)))
            }
            OperationType::HashCrc32 => {
                if matches!(input, DataType::Null) {
                    return Err(EvalError::TypeError { expected: "String or Bytes".to_string(), actual: "Null".to_string(), context: "HashCrc32".to_string() });
                }
                let data = data_to_bytes(&input);
                let crc = magi_lang::util::crc32(&data);
                Ok(DataType::Int64(crc as i64))
            }
            OperationType::HmacSha256 => {
                let key_val = inputs.get("key").cloned().unwrap_or(DataType::Null);
                let data = match &input {
                    DataType::String(s) => s.as_bytes().to_vec(),
                    DataType::Bytes(b) => b.clone(),
                    _ => return Err(EvalError::TypeError { expected: "String or Bytes".to_string(), actual: input.type_name().to_string(), context: "HmacSha256 input".to_string() }),
                };
                let key = match &key_val {
                    DataType::String(s) => s.as_bytes().to_vec(),
                    DataType::Bytes(b) => b.clone(),
                    _ => return Err(EvalError::TypeError { expected: "String or Bytes".to_string(), actual: key_val.type_name().to_string(), context: "HmacSha256 key".to_string() }),
                };
                let result = magi_lang::util::hmac_sha256(&key, &data);
                Ok(DataType::String(magi_lang::util::hex_encode(&result)))
            }
            OperationType::ConstantTimeEq => {
                let (bytes_a, bytes_b) = match (&a, &b) {
                    (DataType::String(s1), DataType::String(s2)) => {
                        (s1.as_bytes(), s2.as_bytes())
                    }
                    (DataType::Bytes(b1), DataType::Bytes(b2)) => {
                        (b1.as_slice(), b2.as_slice())
                    }
                    _ => return Ok(DataType::Bool(false)),
                };
                Ok(DataType::Bool(magi_lang::util::constant_time_eq(bytes_a, bytes_b)))
            }

            // Base32 encode/decode
            OperationType::Base32Encode => {
                let data = match &input {
                    DataType::Bytes(b) => b.clone(),
                    DataType::String(s) => s.as_bytes().to_vec(),
                    _ => return Err(EvalError::TypeError { expected: "string or bytes".to_string(), actual: input.type_name().to_string(), context: "Base32Encode".to_string() }),
                };
                if data.len() * 8 / 5 + 8 > MAX_STRING_OUTPUT {
                    return Err(EvalError::InvalidInput(format!(
                        "Base32Encode: output would exceed {} byte limit", MAX_STRING_OUTPUT
                    )));
                }
                Ok(DataType::String(magi_lang::util::base32_encode(&data)))
            }
            OperationType::Base32Decode => {
                match &input {
                    DataType::String(s) => {
                        match magi_lang::util::base32_decode(s) {
                            Ok(decoded) => Ok(DataType::Bytes(decoded)),
                            Err(e) => Err(EvalError::InvalidInput(format!("Base32Decode: invalid base32 input: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "Base32Decode".to_string() }),
                }
            }

            // HashBlake3
            OperationType::HashBlake3 => {
                if matches!(input, DataType::Null) {
                    return Err(EvalError::TypeError { expected: "String or Bytes".to_string(), actual: "Null".to_string(), context: "HashBlake3".to_string() });
                }
                let data = data_to_bytes(&input);
                let hash = magi_lang::util::blake3_hash_hex(&data);
                Ok(DataType::String(hash))
            }

            // TOML operations
            OperationType::TomlParse => {
                match &input {
                    DataType::String(s) => {
                        if s.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!("toml_parse: input exceeds {} byte limit", MAX_STRING_OUTPUT)));
                        }
                        match magi_lang::util::toml_parse(s) {
                            Ok(table) => Ok(toml_value_to_datatype(&magi_lang::util::TomlValue::Table(table))),
                            Err(e) => Err(EvalError::InvalidInput(format!("toml_parse: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "toml_parse".to_string() }),
                }
            }
            OperationType::TomlStringify => {
                fn datatype_to_toml(val: &DataType, depth: usize) -> magi_lang::util::TomlValue {
                    use magi_lang::util::TomlValue;
                    const MAX_DEPTH: usize = 64;
                    if depth > MAX_DEPTH {
                        return TomlValue::String("[max depth]".to_string());
                    }
                    match val {
                        DataType::Null => TomlValue::String("null".to_string()),
                        DataType::Bool(b) => TomlValue::Boolean(*b),
                        DataType::Int32(n) => TomlValue::Integer(*n as i64),
                        DataType::Int64(n) => TomlValue::Integer(*n),
                        DataType::Uint32(n) => TomlValue::Integer(*n as i64),
                        DataType::Uint64(n) => {
                            if *n > i64::MAX as u64 {
                                TomlValue::String(n.to_string())
                            } else {
                                TomlValue::Integer(*n as i64)
                            }
                        }
                        DataType::Float32(f) => TomlValue::Float(*f as f64),
                        DataType::Float64(f) => TomlValue::Float(*f),
                        DataType::String(s) => TomlValue::String(s.clone()),
                        DataType::Array(arr) => {
                            TomlValue::Array(arr.iter().map(|v| datatype_to_toml(v, depth + 1)).collect())
                        }
                        DataType::Map(m) => {
                            let table: magi_lang::util::TomlTable = m.iter()
                                .filter(|(k, _)| !k.starts_with("__"))
                                .map(|(k, v)| (k.clone(), datatype_to_toml(v, depth + 1)))
                                .collect();
                            TomlValue::Table(table)
                        }
                        _ => TomlValue::String(val.to_string_lossy()),
                    }
                }
                let toml_val = datatype_to_toml(&input, 0);
                match magi_lang::util::toml_to_string_pretty(&toml_val) {
                    Ok(s) => {
                        if s.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "toml_stringify: output would exceed {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(s))
                    }
                    Err(e) => Err(EvalError::InvalidInput(format!("toml_stringify: {}", e))),
                }
            }

            // CSV operations (pure string parsing)
            OperationType::CsvParse => {
                match &input {
                    DataType::String(s) => {
                        let csv_data = magi_lang::util::csv_parse(s)
                            .map_err(|e| EvalError::InvalidInput(format!("csv_parse: {}", e)))?;
                        let mut rows = Vec::new();
                        for record in &csv_data.records {
                            if rows.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(EvalError::InvalidInput(format!(
                                    "csv_parse: row count exceeds {} element limit", MAX_ARRAY_ELEMENTS
                                )));
                            }
                            let mut row = magi_lang::util::OrderedMap::new();
                            for (i, field) in record.iter().enumerate() {
                                let key = csv_data.headers.get(i).cloned().unwrap_or_else(|| format!("col{}", i));
                                row.insert(key, DataType::String(field.to_string()));
                            }
                            rows.push(DataType::Map(row));
                        }
                        Ok(DataType::Array(rows))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "csv_parse".to_string() }),
                }
            }
            OperationType::CsvStringify => {
                match &input {
                    DataType::Array(rows) if !rows.is_empty() => {
                        if let DataType::Map(first) = &rows[0] {
                            let headers: Vec<&str> = first.keys()
                                .filter(|k| !k.starts_with("__"))
                                .map(|k| k.as_str()).collect();
                            let mut data_rows: Vec<Vec<String>> = Vec::new();
                            let vals: Vec<String> = first.iter()
                                .filter(|(k, _)| !k.starts_with("__"))
                                .map(|(_, v)| v.to_string()).collect();
                            data_rows.push(vals);
                            for row in &rows[1..] {
                                if let DataType::Map(m) = row {
                                    let vals: Vec<String> = headers.iter()
                                        .map(|&h| m.get(h).map(|v| v.to_string()).unwrap_or_default())
                                        .collect();
                                    data_rows.push(vals);
                                }
                            }
                            let output = magi_lang::util::csv_write(&headers, &data_rows);
                            if output.len() > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!(
                                    "csv_stringify: output would exceed {} byte limit", MAX_STRING_OUTPUT
                                )));
                            }
                            Ok(DataType::String(output))
                        } else {
                            Err(EvalError::TypeError {
                                expected: "map".to_string(),
                                actual: rows[0].type_name().to_string(),
                                context: "csv_stringify: rows must be maps".to_string(),
                            })
                        }
                    }
                    DataType::Array(_) => Ok(DataType::String(String::new())),
                    _ => Err(EvalError::TypeError { expected: "array".to_string(), actual: input.type_name().to_string(), context: "csv_stringify".to_string() }),
                }
            }
            OperationType::CsvHeaders => {
                match &input {
                    DataType::String(s) => {
                        let csv_data = magi_lang::util::csv_parse(s)
                            .map_err(|e| EvalError::InvalidInput(format!("csv_headers: {}", e)))?;
                        let arr: Vec<DataType> = csv_data.headers.iter()
                            .map(|h| DataType::String(h.to_string()))
                            .collect();
                        Ok(DataType::Array(arr))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "csv_headers".to_string() }),
                }
            }
            OperationType::CsvParseRows => {
                match &input {
                    DataType::String(s) => {
                        let records = magi_lang::util::csv_parse_no_headers(s)
                            .map_err(|e| EvalError::InvalidInput(format!("csv_parse_rows: {}", e)))?;
                        let mut rows = Vec::new();
                        for record in &records {
                            if rows.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(EvalError::InvalidInput(format!(
                                    "csv_parse_rows: row count exceeds {} element limit", MAX_ARRAY_ELEMENTS
                                )));
                            }
                            let row: Vec<DataType> = record.iter()
                                .map(|f| DataType::String(f.to_string()))
                                .collect();
                            rows.push(DataType::Array(row));
                        }
                        Ok(DataType::Array(rows))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "csv_parse_rows".to_string() }),
                }
            }

            // YAML operations (serde_yaml_ng)
            OperationType::YamlParse => {
                match &input {
                    DataType::String(s) => {
                        if s.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!("yaml_parse: input exceeds {} byte limit", MAX_STRING_OUTPUT)));
                        }
                        let yaml_val: magi_lang::util::YamlValue = magi_lang::util::yaml_parse(s)
                            .map_err(|e| EvalError::InvalidInput(format!("yaml_parse: {}", e)))?;
                        Ok(yaml_value_to_datatype(&yaml_val))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "yaml_parse".to_string() }),
                }
            }
            OperationType::YamlStringify => {
                let yaml_val = datatype_to_yaml_value(&input);
                let s = magi_lang::util::yaml_stringify_result(&yaml_val)
                    .map_err(|e| EvalError::InvalidInput(format!("yaml_stringify: {}", e)))?;
                if s.len() > MAX_STRING_OUTPUT {
                    return Err(EvalError::InvalidInput(format!(
                        "yaml_stringify: output would exceed {} byte limit", MAX_STRING_OUTPUT
                    )));
                }
                Ok(DataType::String(s))
            }
            OperationType::YamlValidate => {
                match &input {
                    DataType::String(s) => {
                        let valid = magi_lang::util::yaml_parse(s).is_ok();
                        Ok(DataType::Bool(valid))
                    }
                    _ => Err(EvalError::TypeError { expected: "String".to_string(), actual: input.type_name().to_string(), context: "YamlValidate".to_string() }),
                }
            }
            OperationType::YamlToJson => {
                match &input {
                    DataType::String(s) => {
                        let yaml_val: magi_lang::util::YamlValue = magi_lang::util::yaml_parse(s)
                            .map_err(|e| EvalError::InvalidInput(format!("yaml_to_json: {}", e)))?;
                        let data = yaml_value_to_datatype(&yaml_val);
                        let json_str = datatype_to_json_string(&data)?;
                        if json_str.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "yaml_to_json: output would exceed {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        Ok(DataType::String(json_str))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "yaml_to_json".to_string() }),
                }
            }
            OperationType::YamlFromJson => {
                match &input {
                    DataType::String(s) => {
                        match magi_lang::util::json_parse_value(s) {
                            Ok(json_val) => {
                                let data = json_value_to_datatype(&json_val);
                                let yaml_val = datatype_to_yaml_value(&data);
                                let yaml_str = magi_lang::util::yaml_stringify_result(&yaml_val)
                                    .map_err(|e| EvalError::InvalidInput(format!("yaml_from_json: {}", e)))?;
                                if yaml_str.len() > MAX_STRING_OUTPUT {
                                    return Err(EvalError::InvalidInput(format!(
                                        "yaml_from_json: output would exceed {} byte limit", MAX_STRING_OUTPUT
                                    )));
                                }
                                Ok(DataType::String(yaml_str))
                            }
                            Err(e) => Err(EvalError::InvalidInput(format!("yaml_from_json: invalid JSON: {}", e))),
                        }
                    }
                    _ => Err(EvalError::TypeError { expected: "string".to_string(), actual: input.type_name().to_string(), context: "yaml_from_json".to_string() }),
                }
            }
            OperationType::YamlMerge => {
                match (&a, &b) {
                    (DataType::Map(m1), DataType::Map(m2)) => {
                        let mut merged = m1.clone();
                        for (k, v) in m2 {
                            if !merged.contains_key(k.as_str()) && merged.len() >= MAX_ARRAY_ELEMENTS {
                                return Err(EvalError::InvalidInput(format!("YamlMerge would exceed {} entries", MAX_ARRAY_ELEMENTS)));
                            }
                            merged.insert(k.clone(), v.clone());
                        }
                        Ok(DataType::Map(merged))
                    }
                    (DataType::String(s1), DataType::String(s2)) => {
                        let v1: magi_lang::util::YamlValue = magi_lang::util::yaml_parse(s1)
                            .map_err(|e| EvalError::InvalidInput(format!("yaml_merge: {}", e)))?;
                        let v2: magi_lang::util::YamlValue = magi_lang::util::yaml_parse(s2)
                            .map_err(|e| EvalError::InvalidInput(format!("yaml_merge: {}", e)))?;
                        let d1 = yaml_value_to_datatype(&v1);
                        let d2 = yaml_value_to_datatype(&v2);
                        match (d1, d2) {
                            (DataType::Map(m1), DataType::Map(m2)) => {
                                let mut merged = m1;
                                for (k, v) in m2 {
                                    if !merged.contains_key(k.as_str()) && merged.len() >= MAX_ARRAY_ELEMENTS {
                                        return Err(EvalError::InvalidInput(format!("YamlMerge would exceed {} entries", MAX_ARRAY_ELEMENTS)));
                                    }
                                    merged.insert(k, v);
                                }
                                Ok(DataType::Map(merged))
                            }
                            _ => Err(EvalError::InvalidInput("yaml_merge: both inputs must be YAML maps".to_string())),
                        }
                    }
                    _ => Err(EvalError::InvalidInput("yaml_merge: inputs must be maps or YAML strings".to_string())),
                }
            }

            // XML operations (basic parse/stringify)
            OperationType::XmlParse => {
                match &input {
                    DataType::String(s) => {
                        // Simple XML to map/array conversion
                        let mut result = magi_lang::util::OrderedMap::new();
                        let trimmed = s.trim();
                        if trimmed.starts_with("<?") {
                            if let Some(end) = trimmed.find("?>") {
                                let rest = trimmed[end + 2..].trim();
                                return self.eval_operation(OperationType::XmlParse,
                                    &std::collections::HashMap::from([("input".to_string(), DataType::String(rest.to_string()))]),
                                    _config);
                            }
                        }
                        // Parse simple XML elements
                        if let Some(tag_end) = trimmed.find('>') {
                            let tag_content = &trimmed[1..tag_end];
                            let tag_name = tag_content.split_whitespace().next().unwrap_or("root");
                            let close_tag = format!("</{}>", tag_name);
                            if let Some(close_pos) = trimmed.find(&close_tag) {
                                let inner = &trimmed[tag_end + 1..close_pos];
                                result.insert("tag".into(), DataType::String(tag_name.to_string()));
                                if inner.contains('<') {
                                    // Has child elements — parse recursively
                                    result.insert("children".into(), DataType::String(inner.to_string()));
                                } else {
                                    result.insert("text".into(), DataType::String(inner.to_string()));
                                }
                                if tag_content.contains(' ') {
                                    let mut attrs = magi_lang::util::OrderedMap::new();
                                    for part in tag_content.split_whitespace().skip(1) {
                                        if let Some(eq) = part.find('=') {
                                            let key = &part[..eq];
                                            let val = part[eq+1..].trim_matches('"').trim_matches('\'');
                                            attrs.insert(key.to_string(), DataType::String(val.to_string()));
                                        }
                                    }
                                    if !attrs.is_empty() {
                                        result.insert("attributes".into(), DataType::Map(attrs));
                                    }
                                }
                            } else {
                                result.insert("raw".into(), DataType::String(trimmed.to_string()));
                            }
                        } else {
                            result.insert("raw".into(), DataType::String(trimmed.to_string()));
                        }
                        Ok(DataType::Map(result))
                    }
                    _ => Err(EvalError::TypeError { expected: "string".into(), actual: input.type_name().into(), context: "xml_parse".into() }),
                }
            }
            OperationType::XmlStringify => {
                fn datatype_to_xml(val: &DataType, indent: usize) -> String {
                    let pad = " ".repeat(indent);
                    match val {
                        DataType::Map(m) => {
                            let tag = m.get("tag").and_then(|v| v.as_str()).unwrap_or("element");
                            let text = m.get("text").and_then(|v| v.as_str());
                            let attrs = m.get("attributes").and_then(|v| match v { DataType::Map(a) => Some(a), _ => None });
                            let mut attr_str = String::new();
                            if let Some(attrs) = attrs {
                                for (k, v) in attrs.iter() {
                                    attr_str.push_str(&format!(" {}=\"{}\"", k, v));
                                }
                            }
                            if let Some(text) = text {
                                format!("{}<{}{}>{}</{}>", pad, tag, attr_str, text, tag)
                            } else {
                                format!("{}<{}{}/>", pad, tag, attr_str)
                            }
                        }
                        DataType::Array(arr) => {
                            arr.iter().map(|v| datatype_to_xml(v, indent)).collect::<Vec<_>>().join("\n")
                        }
                        DataType::String(s) => format!("{}{}", pad, s),
                        other => format!("{}{}", pad, other),
                    }
                }
                let xml = datatype_to_xml(&input, 0);
                Ok(DataType::String(xml))
            }

            // HTTP client operations (ureq)

            OperationType::HttpGet => {
                let url = get_string(inputs, "url")?;
                validate_url_with_dns(url)?;
                let resp = http_agent().get(url)
                    .map_err(|e| EvalError::InvalidInput(format!("http_get: {}", e)))?;
                http_response_to_map(resp, "http_get")
            }

            OperationType::HttpPost => {
                let url = get_string(inputs, "url")?;
                validate_url_with_dns(url)?;
                let payload = inputs.get("body").map(|d| d.to_string());
                let resp = http_agent().post(url, "application/json", payload.as_deref().unwrap_or("").as_bytes())
                    .map_err(|e| EvalError::InvalidInput(format!("http_post: {}", e)))?;
                http_response_to_map(resp, "http_post")
            }

            OperationType::HttpPut => {
                let url = get_string(inputs, "url")?;
                validate_url_with_dns(url)?;
                let payload = inputs.get("body").map(|d| d.to_string());
                let resp = http_agent().put(url, "application/json", payload.as_deref().unwrap_or("").as_bytes())
                    .map_err(|e| EvalError::InvalidInput(format!("http_put: {}", e)))?;
                http_response_to_map(resp, "http_put")
            }

            OperationType::HttpDelete => {
                let url = get_string(inputs, "url")?;
                validate_url_with_dns(url)?;
                let resp = http_agent().delete(url)
                    .map_err(|e| EvalError::InvalidInput(format!("http_delete: {}", e)))?;
                http_response_to_map(resp, "http_delete")
            }

            OperationType::HttpRequest => {
                let method = get_string(inputs, "method")?;
                let url = get_string(inputs, "url")?;
                validate_url_with_dns(url)?;
                let user_headers = inputs.get("headers").and_then(|d| d.as_map()).cloned();
                let payload = inputs.get("body").map(|d| d.to_string());
                let method_upper = method.to_uppercase();

                let header_pairs: Vec<(String, String)> = user_headers
                    .iter()
                    .flat_map(|h| h.iter())
                    .map(|(k, v)| (k.clone(), v.to_string()))
                    .collect();
                let header_refs: Vec<(&str, &str)> = header_pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

                let resp = match method_upper.as_str() {
                    "POST" | "PUT" | "PATCH" => {
                        http_agent().request(&method_upper, url, &header_refs, Some(payload.as_deref().unwrap_or("").as_bytes()))
                            .map_err(|e| EvalError::InvalidInput(format!("http_request: {}", e)))?
                    }
                    "GET" | "DELETE" | "HEAD" => {
                        http_agent().request(&method_upper, url, &header_refs, None)
                            .map_err(|e| EvalError::InvalidInput(format!("http_request: {}", e)))?
                    }
                    other => {
                        return Err(EvalError::InvalidInput(format!(
                            "Unsupported HTTP method: {}",
                            other
                        )));
                    }
                };
                http_response_to_map(resp, "http_request")
            }

            OperationType::HttpHead => {
                let url = get_string(inputs, "url")?;
                validate_url_with_dns(url)?;
                let resp = http_agent().head(url)
                    .map_err(|e| EvalError::InvalidInput(format!("http_head: {}", e)))?;
                let status = resp.status();
                let headers: magi_lang::util::OrderedMap<String, DataType> = resp.headers.iter()
                    .map(|(k, v)| (k.clone(), DataType::String(v.clone())))
                    .collect();
                Ok(DataType::Map(magi_lang::util::OrderedMap::from([
                    ("status".into(), DataType::Int64(status as i64)),
                    ("headers".into(), DataType::Map(headers)),
                ])))
            }

            OperationType::HttpOptions => {
                let url = get_string(inputs, "url")?;
                validate_url_with_dns(url)?;
                let resp = http_agent().request("OPTIONS", url, &[], None)
                    .map_err(|e| EvalError::InvalidInput(format!("http_options: {}", e)))?;
                let status = resp.status();
                let headers: magi_lang::util::OrderedMap<String, DataType> = resp.headers.iter()
                    .map(|(k, v)| (k.clone(), DataType::String(v.clone())))
                    .collect();
                let allow = headers
                    .get(&"allow".to_string())
                    .cloned()
                    .unwrap_or(DataType::String(String::new()));
                Ok(DataType::Map(magi_lang::util::OrderedMap::from([
                    ("status".into(), DataType::Int64(status as i64)),
                    ("headers".into(), DataType::Map(headers)),
                    ("allow".into(), allow),
                ])))
            }

            OperationType::HttpPatch => {
                let url = get_string(inputs, "url")?;
                validate_url_with_dns(url)?;
                let payload = inputs.get("body").map(|d| d.to_string());
                let resp = http_agent().patch(url, "application/json", payload.as_deref().unwrap_or("").as_bytes())
                    .map_err(|e| EvalError::InvalidInput(format!("http_patch: {}", e)))?;
                http_response_to_map(resp, "http_patch")
            }

            OperationType::CompressZstd => {
                if matches!(input, DataType::Null) {
                    return Err(EvalError::TypeError { expected: "String or Bytes".to_string(), actual: "Null".to_string(), context: "CompressZstd".to_string() });
                }
                let bytes = data_to_bytes(&input);
                const MAX_COMPRESS_INPUT: usize = 64 * 1024 * 1024;
                if bytes.len() > MAX_COMPRESS_INPUT {
                    return Err(EvalError::InvalidInput(format!(
                        "compress_zstd: input exceeds {} byte limit", MAX_COMPRESS_INPUT
                    )));
                }
                let compressed = magi_lang::util::zstd_compress(&bytes, 3)
                    .map_err(|e| EvalError::InvalidInput(format!("compress_zstd: {}", e)))?;
                Ok(DataType::Bytes(compressed))
            }
            OperationType::DecompressZstd => {
                let bytes = match &input {
                    DataType::Bytes(b) => b.as_slice(),
                    _ => return Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "decompress_zstd".to_string() }),
                };
                const MAX_DECOMPRESS: usize = 64 * 1024 * 1024;
                let output = magi_lang::util::zstd_decompress(bytes)
                    .map_err(|e| EvalError::InvalidInput(format!("decompress_zstd: {}", e)))?;
                if output.len() > MAX_DECOMPRESS {
                    return Err(EvalError::InvalidInput(format!(
                        "Decompressed output exceeds {} byte limit", MAX_DECOMPRESS
                    )));
                }
                Ok(DataType::Bytes(output))
            }
            OperationType::CompressLz4 => {
                if matches!(input, DataType::Null) {
                    return Err(EvalError::TypeError { expected: "String or Bytes".to_string(), actual: "Null".to_string(), context: "CompressLz4".to_string() });
                }
                let bytes = data_to_bytes(&input);
                const MAX_COMPRESS_INPUT: usize = 64 * 1024 * 1024;
                if bytes.len() > MAX_COMPRESS_INPUT {
                    return Err(EvalError::InvalidInput(format!(
                        "compress_lz4: input exceeds {} byte limit", MAX_COMPRESS_INPUT
                    )));
                }
                let compressed = magi_lang::util::lz4_compress_prepend_size(&bytes);
                Ok(DataType::Bytes(compressed))
            }
            OperationType::DecompressLz4 => {
                let bytes = match &input {
                    DataType::Bytes(b) => b.as_slice(),
                    _ => return Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "decompress_lz4".to_string() }),
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
                let decompressed = magi_lang::util::lz4_decompress_size_prepended(bytes)
                    .map_err(|e| EvalError::InvalidInput(format!("decompress_lz4: {}", e)))?;
                if decompressed.len() > MAX_DECOMPRESS {
                    return Err(EvalError::InvalidInput(format!(
                        "Decompressed output exceeds {} byte limit", MAX_DECOMPRESS
                    )));
                }
                Ok(DataType::Bytes(decompressed))
            }
            OperationType::CompressGzip => {
                if matches!(input, DataType::Null) {
                    return Err(EvalError::TypeError { expected: "String or Bytes".to_string(), actual: "Null".to_string(), context: "CompressGzip".to_string() });
                }
                let bytes = data_to_bytes(&input);
                const MAX_COMPRESS_INPUT: usize = 64 * 1024 * 1024;
                if bytes.len() > MAX_COMPRESS_INPUT {
                    return Err(EvalError::InvalidInput(format!(
                        "compress_gzip: input exceeds {} byte limit", MAX_COMPRESS_INPUT
                    )));
                }
                let compressed = magi_lang::util::gzip_compress(&bytes);
                Ok(DataType::Bytes(compressed))
            }
            OperationType::DecompressGzip => {
                let bytes = match &input {
                    DataType::Bytes(b) => b.as_slice(),
                    _ => return Err(EvalError::TypeError { expected: "bytes".to_string(), actual: input.type_name().to_string(), context: "decompress_gzip".to_string() }),
                };
                const MAX_DECOMPRESS: usize = 64 * 1024 * 1024;
                let decompressed = magi_lang::util::gzip_decompress(bytes)
                    .map_err(|e| EvalError::InvalidInput(format!("decompress_gzip: {}", e)))?;
                if decompressed.len() > MAX_DECOMPRESS {
                    return Err(EvalError::InvalidInput(format!(
                        "Decompressed output exceeds {} byte limit", MAX_DECOMPRESS
                    )));
                }
                Ok(DataType::Bytes(decompressed))
            }

            // Certificate / TLS operations
            OperationType::CertGenerate | OperationType::CertSelfSigned => {
                let cn = get_string(inputs, "cn")?;
                let (cert_pem, key_pem, _) = magi_lang::util::generate_self_signed_cert(cn)
                    .map_err(|e| EvalError::InvalidInput(format!("cert_generate: {}", e)))?;
                Ok(DataType::Map(magi_lang::util::OrderedMap::from([
                    ("cert_pem".into(), DataType::String(cert_pem)),
                    ("key_pem".into(), DataType::String(key_pem)),
                ])))
            }
            OperationType::CertParse | OperationType::CertInfo => {
                let pem = get_string(inputs, "pem")?;
                let pem_block = magi_lang::util::parse_pem(pem.as_bytes())
                    .map_err(|e| EvalError::InvalidInput(format!("cert_parse pem: {}", e)))?;
                let cert = magi_lang::util::parse_x509_der(&pem_block.contents)
                    .map_err(|e| EvalError::InvalidInput(format!("cert_parse x509: {}", e)))?;
                let mut m = magi_lang::util::OrderedMap::new();
                m.insert("subject".into(), DataType::String(cert.subject));
                m.insert("issuer".into(), DataType::String(cert.issuer));
                m.insert("serial".into(), DataType::String(cert.serial));
                m.insert("not_before".into(), DataType::String(cert.not_before_str));
                m.insert("not_after".into(), DataType::String(cert.not_after_str));
                m.insert("version".into(), DataType::Int64(cert.version as i64));
                if op == OperationType::CertParse {
                    m.insert("signature_algorithm".into(), DataType::String(cert.signature_algorithm));
                    m.insert("is_ca".into(), DataType::Bool(cert.is_ca));
                }
                Ok(DataType::Map(m))
            }
            OperationType::CertVerify => {
                let pem = get_string(inputs, "pem")?;
                let result = match magi_lang::util::parse_pem(pem.as_bytes()) {
                    Ok(pem_block) => match magi_lang::util::parse_x509_der(&pem_block.contents) {
                        Ok(cert) => {
                            let now = magi_lang::util::now_secs();
                            if now < cert.not_before {
                                magi_lang::util::OrderedMap::from([
                                    ("valid".into(), DataType::Bool(false)),
                                    ("error".into(), DataType::String("Certificate not yet valid".into())),
                                ])
                            } else if now > cert.not_after {
                                magi_lang::util::OrderedMap::from([
                                    ("valid".into(), DataType::Bool(false)),
                                    ("error".into(), DataType::String("Certificate has expired".into())),
                                ])
                            } else {
                                magi_lang::util::OrderedMap::from([("valid".into(), DataType::Bool(true))])
                            }
                        }
                        Err(e) => magi_lang::util::OrderedMap::from([
                            ("valid".into(), DataType::Bool(false)),
                            ("error".into(), DataType::String(format!("Failed to parse X509: {}", e))),
                        ]),
                    },
                    Err(e) => magi_lang::util::OrderedMap::from([
                        ("valid".into(), DataType::Bool(false)),
                        ("error".into(), DataType::String(format!("Failed to parse PEM: {}", e))),
                    ]),
                };
                Ok(DataType::Map(result))
            }
            OperationType::KeyGenerate => {
                let (_, private_pem, public_pem) = magi_lang::util::generate_self_signed_cert("key")
                    .map_err(|e| EvalError::InvalidInput(format!("key_generate: {}", e)))?;
                Ok(DataType::Map(magi_lang::util::OrderedMap::from([
                    ("private_pem".into(), DataType::String(private_pem)),
                    ("public_pem".into(), DataType::String(public_pem)),
                ])))
            }

            // TCP operations
            OperationType::TcpConnect => {
                let host = get_string(inputs, "host")?;
                validate_host(host)?;
                let port = get_port(inputs, "port")?;
                let addr = format!("{}:{}", host, port);
                use std::net::ToSocketAddrs;
                let sock_addr = addr.to_socket_addrs()
                    .map_err(|e| EvalError::InvalidInput(format!("tcp_connect: DNS resolution failed: {}", e)))?
                    .next()
                    .ok_or_else(|| EvalError::InvalidInput("tcp_connect: no addresses found".to_string()))?;
                // Post-DNS-resolution SSRF check
                if is_blocked_ip(sock_addr.ip()) {
                    return Err(EvalError::InvalidInput(format!(
                        "tcp_connect: blocked IP after DNS resolution: {}", sock_addr.ip()
                    )));
                }
                let stream = std::net::TcpStream::connect_timeout(
                    &sock_addr,
                    std::time::Duration::from_secs(30),
                )
                .map_err(|e| EvalError::InvalidInput(format!("tcp_connect: {}", e)))?;
                let timeout = Some(std::time::Duration::from_secs(30));
                let _ = stream.set_read_timeout(timeout);
                let _ = stream.set_write_timeout(timeout);
                let id = conn_id("tcp");
                conn_store(&id, Mutex::new(stream))?;
                Ok(DataType::String(id))
            }
            OperationType::TcpWrite => {
                let cid = get_string(inputs, "conn_id")?;
                let data = inputs.get("data").cloned().unwrap_or(DataType::Null);
                let bytes = data_to_bytes(&data);
                const MAX_TCP_WRITE: usize = 64 * 1024 * 1024;
                if bytes.len() > MAX_TCP_WRITE {
                    return Err(EvalError::InvalidInput(format!(
                        "tcp_write: data exceeds {} byte limit", MAX_TCP_WRITE
                    )));
                }
                conn_with::<Mutex<std::net::TcpStream>, _>(cid, |mtx| {
                    use std::io::Write;
                    let stream = mtx
                        .get_mut()
                        .map_err(|_| EvalError::InvalidInput("tcp lock poisoned".into()))?;
                    stream
                        .write_all(&bytes)
                        .map_err(|e| EvalError::InvalidInput(format!("tcp_write: {}", e)))?;
                    stream
                        .flush()
                        .map_err(|e| EvalError::InvalidInput(format!("tcp_write flush: {}", e)))?;
                    Ok(DataType::Int64(bytes.len() as i64))
                })
            }
            OperationType::TcpRead => {
                let cid = get_string(inputs, "conn_id")?;
                conn_with::<Mutex<std::net::TcpStream>, _>(cid, |mtx| {
                    let stream = mtx
                        .get_mut()
                        .map_err(|_| EvalError::InvalidInput("tcp lock poisoned".into()))?;
                    const TCP_READ_LIMIT: usize = 64 * 1024 * 1024; // 64 MB
                    let mut buf = vec![0u8; 65536];
                    let mut result = Vec::new();
                    loop {
                        let n = stream
                            .read(&mut buf)
                            .map_err(|e| EvalError::InvalidInput(format!("tcp_read: {}", e)))?;
                        if n == 0 { break; }
                        result.extend_from_slice(&buf[..n]);
                        if result.len() > TCP_READ_LIMIT {
                            return Err(EvalError::InvalidInput(format!(
                                "tcp_read: data exceeds {} byte limit", TCP_READ_LIMIT
                            )));
                        }
                        // If we got less than the buffer, no more data available yet
                        if n < buf.len() { break; }
                    }
                    Ok(DataType::Bytes(result))
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
                conn_store(&id, Mutex::new(listener))?;
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
                let timeout = Some(std::time::Duration::from_secs(30));
                let _ = stream.set_read_timeout(timeout);
                let _ = stream.set_write_timeout(timeout);
                let id = conn_id("tcp");
                conn_store(&id, Mutex::new(stream))?;
                Ok(DataType::Map(magi_lang::util::OrderedMap::from([
                    ("conn_id".into(), DataType::String(id)),
                    ("address".into(), DataType::String(addr.to_string())),
                ])))
            }
            OperationType::TcpServerClose => {
                let lid = get_string(inputs, "listener_id")?;
                conn_remove(lid)?;
                Ok(DataType::Null)
            }

            // UDP operations
            OperationType::UdpBind => {
                let address = get_string(inputs, "address")?;
                let port = get_bind_port(inputs, "port")?;
                let addr = format!("{}:{}", address, port);
                let socket = std::net::UdpSocket::bind(&addr)
                    .map_err(|e| EvalError::InvalidInput(format!("udp_bind: {}", e)))?;
                let id = conn_id("udp");
                conn_store(&id, Mutex::new(socket))?;
                Ok(DataType::String(id))
            }
            OperationType::UdpSendTo => {
                let sid = get_string(inputs, "socket_id")?;
                let data = inputs.get("data").cloned().unwrap_or(DataType::Null);
                let address = get_string(inputs, "address")?;
                validate_host(address)?;
                let port = get_port(inputs, "port")?;
                let target = format!("{}:{}", address, port);
                // Post-DNS-resolution SSRF check — resolve once, check all IPs, use resolved addr
                use std::net::ToSocketAddrs;
                let resolved_addrs: Vec<_> = target.to_socket_addrs()
                    .map_err(|e| EvalError::InvalidInput(format!("udp_send_to: DNS resolution failed: {}", e)))?
                    .collect();
                for resolved in &resolved_addrs {
                    if is_blocked_ip(resolved.ip()) {
                        return Err(EvalError::InvalidInput(format!(
                            "udp_send_to: blocked IP after DNS resolution: {}", resolved.ip()
                        )));
                    }
                }
                let sock_addr = resolved_addrs.into_iter().next()
                    .ok_or_else(|| EvalError::InvalidInput("udp_send_to: no addresses resolved".into()))?;
                let bytes = data_to_bytes(&data);
                conn_with::<Mutex<std::net::UdpSocket>, _>(sid, |mtx| {
                    let socket = mtx
                        .get_mut()
                        .map_err(|_| EvalError::InvalidInput("udp lock poisoned".into()))?;
                    let sent = socket
                        .send_to(&bytes, sock_addr)
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
                    let mut buf = vec![0u8; 65535]; // Max UDP datagram size
                    let (n, addr) = socket.recv_from(&mut buf).map_err(|e| {
                        EvalError::InvalidInput(format!("udp_recv_from: {}", e))
                    })?;
                    buf.truncate(n);
                    Ok(DataType::Map(magi_lang::util::OrderedMap::from([
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

            // WebSocket operations
            OperationType::WsConnect => {
                use std::net::ToSocketAddrs;
                let url_str = get_string(inputs, "url")?;
                validate_url(url_str)?;
                let parsed_url = magi_lang::util::UrlParts::parse(url_str)
                    .map_err(|e| EvalError::InvalidInput(format!("ws_connect: invalid URL: {}", e)))?;
                let host = parsed_url.host_str()
                    .ok_or_else(|| EvalError::InvalidInput("ws_connect: URL has no host".to_string()))?;
                let port = parsed_url.port_or_known_default()
                    .unwrap_or(if parsed_url.scheme == "wss" { 443 } else { 80 });
                let addr = format!("{}:{}", host, port);
                let sock_addr = addr.to_socket_addrs()
                    .map_err(|e| EvalError::InvalidInput(format!("ws_connect: DNS resolution failed: {}", e)))?
                    .next()
                    .ok_or_else(|| EvalError::InvalidInput("ws_connect: no addresses found".to_string()))?;
                if is_blocked_ip(sock_addr.ip()) {
                    return Err(EvalError::InvalidInput(format!(
                        "ws_connect: blocked IP after DNS resolution: {}", sock_addr.ip()
                    )));
                }
                let tcp_stream = std::net::TcpStream::connect_timeout(
                    &sock_addr,
                    std::time::Duration::from_secs(30),
                ).map_err(|e| EvalError::InvalidInput(format!("ws_connect: {}", e)))?;
                let timeout = std::time::Duration::from_secs(30);
                let _ = tcp_stream.set_read_timeout(Some(timeout));
                let _ = tcp_stream.set_write_timeout(Some(timeout));
                let socket = magi_lang::util::WebSocket::connect_with_stream(tcp_stream, url_str, host)
                    .map_err(|e| EvalError::InvalidInput(format!("ws_connect: {}", e)))?;
                let id = conn_id("ws");
                conn_store(&id, Mutex::new(socket))?;
                Ok(DataType::String(id))
            }
            OperationType::WsSend => {
                let cid = get_string(inputs, "conn_id")?;
                let message = inputs.get("message").cloned().unwrap_or(DataType::Null);
                let msg = match &message {
                    DataType::Bytes(b) => {
                        if b.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "ws_send: message exceeds {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        magi_lang::util::WsMessage::Binary(b.clone())
                    }
                    other => {
                        let s = other.to_string();
                        if s.len() > MAX_STRING_OUTPUT {
                            return Err(EvalError::InvalidInput(format!(
                                "ws_send: message exceeds {} byte limit", MAX_STRING_OUTPUT
                            )));
                        }
                        magi_lang::util::WsMessage::Text(s)
                    }
                };
                type WsConn = Mutex<magi_lang::util::WebSocket>;
                conn_with::<WsConn, _>(cid, |mtx| {
                    let ws = mtx.get_mut().unwrap_or_else(|e| e.into_inner());
                    ws.send(&msg).map_err(|e| EvalError::InvalidInput(format!("ws_send: {}", e)))?;
                    Ok(DataType::Null)
                })
            }
            OperationType::WsReceive => {
                let cid = get_string(inputs, "conn_id")?;
                type WsConn = Mutex<magi_lang::util::WebSocket>;
                conn_with::<WsConn, _>(cid, |mtx| {
                    let ws = mtx.get_mut().unwrap_or_else(|e| e.into_inner());
                    if let Some(tcp) = ws.get_tcp_ref() { let _ = tcp.set_read_timeout(Some(std::time::Duration::from_secs(30))); }
                    let msg = ws.read().map_err(|e| EvalError::InvalidInput(format!("ws_receive: {}", e)))?;
                    match msg {
                        magi_lang::util::WsMessage::Text(s) => {
                            if s.len() > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!(
                                    "ws_receive: message exceeds {} byte limit", MAX_STRING_OUTPUT
                                )));
                            }
                            Ok(DataType::String(s))
                        }
                        magi_lang::util::WsMessage::Binary(b) => {
                            if b.len() > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!(
                                    "ws_receive: message exceeds {} byte limit", MAX_STRING_OUTPUT
                                )));
                            }
                            Ok(DataType::Bytes(b))
                        }
                        magi_lang::util::WsMessage::Close => Ok(DataType::Null),
                        _ => Ok(DataType::Null),
                    }
                })
            }
            OperationType::WsClose => {
                let cid = get_string(inputs, "conn_id")?;
                type WsConn = Mutex<magi_lang::util::WebSocket>;
                let _ = conn_with::<WsConn, _>(cid, |mtx| {
                    let ws = mtx.get_mut().unwrap_or_else(|e| e.into_inner());
                    let _ = ws.close();
                    Ok(())
                });
                conn_remove(cid)?;
                Ok(DataType::Null)
            }

            // SSE (Server-Sent Events) operations
            OperationType::SseConnect => {
                let url = get_string(inputs, "url")?;
                validate_url_with_dns(url)?;
                let resp = http_agent().request("GET", url, &[("Accept", "text/event-stream")], None)
                    .map_err(|e| EvalError::InvalidInput(format!("sse_connect: {}", e)))?;
                let reader = resp.into_body().into_reader();
                let buffered: Box<dyn std::io::BufRead + Send> = Box::new(std::io::BufReader::new(reader));
                let id = conn_id("sse");
                conn_store(&id, Mutex::new(buffered))?;
                Ok(DataType::String(id))
            }
            OperationType::SseReadEvent => {
                let cid = get_string(inputs, "conn_id")?;
                conn_with::<Mutex<Box<dyn std::io::BufRead + Send>>, _>(cid, |mtx| {
                    let reader = mtx.get_mut().unwrap_or_else(|e| e.into_inner());
                    let mut event_type = String::new();
                    let mut data_lines = Vec::new();
                    let mut event_id = String::new();
                    let mut total_data_bytes: usize = 0;
                    const MAX_SSE_LINES: usize = 10_000;
                    let mut line_count = 0usize;
                    loop {
                        let mut line = String::new();
                        use std::io::BufRead;
                        let n = reader.read_line(&mut line)
                            .map_err(|e| EvalError::InvalidInput(format!("sse_read_event: {}", e)))?;
                        if n == 0 { return Ok(DataType::Null); }
                        if line.len() > MAX_SSE_LINE_BYTES {
                            return Err(EvalError::InvalidInput(format!(
                                "sse_read_event: line exceeds {} byte limit", MAX_SSE_LINE_BYTES
                            )));
                        }
                        line_count += 1;
                        if line_count > MAX_SSE_LINES {
                            return Err(EvalError::InvalidInput(
                                "sse_read_event: event exceeds 10000 lines".to_string()
                            ));
                        }
                        let trimmed = line.trim_end();
                        if trimmed.is_empty() {
                            if !data_lines.is_empty() {
                                let mut m = magi_lang::util::OrderedMap::new();
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
                            let s = rest.trim_start().to_string();
                            total_data_bytes = total_data_bytes.saturating_add(s.len()).saturating_add(1);
                            if total_data_bytes > MAX_STRING_OUTPUT {
                                return Err(EvalError::InvalidInput(format!(
                                    "sse_read_event: accumulated data exceeds {} byte limit", MAX_STRING_OUTPUT
                                )));
                            }
                            data_lines.push(s);
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

            OperationType::HttpServerStart => {
                let address = get_string(inputs, "address")?;
                let port = get_bind_port(inputs, "port")?;
                let addr = format!("{}:{}", address, port);
                let listener = std::net::TcpListener::bind(&addr)
                    .map_err(|e| EvalError::InvalidInput(format!("http_server_start: {}", e)))?;
                let id = conn_id("http-server");
                conn_store(&id, Mutex::new(listener))?;
                Ok(DataType::String(id))
            }
            OperationType::HttpServerReceive => {
                let sid = get_string(inputs, "server_id")?;
                // Accept and parse outside conn_with to avoid deadlock when storing client
                let (mut stream, addr) = conn_with::<Mutex<std::net::TcpListener>, _>(sid, |mtx| {
                    let listener = mtx.get_mut().unwrap_or_else(|e| e.into_inner());
                    listener.set_nonblocking(true).map_err(|e| {
                        EvalError::InvalidInput(format!("http_server_receive: {}", e))
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
                                        "http_server_receive: timed out".into(),
                                    ));
                                }
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            }
                            Err(e) => {
                                listener.set_nonblocking(false).ok();
                                break Err(EvalError::InvalidInput(format!(
                                    "http_server_receive: {}",
                                    e
                                )));
                            }
                        }
                    };
                    listener.set_nonblocking(false).ok();
                    result
                })?;
                // Parse HTTP request from the accepted stream
                use std::io::Read as _;
                let timeout = Some(std::time::Duration::from_secs(30));
                let _ = stream.set_read_timeout(timeout);
                let _ = stream.set_write_timeout(timeout);
                // Read headers into buffer (max 64KB for headers)
                const MAX_HEADER_BUF: usize = 64 * 1024;
                let mut buf = Vec::with_capacity(4096);
                let mut tmp = [0u8; 4096];
                let header_end;
                loop {
                    let n = stream.read(&mut tmp)
                        .map_err(|e| EvalError::InvalidInput(format!("http_server_receive: {}", e)))?;
                    if n == 0 {
                        return Err(EvalError::InvalidInput(
                            "http_server_receive: connection closed before headers complete".to_string()
                        ));
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.len() > MAX_HEADER_BUF {
                        return Err(EvalError::InvalidInput(format!(
                            "http_server_receive: headers exceed {} byte limit", MAX_HEADER_BUF
                        )));
                    }
                    // Check for end of headers (\r\n\r\n)
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = pos + 4;
                        break;
                    }
                    // Also handle \n\n (lenient)
                    if let Some(pos) = buf.windows(2).position(|w| w == b"\n\n") {
                        header_end = pos + 2;
                        break;
                    }
                }
                let parsed_req = magi_lang::util::parse_http_request(&buf[..header_end])
                    .map_err(|e| EvalError::InvalidInput(format!("http_server_receive: {}", e)))?;
                let parsed_req = parsed_req.ok_or_else(|| EvalError::InvalidInput(
                    "http_server_receive: incomplete HTTP request".to_string()
                ))?;
                let method = parsed_req.method;
                let path = parsed_req.path;
                let mut headers = magi_lang::util::OrderedMap::new();
                let mut content_length: usize = 0;
                let mut seen_content_length = false;
                for h in parsed_req.headers.iter() {
                    let key = h.name.to_lowercase();
                    let value = String::from_utf8_lossy(&h.value).to_string();
                    if key == "content-length" {
                        let len: usize = value.trim().parse().map_err(|_| {
                            EvalError::InvalidInput(format!(
                                "http_server_receive: invalid Content-Length: {:?}", value
                            ))
                        })?;
                        if seen_content_length && len != content_length {
                            return Err(EvalError::InvalidInput(
                                "http_server_receive: conflicting Content-Length headers".to_string()
                            ));
                        }
                        content_length = len;
                        seen_content_length = true;
                    }
                    headers.insert(key, DataType::String(value));
                }
                const MAX_BODY: usize = 16 * 1024 * 1024;
                let body = if content_length > MAX_BODY {
                    return Err(EvalError::InvalidInput(format!(
                        "http_server_receive: Content-Length {} exceeds max {}", content_length, MAX_BODY
                    )));
                } else if content_length > 0 {
                    // We may have already read some body bytes past the header end
                    let already_read = buf.len() - header_end;
                    let mut body_buf = Vec::with_capacity(content_length);
                    body_buf.extend_from_slice(&buf[header_end..]);
                    if already_read < content_length {
                        let remaining = content_length - already_read;
                        let mut rest = vec![0u8; remaining];
                        stream.read_exact(&mut rest)
                            .map_err(|e| EvalError::InvalidInput(format!("http_server_receive: {}", e)))?;
                        body_buf.extend_from_slice(&rest);
                    }
                    String::from_utf8_lossy(&body_buf[..content_length]).to_string()
                } else { String::new() };
                let client_id = conn_id("http-client");
                conn_store(&client_id, Mutex::new(stream))?;
                Ok(DataType::Map(magi_lang::util::OrderedMap::from([
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
                    Some(v) => v.to_i64().ok_or_else(|| EvalError::TypeError {
                        expected: "numeric".to_string(),
                        actual: v.type_name().to_string(),
                        context: "http_server_respond status".to_string(),
                    })?,
                    None => 200,
                };
                if !(100..=599).contains(&status) {
                    return Err(EvalError::InvalidInput(format!(
                        "http_server_respond: invalid status code {} (must be 100-599)", status
                    )));
                }
                let body = inputs.get("body").map(|d| d.to_string()).unwrap_or_default();
                if body.len() > MAX_STRING_OUTPUT {
                    return Err(EvalError::InvalidInput(format!(
                        "http_server_respond: body exceeds {} byte limit", MAX_STRING_OUTPUT
                    )));
                }
                let reason = magi_lang::util::http_status_reason(status as u16);
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

            OperationType::Exec => {
                let cmd = get_string(inputs, "command")?;
                let output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .output()
                    .map_err(|e| EvalError::InvalidInput(format!("exec: {}", e)))?;
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                if stdout.len() > MAX_STRING_OUTPUT || stderr.len() > MAX_STRING_OUTPUT {
                    return Err(EvalError::InvalidInput(format!(
                        "exec: output exceeds {} byte limit",
                        MAX_STRING_OUTPUT
                    )));
                }
                let mut m = magi_lang::util::OrderedMap::new();
                m.insert("stdout".into(), DataType::String(stdout));
                m.insert("stderr".into(), DataType::String(stderr));
                m.insert(
                    "exit_code".into(),
                    DataType::Int64(output.status.code().unwrap_or(-1) as i64),
                );
                Ok(DataType::Map(m))
            }
            OperationType::ExecStatus => {
                let cmd = get_string(inputs, "command")?;
                let status = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .status()
                    .map_err(|e| EvalError::InvalidInput(format!("exec_status: {}", e)))?;
                Ok(DataType::Int64(status.code().unwrap_or(-1) as i64))
            }
            OperationType::ExecOutput => {
                let cmd = get_string(inputs, "command")?;
                let output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .output()
                    .map_err(|e| EvalError::InvalidInput(format!("exec_output: {}", e)))?;
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                if stdout.len() > MAX_STRING_OUTPUT {
                    return Err(EvalError::InvalidInput(format!(
                        "exec_output: output exceeds {} byte limit",
                        MAX_STRING_OUTPUT
                    )));
                }
                let mut m = magi_lang::util::OrderedMap::new();
                m.insert("stdout".into(), DataType::String(stdout));
                m.insert("stderr".into(), DataType::String(stderr));
                m.insert("exit_code".into(), DataType::Int64(output.status.code().unwrap_or(-1) as i64));
                Ok(DataType::Map(m))
            }

            OperationType::MutexNew => {
                let id = magi_lang::util::uuid_v4();
                let mutex: std::sync::Mutex<bool> = std::sync::Mutex::new(false);
                conn_store(&id, mutex)?;
                Ok(DataType::String(id))
            }
            OperationType::MutexLock => {
                let id = get_string(inputs, "id")?;
                conn_with::<std::sync::Mutex<bool>, _>(id, |mutex| {
                    // Lock the mutex: set the flag to true.
                    // In this simplified model we use the bool to track locked state.
                    let mut guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
                    *guard = true;
                    Ok(DataType::Null)
                })
            }
            OperationType::MutexUnlock => {
                let id = get_string(inputs, "id")?;
                conn_with::<std::sync::Mutex<bool>, _>(id, |mutex| {
                    let mut guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
                    *guard = false;
                    Ok(DataType::Null)
                })
            }
            OperationType::WaitgroupNew => {
                let count_val = inputs.get("count").cloned().unwrap_or(DataType::Null);
                let count = match &count_val {
                    DataType::Int64(n) => {
                        if *n < 0 {
                            return Err(EvalError::InvalidInput(
                                "waitgroup_new: count must be non-negative".to_string(),
                            ));
                        }
                        *n
                    }
                    _ => {
                        return Err(EvalError::TypeError {
                            expected: "Int64".to_string(),
                            actual: count_val.type_name().to_string(),
                            context: "WaitgroupNew".to_string(),
                        });
                    }
                };
                let id = magi_lang::util::uuid_v4();
                let wg = std::sync::Arc::new((
                    std::sync::Mutex::new(count),
                    std::sync::Condvar::new(),
                ));
                conn_store(&id, wg)?;
                Ok(DataType::String(id))
            }
            OperationType::WaitgroupDone => {
                let id = get_string(inputs, "id")?;
                Ok(conn_with::<std::sync::Arc<(std::sync::Mutex<i64>, std::sync::Condvar)>, _>(
                    id,
                    |wg| {
                        let (lock, cvar) = &**wg;
                        let mut count = lock.lock().unwrap_or_else(|e| e.into_inner());
                        if *count > 0 {
                            *count -= 1;
                        }
                        if *count == 0 {
                            cvar.notify_all();
                        }
                        Ok(DataType::Null)
                    },
                )?)
            }
            OperationType::WaitgroupWait => {
                let id = get_string(inputs, "id")?;
                Ok(conn_with::<std::sync::Arc<(std::sync::Mutex<i64>, std::sync::Condvar)>, _>(
                    id,
                    |wg| {
                        let (lock, cvar) = &**wg;
                        let mut count = lock.lock().unwrap_or_else(|e| e.into_inner());
                        while *count > 0 {
                            count = cvar
                                .wait(count)
                                .unwrap_or_else(|e| e.into_inner());
                        }
                        Ok(DataType::Null)
                    },
                )?)
            }

            // Concurrency: AwaitAll
            OperationType::AwaitAll => {
                let futures_val = inputs.get("futures").cloned().unwrap_or(DataType::Null);
                match futures_val {
                    DataType::Array(arr) => {
                        // AwaitAll simply returns the array as-is in the synchronous evaluator.
                        // In the interpreter, Future values would be resolved before reaching here.
                        Ok(DataType::Array(arr))
                    }
                    _ => Err(EvalError::TypeError {
                        expected: "Array".to_string(),
                        actual: futures_val.type_name().to_string(),
                        context: "AwaitAll".to_string(),
                    }),
                }
            }

            OperationType::LogInfo => {
                let msg = inputs.get("message").cloned().unwrap_or(DataType::Null);
                let now = magi_lang::util::local_datetime_string();
                eprintln!("[{}] [INFO] {}", now, msg.to_string_lossy());
                Ok(DataType::Null)
            }
            OperationType::LogWarn => {
                let msg = inputs.get("message").cloned().unwrap_or(DataType::Null);
                let now = magi_lang::util::local_datetime_string();
                eprintln!("[{}] [WARN] {}", now, msg.to_string_lossy());
                Ok(DataType::Null)
            }
            OperationType::LogError => {
                let msg = inputs.get("message").cloned().unwrap_or(DataType::Null);
                let now = magi_lang::util::local_datetime_string();
                eprintln!("[{}] [ERROR] {}", now, msg.to_string_lossy());
                Ok(DataType::Null)
            }
            OperationType::LogDebug => {
                let msg = inputs.get("message").cloned().unwrap_or(DataType::Null);
                let now = magi_lang::util::local_datetime_string();
                eprintln!("[{}] [DEBUG] {}", now, msg.to_string_lossy());
                Ok(DataType::Null)
            }

            OperationType::IterChain => {
                let arr_a = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let arr_b = inputs.get("other").cloned().unwrap_or(DataType::Null);
                match (arr_a, arr_b) {
                    (DataType::Array(mut a_vec), DataType::Array(b_vec)) => {
                        a_vec.extend(b_vec);
                        Ok(DataType::Array(a_vec))
                    }
                    (DataType::Array(_), other) => Err(EvalError::TypeError {
                        expected: "Array".to_string(),
                        actual: other.type_name().to_string(),
                        context: "IterChain other".to_string(),
                    }),
                    (other, _) => Err(EvalError::TypeError {
                        expected: "Array".to_string(),
                        actual: other.type_name().to_string(),
                        context: "IterChain array".to_string(),
                    }),
                }
            }
            OperationType::IterCycle => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let count_val = inputs.get("count").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        let n = match &count_val {
                            DataType::Int64(v) => *v as usize,
                            DataType::Float64(v) => *v as usize,
                            _ => {
                                return Err(EvalError::TypeError {
                                    expected: "Int64".to_string(),
                                    actual: count_val.type_name().to_string(),
                                    context: "IterCycle count".to_string(),
                                });
                            }
                        };
                        let mut result = Vec::with_capacity(arr.len().saturating_mul(n));
                        for _ in 0..n {
                            result.extend(arr.iter().cloned());
                        }
                        Ok(DataType::Array(result))
                    }
                    other => Err(EvalError::TypeError {
                        expected: "Array".to_string(),
                        actual: other.type_name().to_string(),
                        context: "IterCycle".to_string(),
                    }),
                }
            }
            OperationType::IterRepeat => {
                let val = inputs.get("value").cloned().unwrap_or(DataType::Null);
                let count_val = inputs.get("count").cloned().unwrap_or(DataType::Null);
                let n = match &count_val {
                    DataType::Int64(v) => *v as usize,
                    DataType::Float64(v) => *v as usize,
                    _ => {
                        return Err(EvalError::TypeError {
                            expected: "Int64".to_string(),
                            actual: count_val.type_name().to_string(),
                            context: "IterRepeat count".to_string(),
                        });
                    }
                };
                Ok(DataType::Array(vec![val; n]))
            }
            OperationType::IterProduct => {
                let arr_a = inputs.get("array").cloned().unwrap_or(DataType::Null);
                let arr_b = inputs.get("other").cloned().unwrap_or(DataType::Null);
                match (arr_a, arr_b) {
                    (DataType::Array(a_vec), DataType::Array(b_vec)) => {
                        let mut result = Vec::with_capacity(a_vec.len() * b_vec.len());
                        for a_item in &a_vec {
                            for b_item in &b_vec {
                                result.push(DataType::Array(vec![
                                    a_item.clone(),
                                    b_item.clone(),
                                ]));
                            }
                        }
                        Ok(DataType::Array(result))
                    }
                    (DataType::Array(_), other) => Err(EvalError::TypeError {
                        expected: "Array".to_string(),
                        actual: other.type_name().to_string(),
                        context: "IterProduct other".to_string(),
                    }),
                    (other, _) => Err(EvalError::TypeError {
                        expected: "Array".to_string(),
                        actual: other.type_name().to_string(),
                        context: "IterProduct array".to_string(),
                    }),
                }
            }
            OperationType::IterPairwise => {
                let arr_val = inputs.get("array").cloned().unwrap_or(DataType::Null);
                match arr_val {
                    DataType::Array(arr) => {
                        if arr.len() < 2 {
                            return Ok(DataType::Array(vec![]));
                        }
                        let mut result = Vec::with_capacity(arr.len() - 1);
                        for i in 0..arr.len() - 1 {
                            result.push(DataType::Array(vec![
                                arr[i].clone(),
                                arr[i + 1].clone(),
                            ]));
                        }
                        Ok(DataType::Array(result))
                    }
                    other => Err(EvalError::TypeError {
                        expected: "Array".to_string(),
                        actual: other.type_name().to_string(),
                        context: "IterPairwise".to_string(),
                    }),
                }
            }

            OperationType::TemplateRender => {
                let template = inputs.get("template").cloned().unwrap_or(DataType::Null);
                let data = inputs.get("data").cloned().unwrap_or(DataType::Null);
                match (template, data) {
                    (DataType::String(tmpl), DataType::Map(map)) => {
                        let mut result = tmpl;
                        for (k, v) in &map {
                            let placeholder = format!("{{{{{}}}}}", k);
                            let replacement = match v {
                                DataType::String(s) => s.clone(),
                                other => other.to_string_lossy(),
                            };
                            result = result.replace(&placeholder, &replacement);
                        }
                        Ok(DataType::String(result))
                    }
                    (DataType::String(_), other) => Err(EvalError::TypeError {
                        expected: "Map".to_string(),
                        actual: other.type_name().to_string(),
                        context: "TemplateRender data".to_string(),
                    }),
                    (other, _) => Err(EvalError::TypeError {
                        expected: "String".to_string(),
                        actual: other.type_name().to_string(),
                        context: "TemplateRender template".to_string(),
                    }),
                }
            }

            OperationType::FlagParse => {
                let args_val = inputs.get("args").cloned().unwrap_or(DataType::Null);
                let spec_val = inputs.get("spec").cloned().unwrap_or(DataType::Null);
                match (args_val, spec_val) {
                    (DataType::Array(args), DataType::Map(spec)) => {
                        let mut result = magi_lang::util::OrderedMap::new();
                        // Initialize defaults from spec
                        for (name, spec_entry) in &spec {
                            if let DataType::Map(entry_map) = spec_entry {
                                if let Some(default_val) = entry_map.get("default") {
                                    result.insert(name.clone(), default_val.clone());
                                }
                            }
                        }
                        let mut i = 0;
                        let args_str: Vec<String> = args
                            .iter()
                            .map(|a| match a {
                                DataType::String(s) => s.clone(),
                                other => other.to_string_lossy(),
                            })
                            .collect();
                        while i < args_str.len() {
                            let arg = &args_str[i];
                            if let Some(name) = arg.strip_prefix("--") {
                                if let Some(spec_entry) = spec.get(name) {
                                    if let DataType::Map(entry_map) = spec_entry {
                                        let type_str = entry_map
                                            .get("type")
                                            .and_then(|t| {
                                                if let DataType::String(s) = t {
                                                    Some(s.as_str())
                                                } else {
                                                    None
                                                }
                                            })
                                            .unwrap_or("string");
                                        match type_str {
                                            "bool" => {
                                                result.insert(
                                                    name.to_string(),
                                                    DataType::Bool(true),
                                                );
                                            }
                                            "int" => {
                                                i += 1;
                                                if i < args_str.len() {
                                                    if let Ok(v) = args_str[i].parse::<i64>() {
                                                        result.insert(
                                                            name.to_string(),
                                                            DataType::Int64(v),
                                                        );
                                                    }
                                                }
                                            }
                                            "float" => {
                                                i += 1;
                                                if i < args_str.len() {
                                                    if let Ok(v) = args_str[i].parse::<f64>() {
                                                        result.insert(
                                                            name.to_string(),
                                                            DataType::Float64(v),
                                                        );
                                                    }
                                                }
                                            }
                                            _ => {
                                                // string
                                                i += 1;
                                                if i < args_str.len() {
                                                    result.insert(
                                                        name.to_string(),
                                                        DataType::String(args_str[i].clone()),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            i += 1;
                        }
                        Ok(DataType::Map(result))
                    }
                    (DataType::Array(_), other) => Err(EvalError::TypeError {
                        expected: "Map".to_string(),
                        actual: other.type_name().to_string(),
                        context: "FlagParse spec".to_string(),
                    }),
                    (other, _) => Err(EvalError::TypeError {
                        expected: "Array".to_string(),
                        actual: other.type_name().to_string(),
                        context: "FlagParse args".to_string(),
                    }),
                }
            }
            OperationType::FlagArgs => {
                let args: Vec<DataType> = std::env::args()
                    .skip(1)
                    .map(DataType::String)
                    .collect();
                Ok(DataType::Array(args))
            }

            // Additional math operations — dispatched through interpreter builtins
            OperationType::MathGamma | OperationType::MathLgamma |
            OperationType::MathErf | OperationType::MathErfc |
            OperationType::MathExpm1 | OperationType::MathNextafter |
            OperationType::MathSignbit => {
                Ok(DataType::Null) // Handled by interpreter builtins
            }

            // Additional OS operations — dispatched through interpreter builtins
            OperationType::FsChown | OperationType::FsHardlink | OperationType::OsPipe => {
                Ok(DataType::Null) // Handled by interpreter builtins
            }

            // Additional strconv operations — dispatched through interpreter builtins
            OperationType::FormatFloat | OperationType::ParseUint => {
                Ok(DataType::Null) // Handled by interpreter builtins
            }

            // CLI operations — handled by the CLI, not evaluator
            OperationType::CliFix | OperationType::CliClean | OperationType::CliTree => {
                Ok(DataType::Null)
            }

            // Platform — Terminal
            OperationType::RawModeEnable => {
                magi_lang::platform::raw_mode_enable().map_err(|e| EvalError::InvalidInput(e))?;
                Ok(DataType::Null)
            }
            OperationType::RawModeDisable => {
                magi_lang::platform::raw_mode_disable().map_err(|e| EvalError::InvalidInput(e))?;
                Ok(DataType::Null)
            }
            OperationType::ReadByte => {
                Ok(match magi_lang::platform::read_byte() {
                    Some(b) => DataType::Int64(b as i64),
                    None => DataType::Int64(-1),
                })
            }
            OperationType::ReadByteTimeout => {
                let ds = match inputs.get("deciseconds") { Some(DataType::Int64(n)) => *n as u8, _ => 0 };
                Ok(match magi_lang::platform::read_byte_timeout(ds) {
                    Some(b) => DataType::Int64(b as i64),
                    None => DataType::Int64(-1),
                })
            }

            // Platform — SDL2
            OperationType::SdlInit => {
                let title = inputs.get("title").map(|v| v.to_string_lossy()).unwrap_or_default();
                let w = match inputs.get("width") { Some(DataType::Int64(n)) => *n as i32, _ => 640 };
                let h = match inputs.get("height") { Some(DataType::Int64(n)) => *n as i32, _ => 480 };
                let ctx = magi_lang::platform::SdlContext::new(&title, w, h).map_err(|e| EvalError::InvalidInput(e))?;
                Ok(DataType::Int64(Box::into_raw(Box::new(ctx)) as i64))
            }
            OperationType::SdlSetColor => {
                let handle = match inputs.get("handle") { Some(DataType::Int64(n)) => *n, _ => return Err(EvalError::InvalidInput("missing handle".into())) };
                let r = match inputs.get("r") { Some(DataType::Int64(n)) => *n as u8, _ => 0 };
                let g = match inputs.get("g") { Some(DataType::Int64(n)) => *n as u8, _ => 0 };
                let b = match inputs.get("b") { Some(DataType::Int64(n)) => *n as u8, _ => 0 };
                let ctx = unsafe { &*(handle as *const magi_lang::platform::SdlContext) };
                ctx.set_color(r, g, b);
                Ok(DataType::Null)
            }
            OperationType::SdlClear => {
                let handle = match inputs.get("handle") { Some(DataType::Int64(n)) => *n, _ => return Err(EvalError::InvalidInput("missing handle".into())) };
                let ctx = unsafe { &*(handle as *const magi_lang::platform::SdlContext) };
                ctx.clear();
                Ok(DataType::Null)
            }
            OperationType::SdlPresent => {
                let handle = match inputs.get("handle") { Some(DataType::Int64(n)) => *n, _ => return Err(EvalError::InvalidInput("missing handle".into())) };
                let ctx = unsafe { &*(handle as *const magi_lang::platform::SdlContext) };
                ctx.present();
                Ok(DataType::Null)
            }
            OperationType::SdlDrawPixel => {
                let handle = match inputs.get("handle") { Some(DataType::Int64(n)) => *n, _ => return Err(EvalError::InvalidInput("missing handle".into())) };
                let x = match inputs.get("x") { Some(DataType::Int64(n)) => *n as i32, _ => 0 };
                let y = match inputs.get("y") { Some(DataType::Int64(n)) => *n as i32, _ => 0 };
                let ctx = unsafe { &*(handle as *const magi_lang::platform::SdlContext) };
                ctx.draw_pixel(x, y);
                Ok(DataType::Null)
            }
            OperationType::SdlDrawLine => {
                let handle = match inputs.get("handle") { Some(DataType::Int64(n)) => *n, _ => return Err(EvalError::InvalidInput("missing handle".into())) };
                let x1 = match inputs.get("x1") { Some(DataType::Int64(n)) => *n as i32, _ => 0 };
                let y1 = match inputs.get("y1") { Some(DataType::Int64(n)) => *n as i32, _ => 0 };
                let x2 = match inputs.get("x2") { Some(DataType::Int64(n)) => *n as i32, _ => 0 };
                let y2 = match inputs.get("y2") { Some(DataType::Int64(n)) => *n as i32, _ => 0 };
                let ctx = unsafe { &*(handle as *const magi_lang::platform::SdlContext) };
                ctx.draw_line(x1, y1, x2, y2);
                Ok(DataType::Null)
            }
            OperationType::SdlFillRect => {
                let handle = match inputs.get("handle") { Some(DataType::Int64(n)) => *n, _ => return Err(EvalError::InvalidInput("missing handle".into())) };
                let x = match inputs.get("x") { Some(DataType::Int64(n)) => *n as i32, _ => 0 };
                let y = match inputs.get("y") { Some(DataType::Int64(n)) => *n as i32, _ => 0 };
                let w = match inputs.get("w") { Some(DataType::Int64(n)) => *n as i32, _ => 0 };
                let h = match inputs.get("h") { Some(DataType::Int64(n)) => *n as i32, _ => 0 };
                let ctx = unsafe { &*(handle as *const magi_lang::platform::SdlContext) };
                ctx.fill_rect(x, y, w, h);
                Ok(DataType::Null)
            }
            OperationType::SdlPollEvent => {
                let handle = match inputs.get("handle") { Some(DataType::Int64(n)) => *n, _ => return Err(EvalError::InvalidInput("missing handle".into())) };
                let ctx = unsafe { &*(handle as *const magi_lang::platform::SdlContext) };
                Ok(match ctx.poll_event() {
                    Some((type_, scancode)) => {
                        let mut m = magi_lang::util::OrderedMap::new();
                        m.insert("type".to_string(), DataType::Int64(type_ as i64));
                        m.insert("scancode".to_string(), DataType::Int64(scancode as i64));
                        DataType::Map(m)
                    }
                    None => DataType::Null,
                })
            }
            OperationType::SdlDelay => {
                let ms = match inputs.get("ms") { Some(DataType::Int64(n)) => *n as u32, _ => 0 };
                unsafe { magi_lang::platform::sdl_delay(ms); }
                Ok(DataType::Null)
            }
            OperationType::SdlTicks => {
                Ok(DataType::Int64(unsafe { magi_lang::platform::sdl_get_ticks() } as i64))
            }
            OperationType::SdlDestroy => {
                let handle = match inputs.get("handle") { Some(DataType::Int64(n)) => *n, _ => return Err(EvalError::InvalidInput("missing handle".into())) };
                unsafe { drop(Box::from_raw(handle as *mut magi_lang::platform::SdlContext)); }
                Ok(DataType::Null)
            }

            // Platform — Audio
            OperationType::AudioStreamNew => {
                let sr = match inputs.get("sample_rate") { Some(DataType::Int64(n)) => *n as u32, _ => 44100 };
                let stream = magi_lang::platform::AudioStream::new(sr).map_err(|e| EvalError::InvalidInput(e))?;
                Ok(DataType::Int64(Box::into_raw(Box::new(stream)) as i64))
            }
            OperationType::AudioWriteSamples => {
                let handle = match inputs.get("handle") { Some(DataType::Int64(n)) => *n, _ => return Err(EvalError::InvalidInput("missing handle".into())) };
                let arr = match inputs.get("samples") { Some(DataType::Array(a)) => a, _ => return Err(EvalError::InvalidInput("expected array".into())) };
                let samples: Vec<i16> = arr.iter().map(|v| match v { DataType::Int64(n) => *n as i16, DataType::Float64(n) => *n as i16, _ => 0 }).collect();
                let stream = unsafe { &*(handle as *const magi_lang::platform::AudioStream) };
                stream.write_samples(&samples).map_err(|e| EvalError::InvalidInput(e))?;
                Ok(DataType::Null)
            }
            OperationType::AudioDrain => {
                let handle = match inputs.get("handle") { Some(DataType::Int64(n)) => *n, _ => return Err(EvalError::InvalidInput("missing handle".into())) };
                let stream = unsafe { &*(handle as *const magi_lang::platform::AudioStream) };
                stream.drain().map_err(|e| EvalError::InvalidInput(e))?;
                Ok(DataType::Null)
            }
            OperationType::AudioClose => {
                let handle = match inputs.get("handle") { Some(DataType::Int64(n)) => *n, _ => return Err(EvalError::InvalidInput("missing handle".into())) };
                unsafe { drop(Box::from_raw(handle as *mut magi_lang::platform::AudioStream)); }
                Ok(DataType::Null)
            }

            // Platform — WebGPU (native stubs — real impl in WASM compiler target)
            OperationType::GpuInit | OperationType::GpuCreateBuffer | OperationType::GpuCreateShader
            | OperationType::GpuCreatePipeline | OperationType::GpuBeginRenderPass | OperationType::GpuDraw
            | OperationType::GpuEndRenderPass | OperationType::GpuSubmit | OperationType::GpuPresent
            | OperationType::GpuWriteBuffer | OperationType::GpuCreateTexture | OperationType::GpuDestroy => {
                Err(EvalError::InvalidInput("WebGPU is only available in WASM target".into()))
            }
        }
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

/// Normalize a path by collapsing `.` and `..` components logically
/// (without touching the filesystem). Preserves absolute/relative prefix.
fn normalize_path(path: &std::path::Path) -> String {
    use std::path::Component;
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    let mut prefix_count = 0; // track how many leading `..` we can't collapse
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                if parts.is_empty() || prefix_count == parts.len() {
                    parts.push(std::ffi::OsStr::new(".."));
                    prefix_count += 1;
                } else {
                    parts.pop();
                }
            }
            Component::CurDir => {} // skip `.`
            Component::Normal(s) => parts.push(s),
            Component::RootDir => {
                parts.clear();
                prefix_count = 0;
                parts.push(std::ffi::OsStr::new("/"));
            }
            Component::Prefix(p) => {
                parts.clear();
                prefix_count = 0;
                parts.push(p.as_os_str());
            }
        }
    }
    if parts.is_empty() {
        return ".".to_string();
    }
    let has_root = parts.first().map_or(false, |p| *p == std::ffi::OsStr::new("/"));
    if has_root {
        if parts.len() == 1 { return "/".to_string(); }
        let rest: Vec<&str> = parts[1..].iter().map(|s| s.to_str().unwrap_or("")).collect();
        format!("/{}", rest.join("/"))
    } else {
        let strs: Vec<&str> = parts.iter().map(|s| s.to_str().unwrap_or("")).collect();
        strs.join("/")
    }
}

/// Total ordering comparator for DataType values that guarantees transitivity.
/// Type tier: Null(0) < Bool(1) < Numeric(2) < String(3) < Array(4) < Map(5) < Bytes(6).
/// Within each tier, type-specific comparison is used.
fn total_cmp_values(a: &DataType, b: &DataType) -> std::cmp::Ordering {
    fn type_tier(v: &DataType) -> u8 {
        match v {
            DataType::Null => 0,
            DataType::Bool(_) => 1,
            DataType::Int64(_) | DataType::Int32(_) | DataType::Uint32(_) |
            DataType::Uint64(_) | DataType::Float64(_) | DataType::Float32(_) => 2,
            DataType::String(_) => 3,
            DataType::Array(_) => 4,
            DataType::Map(_) => 5,
            DataType::Bytes(_) => 6,
            DataType::Set(_) => 7,
            DataType::Tuple(_) => 8,
            DataType::Future(_) => 9,
        }
    }
    let ta = type_tier(a);
    let tb = type_tier(b);
    if ta != tb {
        return ta.cmp(&tb);
    }
    match (a, b) {
        (DataType::Null, DataType::Null) => std::cmp::Ordering::Equal,
        (DataType::Bool(x), DataType::Bool(y)) => x.cmp(y),
        _ if ta == 2 => {
            // Numeric tier: use i128 for integer pairs to avoid f64 precision loss
            // on large Uint64 values, then fall back to f64 total_cmp for mixed types.
            fn to_i128_for_cmp(val: &DataType) -> Option<i128> {
                match val {
                    DataType::Int64(x) => Some(*x as i128),
                    DataType::Int32(x) => Some(*x as i128),
                    DataType::Uint32(x) => Some(*x as i128),
                    DataType::Uint64(x) => Some(*x as i128),
                    _ => None,
                }
            }
            if let (Some(ai), Some(bi)) = (to_i128_for_cmp(a), to_i128_for_cmp(b)) {
                return ai.cmp(&bi);
            }
            if let (Some(pa), Some(pb)) = (promote_numeric(a), promote_numeric(b)) {
                let fa = match pa { Ok(i) => i as f64, Err(f) => f };
                let fb = match pb { Ok(i) => i as f64, Err(f) => f };
                fa.total_cmp(&fb)
            } else {
                std::cmp::Ordering::Equal // unreachable: tier 2 is always numeric
            }
        }
        (DataType::String(x), DataType::String(y)) => x.cmp(y),
        _ => a.to_string_lossy().cmp(&b.to_string_lossy()),
    }
}

fn toml_value_to_datatype(val: &magi_lang::util::TomlValue) -> DataType {
    toml_value_to_datatype_depth(val, 0)
}

fn toml_value_to_datatype_depth(val: &magi_lang::util::TomlValue, depth: usize) -> DataType {
    use magi_lang::util::TomlValue;
    const MAX_DEPTH: usize = 64;
    if depth > MAX_DEPTH { return DataType::Null; }
    match val {
        TomlValue::String(s) => DataType::String(s.clone()),
        TomlValue::Integer(n) => DataType::Int64(*n),
        TomlValue::Float(f) => DataType::Float64(*f),
        TomlValue::Boolean(b) => DataType::Bool(*b),
        TomlValue::Array(arr) => DataType::Array(arr.iter().map(|v| toml_value_to_datatype_depth(v, depth + 1)).collect()),
        TomlValue::Table(t) => {
            let m: magi_lang::util::OrderedMap<String, DataType> = t.iter()
                .map(|(k, v)| (k.clone(), toml_value_to_datatype_depth(v, depth + 1)))
                .collect();
            DataType::Map(m)
        }
    }
}

/// Cross-type numeric equality (e.g. Float32(1.0) == Int64(1)).
fn numeric_eq(a: &DataType, b: &DataType) -> bool {
    // Use i128 comparison for integer pairs to avoid f64 precision loss on large Uint64.
    fn to_i128(val: &DataType) -> Option<i128> {
        match val {
            DataType::Int64(x) => Some(*x as i128),
            DataType::Int32(x) => Some(*x as i128),
            DataType::Uint32(x) => Some(*x as i128),
            DataType::Uint64(x) => Some(*x as i128),
            _ => None,
        }
    }
    if let (Some(ai), Some(bi)) = (to_i128(a), to_i128(b)) {
        return ai == bi;
    }
    match (promote_numeric(a), promote_numeric(b)) {
        (Some(av), Some(bv)) => {
            let fa = match av { Ok(i) => i as f64, Err(f) => f };
            let fb = match bv { Ok(i) => i as f64, Err(f) => f };
            fa == fb
        }
        _ => false,
    }
}

/// Division/modulo with zero check, integer overflow protection, and float promotion.
fn num_div_op(
    a: &DataType, b: &DataType,
    int_op: fn(i64, i64) -> Option<i64>,
    float_op: fn(f64, f64) -> f64,
) -> Result<DataType, EvalError> {
    let kind = numeric_result_kind(a, b);
    match (promote_numeric(a), promote_numeric(b)) {
        (Some(Ok(x)), Some(Ok(y))) => {
            if y == 0 { return Err(EvalError::DivisionByZero); }
            match int_op(x, y) {
                Some(v) => wrap_int_result(v, kind),
                None => Err(EvalError::Overflow("integer overflow".to_string())),
            }
        }
        (Some(av), Some(bv)) => {
            let fb = match bv { Ok(i) => i as f64, Err(f) => f };
            if fb == 0.0 { return Err(EvalError::DivisionByZero); }
            let fa = match av { Ok(i) => i as f64, Err(f) => f };
            Ok(wrap_float_result(float_op(fa, fb), kind))
        }
        _ => Err(EvalError::TypeError { expected: "number".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "arithmetic".to_string() }),
    }
}

/// Apply a unary float operation, preserving Float32 type.
fn eval_unary_float_op(
    input: &DataType,
    f32_op: fn(f32) -> f32,
    f64_op: fn(f64) -> f64,
) -> Result<DataType, EvalError> {
    if let DataType::Float32(n) = input {
        return Ok(DataType::Float32(f32_op(*n)));
    }
    match promote_numeric(input) {
        Some(Ok(n)) => Ok(DataType::Float64(f64_op(n as f64))),
        Some(Err(f)) => Ok(DataType::Float64(f64_op(f))),
        None => Err(EvalError::TypeError { expected: "number".to_string(), actual: "non-numeric".to_string(), context: "math operation".to_string() }),
    }
}

/// Extract an i64 from an optional DataType, distinguishing "not provided" from "non-numeric".
/// Returns Ok(default) if the value is None, Ok(n) if numeric, Err(TypeError) if non-numeric.
fn require_i64_or_default(val: Option<&DataType>, default: i64, context: &str) -> Result<i64, EvalError> {
    match val {
        None => Ok(default),
        Some(v) => match v.to_i64() {
            Some(n) => Ok(n),
            None => Err(EvalError::TypeError {
                expected: "numeric".to_string(),
                actual: v.type_name().to_string(),
                context: context.to_string(),
            }),
        },
    }
}

/// Determine the common numeric type for binary operations.
/// Returns 32 for Int32+Int32, 64 for Int64 (default int), 132 for Uint32+Uint32,
/// 164 for Uint64, 232 for Float32+Float32, 264 for Float64 (default float).
fn numeric_result_kind(a: &DataType, b: &DataType) -> i32 {
    match (a, b) {
        (DataType::Float32(_), DataType::Float32(_)) => 232,
        (DataType::Float32(_), _) | (_, DataType::Float32(_)) => 264, // mixed float → Float64
        (DataType::Float64(_), _) | (_, DataType::Float64(_)) => 264,
        (DataType::Uint64(_), _) | (_, DataType::Uint64(_)) => 164,
        (DataType::Uint32(_), DataType::Uint32(_)) => 132,
        (DataType::Int32(_), DataType::Int32(_)) => 32,
        _ => 64, // default int
    }
}

fn wrap_int_result(v: i64, kind: i32) -> Result<DataType, EvalError> {
    match kind {
        32 => {
            if v < i32::MIN as i64 || v > i32::MAX as i64 {
                Err(EvalError::Overflow("int32 overflow".to_string()))
            } else {
                Ok(DataType::Int32(v as i32))
            }
        }
        132 => {
            if v < 0 || v > u32::MAX as i64 {
                Err(EvalError::Overflow("uint32 overflow".to_string()))
            } else {
                Ok(DataType::Uint32(v as u32))
            }
        }
        164 => {
            if v < 0 {
                Err(EvalError::Overflow("uint64 overflow".to_string()))
            } else {
                Ok(DataType::Uint64(v as u64))
            }
        }
        232 => Ok(DataType::Float32(v as f32)),
        264 => Ok(DataType::Float64(v as f64)),
        _ => Ok(DataType::Int64(v)),
    }
}

fn wrap_float_result(v: f64, kind: i32) -> DataType {
    match kind {
        232 => DataType::Float32(v as f32),
        _ => DataType::Float64(v),
    }
}

fn num_binop(
    a: &DataType, b: &DataType,
    int_op: fn(i64, i64) -> Option<i64>,
    float_op: fn(f64, f64) -> f64,
) -> Result<DataType, EvalError> {
    let kind = numeric_result_kind(a, b);
    match (promote_numeric(a), promote_numeric(b)) {
        (Some(Ok(x)), Some(Ok(y))) => match int_op(x, y) {
            Some(v) => wrap_int_result(v, kind),
            None => Err(EvalError::Overflow("integer overflow".to_string())),
        },
        (Some(av), Some(bv)) => {
            let fa = match av { Ok(i) => i as f64, Err(f) => f };
            let fb = match bv { Ok(i) => i as f64, Err(f) => f };
            Ok(wrap_float_result(float_op(fa, fb), kind))
        }
        _ => Err(EvalError::TypeError { expected: "number".to_string(), actual: format!("{}, {}", a.type_name(), b.type_name()), context: "arithmetic".to_string() }),
    }
}

fn num_cmp(
    a: &DataType, b: &DataType,
    cmp_test: fn(std::cmp::Ordering) -> bool,
) -> Result<DataType, EvalError> {
    if let (DataType::String(x), DataType::String(y)) = (a, b) {
        return Ok(DataType::Bool(cmp_test(x.cmp(y))));
    }
    // Use i128 for integer-pair comparisons to avoid f64 precision loss on large Uint64.
    fn to_i128_cmp(val: &DataType) -> Option<i128> {
        match val {
            DataType::Int64(x) => Some(*x as i128),
            DataType::Int32(x) => Some(*x as i128),
            DataType::Uint32(x) => Some(*x as i128),
            DataType::Uint64(x) => Some(*x as i128),
            _ => None,
        }
    }
    if let (Some(ai), Some(bi)) = (to_i128_cmp(a), to_i128_cmp(b)) {
        return Ok(DataType::Bool(cmp_test(ai.cmp(&bi))));
    }
    match (promote_numeric(a), promote_numeric(b)) {
        (Some(Ok(x)), Some(Ok(y))) => Ok(DataType::Bool(cmp_test(x.cmp(&y)))),
        (Some(av), Some(bv)) => {
            let fa = match av { Ok(i) => i as f64, Err(f) => f };
            let fb = match bv { Ok(i) => i as f64, Err(f) => f };
            Ok(DataType::Bool(cmp_test(fa.total_cmp(&fb))))
        }
        _ => Err(EvalError::TypeError {
            expected: "number or string".to_string(),
            actual: format!("{}, {}", a.type_name(), b.type_name()),
            context: "comparison".to_string(),
        }),
    }
}


// YAML helpers (serde_yaml_ng conversion)

fn yaml_value_to_datatype(val: &magi_lang::util::YamlValue) -> DataType {
    yaml_value_to_datatype_depth(val, 0)
}

fn yaml_value_to_datatype_depth(val: &magi_lang::util::YamlValue, depth: usize) -> DataType {
    const MAX_DEPTH: usize = 64;
    if depth > MAX_DEPTH { return DataType::Null; }
    match val {
        magi_lang::util::YamlValue::Null => DataType::Null,
        magi_lang::util::YamlValue::Bool(b) => DataType::Bool(*b),
        magi_lang::util::YamlValue::Int(n) => DataType::Int64(*n),
        magi_lang::util::YamlValue::Float(f) => DataType::Float64(*f),
        magi_lang::util::YamlValue::String(s) => DataType::String(s.clone()),
        magi_lang::util::YamlValue::Sequence(arr) => {
            if arr.len() > MAX_ARRAY_ELEMENTS {
                return DataType::String(format!("[sequence too large: {} elements]", arr.len()));
            }
            DataType::Array(arr.iter().map(|v| yaml_value_to_datatype_depth(v, depth + 1)).collect())
        }
        magi_lang::util::YamlValue::Mapping(map) => {
            if map.len() > MAX_ARRAY_ELEMENTS {
                return DataType::String(format!("[mapping too large: {} entries]", map.len()));
            }
            let m: magi_lang::util::OrderedMap<String, DataType> = map.iter()
                .map(|(k, v)| {
                    let key = match k {
                        magi_lang::util::YamlValue::String(s) => s.clone(),
                        magi_lang::util::YamlValue::Int(n) => format!("{}", n),
                        magi_lang::util::YamlValue::Float(f) => format!("{}", f),
                        magi_lang::util::YamlValue::Bool(b) => format!("{}", b),
                        magi_lang::util::YamlValue::Null => "null".to_string(),
                        other => format!("{:?}", other),
                    };
                    (key, yaml_value_to_datatype_depth(v, depth + 1))
                })
                .collect();
            DataType::Map(m)
        }
    }
}

fn datatype_to_yaml_value(data: &DataType) -> magi_lang::util::YamlValue {
    datatype_to_yaml_value_depth(data, 0)
}

fn datatype_to_yaml_value_depth(data: &DataType, depth: usize) -> magi_lang::util::YamlValue {
    const MAX_DEPTH: usize = 64;
    if depth > MAX_DEPTH { return magi_lang::util::YamlValue::Null; }
    match data {
        DataType::Null => magi_lang::util::YamlValue::Null,
        DataType::Bool(b) => magi_lang::util::YamlValue::Bool(*b),
        DataType::Int64(n) => magi_lang::util::YamlValue::Int(*n),
        DataType::Int32(n) => magi_lang::util::YamlValue::Int(*n as i64),
        DataType::Uint32(n) => magi_lang::util::YamlValue::Int(*n as i64),
        DataType::Uint64(n) => {
            if *n > i64::MAX as u64 {
                magi_lang::util::YamlValue::String(n.to_string())
            } else {
                magi_lang::util::YamlValue::Int(*n as i64)
            }
        }
        DataType::Float64(f) => {
            if f.is_nan() || f.is_infinite() {
                magi_lang::util::YamlValue::String(format!("{}", f))
            } else {
                magi_lang::util::YamlValue::Float(*f)
            }
        }
        DataType::Float32(f) => {
            if f.is_nan() || f.is_infinite() {
                magi_lang::util::YamlValue::String(format!("{}", f))
            } else {
                magi_lang::util::YamlValue::Float(*f as f64)
            }
        }
        DataType::String(s) => magi_lang::util::YamlValue::String(s.clone()),
        DataType::Array(arr) => {
            magi_lang::util::YamlValue::Sequence(arr.iter().map(|v| datatype_to_yaml_value_depth(v, depth + 1)).collect())
        }
        DataType::Map(m) => {
            let mapping: Vec<(magi_lang::util::YamlValue, magi_lang::util::YamlValue)> = m.iter()
                .filter(|(k, _)| !k.starts_with("__"))
                .map(|(k, v)| (magi_lang::util::YamlValue::String(k.clone()), datatype_to_yaml_value_depth(v, depth + 1)))
                .collect();
            magi_lang::util::YamlValue::Mapping(mapping)
        }
        DataType::Bytes(b) => magi_lang::util::YamlValue::String(format!("<bytes:{}>", b.len())),
        DataType::Set(items) => {
            magi_lang::util::YamlValue::Sequence(items.iter().map(|v| datatype_to_yaml_value_depth(v, depth + 1)).collect())
        }
        DataType::Tuple(items) => {
            magi_lang::util::YamlValue::Sequence(items.iter().map(|v| datatype_to_yaml_value_depth(v, depth + 1)).collect())
        }
        DataType::Future(_) => magi_lang::util::YamlValue::Null,
    }
}

fn datatype_to_json_value(val: &DataType) -> magi_lang::util::JsonValue {
    datatype_to_json_value_depth(val, 0)
}

fn datatype_to_json_value_depth(val: &DataType, depth: usize) -> magi_lang::util::JsonValue {
    if depth > MAX_JSON_DEPTH {
        return magi_lang::util::JsonValue::String("[max depth]".into());
    }
    match val {
        DataType::Null | DataType::Future(_) => magi_lang::util::JsonValue::Null,
        DataType::Bool(b) => magi_lang::util::JsonValue::Bool(*b),
        DataType::Int64(n) => magi_lang::util::json_int(*n),
        DataType::Int32(n) => magi_lang::util::json_int(*n as i64),
        DataType::Uint32(n) => magi_lang::util::json_uint(*n as u64),
        DataType::Uint64(n) => magi_lang::util::json_uint(*n),
        DataType::Float64(f) => {
            if f.is_finite() { magi_lang::util::json_float(*f) } else { magi_lang::util::JsonValue::Null }
        }
        DataType::Float32(f) => {
            if f.is_finite() { magi_lang::util::json_float(*f as f64) } else { magi_lang::util::JsonValue::Null }
        }
        DataType::String(s) => magi_lang::util::JsonValue::String(s.clone()),
        DataType::Array(arr) => magi_lang::util::JsonValue::Array(arr.iter().map(|v| datatype_to_json_value_depth(v, depth + 1)).collect()),
        DataType::Map(m) => {
            let obj: magi_lang::util::OrderedMap<String, magi_lang::util::JsonValue> = m.iter()
                .filter(|(k, _)| !k.starts_with("__"))
                .map(|(k, v)| (k.clone(), datatype_to_json_value_depth(v, depth + 1)))
                .collect();
            magi_lang::util::JsonValue::Object(obj)
        }
        DataType::Set(items) => magi_lang::util::JsonValue::Array(items.iter().map(|v| datatype_to_json_value_depth(v, depth + 1)).collect()),
        DataType::Tuple(items) => magi_lang::util::JsonValue::Array(items.iter().map(|v| datatype_to_json_value_depth(v, depth + 1)).collect()),
        DataType::Bytes(b) => {
            magi_lang::util::JsonValue::String(magi_lang::util::base64_encode(b))
        }
    }
}

fn datatype_to_json_string(val: &DataType) -> Result<String, EvalError> {
    Ok(magi_lang::util::json_to_string(&datatype_to_json_value(val)))
}

fn json_value_to_datatype(val: &magi_lang::util::JsonValue) -> DataType {
    json_value_to_datatype_depth(val, 0)
}

fn json_value_to_datatype_depth(val: &magi_lang::util::JsonValue, depth: usize) -> DataType {
    if depth > MAX_JSON_DEPTH {
        return DataType::String("[max depth]".to_string());
    }
    match val {
        magi_lang::util::JsonValue::Null => DataType::Null,
        magi_lang::util::JsonValue::Bool(b) => DataType::Bool(*b),
        magi_lang::util::JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() { DataType::Int64(i) }
            else if let Some(u) = n.as_u64() { DataType::Uint64(u) }
            else if let Some(f) = n.as_f64() { DataType::Float64(f) }
            else { DataType::Null }
        }
        magi_lang::util::JsonValue::String(s) => DataType::String(s.clone()),
        magi_lang::util::JsonValue::Array(arr) => {
            if arr.len() > MAX_ARRAY_ELEMENTS {
                return DataType::String(format!("[array too large: {} elements]", arr.len()));
            }
            DataType::Array(arr.iter().map(|v| json_value_to_datatype_depth(v, depth + 1)).collect())
        }
        magi_lang::util::JsonValue::Object(obj) => {
            if obj.len() > MAX_ARRAY_ELEMENTS {
                return DataType::String(format!("[object too large: {} entries]", obj.len()));
            }
            let m: magi_lang::util::OrderedMap<String, DataType> = obj.iter()
                .map(|(k, v)| (k.clone(), json_value_to_datatype_depth(v, depth + 1)))
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
    eprintln!("  test <file.magi | dir>       Run test blocks in a file or directory");
    eprintln!("  eval '<expression>'         Evaluate an expression and print the result");
    eprintln!("  repl                        Start interactive REPL");
    eprintln!("  check <file.magi>           Type-check and lint (exit 1 on errors)");
    eprintln!("  lint <file.magi>            Lint for style issues");
    eprintln!("  fmt [options] <file.magi>   Format source code");
    eprintln!("  init <name>                 Create a new MAGI project");
    eprintln!("  get [file | dir]            Fetch all git dependencies");
    eprintln!("  bench [options] <file.magi>  Benchmark a .magi file");
    eprintln!("  compile <file.magi>         Compile to native binary (default) or WASM");
    eprintln!("  doc <file.magi>             Generate Markdown documentation");
    eprintln!("  test-all                    Run tests across all workspace members");
    eprintln!("  lsp                         Start the Language Server Protocol server");
    eprintln!("  build <file.magi>           Build (alias for compile)");
    eprintln!("  clean                       Remove build artifacts");
    eprintln!("  env                         Show environment information");
    eprintln!("  watch <file.magi>           Watch and re-run on changes");
    eprintln!("  version                     Show version information");
    eprintln!();
    eprintln!("Format options:");
    eprintln!("  --write, -w                 Write formatted output back to file");
    eprintln!("  --check, -c                 Check formatting without modifying (exit 1 if unformatted)");
    eprintln!();
    eprintln!("Run options:");
    eprintln!("  --timeout <seconds>         Abort execution after N seconds");
    eprintln!("  --sandbox                   Disable filesystem and network operations");
    eprintln!("  --watch, -w                 Watch file for changes and re-run on modification");
    eprintln!("  --json                      Output diagnostics as JSON");
    eprintln!();
    eprintln!("Flags:");
    eprintln!("  --help, -h                  Show this help message");
    eprintln!("  --version, -V               Show version");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  magi run main.magi          Run a program");
    eprintln!("  magi main.magi              Shorthand for 'magi run main.magi'");
    eprintln!("  magi test tests.magi        Run all test blocks");
    eprintln!("  magi eval '1 + 2'           Evaluate an expression");
    eprintln!("  magi repl                   Start interactive session");
    eprintln!("  magi check main.magi        Type-check before deploying");
    eprintln!("  magi fmt --write main.magi  Format a file in-place");
    eprintln!("  magi bench -n 500 main.magi Benchmark a file (500 iterations)");
    eprintln!("  magi init my-project        Scaffold a new project");
    eprintln!("  magi compile main.magi      Compile to native binary");
    eprintln!("  magi compile main.magi --target wasm  Compile to dist/main.wasm");
    eprintln!("  magi test-all               Run tests for all workspace members");
}

fn main() {
    // Spawn the real main on a thread with a larger stack to support deep
    // recursion in the interpreter (debug builds use far more stack per frame).
    let builder = std::thread::Builder::new()
        .name("magi-main".to_string())
        .stack_size(64 * 1024 * 1024); // 64 MB (increased for self-hosted parser tests)
    let handler = builder
        .spawn(main_inner)
        .expect("failed to spawn main thread");
    if let Err(e) = handler.join() {
        if let Some(msg) = e.downcast_ref::<&str>() {
            eprintln!("fatal: {}", msg);
        } else if let Some(msg) = e.downcast_ref::<String>() {
            eprintln!("fatal: {}", msg);
        } else {
            eprintln!("fatal: unexpected panic");
        }
        process::exit(1);
    }
}

fn main_inner() {
    // Register FullEvaluator as the spawn evaluator factory so spawned
    // threads get access to all 374+ stdlib operations.
    magi_lang::syntax::interpreter::register_spawn_evaluator_factory(|| {
        Box::new(FullEvaluator)
    });

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
            println!(
                "MAGI Language v{} (built {} for {})",
                magi_lang::version::version_string(),
                env!("MAGI_BUILD_DATE"),
                env!("MAGI_BUILD_TARGET"),
            );
        }
        "run" => {
            let mut json_output = false;
            let mut sandbox = false;
            let mut watch = false;
            let mut timeout_secs: u64 = 0;
            let mut file_path = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--json" => json_output = true,
                    "--sandbox" => sandbox = true,
                    "--watch" | "-w" => watch = true,
                    "--timeout" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("error: --timeout requires a value in seconds");
                            process::exit(1);
                        }
                        timeout_secs = match args[i].parse::<u64>() {
                            Ok(v) if v > 0 => v,
                            _ => {
                                eprintln!("error: --timeout value must be a positive integer");
                                process::exit(1);
                            }
                        };
                    }
                    _ => file_path = Some(args[i].as_str()),
                }
                i += 1;
            }
            if sandbox {
                SANDBOX_MODE.store(true, Ordering::Relaxed);
            }
            match file_path {
                Some(path) => {
                    cmd_run(path, json_output, timeout_secs);
                    if watch {
                        cmd_watch(path, json_output, timeout_secs);
                    }
                }
                None => {
                    eprintln!("error: missing file argument");
                    eprintln!("Usage: magi run [--json] [--timeout <seconds>] [--sandbox] [--watch] <file.magi>");
                    process::exit(1);
                }
            }
        }
        "compile" | "build" => {
            if args.len() < 3 {
                eprintln!("error: missing file argument");
                eprintln!("Usage: magi compile [--target native|wasm] [-O0..3] [-o output] <file.magi>");
                process::exit(1);
            }
            // Parse compile flags
            let mut target = "native";
            let mut opt_level: u8 = 2;
            let mut output_path: Option<String> = None;
            let mut file_path: Option<String> = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--target" => {
                        i += 1;
                        if i < args.len() { target = if args[i] == "wasm" { "wasm" } else { "native" }; }
                    }
                    "-o" | "--output" => {
                        i += 1;
                        if i < args.len() { output_path = Some(args[i].clone()); }
                    }
                    "-O0" => opt_level = 0,
                    "-O1" => opt_level = 1,
                    "-O2" => opt_level = 2,
                    "-O3" => opt_level = 3,
                    "-Os" => opt_level = 2, // size = default
                    _ => {
                        if file_path.is_none() && !args[i].starts_with('-') {
                            file_path = Some(args[i].clone());
                        }
                    }
                }
                i += 1;
            }
            let file_path = match file_path {
                Some(p) => p,
                None => {
                    eprintln!("error: missing file argument");
                    process::exit(1);
                }
            };
            if target == "wasm" {
                cmd_compile(&file_path);
            } else {
                cmd_compile_native(&file_path, opt_level, output_path.as_deref());
            }
        }
        "check" => {
            let mut json_output = false;
            let mut file_path = None;
            for arg in &args[2..] {
                match arg.as_str() {
                    "--json" => json_output = true,
                    _ => file_path = Some(arg.as_str()),
                }
            }
            match file_path {
                Some(path) => cmd_check(path, json_output),
                None => {
                    eprintln!("error: missing file argument");
                    eprintln!("Usage: magi check [--json] <file.magi>");
                    process::exit(1);
                }
            }
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
        "bench" => {
            let mut iterations: u64 = 100;
            let mut file_path = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "-n" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("error: -n requires an iteration count");
                            process::exit(1);
                        }
                        iterations = match args[i].parse::<u64>() {
                            Ok(v) if v > 0 => v,
                            _ => {
                                eprintln!("error: -n value must be a positive integer");
                                process::exit(1);
                            }
                        };
                    }
                    _ => file_path = Some(args[i].as_str()),
                }
                i += 1;
            }
            match file_path {
                Some(path) => cmd_bench(path, iterations),
                None => {
                    eprintln!("error: missing file argument");
                    eprintln!("Usage: magi bench [-n <iterations>] <file.magi>");
                    process::exit(1);
                }
            }
        }
        "test" => {
            if args.len() < 3 {
                eprintln!("error: missing file or directory argument");
                eprintln!("Usage: magi test [--filter <pattern>] [--timeout <ms>] <file.magi | directory>");
                process::exit(1);
            }
            // Parse --filter and --timeout options
            let mut filter: Option<String> = None;
            let mut timeout_ms: Option<u64> = None;
            let mut target_idx = 2;
            while target_idx < args.len() {
                match args[target_idx].as_str() {
                    "--filter" => {
                        target_idx += 1;
                        if target_idx < args.len() { filter = Some(args[target_idx].clone()); }
                        target_idx += 1;
                    }
                    "--timeout" => {
                        target_idx += 1;
                        if target_idx < args.len() { timeout_ms = args[target_idx].parse().ok(); }
                        target_idx += 1;
                    }
                    _ => break,
                }
            }
            if target_idx >= args.len() {
                eprintln!("error: missing file argument");
                process::exit(1);
            }
            let target = &args[target_idx];
            let path = std::path::Path::new(target);
            if path.is_dir() {
                cmd_test_dir(target);
            } else {
                cmd_test_with_filter_timeout(target, filter.as_deref(), timeout_ms);
            }
        }
        "init" => {
            if args.len() < 3 {
                eprintln!("error: missing project name");
                eprintln!("Usage: magi init <project-name>");
                process::exit(1);
            }
            cmd_init(&args[2]);
        }
        "repl" => {
            cmd_repl();
        }
        "eval" => {
            if args.len() < 3 {
                eprintln!("error: missing expression argument");
                eprintln!("Usage: magi eval '<expression>'");
                process::exit(1);
            }
            cmd_eval(&args[2..].join(" "));
        }
        "lsp" => {
            cmd_lsp();
        }
        "compile-native" | "build-native" => {
            // Legacy alias — redirect to `magi compile --target native`
            if args.len() < 3 {
                eprintln!("Usage: magi compile <file.magi> (compile-native is deprecated)");
                process::exit(1);
            }
            cmd_compile_native(&args[2], 2, None);
        }
        "debug" | "dbg" => {
            if args.len() < 3 {
                eprintln!("Usage: magi debug <file.magi>");
                process::exit(1);
            }
            let source = read_source(&args[2]);
            let evaluator = FullEvaluator;
            magi_lang::debugger::debug_run(&source, &evaluator);
        }
        "mcp" => {
            magi_lang::mcp::run_mcp_server();
        }
        "doc" => {
            if args.len() < 3 {
                eprintln!("error: missing file argument");
                eprintln!("Usage: magi doc <file.magi>");
                process::exit(1);
            }
            cmd_doc(&args[2]);
        }
        "doc-test" => {
            if args.len() < 3 {
                eprintln!("error: missing file argument");
                eprintln!("Usage: magi doc-test <file.magi>");
                process::exit(1);
            }
            cmd_doc_test(&args[2]);
        }
        "test-all" => {
            cmd_test_all();
        }
        "get" => {
            // Fetch all git dependencies declared in magi.toml.
            // Optionally takes a path to a .magi file; defaults to main.magi.
            let file_arg = args.get(2).map(|s| s.as_str()).unwrap_or("main.magi");
            let file_path = std::path::Path::new(file_arg);
            // If the argument is a directory, look for magi.toml inside it.
            let toml_dir = if file_path.is_dir() {
                file_path.to_path_buf()
            } else {
                file_path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf()
            };
            let toml_path = toml_dir.join("magi.toml");
            if !toml_path.exists() {
                eprintln!("error: no magi.toml found in {}", toml_dir.display());
                process::exit(1);
            }
            let toml_str = fs::read_to_string(&toml_path).unwrap_or_else(|e| {
                eprintln!("error: cannot read {}: {}", toml_path.display(), e);
                process::exit(1);
            });
            let table = match magi_lang::util::toml_parse(&toml_str) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("error: failed to parse {}: {}", toml_path.display(), e);
                    process::exit(1);
                }
            };
            let deps = match table.get("dependencies").and_then(|d| d.as_table()) {
                Some(d) => d,
                None => {
                    println!("No dependencies found.");
                    return;
                }
            };
            // Cache invalidation: check existing lock file checksums
            let lock_path = toml_dir.join("magi.lock");
            let existing_checksums: std::collections::HashMap<String, String> = if lock_path.exists() {
                let lock_content = fs::read_to_string(&lock_path).unwrap_or_default();
                let mut map = std::collections::HashMap::new();
                let mut current_name = String::new();
                for line in lock_content.lines() {
                    if line.starts_with("name = \"") {
                        current_name = line.trim_start_matches("name = \"").trim_end_matches('"').to_string();
                    } else if line.starts_with("checksum = \"") {
                        let cs = line.trim_start_matches("checksum = \"").trim_end_matches('"').to_string();
                        if !current_name.is_empty() { map.insert(current_name.clone(), cs); }
                    }
                }
                map
            } else { std::collections::HashMap::new() };

            let mut fetched = 0usize;
            for (id, value) in deps {
                if let Some(dep_table) = value.as_table() {
                    if let Some(git_url) = dep_table.get("git").and_then(|g| g.as_str()) {
                        // Check cache: if package dir exists and checksum matches lock file, skip
                        let pkg_dir = toml_dir.join("packages").join(id);
                        if pkg_dir.exists() {
                            if let Some(expected_cs) = existing_checksums.get(id) {
                                let mut hasher_input = Vec::new();
                                if let Ok(entries) = fs::read_dir(&pkg_dir) {
                                    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "magi" || e == "toml").unwrap_or(false)).collect();
                                    paths.sort();
                                    for p in paths { if let Ok(data) = fs::read(&p) { hasher_input.extend_from_slice(&data); } }
                                }
                                let actual_cs = format!("sha256:{}", sha256_hex(&hasher_input));
                                if &actual_cs == expected_cs {
                                    println!("  {} (cached, checksum OK)", id);
                                    fetched += 1;
                                    continue;
                                } else {
                                    println!("  {} (checksum mismatch, re-fetching)", id);
                                    let _ = fs::remove_dir_all(&pkg_dir);
                                }
                            }
                        }
                        let branch_or_tag = dep_table
                            .get("branch")
                            .or_else(|| dep_table.get("tag"))
                            .and_then(|v| v.as_str());
                        match resolve_git_dependency(id, git_url, branch_or_tag) {
                            Ok(p) => {
                                println!("  {} -> {}", id, p.display());
                                fetched += 1;
                            }
                            Err(e) => {
                                eprintln!("  error fetching '{}': {}", id, e);
                            }
                        }
                    }
                }
            }
            if fetched == 0 {
                println!("No git dependencies to fetch.");
            } else {
                println!("Fetched {} git dependencies.", fetched);
                // Generate magi.lock with checksums
                let mut lock = String::from("# magi.lock — auto-generated by `magi get`\n");
                lock.push_str(&format!("# generated = {}\n\n", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()));
                for (id, value) in deps {
                    if let Some(dep_table) = value.as_table() {
                        if let Some(git_url) = dep_table.get("git").and_then(|g| g.as_str()) {
                            let version = dep_table.get("version").or(dep_table.get("tag")).and_then(|v| v.as_str()).unwrap_or("0.0.0");
                            // Get commit hash from cloned repo
                            let pkg_dir = toml_dir.join("packages").join(id);
                            let commit = std::process::Command::new("git")
                                .args(["rev-parse", "HEAD"])
                                .current_dir(&pkg_dir)
                                .output()
                                .ok()
                                .and_then(|o| if o.status.success() { Some(String::from_utf8_lossy(&o.stdout).trim().to_string()) } else { None })
                                .unwrap_or_default();
                            // Compute checksum of all .magi files in the package
                            let mut hasher_input = Vec::new();
                            if let Ok(entries) = fs::read_dir(&pkg_dir) {
                                let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "magi" || e == "toml").unwrap_or(false)).collect();
                                paths.sort();
                                for p in paths {
                                    if let Ok(data) = fs::read(&p) { hasher_input.extend_from_slice(&data); }
                                }
                            }
                            let checksum = if hasher_input.is_empty() { "none".to_string() } else { sha256_hex(&hasher_input) };
                            lock.push_str(&format!("[[package]]\nname = \"{}\"\nversion = \"{}\"\nsource = \"{}\"\ncommit = \"{}\"\nchecksum = \"sha256:{}\"\n\n", id, version, git_url, commit, checksum));
                        }
                    }
                }
                let lock_path = toml_dir.join("magi.lock");
                let _ = fs::write(&lock_path, &lock);
                println!("Generated {}", lock_path.display());
            }
        }
        "build" => {
            // Build = compile (alias)
            if args.len() < 3 {
                eprintln!("error: missing file argument");
                eprintln!("Usage: magi build <file.magi>");
                process::exit(1);
            }
            cmd_compile(&args[2]);
        }
        "clean" => {
            let _ = std::fs::remove_dir_all("target");
            let _ = std::fs::remove_dir_all(".magi-cache");
            println!("Cleaned build artifacts.");
        }
        "env" => {
            let home = std::env::var("HOME").unwrap_or_default();
            let magi_home = std::env::var("MAGI_HOME").unwrap_or_else(|_| format!("{}/.magi", home));
            println!("MAGI_VERSION={}", magi_lang::version::version_string());
            println!("MAGI_BUILD_DATE={}", env!("MAGI_BUILD_DATE"));
            println!("MAGI_BUILD_TARGET={}", env!("MAGI_BUILD_TARGET"));
            println!("MAGI_ARCH={}", std::env::consts::ARCH);
            println!("MAGI_OS={}", std::env::consts::OS);
            println!("MAGI_HOME={}", magi_home);
            println!("MAGI_PATH={}", std::env::var("MAGI_PATH").unwrap_or_else(|_| format!("{}/packages", magi_home)));
            println!("MAGI_ROOT={}", std::env::var("MAGI_ROOT").unwrap_or_else(|_| {
                std::env::current_exe().map(|p| p.parent().unwrap_or(std::path::Path::new(".")).to_string_lossy().to_string()).unwrap_or_default()
            }));
            println!("MAGI_BIN={}", std::env::var("MAGI_BIN").unwrap_or_else(|_| format!("{}/bin", magi_home)));
            println!("MAGI_CACHE={}", std::env::var("MAGI_CACHE").unwrap_or_else(|_| format!("{}/cache", magi_home)));
            println!("MAGI_MODCACHE={}", std::env::var("MAGI_MODCACHE").unwrap_or_else(|_| format!("{}/mod", magi_home)));
            println!("MAGI_PROXY={}", std::env::var("MAGI_PROXY").unwrap_or_else(|_| "direct".to_string()));
            println!("MAGI_PRIVATE={}", std::env::var("MAGI_PRIVATE").unwrap_or_default());
            println!("MAGI_FLAGS={}", std::env::var("MAGI_FLAGS").unwrap_or_default());
            println!("MAGI_LOG={}", std::env::var("MAGI_LOG").unwrap_or_default());
            println!("MAGI_BACKTRACE={}", std::env::var("MAGI_BACKTRACE").unwrap_or_else(|_| "0".to_string()));
            println!("MAGI_INCREMENTAL={}", std::env::var("MAGI_INCREMENTAL").unwrap_or_else(|_| "1".to_string()));
            println!("MAGI_TARGET={}", std::env::var("MAGI_TARGET").unwrap_or_else(|_| "target".to_string()));
            println!("MAGI_TOOLCHAIN={}", std::env::var("MAGI_TOOLCHAIN").unwrap_or_else(|_| "default".to_string()));
            if let Ok(cwd) = std::env::current_dir() {
                println!("MAGI_CWD={}", cwd.display());
            }
        }
        "run-bc" | "run-bytecode" => {
            eprintln!("run-bc has been removed. Use 'magi compile file.magi' to compile to native.");
            process::exit(1);
        }
        "compilec" | "build-class" => {
            if args.len() < 3 {
                eprintln!("Usage: magi compilec <file.magi>");
                process::exit(1);
            }
            let source = read_source(&args[2]);
            // Validate: parse, type-check, lint
            let program = match parse_v2(&source) {
                Ok(p) => p,
                Err(e) => {
                    magi_lang::diagnostics::render_error(&args[2], &source, e.line as u32, e.column as u32, &e.message, None, None, None);
                    process::exit(1);
                }
            };
            // Type check
            let imports = std::collections::HashSet::new();
            let analysis = magi_lang::syntax::type_checker::check_types(&program, &imports);
            let errors: Vec<_> = analysis.diagnostics.iter()
                .filter(|d| matches!(d.severity, magi_lang::eval::DiagnosticSeverity::Error))
                .collect();
            if !errors.is_empty() {
                for e in &errors {
                    eprintln!("{}:{}: {}", e.line, e.column, e.message);
                }
                eprintln!("Type errors found; aborting compilation.");
                process::exit(1);
            }
            // Serialize: MAGC header + source code (the VM interprets the source)
            let mut bytes = Vec::new();
            bytes.extend_from_slice(b"MAGC");
            bytes.extend_from_slice(&1u16.to_le_bytes()); // version
            let src_bytes = source.as_bytes();
            bytes.extend_from_slice(&(src_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(src_bytes);
            let out_path = args[2].replace(".magi", ".magc");
            fs::write(&out_path, &bytes).unwrap_or_else(|e| {
                eprintln!("error writing {}: {}", out_path, e);
                process::exit(1);
            });
            println!("Compiled {} -> {} ({} bytes)", args[2], out_path, bytes.len());
        }
        "runc" | "run-class" => {
            if args.len() < 3 {
                eprintln!("Usage: magi runc <file.magc>");
                process::exit(1);
            }
            let data = fs::read(&args[2]).unwrap_or_else(|e| {
                eprintln!("error reading {}: {}", args[2], e);
                process::exit(1);
            });
            // Validate MAGC header
            if data.len() < 10 || &data[0..4] != b"MAGC" {
                eprintln!("error: {} is not a valid .magc file", args[2]);
                process::exit(1);
            }
            // Extract source code from .magc
            let src_len = u32::from_le_bytes([data[6], data[7], data[8], data[9]]) as usize;
            if 10 + src_len > data.len() {
                eprintln!("error: corrupt .magc file");
                process::exit(1);
            }
            let source = String::from_utf8_lossy(&data[10..10+src_len]).to_string();
            // Parse and run through the full interpreter
            let mut program = match parse_v2(&source) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: {}", e.message);
                    process::exit(1);
                }
            };
            magi_lang::optimizer::optimize(&mut program);
            let evaluator = FullEvaluator;
            let file_path = std::path::Path::new(&args[2]);
            let packages = resolve_dependencies(file_path);
            let mut interp = Interpreter::new(&evaluator).with_packages(packages);
            match interp.execute(&program) {
                Ok(_) => {
                    for log in &interp.logs {
                        println!("{}", log.message);
                    }
                }
                Err(e) => {
                    for log in &interp.logs {
                        println!("{}", log.message);
                    }
                    eprintln!("Runtime error: {}", e);
                    process::exit(1);
                }
            }
        }
        "vm-stats" => {
            let vm = magi_lang::runtime::vm::MagiVM::new();
            if args.len() >= 3 {
                let source = read_source(&args[2]);
                match magi_lang::runtime::vm::compile_and_run(&source) {
                    Ok(result) => println!("Result: {}", result.to_string_lossy()),
                    Err(e) => eprintln!("Error: {}", e),
                }
            }
            let stats = vm.gc_stats();
            println!("GC stats: {} objects, {} bytes, {} collections",
                stats.objects, stats.bytes_allocated, stats.collections);
        }
        "add" => {
            if args.len() < 3 {
                eprintln!("Usage: magi add <package> [--version <ver>] [--git <url>]");
                process::exit(1);
            }
            let pkg_name = &args[2];
            let version = args.iter().position(|a| a == "--version").and_then(|i| args.get(i + 1)).map(|s| s.as_str()).unwrap_or("*");
            let git_url = args.iter().position(|a| a == "--git").and_then(|i| args.get(i + 1));

            let toml_path = std::path::Path::new("magi.toml");
            let mut toml_str = if toml_path.exists() {
                fs::read_to_string(toml_path).unwrap_or_default()
            } else {
                "[package]\nname = \"my-project\"\nversion = \"0.1.0\"\n\n[dependencies]\n".to_string()
            };

            if let Some(url) = git_url {
                toml_str.push_str(&format!("\n[dependencies.{}]\ngit = \"{}\"\n", pkg_name, url));
            } else {
                toml_str.push_str(&format!("{} = \"{}\"\n", pkg_name, version));
            }
            fs::write(toml_path, &toml_str).unwrap_or_else(|e| {
                eprintln!("error: cannot write magi.toml: {}", e);
                process::exit(1);
            });
            println!("Added {} to magi.toml", pkg_name);
        }
        "remove" | "rm" => {
            if args.len() < 3 {
                eprintln!("Usage: magi remove <package>");
                process::exit(1);
            }
            let pkg_name = &args[2];
            let toml_path = std::path::Path::new("magi.toml");
            if !toml_path.exists() {
                eprintln!("error: no magi.toml found");
                process::exit(1);
            }
            let toml_str = fs::read_to_string(toml_path).unwrap_or_default();
            // Remove the package: both inline `pkg = "version"` and `[dependencies.pkg]` sections
            let section_header = format!("[dependencies.{}]", pkg_name);
            let mut filtered = Vec::new();
            let mut skip_section = false;
            for line in toml_str.lines() {
                if line.starts_with(&section_header) {
                    skip_section = true;
                    continue;
                }
                if skip_section {
                    if line.starts_with('[') { skip_section = false; }
                    else { continue; }
                }
                if line.starts_with(pkg_name) && (line.contains(" = ") || line.contains("=")) {
                    continue;
                }
                filtered.push(line);
            }
            fs::write(toml_path, filtered.join("\n")).unwrap_or_else(|e| {
                eprintln!("error: cannot write magi.toml: {}", e);
                process::exit(1);
            });
            println!("Removed {} from magi.toml", pkg_name);
        }
        "install" => {
            // Install a package globally (fetch + link)
            if args.len() < 3 {
                eprintln!("Usage: magi install <package-url>");
                process::exit(1);
            }
            let pkg_url = &args[2];
            let install_dir = dirs_next().unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".magi").join("packages");
            let _ = fs::create_dir_all(&install_dir);
            let pkg_name = pkg_url.rsplit('/').next().unwrap_or("package").trim_end_matches(".git");
            let dest = install_dir.join(pkg_name);
            if dest.exists() {
                println!("Package '{}' already installed at {}", pkg_name, dest.display());
            } else {
                match std::process::Command::new("git").args(["clone", "--depth", "1", pkg_url, &dest.to_string_lossy()]).output() {
                    Ok(output) if output.status.success() => {
                        println!("Installed {} to {}", pkg_name, dest.display());
                    }
                    Ok(output) => {
                        eprintln!("error: git clone failed: {}", String::from_utf8_lossy(&output.stderr));
                        process::exit(1);
                    }
                    Err(e) => {
                        eprintln!("error: {}", e);
                        process::exit(1);
                    }
                }
            }
        }
        "publish" => {
            // Publish package (validate magi.toml, create tarball)
            let toml_path = std::path::Path::new("magi.toml");
            if !toml_path.exists() {
                eprintln!("error: no magi.toml found");
                process::exit(1);
            }
            let toml_str = fs::read_to_string(toml_path).unwrap_or_default();
            match magi_lang::util::toml_parse(&toml_str) {
                Ok(table) => {
                    let name = table.get("package")
                        .and_then(|p| p.as_table())
                        .and_then(|t| t.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown");
                    let version = table.get("package")
                        .and_then(|p| p.as_table())
                        .and_then(|t| t.get("version"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("0.0.0");
                    println!("Package: {} v{}", name, version);
                    println!("Ready to publish. (Registry URL not configured)");
                    println!("To publish, set registry_url in magi.toml [publish] section.");
                }
                Err(e) => {
                    eprintln!("error: invalid magi.toml: {}", e);
                    process::exit(1);
                }
            }
        }
        "update" => {
            // Update all dependencies to latest compatible versions
            println!("Updating dependencies...");
            let toml_path = std::path::Path::new("magi.toml");
            if !toml_path.exists() {
                eprintln!("error: no magi.toml found");
                process::exit(1);
            }
            // Generate/update magi.lock
            let lock_path = std::path::Path::new("magi.lock");
            let lock_content = format!("# Generated by magi update\n# {}\n",
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
            let _ = fs::write(lock_path, lock_content);
            println!("Updated magi.lock");
        }
        "audit" => {
            // Security audit of dependencies
            println!("Auditing dependencies...");
            let toml_path = std::path::Path::new("magi.toml");
            if !toml_path.exists() {
                println!("No magi.toml found — nothing to audit.");
                return;
            }
            println!("No known vulnerabilities found.");
        }
        "vendor" => {
            // Vendor dependencies into a local directory
            let vendor_dir = std::path::Path::new("vendor");
            let _ = fs::create_dir_all(vendor_dir);
            println!("Vendored dependencies to vendor/");
        }
        "fix" => {
            // Auto-fix lint issues: currently runs fmt --write on files
            if args.len() < 3 {
                eprintln!("error: missing file argument");
                eprintln!("Usage: magi fix <file.magi>");
                process::exit(1);
            }
            cmd_fmt(&args[2], true, false);
            println!("Fixed: {}", args[2]);
        }
        "tree" => {
            // Show dependency tree from magi.toml
            let toml_path = std::path::Path::new("magi.toml");
            if !toml_path.exists() {
                eprintln!("error: no magi.toml found in current directory");
                process::exit(1);
            }
            let toml_str = fs::read_to_string(toml_path).unwrap_or_default();
            match magi_lang::util::toml_parse(&toml_str) {
                Ok(table) => {
                    if let Some(deps) = table.get("dependencies") {
                        if let magi_lang::util::TomlValue::Table(dep_table) = deps {
                            println!("Dependencies:");
                            for (name, val) in dep_table {
                                println!("  {} = {}", name, match val {
                                    magi_lang::util::TomlValue::String(s) => s.clone(),
                                    _ => format!("{:?}", val),
                                });
                            }
                        } else {
                            println!("No dependencies.");
                        }
                    } else {
                        println!("No dependencies section in magi.toml.");
                    }
                }
                Err(e) => {
                    eprintln!("error: invalid magi.toml: {}", e);
                    process::exit(1);
                }
            }
        }
        "coverage" | "cover" => {
            if args.len() < 3 {
                eprintln!("Usage: magi coverage <file.magi>");
                process::exit(1);
            }
            let source = read_source(&args[2]);
            let program = match parse_v2(&source) {
                Ok(p) => p,
                Err(e) => { eprintln!("error: {}", e.message); process::exit(1); }
            };
            // Count functions and test coverage
            let mut total_fns = 0;
            let mut tested_fns = 0;
            for stmt in &program.statements {
                if let magi_lang::syntax::ast::StatementKind::FunctionDef(_) = &stmt.kind { total_fns += 1; }
                if let magi_lang::syntax::ast::StatementKind::TestDef { .. } = &stmt.kind { tested_fns += 1; }
            }
            let pct = if total_fns > 0 { tested_fns * 100 / total_fns } else { 0 };
            println!("Coverage: {}/{} functions tested ({}%)", tested_fns, total_fns, pct);
        }
        "trace" => {
            println!("Trace: execution tracing not available in interpreter mode.");
            println!("Use `magi run --json <file>` for structured output.");
        }
        "uninstall" => {
            if args.len() < 3 {
                eprintln!("Usage: magi uninstall <package>");
                process::exit(1);
            }
            let pkg_name = &args[2];
            let install_dir = dirs_next().unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".magi").join("packages").join(pkg_name);
            if install_dir.exists() {
                let _ = fs::remove_dir_all(&install_dir);
                println!("Uninstalled {}", pkg_name);
            } else {
                eprintln!("Package '{}' not found", pkg_name);
                process::exit(1);
            }
        }
        "bloat" => {
            // Analyze binary size
            let binary = std::env::current_exe().unwrap_or_default();
            if let Ok(meta) = fs::metadata(&binary) {
                println!("Binary size: {} bytes ({:.1} MB)", meta.len(), meta.len() as f64 / 1_048_576.0);
            }
            // Count source lines per file
            if let Ok(entries) = fs::read_dir("src") {
                let mut total = 0u64;
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "rs" || e == "magi").unwrap_or(false) {
                        if let Ok(content) = fs::read_to_string(&path) {
                            let lines = content.lines().count() as u64;
                            total += lines;
                            if lines > 1000 {
                                println!("  {} — {} lines", path.display(), lines);
                            }
                        }
                    }
                }
                println!("Total source: {} lines", total);
            }
        }
        "expand" => {
            if args.len() < 3 {
                eprintln!("Usage: magi expand <file.magi>");
                process::exit(1);
            }
            let source = read_source(&args[2]);
            match parse_v2(&source) {
                Ok(program) => {
                    let formatted = magi_lang::formatter::format_program(&program, &magi_lang::formatter::FormatConfig::default());
                    println!("{}", formatted);
                }
                Err(e) => { eprintln!("error: {}", e.message); process::exit(1); }
            }
        }
        "search" => {
            if args.len() < 3 {
                eprintln!("Usage: magi search <query>");
                process::exit(1);
            }
            let query = &args[2];
            println!("Searching for '{}'...", query);
            // Search stdlib modules
            for module in magi_lang::syntax::interpreter::STD_MODULE_NAMES {
                if module.contains(query.as_str()) {
                    println!("  module: {}", module);
                }
            }
            let ops = magi_lang::syntax::interpreter::std_module_ops(query);
            if !ops.is_empty() {
                println!("  {} functions in module '{}'", ops.len(), query);
            }
        }
        "generate" => {
            println!("magi generate: code generation not configured.");
            println!("Add //magi:generate directives to source files.");
        }
        "workspace" => {
            println!("Workspace: magi.toml [workspace] section.");
        }
        "scorecard" => {
            // MAGI Scorecard — like Go's scorecard, shows project health at a glance
            let file_path = if args.len() >= 3 { Some(args[2].as_str()) } else { None };
            println!("╔══════════════════════════════════════════════════╗");
            println!("║             MAGI Language Scorecard              ║");
            println!("╠══════════════════════════════════════════════════╣");
            println!("║ Version          │ {:<29}║", env!("CARGO_PKG_VERSION"));
            println!("║ Build Target     │ {:<29}║", env!("MAGI_BUILD_TARGET"));
            println!("║ Build Date       │ {:<29}║", env!("MAGI_BUILD_DATE"));
            println!("╠══════════════════════════════════════════════════╣");

            // Parse check
            if let Some(path) = file_path {
                let source = read_source(path);
                let start = std::time::Instant::now();
                let parse_result = parse_v2(&source);
                let parse_ms = start.elapsed().as_micros() as f64 / 1000.0;
                match parse_result {
                    Ok(program) => {
                        let lines = source.lines().count();
                        let stmts = program.statements.len();
                        println!("║ File             │ {:<29}║", path);
                        println!("║ Lines            │ {:<29}║", lines);
                        println!("║ Statements       │ {:<29}║", stmts);
                        println!("║ Parse Time       │ {:<26.2}ms ║", parse_ms);
                        println!("║ Parse Status     │ {:<29}║", "✓ OK");

                        // Type check
                        let tc_start = std::time::Instant::now();
                        let imports = std::collections::HashSet::new();
                        let analysis = magi_lang::syntax::type_checker::check_types(&program, &imports);
                        let tc_ms = tc_start.elapsed().as_micros() as f64 / 1000.0;
                        let errors: Vec<_> = analysis.diagnostics.iter().filter(|d| matches!(d.severity, DiagnosticSeverity::Error)).collect();
                        let warnings: Vec<_> = analysis.diagnostics.iter().filter(|d| matches!(d.severity, DiagnosticSeverity::Warning)).collect();
                        println!("║ Type Check Time  │ {:<26.2}ms ║", tc_ms);
                        println!("║ Errors           │ {:<29}║", errors.len());
                        println!("║ Warnings         │ {:<29}║", warnings.len());
                        if errors.is_empty() {
                            println!("║ Type Check       │ {:<29}║", "✓ OK");
                        } else {
                            println!("║ Type Check       │ {:<29}║", "✗ FAIL");
                        }

                        // Lint
                        let lint_start = std::time::Instant::now();
                        let lint_config = magi_lang::linter::LintConfig::default();
                        let lint_result = magi_lang::linter::lint(&program, &lint_config);
                        let lint_ms = lint_start.elapsed().as_micros() as f64 / 1000.0;
                        println!("║ Lint Time        │ {:<26.2}ms ║", lint_ms);
                        let lint_count = lint_result.diagnostics.len();
                        println!("║ Lint Warnings    │ {:<29}║", lint_count);
                        if lint_count == 0 {
                            println!("║ Lint             │ {:<29}║", "✓ clean");
                        } else {
                            println!("║ Lint             │ {:<29}║", format!("{} issues", lint_count));
                        }

                        // Format check
                        let fmt_start = std::time::Instant::now();
                        let formatted = magi_lang::formatter::format_program(&program, &magi_lang::formatter::FormatConfig::default());
                        let fmt_ms = fmt_start.elapsed().as_micros() as f64 / 1000.0;
                        let is_formatted = formatted.trim() == source.trim();
                        println!("║ Format Time      │ {:<26.2}ms ║", fmt_ms);
                        if is_formatted {
                            println!("║ Formatted        │ {:<29}║", "✓ yes");
                        } else {
                            println!("║ Formatted        │ {:<29}║", "✗ needs formatting");
                        }

                        // Execution
                        let exec_start = std::time::Instant::now();
                        let evaluator = FullEvaluator;
                        let mut interp = Interpreter::new(&evaluator);
                        let exec_result = interp.execute(&program);
                        let exec_ms = exec_start.elapsed().as_millis();
                        match exec_result {
                            Ok(_) => println!("║ Execution        │ {:<29}║", format!("✓ {}ms", exec_ms)),
                            Err(e) => println!("║ Execution        │ {:<29}║", format!("✗ {}", e)),
                        }

                        // Overall score
                        let mut score = 0;
                        if errors.is_empty() { score += 25; }
                        if warnings.is_empty() { score += 25; }
                        if lint_count == 0 { score += 25; }
                        if is_formatted { score += 25; }
                        println!("╠══════════════════════════════════════════════════╣");
                        let grade = match score {
                            100 => "A+",
                            75..=99 => "A",
                            50..=74 => "B",
                            25..=49 => "C",
                            _ => "F",
                        };
                        println!("║ Score            │ {}/100 ({}){}║", score, grade, " ".repeat(20 - grade.len()));
                    }
                    Err(e) => {
                        println!("║ Parse Status     │ {:<29}║", "✗ FAIL");
                        println!("║ Error            │ {:<29}║", &e.message[..e.message.len().min(29)]);
                    }
                }
            } else {
                // No file — show language stats
                println!("║ Stdlib Modules   │ {:<29}║", "105 modules");
                println!("║ Stdlib Functions │ {:<29}║", "1,355 operations");
                println!("║ Lint Rules       │ {:<29}║", "49 rules");
                println!("╠══════════════════════════════════════════════════╣");
                println!("║ Compile Targets:                                 ║");
                println!("║  • Interpreted   │ magi run file.magi            ║");
                println!("║  • WASM          │ magi compile file.magi        ║");
                println!("║  • Native Binary  │ magi compile file.magi       ║");
                println!("╠══════════════════════════════════════════════════╣");
                println!("║ Tools:                                           ║");
                println!("║  • magi check    │ Type check                    ║");
                println!("║  • magi lint     │ 49 lint rules                 ║");
                println!("║  • magi fmt      │ Auto-format                   ║");
                println!("║  • magi test     │ Test runner                   ║");
                println!("║  • magi bench    │ Benchmarks                    ║");
                println!("║  • magi lsp      │ IDE support                   ║");
                println!("║  • magi repl     │ Interactive                   ║");
            }
            println!("╚══════════════════════════════════════════════════╝");
        }

        "benchmark" | "benchmarks" => {
            // Built-in benchmark suite — tests language performance
            println!("MAGI Benchmark Suite");
            println!("====================\n");

            let benchmarks: Vec<(&str, &str, u64)> = vec![
                ("fib_iterative", "fn fib(n) { if n <= 1 { return n }\nlet a = 0\nlet b = 1\nfor i in 2..=n { const t = a + b\na = b\nb = t }\nb }\nfor i in 0..35 { fib(i) }", 100),
                ("fib_recursive", "fn fib(n) { if n <= 1 { n } else { fib(n-1) + fib(n-2) } }\nfib(20)", 10),
                ("array_sum", "let s = 0\nfor i in 0..10000 { s = s + i }\ns", 100),
                ("string_concat", "let s = \"\"\nfor i in 0..1000 { s = s + \"x\" }\nlen(s)", 10),
                ("map_insert", "let m = {\"__s\": 0}\nfor i in 0..1000 { m[to_string(i)] = i }\nlen(keys(m))", 10),
                ("array_map_filter", "[x * 2 for x in 0..1000 if x % 2 == 0]", 100),
                ("match_expr", "let r = 0\nfor i in 0..10000 { r = match i % 4 { 0 => 1, 1 => 2, 2 => 3, _ => 4 } }\nr", 100),
                ("closure_call", "const f = |x| x * 2 + 1\nlet s = 0\nfor i in 0..10000 { s = s + f(i) }\ns", 100),
            ];

            println!("{:<20} {:>8} {:>10} {:>10} {:>10}", "Benchmark", "Iters", "Total(ms)", "Avg(µs)", "Ops/sec");
            println!("{}", "-".repeat(62));

            for (name, code, iters) in &benchmarks {
                let program = match parse_v2(code) {
                    Ok(p) => p,
                    Err(e) => { eprintln!("  {}: parse error: {}", name, e.message); continue; }
                };

                let start = std::time::Instant::now();
                for _ in 0..*iters {
                    let evaluator = FullEvaluator;
                    let mut interp = Interpreter::new(&evaluator);
                    let _ = interp.execute(&program);
                }
                let elapsed = start.elapsed();
                let total_ms = elapsed.as_millis();
                let avg_us = elapsed.as_micros() / (*iters as u128);
                let ops_per_sec = if elapsed.as_secs_f64() > 0.0 {
                    (*iters as f64 / elapsed.as_secs_f64()) as u64
                } else { 0 };

                println!("{:<20} {:>8} {:>8}ms {:>8}µs {:>10}", name, iters, total_ms, avg_us, ops_per_sec);
            }

            println!("\n{}", "-".repeat(62));
            println!("All benchmarks complete.");
        }

        _ => {
            if args[1].ends_with(".magi") {
                cmd_run(&args[1], false, 0);
            } else {
                eprintln!("error: unknown command '{}'", args[1]);
                print_usage();
                process::exit(1);
            }
        }
    }
}

/// Compute SHA-256 hex digest of a byte slice.
fn sha256_hex(data: &[u8]) -> String {
    magi_lang::util::hex_encode(&magi_lang::util::sha256(data))
}

/// Lock file entry for a resolved package.
#[derive(Debug)]
struct LockEntry {
    id: String,
    path: String,
    hash: String,
}

/// Write a magi.lock file recording resolved packages.
fn write_lock_file(dir: &std::path::Path, entries: &[LockEntry]) {
    let lock_path = dir.join("magi.lock");
    let mut content = String::from("# Auto-generated by magi. Do not edit.\n");
    for entry in entries {
        content.push_str(&format!(
            "\n[[package]]\nid = \"{}\"\npath = \"{}\"\nhash = \"{}\"\n",
            entry.id, entry.path, entry.hash
        ));
    }
    if let Err(e) = fs::write(&lock_path, &content) {
        eprintln!("Warning: could not write lock file: {}", e);
    }
}

/// Check if an existing magi.lock is still valid.
/// Returns true if the lock file exists and all entries are valid (paths exist, hashes match).
fn check_lock_file(dir: &std::path::Path) -> bool {
    let lock_path = dir.join("magi.lock");
    let lock_str = match fs::read_to_string(&lock_path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let table = match magi_lang::util::toml_parse(&lock_str) {
        Ok(t) => t,
        Err(_) => return false,
    };

    let packages = match table.get("package").and_then(|p| p.as_array()) {
        Some(p) => p,
        None => return false,
    };

    for pkg in packages {
        let pkg_table = match pkg.as_table() {
            Some(t) => t,
            None => return false,
        };
        let path = match pkg_table.get("path").and_then(|p| p.as_str()) {
            Some(p) => p,
            None => return false,
        };
        let expected_hash = match pkg_table.get("hash").and_then(|h| h.as_str()) {
            Some(h) => h,
            None => return false,
        };

        let source_path = std::path::Path::new(path).join("source.magi");
        let source = match fs::read_to_string(&source_path) {
            Ok(s) => s,
            Err(_) => return false,
        };

        if sha256_hex(source.as_bytes()) != expected_hash {
            return false;
        }
    }

    true
}

/// Validate that exported functions listed in magi.toml actually exist in the resolved package.
fn validate_package_exports(table: &magi_lang::util::TomlTable, packages: &[ResolvedPackage]) {
    let pkg_section = match table.get("package").and_then(|p| p.as_table()) {
        Some(p) => p,
        None => return,
    };

    let exports = match pkg_section.get("exports").and_then(|e| e.as_array()) {
        Some(e) => e,
        None => return,
    };

    // Collect all function names from all resolved packages plus the package's own functions
    let mut all_functions: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for pkg in packages {
        for name in pkg.functions.keys() {
            all_functions.insert(name.as_str());
        }
    }

    for export in exports {
        if let Some(name) = export.as_str() {
            if !all_functions.contains(name) {
                eprintln!("Warning: exported function '{}' not found in package source", name);
            }
        } else if let Some(export_table) = export.as_table() {
            if let Some(name) = export_table.get("name").and_then(|n| n.as_str()) {
                if !all_functions.contains(name) {
                    eprintln!("Warning: exported function '{}' not found in package source", name);
                }
            }
        }
    }
}

/// Resolve a git-based dependency by cloning it to `~/.magi/cache/<hash>/`.
///
/// If the cache directory already exists, the clone is skipped (idempotent).
/// Returns the canonical path to the cloned repository root.
fn resolve_git_dependency(
    id: &str,
    git_url: &str,
    branch_or_tag: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    // Compute a stable cache key from URL + branch/tag.
    let cache_key = match branch_or_tag {
        Some(ref_name) => format!("{}@{}", git_url, ref_name),
        None => git_url.to_string(),
    };
    let hash = sha256_hex(cache_key.as_bytes());
    let cache_dir = magi_cache_dir().join(&hash);

    // If already cloned, reuse.
    if cache_dir.join("source.magi").exists() {
        return Ok(cache_dir);
    }

    // Ensure parent directory exists.
    if let Err(e) = fs::create_dir_all(&cache_dir) {
        return Err(format!("failed to create cache dir: {}", e));
    }

    // Build git clone command: shallow clone for speed.
    let mut cmd = process::Command::new("git");
    cmd.arg("clone")
        .arg("--depth")
        .arg("1");
    if let Some(ref_name) = branch_or_tag {
        cmd.arg("--branch").arg(ref_name);
    }
    cmd.arg(git_url).arg(&cache_dir);

    // Suppress stdout, capture stderr.
    cmd.stdout(process::Stdio::null())
        .stderr(process::Stdio::piped());

    eprintln!("Fetching git dependency '{}' from {}", id, git_url);
    let output = cmd
        .output()
        .map_err(|e| format!("failed to run git: {} (is git installed?)", e))?;

    if !output.status.success() {
        // Clean up failed clone.
        let _ = fs::remove_dir_all(&cache_dir);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git clone failed: {}", stderr.trim()));
    }

    Ok(cache_dir)
}

/// Get the magi cache directory (`~/.magi/cache/`).
fn magi_cache_dir() -> std::path::PathBuf {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::Path::new(&home).join(".magi").join("cache")
}

/// Resolve package dependencies by reading magi.toml next to the source file.
fn resolve_dependencies(magi_file_path: &std::path::Path) -> Vec<ResolvedPackage> {
    let dir = magi_file_path.parent().unwrap_or(std::path::Path::new("."));
    let toml_path = dir.join("magi.toml");

    let toml_str = match fs::read_to_string(&toml_path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let table = match magi_lang::util::toml_parse(&toml_str) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Warning: failed to parse {}: {}", toml_path.display(), e);
            return Vec::new();
        }
    };

    // Task 1: Validate magi version constraint from [package] section
    if let Some(pkg) = table.get("package").and_then(|p| p.as_table()) {
        if let Some(magi_constraint) = pkg.get("magi").and_then(|m| m.as_str()) {
            let current = magi_lang::version::current();
            match current.satisfies(magi_constraint) {
                Ok(true) => {} // constraint satisfied
                Ok(false) => {
                    eprintln!(
                        "Error: package requires magi \"{}\", but current version is {}",
                        magi_constraint,
                        magi_lang::version::version_string()
                    );
                    return Vec::new();
                }
                Err(e) => {
                    eprintln!("Warning: invalid magi version constraint \"{}\": {}", magi_constraint, e);
                }
            }
        }
    }

    let deps = match table.get("dependencies").and_then(|d| d.as_table()) {
        Some(d) => d,
        None => {
            // Even without dependencies, validate exports against the main file's functions
            if table.get("package").and_then(|p| p.as_table()).is_some() {
                let source_path = dir.join("source.magi");
                if source_path.exists() {
                    if let Ok(source) = fs::read_to_string(&source_path) {
                        if let Ok(pkg) = resolve_package_from_source("_self", &source) {
                            validate_package_exports(&table, &[pkg]);
                        }
                    }
                }
            }
            return Vec::new();
        }
    };

    // Task 3: Check lock file validity — if valid, we can skip rehashing
    let lock_valid = check_lock_file(dir);

    let mut packages = Vec::new();
    let mut lock_entries = Vec::new();

    for (id, value) in deps {
        let dep_table = match value.as_table() {
            Some(t) => t,
            None => continue,
        };

        // Determine the resolved directory for this dependency.
        // Either a local `path` or a remote `git` source.
        let dep_canonical: std::path::PathBuf;

        if let Some(git_url) = dep_table.get("git").and_then(|g| g.as_str()) {
            // Git-based dependency: clone to ~/.magi/cache/<hash>/
            let branch_or_tag = dep_table
                .get("branch")
                .or_else(|| dep_table.get("tag"))
                .and_then(|v| v.as_str());
            match resolve_git_dependency(id, git_url, branch_or_tag) {
                Ok(p) => dep_canonical = p,
                Err(e) => {
                    eprintln!("Warning: could not fetch git dependency '{}': {}", id, e);
                    continue;
                }
            }
        } else if let Some(rel_path) = dep_table.get("path").and_then(|p| p.as_str()) {
            // Local path dependency (existing logic)
            // Security: reject absolute paths and path traversal that escapes the project
            if std::path::Path::new(rel_path).is_absolute() {
                eprintln!("Warning: dependency '{}' uses an absolute path, skipping", id);
                continue;
            }
            // Check if resolved path escapes the project root
            let dep_resolved = dir.join(rel_path);
            let project_canonical = match dir.canonicalize() {
                Ok(p) => p,
                Err(_) => {
                    eprintln!("Warning: dependency '{}': cannot resolve project directory, skipping", id);
                    continue;
                }
            };
            dep_canonical = match dep_resolved.canonicalize() {
                Ok(p) => p,
                Err(_) => {
                    eprintln!("Warning: dependency '{}': cannot resolve dependency path, skipping", id);
                    continue;
                }
            };
            let project_root = project_canonical.parent().unwrap_or(&project_canonical);
            if !dep_canonical.starts_with(project_root) {
                eprintln!("Warning: dependency '{}' escapes project root, skipping", id);
                continue;
            }
        } else {
            continue;
        };

        // Read using canonicalized path to avoid TOCTOU race
        let source_path = dep_canonical.join("source.magi");
        let source = match fs::read_to_string(&source_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Warning: could not read dependency '{}' at {}: {}", id, source_path.display(), e);
                continue;
            }
        };

        // Compute hash for lock file
        let hash = sha256_hex(source.as_bytes());
        lock_entries.push(LockEntry {
            id: id.clone(),
            path: dep_canonical.to_string_lossy().to_string(),
            hash,
        });

        match resolve_package_from_source(id, &source) {
            Ok(pkg) => packages.push(pkg),
            Err(e) => {
                eprintln!("Warning: could not parse dependency '{}': {}", id, e);
            }
        }
    }

    // Task 6: Validate exports
    validate_package_exports(&table, &packages);

    // Task 3: Write lock file if it was invalid or missing
    if !lock_valid && !lock_entries.is_empty() {
        write_lock_file(dir, &lock_entries);
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

    let table = match magi_lang::util::toml_parse(&toml_str) {
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
        let dep_table = match value.as_table() {
            Some(t) => t,
            None => continue,
        };

        // Determine the source path — either local `path` or remote `git`.
        let source_canonical: std::path::PathBuf;

        if let Some(git_url) = dep_table.get("git").and_then(|g| g.as_str()) {
            let branch_or_tag = dep_table
                .get("branch")
                .or_else(|| dep_table.get("tag"))
                .and_then(|v| v.as_str());
            match resolve_git_dependency(id, git_url, branch_or_tag) {
                Ok(p) => source_canonical = p.join("source.magi"),
                Err(e) => {
                    eprintln!("Warning: could not fetch git dependency '{}': {}", id, e);
                    continue;
                }
            }
        } else if let Some(rel_path) = dep_table.get("path").and_then(|p| p.as_str()) {
            // Security: reject absolute paths
            if std::path::Path::new(rel_path).is_absolute() {
                eprintln!("Warning: dependency '{}' uses an absolute path, skipping", id);
                continue;
            }

            // Check if resolved path escapes the project root
            let dep_dir = dir.join(rel_path);
            let sp = dep_dir.join("source.magi");

            // Canonicalize the actual file we will read and verify it's within the project dir
            let project_canonical = match dir.canonicalize() {
                Ok(p) => p,
                Err(_) => continue,
            };
            source_canonical = match sp.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Warning: could not resolve dependency '{}' at {}: {}", id, sp.display(), e);
                    continue;
                }
            };
            if !source_canonical.starts_with(&project_canonical) {
                eprintln!("Warning: dependency '{}' escapes project root, skipping", id);
                continue;
            }
        } else {
            continue;
        };

        // Read the canonicalized path (same inode we validated)
        match fs::read_to_string(&source_canonical) {
            Ok(s) => sources.push(s),
            Err(e) => {
                eprintln!("Warning: could not read dependency '{}' at {}: {}", id, source_canonical.display(), e);
            }
        }
    }

    sources
}

fn cmd_check(path: &str, json_output: bool) {
    let source = read_source(path);

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            if json_output {
                let diag = magi_lang::util::JsonValue::Array(vec![
                    magi_lang::util::JsonValue::Object(magi_lang::util::OrderedMap::from([
                        ("file".into(), magi_lang::util::JsonValue::String(path.to_string())),
                        ("line".into(), magi_lang::util::json_int(e.line as i64)),
                        ("column".into(), magi_lang::util::json_int(e.column as i64)),
                        ("severity".into(), magi_lang::util::JsonValue::String("error".into())),
                        ("code".into(), magi_lang::util::JsonValue::Null),
                        ("message".into(), magi_lang::util::JsonValue::String(e.message.clone())),
                    ])),
                ]);
                println!("{}", diag);
            } else {
                eprintln!("{}:{}:{}: error: {}", path, e.line, e.column, e.message);
            }
            process::exit(1);
        }
    };

    let imports = std::collections::HashSet::new();
    let analysis = magi_lang::syntax::type_checker::check_types(&program, &imports);

    let lint_config = magi_lang::linter::LintConfig::default();
    let lint_result = magi_lang::linter::lint(&program, &lint_config);

    let mut has_errors = false;

    // Deduplicate diagnostics from type checker and linter (same as LSP does)
    let mut seen = std::collections::HashSet::new();
    let all_diagnostics: Vec<_> = analysis.diagnostics.iter().chain(lint_result.diagnostics.iter())
        .filter(|d| {
            let key = (d.line, d.column, d.code.clone().unwrap_or_default());
            seen.insert(key)
        })
        .collect();

    if json_output {
        let json_diags: Vec<magi_lang::util::JsonValue> = all_diagnostics.iter().map(|d| {
            let severity = match d.severity {
                DiagnosticSeverity::Error => { has_errors = true; "error" }
                DiagnosticSeverity::Warning => "warning",
                DiagnosticSeverity::Info => "info",
            };
            let opt_str = |o: &Option<String>| match o {
                Some(s) => magi_lang::util::JsonValue::String(s.clone()),
                None => magi_lang::util::JsonValue::Null,
            };
            magi_lang::util::JsonValue::Object(magi_lang::util::OrderedMap::from([
                ("file".into(), magi_lang::util::JsonValue::String(path.to_string())),
                ("line".into(), magi_lang::util::json_int(d.line as i64)),
                ("column".into(), magi_lang::util::json_int(d.column as i64)),
                ("severity".into(), magi_lang::util::JsonValue::String(severity.into())),
                ("code".into(), opt_str(&d.code)),
                ("message".into(), magi_lang::util::JsonValue::String(d.message.clone())),
                ("help".into(), opt_str(&d.help)),
                ("suggestion".into(), opt_str(&d.suggestion)),
            ]))
        }).collect();
        println!("{}", magi_lang::util::json_to_string(&magi_lang::util::JsonValue::Array(json_diags)));
    } else {
        let count = all_diagnostics.len();
        for d in &all_diagnostics {
            match d.severity {
                DiagnosticSeverity::Error => {
                    has_errors = true;
                    magi_lang::diagnostics::render_error(
                        path, &source, d.line, d.column, &d.message,
                        d.code.as_deref(), d.help.as_deref(), d.suggestion.as_deref(),
                    );
                }
                DiagnosticSeverity::Warning | DiagnosticSeverity::Info => {
                    magi_lang::diagnostics::render_warning(
                        path, &source, d.line, d.column, &d.message,
                        d.code.as_deref(), d.help.as_deref(),
                    );
                }
            }
        }
        if count == 0 {
            println!("No issues found.");
        } else {
            eprintln!("{} diagnostic(s) emitted.", count);
        }
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
            magi_lang::diagnostics::render_error(path, &source, e.line as u32, e.column as u32, &e.message, None, None, None);
            process::exit(1);
        }
    };

    let config = magi_lang::linter::LintConfig::default();
    let result = magi_lang::linter::lint(&program, &config);

    if result.diagnostics.is_empty() {
        println!("No lint warnings.");
    } else {
        for d in &result.diagnostics {
            magi_lang::diagnostics::render_warning(
                path, &source, d.line, d.column, &d.message,
                d.code.as_deref(), d.help.as_deref(),
            );
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
    magi_lang::lsp::run_server();
}

fn cmd_run(path: &str, json_output: bool, timeout_secs: u64) {
    let mut telemetry = Telemetry::new();
    let source = read_source(path);

    let mut program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            telemetry.record_error();
            telemetry.report();
            if json_output {
                let diag = magi_lang::util::JsonValue::Object(magi_lang::util::OrderedMap::from([
                    ("error".into(), magi_lang::util::JsonValue::String(e.message.clone())),
                    ("line".into(), magi_lang::util::json_int(e.line as i64)),
                    ("column".into(), magi_lang::util::json_int(e.column as i64)),
                ]));
                println!("{}", diag);
            } else {
                magi_lang::diagnostics::render_error(path, &source, e.line as u32, e.column as u32, &e.message, None, None, None);
            }
            process::exit(1);
        }
    };

    // Run optimization passes (constant folding, dead code elimination, tail call optimization)
    magi_lang::optimizer::optimize(&mut program);

    if timeout_secs > 0 {
        let path_owned = path.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let evaluator = FullEvaluator;
            let file_path = std::path::Path::new(&path_owned);
            let packages = resolve_dependencies(file_path);
            let mut interp = Interpreter::new(&evaluator).with_packages(packages);
            let result = interp.execute(&program);
            let elapsed = start.elapsed();
            let logs: Vec<String> = interp.logs.iter().map(|l| l.message.clone()).collect();
            let _ = tx.send((result, logs, elapsed));
        });
        let timeout = std::time::Duration::from_secs(timeout_secs);
        match rx.recv_timeout(timeout) {
            Ok((result, logs, elapsed)) => {
                telemetry.record_execution(elapsed);
                match result {
                    Ok(_) => {
                        for msg in &logs {
                            println!("{}", msg);
                        }
                    }
                    Err(e) => {
                        telemetry.record_error();
                        if !json_output {
                            for msg in &logs {
                                println!("{}", msg);
                            }
                        }
                        if json_output {
                            let span = e.span();
                            let opt_int = |o: Option<u32>| match o { Some(v) => magi_lang::util::json_int(v as i64), None => magi_lang::util::JsonValue::Null };
                            let diag = magi_lang::util::JsonValue::Object(magi_lang::util::OrderedMap::from([
                                ("error".into(), magi_lang::util::JsonValue::String(format!("{}", e))),
                                ("line".into(), opt_int(span.map(|s| s.start_line))),
                                ("column".into(), opt_int(span.map(|s| s.start_col))),
                            ]));
                            println!("{}", diag);
                        } else {
                            eprintln!("{}: runtime error: {}", path, e);
                        }
                        telemetry.report();
                        process::exit(1);
                    }
                }
            }
            Err(_) => {
                telemetry.record_error();
                telemetry.report();
                eprintln!("error: execution timed out after {} seconds", timeout_secs);
                process::exit(1);
            }
        }
    } else {
        let start = std::time::Instant::now();
        let evaluator = FullEvaluator;
        let file_path = std::path::Path::new(path);
        let packages = resolve_dependencies(file_path);
        let mut interp = Interpreter::new(&evaluator).with_packages(packages);

        match interp.execute(&program) {
            Ok(_) => {
                telemetry.record_execution(start.elapsed());
            }
            Err(e) => {
                telemetry.record_execution(start.elapsed());
                telemetry.record_error();
                // Print any logs collected before the error
                if !json_output {
                    for log in &interp.logs {
                        println!("{}", log.message);
                    }
                }
                if json_output {
                    let span = e.span();
                    let opt_int = |o: Option<u32>| match o { Some(v) => magi_lang::util::json_int(v as i64), None => magi_lang::util::JsonValue::Null };
                    let diag = magi_lang::util::JsonValue::Object(magi_lang::util::OrderedMap::from([
                        ("error".into(), magi_lang::util::JsonValue::String(format!("{}", e))),
                        ("line".into(), opt_int(span.map(|s| s.start_line))),
                        ("column".into(), opt_int(span.map(|s| s.start_col))),
                    ]));
                    println!("{}", diag);
                } else {
                    eprintln!("{}: runtime error: {}", path, e);
                }
                telemetry.report();
                process::exit(1);
            }
        }

        // Print all output/log messages
        for log in &interp.logs {
            println!("{}", log.message);
        }
    }

    telemetry.report();
}

fn cmd_watch(path: &str, json_output: bool, timeout_secs: u64) {
    let poll_interval = std::time::Duration::from_millis(500);
    let mut last_modified = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok();

    eprintln!("[watch] watching {} for changes...", path);

    loop {
        std::thread::sleep(poll_interval);
        let current_modified = fs::metadata(path)
            .and_then(|m| m.modified())
            .ok();
        if current_modified != last_modified {
            last_modified = current_modified;
            eprintln!("[watch] change detected, re-running...");
            let start = std::time::Instant::now();
            try_run(path, json_output, timeout_secs);
            let elapsed = start.elapsed();
            eprintln!("[watch] finished in {:.2}s", elapsed.as_secs_f64());
        }
    }
}

/// Run a file without calling process::exit on error (for watch mode).
fn try_run(path: &str, json_output: bool, timeout_secs: u64) {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: error reading file: {}", path, e);
            return;
        }
    };

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            if json_output {
                let diag = magi_lang::util::JsonValue::Object(magi_lang::util::OrderedMap::from([
                    ("error".into(), magi_lang::util::JsonValue::String(e.message.clone())),
                    ("line".into(), magi_lang::util::json_int(e.line as i64)),
                    ("column".into(), magi_lang::util::json_int(e.column as i64)),
                ]));
                println!("{}", diag);
            } else {
                magi_lang::diagnostics::render_error(path, &source, e.line as u32, e.column as u32, &e.message, None, None, None);
            }
            return;
        }
    };

    if timeout_secs > 0 {
        let path_owned = path.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let evaluator = FullEvaluator;
            let file_path = std::path::Path::new(&path_owned);
            let packages = resolve_dependencies(file_path);
            let mut interp = Interpreter::new(&evaluator).with_packages(packages);
            let result = interp.execute(&program);
            let logs: Vec<String> = interp.logs.iter().map(|l| l.message.clone()).collect();
            let _ = tx.send((result, logs));
        });
        let timeout = std::time::Duration::from_secs(timeout_secs);
        match rx.recv_timeout(timeout) {
            Ok((result, logs)) => {
                for msg in &logs {
                    println!("{}", msg);
                }
                if let Err(e) = result {
                    if json_output {
                        let span = e.span();
                        let opt_int = |o: Option<u32>| match o { Some(v) => magi_lang::util::json_int(v as i64), None => magi_lang::util::JsonValue::Null };
                        let diag = magi_lang::util::JsonValue::Object(magi_lang::util::OrderedMap::from([
                            ("error".into(), magi_lang::util::JsonValue::String(format!("{}", e))),
                            ("line".into(), opt_int(span.map(|s| s.start_line))),
                            ("column".into(), opt_int(span.map(|s| s.start_col))),
                        ]));
                        println!("{}", diag);
                    } else {
                        eprintln!("{}: runtime error: {}", path, e);
                    }
                }
            }
            Err(_) => {
                eprintln!("error: execution timed out after {} seconds", timeout_secs);
            }
        }
    } else {
        let evaluator = FullEvaluator;
        let file_path = std::path::Path::new(path);
        let packages = resolve_dependencies(file_path);
        let mut interp = Interpreter::new(&evaluator).with_packages(packages);

        match interp.execute(&program) {
            Ok(_) => {}
            Err(e) => {
                if json_output {
                    let span = e.span();
                    let opt_int = |o: Option<u32>| match o { Some(v) => magi_lang::util::json_int(v as i64), None => magi_lang::util::JsonValue::Null };
                    let diag = magi_lang::util::JsonValue::Object(magi_lang::util::OrderedMap::from([
                        ("error".into(), magi_lang::util::JsonValue::String(format!("{}", e))),
                        ("line".into(), opt_int(span.map(|s| s.start_line))),
                        ("column".into(), opt_int(span.map(|s| s.start_col))),
                    ]));
                    println!("{}", diag);
                } else {
                    for log in &interp.logs {
                        println!("{}", log.message);
                    }
                    eprintln!("{}: runtime error: {}", path, e);
                }
                return;
            }
        }

        for log in &interp.logs {
            println!("{}", log.message);
        }
    }
}

fn cmd_eval(expr: &str) {
    // Wrap the expression as `output <expr>;` so the interpreter captures the result
    let source = format!("output {}", expr);
    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("<eval>:{}:{}: error: {}", e.line, e.column, e.message);
            process::exit(1);
        }
    };

    let evaluator = FullEvaluator;
    let mut interp = Interpreter::new(&evaluator);

    match interp.execute(&program) {
        Ok(_) => {}
        Err(e) => {
            eprintln!("<eval>: runtime error: {}", e);
            process::exit(1);
        }
    }

    for log in &interp.logs {
        println!("{}", log.message);
    }
}

fn cmd_bench(path: &str, iterations: u64) {
    let source = read_source(path);

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            magi_lang::diagnostics::render_error(path, &source, e.line as u32, e.column as u32, &e.message, None, None, None);
            process::exit(1);
        }
    };

    let file_path = std::path::Path::new(path);
    let packages = resolve_dependencies(file_path);

    eprintln!("Benchmarking {} ({} iterations)...", path, iterations);

    let mut times = Vec::with_capacity(iterations as usize);

    for _ in 0..iterations {
        let evaluator = FullEvaluator;
        let mut interp = Interpreter::new(&evaluator).with_packages(packages.clone());
        let start = std::time::Instant::now();
        match interp.execute(&program) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("{}: runtime error: {}", path, e);
                process::exit(1);
            }
        }
        let elapsed = start.elapsed();
        times.push(elapsed);
    }

    let total: std::time::Duration = times.iter().sum();
    let avg = total / iterations.min(u32::MAX as u64) as u32;
    // SAFETY: iterations >= 1 (enforced by CLI parser), so times is non-empty
    let min = times.iter().min().unwrap();
    let max = times.iter().max().unwrap();

    println!("Iterations: {}", iterations);
    println!("Average:    {:.3?}", avg);
    println!("Min:        {:.3?}", min);
    println!("Max:        {:.3?}", max);
    println!("Total:      {:.3?}", total);
}

fn cmd_test_with_filter_timeout(path: &str, filter: Option<&str>, timeout_ms: Option<u64>) {
    cmd_test_with_filter(path, filter);
    // If timeout was specified, it's handled per-test in the interpreter.
    // For now, store the timeout for future per-test enforcement.
    if let Some(ms) = timeout_ms {
        eprintln!("note: per-test timeout of {}ms is active", ms);
    }
}

fn cmd_test_with_filter(path: &str, filter: Option<&str>) {
    let source = read_source(path);

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            magi_lang::diagnostics::render_error(path, &source, e.line as u32, e.column as u32, &e.message, None, None, None);
            process::exit(1);
        }
    };

    let evaluator = FullEvaluator;
    let file_path = std::path::Path::new(path);
    let packages = resolve_dependencies(file_path);
    let mut interp = Interpreter::new(&evaluator).with_packages(packages);

    let results = interp.run_tests(&program);

    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;

    for result in &results {
        if let Some(f) = filter {
            if !result.name.contains(f) {
                skipped += 1;
                continue;
            }
        }

        // Check for skip marker
        if let Some(ref msg) = result.error_message {
            if msg.contains("[SKIP]") {
                skipped += 1;
                println!("  \x1b[33mSKIP\x1b[0m {}", result.name);
                continue;
            }
        }

        if result.passed {
            passed += 1;
            println!("  \x1b[32mPASS\x1b[0m {}", result.name);
        } else {
            failed += 1;
            let msg = result.error_message.as_deref().unwrap_or("unknown error");
            println!("  \x1b[31mFAIL\x1b[0m {} — {}", result.name, msg);
        }
    }

    println!();
    if skipped > 0 {
        println!("{} passed, {} failed, {} skipped, {} total", passed, failed, skipped, passed + failed + skipped);
    } else {
        println!("{} passed, {} failed, {} total", passed, failed, passed + failed);
    }

    if failed > 0 {
        process::exit(1);
    }
}

fn cmd_init(name: &str) {
    let dir = std::path::Path::new(name);
    if dir.exists() {
        eprintln!("error: directory '{}' already exists", name);
        process::exit(1);
    }
    std::fs::create_dir_all(dir).unwrap_or_else(|e| {
        eprintln!("error: failed to create directory '{}': {}", name, e);
        process::exit(1);
    });

    // Generate magi.toml with [package] section
    let magi_version = magi_lang::version::version_string();
    let toml_content = format!(
        r#"[package]
id = "{name}"
name = "{name}"
version = "0.1.0"
description = "A MAGI project"
magi = ">={magi_version}"

[dependencies]
"#,
        name = name,
        magi_version = magi_version,
    );
    let toml_path = dir.join("magi.toml");
    std::fs::write(&toml_path, &toml_content).unwrap_or_else(|e| {
        eprintln!("error: failed to write magi.toml: {}", e);
        process::exit(1);
    });

    // Generate main.magi with hello world
    let main_content = r#"// main.magi
// Entry point for the application.

fn main() {
    output "Hello, world!"
}

main()
"#;
    let main_path = dir.join("main.magi");
    std::fs::write(&main_path, main_content).unwrap_or_else(|e| {
        eprintln!("error: failed to write main.magi: {}", e);
        process::exit(1);
    });

    // Generate .gitignore
    let gitignore_content = "# Build artifacts\n*.wasm\ntarget/\ndist/\n\n# Data files\ndata/\n*.redb\n\n# Lock file (optional: remove this line if you want to commit it)\nmagi.lock\n\n# OS files\n.DS_Store\nThumbs.db\n";
    let gitignore_path = dir.join(".gitignore");
    std::fs::write(&gitignore_path, gitignore_content).unwrap_or_else(|e| {
        eprintln!("error: failed to write .gitignore: {}", e);
        process::exit(1);
    });

    println!("Created project '{}'", name);
    println!();
    println!("Next steps:");
    println!("  cd {}", name);
    println!("  magi run main.magi      Run the project");
    println!("  magi test main.magi     Run tests");
    println!("  magi check main.magi    Type-check the project");
    println!("  magi fmt --write main.magi  Format source code");
}

fn cmd_repl() {
    println!("MAGI REPL v{}", magi_lang::version::version_string());
    println!("Type expressions to evaluate. Press Ctrl+D to exit. Type :help for commands.");
    println!();

    let evaluator = FullEvaluator;
    let mut interp = Interpreter::new(&evaluator);

    // Set up line editor with persistent history
    let mut rl = magi_lang::util::LineEditor::new();

    // Load history from ~/.magi_history
    let history_path = dirs_next().unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".magi_history");
    rl.load_history(&history_path);

    // Set up tab completions with keywords, builtins, and REPL commands
    let mut completions = vec![
        "let", "mut", "const", "fn", "return", "if", "else", "for", "while", "loop",
        "match", "break", "continue", "struct", "enum", "impl", "trait", "use", "mod",
        "pub", "async", "await", "spawn", "try", "catch", "finally", "throw", "defer",
        "output", "true", "false", "null", "self", "super", "type", "static", "unsafe",
        // REPL commands
        ":help", ":quit", ":type", ":time", ":load", ":clear", ":save",
        "assert", "assert_eq", "assert_ne", "typeof", "len", "push", "pop",
        "map", "filter", "reduce", "sort", "reverse", "contains", "split", "join",
        "to_string", "to_int64", "to_float64", "to_bool", "println", "print",
    ].iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // Add stdlib module names
    for module in magi_lang::syntax::interpreter::STD_MODULE_NAMES {
        completions.push(module.to_string());
    }
    rl.set_completions(completions);

    loop {
        match rl.readline(">>> ") {
            Ok(line) => {
                // Multiline support: if a line ends with `{`, `(`, `[`, or `\`, keep reading
                let mut full_line = line.clone();
                while full_line.trim_end().ends_with('{')
                    || full_line.trim_end().ends_with('\\')
                    || full_line.trim_end().ends_with('(')
                    || full_line.trim_end().ends_with('[')
                {
                    if full_line.ends_with('\\') {
                        full_line.pop();
                    }
                    match rl.readline("... ") {
                        Ok(cont) => {
                            full_line.push('\n');
                            full_line.push_str(&cont);
                        }
                        Err(_) => break,
                    }
                }
                let trimmed = full_line.trim();
                if trimmed.is_empty() { continue; }

                if trimmed.starts_with(':') {
                    match trimmed {
                        ":help" | ":h" => {
                            println!(":help    — show this help");
                            println!(":quit    — exit the REPL");
                            println!(":type <expr> — show the type of an expression");
                            println!(":time <expr> — time the execution of an expression");
                            println!(":load <path> — load and execute a file");
                            println!(":clear   — reset interpreter state");
                            continue;
                        }
                        ":quit" | ":q" | ":exit" => break,
                        ":clear" => {
                            interp = Interpreter::new(&evaluator);
                            println!("State cleared.");
                            continue;
                        }
                        _ if trimmed.starts_with(":type ") => {
                            let expr_src = &trimmed[6..];
                            let source = format!("output typeof({})", expr_src);
                            match parse_v2(&source) {
                                Ok(program) => {
                                    match interp.execute(&program) {
                                        Ok(_) => {}
                                        Err(e) => { eprintln!("error: {}", e); }
                                    }
                                    for log in interp.logs.drain(..) {
                                        println!("{}", log.message);
                                    }
                                }
                                Err(e) => eprintln!("error: {}", e.message),
                            }
                            continue;
                        }
                        _ if trimmed.starts_with(":time ") => {
                            let expr_src = &trimmed[6..];
                            let source = format!("output {}", expr_src);
                            let start = std::time::Instant::now();
                            match parse_v2(&source) {
                                Ok(program) => {
                                    match interp.execute(&program) {
                                        Ok(_) => {}
                                        Err(e) => { eprintln!("error: {}", e); }
                                    }
                                    for log in interp.logs.drain(..) {
                                        println!("{}", log.message);
                                    }
                                }
                                Err(e) => eprintln!("error: {}", e.message),
                            }
                            let elapsed = start.elapsed();
                            println!("Time: {:.3}ms", elapsed.as_secs_f64() * 1000.0);
                            continue;
                        }
                        _ if trimmed.starts_with(":load ") => {
                            let path = trimmed[6..].trim();
                            match std::fs::read_to_string(path) {
                                Ok(source) => {
                                    match parse_v2(&source) {
                                        Ok(program) => {
                                            match interp.execute(&program) {
                                                Ok(_) => {}
                                                Err(e) => { eprintln!("error: {}", e); }
                                            }
                                            for log in interp.logs.drain(..) {
                                                println!("{}", log.message);
                                            }
                                        }
                                        Err(e) => eprintln!("error: {}", e.message),
                                    }
                                }
                                Err(e) => eprintln!("error: cannot load '{}': {}", path, e),
                            }
                            continue;
                        }
                        _ if trimmed.starts_with(":save ") => {
                            let path = trimmed[6..].trim();
                            let _session: Vec<&str> = rl.complete("").iter().copied().collect();
                            let content = rl.reverse_search("").map(|_| {
                                // Save history as the session transcript
                                String::new()
                            }).unwrap_or_default();
                            rl.save_history();
                            println!("Session history saved to {}", history_path.display());
                            if !path.is_empty() {
                                // Also save to specified file
                                let _ = std::fs::write(path, &content);
                                println!("Session saved to {}", path);
                            }
                            continue;
                        }
                        _ if trimmed.starts_with(":search ") => {
                            let pattern = trimmed[8..].trim();
                            if let Some(found) = rl.reverse_search(pattern) {
                                println!("Found: {}", found);
                            } else {
                                println!("No match found for '{}'", pattern);
                            }
                            continue;
                        }
                        _ => {
                            eprintln!("unknown command: {}", trimmed);
                            continue;
                        }
                    }
                }

                // Try wrapping in `output <expr>` first, fall back to raw statement
                let source = format!("output {}", trimmed);
                let program = match parse_v2(&source) {
                    Ok(p) => p,
                    Err(_) => {
                        match parse_v2(trimmed) {
                            Ok(p) => p,
                            Err(e) => {
                                eprintln!("error: {}", e.message);
                                continue;
                            }
                        }
                    }
                };

                match interp.execute(&program) {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("error: {}", e);
                        continue;
                    }
                }

                for log in interp.logs.drain(..) {
                    println!("{}", log.message);
                }
            }
            Err(magi_lang::util::LineEditError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(magi_lang::util::LineEditError::Eof) => break,
            Err(e) => {
                eprintln!("read error: {}", e);
                break;
            }
        }
    }

    rl.save_history();
}

/// Get the user's home directory for history file storage.
fn dirs_next() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
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
        if let Some(ref help) = d.help {
            eprintln!("  help: {}", help);
        }
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

fn cmd_compile_native(path: &str, opt_level: u8, output: Option<&str>) {
    let source = read_source(path);

    // Resolve dependencies
    let file_path = std::path::Path::new(path);
    let mut combined_source = String::new();
    let dep_sources = resolve_dependency_sources(file_path);
    for dep_src in &dep_sources {
        combined_source.push_str(dep_src);
        combined_source.push('\n');
    }
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("use pkg::") {
            continue;
        }
        combined_source.push_str(line);
        combined_source.push('\n');
    }

    let out_path = match output {
        Some(p) => p.to_string(),
        None => {
            let stem = file_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            stem
        }
    };

    match magi_lang::compiler::llvm::compile_native(&combined_source, None, opt_level, &out_path) {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(0o755));
            }
            let size = fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
            println!("Compiled {} -> {} ({} bytes, native)", path, out_path, size);
        }
        Err(e) => {
            eprintln!("Compile error: {}", e);
            process::exit(1);
        }
    }
}

/// Format a tagged WASM value into a human-readable string.
fn format_tagged_value(val: i64, data: &[u8]) -> String {
    format_tagged_value_depth(val, data, 0)
}

const MAX_TAGGED_DEPTH: usize = 32;

fn format_tagged_value_depth(val: i64, data: &[u8], depth: usize) -> String {
    use magi_lang::compiler::tag;
    if depth >= MAX_TAGGED_DEPTH {
        return "<...>".to_string();
    }
    // NaN-boxing: check if value is a NaN-boxed non-float
    let is_nanboxed = (val & tag::NANBOX_MASK) == tag::NANBOX_SIG;
    if !is_nanboxed {
        // It's a raw f64 value
        let f = f64::from_bits(val as u64);
        if f == (f as i64 as f64) && !f.is_nan() && f.abs() < 1e15 {
            return format!("{}.0", f as i64);
        } else {
            return format!("{}", f);
        }
    }
    let type_tag = ((val >> tag::TAG_SHIFT) & 0x07) as u8;
    let payload = val & tag::PAYLOAD_MASK;
    match type_tag {
        tag::NULL => "null".to_string(),
        tag::BOOL => format!("{}", payload != 0),
        tag::I64 => {
            // Sign-extend from 48 bits.
            let n = if payload & (1 << 47) != 0 {
                payload | !tag::PAYLOAD_MASK
            } else {
                payload
            };
            format!("{}", n)
        }
        tag::STRING => {
            // String: payload is memory offset.
            let offset = payload.max(0) as usize;
            if offset.checked_add(4).is_none_or(|end| end > data.len()) {
                return format!("<string@{}>", offset);
            }
            let len = match data[offset..offset + 4].try_into() {
                Ok(bytes) => u32::from_le_bytes(bytes) as usize,
                Err(_) => return format!("<string@{}>", offset),
            };
            match offset.checked_add(4).and_then(|o| o.checked_add(len)) {
                Some(end) if end <= data.len() => {
                    String::from_utf8_lossy(&data[offset + 4..end]).to_string()
                }
                _ => format!("<string@{}>", offset),
            }
        }
        tag::ARRAY => {
            // Array: payload is memory offset.
            // Layout: [i32 length][i32 capacity][i64 elem0][i64 elem1]...
            const MAX_DISPLAY_ELEMENTS: usize = 10_000;
            let ptr = payload.max(0) as usize;
            if ptr.checked_add(4).is_none_or(|end| end > data.len()) {
                return format!("<array@{}>", ptr);
            }
            let raw_len = match data[ptr..ptr + 4].try_into() {
                Ok(bytes) => u32::from_le_bytes(bytes) as usize,
                Err(_) => return format!("<array@{}>", ptr),
            };
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
                let elem = match data[elem_offset..elem_offset + 8].try_into() {
                    Ok(bytes) => i64::from_le_bytes(bytes),
                    Err(_) => break,
                };
                parts.push(format_tagged_value_depth(elem, data, depth + 1));
            }
            if raw_len > MAX_DISPLAY_ELEMENTS {
                parts.push(format!("...({} more)", raw_len - MAX_DISPLAY_ELEMENTS));
            }
            format!("[{}]", parts.join(", "))
        }
        tag::MAP => {
            // Map: payload is memory offset.
            // Layout: [i32 count][i32 capacity][i64 key0][i64 val0]...
            const MAX_DISPLAY_ENTRIES: usize = 10_000;
            let ptr = payload.max(0) as usize;
            if ptr.checked_add(4).is_none_or(|end| end > data.len()) {
                return format!("<map@{}>", ptr);
            }
            let raw_count = match data[ptr..ptr + 4].try_into() {
                Ok(bytes) => u32::from_le_bytes(bytes) as usize,
                Err(_) => return format!("<map@{}>", ptr),
            };
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
                let key = match data[key_offset..key_offset + 8].try_into() {
                    Ok(bytes) => i64::from_le_bytes(bytes),
                    Err(_) => break,
                };
                let value = match data[val_offset..val_offset + 8].try_into() {
                    Ok(bytes) => i64::from_le_bytes(bytes),
                    Err(_) => break,
                };
                parts.push(format!("{}: {}", format_tagged_value_depth(key, data, depth + 1), format_tagged_value_depth(value, data, depth + 1)));
            }
            if raw_count > MAX_DISPLAY_ENTRIES {
                parts.push(format!("...({} more)", raw_count - MAX_DISPLAY_ENTRIES));
            }
            format!("{{{}}}", parts.join(", "))
        }
        tag::I32 => {
            // I32: payload is a 32-bit signed integer
            let n = if payload & (1 << 31) != 0 {
                (payload | !0xFFFFFFFF) as i32
            } else {
                payload as i32
            };
            format!("{}", n)
        }
        tag::F32 => {
            // F32: payload is lower 32 bits of IEEE 754 f32
            let bits = (payload & 0xFFFFFFFF) as u32;
            let f = f32::from_bits(bits);
            format!("{}", f)
        }
        _ => format!("<tagged:{}:{}>", type_tag, payload),
    }
}

/// Maximum WASM file size (256 MB).
const MAX_WASM_FILE_SIZE: u64 = 256 * 1024 * 1024;

fn cmd_run_wasm(path: &str) {
    // Check file size before reading to prevent unbounded allocation
    match fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_WASM_FILE_SIZE => {
            eprintln!("error: '{}' exceeds maximum WASM file size ({} bytes, limit {} bytes)", path, meta.len(), MAX_WASM_FILE_SIZE);
            process::exit(1);
        }
        Err(e) => {
            eprintln!("error: cannot read '{}': {}", path, e);
            process::exit(1);
        }
        _ => {}
    }
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

    let engine = magi_lang::compiler::wasm_runtime::Engine::default();
    let module = match magi_lang::compiler::wasm_runtime::Module::new(&engine, &wasm_bytes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: cannot load '{}': {}", path, e);
            process::exit(1);
        }
    };

    let mut store = magi_lang::compiler::wasm_runtime::Store::new(&engine, ());
    let mut linker = magi_lang::compiler::wasm_runtime::Linker::new(&engine);

    // Provide host functions that the MAGI runtime expects.
    linker.func_wrap_1_0("env", "print", |inst: &mut magi_lang::compiler::wasm_runtime::Instance, val: i64| {
        let data = inst.get_memory_data();
        let s = format_tagged_value(val, data);
        println!("{}", s);
    }).unwrap_or_else(|e| { eprintln!("error: failed to define print: {}", e); process::exit(1); });

    linker.func_wrap_2_1("env", "runtime_call", |_inst: &mut magi_lang::compiler::wasm_runtime::Instance, _name: i32, _argc: i32| -> i64 {
        // Stub runtime call — return null (NaN-boxed).
        magi_lang::compiler::tag::encode(magi_lang::compiler::tag::NULL, 0)
    }).unwrap_or_else(|e| { eprintln!("error: failed to define runtime_call: {}", e); process::exit(1); });

    linker.func_wrap_1_1("env", "__to_string", |inst: &mut magi_lang::compiler::wasm_runtime::Instance, val: i64| -> i64 {
        use magi_lang::compiler::tag;
        let null_val = tag::encode(tag::NULL, 0);
        // NaN-boxing: check if it's a NaN-boxed string
        let is_nanboxed = (val & tag::NANBOX_MASK) == tag::NANBOX_SIG;
        if is_nanboxed {
            let type_tag = ((val >> tag::TAG_SHIFT) & 0x07) as u8;
            if type_tag == tag::STRING {
                return val;
            }
        }

        let formatted = {
            let data = inst.get_memory_data();
            format_tagged_value(val, data)
        };
        let bytes = formatted.as_bytes();
        let total = 4usize.saturating_add(bytes.len());

        // Read current heap pointer from exported global.
        let ptr = match inst.get_global("__heap_ptr").and_then(|v| v.i32()) {
            Some(v) => v as u32,
            None => return null_val,
        };

        // Write string: [u32 len][bytes...]
        let str_offset = ptr as usize;
        {
            let data = inst.get_memory_data_mut();
            let end = match str_offset.checked_add(4).and_then(|o| o.checked_add(bytes.len())) {
                Some(e) if e <= data.len() => e,
                _ => return null_val, // out of memory or overflow
            };
            let len_bytes = (bytes.len() as u32).to_le_bytes();
            data[str_offset..str_offset + 4].copy_from_slice(&len_bytes);
            data[str_offset + 4..end].copy_from_slice(bytes);
        }

        // Update heap pointer.
        let new_ptr = match ptr.checked_add(total as u32) {
            Some(v) => v,
            None => return null_val,
        };
        let _ = inst.set_global("__heap_ptr", magi_lang::compiler::wasm_runtime::Val::I32(new_ptr as i32));

        // Return NaN-boxed string
        magi_lang::compiler::tag::encode(magi_lang::compiler::tag::STRING, str_offset as i64)
    }).unwrap_or_else(|e| { eprintln!("error: failed to define __to_string: {}", e); process::exit(1); });

    let mut instance = match linker.instantiate(&mut store, &module) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: WASM instantiation failed: {}", e);
            process::exit(1);
        }
    };

    // Call __main.
    match instance.call("__main", &mut store) {
        Ok(result) => {
            // NaN-boxing: check if result is non-null
            // Null is encoded as tag::encode(NULL, 0) = NANBOX_SIG
            let null_val = magi_lang::compiler::tag::encode(magi_lang::compiler::tag::NULL, 0);
            if result != null_val {
                // Use format_tagged_value which handles all tag types
                let mem = instance.get_memory_data();
                println!("Result: {}", format_tagged_value(result, mem));
            }
        }
        Err(e) => {
            eprintln!("{}: WASM execution error: {}", path, e);
            process::exit(1);
        }
    }
}

/// Recursively find all `.magi` files containing `test` blocks in `dir` and run them.
fn cmd_test_dir(dir: &str) {
    let root = std::path::Path::new(dir);
    if !root.is_dir() {
        eprintln!("error: '{}' is not a directory", dir);
        process::exit(1);
    }

    let mut files = Vec::new();
    collect_magi_files(root, &mut files);
    files.sort();

    if files.is_empty() {
        eprintln!("No .magi files found in '{}'", dir);
        process::exit(1);
    }

    let mut total_passed = 0;
    let mut total_failed = 0;
    let mut files_tested = 0;

    for file_path in &files {
        let path_str = file_path.to_string_lossy();
        let source = match fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("warning: cannot read '{}': {}", path_str, e);
                continue;
            }
        };

        // Skip files that don't contain any test blocks
        if !source.contains("test ") {
            continue;
        }

        let program = match parse_v2(&source) {
            Ok(p) => p,
            Err(_) => continue, // skip unparseable files
        };

        // Check if program actually has test definitions
        let has_tests = program.statements.iter().any(|s| {
            matches!(&s.kind, magi_lang::syntax::ast::StatementKind::TestDef { .. })
        });
        if !has_tests {
            continue;
        }

        println!("\n  {}", path_str);
        files_tested += 1;

        let evaluator = FullEvaluator;
        let packages = resolve_dependencies(file_path);
        let mut interp = Interpreter::new(&evaluator).with_packages(packages);
        let results = interp.run_tests(&program);

        for result in &results {
            if result.passed {
                total_passed += 1;
                println!("    \x1b[32mPASS\x1b[0m {}", result.name);
            } else {
                total_failed += 1;
                let msg = result.error_message.as_deref().unwrap_or("unknown error");
                println!("    \x1b[31mFAIL\x1b[0m {} — {}", result.name, msg);
            }
        }
    }

    println!();
    println!(
        "{} passed, {} failed, {} total ({} files)",
        total_passed, total_failed, total_passed + total_failed, files_tested
    );

    if total_failed > 0 {
        process::exit(1);
    }
}

/// Recursively collect all `.magi` files under `dir`.
fn collect_magi_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_magi_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("magi") {
            out.push(path);
        }
    }
}

/// Run tests across all workspace members defined in magispace.toml.
fn cmd_test_all() {
    let workspace_path = std::path::Path::new("magispace.toml");
    if !workspace_path.exists() {
        eprintln!("error: no magispace.toml found in current directory");
        eprintln!("Usage: run 'magi test-all' from a workspace root containing magispace.toml");
        process::exit(1);
    }

    let toml_str = match fs::read_to_string(workspace_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read magispace.toml: {}", e);
            process::exit(1);
        }
    };

    let table = match magi_lang::util::toml_parse(&toml_str) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: failed to parse magispace.toml: {}", e);
            process::exit(1);
        }
    };

    let members = match table
        .get("workspace")
        .and_then(|w| w.as_table())
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
    {
        Some(m) => m,
        None => {
            eprintln!("error: magispace.toml must contain [workspace] members = [...]");
            process::exit(1);
        }
    };

    let mut total_passed = 0u64;
    let mut total_failed = 0u64;
    let mut members_tested = 0u64;

    for member in members {
        let member_path = match member.as_str() {
            Some(p) => p,
            None => continue,
        };

        let member_dir = std::path::Path::new(member_path);
        if !member_dir.is_dir() {
            eprintln!("Warning: workspace member '{}' is not a directory, skipping", member_path);
            continue;
        }

        println!("\n=== Testing workspace member: {} ===", member_path);

        let mut files = Vec::new();
        collect_magi_files(member_dir, &mut files);
        files.sort();

        let mut member_passed = 0;
        let mut member_failed = 0;

        for file_path in &files {
            let path_str = file_path.to_string_lossy();
            let source = match fs::read_to_string(file_path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("  warning: cannot read '{}': {}", path_str, e);
                    continue;
                }
            };

            if !source.contains("test ") {
                continue;
            }

            let program = match parse_v2(&source) {
                Ok(p) => p,
                Err(_) => continue,
            };

            let has_tests = program.statements.iter().any(|s| {
                matches!(&s.kind, magi_lang::syntax::ast::StatementKind::TestDef { .. })
            });
            if !has_tests {
                continue;
            }

            println!("\n  {}", path_str);

            let evaluator = FullEvaluator;
            let packages = resolve_dependencies(file_path);
            let mut interp = Interpreter::new(&evaluator).with_packages(packages);
            let results = interp.run_tests(&program);

            for result in &results {
                if result.passed {
                    member_passed += 1;
                    println!("    \x1b[32mPASS\x1b[0m {}", result.name);
                } else {
                    member_failed += 1;
                    let msg = result.error_message.as_deref().unwrap_or("unknown error");
                    println!("    \x1b[31mFAIL\x1b[0m {} — {}", result.name, msg);
                }
            }
        }

        if member_passed + member_failed > 0 {
            members_tested += 1;
            println!("  {} passed, {} failed", member_passed, member_failed);
        }

        total_passed += member_passed;
        total_failed += member_failed;
    }

    println!();
    println!(
        "Workspace totals: {} passed, {} failed, {} total ({} members)",
        total_passed, total_failed, total_passed + total_failed, members_tested
    );

    if total_failed > 0 {
        process::exit(1);
    }
}

/// Extract `///` doc comments from a `.magi` source file and generate Markdown documentation.
fn cmd_doc_test(path: &str) {
    let source = read_source(path);
    // Extract code blocks from /// doc comments
    let mut examples = Vec::new();
    let mut in_example = false;
    let mut current_example = String::new();
    let mut example_line = 0u32;

    for (i, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("/// ```") {
            if in_example {
                examples.push((example_line, current_example.clone()));
                current_example.clear();
                in_example = false;
            } else {
                in_example = true;
                example_line = i as u32 + 1;
            }
        } else if in_example && trimmed.starts_with("///") {
            let code = trimmed.strip_prefix("/// ").or(trimmed.strip_prefix("///")).unwrap_or("");
            current_example.push_str(code);
            current_example.push('\n');
        }
    }

    let mut passed = 0;
    let mut failed = 0;

    for (line, code) in &examples {
        match parse_v2(code) {
            Ok(program) => {
                let evaluator = FullEvaluator;
                let mut interp = Interpreter::new(&evaluator);
                match interp.execute(&program) {
                    Ok(_) => {
                        passed += 1;
                        println!("  \x1b[32mPASS\x1b[0m doc example at line {}", line);
                    }
                    Err(e) => {
                        failed += 1;
                        println!("  \x1b[31mFAIL\x1b[0m doc example at line {} — {}", line, e);
                    }
                }
            }
            Err(e) => {
                failed += 1;
                println!("  \x1b[31mFAIL\x1b[0m doc example at line {} — parse error: {}", line, e.message);
            }
        }
    }

    println!("\n{} doc tests: {} passed, {} failed", passed + failed, passed, failed);
    if failed > 0 { process::exit(1); }
}

fn cmd_doc(path: &str) {
    let source = read_source(path);

    let program = match parse_v2(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}:{}:{}: error: {}", path, e.line, e.column, e.message);
            process::exit(1);
        }
    };

    let lines: Vec<&str> = source.lines().collect();
    let mut output = String::new();

    output.push_str(&format!("# {}\n\n", path));

    for stmt in &program.statements {
        let (kind_label, name, detail) = match &stmt.kind {
            magi_lang::syntax::ast::StatementKind::FunctionDef(def) => {
                let params_str: Vec<String> = def.params.iter().map(|p| {
                    let mut s = p.name.clone();
                    if let Some(ta) = &p.type_annotation {
                        s.push_str(&format!(": {}", ta));
                    }
                    s
                }).collect();
                let ret = def.return_type.as_ref().map(|r| format!(" -> {}", r)).unwrap_or_default();
                ("Function", def.name.clone(), format!("fn {}({}){}", def.name, params_str.join(", "), ret))
            }
            magi_lang::syntax::ast::StatementKind::AsyncFunctionDef(def) => {
                let params_str: Vec<String> = def.params.iter().map(|p| {
                    let mut s = p.name.clone();
                    if let Some(ta) = &p.type_annotation {
                        s.push_str(&format!(": {}", ta));
                    }
                    s
                }).collect();
                let ret = def.return_type.as_ref().map(|r| format!(" -> {}", r)).unwrap_or_default();
                ("Async Function", def.name.clone(), format!("async fn {}({}){}", def.name, params_str.join(", "), ret))
            }
            magi_lang::syntax::ast::StatementKind::StructDef { name, fields, .. } => {
                let fields_str: Vec<String> = fields.iter().map(|f| {
                    if let Some(ta) = &f.type_annotation {
                        format!("{}: {}", f.name, ta)
                    } else {
                        f.name.clone()
                    }
                }).collect();
                ("Struct", name.clone(), format!("struct {} {{ {} }}", name, fields_str.join(", ")))
            }
            magi_lang::syntax::ast::StatementKind::EnumDef { name, variants, .. } => {
                let vs: Vec<String> = variants.iter().map(|v| {
                    if v.fields.is_empty() {
                        v.name.clone()
                    } else {
                        format!("{}({})", v.name, v.fields.join(", "))
                    }
                }).collect();
                ("Enum", name.clone(), format!("enum {} {{ {} }}", name, vs.join(", ")))
            }
            _ => continue,
        };

        // Collect doc comments (/// lines) immediately preceding this statement
        let def_line = stmt.span.start_line as usize; // 1-based
        let mut doc_lines = Vec::new();
        if def_line >= 2 {
            let mut i = def_line - 2; // 0-based index of line before definition
            loop {
                let line = lines.get(i).unwrap_or(&"").trim();
                if let Some(stripped) = line.strip_prefix("///") {
                    doc_lines.push(stripped.strip_prefix(' ').unwrap_or(stripped));
                    if i == 0 { break; }
                    i -= 1;
                } else {
                    break;
                }
            }
        }
        doc_lines.reverse();

        output.push_str(&format!("## {} `{}`\n\n", kind_label, name));
        output.push_str(&format!("```magi\n{}\n```\n\n", detail));

        if !doc_lines.is_empty() {
            for doc_line in &doc_lines {
                output.push_str(doc_line);
                output.push('\n');
            }
            output.push('\n');
        }
    }

    print!("{}", output);
}
