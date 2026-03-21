# CLAUDE.md — magi-lang

## Overview

MAGI is a dynamically-typed scripting language with Rust-inspired syntax. This crate contains the complete language implementation: lexer, parser, AST, type checker, linter, formatter, interpreter, WASM compiler, LSP server, optimizer, and CLI.

## Architecture

```
Source (.magi)
    -> Lexer (src/syntax/lexer.rs)
    -> Parser (src/syntax/parser.rs)
    -> AST (src/syntax/ast.rs)
    -> Type Checker (src/syntax/type_checker.rs)
    -> Linter (src/linter/)
    -> Optimizer (src/optimizer.rs)
    -> Interpreter (src/syntax/interpreter.rs)
    -> Compiler (src/compiler/) -> WASM binary
    -> Formatter (src/formatter/mod.rs)
    -> LSP (src/lsp/)
    -> Diagnostics (src/diagnostics.rs) [ariadne-based rich errors]
```

## Essential Commands

```bash
# Build
cargo build --bin magi
cargo build --bin magi --release

# Test
cargo test --lib                    # 1386 library tests
cargo test --test integration       # 1506 integration tests (2892+ total)

# Run
cargo run --bin magi -- run file.magi       # Execute a .magi file
cargo run --bin magi -- repl                # Interactive REPL
cargo run --bin magi -- check file.magi     # Type-check without running
cargo run --bin magi -- test file.magi      # Run #[test] functions
cargo run --bin magi -- fmt --write file.magi  # Auto-format in place
cargo run --bin magi -- compile file.magi   # Compile to WASM
cargo run --bin magi -- eval '1 + 2'        # Evaluate an expression
cargo run --bin magi -- init my-project     # Scaffold a new project
cargo run --bin magi -- lsp                 # Start the LSP server
cargo run --bin magi -- doc file.magi       # Generate docs from /// comments
cargo run --bin magi -- bench file.magi     # Benchmark execution
```

## Language Features

### Core
- Constants (`const`), mutable variables (`let mut`), type annotations
- Numeric types: `int64`, `float64` with hex/octal/binary/underscore literals
- Strings: interpolation (`f"...{expr}..."`), multiline (`"""`), raw (`r"..."`)
- Structs with typed fields, default values, and struct update syntax (`...spread`)
- Enums with data variants and pattern matching
- Type aliases (`type UserId = int64`)

### Functions and Closures
- Functions with default parameters, rest parameters (`...rest`), return types
- Closures/lambdas (`|x| x * 2`), multi-line block bodies
- Higher-order functions, closures capture by value
- Spread call syntax (`fn(...args)`)
- Async functions (`async fn`), `spawn`, `await`

### Control Flow
- `if`/`else if`/`else` (expression-based)
- `match` with literal, type, range, enum, or-patterns, and guards
- `for..in` loops, C-style `for` loops (`for (let mut i = 0; i < n; i += 1)`)
- `while` loops, `do { } while` loops
- `loop` with `break` (value-returning)
- Labeled loops (`'outer: for`, `break 'outer`, `continue 'outer`)
- `defer` statements (run cleanup at scope exit)
- `try`/`catch`/`finally`, `throw`
- Try-propagate operator (`?`)

### Types
- `Set` type (`Set(1, 2, 3)`) with `contains`, `union`, `intersection`, `difference`
- `Tuple` type (`Tuple(1, "hello", true)`)
- `Optional` pattern: `Some(value)`, `None`, `is_some`, `is_none`, `unwrap`, `unwrap_or`
- `Result` pattern: `Ok(value)`, `Err(msg)`, `is_ok`, `is_err`, `unwrap`

### Object System
- Impl blocks (`impl Type { fn method(self) { ... } }`)
- Traits (`trait HasArea { fn area(self); }`)
- Trait implementation (`impl HasArea for Circle { ... }`)
- Operator overloading (`__add__`, `__sub__`, `__eq__`, etc.)
- Deprecation attributes (`#[deprecated]`)

### Concurrency
- Real concurrency via `spawn` (OS threads)
- Channels: `channel()`, `channel(capacity)` for bounded
- `chan_send`, `chan_recv`, `chan_try_recv`, `chan_close`
- Producer-consumer and fan-out/fan-in patterns

### Collections
- Array methods: `map`, `filter`, `reduce`, `find`, `any`, `all`, `sort`, `reverse`, `unique`, `flat_map`, `group_by`, `partition`, `chunk`, `enumerate`, `zip`, `take_while`, `skip_while`, `scan`, `sort_by`, `min_by`, `max_by`
- List comprehensions: `[expr for x in iter]`, `[expr for x in iter if cond]`
- Map literals, optional chaining (`?.`), null coalescing (`??`)
- Destructuring: arrays, maps, and structs in `let` and `for` bindings
- Spread in arrays (`[...a, ...b]`)

