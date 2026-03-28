# Phase 1: Core Types + Utilities — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the foundational type system and utility library for the self-hosted MAGI compiler in MAGI itself.

**Architecture:** All code lives in `self/`. Stage 0 (`magi run`) executes it. Types mirror `src/types/mod.rs`. Utilities mirror `src/util.rs`. Every function is tested via `magi test`.

**Tech Stack:** MAGI (executed by stage 0 binary), `magi run` for execution, `magi test` for testing.

**Bootstrap:** `magi run self/types.magi` must work. `magi test self/tests/test_types.magi` must pass.

---

## File Structure

```
self/
├── types.magi           # DataType enum, OrderedMap, Span, error types
├── util.magi            # String algorithms, encoding, JSON, regex
├── tests/
│   ├── test_types.magi  # Unit tests for types
│   └── test_util.magi   # Unit tests for utilities
```

---

### Task 1: DataType Enum

**Files:**
- Create: `self/types.magi`
- Test: `self/tests/test_types.magi`

- [ ] **Step 1: Write the failing test**

```magi
// self/tests/test_types.magi
use std::test::*;

fn test_datatype_null() {
    let v = DataType::Null;
    assert(v == DataType::Null);
    output "PASS: null";
}

fn test_datatype_int() {
    let v = DataType::Int64(42);
    match v {
        DataType::Int64(n) => assert(n == 42),
        _ => assert(false),
    }
    output "PASS: int64";
}

fn test_datatype_string() {
    let v = DataType::String("hello");
    match v {
        DataType::String(s) => assert(s == "hello"),
        _ => assert(false),
    }
    output "PASS: string";
}

fn test_datatype_bool() {
    let v = DataType::Bool(true);
    match v {
        DataType::Bool(b) => assert(b == true),
        _ => assert(false),
    }
    output "PASS: bool";
}

fn test_datatype_float() {
    let v = DataType::Float64(3.14);
    match v {
        DataType::Float64(f) => assert(f > 3.0 && f < 4.0),
        _ => assert(false),
    }
    output "PASS: float64";
}

fn test_datatype_array() {
    let v = DataType::Array([DataType::Int64(1), DataType::Int64(2)]);
    match v {
        DataType::Array(arr) => assert(len(arr) == 2),
        _ => assert(false),
    }
    output "PASS: array";
}

fn test_datatype_map() {
    let v = DataType::Map({"key": DataType::String("val")});
    match v {
        DataType::Map(m) => assert(m.key == DataType::String("val")),
        _ => assert(false),
    }
    output "PASS: map";
}

test_datatype_null();
test_datatype_int();
test_datatype_string();
test_datatype_bool();
test_datatype_float();
test_datatype_array();
test_datatype_map();
output "All DataType tests passed";
```

- [ ] **Step 2: Run test to verify it fails**

Run: `magi run self/tests/test_types.magi`
Expected: FAIL — `DataType` not defined yet

- [ ] **Step 3: Write the DataType enum**

