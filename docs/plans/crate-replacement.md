# Plan B: Replace External Crates with Own Implementations

## Goal
Replace third-party Rust crate dependencies with MAGI's own implementations to:
- Reduce binary size
- Remove external supply chain risk
- Deepen understanding and control of the runtime
- Move toward self-hosting capability

## Current Dependencies: 47 crates

### PHASE 1: Easy Replacements (< 100 lines each)

| Crate | Purpose | Lines to Replace | Priority |
|-------|---------|-----------------|----------|
| `hex` | Hex encode/decode | ~30 | High |
| `slug` | URL slug generation | ~20 | High |
| `html-escape` | HTML entity escaping | ~40 | High |
| `percent-encoding` | URL percent encoding | ~50 | High |
| `data-encoding` | Base32/Base64 encoding | ~80 | High |
| `heck` | Case conversion (snake_case, PascalCase) | ~60 | High |
| `ordered-float` | OrderedFloat wrapper | ~30 | High |
| `strsim` | Levenshtein distance | ~40 | High |
| `crc32fast` | CRC32 checksum | ~50 | Medium |
| `glob` | File glob patterns | ~80 | Medium |

**Subtotal: 10 crates, ~480 lines**

### PHASE 2: Medium Replacements (100-500 lines each)

| Crate | Purpose | Lines to Replace | Priority |
|-------|---------|-----------------|----------|
| `base64` | Base64 encode/decode | ~120 | High |
| `semver` | Semantic versioning | ~150 | High |
| `textwrap` | Text wrapping/filling | ~100 | Medium |
| `httparse` | HTTP request/response parsing | ~300 | Medium |
| `http` | HTTP types (Method, StatusCode, etc.) | ~200 | Medium |
| `csv` | CSV parsing/writing | ~400 | Medium |
| `uuid` | UUID v4 generation | ~80 (just v4 random) | High |
| `subtle` | Constant-time comparison | ~20 | High |
| `hmac` | HMAC computation | ~100 | Medium |
| `md-5` | MD5 hash | ~200 | Low |

**Subtotal: 10 crates, ~1,670 lines**

### PHASE 3: Large Replacements (500-2000 lines each)

| Crate | Purpose | Lines to Replace | Priority |
|-------|---------|-----------------|----------|
| `sha2` | SHA-256/SHA-512 | ~500 | Medium |
| `toml` | TOML parsing | ~800 | Medium |
| `url` | URL parsing (RFC 3986) | ~600 | Medium |
| `regex` | Regular expressions | ~5,000+ | Low (keep crate) |
| `serde_json` | JSON parsing/serialization | ~1,500 | Low (complex) |
| `serde_yaml_ng` | YAML parsing | ~2,000+ | Low (keep crate) |
| `lz4_flex` | LZ4 compression | ~800 | Low |
| `zstd` | Zstandard compression | ~native C binding | Skip (keep crate) |
| `chrono` | Date/time handling | ~1,000 | Medium |
| `rand` | Random number generation | ~300 (just basic RNG) | Medium |

**Subtotal: 10 crates, ~12,500 lines (only ~4,000 for Medium+ priority)**

### PHASE 4: Infrastructure Crates (keep or strategically replace)

| Crate | Purpose | Recommendation |
|-------|---------|---------------|
| `serde` | Serialization framework | **KEEP** — foundational, used everywhere |
| `indexmap` | Ordered HashMap | **KEEP** — complex, well-optimized |
| `tokio` | Async runtime (LSP) | **KEEP** — required by tower-lsp |
| `tower-lsp` | LSP protocol | **KEEP** — protocol implementation |
| `wasm-encoder` | WASM binary encoding | **KEEP** — WASM spec compliance |
| `wasmtime` | WASM execution | **KEEP** — complex runtime |
| `wasmparser` | WASM validation | **KEEP** — spec compliance |
| `tungstenite` | WebSocket client | **KEEP** — protocol complexity |
| `native-tls` | TLS/SSL | **KEEP** — security critical |
| `ureq` | HTTP client | **KEEP** — TLS integration |
| `rcgen` | Certificate generation | **KEEP** — crypto complexity |
| `x509-parser` | Certificate parsing | **KEEP** — ASN.1 complexity |
| `blake3` | BLAKE3 hash | **KEEP** — performance-critical, SIMD |
| `ariadne` | Error rendering | Could replace (~500 lines) |
| `rustyline` | REPL readline | Could replace (~300 lines) |
| `tracing` | Logging framework | Could replace (~100 lines) |
| `thiserror` | Error derive macro | Could replace (~50 lines) |

### Implementation Order

1. **Phase 1 first** — quick wins, reduce 10 dependencies immediately
2. **Phase 2 second** — medium effort, high value (base64, semver, uuid, hmac)
3. **Phase 3 selectively** — only sha2, toml, chrono, rand (skip regex, JSON, YAML)
4. **Phase 4 never** — these crates provide critical infrastructure

### Expected Results

| Metric | Before | After Phase 1 | After Phase 2 | After Phase 3 |
|--------|--------|---------------|---------------|---------------|
| Crate count | 47 | 37 | 27 | ~23 |
| Binary size | 22MB | ~20MB | ~18MB | ~16MB |
| New code | 0 | ~480 lines | ~2,150 lines | ~6,150 lines |
| Compile time | ~3min | ~2.5min | ~2min | ~1.5min |

### Crates to NEVER Replace
- `regex` — 50K+ lines, years of optimization, security hardening
- `serde`/`serde_json` — foundational serialization, used by 50+ downstream crates
- `tokio` — async runtime, required for LSP
- `wasmtime` — WASM execution engine, millions of lines
- `wasm-encoder`/`wasmparser` — WASM spec compliance
- `tungstenite`/`native-tls`/`ureq` — network protocol + TLS security
- `blake3` — SIMD-optimized, security critical
