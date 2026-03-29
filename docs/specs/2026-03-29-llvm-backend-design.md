# LLVM Backend + Static Bundling Design

**Date**: 2026-03-29
**Status**: Final
**Goal**: Replace hand-written native compiler with LLVM backend. Bundle OpenSSL and zlib statically. Simplify CLI commands.

## Summary

- Add LLVM backend via inkwell crate for native compilation
- Bundle OpenSSL (vendored) and zlib (static) — no system deps for core language
- Remove hand-written native backend, bytecode compiler, IR VM, run-wasm
- Unify `compile-native` and `compile` into single `magi compile` with `--target` flag

## Dependencies Added

```toml
[dependencies]
inkwell = { version = "0.5", features = ["llvm18-0"] }
openssl = { version = "0.10", features = ["vendored"] }
libz-sys = { version = "1", features = ["static"] }
```

## Files Removed

- `src/compiler/native.rs` — hand-written x86-64/aarch64 codegen
- `src/compiler/bytecode.rs` — limited bytecode compiler and VM
- `src/compiler/ir_vm.rs` — IR stack-machine interpreter

## Files Added

- `src/compiler/llvm.rs` — MAGI IR → LLVM IR → machine code

## Files Modified

- `Cargo.toml` — add inkwell, openssl vendored, libz-sys static
- `build.rs` — remove dynamic OpenSSL linking, remove probe_lib
- `src/compiler/mod.rs` — remove bytecode, native, ir_vm modules; add llvm module
- `src/bin/magi.rs` — remove `compile-native`, `run-bc`, `run-wasm` commands; update `compile` to use `--target` flag
- `src/lib.rs` — no changes (compiler module re-exports handled in mod.rs)

## Compilation Pipeline

```
Source → Lexer → Parser → AST → MAGI IR → LLVM IR → LLVM Optimizer → Machine Code
                                         → WASM Binary (unchanged)
```

Both native and WASM compilation share the same AST → MAGI IR frontend. Only the backend differs.

## LLVM Backend (`src/compiler/llvm.rs`)

### Entry Point

```rust
pub fn compile_to_native(source: &str, target: &str, opt_level: u8, output: &str) -> Result<(), String>
```

### IR Translation

Map each MAGI IR instruction to LLVM IR:

| MAGI IR | LLVM IR |
|---------|---------|
| PushI64(n) | `i64 constant` |
| PushF64(f) | `double constant` |
| PushString(idx) | `global string constant` |
| PushNull | `i64 NaN-boxed null` |
| PushBool(b) | `i64 NaN-boxed bool` |
| I64Add | `add i64` |
| I64Sub | `sub i64` |
| I64Mul | `mul i64` |
| I64Div | `sdiv i64` |
| F64Add | `fadd double` |
| F64Sub | `fsub double` |
| F64Mul | `fmul double` |
| F64Div | `fdiv double` |
| I64Eq/Ne/Lt/Gt/Le/Ge | `icmp` |
| F64Eq/Ne/Lt/Gt/Le/Ge | `fcmp` |
| LocalGet(idx) | `load from alloca` |
| LocalSet(idx) | `store to alloca` |
| GlobalGet(idx) | `load from global` |
| GlobalSet(idx) | `store to global` |
| Call(fn_idx) | `call @function` |
| Return | `ret` |
| Block/Loop/If/Else/End/Br/BrIf | LLVM basic blocks with br/br_cond |
| Print | `call @__magi_print` |
| ArrayNew(n) | `call @__magi_array_new` |
| ArrayGet | `call @__magi_array_get` |
| ArraySet | `call @__magi_array_set` |
| ArrayLen | `call @__magi_array_len` |
| MapNew(n) | `call @__magi_map_new` |
| MapGet | `call @__magi_map_get` |
| MapSet | `call @__magi_map_set` |
| StringConcat | `call @__magi_string_concat` |
| RuntimeCall | `call @__magi_runtime_call` |

### NaN-Boxing

All MAGI values are represented as i64 using the same NaN-boxing scheme as the WASM backend:
- Float64: raw IEEE 754 bits
- Non-float: `0xFFF8 | (tag << 48) | payload`
- Tags: NULL=0, BOOL=1, I64=2, STRING=3, ARRAY=4, MAP=5

### Runtime Library

