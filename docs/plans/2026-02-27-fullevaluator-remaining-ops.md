# FullEvaluator: Remaining 40 Operations Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Complete the CLI FullEvaluator by implementing all 40 remaining operations (HTTP client, compression, certificates, TCP, UDP, WebSocket, SSE, HTTP server), bringing coverage from 334/374 to 374/374.

**Architecture:** Port implementations from the working magi-api reference code in `magi-api/src/graph/eval/`. The magi-lang FullEvaluator in `src/bin/magi.rs` uses the same `HashMap<String, DataType>` input pattern. Add a connection registry module for stateful ops. All new code goes into `src/bin/magi.rs` (match arms) plus a small connection registry helper at the top of the file.

**Tech Stack:** `ureq` (HTTP), `zstd`/`lz4_flex` (compression), `rcgen`/`x509-parser`/`chrono` (certs), `tungstenite` (WebSocket), `uuid` (connection IDs), `std::net` (TCP/UDP)

---

## Task 1: Add dependencies to Cargo.toml

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add all required dependencies**

Add to the `[dependencies]` section of `Cargo.toml`:

```toml
ureq = "3"
zstd = "0.13"
lz4_flex = "0.11"
rcgen = "0.13"
x509-parser = "0.16"
chrono = "0.4"
uuid = { version = "1", features = ["v4"] }
tungstenite = "0.26"
```

**Step 2: Verify it compiles**

Run: `cargo check`
Expected: Compiles with no errors (warnings OK during transition).

**Step 3: Commit**

```
feat: add dependencies for remaining FullEvaluator operations
```

---

## Task 2: Add connection registry and helper functions

**Files:**
- Modify: `src/bin/magi.rs` (add module-level code before the `FullEvaluator` struct)

**Step 1: Add imports and connection registry**

At the top of `src/bin/magi.rs`, after the existing `use` statements (line ~12), add:

```rust
use std::any::Any;
use std::sync::{LazyLock, Mutex};

// Connection registry for stateful network operations (TCP, UDP, WS, SSE, HTTP server).
static CONNECTIONS: LazyLock<Mutex<HashMap<String, Box<dyn Any + Send>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn conn_store<T: Send + 'static>(id: &str, conn: T) {
    let mut map = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    map.insert(id.to_string(), Box::new(conn));
}

fn conn_get<T: Send + 'static>(id: &str) -> Result<*mut T, EvalError> {
    let mut map = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let entry = map
        .get_mut(id)
        .ok_or_else(|| EvalError::InvalidInput(format!("Connection not found: {}", id)))?;
    entry
        .downcast_mut::<T>()
        .map(|r| r as *mut T)
        .ok_or_else(|| EvalError::InvalidInput(format!("Connection type mismatch: {}", id)))
}

fn conn_remove(id: &str) -> Result<(), EvalError> {
    let mut map = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    map.remove(id)
        .ok_or_else(|| EvalError::InvalidInput(format!("Connection not found: {}", id)))?;
    Ok(())
}

fn conn_id(prefix: &str) -> String {
    format!("{}:{}", prefix, uuid::Uuid::new_v4())
}
```

Also add SSRF-protection helper functions:

