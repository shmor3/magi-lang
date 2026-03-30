# MAGI Compiler Stability & Performance Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix all compiler bugs, runtime gaps, and performance issues discovered during Doom engine development, making MAGI production-ready for non-trivial compiled programs.

**Architecture:** The MAGI compiler (AST → IR → LLVM native) has a C runtime (`magi_runtime.c`) linked into every binary. NaN-boxing tags values as null/bool/int/string/array/map/float in 64-bit words. The LLVM backend generates object files, the C runtime handles type dispatch. Fixes span the IR compiler (`compile.rs`), LLVM backend (`llvm.rs`), and C runtime (`magi_runtime.c`).

**Tech Stack:** Rust (compiler), C (runtime), LLVM 18 (code generation), inkwell (Rust LLVM bindings)

**Test server:** dev@10.0.0.111 (all tests run here, never locally)

---

## Critical Bugs (crash/correctness)

### Task 1: Fix unary float negation (compile.rs)

**Status:** Fixed in this session but broke 9 unit tests. Needs proper fix.

**Problem:** `-41.0` compiles to `I64Neg` (integer negation of float bits) instead of `F64Neg`. The tag-dispatch `GetTag`/`I64Eq`/`If` branch doesn't work because LLVM's `icmp` on NaN-boxed tag values fails for untagged floats.

**Files:**
- Modify: `src/compiler/compile.rs:1087-1113` (UnaryOp::Neg handler)
- Modify: `src/compiler/llvm.rs` (I64Neg handler — add float check)
- Test: existing tests `test_compile_negation`, `test_e2e_negative_float_arithmetic`

**Current workaround:** Compiles as `RuntimeCall("__neg")` — works but slow.

**Proper fix:** In the LLVM backend, make `I64Neg` check the tag at runtime (LLVM IR branch) and dispatch to float or int negation:

- [ ] **Step 1:** Revert compile.rs to emit proper `I64Neg` (not RuntimeCall)
- [ ] **Step 2:** In llvm.rs `I64Neg` handler, generate LLVM IR that checks if the value is an untagged float (top bits != NANBOX_SIG), and if so, XOR the sign bit (bit 63), otherwise do integer negation via ext/neg/tag
- [ ] **Step 3:** Run `cargo test --lib` on 10.0.0.111 — all 1651 tests must pass
- [ ] **Step 4:** Commit

---

### Task 2: Fix binary ops for mixed int/float types (compile.rs + llvm.rs)

**Problem:** `+`, `-`, `*`, `/` go through RuntimeCall which does 50+ strcmp calls. The fast path (`__magi_fast_binop`) only handles int+int. Float operands (common in rendering, math) fall through to the slow strcmp chain.

**Files:**
- Modify: `src/compiler/magi_runtime.c` (extend `__magi_fast_binop` to handle float cases)
- Test: new test for `1.5 + 2.5`, `3 * 1.0`, `-41.0 * 192.0`

**Fix:** Extend the C fast path to check for float operands:

- [ ] **Step 1:** In `__magi_fast_binop`, after the int-int fast path, add float-float and int-float paths using `magi_as_float()` for the arithmetic and `magi_make_float()` for the result
- [ ] **Step 2:** Handle `__add` with string concat: if either operand is TAG_STRING, return 0 (sentinel) to fall through to full dispatch
- [ ] **Step 3:** Handle `__mul` with string repeat: if either is TAG_STRING, return 0
- [ ] **Step 4:** Handle `__eq`/`__ne` with string equality: if both are TAG_STRING, use strcmp
- [ ] **Step 5:** Test on 10.0.0.111: `cargo test --lib` + manual float arithmetic tests
- [ ] **Step 6:** Commit

---

### Task 3: Fix LLVM module target triple for cross-compilation (llvm.rs)

**Status:** Fixed in this session.

**Problem:** The LLVM module's target triple was never set, causing Linux calling convention (System V ABI) in Windows object files. Every cross-compiled function call had mismatched argument registers.

**Files:**
- Verify: `src/compiler/llvm.rs:55-68` (module.set_triple already added)
- Test: cross-compile a simple program for Windows, run under Wine on 10.0.0.111

- [ ] **Step 1:** Verify the fix is in place and tests pass
- [ ] **Step 2:** Add integration test: compile `println(42)` for `--target x86_64-windows`, verify output contains "42"
- [ ] **Step 3:** Commit

---

### Task 4: Fix alloca-in-loop stack overflow (llvm.rs)

