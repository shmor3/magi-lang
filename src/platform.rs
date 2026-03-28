//! Platform FFI — termios, SDL2, and audio bindings.
//!
//! Provides direct system access for terminal raw mode, graphics, and sound.
//! SDL2 and PulseAudio are optional — gated behind `has_sdl2` and `has_pulseaudio` cfg flags.

// ── Termios FFI ──────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone)]
struct Termios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; 32],
    c_ispeed: u32,
    c_ospeed: u32,
}

extern "C" {
    fn tcgetattr(fd: i32, termios: *mut Termios) -> i32;
    fn tcsetattr(fd: i32, action: i32, termios: *const Termios) -> i32;
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
}

const STDIN_FD: i32 = 0;
const TCSANOW: i32 = 0;
const ICANON: u32 = 0o0000002;
const ECHO: u32 = 0o0000010;
const ISIG: u32 = 0o0000001;
const IEXTEN: u32 = 0o0100000;
const IXON: u32 = 0o0002000;
const ICRNL: u32 = 0o0000400;
const OPOST: u32 = 0o0000001;
const VMIN: usize = 6;
const VTIME: usize = 5;

static mut ORIGINAL_TERMIOS: Option<Termios> = None;

/// Enable terminal raw mode via termios.
pub fn raw_mode_enable() -> Result<(), String> {
    unsafe {
        let mut term = std::mem::zeroed::<Termios>();
        if tcgetattr(STDIN_FD, &mut term) != 0 {
            return Err("tcgetattr failed".into());
        }
        ORIGINAL_TERMIOS = Some(term.clone());
        term.c_lflag &= !(ICANON | ECHO | ISIG | IEXTEN);
        term.c_iflag &= !(IXON | ICRNL);
        term.c_oflag &= !OPOST;
        term.c_cc[VMIN] = 1;
        term.c_cc[VTIME] = 0;
        if tcsetattr(STDIN_FD, TCSANOW, &term) != 0 {
            return Err("tcsetattr failed".into());
        }
        Ok(())
    }
}

/// Disable terminal raw mode, restore original settings.
pub fn raw_mode_disable() -> Result<(), String> {
    unsafe {
        if let Some(ref term) = ORIGINAL_TERMIOS {
            if tcsetattr(STDIN_FD, TCSANOW, term) != 0 {
                return Err("tcsetattr restore failed".into());
            }
        }
        Ok(())
    }
}

/// Read a single byte from stdin.
pub fn read_byte() -> Option<u8> {
    let mut buf = [0u8; 1];
    unsafe {
        let n = read(STDIN_FD, buf.as_mut_ptr(), 1);
        if n == 1 { Some(buf[0]) } else { None }
    }
}

/// Read a byte with timeout (in deciseconds, 0 = non-blocking).
pub fn read_byte_timeout(deciseconds: u8) -> Option<u8> {
    unsafe {
        let mut term = std::mem::zeroed::<Termios>();
        tcgetattr(STDIN_FD, &mut term);
        let saved_vmin = term.c_cc[VMIN];
        let saved_vtime = term.c_cc[VTIME];
        term.c_cc[VMIN] = 0;
        term.c_cc[VTIME] = deciseconds;
        tcsetattr(STDIN_FD, TCSANOW, &term);

        let mut buf = [0u8; 1];
        let n = read(STDIN_FD, buf.as_mut_ptr(), 1);

        term.c_cc[VMIN] = saved_vmin;
        term.c_cc[VTIME] = saved_vtime;
        tcsetattr(STDIN_FD, TCSANOW, &term);

        if n == 1 { Some(buf[0]) } else { None }
    }
}

// ── SDL2 FFI ─────────────────────────────────────────────────────────

pub const SDL_QUIT_EVENT: u32 = 0x100;
pub const SDL_KEYDOWN: u32 = 0x300;
pub const SDL_KEYUP: u32 = 0x301;

#[cfg(has_sdl2)]
mod sdl2_ffi {
    #[allow(non_camel_case_types)]
    pub type SDL_Window = *mut std::ffi::c_void;
    #[allow(non_camel_case_types)]
    pub type SDL_Renderer = *mut std::ffi::c_void;

