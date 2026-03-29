# MAGI Roadmap

## Syntax Overhaul (v1.0.0) - Complete

- `func` keyword, `interface` keyword, `import std.math`
- Arrow functions, dot receiver methods, multi-return errors
- `const`/`let` bindings, optional semicolons, clean enum display
- Named operator interfaces (`Add`, `Display`, `Equal`, `Compare`)

Full spec: `docs/specs/2026-03-28-syntax-overhaul-design.md`

## Self-Hosting (v1.0.0) - Complete

MAGI compiler rewritten in MAGI: 95,454 lines across 31 files.
- Lexer, parser, AST, type checker, interpreter, 468 operations
- Optimizer, linter (49 rules), formatter
- IR compiler, IR VM, WASM backend, native backend, WebGPU
- LSP server (43 handlers), MCP server (7 tools), debugger
- Package registry, runtime (.magc classfiles, VM, GC)
- Utilities (JSON, regex, encoding, crypto, collections)

## Official Packages

- `canvas` — Terminal display and SDL2 pixel graphics
- `keypress` — Raw keyboard input via termios FFI
- `speaker` — Audio playback via PulseAudio FFI

## Platform FFI - Complete

- Termios FFI (terminal raw mode)
- SDL2 FFI (pixel graphics)
- PulseAudio FFI (real-time audio)
- WebGPU WASM bindings

## Compilation Targets - Complete

- Interpreted (`magi run`) — 100% spec conformance
- Runtime (`magi compilec` + `magi runc`) — 100% spec conformance
- IR VM (`magi run-bc`) — full language via shared IR
- WASM (`magi compile`) — binary output
- Native (`magi compile-native`) — x86-64/aarch64 ELF/Mach-O
