# MAGI Roadmap

## Official Packages (v0.1.0)

Three official packages in `packages/`:

### canvas
Terminal display, ANSI rendering, cursor control, color output, character-cell framebuffer, and SDL2 pixel graphics.
- ANSI: `canvas_new(w, h)`, `canvas_set()`, `canvas_render()`
- Drawing: `canvas_rect()`, `canvas_fill()`, `canvas_line()`, `canvas_center()`
- Colors: `color()`, `bold()`, `rgb()`, `truecolor()`
- Terminal: `screen_clear()`, `cursor_move()`, `cursor_hide()`, `cursor_show()`
- SDL2 Pixel: `pixel_window()`, `pixel_set()`, `pixel_line()`, `pixel_rect()`
- SDL2 Control: `pixel_color()`, `pixel_clear()`, `pixel_present()`, `pixel_poll()`

### keypress
Raw keyboard input via termios FFI, key events, escape sequence parsing.
- `raw_mode_enable()`, `raw_mode_disable()` (termios FFI)
- `read_key()`, `wait_key()`, `read_byte()`, `read_byte_timeout()`
- Constants: `KEY_UP`, `KEY_DOWN`, `KEY_LEFT`, `KEY_RIGHT`, `KEY_ESCAPE`, `KEY_ENTER`, `KEY_F1`-`KEY_F12`
- Helpers: `is_printable()`, `key_name()`, `is_arrow()`, `is_ctrl()`

### speaker
Audio playback via PulseAudio FFI streaming with WAV fallback.
- Waveforms: `generate_sine()`, `generate_square()`, `generate_sawtooth()`, `generate_triangle()`
- Streaming: `stream_open()`, `stream_write()`, `stream_close()`
- Playback: `beep(freq, ms)`, `play_wav()`, `play_sequence()`
- WAV encoding: `encode_wav()`, `save_wav()`
- Effects: `mix()`, `volume()`, `fade_in()`, `fade_out()`
- Notes: `NOTE_C4` through `NOTE_C5`

## Platform FFI (v0.2.0) ✓

All implemented in `std::platform`:
- **Termios FFI**: `raw_mode_enable()`, `raw_mode_disable()`, `read_byte()`, `read_byte_timeout()`
- **SDL2 FFI**: `sdl_init()`, `sdl_draw_pixel()`, `sdl_draw_line()`, `sdl_fill_rect()`, `sdl_poll_event()`, `sdl_set_color()`, `sdl_clear()`, `sdl_present()`, `sdl_delay()`, `sdl_ticks()`, `sdl_destroy()`
- **PulseAudio FFI**: `audio_stream_new()`, `audio_write_samples()`, `audio_drain()`, `audio_close()`

## WebGPU Backend (v0.2.0) ✓

- WebGPU WASM import bindings (19 functions)
- JavaScript host bridge (`MagiWebGPU` class)
- Buffer, shader, pipeline, render pass, texture management
- Indexed and instanced drawing

## Future Work

### v1.0.0
- Package proxy/mirror for caching
- Package search website