```magi
// self/types.magi — Core types for self-hosted MAGI compiler

// DataType — the universal value type
// Mirrors src/types/mod.rs DataType enum
enum DataType {
    Null,
    Bool(bool),
    Int64(int),
    Float64(float),
    Int32(int),
    Uint32(int),
    Uint64(int),
    Float32(float),
    String(string),
    Bytes([int]),
    Array([DataType]),
    Map({string: DataType}),
    Set([DataType]),
    Tuple([DataType]),
    Future(FutureState),
}

enum FutureState {
    Pending(string),
    Resolved(DataType),
    Rejected(string),
}

// Type name for display
fn datatype_name(val) {
    match val {
        DataType::Null => "null",
        DataType::Bool(_) => "bool",
        DataType::Int64(_) => "int",
        DataType::Float64(_) => "float",
        DataType::String(_) => "string",
        DataType::Bytes(_) => "bytes",
        DataType::Array(_) => "array",
        DataType::Map(_) => "map",
        DataType::Set(_) => "set",
        DataType::Tuple(_) => "tuple",
        _ => "unknown",
    }
}

// Truthiness check (matches interpreter behavior)
fn is_truthy(val) {
    match val {
        DataType::Null => false,
        DataType::Bool(b) => b,
        DataType::Int64(n) => n != 0,
        DataType::Float64(f) => f != 0.0,
        DataType::String(s) => len(s) > 0,
        DataType::Array(a) => len(a) > 0,
        DataType::Map(m) => len(keys(m)) > 0,
        _ => true,
    }
}

// Convert DataType to display string
fn datatype_to_string(val) {
    match val {
        DataType::Null => "null",
        DataType::Bool(b) => to_string(b),
        DataType::Int64(n) => to_string(n),
        DataType::Float64(f) => to_string(f),
        DataType::String(s) => s,
        DataType::Array(a) => f"[{a.map(|v| datatype_to_string(v)).join(\", \")}]",
        DataType::Map(m) => {
            let pairs = keys(m).map(|k| f"\"{k}\": {datatype_to_string(m[k])}");
            f"{{{pairs.join(\", \")}}}"
        },
        _ => "<value>",
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `magi run self/tests/test_types.magi`
Expected: All 7 tests PASS

- [ ] **Step 5: Commit**

```bash
git add self/types.magi self/tests/test_types.magi
git commit -m "self-hosting phase 1: DataType enum with tests"
git push origin main
```

---

### Task 2: Span and Error Types

**Files:**
- Modify: `self/types.magi`
- Test: `self/tests/test_types.magi`

- [ ] **Step 1: Write the failing test**

Append to `self/tests/test_types.magi`:

```magi
fn test_span() {
    let s = Span { start_line: 1, start_col: 5, end_line: 1, end_col: 10, start_byte: 4, end_byte: 9, tail_call: false };
    assert(s.start_line == 1);
    assert(s.start_col == 5);
    assert(s.end_line == 1);
    assert(s.end_col == 10);
    output "PASS: span";
}

fn test_span_merge() {
    let a = Span { start_line: 1, start_col: 1, end_line: 1, end_col: 5, start_byte: 0, end_byte: 4, tail_call: false };
    let b = Span { start_line: 1, start_col: 10, end_line: 1, end_col: 15, start_byte: 9, end_byte: 14, tail_call: false };
    let merged = span_merge(a, b);
    assert(merged.start_col == 1);
    assert(merged.end_col == 15);
    output "PASS: span merge";
}

fn test_compile_error() {
    let err = CompileError::Error { line: 5, col: 10, message: "unexpected token" };
    match err {
        CompileError::Error { line, col, message } => {
            assert(line == 5);
            assert(message == "unexpected token");
        },
        _ => assert(false),
    }
    output "PASS: compile error";
}

test_span();
test_span_merge();
test_compile_error();
```

- [ ] **Step 2: Run test to verify it fails**

Run: `magi run self/tests/test_types.magi`
Expected: FAIL — `Span` not defined

- [ ] **Step 3: Add Span and error types to types.magi**

Append to `self/types.magi`:

```magi
// Source location span
struct Span {
    start_line: int,
    start_col: int,
    end_line: int,
    end_col: int,
    start_byte: int,
    end_byte: int,
    tail_call: bool,
}

fn span_new(line, col) {
    Span { start_line: line, start_col: col, end_line: line, end_col: col, start_byte: 0, end_byte: 0, tail_call: false }
}

fn span_merge(a, b) {
    Span {
        start_line: if a.start_line < b.start_line { a.start_line } else { b.start_line },
        start_col: if a.start_line < b.start_line { a.start_col } else if a.start_line == b.start_line { if a.start_col < b.start_col { a.start_col } else { b.start_col } } else { b.start_col },
        end_line: if a.end_line > b.end_line { a.end_line } else { b.end_line },
        end_col: if a.end_line > b.end_line { a.end_col } else if a.end_line == b.end_line { if a.end_col > b.end_col { a.end_col } else { b.end_col } } else { b.end_col },
        start_byte: if a.start_byte < b.start_byte { a.start_byte } else { b.start_byte },
        end_byte: if a.end_byte > b.end_byte { a.end_byte } else { b.end_byte },
        tail_call: false,
    }
}

