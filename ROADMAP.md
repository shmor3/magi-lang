# MAGI Roadmap

## Syntax Overhaul (v1.0.0) - Complete

New syntax implemented:
- `func` keyword (replaces `fn`)
- `interface` keyword (replaces `trait`)
- `import std.math` with qualified access (`math.sqrt()`)
- Arrow functions (`x => x * 2`)
- Dot receiver methods (`func Vec2.length(self)`)
- Multi-return errors (`let val, err = f()`)
- `println()`/`print()` as expression builtins
- Clean enum display (`Color::Red`)
- `const`/`let` bindings (immutable/mutable)
- Optional semicolons

Full spec: `docs/specs/2026-03-28-syntax-overhaul-design.md`

## Official Packages (v0.1.0)

Three official packages in `packages/`:

### canvas
Terminal display and SDL2 pixel graphics.

### keypress
Raw keyboard input via termios FFI.

### speaker
Audio playback via PulseAudio FFI streaming.

## Platform FFI (v0.2.0) - Complete

- Termios FFI (terminal raw mode)
- SDL2 FFI (pixel graphics)
- PulseAudio FFI (real-time audio)
- WebGPU WASM bindings

## Self-Hosting

Rewrite the entire MAGI compiler in MAGI itself.
Design: `docs/specs/2026-03-24-self-hosting-design.md`
