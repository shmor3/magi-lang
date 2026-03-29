// MAGI SDL2 wrapper for compiled binaries
// Provides __magi_sdl_* functions called by the runtime

#ifdef __has_include
#if __has_include(<SDL2/SDL.h>)
#define HAS_SDL2 1
#include <SDL2/SDL.h>
#elif __has_include(<SDL.h>)
#define HAS_SDL2 1
#include <SDL.h>
#else
#define HAS_SDL2 0
#endif
#else
#define HAS_SDL2 0
#endif

#include <stdint.h>
#include <stdlib.h>

#if HAS_SDL2

typedef struct {
    SDL_Window* window;
    SDL_Renderer* renderer;
} SdlCtx;

void* __magi_sdl_init(const char* title, int w, int h) {
    if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_EVENTS) < 0) return NULL;
    SDL_Window* win = SDL_CreateWindow(title, SDL_WINDOWPOS_CENTERED, SDL_WINDOWPOS_CENTERED, w, h, SDL_WINDOW_SHOWN);
    if (!win) return NULL;
    SDL_Renderer* ren = SDL_CreateRenderer(win, -1, SDL_RENDERER_ACCELERATED);
    if (!ren) { SDL_DestroyWindow(win); return NULL; }
    SdlCtx* ctx = (SdlCtx*)malloc(sizeof(SdlCtx));
    ctx->window = win;
    ctx->renderer = ren;
    return ctx;
}

void __magi_sdl_set_color(void* handle, int r, int g, int b) {
    SdlCtx* ctx = (SdlCtx*)handle;
    if (ctx) SDL_SetRenderDrawColor(ctx->renderer, r, g, b, 255);
}

void __magi_sdl_clear(void* handle) {
    SdlCtx* ctx = (SdlCtx*)handle;
    if (ctx) SDL_RenderClear(ctx->renderer);
}

void __magi_sdl_present(void* handle) {
    SdlCtx* ctx = (SdlCtx*)handle;
    if (ctx) SDL_RenderPresent(ctx->renderer);
}

void __magi_sdl_fill_rect(void* handle, int x, int y, int w, int h) {
    SdlCtx* ctx = (SdlCtx*)handle;
    if (!ctx) return;
    SDL_Rect rect = {x, y, w, h};
    SDL_RenderFillRect(ctx->renderer, &rect);
}

void __magi_sdl_draw_pixel(void* handle, int x, int y) {
    SdlCtx* ctx = (SdlCtx*)handle;
    if (ctx) SDL_RenderDrawPoint(ctx->renderer, x, y);
}

void __magi_sdl_draw_line(void* handle, int x1, int y1, int x2, int y2) {
    SdlCtx* ctx = (SdlCtx*)handle;
    if (ctx) SDL_RenderDrawLine(ctx->renderer, x1, y1, x2, y2);
}

// NaN-boxing helpers (must match magi_runtime.c)
#define NANBOX_SIG   ((uint64_t)0xFFF8000000000000ULL)
#define PAYLOAD_MASK ((uint64_t)0x0000FFFFFFFFFFFFULL)
#define TAG_SHIFT    48
#define TAG_NULL   0
#define TAG_I64    2
#define TAG_STRING 3
#define TAG_MAP    5

static int64_t make_null(void) { return (int64_t)(NANBOX_SIG | ((uint64_t)TAG_NULL << TAG_SHIFT)); }
static int64_t make_int(int64_t n) { return (int64_t)(NANBOX_SIG | ((uint64_t)TAG_I64 << TAG_SHIFT) | ((uint64_t)n & PAYLOAD_MASK)); }
static int64_t make_string(const char* s) { return (int64_t)(NANBOX_SIG | ((uint64_t)TAG_STRING << TAG_SHIFT) | ((uint64_t)(uintptr_t)s & PAYLOAD_MASK)); }

// Forward declare map creation from runtime
extern int64_t __magi_map_new(int32_t count, int64_t* entries);

int64_t __magi_sdl_poll_event(void* handle) {
    SdlCtx* ctx = (SdlCtx*)handle;
    if (!ctx) return make_null();
    SDL_Event ev;
    if (!SDL_PollEvent(&ev)) return make_null();
    // Build {"type": N, "scancode": N} map
    int64_t entries[4];
    entries[0] = make_string("type");
    entries[1] = make_int(ev.type);
    entries[2] = make_string("scancode");
    entries[3] = make_int(ev.type == SDL_KEYDOWN || ev.type == SDL_KEYUP ? ev.key.keysym.scancode : 0);
    return __magi_map_new(2, entries);
}

void __magi_sdl_delay(int ms) {
    SDL_Delay(ms);
}

int __magi_sdl_ticks(void* handle) {
    (void)handle;
    return (int)SDL_GetTicks();
}

void __magi_sdl_destroy(void* handle) {
    SdlCtx* ctx = (SdlCtx*)handle;
    if (!ctx) return;
    SDL_DestroyRenderer(ctx->renderer);
    SDL_DestroyWindow(ctx->window);
    SDL_Quit();
    free(ctx);
}

#else
// No SDL2 — stub implementations
#ifndef NANBOX_SIG
#define NANBOX_SIG   ((uint64_t)0xFFF8000000000000ULL)
#define PAYLOAD_MASK ((uint64_t)0x0000FFFFFFFFFFFFULL)
#define TAG_SHIFT    48
#define TAG_NULL   0
#endif
void* __magi_sdl_init(const char* t, int w, int h) { (void)t;(void)w;(void)h; return NULL; }
void __magi_sdl_set_color(void* h, int r, int g, int b) { (void)h;(void)r;(void)g;(void)b; }
void __magi_sdl_clear(void* h) { (void)h; }
void __magi_sdl_present(void* h) { (void)h; }
void __magi_sdl_fill_rect(void* h, int x, int y, int w, int hh) { (void)h;(void)x;(void)y;(void)w;(void)hh; }
void __magi_sdl_draw_pixel(void* h, int x, int y) { (void)h;(void)x;(void)y; }
void __magi_sdl_draw_line(void* h, int x1, int y1, int x2, int y2) { (void)h;(void)x1;(void)y1;(void)x2;(void)y2; }
int64_t __magi_sdl_poll_event(void* h) { (void)h; return (int64_t)(NANBOX_SIG | ((uint64_t)TAG_NULL << TAG_SHIFT)); }
void __magi_sdl_delay(int ms) { (void)ms; }
int __magi_sdl_ticks(void* h) { (void)h; return 0; }
void __magi_sdl_destroy(void* h) { (void)h; }
#endif