    pub const SDL_INIT_VIDEO: u32 = 0x00000020;
    pub const SDL_INIT_AUDIO: u32 = 0x00000010;
    pub const SDL_WINDOWPOS_CENTERED: i32 = 0x2FFF0000u32 as i32;
    pub const SDL_WINDOW_SHOWN: u32 = 0x00000004;
    pub const SDL_KEYDOWN: u32 = 0x300;
    pub const SDL_KEYUP: u32 = 0x301;

    extern "C" {
        pub fn SDL_Init(flags: u32) -> i32;
        pub fn SDL_Quit();
        pub fn SDL_CreateWindow(title: *const u8, x: i32, y: i32, w: i32, h: i32, flags: u32) -> SDL_Window;
        pub fn SDL_DestroyWindow(window: SDL_Window);
        pub fn SDL_CreateRenderer(window: SDL_Window, index: i32, flags: u32) -> SDL_Renderer;
        pub fn SDL_DestroyRenderer(renderer: SDL_Renderer);
        pub fn SDL_SetRenderDrawColor(renderer: SDL_Renderer, r: u8, g: u8, b: u8, a: u8) -> i32;
        pub fn SDL_RenderClear(renderer: SDL_Renderer) -> i32;
        pub fn SDL_RenderPresent(renderer: SDL_Renderer);
        pub fn SDL_RenderDrawPoint(renderer: SDL_Renderer, x: i32, y: i32) -> i32;
        pub fn SDL_RenderDrawLine(renderer: SDL_Renderer, x1: i32, y1: i32, x2: i32, y2: i32) -> i32;
        pub fn SDL_RenderFillRect(renderer: SDL_Renderer, rect: *const super::SDL_Rect) -> i32;
        pub fn SDL_PollEvent(event: *mut super::SDL_Event) -> i32;
        pub fn SDL_Delay(ms: u32);
        pub fn SDL_GetTicks() -> u32;
        pub fn SDL_GetError() -> *const u8;
    }
}

#[cfg(has_sdl2)]
#[repr(C)]
struct SDL_Rect {
    x: i32, y: i32, w: i32, h: i32,
}

#[cfg(has_sdl2)]
#[repr(C)]
struct SDL_Event {
    type_: u32,
    padding: [u8; 52],
}

/// SDL2 window handle for MAGI programs.
pub struct SdlContext {
    #[cfg(has_sdl2)]
    window: sdl2_ffi::SDL_Window,
    #[cfg(has_sdl2)]
    renderer: sdl2_ffi::SDL_Renderer,
    pub width: i32,
    pub height: i32,
}

impl SdlContext {
    #[cfg(has_sdl2)]
    pub fn new(title: &str, width: i32, height: i32) -> Result<Self, String> {
        unsafe {
            if sdl2_ffi::SDL_Init(sdl2_ffi::SDL_INIT_VIDEO | sdl2_ffi::SDL_INIT_AUDIO) != 0 {
                return Err(format!("SDL_Init failed: {}", get_sdl_error()));
            }
            let title_c = std::ffi::CString::new(title).unwrap_or_default();
            let window = sdl2_ffi::SDL_CreateWindow(
                title_c.as_ptr() as *const u8,
                sdl2_ffi::SDL_WINDOWPOS_CENTERED, sdl2_ffi::SDL_WINDOWPOS_CENTERED,
                width, height, sdl2_ffi::SDL_WINDOW_SHOWN,
            );
            if window.is_null() {
                return Err(format!("SDL_CreateWindow failed: {}", get_sdl_error()));
            }
            let renderer = sdl2_ffi::SDL_CreateRenderer(window, -1, 0);
            if renderer.is_null() {
                sdl2_ffi::SDL_DestroyWindow(window);
                return Err(format!("SDL_CreateRenderer failed: {}", get_sdl_error()));
            }
            Ok(SdlContext { window, renderer, width, height })
        }
    }

