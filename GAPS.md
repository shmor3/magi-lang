# Known Gaps

Issues discovered during Doom engine development.

## Compiler

- **No compile-time function validation**: Unknown functions silently become RuntimeCalls that return null. The compiler should warn when calling a function that isn't defined or isn't a known builtin.
- **No `atan`/`atan2`/`asin`/`acos`/`pow`/`fmod` until manually added**: Math functions were missing from the C runtime. Added in commit dd32597.
- **Float equality uses bit comparison**: `a == 0.0` compares raw i64 bits via `__eq`. Two floats representing the same value but with different bit patterns (e.g., -0.0 vs 0.0) compare as not equal.
- **WASM backend**: `parse_int` builtin has invalid local index. WASM globals work but some builtins have mismatched local declarations.
- **48-bit pointer truncation risk**: NaN-boxing uses 48-bit payloads for pointers. On systems where `malloc` returns addresses > 0x7FFFFFFFFFFF, pointers get truncated. Currently works on Linux/Windows/macOS x86_64 but theoretically unsafe.
- **`return` inside match arms**: `return` in a match arm expression doesn't terminate the enclosing function. Workaround: use `if/return` before the match.

## Runtime (magi_runtime.c)

- **No `abs()` for floats in expressions**: `abs(c)` on a float goes through RuntimeCall which works, but inline `if c > x || c < -x` comparisons on floats may not work correctly due to NaN-boxing type dispatch.
- **String equality fixed but array/map equality not deep**: `__eq` does string content comparison but arrays and maps still compare by pointer identity, not by value.
- **`embed()` byte arrays are read-only**: `arr[i] = x` on an embedded byte array silently does nothing.
- **No garbage collection**: All `malloc` calls in the runtime never free. Long-running programs leak memory.

## SDL2 / Canvas Package

- **SDL2 on Windows requires `SDL_MAIN_HANDLED`**: Fixed, but if a user links SDL2 without the canvas package, they'll hit this.
- **macOS SDL2 built without Cocoa/Metal**: The cross-compiled macOS SDL2 static lib uses dummy video driver. Native macOS build needs `brew install sdl2` for real display support.
- **No audio on Windows**: PulseAudio RuntimeCalls return null on Windows. Need Windows audio backend (WASAPI/DirectSound) in the canvas package.

## Doom Engine

- **Tangent table computation**: Division by near-zero cos values can produce Infinity/NaN that crashes `floor()`. Clamped to safe range.
- **Missing Doom thing types**: mobjinfo table has ~90 entries; full Doom has 137. Missing: Arch-vile, Revenant, Mancubus, Arachnotron, SS Nazi, Commander Keen, Icon of Sin.
- **Missing states**: ~500 states defined; full Doom has 967. Missing states for Doom 2 enemies and some weapon animations.
- **Incomplete line specials**: ~50 of 140 line specials have detailed implementations; the rest dispatch to stubs.
- **No save/load**: Game state serialization not implemented.
- **No demo playback**: LMP demo format not parsed.
- **No animated textures**: ANIMATED lump not loaded; switches don't animate.
- **No intermission map animations**: The episode map dot progression not rendered.