// Compilation errors
enum CompileError {
    Error { line: int, col: int, message: string },
    Unsupported(string),
    Internal(string),
}

fn compile_error_at(line, col, msg) {
    CompileError::Error { line: line, col: col, message: msg }
}

fn compile_error_display(err) {
    match err {
        CompileError::Error { line, col, message } => f"error at {line}:{col}: {message}",
        CompileError::Unsupported(msg) => f"unsupported: {msg}",
        CompileError::Internal(msg) => f"internal error: {msg}",
    }
}

// Interpreter errors
enum InterpError {
    TypeError { expected: string, actual: string, context: string, span: Span },
    ArityMismatch { name: string, expected: string, actual: int, span: Span },
    UndefinedVariable { name: string, span: Span },
    UndefinedFunction { name: string, span: Span },
    EvalError { message: string, span: Span },
    BreakSignal(DataType),
    ContinueSignal,
    ReturnSignal(DataType),
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `magi run self/tests/test_types.magi`
Expected: All tests PASS

- [ ] **Step 5: Commit**

```bash
git add self/types.magi self/tests/test_types.magi
git commit -m "self-hosting phase 1: Span, CompileError, InterpError types"
git push origin main
```

---

### Task 3: OrderedMap

**Files:**
- Modify: `self/types.magi`
- Test: `self/tests/test_types.magi`

- [ ] **Step 1: Write the failing test**

Append to `self/tests/test_types.magi`:

```magi
fn test_ordered_map_new() {
    let m = ordered_map_new();
    assert(ordered_map_len(m) == 0);
    output "PASS: ordered_map new";
}

fn test_ordered_map_insert_get() {
    let mut m = ordered_map_new();
    m = ordered_map_insert(m, "a", 1);
    m = ordered_map_insert(m, "b", 2);
    m = ordered_map_insert(m, "c", 3);
    assert(ordered_map_get(m, "a") == 1);
    assert(ordered_map_get(m, "b") == 2);
    assert(ordered_map_get(m, "c") == 3);
    assert(ordered_map_len(m) == 3);
    output "PASS: ordered_map insert/get";
}

fn test_ordered_map_preserves_order() {
    let mut m = ordered_map_new();
    m = ordered_map_insert(m, "z", 1);
    m = ordered_map_insert(m, "a", 2);
    m = ordered_map_insert(m, "m", 3);
    let k = ordered_map_keys(m);
    assert(k[0] == "z");
    assert(k[1] == "a");
    assert(k[2] == "m");
    output "PASS: ordered_map preserves insertion order";
}

fn test_ordered_map_update() {
    let mut m = ordered_map_new();
    m = ordered_map_insert(m, "key", "old");
    m = ordered_map_insert(m, "key", "new");
    assert(ordered_map_get(m, "key") == "new");
    assert(ordered_map_len(m) == 1);
    output "PASS: ordered_map update";
}

fn test_ordered_map_remove() {
    let mut m = ordered_map_new();
    m = ordered_map_insert(m, "a", 1);
    m = ordered_map_insert(m, "b", 2);
    m = ordered_map_remove(m, "a");
    assert(ordered_map_len(m) == 1);
    assert(ordered_map_get(m, "b") == 2);
    assert(ordered_map_has(m, "a") == false);
    output "PASS: ordered_map remove";
}

test_ordered_map_new();
test_ordered_map_insert_get();
test_ordered_map_preserves_order();
test_ordered_map_update();
test_ordered_map_remove();
```

- [ ] **Step 2: Run test to verify it fails**

Run: `magi run self/tests/test_types.magi`
Expected: FAIL — `ordered_map_new` not defined

- [ ] **Step 3: Implement OrderedMap**

Append to `self/types.magi`:

```magi
// OrderedMap — insertion-order preserving map
// Backed by parallel arrays of keys and values.
// This matches MAGI's native Map behavior.

fn ordered_map_new() {
    { "__keys": [], "__values": [] }
}

fn ordered_map_insert(m, key, value) {
    let mut ks = m.__keys;
    let mut vs = m.__values;
    // Check if key exists — update in place
    for (let mut i = 0; i < len(ks); i += 1) {
        if ks[i] == key {
            vs[i] = value;
            return { "__keys": ks, "__values": vs };
        }
    }
    // New key — append
    ks.push(key);
    vs.push(value);
    { "__keys": ks, "__values": vs }
}

fn ordered_map_get(m, key) {
    let ks = m.__keys;
    let vs = m.__values;
    for (let mut i = 0; i < len(ks); i += 1) {
        if ks[i] == key { return vs[i]; }
    }
    null
}

fn ordered_map_has(m, key) {
    for k in m.__keys {
        if k == key { return true; }
    }
    false
}

fn ordered_map_remove(m, key) {
    let mut ks = [];
    let mut vs = [];
    for (let mut i = 0; i < len(m.__keys); i += 1) {
        if m.__keys[i] != key {
            ks.push(m.__keys[i]);
            vs.push(m.__values[i]);
        }
    }
    { "__keys": ks, "__values": vs }
}

fn ordered_map_keys(m) { m.__keys }
fn ordered_map_values(m) { m.__values }
fn ordered_map_len(m) { len(m.__keys) }

fn ordered_map_entries(m) {
    let mut result = [];
    for (let mut i = 0; i < len(m.__keys); i += 1) {
        result.push([m.__keys[i], m.__values[i]]);
    }
    result
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `magi run self/tests/test_types.magi`
Expected: All tests PASS

- [ ] **Step 5: Commit**

```bash
git add self/types.magi self/tests/test_types.magi
git commit -m "self-hosting phase 1: OrderedMap with insertion-order preservation"
git push origin main
```

---

### Task 4: ChannelType and OperationType Stubs

**Files:**
- Modify: `self/types.magi`
- Test: `self/tests/test_types.magi`

- [ ] **Step 1: Write the failing test**

```magi
fn test_channel_type() {
    let ct = ChannelType::String;
    assert(ct == ChannelType::String);
    let ct2 = ChannelType::Int64;
    assert(ct2 == ChannelType::Int64);
    output "PASS: channel type";
}

fn test_operation_type_parse() {
    let op = operation_type_parse("add");
    assert(op == OperationType::Add);
    let op2 = operation_type_parse("http_get");
    assert(op2 == OperationType::HttpGet);
    let op3 = operation_type_parse("nonexistent");
    assert(op3 == null);
    output "PASS: operation type parse";
}

fn test_operation_type_as_str() {
    assert(operation_type_as_str(OperationType::Add) == "add");
    assert(operation_type_as_str(OperationType::HttpGet) == "http_get");
    output "PASS: operation type as_str";
}

test_channel_type();
test_operation_type_parse();
test_operation_type_as_str();
```

- [ ] **Step 2: Run test to verify it fails**

- [ ] **Step 3: Add ChannelType and OperationType**

Append to `self/types.magi`:

```magi
// Channel types for operation ports
enum ChannelType {
    String,
    Int32,
    Int64,
    Uint32,
    Uint64,
    Float32,
    Float64,
    Bool,
    Bytes,
    Array,
    Map,
    Null,
}

// OperationType — all 468 stdlib operations
// Full enum with parse/as_str functions
// (Only showing first 20 for brevity — full list mirrors src/types/operations.rs)
enum OperationType {
    Add, Subtract, Multiply, Divide, Modulo, Power, Sqrt, Cbrt, Hypot, Negate,
    Abs, MinVal, MaxVal, Clamp,
    Eq, NotEq, Lt, Gt, LtEq, GtEq,
    And, Or, Not, Xor,
    BitAnd, BitOr, BitXor, BitShl, BitShr, BitNot,
    // ... (all 468 variants)
    HttpGet, HttpPost, HttpPut, HttpDelete, HttpRequest, HttpHead, HttpOptions, HttpPatch,
    // ... remaining variants added incrementally
}

// Parse operation name to enum
fn operation_type_parse(name) {
    match name {
        "add" => OperationType::Add,
        "subtract" => OperationType::Subtract,
        "multiply" => OperationType::Multiply,
        "divide" => OperationType::Divide,
        "http_get" => OperationType::HttpGet,
        "http_post" => OperationType::HttpPost,
        // ... all 468 mappings
        _ => null,
    }
}

// Convert operation enum to string name
fn operation_type_as_str(op) {
    match op {
        OperationType::Add => "add",
        OperationType::Subtract => "subtract",
        OperationType::Multiply => "multiply",
        OperationType::Divide => "divide",
        OperationType::HttpGet => "http_get",
        OperationType::HttpPost => "http_post",
        // ... all 468 mappings
        _ => "unknown",
    }
}
```

Note: The full 468-variant enum and parse/as_str functions will be generated from `src/types/operations.rs` via a script to avoid manual transcription errors.

- [ ] **Step 4: Run test to verify it passes**

- [ ] **Step 5: Commit**

```bash
git add self/types.magi self/tests/test_types.magi
git commit -m "self-hosting phase 1: ChannelType, OperationType with parse/as_str"
git push origin main
```

---

### Task 5: Utility — String Operations

**Files:**
- Create: `self/util.magi`
- Test: `self/tests/test_util.magi`

- [ ] **Step 1: Write the failing test**

```magi
// self/tests/test_util.magi

fn test_hex_encode() {
    assert(hex_enc("hello") == "68656c6c6f");
    assert(hex_enc("") == "");
    assert(hex_enc("AB") == "4142");
    output "PASS: hex encode";
}

fn test_hex_decode() {
    assert(hex_dec("68656c6c6f") == "hello");
    assert(hex_dec("") == "");
    assert(hex_dec("4142") == "AB");
    output "PASS: hex decode";
}

fn test_base64_encode() {
    assert(b64_encode("hello") == "aGVsbG8=");
    assert(b64_encode("") == "");
    assert(b64_encode("a") == "YQ==");
    output "PASS: base64 encode";
}

fn test_base64_decode() {
    assert(b64_decode("aGVsbG8=") == "hello");
    assert(b64_decode("") == "");
    assert(b64_decode("YQ==") == "a");
    output "PASS: base64 decode";
}

fn test_levenshtein() {
    assert(levenshtein("kitten", "sitting") == 3);
    assert(levenshtein("", "") == 0);
    assert(levenshtein("abc", "abc") == 0);
    assert(levenshtein("abc", "") == 3);
    output "PASS: levenshtein";
}

fn test_slug() {
    assert(to_slug("Hello World!") == "hello-world");
    assert(to_slug("  foo  BAR  ") == "foo-bar");
    output "PASS: slug";
}

fn test_case_conversion() {
    assert(to_camel("hello_world") == "helloWorld");
    assert(to_snake("helloWorld") == "hello_world");
    assert(to_pascal("hello_world") == "HelloWorld");
    assert(to_kebab("helloWorld") == "hello-world");
    output "PASS: case conversion";
}

test_hex_encode();
test_hex_decode();
test_base64_encode();
test_base64_decode();
test_levenshtein();
test_slug();
test_case_conversion();
output "All util tests passed";
```

- [ ] **Step 2: Run test to verify it fails**

Run: `magi run self/tests/test_util.magi`
Expected: FAIL — functions not defined

- [ ] **Step 3: Implement string utilities**

```magi
// self/util.magi — Utility functions for self-hosted compiler

// ── Hex encoding ────────────────────────────────────────────────────

const HEX_CHARS = "0123456789abcdef";

fn hex_enc(input) {
    let mut result = string_builder_new();
    let bytes = input.bytes();
    for b in bytes {
        result = string_builder_append(result, HEX_CHARS[b / 16]);
        result = string_builder_append(result, HEX_CHARS[b % 16]);
    }
    string_builder_to_string(result)
}

fn hex_dec(input) {
    let mut result = [];
    for (let mut i = 0; i < len(input); i += 2) {
        let hi = hex_digit(input[i]);
        let lo = hex_digit(input[i + 1]);
        result.push(hi * 16 + lo);
    }
    // Convert byte array to string
    let mut s = "";
    for b in result { s = s + char_at_code(b); }
    s
}

fn hex_digit(ch) {
    match ch {
        "0" => 0, "1" => 1, "2" => 2, "3" => 3, "4" => 4,
        "5" => 5, "6" => 6, "7" => 7, "8" => 8, "9" => 9,
        "a" | "A" => 10, "b" | "B" => 11, "c" | "C" => 12,
        "d" | "D" => 13, "e" | "E" => 14, "f" | "F" => 15,
        _ => 0,
    }
}

// ── Base64 encoding ─────────────────────────────────────────────────

const B64_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(input) {
    let bytes = input.bytes();
    let mut result = string_builder_new();
    let mut i = 0;
    while i < len(bytes) {
        let b0 = bytes[i];
        let b1 = if i + 1 < len(bytes) { bytes[i + 1] } else { 0 };
        let b2 = if i + 2 < len(bytes) { bytes[i + 2] } else { 0 };
        let remaining = len(bytes) - i;

        result = string_builder_append(result, B64_CHARS[b0 / 4]);
        result = string_builder_append(result, B64_CHARS[(b0 % 4) * 16 + b1 / 16]);
        if remaining > 1 {
            result = string_builder_append(result, B64_CHARS[(b1 % 16) * 4 + b2 / 64]);
        } else {
            result = string_builder_append(result, "=");
        }
        if remaining > 2 {
            result = string_builder_append(result, B64_CHARS[b2 % 64]);
        } else {
            result = string_builder_append(result, "=");
        }
        i += 3;
    }
    string_builder_to_string(result)
}

fn b64_decode(input) {
    let mut bytes = [];
    let mut buf = [];
    for ch in input.chars() {
        if ch == "=" { break; }
        let val = b64_char_val(ch);
        buf.push(val);
        if len(buf) == 4 {
            bytes.push(buf[0] * 4 + buf[1] / 16);
            bytes.push((buf[1] % 16) * 16 + buf[2] / 4);
            bytes.push((buf[2] % 4) * 64 + buf[3]);
            buf = [];
        }
    }
    // Handle remaining
    if len(buf) == 3 {
        bytes.push(buf[0] * 4 + buf[1] / 16);
        bytes.push((buf[1] % 16) * 16 + buf[2] / 4);
    } else if len(buf) == 2 {
        bytes.push(buf[0] * 4 + buf[1] / 16);
    }
    let mut s = "";
    for b in bytes { s = s + char_at_code(b); }
    s
}

fn b64_char_val(ch) {
    if ch >= "A" && ch <= "Z" { return ch.char_code_at(0) - 65; }
    if ch >= "a" && ch <= "z" { return ch.char_code_at(0) - 97 + 26; }
    if ch >= "0" && ch <= "9" { return ch.char_code_at(0) - 48 + 52; }
    if ch == "+" { return 62; }
    if ch == "/" { return 63; }
    0
}

// ── Levenshtein distance ────────────────────────────────────────────

fn levenshtein(a, b) {
    let m = len(a);
    let n = len(b);
    if m == 0 { return n; }
    if n == 0 { return m; }

    // Build distance matrix
    let mut prev = [];
    for (let mut j = 0; j <= n; j += 1) { prev.push(j); }

    for (let mut i = 1; i <= m; i += 1) {
        let mut curr = [i];
        for (let mut j = 1; j <= n; j += 1) {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            let insert = curr[j - 1] + 1;
            let delete = prev[j] + 1;
            let replace = prev[j - 1] + cost;
            let min = if insert < delete { insert } else { delete };
            curr.push(if min < replace { min } else { replace });
        }
        prev = curr;
    }
    prev[n]
}

// ── Slug generation ─────────────────────────────────────────────────

fn to_slug(input) {
    let mut result = string_builder_new();
    let mut last_was_dash = true;
    for ch in input.trim().chars() {
        if (ch >= "a" && ch <= "z") || (ch >= "0" && ch <= "9") {
            result = string_builder_append(result, ch);
            last_was_dash = false;
        } else if ch >= "A" && ch <= "Z" {
            result = string_builder_append(result, ch.to_lower());
            last_was_dash = false;
        } else if !last_was_dash {
            result = string_builder_append(result, "-");
            last_was_dash = true;
        }
    }
    let s = string_builder_to_string(result);
    if s.ends_with("-") { s.substring(0, len(s) - 1) } else { s }
}

// ── Case conversion ─────────────────────────────────────────────────

fn to_camel(input) {
    let parts = input.split("_");
    let mut result = parts[0];
    for (let mut i = 1; i < len(parts); i += 1) {
        if len(parts[i]) > 0 {
            result = result + parts[i][0].to_upper() + parts[i].substring(1, len(parts[i]));
        }
    }
    result
}

fn to_snake(input) {
    let mut result = string_builder_new();
    for (let mut i = 0; i < len(input); i += 1) {
        let ch = input[i];
        if ch >= "A" && ch <= "Z" {
            if i > 0 { result = string_builder_append(result, "_"); }
            result = string_builder_append(result, ch.to_lower());
        } else {
            result = string_builder_append(result, ch);
        }
    }
    string_builder_to_string(result)
}

fn to_pascal(input) {
    let parts = input.split("_");
    let mut result = "";
    for part in parts {
        if len(part) > 0 {
            result = result + part[0].to_upper() + part.substring(1, len(part));
        }
    }
    result
}

fn to_kebab(input) {
    to_snake(input).replace("_", "-")
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `magi run self/tests/test_util.magi`
Expected: All tests PASS

- [ ] **Step 5: Commit**

```bash
git add self/util.magi self/tests/test_util.magi
git commit -m "self-hosting phase 1: hex, base64, levenshtein, slug, case conversion"
git push origin main
```

---

### Task 6: Utility — JSON Parser/Emitter

**Files:**
- Modify: `self/util.magi`
- Test: `self/tests/test_util.magi`

- [ ] **Step 1: Write the failing test**

Append to `self/tests/test_util.magi`:

```magi
fn test_json_stringify() {
    assert(json_stringify(42) == "42");
    assert(json_stringify("hello") == "\"hello\"");
    assert(json_stringify(true) == "true");
    assert(json_stringify(null) == "null");
    assert(json_stringify([1, 2, 3]) == "[1,2,3]");
    assert(json_stringify({"a": 1}) == "{\"a\":1}");
    output "PASS: json stringify";
}

fn test_json_parse() {
    assert(json_parse_value("42") == 42);
    assert(json_parse_value("\"hello\"") == "hello");
    assert(json_parse_value("true") == true);
    assert(json_parse_value("null") == null);
    let arr = json_parse_value("[1,2,3]");
    assert(len(arr) == 3);
    assert(arr[0] == 1);
    let obj = json_parse_value("{\"a\":1}");
    assert(obj.a == 1);
    output "PASS: json parse";
}

test_json_stringify();
test_json_parse();
```

- [ ] **Step 2: Run test to verify it fails**

- [ ] **Step 3: Implement JSON parser and emitter**

Append to `self/util.magi`:

```magi
// ── JSON stringify ──────────────────────────────────────────────────

fn json_stringify(val) {
    match typeof(val) {
        "null" => "null",
        "bool" => if val { "true" } else { "false" },
        "int" => to_string(val),
        "float" => to_string(val),
        "string" => f"\"{json_escape(val)}\"",
        "array" => f"[{val.map(|v| json_stringify(v)).join(\",\")}]",
        "map" => {
            let pairs = keys(val).map(|k| f"\"{json_escape(k)}\":{json_stringify(val[k])}");
            f"{{{pairs.join(\",\")}}}"
        },
        _ => "null",
    }
}

fn json_escape(s) {
    s.replace("\\", "\\\\")
     .replace("\"", "\\\"")
     .replace("\n", "\\n")
     .replace("\t", "\\t")
     .replace("\r", "\\r")
}

// ── JSON parse ──────────────────────────────────────────────────────

fn json_parse_value(input) {
    let trimmed = input.trim();
    if len(trimmed) == 0 { return null; }
    let ch = trimmed[0];

    if ch == "\"" { return json_parse_string(trimmed); }
    if ch == "[" { return json_parse_array(trimmed); }
    if ch == "{" { return json_parse_object(trimmed); }
    if trimmed == "true" { return true; }
    if trimmed == "false" { return false; }
    if trimmed == "null" { return null; }
    // Number
    if trimmed.contains(".") { return parse_float(trimmed); }
    parse_int(trimmed)
}

fn json_parse_string(input) {
    // Strip quotes and unescape
    let inner = input.substring(1, len(input) - 1);
    inner.replace("\\\"", "\"")
         .replace("\\\\", "\\")
         .replace("\\n", "\n")
         .replace("\\t", "\t")
         .replace("\\r", "\r")
}

fn json_parse_array(input) {
    let inner = input.substring(1, len(input) - 1).trim();
    if len(inner) == 0 { return []; }
    let parts = json_split_top_level(inner);
    parts.map(|p| json_parse_value(p))
}

fn json_parse_object(input) {
    let inner = input.substring(1, len(input) - 1).trim();
    if len(inner) == 0 { return {}; }
    let pairs = json_split_top_level(inner);
    let mut result = {};
    for pair in pairs {
        let colon_idx = json_find_colon(pair);
        let key = json_parse_value(pair.substring(0, colon_idx).trim());
        let val = json_parse_value(pair.substring(colon_idx + 1, len(pair)).trim());
        result[key] = val;
    }
    result
}

fn json_split_top_level(input) {
    let mut parts = [];
    let mut depth = 0;
    let mut in_string = false;
    let mut start = 0;
    for (let mut i = 0; i < len(input); i += 1) {
        let ch = input[i];
        if ch == "\"" && (i == 0 || input[i - 1] != "\\") { in_string = !in_string; }
        if !in_string {
            if ch == "[" || ch == "{" { depth += 1; }
            if ch == "]" || ch == "}" { depth -= 1; }
            if ch == "," && depth == 0 {
                parts.push(input.substring(start, i).trim());
                start = i + 1;
            }
        }
    }
    if start < len(input) { parts.push(input.substring(start, len(input)).trim()); }
    parts
}

fn json_find_colon(input) {
    let mut in_string = false;
    for (let mut i = 0; i < len(input); i += 1) {
        let ch = input[i];
        if ch == "\"" && (i == 0 || input[i - 1] != "\\") { in_string = !in_string; }
        if !in_string && ch == ":" { return i; }
    }
    -1
}
```

- [ ] **Step 4: Run test to verify it passes**

- [ ] **Step 5: Commit**

```bash
git add self/util.magi self/tests/test_util.magi
git commit -m "self-hosting phase 1: JSON parser and emitter"
git push origin main
```

---

### Task 7: Run All Phase 1 Tests and Validate

**Files:**
- All files in `self/`

- [ ] **Step 1: Run all Phase 1 tests**

```bash
magi run self/tests/test_types.magi
magi run self/tests/test_util.magi
```

Expected: All tests pass, output shows "All X tests passed" for both.

- [ ] **Step 2: Verify on dedicated server**

```bash
ssh dev@10.0.0.111 "source ~/.cargo/env && cd ~/magi-lang && git pull && magi run self/tests/test_types.magi && magi run self/tests/test_util.magi"
```

Expected: Same results on dedicated server.

- [ ] **Step 3: Final commit for Phase 1**

```bash
git add -A
git commit -m "self-hosting phase 1 complete: core types + utilities"
git push origin main
```

---

## Phase 1 Acceptance Criteria

1. `magi run self/types.magi` executes without error
2. `magi run self/util.magi` executes without error
3. All DataType enum variants constructable and matchable
4. OrderedMap preserves insertion order
5. Span creation and merging works
6. CompileError and InterpError types constructable
7. Hex encode/decode roundtrips correctly
8. Base64 encode/decode roundtrips correctly
9. Levenshtein distance matches known values
10. JSON parse/stringify roundtrips correctly
11. Case conversion (camel, snake, pascal, kebab) works

## Next Phase

Phase 2 (Lexer) depends on Phase 1 completion. The lexer will import `self/types.magi` for Span and Token types.
