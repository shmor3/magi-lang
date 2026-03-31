# Known Gaps

Issues discovered during Doom engine development. Updated 2026-03-31.

## Compiler — Fixed

- ~~**No compile-time function validation**~~: **Fixed.** Compiler now warns on unknown function calls.
- ~~**Unary float negation broken**~~: **Fixed.** `-41.0` now correctly negates via LLVM IR tag check + XOR sign bit.
- ~~**Function callbacks broken (CallIndirect)**~~: **Fixed.** Local/global variables called as functions now use CallIndirect instead of RuntimeCall.
- ~~**Windows cross-compilation broken**~~: **Fixed.** LLVM module target triple now set for correct calling convention.
- ~~**alloca-in-loop stack overflow**~~: **Fixed.** RuntimeCall args buffer pre-allocated at function entry.
- ~~**Duplicate global declarations corrupt indices**~~: **Fixed.** Second `let x = []` reuses existing global slot with warning.
- ~~**Empty map `{}` parsed as block**~~: **Fixed.** Now parsed as empty MapLiteral in expression context.
- ~~**`return` inside match arms**~~: **Not a bug.** Works correctly in all backends.

## Compiler — Open

- **Float equality edge case**: `-0.0 == 0.0` may not work correctly in all contexts due to NaN-boxing bit comparison.
- **48-bit pointer truncation risk**: NaN-boxing uses 48-bit payloads for pointers. Works on x86_64 but theoretically unsafe on systems with >48-bit addresses.
- **Self-hosted parser `|` before `(`**: The self-hosted MAGI parser treats `data[offset] | (data[offset + 1] << 8)` as a lambda parameter list instead of bitwise OR.

## WASM Backend — Open

- **No browser runtime**: WASM module compiles and validates but requires a full JavaScript runtime to execute. The browser page loads the module but stubs all imports — no actual execution.
- **Tail expression codegen**: Functions ending with a value expression (not explicit `return`) may produce stack validation errors in WASM. Workaround: use explicit `return`.
- **Duplicate function names**: Two functions with the same name in a combined file can cause WASM validation errors. Workaround: remove duplicates.

## Runtime — Fixed

- ~~**`has()` method missing**~~: **Fixed.** Implemented for maps and arrays.
- ~~**Array/map equality not deep**~~: **Fixed.** `[1,2,3] == [1,2,3]` now returns true.
- ~~**`embed()` byte arrays read-only**~~: **Fixed.** Copy-on-write: first `arr[i] = x` converts byte array to regular array.

## Runtime — Open

- **No garbage collection**: All `malloc` calls never free. Long-running programs leak memory. Arena allocator added for print formatting only.
- **String concatenation O(n^2)**: Building strings in loops (`s = s + piece`) copies the entire accumulated string each time.

## Performance — Done

- **Inline binary ops**: `+,-,*,/,%,<,>,<=,>=` compiled as LLVM IR with 3-path int/float/fallback dispatch (3-5x speedup).
- **Inline ArrayGet**: `array[i]` compiled as LLVM IR pointer math (2-3x speedup).
- **Hash table maps**: FNV-1a open addressing replaces O(n) strcmp scan (2-5x speedup).
- **Numeric dispatch**: 119 runtime handlers via O(1) jump table instead of 116 strcmp calls (2-3x speedup).
- **Untagged loop counters**: 6 raw IR instructions for for-in loops (2x speedup).
- **Direct builtin calls**: len/push/abs/floor/sqrt/cos/sin/atan2 bypass dispatcher.
- **Float fast path**: Mixed int/float arithmetic handled without full dispatch.
- **Arena allocator**: Bump allocation for print formatting (less malloc pressure).

## Performance — Open

- **No inline MapGet**: Map field access (`obj["x"]`) still calls C function with hash lookup. Could be inlined for known string keys.
- **No inline ArraySet**: `array[i] = val` still calls C function. Could be inlined like ArrayGet.
- **String operations slow**: `to_string`, `split`, `join`, `replace` all go through RuntimeCall dispatch.

## SDL2 / Canvas Package

- **macOS SDL2 static lib invalid**: Cross-compiled macOS libSDL2.a has wrong object format. Workaround: `brew install sdl2` and link dynamically.
- **No audio on Windows**: PulseAudio RuntimeCalls return null. Need WASAPI/DirectSound backend.
- **SDL2 not bundled for Linux/Mac**: Users must install SDL2 system package. Windows .exe has it statically linked.

## Doom Engine

- **Renderer uses float perspective projection**: Ported from doom-rust-renderer, not the original angle-based Doom renderer. Visually similar but not pixel-accurate.
- **No sprites**: Enemies, items, decorations not rendered.
- **Missing Doom thing types**: ~90 of 137 mobjinfo entries. Missing Doom 2 enemies.
- **Missing states**: ~500 of 967. Missing Doom 2 enemy states.
- **Incomplete line specials**: ~50 of 140 implemented.
- **No save/load, demo playback, animated textures, intermission maps**.
- **~25 fps on modern hardware**: Native builds run at 20-30 fps. Rust reference achieves 60 fps with same algorithm. Gap is from NaN-boxing overhead on non-inlined operations.
