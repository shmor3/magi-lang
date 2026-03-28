# CLAUDE.md — magi-lang

## Overview

MAGI is a general-purpose programming language with multiple compilation targets. Complete implementation: lexer, parser, AST, type checker, linter, formatter, interpreter, runtime (.magc classfiles + MagiVM), bytecode VM, WASM compiler, native compiler (x86-64 ELF), WebGPU backend, LSP server, MCP server, optimizer, and CLI.

Zero dependencies. ~114,000 lines. 3,263 tests (1,663 lib + 1,600 integration).

## Commands

```bash
cargo build --bin magi
cargo test --lib                     # 1663 tests
cargo test --test integration        # 1600 tests (needs RUST_MIN_STACK=33554432)
```

## Execution Modes

```bash
magi run file.magi                   # interpreted (100% spec conformance)
magi compilec file.magi && magi runc file.magc  # runtime (100% spec conformance)
magi run-bc file.magi                # IR VM (AST → IR → stack machine)
magi compile file.magi && magi run-wasm dist/file.wasm  # WASM (AST → IR → WASM binary)
magi compile-native file.magi        # native (AST → IR → x86-64/aarch64 ELF/Mach-O)
```

## Compiler Architecture

All compilation backends share a single IR:
```
Source → AST → MAGI IR → WASM backend / Native backend / IR VM
```

## Documentation

All documentation lives in `docs/`:
- `docs/spec.md` — Language specification
- `docs/stdlib.md` — Standard library reference (105 modules, 1355 operations/functions)
- `docs/errors.md` — Error and warning code reference
- `docs/cli.md` — CLI command reference (40+ commands, 16 env flags)
- `docs/status.md` — Project status and metrics
- `docs/mcp.md` — MCP server documentation

## Key Design Decisions

- **DataType::Map** uses OrderedMap (insertion-order preserving)
- **OperationType** has 468 variants — all stdlib operations are enum variants
- **Interpreter** uses virtual heap with address-based HashMap
- **Runtime** uses .magc classfiles with MAGC header + source, executed by full interpreter
- **Closures capture by value** (snapshot at definition time)
- **&&/|| use truthiness** (any value, not just Bool)

## Rules

- **NO backward compatibility** — always write modern code
- **NEVER add AI attribution** to commits
- Tests must pass: `cargo test --lib && cargo test --test integration`
