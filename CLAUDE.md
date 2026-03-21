# CLAUDE.md — magi-lang

## Overview

MAGI is a dynamically-typed scripting language with Rust-inspired syntax. This crate contains the complete language implementation: lexer, parser, AST, type checker, linter, formatter, interpreter, WASM compiler, LSP server, and CLI.

## Architecture

```
Source (.magi)
    -> Lexer (src/syntax/lexer.rs)
    -> Parser (src/syntax/parser.rs)
    -> AST (src/syntax/ast.rs)
    -> Type Checker (src/syntax/type_checker.rs)
    -> Linter (src/linter/)
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
cargo test --lib                    # 1122 library tests
cargo test --test integration       # 1378 integration tests

# Run
cargo run --bin magi -- run file.magi
cargo run --bin magi -- repl
cargo run --bin magi -- check file.magi
cargo run --bin magi -- test file.magi
cargo run --bin magi -- fmt --write file.magi
cargo run --bin magi -- compile file.magi
cargo run --bin magi -- eval '1 + 2'
cargo run --bin magi -- init my-project
cargo run --bin magi -- lsp
```

## Key Design Decisions

- **DataType::Map uses IndexMap** (insertion-order preserving, not alphabetical)
- **ChannelType::Any vs Null** — `Any` is the universal acceptor/polymorphic type; `Null` is a concrete type
- **OperationType has 374+ variants** — all stdlib operations are enum variants
- **Interpreter uses virtual heap** with address-based HashMap for value storage
- **FullEvaluator in magi.rs** is organized into 29 category `eval_*` functions
- **Closures capture by value** (snapshot at definition time, not by reference)
- **&&/|| use truthiness** (any value, not just Bool)
- **assert() uses truthiness** (consistent with &&/||)

## File Structure

| File | Lines | Purpose |
|------|-------|---------|
| `src/bin/magi.rs` | ~7800 | CLI + FullEvaluator (374 operations) |
| `src/syntax/interpreter.rs` | ~6200 | AST tree-walking interpreter |
| `src/syntax/type_checker.rs` | ~5900 | Static analysis + diagnostics |
| `src/compiler/wasm.rs` | ~5400 | IR -> WASM binary generation |
| `src/syntax/parser.rs` | ~4200 | Recursive descent + Pratt parsing |
| `src/compiler/compile.rs` | ~2700 | AST -> IR compilation |
| `src/syntax/lexer.rs` | ~2300 | Tokenizer |
| `src/formatter/mod.rs` | ~2200 | AST pretty-printer |
| `src/types/operations.rs` | ~1800 | OperationType enum |
| `src/types/mod.rs` | ~1100 | DataType, ChannelType |
| `tests/integration.rs` | ~19000 | Integration test suite |

## Standard Library

35 modules, 374+ operations available via `use std::module::*`:
math, cmp, logic, bits, str, convert, array, map, bytes, json, time, hash, io, control, rand, fs, env, net, tcp, udp, ws, sse, http_server, cert, path, yaml, csv, toml, regex, uuid, crypto, compress, fmt, stats, text, encode, reflect, collections, sort

## Dependencies

Key crates: `indexmap` (ordered maps), `ariadne` (error rendering), `rustyline` (REPL), `wasm-encoder` (WASM codegen), `wasmtime` (optional WASM runtime), `tower-lsp` (LSP server), `ureq` (HTTP client), `regex`, `serde`/`serde_json`, `tokio`

## Rules

- **NO backward compatibility** — always write modern code
- **NEVER add AI attribution** to commits
- Maps use `IndexMap` not `BTreeMap`
- The `wasm-runtime` feature flag gates wasmtime (default enabled)
- Tests must pass: `cargo test --lib && cargo test --test integration`