```rust
fn is_blocked_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_private() || v4.is_loopback() || v4.is_link_local()
            || v4.is_unspecified() || v4.is_broadcast()
            || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
        }
        std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

fn validate_url(url: &str) -> Result<(), EvalError> {
    let host = if let Some(rest) = url.strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .or_else(|| url.strip_prefix("ws://"))
        .or_else(|| url.strip_prefix("wss://"))
    {
        rest.split('/').next().unwrap_or(rest)
            .split(':').next().unwrap_or(rest)
    } else {
        return Err(EvalError::InvalidInput(
            "URL must use http://, https://, ws://, or wss:// scheme".into(),
        ));
    };
    validate_host(host)
}

fn validate_host(host: &str) -> Result<(), EvalError> {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(EvalError::InvalidInput(format!(
                "Access to private/internal address {} is blocked", ip
            )));
        }
    }
    let lower = host.to_lowercase();
    if lower == "localhost"
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
        || lower == "metadata.google.internal"
    {
        return Err(EvalError::InvalidInput(format!(
            "Access to hostname '{}' is blocked", host
        )));
    }
    Ok(())
}

fn get_port(inputs: &HashMap<String, DataType>, key: &str) -> Result<u16, EvalError> {
    let val = inputs.get(key).cloned().unwrap_or(DataType::Null);
    let port_raw = match &val {
        DataType::Int64(n) => *n,
        DataType::Int32(n) => *n as i64,
        DataType::Uint32(n) => *n as i64,
        DataType::Uint64(n) => *n as i64,
        DataType::Float64(f) => *f as i64,
        DataType::Float32(f) => *f as i64,
        _ => return Err(EvalError::InvalidInput(format!("Expected numeric port, got {}", val.type_name()))),
    };
    u16::try_from(port_raw)
        .map_err(|_| EvalError::InvalidInput(format!("Invalid port: {}", port_raw)))
}

fn get_string<'a>(inputs: &'a HashMap<String, DataType>, key: &str) -> Result<&'a str, EvalError> {
    match inputs.get(key) {
        Some(DataType::String(s)) => Ok(s.as_str()),
        Some(other) => Err(EvalError::InvalidInput(format!(
            "Expected string for '{}', got {}", key, other.type_name()
        ))),
        None => Err(EvalError::InvalidInput(format!("Missing input: {}", key))),
    }
}

fn data_to_bytes(data: &DataType) -> Vec<u8> {
    match data {
        DataType::Bytes(b) => b.clone(),
        DataType::String(s) => s.as_bytes().to_vec(),
        other => other.to_string().into_bytes(),
    }
}
```

**Step 2: Verify it compiles**

Run: `cargo check`
Expected: Compiles (helpers are unused for now, may get warnings).

**Step 3: Commit**

```
feat: add connection registry and network helpers for FullEvaluator
```

---

## Task 3: Implement HTTP client operations (8 ops)

**Files:**
- Modify: `src/bin/magi.rs` (add match arms before the `other =>` catch-all at line ~4298)
- Test: `tests/integration.rs`

**Step 1: Write integration tests**

Add to `tests/integration.rs`:

```rust
#[test]
fn test_http_get_invalid_url() {
    let result = run_eval("std::http_get(\"not-a-url\")");
    assert!(result.contains("error") || result.contains("Error"));
}

#[test]
fn test_http_get_blocked_localhost() {
    let result = run_eval("std::http_get(\"http://localhost:9999/\")");
    assert!(result.contains("blocked") || result.contains("error") || result.contains("Error"));
}

#[test]
fn test_http_request_bad_method() {
    let result = run_eval("std::http_request(\"INVALID\", \"http://example.com\", \"\", {})");
    assert!(result.contains("error") || result.contains("Error"));
}
```

**Step 2: Implement HTTP client match arms**

Insert before the `other =>` catch-all in the FullEvaluator match block:

```rust
// ================================================================
// HTTP Client operations
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
    let headers_map = inputs.get("headers").and_then(|d| {
        if let DataType::Map(m) = d { Some(m) } else { None }
    });
    let payload = inputs.get("body").map(|d| d.to_string());
    let method_upper = method.to_uppercase();
    let resp = match method_upper.as_str() {
        "POST" | "PUT" | "PATCH" => {
            let req = match method_upper.as_str() {
                "POST" => ureq::post(url),
                "PUT" => ureq::put(url),
                _ => ureq::patch(url),
            };
            let req = if let Some(hdrs) = headers_map {
                hdrs.iter().fold(req, |r, (k, v)| r.header(k.as_str(), &v.to_string()))
            } else { req };
            req.send(payload.as_deref().unwrap_or("").as_bytes())
                .map_err(|e| EvalError::InvalidInput(format!("http_request: {}", e)))?
        }
        "GET" | "DELETE" | "HEAD" | "OPTIONS" => {
            let req = match method_upper.as_str() {
                "DELETE" => ureq::delete(url),
                "HEAD" => ureq::head(url),
                "OPTIONS" => { let a = ureq::Agent::new_with_defaults(); a.options(url) }
                _ => ureq::get(url),
            };
            let req = if let Some(hdrs) = headers_map {
                hdrs.iter().fold(req, |r, (k, v)| r.header(k.as_str(), &v.to_string()))
            } else { req };
            req.call()
                .map_err(|e| EvalError::InvalidInput(format!("http_request: {}", e)))?
        }
        other => return Err(EvalError::InvalidInput(format!("Unsupported HTTP method: {}", other))),
    };
    let status = resp.status().as_u16();
    let body: String = resp.into_body().read_to_string()
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
    let headers: std::collections::BTreeMap<String, DataType> = resp.headers().keys()
        .map(|name| {
            let value = resp.headers().get(name)
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
    let resp = agent.options(url)
        .call()
        .map_err(|e| EvalError::InvalidInput(format!("http_options: {}", e)))?;
    let status = resp.status().as_u16();
    let headers: std::collections::BTreeMap<String, DataType> = resp.headers().keys()
        .map(|name| {
            let value = resp.headers().get(name)
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
```

