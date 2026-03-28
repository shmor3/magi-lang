# MAGI Language Status

**Version**: 0.9.0
**Dependencies**: Zero (OpenSSL, SDL2, PulseAudio linked via system FFI)

## Numbers

| Metric | Value |
|--------|-------|
| OperationType variants | 468 |
| Stdlib modules | 105 |
| Stdlib functions | 1,355 |
| LSP handlers | 43 |
| Lint codes (W) | 49 |
| Error codes (E) | 27 |
| CLI commands | 40+ |
| Lib tests | 1,663 |
| Integration tests | 1,600 |
| Source lines | ~93,000 |
| Total lines (with tests) | ~114,000 |

## Execution Modes

| Mode | Command | Description |
|------|---------|-------------|
| Interpreted | `magi run file.magi` | Tree-walking interpreter (100% spec) |
| Runtime | `magi compilec file.magi` then `magi runc file.magc` | Compile to .magc, execute on MagiVM (100% spec) |
| WASM | `magi compile file.magi` then `magi run-wasm file.wasm` | Compile to WASM binary |
| Bytecode | `magi run-bc file.magi` | Bytecode VM with function calls |
| Native | `magi compile-native file.magi` | x86-64 ELF binary |

## Architecture

```
Source (.magi)
    -> Lexer -> Parser -> AST
    -> Optimizer (constant folding, DCE, TCO, inlining)
    -> Type Checker (inference, null safety, generics)
    -> Linter (49 rules)
    -> Interpreter (tree-walking, virtual heap, scope-based GC)
    -> Runtime (.magc classfiles, MagiVM, mark-and-sweep GC, classloader)
    -> MAGI IR (shared by all compilation backends)
        -> IR VM (stack-machine interpreter)
        -> WASM Compiler -> WASM binary (198 opcodes in runtime)
        -> Native Compiler -> x86-64/aarch64 ELF/Mach-O
    -> WebGPU Backend -> WASM + JS host bridge (19 imports)
    -> Formatter (AST pretty-printer)
    -> LSP Server (43 handlers, JSON-RPC over stdio)
    -> MCP Server (7 tools, Model Context Protocol)
```

## Toolchain

| Tool | Command |
|------|---------|
| Run (interpreted) | `magi run file.magi` |
| Run (runtime) | `magi compilec file.magi` then `magi runc file.magc` |
| Run (bytecode) | `magi run-bc file.magi` |
| Run (WASM) | `magi compile file.magi` then `magi run-wasm file.wasm` |
| REPL | `magi repl` |
| Test | `magi test file.magi` |
| Format | `magi fmt --write file.magi` |
| Lint | `magi lint file.magi` |
| Type check | `magi check file.magi` |
| Benchmark | `magi bench file.magi` |
| Coverage | `magi coverage file.magi` |
| Doc generate | `magi doc file.magi` |
| Doc test | `magi doc-test file.magi` |
| LSP server | `magi lsp` |
| MCP server | `magi mcp` |
| Debug | `magi debug file.magi` |
| Init project | `magi init my-project` |
| Add dependency | `magi add package` |
| Install | `magi install url` |
| Publish | `magi publish` |
| Update deps | `magi update` |
| Audit | `magi audit` |
| Clean | `magi clean` |
| Env | `magi env` |
| Evaluate | `magi eval '1 + 2'` |
| Search | `magi search query` |
| Expand | `magi expand file.magi` |
| VM stats | `magi vm-stats` |

## Platform FFI

System libraries linked at build time via FFI:
- **OpenSSL** (`libssl`, `libcrypto`) — TLS 1.2/1.3, AES, RSA, ECDSA, Ed25519, SHA, PBKDF2, bcrypt, Argon2, X.509
- **SDL2** (`libSDL2`, optional) — Pixel graphics, window management, input events
- **PulseAudio** (`libpulse-simple`, optional) — Real-time audio streaming
