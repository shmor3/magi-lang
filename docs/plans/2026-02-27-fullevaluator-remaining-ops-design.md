# FullEvaluator: Remaining 40 Operations

## Summary

Complete the CLI FullEvaluator by implementing all 40 remaining operations, bringing coverage from 334/374 (89.3%) to 374/374 (100%). Port implementations from the working magi-api reference code.

## Operations by Category

### Tier 1: Stateless (18 ops)

**HTTP Client (8)**: HttpGet, HttpPost, HttpPut, HttpDelete, HttpRequest, HttpHead, HttpOptions, HttpPatch
- Dependency: `ureq = "3"`
- Pattern: URL validation, sync HTTP call, return String or Map with status/body/headers
- Reference: `magi-api/src/graph/eval/net_ops.rs`

**Compression (4)**: CompressZstd, DecompressZstd, CompressLz4, DecompressLz4
- Dependencies: `zstd = "0.13"`, `lz4_flex = "0.11"`
- Pattern: input bytes/string -> compressed Bytes, with 64MB decompression bomb limit
- Reference: `magi-api/src/graph/eval/compress_ops.rs`

**Certificate/TLS (6)**: CertGenerate, CertParse, CertInfo, CertVerify, KeyGenerate, CertSelfSigned
- Dependencies: `rcgen = "0.13"`, `x509-parser = "0.16"`, `chrono = "0.4"`
- Pattern: Generate self-signed certs, parse PEM, verify validity dates
- Reference: `magi-api/src/graph/eval/cert_ops.rs`

### Tier 2: Stateful (22 ops)

All stateful ops need a connection registry (global Mutex<HashMap>) and UUID for connection IDs.
- Dependencies: `uuid = { version = "1", features = ["v4"] }`

**TCP (7)**: TcpConnect, TcpWrite, TcpRead, TcpClose, TcpBind, TcpAccept, TcpServerClose
- Uses std::net::TcpStream/TcpListener (blocking, sync)
- Store connections in registry, return conn_id strings

**UDP (4)**: UdpBind, UdpSendTo, UdpRecvFrom, UdpClose
- Uses std::net::UdpSocket (blocking, sync)

**WebSocket (4)**: WsConnect, WsSend, WsReceive, WsClose
- Dependency: `tungstenite = "0.26"`
- Sync WebSocket client via tungstenite

**SSE (3)**: SseConnect, SseReadEvent, SseClose
- Uses ureq for HTTP streaming, buffered reader for event parsing

**HTTP Server (4)**: HttpServerStart, HttpServerReceive, HttpServerRespond, HttpServerStop
- Uses std::net::TcpListener with manual HTTP/1.1 parsing
- Minimal implementation sufficient for scripting use cases

## Architecture

All 40 ops implemented directly in `src/bin/magi.rs` FullEvaluator match arms. No new modules — keeps the single-file pattern consistent with existing ops.

Connection registry: simple module-level `LazyLock<Mutex<HashMap<String, Box<dyn Any + Send>>>>` with store/get/remove helpers, similar to magi-api's `connection_registry.rs`.

## Dependencies to Add

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

## Testing

- Unit tests for each operation category within `src/bin/magi.rs` (or integration tests)
- HTTP tests can use a simple local server or mock patterns
- Compression tests: round-trip compress/decompress
- Cert tests: generate + parse + verify chain
- TCP/UDP: bind + connect + write + read + close lifecycle
- WS/SSE: connect + send/read + close (may need test servers)