Add `use std::io::Read as _;` at top for `.read_to_string()`.

**Step 3: Run tests**

Run: `cargo test`
Expected: All 2086+ tests pass (new tests included).

**Step 4: Commit**

```
feat: implement 8 HTTP client operations in FullEvaluator
```

---

## Task 4: Implement compression operations (4 ops)

**Files:**
- Modify: `src/bin/magi.rs`
- Test: `tests/integration.rs`

**Step 1: Write integration tests**

```rust
#[test]
fn test_compress_zstd_roundtrip() {
    let result = run_eval(r#"
        let data = "hello world repeated many times for compression"
        let compressed = std::compress_zstd(data)
        let decompressed = std::decompress_zstd(compressed)
        std::from_bytes(decompressed)
    "#);
    assert!(result.contains("hello world"));
}

#[test]
fn test_compress_lz4_roundtrip() {
    let result = run_eval(r#"
        let data = "lz4 compression test data"
        let compressed = std::compress_lz4(data)
        let decompressed = std::decompress_lz4(compressed)
        std::from_bytes(decompressed)
    "#);
    assert!(result.contains("lz4 compression"));
}
```

**Step 2: Implement compression match arms**

```rust
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
```

**Step 3: Run tests and commit**

Run: `cargo test`

```
feat: implement 4 compression operations in FullEvaluator
```

---

## Task 5: Implement certificate/TLS operations (6 ops)

**Files:**
- Modify: `src/bin/magi.rs`
- Test: `tests/integration.rs`

**Step 1: Write integration tests**

```rust
#[test]
fn test_key_generate() {
    let result = run_eval("let k = std::key_generate(); typeof(k)");
    assert!(result.contains("map") || result.contains("Map"));
}

#[test]
fn test_cert_generate_and_parse() {
    let result = run_eval(r#"
        let c = std::cert_generate("test.example.com")
        let info = std::cert_parse(c.cert_pem)
        info.subject
    "#);
    assert!(result.contains("test.example.com"));
}
```

**Step 2: Implement certificate match arms**

```rust
// ================================================================
// Certificate / TLS operations
// ================================================================
OperationType::CertGenerate | OperationType::CertSelfSigned => {
    let cn = get_string(inputs, "cn")?;
    let params = rcgen::CertificateParams::new(vec![cn.to_string()])
        .map_err(|e| EvalError::InvalidInput(format!("cert_generate: {}", e)))?;
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
```

**Step 3: Run tests and commit**

Run: `cargo test`

```
feat: implement 6 certificate/TLS operations in FullEvaluator
```

---

## Task 6: Implement TCP operations (7 ops)

**Files:**
- Modify: `src/bin/magi.rs`
- Test: `tests/integration.rs`

**Step 1: Write integration tests**

```rust
#[test]
fn test_tcp_bind_and_close() {
    // Bind to ephemeral port and close
    let result = run_eval(r#"
        let listener = std::tcp_bind("127.0.0.1", 0)
        std::tcp_server_close(listener)
        "closed"
    "#);
    assert!(result.contains("closed"));
}

#[test]
fn test_tcp_connect_refused() {
    let result = run_eval(r#"std::tcp_connect("127.0.0.1", 1)"#);
    assert!(result.contains("error") || result.contains("Error"));
}
```

