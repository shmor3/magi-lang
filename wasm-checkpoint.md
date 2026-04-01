# WASM Browser Runtime — Checkpoint (2026-03-31)

## Status
WASM compilation and browser execution works. Doom WASM module loads, parses WAD header, reads 1264 lumps, but `wad_find()` string comparison fails at runtime despite byte-level string equality being implemented in the compiler.

## Key Fixes Made
1. **NANBOX_SIG**: JS runtime had `0x7FF8...` (wrong) vs compiler's `0xFFF8...` — bit 63 mismatch. Fixed.
2. **String equality**: WASM backend `I64Eq`/`I64Ne` now does byte-by-byte content comparison for STRING-tagged values (not just pointer equality). Added `I32And`, `I32Load8U` to wasm_binary.rs.
3. **Memory sizing**: WASM initial memory now auto-sized to fit data section (strings + embedded files).
4. **allocStr**: JS `allocStr` now writes string data to WASM memory (not just JS-side cache).
5. **push()**: Implemented JS-side `push(array, elem)` that allocates new array in WASM memory.
6. **chr/char_at/upper**: Implemented in JS runtime.

## Known Issues
- `wad_find()` can't match lump names despite both strings printing correctly. Likely `.upper()` method call returns string at different memory region than stored names, and comparison still fails in some edge case.
- `.push()` method syntax (e.g. `arr.push(x)`) doesn't reassign — must use `arr = push(arr, x)`.
- 6 WASM e2e tests fail in Rust interpreter (match/null_coalesce) — browser validation passes.
- `embed_offsets` field added to WasmCodegen struct (unused, can be removed).

## Files Modified
- `src/compiler/wasm.rs` — `&mut self` on emit(), embed_offsets, string-aware I64Eq/I64Ne, push recognition, memory auto-sizing
- `src/compiler/wasm_binary.rs` — Added `I32And`, `I32Load8U` variants
- `src/compiler/mod.rs` — `let mut codegen`
- `Doom/doom/wad.magi` — Byte-level WAD header check, `push()` assignment syntax
- `Doom/doom/main.magi` — `let EMBEDDED_WAD` (moved embed inside doom_main)

## JS Runtime (doom-wasm-sdl.html on 10.0.0.111:/home/dev/site/doom/)
- NaN-boxing with correct `NANBOX_SIG=0xFFF8...`
- Runtime calls: print, sin/cos/atan/floor/ceil/sqrt/abs, min/max, len, push, chr, char_at, upper, lower, trim, to_string, to_int, to_float, typeof, concat, has, __byte_slice, to_byte, __embed
- SDL stub: canvas init, blit_fb (framebuffer→canvas), poll_event, ticks, delay, set_title, destroy
- Audio stubs: stream_new, write_samples, drain, close
- Pre-grows memory to 64MB before execution

## Next Steps
- Debug why string comparison fails in wad_find (printed strings look identical but == returns false)
- Consider implementing StringConcat result caching or interning
- Or: bypass string comparison by using integer lump IDs