    #[cfg(not(has_sdl2))]
    pub fn new(_title: &str, _width: i32, _height: i32) -> Result<Self, String> {
        Err("SDL2 not available — install libsdl2-dev".into())
    }

    #[cfg(has_sdl2)]
    pub fn set_color(&self, r: u8, g: u8, b: u8) {
        unsafe { sdl2_ffi::SDL_SetRenderDrawColor(self.renderer, r, g, b, 255); }
    }
    #[cfg(not(has_sdl2))]
    pub fn set_color(&self, _r: u8, _g: u8, _b: u8) {}

    #[cfg(has_sdl2)]
    pub fn clear(&self) { unsafe { sdl2_ffi::SDL_RenderClear(self.renderer); } }
    #[cfg(not(has_sdl2))]
    pub fn clear(&self) {}

    #[cfg(has_sdl2)]
    pub fn present(&self) { unsafe { sdl2_ffi::SDL_RenderPresent(self.renderer); } }
    #[cfg(not(has_sdl2))]
    pub fn present(&self) {}

    #[cfg(has_sdl2)]
    pub fn draw_pixel(&self, x: i32, y: i32) {
        unsafe { sdl2_ffi::SDL_RenderDrawPoint(self.renderer, x, y); }
    }
    #[cfg(not(has_sdl2))]
    pub fn draw_pixel(&self, _x: i32, _y: i32) {}

    #[cfg(has_sdl2)]
    pub fn draw_line(&self, x1: i32, y1: i32, x2: i32, y2: i32) {
        unsafe { sdl2_ffi::SDL_RenderDrawLine(self.renderer, x1, y1, x2, y2); }
    }
    #[cfg(not(has_sdl2))]
    pub fn draw_line(&self, _x1: i32, _y1: i32, _x2: i32, _y2: i32) {}

    #[cfg(has_sdl2)]
    pub fn fill_rect(&self, x: i32, y: i32, w: i32, h: i32) {
        let rect = SDL_Rect { x, y, w, h };
        unsafe { sdl2_ffi::SDL_RenderFillRect(self.renderer, &rect); }
    }
    #[cfg(not(has_sdl2))]
    pub fn fill_rect(&self, _x: i32, _y: i32, _w: i32, _h: i32) {}

    #[cfg(has_sdl2)]
    pub fn poll_event(&self) -> Option<(u32, u32)> {
        let mut event = SDL_Event { type_: 0, padding: [0; 52] };
        unsafe {
            if sdl2_ffi::SDL_PollEvent(&mut event) != 0 {
                let scancode = if event.type_ == sdl2_ffi::SDL_KEYDOWN || event.type_ == sdl2_ffi::SDL_KEYUP {
                    u32::from_le_bytes([event.padding[4], event.padding[5], event.padding[6], event.padding[7]])
                } else { 0 };
                Some((event.type_, scancode))
            } else {
                None
            }
        }
    }
    #[cfg(not(has_sdl2))]
    pub fn poll_event(&self) -> Option<(u32, u32)> { None }
}

#[cfg(has_sdl2)]
impl Drop for SdlContext {
    fn drop(&mut self) {
        unsafe {
            sdl2_ffi::SDL_DestroyRenderer(self.renderer);
            sdl2_ffi::SDL_DestroyWindow(self.window);
            sdl2_ffi::SDL_Quit();
        }
    }
}

#[cfg(has_sdl2)]
pub unsafe fn sdl_delay(ms: u32) { sdl2_ffi::SDL_Delay(ms); }
#[cfg(not(has_sdl2))]
pub unsafe fn sdl_delay(_ms: u32) {}

#[cfg(has_sdl2)]
pub unsafe fn sdl_get_ticks() -> u32 { sdl2_ffi::SDL_GetTicks() }
#[cfg(not(has_sdl2))]
pub unsafe fn sdl_get_ticks() -> u32 { 0 }

#[cfg(has_sdl2)]
fn get_sdl_error() -> String {
    unsafe {
        let ptr = sdl2_ffi::SDL_GetError();
        if ptr.is_null() { return "unknown".into(); }
        let mut len = 0;
        while *ptr.add(len) != 0 { len += 1; }
        String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len)).to_string()
    }
}