**Step 2: Implement TCP match arms**

```rust
// ================================================================
// TCP operations
// ================================================================
OperationType::TcpConnect => {
    let host = get_string(inputs, "host")?;
    validate_host(host)?;
    let port = get_port(inputs, "port")?;
    let addr = format!("{}:{}", host, port);
    let sock_addr: std::net::SocketAddr = addr.parse()
        .map_err(|e| EvalError::InvalidInput(format!("Invalid address: {}", e)))?;
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
    let conn_id_str = get_string(inputs, "conn_id")?;
    let data = inputs.get("data").cloned().unwrap_or(DataType::Null);
    let ptr = conn_get::<Mutex<std::net::TcpStream>>(conn_id_str)?;
    let stream_mutex = unsafe { &*ptr };
    let mut stream = stream_mutex.lock()
        .map_err(|_| EvalError::InvalidInput("tcp connection lock poisoned".into()))?;
    let bytes = data_to_bytes(&data);
    use std::io::Write;
    let written = stream.write(&bytes)
        .map_err(|e| EvalError::InvalidInput(format!("tcp_write: {}", e)))?;
    stream.flush()
        .map_err(|e| EvalError::InvalidInput(format!("tcp_write flush: {}", e)))?;
    Ok(DataType::Int64(written as i64))
}
OperationType::TcpRead => {
    let conn_id_str = get_string(inputs, "conn_id")?;
    let ptr = conn_get::<Mutex<std::net::TcpStream>>(conn_id_str)?;
    let stream_mutex = unsafe { &*ptr };
    let mut stream = stream_mutex.lock()
        .map_err(|_| EvalError::InvalidInput("tcp connection lock poisoned".into()))?;
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf)
        .map_err(|e| EvalError::InvalidInput(format!("tcp_read: {}", e)))?;
    buf.truncate(n);
    Ok(DataType::Bytes(buf))
}
OperationType::TcpClose => {
    let conn_id_str = get_string(inputs, "conn_id")?;
    conn_remove(conn_id_str)?;
    Ok(DataType::Null)
}
OperationType::TcpBind => {
    let address = get_string(inputs, "address")?;
    let port = get_port(inputs, "port")?;
    let addr = format!("{}:{}", address, port);
    let listener = std::net::TcpListener::bind(&addr)
        .map_err(|e| EvalError::InvalidInput(format!("tcp_bind: {}", e)))?;
    let id = conn_id("tcp-listener");
    conn_store(&id, Mutex::new(listener));
    Ok(DataType::String(id))
}
OperationType::TcpAccept => {
    let listener_id = get_string(inputs, "listener_id")?;
    let ptr = conn_get::<Mutex<std::net::TcpListener>>(listener_id)?;
    let listener_mutex = unsafe { &*ptr };
    let listener = listener_mutex.lock()
        .map_err(|_| EvalError::InvalidInput("tcp listener lock poisoned".into()))?;
    listener.set_nonblocking(true)
        .map_err(|e| EvalError::InvalidInput(format!("tcp_accept: {}", e)))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(30000);
    let (stream, addr) = loop {
        match listener.accept() {
            Ok(result) => break result,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    listener.set_nonblocking(false).ok();
                    return Err(EvalError::InvalidInput("tcp_accept: timed out".into()));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => {
                listener.set_nonblocking(false).ok();
                return Err(EvalError::InvalidInput(format!("tcp_accept: {}", e)));
            }
        }
    };
    listener.set_nonblocking(false).ok();
    stream.set_nonblocking(false).ok();
    let id = conn_id("tcp");
    conn_store(&id, Mutex::new(stream));
    Ok(DataType::Map(std::collections::BTreeMap::from([
        ("conn_id".into(), DataType::String(id)),
        ("address".into(), DataType::String(addr.to_string())),
    ])))
}
OperationType::TcpServerClose => {
    let listener_id = get_string(inputs, "listener_id")?;
    conn_remove(listener_id)?;
    Ok(DataType::Null)
}
```

**Step 3: Run tests and commit**

Run: `cargo test`

```
feat: implement 7 TCP operations in FullEvaluator
```

---