**Status:** Fixed in this session.

**Problem:** `RuntimeCall` allocated args buffer on the stack (`build_array_alloca`) inside loops. After ~25K iterations, stack overflow.

**Files:**
- Verify: `src/compiler/llvm.rs` (rc_args_buf pre-allocated at function entry)
- Test: program with 100K-iteration loop calling RuntimeCall per iteration

- [ ] **Step 1:** Verify fix is in place
- [ ] **Step 2:** Add test: `let arr = []; let i = 0; while i < 100000 { arr.push(i % 256); i = i + 1 }; println(len(arr))` must output `100000`
- [ ] **Step 3:** Commit

---

### Task 5: Fix function callbacks (CallIndirect) for local/global variables (compile.rs)

**Status:** Fixed in this session.

**Problem:** Calling a local variable as a function (`callback(args)`) compiled as `RuntimeCall("callback")` (string dispatch, returns null) instead of `CallIndirect` (function pointer table lookup).

**Files:**
- Verify: `src/compiler/compile.rs:1159-1170` (local/global variable call detection)
- Test: `func foo(x) { println(x) }; const f = foo; f(42)` must print `42`

- [ ] **Step 1:** Verify fix is in place
- [ ] **Step 2:** Add test for callbacks, closures passed as arguments, recursive callbacks
- [ ] **Step 3:** Commit

---

### Task 6: Fix `has()` method missing from runtime (magi_runtime.c)

**Status:** Fixed in this session.

**Problem:** `map.has("key")` returned null — the "has" method was never implemented in the RuntimeCall dispatch.

**Files:**
- Verify: `src/compiler/magi_runtime.c` (has handler for maps and arrays)
- Test: `let m = {"a": 1}; println(m.has("a")); println(m.has("b"))` must print `true` then `false`

- [ ] **Step 1:** Verify fix is in place
- [ ] **Step 2:** Commit

---

### Task 7: Fix `for-in` loop with duplicate global declarations

**Problem:** When a combined file has duplicate `let x = []` at global scope, the second declaration creates a new global that shadows the first. Code before the duplicate references global #N, code after references global #N+1. All subsequent global indices shift, corrupting references.

**Files:**
- Modify: `src/compiler/compile.rs` (global variable registration)
- Test: file with duplicate `let arr = []` then `arr.push(1); println(len(arr))`

**Fix:** Either error on duplicate global declarations, or reuse the existing global index.

- [ ] **Step 1:** In `define_global()`, check if the name already exists in `global_vars`. If so, reuse the existing index instead of creating a new one.
- [ ] **Step 2:** Add warning diagnostic for duplicate global declarations
- [ ] **Step 3:** Test on 10.0.0.111
- [ ] **Step 4:** Commit

---

### Task 8: Fix self-hosted parser `|` before `(` parsed as lambda

**Problem:** The self-hosted MAGI parser treats `data[offset] | (data[offset + 1] << 8)` as a lambda parameter list instead of bitwise OR.

**Files:**
- Modify: `self/parser.magi` (BitOr operator disambiguation)
- Test: parse and evaluate `5 | (3 << 2)` — must return 13

- [ ] **Step 1:** In the self-hosted parser's binary operator handling, ensure `|` followed by `(` is treated as BitOr, not lambda start
- [ ] **Step 2:** Test with the Rust-built magi (interpreter mode): `magi run test.magi`
- [ ] **Step 3:** Commit

---

## Performance (10x improvement target)

### Task 9: Extend fast path for all common binary ops (magi_runtime.c)

**Problem:** The `__magi_fast_binop` only handles int+int. Float operations (used heavily in rendering) fall through to 50+ strcmp calls.

**Files:**
- Modify: `src/compiler/magi_runtime.c` (`__magi_fast_binop` and `__magi_runtime_call`)

**Fix:** After the int-int fast path in `__magi_fast_binop`:
1. Check if either operand is float (not tagged)
2. Convert both to double via `magi_as_float()`
3. Perform the operation
4. Return `magi_make_float(result)`

- [ ] **Step 1:** Add float fast path to `__magi_fast_binop` for add/sub/mul/div/mod/lt/gt/le/ge
- [ ] **Step 2:** Add eq/ne fast path with string content comparison
- [ ] **Step 3:** Benchmark: `192.0 * -41.0 / 100.0` in a 100K loop, measure time before and after
- [ ] **Step 4:** Commit

---

### Task 10: Direct C function calls for hot builtins (llvm.rs)