// ── Audio (PulseAudio simple API) ────────────────────────────────────

/// Audio output stream.
pub struct AudioStream {
    #[cfg(has_pulseaudio)]
    handle: *mut std::ffi::c_void,
    pub sample_rate: u32,
}

#[cfg(has_pulseaudio)]
mod pulse_ffi {
    #[allow(non_camel_case_types)]
    pub type pa_simple = *mut std::ffi::c_void;

    #[repr(C)]
    pub struct pa_sample_spec {
        pub format: i32,
        pub rate: u32,
        pub channels: u8,
    }

    pub const PA_SAMPLE_S16LE: i32 = 3;
    pub const PA_STREAM_PLAYBACK: i32 = 1;

    extern "C" {
        pub fn pa_simple_new(
            server: *const u8, name: *const u8, dir: i32,
            dev: *const u8, stream_name: *const u8,
            ss: *const pa_sample_spec, map: *const std::ffi::c_void,
            attr: *const std::ffi::c_void, error: *mut i32,
        ) -> pa_simple;
        pub fn pa_simple_write(s: pa_simple, data: *const u8, bytes: usize, error: *mut i32) -> i32;
        pub fn pa_simple_drain(s: pa_simple, error: *mut i32) -> i32;
        pub fn pa_simple_free(s: pa_simple);
    }
}

impl AudioStream {
    #[cfg(has_pulseaudio)]
    pub fn new(sample_rate: u32) -> Result<Self, String> {
        let spec = pulse_ffi::pa_sample_spec {
            format: pulse_ffi::PA_SAMPLE_S16LE,
            rate: sample_rate,
            channels: 1,
        };
        let app_name = std::ffi::CString::new("magi").unwrap_or_default();
        let stream_name = std::ffi::CString::new("playback").unwrap_or_default();
        let mut error: i32 = 0;
        let handle = unsafe {
            pulse_ffi::pa_simple_new(
                std::ptr::null(), app_name.as_ptr() as *const u8,
                pulse_ffi::PA_STREAM_PLAYBACK,
                std::ptr::null(), stream_name.as_ptr() as *const u8,
                &spec, std::ptr::null(), std::ptr::null(), &mut error,
            )
        };
        if handle.is_null() {
            return Err(format!("pa_simple_new failed (error {})", error));
        }
        Ok(AudioStream { handle, sample_rate })
    }

    #[cfg(not(has_pulseaudio))]
    pub fn new(_sample_rate: u32) -> Result<Self, String> {
        Err("PulseAudio not available — install libpulse-dev".into())
    }

    #[cfg(has_pulseaudio)]
    pub fn write_samples(&self, samples: &[i16]) -> Result<(), String> {
        let bytes = unsafe {
            std::slice::from_raw_parts(samples.as_ptr() as *const u8, samples.len() * 2)
        };
        let mut error: i32 = 0;
        let ret = unsafe { pulse_ffi::pa_simple_write(self.handle, bytes.as_ptr(), bytes.len(), &mut error) };
        if ret < 0 { Err(format!("pa_simple_write failed ({})", error)) } else { Ok(()) }
    }
    #[cfg(not(has_pulseaudio))]
    pub fn write_samples(&self, _samples: &[i16]) -> Result<(), String> {
        Err("PulseAudio not available".into())
    }

    #[cfg(has_pulseaudio)]
    pub fn drain(&self) -> Result<(), String> {
        let mut error: i32 = 0;
        let ret = unsafe { pulse_ffi::pa_simple_drain(self.handle, &mut error) };
        if ret < 0 { Err(format!("drain failed ({})", error)) } else { Ok(()) }
    }
    #[cfg(not(has_pulseaudio))]
    pub fn drain(&self) -> Result<(), String> {
        Err("PulseAudio not available".into())
    }
}

#[cfg(has_pulseaudio)]
impl Drop for AudioStream {
    fn drop(&mut self) {
        unsafe { pulse_ffi::pa_simple_free(self.handle); }
    }
}