## Task 7: Implement UDP operations (4 ops)

**Files:**
- Modify: `src/bin/magi.rs`
- Test: `tests/integration.rs`

**Step 1: Write integration tests**

```rust
#[test]
fn test_udp_bind_and_close() {
    let result = run_eval(r#"
        let sock = std::udp_bind("127.0.0.1", 0)
        std::udp_close(sock)
        "closed"
    "#);
    assert!(result.contains("closed"));
}
```

**Step 2: Implement UDP match arms**

```rust
// ================================================================
// UDP operations
// ================================================================
OperationType::UdpBind => {
    let address = get_string(inputs, "address")?;
    let port = get_port(inputs, "port")?;
    let addr = format!("{}:{}", address, port);
    let socket = std::net::UdpSocket::bind(&addr)
        .map_err(|e| EvalError::InvalidInput(format!("udp_bind: {}", e)))?;
    let id = conn_id("udp");
    conn_store(&id, Mutex::new(socket));
    Ok(DataType::String(id))
}
OperationType::UdpSendTo => {
    let socket_id = get_string(inputs, "socket_id")?;
    let data = inputs.get("data").cloned().unwrap_or(DataType::Null);
    let address = get_string(inputs, "address")?;
    let port = get_port(inputs, "port")?;
    let target = format!("{}:{}", address, port);
    let ptr = conn_get::<Mutex<std::net::UdpSocket>>(socket_id)?;
    let socket_mutex = unsafe { &*ptr };
    let socket = socket_mutex.lock()
        .map_err(|_| EvalError::InvalidInput("udp socket lock poisoned".into()))?;
    let bytes = data_to_bytes(&data);
    let sent = socket.send_to(&bytes, &target)
        .map_err(|e| EvalError::InvalidInput(format!("udp_send_to: {}", e)))?;
    Ok(DataType::Int64(sent as i64))
}
OperationType::UdpRecvFrom => {
    let socket_id = get_string(inputs, "socket_id")?;
    let ptr = conn_get::<Mutex<std::net::UdpSocket>>(socket_id)?;
    let socket_mutex = unsafe { &*ptr };
    let socket = socket_mutex.lock()
        .map_err(|_| EvalError::InvalidInput("udp socket lock poisoned".into()))?;
    socket.set_read_timeout(Some(std::time::Duration::from_millis(30000)))
        .map_err(|e| EvalError::InvalidInput(format!("udp set_read_timeout: {}", e)))?;
    let mut buf = vec![0u8; 4096];
    let (n, addr) = socket.recv_from(&mut buf)
        .map_err(|e| EvalError::InvalidInput(format!("udp_recv_from: {}", e)))?;
    buf.truncate(n);
    Ok(DataType::Map(std::collections::BTreeMap::from([
        ("data".into(), DataType::Bytes(buf)),
        ("address".into(), DataType::String(addr.ip().to_string())),
        ("port".into(), DataType::Int64(addr.port() as i64)),
    ])))
}
OperationType::UdpClose => {
    let socket_id = get_string(inputs, "socket_id")?;
    conn_remove(socket_id)?;
    Ok(DataType::Null)
}
```

**Step 3: Run tests and commit**

Run: `cargo test`

```
feat: implement 4 UDP operations in FullEvaluator
```

---

## Task 8: Implement WebSocket operations (4 ops)

**Files:**
- Modify: `src/bin/magi.rs`
- Test: `tests/integration.rs`

**Step 1: Write integration tests**

```rust
#[test]
fn test_ws_connect_invalid() {
    let result = run_eval(r#"std::ws_connect("not-a-url")"#);
    assert!(result.contains("error") || result.contains("Error"));
}

#[test]
fn test_ws_close_nonexistent() {
    let result = run_eval(r#"std::ws_close("fake-id")"#);
    assert!(result.contains("not found") || result.contains("Error"));
}
```

**Step 2: Implement WebSocket match arms**