**Problem:** `len()`, `push()`, `to_string()`, `abs()`, `floor()`, `sqrt()`, `cos()`, `sin()`, `atan2()` all go through RuntimeCall string dispatch.

**Files:**
- Modify: `src/compiler/compile.rs` (intercept known builtin names)
- Modify: `src/compiler/llvm.rs` (emit direct calls to C functions)
- Modify: `src/compiler/magi_runtime.c` (export individual functions)

**Fix:** For known builtins, emit `Call` to a named C function instead of `RuntimeCall`. Example: `len(arr)` → direct call to `__magi_len(arr)` instead of `__magi_runtime_call("len", 1, &arr)`.

- [ ] **Step 1:** Create standalone C functions: `__magi_len`, `__magi_push`, `__magi_abs`, `__magi_floor`, `__magi_sqrt`, `__magi_cos`, `__magi_sin`, `__magi_atan2`, `__magi_to_string`
- [ ] **Step 2:** In compile.rs, detect these builtin names in the `Call` handler and emit direct IR calls
- [ ] **Step 3:** In llvm.rs, declare these functions and emit direct LLVM calls
- [ ] **Step 4:** Benchmark: math_init() trig table building time before and after
- [ ] **Step 5:** Commit

---

### Task 11: Native C renderer for hot pixel loops

**Problem:** The Doom renderer's per-column and per-pixel loops go through MAGI RuntimeCall dispatch for every float operation, making it 10x slower than equivalent Rust.

**Files:**
- Modify: `src/compiler/magi_runtime.c` (add `__render_seg_columns` that handles the full column loop)
- Modify: Doom `render2.magi` (call native renderer)

**Fix:** Move the `r2_process_seg` per-column loop entirely to C. The MAGI code handles BSP traversal and setup, but the inner rendering loop runs native.

- [ ] **Step 1:** Write `__render_seg_columns()` in C that takes all seg parameters and renders all columns
- [ ] **Step 2:** Write `__render_visplane()` in C that does per-pixel inverse perspective for floors/ceilings
- [ ] **Step 3:** Update render2.magi to call these native functions
- [ ] **Step 4:** Benchmark FPS before and after
- [ ] **Step 5:** Commit

---

## Runtime Gaps

### Task 12: Fix platform detection for native packages (llvm.rs)

**Status:** Partially fixed.

**Problem:** When no `--target` is specified, `is_macos`/`is_windows` default to false (Linux). On Mac, this causes wrong SDL2 library linking.

**Files:**
- Modify: `src/compiler/llvm.rs:231-235`

**Fix:** Use `cfg!(target_os)` for host detection when target_triple is empty.

- [ ] **Step 1:** Verify the fix uses `cfg!(target_os = "macos")` and `cfg!(target_os = "windows")`
- [ ] **Step 2:** Test: build on Mac without `--target` flag, verify macOS SDL2 is linked
- [ ] **Step 3:** Commit

---

### Task 13: Fix native package search path (llvm.rs)

**Problem:** Package search hardcodes `/home/dev/workspace/magi/magi-lang/packages`. Breaks on any other machine.

**Files:**
- Modify: `src/compiler/llvm.rs:337-356` (`find_native_packages`)

**Fix:** Search relative to the compiler binary (`std::env::current_exe()`) and relative to the source file being compiled.

- [ ] **Step 1:** Add search path: `exe_dir/../../packages` (relative to compiler binary)
- [ ] **Step 2:** Add search path: `source_dir/packages` (relative to the .magi file)
- [ ] **Step 3:** Remove hardcoded `/home/dev` path
- [ ] **Step 4:** Test: compile from a different directory, verify canvas package is found
- [ ] **Step 5:** Commit

---

### Task 14: Add compile-time unknown function warnings

**Problem:** Calling an undefined function silently becomes a RuntimeCall that returns null. No error or warning.

**Files:**
- Modify: `src/compiler/compile.rs` (Call handler)

**Fix:** When a function name is not in `fn_index` and not a known builtin, emit a compiler warning.

- [ ] **Step 1:** Build a set of known runtime builtins (len, push, pop, println, to_string, typeof, etc.)
- [ ] **Step 2:** In the Call handler, if the name is not in fn_index, not a local, not a global, and not a known builtin, emit a warning
- [ ] **Step 3:** Test: calling `nonexistent_func()` should produce a warning
- [ ] **Step 4:** Commit

---

### Task 15: Deep equality for arrays and maps