### Operators
- Pipe operator (`|>` with `_` placeholder)
- `in` operator (arrays, maps, strings)
- String repetition (`"ha" * 3`)
- Ranges: exclusive (`1..5`), inclusive (`1..=5`)
- Compound assignment (`+=`, `-=`, `*=`, `/=`, `%=`)

### Static Analysis
- Type inference and type narrowing in the type checker
- Dead code detection via linter
- Semantic error codes (SyntaxError variants)
- Full LSP integration

### Other
- Modules (`mod name { }`, `use name::*`)
- Packages (workspace project imports via `use pkg::name::*`)
- WASM compilation target

## Key Design Decisions

- **DataType::Map uses IndexMap** (insertion-order preserving, not alphabetical)
- **ChannelType::Any vs Null** -- `Any` is the universal acceptor/polymorphic type; `Null` is a concrete type
- **OperationType has 356+ variants** -- all stdlib operations are enum variants
- **Interpreter uses virtual heap** with address-based HashMap for value storage
- **FullEvaluator in magi.rs** dispatches all 356+ OperationType variants via `eval_operation`
- **Closures capture by value** (snapshot at definition time, not by reference)
- **&&/|| use truthiness** (any value, not just Bool)
- **assert() uses truthiness** (consistent with &&/||)

## File Structure

| File | Lines | Purpose |
|------|-------|---------|
| `src/syntax/interpreter.rs` | ~8870 | AST tree-walking interpreter + stdlib modules |
| `src/bin/magi.rs` | ~8630 | CLI + FullEvaluator (356+ operations) |
| `src/syntax/type_checker.rs` | ~7010 | Static analysis + type inference + diagnostics |
| `src/compiler/wasm.rs` | ~5740 | IR -> WASM binary generation |
| `src/syntax/parser.rs` | ~4610 | Recursive descent + Pratt parsing |
| `src/compiler/compile.rs` | ~3200 | AST -> IR compilation |
| `src/syntax/lexer.rs` | ~2360 | Tokenizer |
| `src/formatter/mod.rs` | ~2370 | AST pretty-printer |
| `src/types/operations.rs` | ~1810 | OperationType enum (356+ variants) |
| `src/lsp/analysis.rs` | ~1740 | LSP analysis engine |
| `src/optimizer.rs` | ~1580 | AST optimizer |
| `src/linter/mod.rs` | ~1260 | Lint engine |
| `src/linter/rules.rs` | ~1190 | Lint rules |
| `src/types/mod.rs` | ~1160 | DataType, ChannelType |
| `src/ops.rs` | ~930 | Operation dispatch |
| `src/syntax/ast.rs` | ~730 | AST node definitions |
| `tests/integration.rs` | ~20410 | Integration test suite (1506 tests) |

Total source: ~62,560 lines of Rust across 40+ files.

## Standard Library

40 modules, 356+ operations available via `use std::module::*`:

| Category | Modules |
|----------|---------|
| Math & Logic | `math`, `cmp`, `logic`, `bits` |
| Strings & Text | `str`, `text`, `fmt`, `encode` |
| Data Structures | `array`, `map`, `collections`, `sort` |
| Serialization | `json`, `yaml`, `csv`, `toml` |
| Type System | `convert`, `reflect` |
| I/O & System | `io`, `fs`, `env`, `path` |
| Networking | `net`, `tcp`, `udp`, `ws`, `sse`, `http_server` |
| Security | `hash`, `crypto`, `cert` |
| Time | `time` |
| Random | `rand` |
| Matching | `regex`, `uuid` |
| Compression | `compress` |
| Control Flow | `control` |
| Binary Data | `bytes` |
| Concurrency | `concurrent` |

## LSP Server

Full Language Server Protocol implementation in `src/lsp/`:
- Completion, hover, go-to-definition, references, rename
- Document symbols, workspace symbols, call hierarchy
- Semantic tokens, folding ranges, selection ranges
- Code actions, code lens, inlay hints, linked editing
- Signature help, document links

## Dependencies

Key crates: `indexmap` (ordered maps), `ariadne` (error rendering), `rustyline` (REPL), `wasm-encoder` (WASM codegen), `wasmtime` (optional WASM runtime), `tower-lsp` (LSP server), `ureq` (HTTP client), `regex`, `serde`/`serde_json`, `tokio`, `zstd`, `lz4_flex`

## Rules

- **NO backward compatibility** -- always write modern code
- **NEVER add AI attribution** to commits
- Maps use `IndexMap` not `BTreeMap`
- The `wasm-runtime` feature flag gates wasmtime (default enabled)
- Tests must pass: `cargo test --lib && cargo test --test integration`