```rust
// ================================================================
// WebSocket operations
// ================================================================
OperationType::WsConnect => {
    let url = get_string(inputs, "url")?;
    validate_url(url)?;
    let (socket, _response) = tungstenite::connect(url)
        .map_err(|e| EvalError::InvalidInput(format!("ws_connect: {}", e)))?;
    let id = conn_id("ws");
    conn_store(&id, Mutex::new(socket));
    Ok(DataType::String(id))
}
OperationType::WsSend => {
    let conn_id_str = get_string(inputs, "conn_id")?;
    let message = inputs.get("message").cloned().unwrap_or(DataType::Null);
    let ptr = conn_get::<Mutex<tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>>>(conn_id_str)?;
    let ws_mutex = unsafe { &*ptr };
    let mut ws = ws_mutex.lock()
        .map_err(|_| EvalError::InvalidInput("websocket lock poisoned".into()))?;
    let msg = match &message {
        DataType::Bytes(b) => tungstenite::Message::Binary(b.clone().into()),
        other => tungstenite::Message::Text(other.to_string().into()),
    };
    ws.send(msg)
        .map_err(|e| EvalError::InvalidInput(format!("ws_send: {}", e)))?;
    Ok(DataType::Null)
}
OperationType::WsReceive => {
    let conn_id_str = get_string(inputs, "conn_id")?;
    let ptr = conn_get::<Mutex<tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>>>(conn_id_str)?;
    let ws_mutex = unsafe { &*ptr };
    let mut ws = ws_mutex.lock()
        .map_err(|_| EvalError::InvalidInput("websocket lock poisoned".into()))?;
    let msg = ws.read()
        .map_err(|e| EvalError::InvalidInput(format!("ws_receive: {}", e)))?;
    match msg {
        tungstenite::Message::Text(t) => Ok(DataType::String(t.to_string())),
        tungstenite::Message::Binary(b) => Ok(DataType::Bytes(b.to_vec())),
        tungstenite::Message::Close(_) => Ok(DataType::Null),
        _ => Ok(DataType::Null),
    }
}
OperationType::WsClose => {
    let conn_id_str = get_string(inputs, "conn_id")?;
    {
        let ptr = conn_get::<Mutex<tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>>>(conn_id_str)?;
        let ws_mutex = unsafe { &*ptr };
        let mut ws = ws_mutex.lock()
            .map_err(|_| EvalError::InvalidInput("websocket lock poisoned".into()))?;
        let _ = ws.close(None);
    }
    conn_remove(conn_id_str)?;
    Ok(DataType::Null)
}
```

**Step 3: Run tests and commit**

Run: `cargo test`

```
feat: implement 4 WebSocket operations in FullEvaluator
```

---

## Task 9: Implement SSE operations (3 ops)

**Files:**
- Modify: `src/bin/magi.rs`
- Test: `tests/integration.rs`

**Step 1: Write integration tests**

```rust
#[test]
fn test_sse_connect_invalid() {
    let result = run_eval(r#"std::sse_connect("not-a-url")"#);
    assert!(result.contains("error") || result.contains("Error"));
}
```

**Step 2: Implement SSE match arms**

```rust
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
    let conn_id_str = get_string(inputs, "conn_id")?;
    let ptr = conn_get::<Mutex<Box<dyn std::io::BufRead + Send>>>(conn_id_str)?;
    let reader_mutex = unsafe { &*ptr };
    let mut reader = reader_mutex.lock()
        .map_err(|_| EvalError::InvalidInput("sse connection lock poisoned".into()))?;
    let mut event_type = String::new();
    let mut data_lines = Vec::new();
    let mut event_id = String::new();
    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line)
            .map_err(|e| EvalError::InvalidInput(format!("sse_read_event: {}", e)))?;
        if bytes_read == 0 { return Ok(DataType::Null); }
        let line = line.trim_end();
        if line.is_empty() {
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
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        } else if let Some(rest) = line.strip_prefix("event:") {
            event_type = rest.trim_start().to_string();
        } else if let Some(rest) = line.strip_prefix("id:") {
            event_id = rest.trim_start().to_string();
        }
    }
}
OperationType::SseClose => {
    let conn_id_str = get_string(inputs, "conn_id")?;
    conn_remove(conn_id_str)?;
    Ok(DataType::Null)
}
```

**Step 3: Run tests and commit**

Run: `cargo test`

```
feat: implement 3 SSE operations in FullEvaluator
```

---