**Problem:** `[1,2,3] == [1,2,3]` returns false (pointer identity comparison).

**Files:**
- Modify: `src/compiler/magi_runtime.c` (`__eq` handler)

**Fix:** In `__eq`, when both operands are TAG_ARRAY, compare element-by-element. When both are TAG_MAP, compare key-by-key.

- [ ] **Step 1:** Add recursive `__magi_deep_eq(a, b)` function
- [ ] **Step 2:** Call it from `__eq` when both operands are arrays or maps
- [ ] **Step 3:** Test: `[1,2,3] == [1,2,3]` → true, `{"a":1} == {"a":1}` → true
- [ ] **Step 4:** Commit

---

### Task 16: Make embed() byte arrays writable

**Problem:** `arr[i] = x` on an embedded byte array silently does nothing (cap == -1 marker).

**Files:**
- Modify: `src/compiler/magi_runtime.c` (`__magi_array_set`)

**Fix:** In `__magi_array_set`, when `cap == -1` (byte array), allocate a real array, copy the bytes as tagged ints, then set the value.

- [ ] **Step 1:** Detect byte array in `__magi_array_set`
- [ ] **Step 2:** Copy-on-write: allocate new data array, copy bytes, update cap/data pointers
- [ ] **Step 3:** Test: `const data = embed("test.bin"); data[0] = 42; println(data[0])` → 42
- [ ] **Step 4:** Commit

---

### Task 17: Fix empty map literal `{}`

**Problem:** Parser treats `{}` as an empty block, not an empty map. Workaround: `{"__e": 0}`.

**Files:**
- Modify: `src/syntax/parser.rs` (map literal disambiguation)

**Fix:** When `{` is followed by `}` in an expression context (not statement), treat as empty map.

- [ ] **Step 1:** In the parser, detect `{}` after `=`, `(`, `,`, `return`, etc. and parse as empty map
- [ ] **Step 2:** Test: `let m = {}; m["key"] = 1; println(m["key"])` → 1
- [ ] **Step 3:** Commit

---

### Task 18: Fix `return` inside match arms

**Problem:** `return` in a match arm doesn't terminate the enclosing function.

**Files:**
- Modify: `src/compiler/compile.rs` (match arm compilation)

- [ ] **Step 1:** Investigate how match arms compile — likely missing a branch terminator
- [ ] **Step 2:** Fix: ensure `return` in a match arm emits proper function return
- [ ] **Step 3:** Test: `func f(x) { match x { 1 => { return 10 }, _ => { return 20 } } }; println(f(1))` → 10
- [ ] **Step 4:** Commit

---

## Test Infrastructure

### Task 19: Add compiled-mode integration tests

**Problem:** The 1620 integration tests only test the interpreter. The compiled mode (LLVM backend) has different codegen paths that need their own tests.

**Files:**
- Create: `tests/compiled_integration.rs`

- [ ] **Step 1:** Create test harness that compiles a .magi file with `magi build`, runs the binary, captures stdout
- [ ] **Step 2:** Add tests for: int arithmetic, float arithmetic, string ops, array ops, map ops, callbacks, for-in loops, closures, embed(), negative floats
- [ ] **Step 3:** Add cross-compilation smoke test (compile for Windows, verify binary is PE format)
- [ ] **Step 4:** Commit

---

## Summary

| # | Issue | Severity | Status |
|---|-------|----------|--------|
| 1 | Unary float negation | Critical | Workaround |
| 2 | Mixed int/float binary ops slow | Performance | Open |
| 3 | Windows calling convention | Critical | Fixed |
| 4 | alloca-in-loop overflow | Critical | Fixed |
| 5 | CallIndirect broken | Critical | Fixed |
| 6 | has() missing | Critical | Fixed |
| 7 | Duplicate globals shift indices | Critical | Workaround |
| 8 | Self-hosted parser `\|` bug | Major | Open |
| 9 | Float fast path | Performance | Open |
| 10 | Direct builtin calls | Performance | Open |
| 11 | Native C renderer | Performance | Partial |
| 12 | Platform detection | Major | Partial |
| 13 | Package search path | Major | Partial |
| 14 | Unknown function warnings | Major | Open |
| 15 | Deep equality | Minor | Open |
| 16 | Byte array writability | Minor | Open |
| 17 | Empty map `{}` | Minor | Open |
| 18 | Return in match arms | Major | Open |
| 19 | Compiled-mode tests | Infrastructure | Open |