A small runtime library compiled into every MAGI binary:

```rust
// Linked into the LLVM output
extern "C" fn __magi_print(val: i64) { ... }
extern "C" fn __magi_array_new(count: i32, ...) -> i64 { ... }
extern "C" fn __magi_array_get(arr: i64, idx: i64) -> i64 { ... }
extern "C" fn __magi_array_set(arr: i64, idx: i64, val: i64) { ... }
extern "C" fn __magi_array_len(arr: i64) -> i64 { ... }
extern "C" fn __magi_string_concat(a: i64, b: i64) -> i64 { ... }
extern "C" fn __magi_map_new(count: i32, ...) -> i64 { ... }
extern "C" fn __magi_map_get(map: i64, key: i64) -> i64 { ... }
extern "C" fn __magi_map_set(map: i64, key: i64, val: i64) { ... }
extern "C" fn __magi_runtime_call(name: i64, argc: i32, ...) -> i64 { ... }
extern "C" fn __magi_gc_collect() { ... }
extern "C" fn __magi_alloc(size: i64) -> i64 { ... }
extern "C" fn __magi_free(ptr: i64) { ... }
```

The runtime is compiled as LLVM IR and linked with the user's program.

### Optimization Levels

| Flag | LLVM Pass |
|------|-----------|
| `-O0` | No optimization (fast compile) |
| `-O1` | Basic optimizations |
| `-O2` | Full optimizations (default) |
| `-O3` | Aggressive optimizations |
| `-Os` | Size optimization |

### Targets

| `--target` | Output |
|------------|--------|
| `native` (default) | ELF/Mach-O for host platform |
| `wasm` | WASM binary (uses existing WASM backend, not LLVM) |
| `x86_64-linux` | Cross-compile x86-64 Linux ELF |
| `aarch64-linux` | Cross-compile aarch64 Linux ELF |
| `x86_64-macos` | Cross-compile x86-64 macOS Mach-O |
| `aarch64-macos` | Cross-compile aarch64 macOS Mach-O |

## CLI Changes

### Before

```
magi compile file.magi           # WASM
magi compile-native file.magi    # native (hand-written)
magi run-bc file.magi            # IR VM
magi run-wasm file.wasm          # WASM runtime
```

### After

```
magi compile file.magi                     # native for host (default)
magi compile file.magi --target wasm       # WASM
magi compile file.magi --target native     # explicit native
magi compile file.magi -O2                 # with optimization
magi compile file.magi -o output           # custom output name
```

### Removed Commands

- `compile-native` — replaced by `compile --target native`
- `run-bc` — no user need
- `run-wasm` — use wasmtime/wasmer

## build.rs Changes

```rust
fn main() {
    // Build date and target (unchanged)
    ...

    // OpenSSL: handled by openssl crate's vendored feature (no manual linking)
    // zlib: handled by libz-sys static feature

    // SDL2 (optional, dynamic)
    if probe_lib("SDL2") {
        println!("cargo:rustc-link-lib=SDL2");
        println!("cargo:rustc-cfg=has_sdl2");
    }

    // PulseAudio (optional, dynamic)
    if probe_lib("pulse-simple") {
        println!("cargo:rustc-link-lib=pulse-simple");
        println!("cargo:rustc-link-lib=pulse");
        println!("cargo:rustc-cfg=has_pulseaudio");
    }
}
```

## Testing

The LLVM backend must produce correct output for all programs the interpreter handles. Test:

```bash
# Must produce identical output:
magi run examples/fibonacci.magi > /tmp/interp.txt
magi compile examples/fibonacci.magi -o /tmp/fib && /tmp/fib > /tmp/compiled.txt
diff /tmp/interp.txt /tmp/compiled.txt  # must be empty
```

Run this comparison for all example files and integration test programs.

## Success Criteria

1. `magi compile file.magi` produces a working native binary for the host platform
2. `magi compile file.magi --target wasm` produces a working WASM binary
3. All example programs compile and produce identical output to the interpreter
4. The `magi` binary is self-contained (no OpenSSL or zlib system dependency)
5. SDL2 and PulseAudio remain optional dynamic deps
6. Cross-compilation works for x86-64/aarch64 Linux/macOS
7. Optimization levels -O0 through -O3 work
8. Build time stays under 5 minutes on a modern machine