## Task 10: Implement HTTP Server operations (4 ops)

**Files:**
- Modify: `src/bin/magi.rs`
- Test: `tests/integration.rs`

**Step 1: Write integration tests**

```rust
#[test]
fn test_http_server_start_and_stop() {
    let result = run_eval(r#"
        let server = std::http_server_start("127.0.0.1", 0)
        std::http_server_stop(server)
        "stopped"
    "#);
    assert!(result.contains("stopped"));
}
```

**Step 2: Implement HTTP Server match arms**

```rust
// ================================================================
// HTTP Server operations
// ================================================================
OperationType::HttpServerStart => {
    let address = get_string(inputs, "address")?;
    let port = get_port(inputs, "port")?;
    let addr = format!("{}:{}", address, port);
    let listener = std::net::TcpListener::bind(&addr)
        .map_err(|e| EvalError::InvalidInput(format!("http_server_start: {}", e)))?;
    let id = conn_id("http-server");
    conn_store(&id, Mutex::new(listener));
    Ok(DataType::String(id))
}
OperationType::HttpServerReceive => {
    let server_id = get_string(inputs, "server_id")?;
    let ptr = conn_get::<Mutex<std::net::TcpListener>>(server_id)?;
    let listener_mutex = unsafe { &*ptr };
    let listener = listener_mutex.lock()
        .map_err(|_| EvalError::InvalidInput("http server lock poisoned".into()))?;
    let (stream, addr) = listener.accept()
        .map_err(|e| EvalError::InvalidInput(format!("http_server_receive: {}", e)))?;
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
        let line = line.trim().to_string();
        if line.is_empty() { break; }
        if let Some((key, value)) = line.split_once(':') {
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
    } else {
        String::new()
    };
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
    let client_id = get_string(inputs, "client_id")?;
    let status = match inputs.get("status") {
        Some(DataType::Int64(n)) => *n,
        Some(DataType::Int32(n)) => *n as i64,
        _ => 200,
    };
    let body = inputs.get("body").map(|d| d.to_string()).unwrap_or_default();
    let ptr = conn_get::<Mutex<std::net::TcpStream>>(client_id)?;
    let stream_mutex = unsafe { &*ptr };
    let mut stream = stream_mutex.lock()
        .map_err(|_| EvalError::InvalidInput("http connection lock poisoned".into()))?;
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
    use std::io::Write;
    stream.write_all(response.as_bytes())
        .map_err(|e| EvalError::InvalidInput(format!("http_server_respond: {}", e)))?;
    stream.flush()
        .map_err(|e| EvalError::InvalidInput(format!("http_server_respond: {}", e)))?;
    drop(stream);
    conn_remove(client_id)?;
    Ok(DataType::Null)
}
OperationType::HttpServerStop => {
    let server_id = get_string(inputs, "server_id")?;
    conn_remove(server_id)?;
    Ok(DataType::Null)
}
```

**Step 3: Run tests and commit**

Run: `cargo test`

```
feat: implement 4 HTTP Server operations in FullEvaluator
```

---

## Task 11: Remove catch-all error, run full test suite, final commit

**Files:**
- Modify: `src/bin/magi.rs` (update the `other =>` catch-all comment)

**Step 1: Update catch-all**

Change the comment above the `other =>` arm from "Remaining operations that require external dependencies" to "All 374 operations are now handled above":

```rust
// All 374 operations are now handled above.
// This catch-all is unreachable for known OperationType variants.
other => Err(EvalError::InvalidInput(format!(
    "operation '{:?}' is not implemented in the standalone evaluator",
    other,
))),
```

**Step 2: Run full test suite**

Run: `cargo test`
Expected: All tests pass (2086+ original + new integration tests).

Run: `cargo clippy`
Expected: 0 warnings.

**Step 3: Commit**

```
feat: complete FullEvaluator with all 374 operations implemented
```

---

## Task 12: Update memory and verify

**Step 1: Update MEMORY.md**

Update the round notes to reflect 374/374 operations complete. Remove "FullEvaluator missing" from known deferred issues.

**Step 2: Final verification**

Run: `cargo test 2>&1 | tail -5`
Verify: All tests pass, zero failures.
