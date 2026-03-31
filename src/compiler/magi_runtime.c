// MAGI LLVM Backend Runtime Library
// Provides NaN-boxed value operations, I/O, collections, and type dispatch.
// Compiled alongside user programs and linked into the final binary.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <math.h>
#include <errno.h>
#include <sys/stat.h>
#include <dirent.h>
#ifdef _WIN32
#include <windows.h>
#include <direct.h>
#include <process.h>
#define getcwd _getcwd
#define getpid _getpid
#else
#include <time.h>
#include <unistd.h>
#endif

// Command line args (set by main before calling __main)
int __magi_argc = 0;
char** __magi_argv = NULL;

// ===== Arena Allocator for Short-Lived Strings =====
// Used by print/println to avoid malloc for temporary string formatting.
// Strings allocated here are valid until __magi_arena_reset() is called.

#define ARENA_BLOCK_SIZE (1024 * 1024)  // 1MB blocks

typedef struct ArenaBlock {
    char* data;
    size_t used;
    size_t capacity;
    struct ArenaBlock* next;
} ArenaBlock;

static ArenaBlock* arena_head = NULL;
static ArenaBlock* arena_current = NULL;
static int arena_mode = 0;  // when 1, magi_val_to_dyn_str uses arena

static void* arena_alloc(size_t size) {
    // Align to 8 bytes
    size = (size + 7) & ~7;

    if (!arena_current || arena_current->used + size > arena_current->capacity) {
        // Need a new block
        size_t block_size = size > ARENA_BLOCK_SIZE ? size : ARENA_BLOCK_SIZE;
        ArenaBlock* block = (ArenaBlock*)malloc(sizeof(ArenaBlock));
        block->data = (char*)malloc(block_size);
        block->used = 0;
        block->capacity = block_size;
        block->next = NULL;
        if (arena_current) arena_current->next = block;
        arena_current = block;
        if (!arena_head) arena_head = block;
    }

    void* ptr = arena_current->data + arena_current->used;
    arena_current->used += size;
    return ptr;
}

// Reset arena: reuse all blocks without freeing
void __magi_arena_reset(void) {
    ArenaBlock* block = arena_head;
    while (block) {
        block->used = 0;
        block = block->next;
    }
    arena_current = arena_head;
}

// Enter/leave arena mode: routes string_concat and to_string through arena
void __magi_arena_enter(void) { arena_mode = 1; }
void __magi_arena_leave(void) { arena_mode = 0; }

// ===== Allocation Tracking =====
static size_t magi_total_malloc = 0;
static size_t magi_malloc_count = 0;
static size_t magi_gc_warn_threshold = 512 * 1024 * 1024; // 512MB
static int magi_gc_warned = 0;

static void* tracked_malloc(size_t size) {
    void* p = malloc(size);
    if (p) {
        magi_total_malloc += size;
        magi_malloc_count++;
        if (!magi_gc_warned && magi_total_malloc > magi_gc_warn_threshold) {
            fprintf(stderr, "[magi] warning: heap allocations exceed %zuMB (%zu allocs). "
                    "Consider calling __arena_reset() in hot loops.\n",
                    magi_total_malloc / (1024 * 1024), magi_malloc_count);
            magi_gc_warned = 1;
        }
    }
    return p;
}

// Query allocation stats from MAGI code
int64_t __magi_heap_allocated(void);

// Arena-aware strdup: copies string into arena when arena_mode is active
static char* arena_strdup(const char* s) {
    size_t len = strlen(s);
    char* p = (char*)arena_alloc(len + 1);
    memcpy(p, s, len + 1);
    return p;
}

// Arena-aware malloc: returns arena memory when arena_mode is active
static void* arena_malloc(size_t size) {
    return arena_alloc(size);
}

// Arena-aware realloc: copies data to new arena allocation when arena_mode is active
static void* arena_realloc(void* ptr, size_t old_size, size_t new_size) {
    void* p = arena_alloc(new_size);
    if (ptr && old_size > 0) memcpy(p, ptr, old_size < new_size ? old_size : new_size);
    return p;
}

// ===== NaN-Boxing Constants =====
#define NANBOX_SIG   ((uint64_t)0xFFF8000000000000ULL)
#define NANBOX_MASK  ((uint64_t)0xFFF8000000000000ULL)
#define PAYLOAD_MASK ((uint64_t)0x0000FFFFFFFFFFFFULL)
#define TAG_SHIFT    48
#define TAG_BITS     ((uint64_t)7)

#define TAG_NULL   0
#define TAG_BOOL   1
#define TAG_I64    2
#define TAG_STRING 3
#define TAG_ARRAY  4
#define TAG_MAP    5

// ===== Value Helpers =====
static inline int magi_is_tagged(int64_t val) {
    return ((uint64_t)val & NANBOX_MASK) == NANBOX_SIG;
}

static inline int magi_get_tag(int64_t val) {
    if (!magi_is_tagged(val)) return 8; // F64 sentinel
    return (int)(((uint64_t)val >> TAG_SHIFT) & TAG_BITS);
}

static inline int64_t magi_get_payload(int64_t val) {
    return (int64_t)((uint64_t)val & PAYLOAD_MASK);
}

static inline int64_t magi_sext48(int64_t payload) {
    return (payload << 16) >> 16;
}

static inline int64_t magi_make_null(void) {
    return (int64_t)(NANBOX_SIG | ((uint64_t)TAG_NULL << TAG_SHIFT));
}

static inline int64_t magi_make_bool(int b) {
    return (int64_t)(NANBOX_SIG | ((uint64_t)TAG_BOOL << TAG_SHIFT) | (uint64_t)(b ? 1 : 0));
}

static inline int64_t magi_make_int(int64_t n) {
    return (int64_t)(NANBOX_SIG | ((uint64_t)TAG_I64 << TAG_SHIFT) | ((uint64_t)n & PAYLOAD_MASK));
}

static inline int64_t magi_make_float(double d) {
    int64_t bits;
    memcpy(&bits, &d, sizeof(bits));
    return bits;
}

static inline int64_t magi_make_string(const char* s) {
    return (int64_t)(NANBOX_SIG | ((uint64_t)TAG_STRING << TAG_SHIFT) | ((uint64_t)(uintptr_t)s & PAYLOAD_MASK));
}

static inline int64_t magi_as_int(int64_t val) {
    if (magi_is_tagged(val)) {
        if (magi_get_tag(val) == TAG_I64) return magi_sext48(magi_get_payload(val));
        if (magi_get_tag(val) == TAG_BOOL) return magi_get_payload(val);
        return 0;
    }
    double d;
    memcpy(&d, &val, sizeof(d));
    return (int64_t)d;
}

static inline double magi_as_float(int64_t val) {
    if (!magi_is_tagged(val)) {
        double d;
        memcpy(&d, &val, sizeof(d));
        return d;
    }
    if (magi_get_tag(val) == TAG_I64) return (double)magi_sext48(magi_get_payload(val));
    if (magi_get_tag(val) == TAG_BOOL) return (double)magi_get_payload(val);
    return 0.0;
}

static inline int magi_as_bool(int64_t val) {
    if (magi_is_tagged(val)) {
        switch (magi_get_tag(val)) {
            case TAG_NULL: return 0;
            case TAG_BOOL: return magi_get_payload(val) != 0;
            case TAG_I64: return magi_get_payload(val) != 0;
            default: return 1; // string, array, map are truthy
        }
    }
    double d;
    memcpy(&d, &val, sizeof(d));
    return d != 0.0;
}

static inline const char* magi_as_string(int64_t val) {
    if (magi_is_tagged(val) && magi_get_tag(val) == TAG_STRING) {
        return (const char*)(uintptr_t)magi_get_payload(val);
    }
    return "";
}

// ===== Array Type =====
typedef struct {
    int64_t* data;
    int32_t len;
    int32_t cap;
} MagiArray;

static inline MagiArray* magi_array_ptr(int64_t val) {
    if (!magi_is_tagged(val) || magi_get_tag(val) != TAG_ARRAY) return NULL;
    return (MagiArray*)(uintptr_t)magi_get_payload(val);
}

static inline int64_t magi_make_array_val(MagiArray* arr) {
    return (int64_t)(NANBOX_SIG | ((uint64_t)TAG_ARRAY << TAG_SHIFT) | ((uint64_t)(uintptr_t)arr & PAYLOAD_MASK));
}

// ===== Map Type =====
typedef struct {
    char** keys;
    int64_t* values;
    int32_t len;
    int32_t cap;
    // Hash table: open addressing with linear probing
    uint32_t* hashes;     // pre-computed FNV-1a hash per key (parallel to keys[])
    int32_t* buckets;     // bucket[hash % bucket_count] = index into keys/values, -1 = empty
    int32_t bucket_count; // always a power of 2
} MagiMap;

static uint32_t fnv1a(const char* key) {
    uint32_t hash = 2166136261u;
    while (*key) {
        hash ^= (uint8_t)*key++;
        hash *= 16777619u;
    }
    return hash;
}

static void magi_map_rehash(MagiMap* map) {
    int32_t new_bc = map->bucket_count * 2;
    if (new_bc < 16) new_bc = 16;
    free(map->buckets);
    map->buckets = (int32_t*)malloc(new_bc * sizeof(int32_t));
    memset(map->buckets, -1, new_bc * sizeof(int32_t));
    map->bucket_count = new_bc;
    for (int i = 0; i < map->len; i++) {
        uint32_t slot = map->hashes[i] & (uint32_t)(new_bc - 1);
        while (map->buckets[slot] != -1) slot = (slot + 1) & (uint32_t)(new_bc - 1);
        map->buckets[slot] = i;
    }
}

static void magi_map_init_hash(MagiMap* map) {
    int32_t bc = 16;
    while (bc < map->cap * 2) bc *= 2;
    map->hashes = (uint32_t*)malloc(sizeof(uint32_t) * map->cap);
    map->buckets = (int32_t*)malloc(bc * sizeof(int32_t));
    memset(map->buckets, -1, bc * sizeof(int32_t));
    map->bucket_count = bc;
    for (int i = 0; i < map->len; i++) {
        uint32_t h = fnv1a(map->keys[i]);
        map->hashes[i] = h;
        uint32_t slot = h & (uint32_t)(bc - 1);
        while (map->buckets[slot] != -1) slot = (slot + 1) & (uint32_t)(bc - 1);
        map->buckets[slot] = i;
    }
}

static inline MagiMap* magi_map_ptr(int64_t val) {
    if (!magi_is_tagged(val) || magi_get_tag(val) != TAG_MAP) return NULL;
    return (MagiMap*)(uintptr_t)magi_get_payload(val);
}

static inline int64_t magi_make_map_val(MagiMap* map) {
    return (int64_t)(NANBOX_SIG | ((uint64_t)TAG_MAP << TAG_SHIFT) | ((uint64_t)(uintptr_t)map & PAYLOAD_MASK));
}

// ===== Forward declarations =====
static char* magi_val_to_dyn_str(int64_t val, int for_display);
static void magi_val_to_str(int64_t val, char* buf, int bufsize);
int64_t __magi_string_concat(int64_t a_val, int64_t b_val);
int64_t __magi_string_len(int64_t val);
int64_t __magi_map_get(int64_t map_val, int64_t key_val);
void __magi_map_set(int64_t map_val, int64_t key_val, int64_t val);
int64_t __magi_byte_slice(int64_t arr_val, int64_t start_val, int64_t len_val);
static inline int magi_is_byte_array(MagiArray* arr) { return arr && arr->cap == -1; }
static inline int64_t magi_byte_array_get(MagiArray* arr, int64_t idx) {
    if (!arr || idx < 0 || idx >= arr->len) return magi_make_null();
    const unsigned char* bytes = (const unsigned char*)(uintptr_t)arr->data;
    return magi_make_int(bytes[idx]);
}
int64_t __magi_array_len(int64_t arr_val);
int64_t __magi_array_push(int64_t arr_val, int64_t val);
int64_t __magi_to_string(int64_t val);
int64_t __magi_runtime_call(const char* name, int32_t argc, int64_t* args);
int64_t __magi_call_fn(int64_t fn_val, int32_t call_argc, int64_t* call_args);

// ===== Print =====
void __magi_print(int64_t val) {
    arena_mode = 1;
    char* s = magi_val_to_dyn_str(val, 1);
    printf("%s\n", s);
    fflush(stdout);
    // No free — s is arena-allocated, reset in bulk
    arena_mode = 0;
}

// Dynamic string builder for value formatting.
// When arena_mode is active, all allocations go to the arena (no free needed).
// When arena_mode is off, uses malloc/strdup as before (caller must free).
static char* magi_val_to_dyn_str(int64_t val, int for_display) {
    int use_arena = arena_mode;
    if (!magi_is_tagged(val)) {
        double d;
        memcpy(&d, &val, sizeof(d));
        char buf[64];
        if (d == (double)(int64_t)d && fabs(d) < 1e15 && !isinf(d) && !isnan(d))
            snprintf(buf, sizeof(buf), "%lld", (long long)(int64_t)d);
        else
            snprintf(buf, sizeof(buf), "%.15g", d);
        return use_arena ? arena_strdup(buf) : strdup(buf);
    }
    switch (magi_get_tag(val)) {
        case TAG_NULL: return use_arena ? arena_strdup("null") : strdup("null");
        case TAG_BOOL: return use_arena ? arena_strdup(magi_get_payload(val) ? "true" : "false") : strdup(magi_get_payload(val) ? "true" : "false");
        case TAG_I64: { char buf[32]; snprintf(buf, sizeof(buf), "%lld", (long long)magi_sext48(magi_get_payload(val))); return use_arena ? arena_strdup(buf) : strdup(buf); }
        case TAG_STRING: {
            const char* s = magi_as_string(val);
            if (for_display) return use_arena ? arena_strdup(s) : strdup(s);
            size_t len = strlen(s);
            char* r = (char*)(use_arena ? arena_malloc(len + 3) : malloc(len + 3));
            r[0] = '"'; memcpy(r+1, s, len); r[len+1] = '"'; r[len+2] = '\0';
            return r;
        }
        case TAG_ARRAY: {
            MagiArray* arr = magi_array_ptr(val);
            if (!arr) return use_arena ? arena_strdup("[]") : strdup("[]");
            size_t cap = 256, pos = 0;
            char* buf = (char*)(use_arena ? arena_malloc(cap) : malloc(cap));
            buf[pos++] = '[';
            for (int i = 0; i < arr->len; i++) {
                if (i > 0) { buf[pos++] = ','; buf[pos++] = ' '; }
                char* elem = magi_val_to_dyn_str(arr->data[i], for_display);
                size_t elen = strlen(elem);
                while (pos + elen + 10 > cap) { size_t old = cap; cap *= 2; buf = (char*)(use_arena ? arena_realloc(buf, old, cap) : realloc(buf, cap)); }
                memcpy(buf + pos, elem, elen); pos += elen;
                if (!use_arena) free(elem);
            }
            if (pos + 2 > cap) { size_t old = cap; cap += 4; buf = (char*)(use_arena ? arena_realloc(buf, old, cap) : realloc(buf, cap)); }
            buf[pos++] = ']'; buf[pos] = '\0';
            return buf;
        }
        case TAG_MAP: {
            MagiMap* map = magi_map_ptr(val);
            if (!map) return use_arena ? arena_strdup("{}") : strdup("{}");
            size_t cap = 256, pos = 0;
            char* buf = (char*)(use_arena ? arena_malloc(cap) : malloc(cap));
            buf[pos++] = '{';
            for (int i = 0; i < map->len; i++) {
                if (i > 0) { buf[pos++] = ','; buf[pos++] = ' '; }
                char* v = magi_val_to_dyn_str(map->values[i], for_display);
                size_t klen = strlen(map->keys[i]), vlen = strlen(v);
                while (pos + klen + vlen + 10 > cap) { size_t old = cap; cap *= 2; buf = (char*)(use_arena ? arena_realloc(buf, old, cap) : realloc(buf, cap)); }
                memcpy(buf+pos, map->keys[i], klen); pos += klen;
                buf[pos++] = ':'; buf[pos++] = ' ';
                memcpy(buf+pos, v, vlen); pos += vlen;
                if (!use_arena) free(v);
            }
            if (pos + 2 > cap) { size_t old = cap; cap += 4; buf = (char*)(use_arena ? arena_realloc(buf, old, cap) : realloc(buf, cap)); }
            buf[pos++] = '}'; buf[pos] = '\0';
            return buf;
        }
        default: return use_arena ? arena_strdup("<unknown>") : strdup("<unknown>");
    }
}

static void magi_val_to_str(int64_t val, char* buf, int bufsize) {
    char* s = magi_val_to_dyn_str(val, 0);
    strncpy(buf, s, bufsize - 1);
    buf[bufsize - 1] = '\0';
    free(s);
}

// ===== Array Operations =====
int64_t __magi_array_new(int32_t count, int64_t* elements) {
    MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
    arr->len = count;
    arr->cap = count > 8 ? count : 8;
    arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
    if (elements) {
        for (int i = 0; i < count; i++) arr->data[i] = elements[i];
    } else {
        for (int i = 0; i < count; i++) arr->data[i] = magi_make_null();
    }
    return magi_make_array_val(arr);
}

int64_t __magi_array_get(int64_t arr_val, int64_t idx_val) {
    // If the object is a map, delegate to map_get
    if (magi_is_tagged(arr_val) && magi_get_tag(arr_val) == TAG_MAP) {
        return __magi_map_get(arr_val, idx_val);
    }
    MagiArray* arr = magi_array_ptr(arr_val);
    int64_t idx = magi_as_int(idx_val);
    if (!arr || idx < 0 || idx >= arr->len) return magi_make_null();
    if (magi_is_byte_array(arr)) return magi_byte_array_get(arr, idx);
    return arr->data[idx];
}

void __magi_array_set(int64_t arr_val, int64_t idx_val, int64_t val) {
    if (magi_is_tagged(arr_val) && magi_get_tag(arr_val) == TAG_MAP) {
        __magi_map_set(arr_val, idx_val, val);
        return;
    }
    MagiArray* arr = magi_array_ptr(arr_val);
    if (!arr) return;
    int64_t idx = magi_as_int(idx_val);

    // Copy-on-write for embedded byte arrays (cap == -1)
    if (magi_is_byte_array(arr)) {
        int len = arr->len;
        const unsigned char* raw = (const unsigned char*)(uintptr_t)arr->data;
        int64_t* new_data = (int64_t*)malloc(len * sizeof(int64_t));
        for (int i = 0; i < len; i++) {
            new_data[i] = magi_make_int(raw[i]);
        }
        arr->data = new_data;
        arr->cap = len;
    }

    if (idx >= 0 && idx < arr->len) {
        arr->data[idx] = val;
    }
}

int64_t __magi_array_len(int64_t arr_val) {
    MagiArray* arr = magi_array_ptr(arr_val);
    if (!arr) return magi_make_int(0);
    return magi_make_int(arr->len);
}

int64_t __magi_array_push(int64_t arr_val, int64_t val) {
    MagiArray* arr = magi_array_ptr(arr_val);
    if (!arr) return arr_val;
    if (arr->len >= arr->cap) {
        arr->cap = arr->cap < 8 ? 8 : arr->cap * 2;
        arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap);
    }
    arr->data[arr->len++] = val;
    return arr_val;
}

// ===== Map Operations =====
int64_t __magi_map_new(int32_t count, int64_t* entries) {
    MagiMap* map = (MagiMap*)malloc(sizeof(MagiMap));
    map->len = count;
    map->cap = count > 8 ? count : 8;
    map->keys = (char**)malloc(sizeof(char*) * map->cap);
    map->values = (int64_t*)malloc(sizeof(int64_t) * map->cap);
    for (int i = 0; i < count; i++) {
        const char* key_str = magi_as_string(entries[i * 2]);
        map->keys[i] = strdup(key_str);
        map->values[i] = entries[i * 2 + 1];
    }
    magi_map_init_hash(map);
    return magi_make_map_val(map);
}

int64_t __magi_map_get(int64_t map_val, int64_t key_val) {
    MagiMap* map = magi_map_ptr(map_val);
    const char* key = magi_as_string(key_val);
    if (!map || !key) return magi_make_null();
    uint32_t h = fnv1a(key);
    uint32_t mask = (uint32_t)(map->bucket_count - 1);
    uint32_t slot = h & mask;
    while (map->buckets[slot] != -1) {
        int idx = map->buckets[slot];
        if (map->hashes[idx] == h && strcmp(map->keys[idx], key) == 0)
            return map->values[idx];
        slot = (slot + 1) & mask;
    }
    return magi_make_null();
}

void __magi_map_set(int64_t map_val, int64_t key_val, int64_t val) {
    MagiMap* map = magi_map_ptr(map_val);
    const char* key = magi_as_string(key_val);
    if (!map || !key) return;
    uint32_t h = fnv1a(key);
    uint32_t mask = (uint32_t)(map->bucket_count - 1);
    uint32_t slot = h & mask;
    while (map->buckets[slot] != -1) {
        int idx = map->buckets[slot];
        if (map->hashes[idx] == h && strcmp(map->keys[idx], key) == 0) {
            map->values[idx] = val;
            return;
        }
        slot = (slot + 1) & mask;
    }
    // New key — grow parallel arrays if needed
    if (map->len >= map->cap) {
        map->cap = map->cap < 8 ? 8 : map->cap * 2;
        map->keys = (char**)realloc(map->keys, sizeof(char*) * map->cap);
        map->values = (int64_t*)realloc(map->values, sizeof(int64_t) * map->cap);
        map->hashes = (uint32_t*)realloc(map->hashes, sizeof(uint32_t) * map->cap);
    }
    int new_idx = map->len;
    map->keys[new_idx] = strdup(key);
    map->values[new_idx] = val;
    map->hashes[new_idx] = h;
    map->len++;
    // Rehash if load factor > 0.75
    if (map->len * 4 > map->bucket_count * 3) {
        magi_map_rehash(map);
    } else {
        map->buckets[slot] = new_idx;
    }
}

// ===== String Operations =====
int64_t __magi_string_concat(int64_t a_val, int64_t b_val) {
    const char* a = magi_as_string(a_val);
    const char* b = magi_as_string(b_val);
    size_t la = strlen(a), lb = strlen(b);
    char* result = arena_mode ? (char*)arena_alloc(la + lb + 1) : (char*)tracked_malloc(la + lb + 1);
    memcpy(result, a, la);
    memcpy(result + la, b, lb);
    result[la + lb] = '\0';
    return magi_make_string(result);
}

int64_t __magi_string_len(int64_t val) {
    const char* s = magi_as_string(val);
    return magi_make_int((int64_t)strlen(s));
}

// ===== to_string =====
int64_t __magi_to_string(int64_t val) {
    if (magi_is_tagged(val) && magi_get_tag(val) == TAG_STRING) return val;
    // When arena_mode is active, magi_val_to_dyn_str already uses arena
    char* s = magi_val_to_dyn_str(val, 1); // display mode (no quotes on strings)
    return magi_make_string(s); // s is heap or arena allocated
}

// ===== JSON serialization =====
static char* magi_to_json(int64_t val) {
    if (!magi_is_tagged(val)) {
        double d; memcpy(&d, &val, sizeof(d));
        char buf[64];
        if (d == (double)(int64_t)d && fabs(d) < 1e15 && !isinf(d) && !isnan(d))
            snprintf(buf, sizeof(buf), "%lld", (long long)(int64_t)d);
        else
            snprintf(buf, sizeof(buf), "%.15g", d);
        return strdup(buf);
    }
    switch (magi_get_tag(val)) {
        case TAG_NULL: return strdup("null");
        case TAG_BOOL: return strdup(magi_get_payload(val) ? "true" : "false");
        case TAG_I64: { char buf[32]; snprintf(buf, sizeof(buf), "%lld", (long long)magi_sext48(magi_get_payload(val))); return strdup(buf); }
        case TAG_STRING: {
            const char* s = magi_as_string(val);
            size_t len = strlen(s);
            char* r = (char*)malloc(len + 3);
            r[0] = '"'; memcpy(r+1, s, len); r[len+1] = '"'; r[len+2] = '\0';
            return r;
        }
        case TAG_ARRAY: {
            MagiArray* arr = magi_array_ptr(val);
            if (!arr) return strdup("[]");
            size_t cap = 256, pos = 0;
            char* buf = (char*)malloc(cap);
            buf[pos++] = '[';
            for (int i = 0; i < arr->len; i++) {
                if (i > 0) { buf[pos++] = ','; }
                char* elem = magi_to_json(arr->data[i]);
                size_t elen = strlen(elem);
                while (pos + elen + 4 > cap) { cap *= 2; buf = (char*)realloc(buf, cap); }
                memcpy(buf + pos, elem, elen); pos += elen;
                free(elem);
            }
            if (pos + 2 > cap) { cap += 4; buf = (char*)realloc(buf, cap); }
            buf[pos++] = ']'; buf[pos] = '\0';
            return buf;
        }
        case TAG_MAP: {
            MagiMap* map = magi_map_ptr(val);
            if (!map) return strdup("{}");
            size_t cap = 256, pos = 0;
            char* buf = (char*)malloc(cap);
            buf[pos++] = '{';
            for (int i = 0; i < map->len; i++) {
                if (i > 0) { buf[pos++] = ','; }
                char* v = magi_to_json(map->values[i]);
                size_t klen = strlen(map->keys[i]), vlen = strlen(v);
                while (pos + klen + vlen + 8 > cap) { cap *= 2; buf = (char*)realloc(buf, cap); }
                buf[pos++] = '"'; memcpy(buf+pos, map->keys[i], klen); pos += klen;
                buf[pos++] = '"'; buf[pos++] = ':';
                memcpy(buf+pos, v, vlen); pos += vlen;
                free(v);
            }
            if (pos + 2 > cap) { cap += 4; buf = (char*)realloc(buf, cap); }
            buf[pos++] = '}'; buf[pos] = '\0';
            return buf;
        }
        default: return strdup("null");
    }
}

// ===== Truthiness =====
int __magi_is_truthy(int64_t val) {
    return magi_as_bool(val);
}

// ===== Indirect Function Calls =====
// Function table generated by the LLVM codegen.
// Each entry is a wrapper: int64_t wrapper(int64_t* args, int32_t argc)
typedef int64_t (*magi_wrap_fn_t)(int64_t*, int32_t);
extern void* __magi_fn_table[];
extern int32_t __magi_fn_count;

int64_t __magi_call_fn(int64_t fn_val, int32_t call_argc, int64_t* call_args) {
    int64_t fn_idx = magi_sext48(magi_get_payload(fn_val));
    if (fn_idx < 0 || fn_idx >= __magi_fn_count) return magi_make_null();
    magi_wrap_fn_t wrapper = (magi_wrap_fn_t)__magi_fn_table[fn_idx];
    return wrapper(call_args, call_argc);
}

// ===== Collection Methods with Callbacks =====

static int64_t magi_method_map(int64_t arr_val, int64_t fn_val) {
    MagiArray* arr = magi_array_ptr(arr_val);
    if (!arr) return magi_make_null();
    MagiArray* result = (MagiArray*)malloc(sizeof(MagiArray));
    result->len = arr->len;
    result->cap = arr->len > 8 ? arr->len : 8;
    result->data = (int64_t*)malloc(sizeof(int64_t) * result->cap);
    for (int i = 0; i < arr->len; i++) {
        int64_t call_args[1] = { arr->data[i] };
        result->data[i] = __magi_call_fn(fn_val, 1, call_args);
    }
    return magi_make_array_val(result);
}

static int64_t magi_method_filter(int64_t arr_val, int64_t fn_val) {
    MagiArray* arr = magi_array_ptr(arr_val);
    if (!arr) return magi_make_null();
    MagiArray* result = (MagiArray*)malloc(sizeof(MagiArray));
    result->len = 0;
    result->cap = arr->len > 8 ? arr->len : 8;
    result->data = (int64_t*)malloc(sizeof(int64_t) * result->cap);
    for (int i = 0; i < arr->len; i++) {
        int64_t call_args[1] = { arr->data[i] };
        int64_t keep = __magi_call_fn(fn_val, 1, call_args);
        if (magi_as_bool(keep)) {
            result->data[result->len++] = arr->data[i];
        }
    }
    return magi_make_array_val(result);
}

static int64_t magi_method_reduce(int64_t arr_val, int64_t init, int64_t fn_val) {
    MagiArray* arr = magi_array_ptr(arr_val);
    if (!arr) return init;
    int64_t acc = init;
    for (int i = 0; i < arr->len; i++) {
        int64_t call_args[2] = { acc, arr->data[i] };
        acc = __magi_call_fn(fn_val, 2, call_args);
    }
    return acc;
}

static int64_t magi_method_for_each(int64_t arr_val, int64_t fn_val) {
    MagiArray* arr = magi_array_ptr(arr_val);
    if (!arr) return magi_make_null();
    for (int i = 0; i < arr->len; i++) {
        int64_t call_args[1] = { arr->data[i] };
        __magi_call_fn(fn_val, 1, call_args);
    }
    return magi_make_null();
}

static int64_t magi_method_find(int64_t arr_val, int64_t fn_val) {
    MagiArray* arr = magi_array_ptr(arr_val);
    if (!arr) return magi_make_null();
    for (int i = 0; i < arr->len; i++) {
        int64_t call_args[1] = { arr->data[i] };
        if (magi_as_bool(__magi_call_fn(fn_val, 1, call_args)))
            return arr->data[i];
    }
    return magi_make_null();
}

static int64_t magi_method_every(int64_t arr_val, int64_t fn_val) {
    MagiArray* arr = magi_array_ptr(arr_val);
    if (!arr) return magi_make_bool(1);
    for (int i = 0; i < arr->len; i++) {
        int64_t call_args[1] = { arr->data[i] };
        if (!magi_as_bool(__magi_call_fn(fn_val, 1, call_args)))
            return magi_make_bool(0);
    }
    return magi_make_bool(1);
}

static int64_t magi_method_some(int64_t arr_val, int64_t fn_val) {
    MagiArray* arr = magi_array_ptr(arr_val);
    if (!arr) return magi_make_bool(0);
    for (int i = 0; i < arr->len; i++) {
        int64_t call_args[1] = { arr->data[i] };
        if (magi_as_bool(__magi_call_fn(fn_val, 1, call_args)))
            return magi_make_bool(1);
    }
    return magi_make_bool(0);
}

static int64_t magi_method_flat_map(int64_t arr_val, int64_t fn_val) {
    MagiArray* arr = magi_array_ptr(arr_val);
    if (!arr) return magi_make_null();
    MagiArray* result = (MagiArray*)malloc(sizeof(MagiArray));
    result->len = 0; result->cap = 16;
    result->data = (int64_t*)malloc(sizeof(int64_t) * result->cap);
    for (int i = 0; i < arr->len; i++) {
        int64_t call_args[1] = { arr->data[i] };
        int64_t sub = __magi_call_fn(fn_val, 1, call_args);
        MagiArray* subarr = magi_array_ptr(sub);
        if (subarr) {
            for (int j = 0; j < subarr->len; j++) {
                if (result->len >= result->cap) { result->cap *= 2; result->data = (int64_t*)realloc(result->data, sizeof(int64_t) * result->cap); }
                result->data[result->len++] = subarr->data[j];
            }
        } else {
            if (result->len >= result->cap) { result->cap *= 2; result->data = (int64_t*)realloc(result->data, sizeof(int64_t) * result->cap); }
            result->data[result->len++] = sub;
        }
    }
    return magi_make_array_val(result);
}

// ===== Runtime Call Dispatch =====
// Sentinel value for "not handled" — uses unused tag 7, which no valid value produces.
// ===== Numeric Runtime Dispatch IDs =====
// Used by __magi_runtime_call_id for O(1) jump-table dispatch
// instead of the O(n) strcmp chain in __magi_runtime_call.
enum MagiBuiltinId {
    MAGI_RT_UNKNOWN = 0,
    // Arithmetic
    MAGI_RT_ADD = 1, MAGI_RT_SUB, MAGI_RT_MUL, MAGI_RT_DIV, MAGI_RT_MOD, MAGI_RT_REM,
    MAGI_RT_EQ, MAGI_RT_NE, MAGI_RT_LT, MAGI_RT_GT, MAGI_RT_LE, MAGI_RT_GE,
    MAGI_RT_NEG, MAGI_RT_POW,
    // Logical
    MAGI_RT_AND, MAGI_RT_OR, MAGI_RT_NOT,
    // Bitwise
    MAGI_RT_BIT_AND, MAGI_RT_BIT_OR, MAGI_RT_BIT_XOR,
    MAGI_RT_SHL, MAGI_RT_SHR, MAGI_RT_BIT_NOT, MAGI_RT_BIT_ANDNOT,
    // Collections
    MAGI_RT_LEN, MAGI_RT_PUSH, MAGI_RT_POP,
    MAGI_RT_HAS, MAGI_RT_CONTAINS, MAGI_RT_KEYS, MAGI_RT_VALUES, MAGI_RT_ENTRIES,
    MAGI_RT_MAP, MAGI_RT_FILTER, MAGI_RT_REDUCE, MAGI_RT_FIND,
    MAGI_RT_EVERY, MAGI_RT_SOME, MAGI_RT_FOR_EACH,
    MAGI_RT_REVERSE, MAGI_RT_SORT, MAGI_RT_SORT_BY, MAGI_RT_FLAT_MAP,
    MAGI_RT_INDEX_OF, MAGI_RT_INCLUDES,
    // String
    MAGI_RT_TO_STRING, MAGI_RT_TYPEOF, MAGI_RT_SPLIT, MAGI_RT_JOIN,
    MAGI_RT_TRIM, MAGI_RT_UPPER, MAGI_RT_LOWER,
    MAGI_RT_STARTS_WITH, MAGI_RT_ENDS_WITH,
    MAGI_RT_REPLACE, MAGI_RT_SUBSTRING, MAGI_RT_CHAR_AT, MAGI_RT_CONCAT,
    // Math
    MAGI_RT_ABS, MAGI_RT_FLOOR, MAGI_RT_CEIL, MAGI_RT_SQRT, MAGI_RT_ROUND,
    MAGI_RT_SIN, MAGI_RT_COS, MAGI_RT_TAN, MAGI_RT_ATAN, MAGI_RT_ATAN2,
    MAGI_RT_ASIN, MAGI_RT_ACOS, MAGI_RT_FPOW, MAGI_RT_FMOD,
    MAGI_RT_LOG, MAGI_RT_LOG2, MAGI_RT_LOG10, MAGI_RT_EXP,
    MAGI_RT_MIN, MAGI_RT_MAX, MAGI_RT_RANDOM,
    MAGI_RT_IS_NAN, MAGI_RT_IS_FINITE,
    // Range/slice
    MAGI_RT_RANGE, MAGI_RT_SLICE, MAGI_RT_REPEAT,
    // Map
    MAGI_RT_MAP_GET, MAGI_RT_MAP_SET, MAGI_RT_HAS_KEY,
    // Parse
    MAGI_RT_PARSE_INT, MAGI_RT_PARSE_FLOAT,
    MAGI_RT_JSON_PARSE, MAGI_RT_JSON_STRINGIFY,
    // I/O
    MAGI_RT_PRINTLN, MAGI_RT_PRINT,
    // Process
    MAGI_RT_EXIT, MAGI_RT_PANIC, MAGI_RT_TIMESTAMP_MS, MAGI_RT_PROCESS_ARGS,
    MAGI_RT_ENV_GET, MAGI_RT_ENV_SET, MAGI_RT_ENV_HAS,
    MAGI_RT_EXEC_CMD, MAGI_RT_CWD, MAGI_RT_OS_NAME, MAGI_RT_PID,
    // Byte
    MAGI_RT_BYTE_SLICE,
    // File I/O
    MAGI_RT_FS_READ, MAGI_RT_FS_WRITE, MAGI_RT_FS_EXISTS, MAGI_RT_FS_DELETE,
    MAGI_RT_FS_READ_BYTES, MAGI_RT_FS_SIZE, MAGI_RT_FS_READ_LINES,
    MAGI_RT_FS_MKDIR, MAGI_RT_FILE_APPEND, MAGI_RT_LIST_DIR,
    // Path
    MAGI_RT_PATH_JOIN,
    // Renderers (domain-specific)
    MAGI_RT_RENDER_SEG_COLS, MAGI_RT_RENDER_WALL_COL, MAGI_RT_RENDER_FLAT_COL,
    // Arena / GC
    MAGI_RT_ARENA_RESET,
    MAGI_RT_ARENA_ENTER,
    MAGI_RT_ARENA_LEAVE,
    MAGI_RT_HEAP_ALLOCATED,
    MAGI_RT_COUNT
};

#define FAST_BINOP_SENTINEL ((int64_t)(NANBOX_SIG | ((uint64_t)7 << TAG_SHIFT)))

// Fast inline arithmetic for common cases: int+int, float ops, string eq/ne.
// Avoids the full strcmp dispatch chain for hot-path operations.
static inline int64_t __magi_fast_binop(const char* name, int64_t a, int64_t b) {
    int ta = magi_get_tag(a), tb = magi_get_tag(b);

    // Case 1: int + int — all arithmetic and comparisons
    if (ta == TAG_I64 && tb == TAG_I64) {
        int64_t av = magi_sext48(magi_get_payload(a));
        int64_t bv = magi_sext48(magi_get_payload(b));
        switch (name[2]) {
            case 'a': return magi_make_int(av + bv); // __add
            case 's': return magi_make_int(av - bv); // __sub
            case 'm': if (name[3]=='u') return magi_make_int(av * bv); // __mul
                      if (name[3]=='o') return (bv==0) ? magi_make_int(0) : magi_make_int(av % bv); // __mod
                      break;
            case 'd': return (bv==0) ? magi_make_int(0) : magi_make_int(av / bv); // __div
            case 'r': return (bv==0) ? magi_make_int(0) : magi_make_int(av % bv); // __rem/__mod
            case 'l': if (name[3]=='t') return magi_make_bool(av < bv);  // __lt
                      if (name[3]=='e') return magi_make_bool(av <= bv); // __le
                      break;
            case 'g': if (name[3]=='t') return magi_make_bool(av > bv);  // __gt
                      if (name[3]=='e') return magi_make_bool(av >= bv); // __ge
                      break;
            case 'e': return magi_make_bool(av == bv); // __eq
            case 'n': return magi_make_bool(av != bv); // __ne
        }
        return FAST_BINOP_SENTINEL;
    }

    // Case 2: string operands — handle eq/ne inline, fall through for concat/repeat
    if (ta == TAG_STRING || tb == TAG_STRING) {
        if (ta == TAG_STRING && tb == TAG_STRING) {
            if (name[2] == 'e') return magi_make_bool(strcmp(magi_as_string(a), magi_as_string(b)) == 0); // __eq
            if (name[2] == 'n') return magi_make_bool(strcmp(magi_as_string(a), magi_as_string(b)) != 0); // __ne
        }
        return FAST_BINOP_SENTINEL; // concat, repeat, etc. need full dispatch
    }

    // Case 2b: array/map operands — need deep equality via full dispatch
    if (ta == TAG_ARRAY || ta == TAG_MAP || tb == TAG_ARRAY || tb == TAG_MAP)
        return FAST_BINOP_SENTINEL;

    // Case 3: float operands — at least one is float (tag == 8), neither is string
    {
        double fa = magi_as_float(a);
        double fb = magi_as_float(b);
        switch (name[2]) {
            case 'a': return magi_make_float(fa + fb); // __add
            case 's': return magi_make_float(fa - fb); // __sub
            case 'm': if (name[3]=='u') return magi_make_float(fa * fb); // __mul
                      if (name[3]=='o') return magi_make_float(fmod(fa, fb)); // __mod
                      break;
            case 'd': return (fb == 0.0) ? magi_make_float(0.0) : magi_make_float(fa / fb); // __div
            case 'r': return magi_make_float(fmod(fa, fb)); // __rem
            case 'l': if (name[3]=='t') return magi_make_bool(fa < fb);  // __lt
                      if (name[3]=='e') return magi_make_bool(fa <= fb); // __le
                      break;
            case 'g': if (name[3]=='t') return magi_make_bool(fa > fb);  // __gt
                      if (name[3]=='e') return magi_make_bool(fa >= fb); // __ge
                      break;
            case 'e': return magi_make_bool(fa == fb); // __eq
            case 'n': if (name[3]=='e') {
                          if (name[4]=='g') return magi_make_float(-fa); // __neg (unary)
                          return magi_make_bool(fa != fb); // __ne
                      }
                      break;
        }
    }

    return FAST_BINOP_SENTINEL;
}

// ===== Direct Builtin Wrappers (bypass RuntimeCall dispatch) =====

int64_t __magi_builtin_len(int64_t a) {
    if (magi_get_tag(a) == TAG_ARRAY) return __magi_array_len(a);
    if (magi_get_tag(a) == TAG_STRING) return __magi_string_len(a);
    if (magi_get_tag(a) == TAG_MAP) {
        MagiMap* map = magi_map_ptr(a);
        return magi_make_int(map ? map->len : 0);
    }
    return magi_make_int(0);
}

int64_t __magi_builtin_push(int64_t arr, int64_t val) {
    return __magi_array_push(arr, val);
}

int64_t __magi_builtin_abs(int64_t a) {
    if (magi_get_tag(a) == TAG_I64) {
        int64_t v = magi_sext48(magi_get_payload(a));
        return magi_make_int(v < 0 ? -v : v);
    }
    return magi_make_float(fabs(magi_as_float(a)));
}

int64_t __magi_builtin_floor(int64_t a) {
    return magi_make_float(floor(magi_as_float(a)));
}

int64_t __magi_builtin_sqrt(int64_t a) {
    return magi_make_float(sqrt(magi_as_float(a)));
}

int64_t __magi_builtin_cos(int64_t a) {
    return magi_make_float(cos(magi_as_float(a)));
}

int64_t __magi_builtin_sin(int64_t a) {
    return magi_make_float(sin(magi_as_float(a)));
}

int64_t __magi_builtin_atan2(int64_t a, int64_t b) {
    return magi_make_float(atan2(magi_as_float(a), magi_as_float(b)));
}

// ===== Numeric ID Dispatch (O(1) jump table) =====
// Called from LLVM-compiled code with a compile-time constant ID.
// Falls back to __magi_runtime_call for unknown IDs.
int64_t __magi_runtime_call_id(int32_t id, int32_t argc, int64_t* args) {
    int64_t a = argc > 0 ? args[0] : magi_make_null();
    int64_t b = argc > 1 ? args[1] : magi_make_null();

    switch (id) {
    // ── Arithmetic ──
    case MAGI_RT_ADD: {
        int ta = magi_get_tag(a), tb = magi_get_tag(b);
        if (ta == TAG_I64 && tb == TAG_I64)
            return magi_make_int(magi_sext48(magi_get_payload(a)) + magi_sext48(magi_get_payload(b)));
        if (ta == TAG_STRING && tb == TAG_STRING)
            return __magi_string_concat(a, b);
        if (ta == TAG_STRING || tb == TAG_STRING) {
            int64_t sa = (ta == TAG_STRING) ? a : __magi_to_string(a);
            int64_t sb = (tb == TAG_STRING) ? b : __magi_to_string(b);
            return __magi_string_concat(sa, sb);
        }
        return magi_make_float(magi_as_float(a) + magi_as_float(b));
    }
    case MAGI_RT_SUB: {
        int ta = magi_get_tag(a), tb = magi_get_tag(b);
        if (ta == TAG_I64 && tb == TAG_I64)
            return magi_make_int(magi_sext48(magi_get_payload(a)) - magi_sext48(magi_get_payload(b)));
        return magi_make_float(magi_as_float(a) - magi_as_float(b));
    }
    case MAGI_RT_MUL: {
        int ta = magi_get_tag(a), tb = magi_get_tag(b);
        if (ta == TAG_I64 && tb == TAG_I64)
            return magi_make_int(magi_sext48(magi_get_payload(a)) * magi_sext48(magi_get_payload(b)));
        if (ta == TAG_STRING && tb == TAG_I64) {
            int64_t repeat_args[2] = { a, b };
            return __magi_runtime_call_id(MAGI_RT_REPEAT, 2, repeat_args);
        }
        if (ta == TAG_I64 && tb == TAG_STRING) {
            int64_t repeat_args[2] = { b, a };
            return __magi_runtime_call_id(MAGI_RT_REPEAT, 2, repeat_args);
        }
        return magi_make_float(magi_as_float(a) * magi_as_float(b));
    }
    case MAGI_RT_DIV: {
        int ta = magi_get_tag(a), tb = magi_get_tag(b);
        if (ta == TAG_I64 && tb == TAG_I64) {
            int64_t bv = magi_sext48(magi_get_payload(b));
            return (bv == 0) ? magi_make_int(0) : magi_make_int(magi_sext48(magi_get_payload(a)) / bv);
        }
        return magi_make_float(magi_as_float(a) / magi_as_float(b));
    }
    case MAGI_RT_MOD:
    case MAGI_RT_REM: {
        int ta = magi_get_tag(a), tb = magi_get_tag(b);
        if (ta == TAG_I64 && tb == TAG_I64) {
            int64_t bv = magi_sext48(magi_get_payload(b));
            return (bv == 0) ? magi_make_int(0) : magi_make_int(magi_sext48(magi_get_payload(a)) % bv);
        }
        return magi_make_float(fmod(magi_as_float(a), magi_as_float(b)));
    }
    case MAGI_RT_POW:
        return magi_make_float(pow(magi_as_float(a), magi_as_float(b)));
    case MAGI_RT_NEG: {
        if (magi_get_tag(a) == TAG_I64) return magi_make_int(-magi_sext48(magi_get_payload(a)));
        return magi_make_float(-magi_as_float(a));
    }

    // ── Comparison ──
    case MAGI_RT_EQ: {
        if (a == b) return magi_make_bool(1);
        if (magi_get_tag(a) == TAG_STRING && magi_get_tag(b) == TAG_STRING)
            return magi_make_bool(strcmp(magi_as_string(a), magi_as_string(b)) == 0);
        if (magi_get_tag(a) == TAG_ARRAY && magi_get_tag(b) == TAG_ARRAY) {
            MagiArray* aa = magi_array_ptr(a);
            MagiArray* ab = magi_array_ptr(b);
            if (!aa && !ab) return magi_make_bool(1);
            if (!aa || !ab || aa->len != ab->len) return magi_make_bool(0);
            for (int i = 0; i < aa->len; i++) {
                int64_t eq_args[2] = {aa->data[i], ab->data[i]};
                int64_t eq = __magi_runtime_call_id(MAGI_RT_EQ, 2, eq_args);
                if (!magi_as_bool(eq)) return magi_make_bool(0);
            }
            return magi_make_bool(1);
        }
        if (magi_get_tag(a) == TAG_MAP && magi_get_tag(b) == TAG_MAP) {
            MagiMap* ma = magi_map_ptr(a);
            MagiMap* mb = magi_map_ptr(b);
            if (!ma && !mb) return magi_make_bool(1);
            if (!ma || !mb || ma->len != mb->len) return magi_make_bool(0);
            for (int i = 0; i < ma->len; i++) {
                int found = 0;
                for (int j = 0; j < mb->len; j++) {
                    if (strcmp(ma->keys[i], mb->keys[j]) == 0) {
                        int64_t eq_args[2] = {ma->values[i], mb->values[j]};
                        int64_t eq = __magi_runtime_call_id(MAGI_RT_EQ, 2, eq_args);
                        if (!magi_as_bool(eq)) return magi_make_bool(0);
                        found = 1;
                        break;
                    }
                }
                if (!found) return magi_make_bool(0);
            }
            return magi_make_bool(1);
        }
        return magi_make_bool(0);
    }
    case MAGI_RT_NE: {
        int64_t eq_args[2] = {a, b};
        int64_t eq = __magi_runtime_call_id(MAGI_RT_EQ, 2, eq_args);
        return magi_make_bool(!magi_as_bool(eq));
    }
    case MAGI_RT_LT: {
        if (magi_get_tag(a) == TAG_I64 && magi_get_tag(b) == TAG_I64)
            return magi_make_bool(magi_sext48(magi_get_payload(a)) < magi_sext48(magi_get_payload(b)));
        return magi_make_bool(magi_as_float(a) < magi_as_float(b));
    }
    case MAGI_RT_GT: {
        if (magi_get_tag(a) == TAG_I64 && magi_get_tag(b) == TAG_I64)
            return magi_make_bool(magi_sext48(magi_get_payload(a)) > magi_sext48(magi_get_payload(b)));
        return magi_make_bool(magi_as_float(a) > magi_as_float(b));
    }
    case MAGI_RT_LE: {
        if (magi_get_tag(a) == TAG_I64 && magi_get_tag(b) == TAG_I64)
            return magi_make_bool(magi_sext48(magi_get_payload(a)) <= magi_sext48(magi_get_payload(b)));
        return magi_make_bool(magi_as_float(a) <= magi_as_float(b));
    }
    case MAGI_RT_GE: {
        if (magi_get_tag(a) == TAG_I64 && magi_get_tag(b) == TAG_I64)
            return magi_make_bool(magi_sext48(magi_get_payload(a)) >= magi_sext48(magi_get_payload(b)));
        return magi_make_bool(magi_as_float(a) >= magi_as_float(b));
    }

    // ── Logical ──
    case MAGI_RT_AND: return magi_make_bool(magi_as_bool(a) && magi_as_bool(b));
    case MAGI_RT_OR: return magi_make_bool(magi_as_bool(a) || magi_as_bool(b));
    case MAGI_RT_NOT: return magi_make_bool(!magi_as_bool(a));

    // ── Bitwise ──
    case MAGI_RT_BIT_AND: return magi_make_int(magi_as_int(a) & magi_as_int(b));
    case MAGI_RT_BIT_OR: return magi_make_int(magi_as_int(a) | magi_as_int(b));
    case MAGI_RT_BIT_XOR: return magi_make_int(magi_as_int(a) ^ magi_as_int(b));
    case MAGI_RT_SHL: return magi_make_int(magi_as_int(a) << (magi_as_int(b) & 63));
    case MAGI_RT_SHR: return magi_make_int(magi_as_int(a) >> (magi_as_int(b) & 63));
    case MAGI_RT_BIT_NOT: return magi_make_int(~magi_as_int(a));
    case MAGI_RT_BIT_ANDNOT: return magi_make_int(magi_as_int(a) & ~magi_as_int(b));

    // ── Range ──
    case MAGI_RT_RANGE: {
        int64_t start = magi_as_int(a);
        int64_t end = magi_as_int(b);
        int inclusive = (argc > 2) ? magi_as_bool(args[2]) : 0;
        int64_t count = inclusive ? (end - start + 1) : (end - start);
        if (count < 0) count = 0;
        if (count > 10000000) count = 10000000;
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = (int32_t)count;
        arr->cap = (int32_t)(count > 8 ? count : 8);
        arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
        for (int64_t i = 0; i < count; i++) {
            arr->data[i] = magi_make_int(start + i);
        }
        return magi_make_array_val(arr);
    }

    // ── Collection operations ──
    case MAGI_RT_LEN: {
        if (magi_get_tag(a) == TAG_ARRAY) return __magi_array_len(a);
        if (magi_get_tag(a) == TAG_STRING) return __magi_string_len(a);
        if (magi_get_tag(a) == TAG_MAP) {
            MagiMap* map = magi_map_ptr(a);
            return magi_make_int(map ? map->len : 0);
        }
        return magi_make_int(0);
    }
    case MAGI_RT_TO_STRING: return __magi_to_string(a);
    case MAGI_RT_TYPEOF: {
        int t = magi_get_tag(a);
        const char* tn;
        switch(t) {
            case TAG_NULL: tn = "null"; break;
            case TAG_BOOL: tn = "bool"; break;
            case TAG_I64: tn = "int"; break;
            case TAG_STRING: tn = "string"; break;
            case TAG_ARRAY: tn = "array"; break;
            case TAG_MAP: tn = "map"; break;
            case 8: tn = "float"; break;
            default: tn = "unknown"; break;
        }
        return magi_make_string(tn);
    }
    case MAGI_RT_PUSH: return __magi_array_push(a, b);
    case MAGI_RT_POP: {
        MagiArray* arr = magi_array_ptr(a);
        if (!arr || arr->len == 0) return magi_make_null();
        return arr->data[--arr->len];
    }
    case MAGI_RT_HAS: {
        if (magi_get_tag(a) == TAG_MAP) {
            MagiMap* map = magi_map_ptr(a);
            const char* key = magi_as_string(b);
            if (!map || !key) return magi_make_bool(0);
            for (int i = 0; i < map->len; i++) {
                if (strcmp(map->keys[i], key) == 0) return magi_make_bool(1);
            }
            return magi_make_bool(0);
        }
        if (magi_get_tag(a) == TAG_ARRAY) {
            MagiArray* arr = magi_array_ptr(a);
            if (!arr) return magi_make_bool(0);
            for (int i = 0; i < arr->len; i++) {
                if (arr->data[i] == b) return magi_make_bool(1);
                if (magi_get_tag(arr->data[i]) == TAG_STRING && magi_get_tag(b) == TAG_STRING &&
                    strcmp(magi_as_string(arr->data[i]), magi_as_string(b)) == 0)
                    return magi_make_bool(1);
            }
            return magi_make_bool(0);
        }
        return magi_make_bool(0);
    }
    case MAGI_RT_CONTAINS: {
        int64_t has_args[2] = { a, b };
        return __magi_runtime_call_id(MAGI_RT_HAS, 2, has_args);
    }
    case MAGI_RT_INCLUDES: {
        if (magi_get_tag(a) == TAG_ARRAY) {
            MagiArray* arr = magi_array_ptr(a);
            if (!arr) return magi_make_bool(0);
            for (int i = 0; i < arr->len; i++) { if (arr->data[i] == b) return magi_make_bool(1); }
            return magi_make_bool(0);
        }
        if (magi_get_tag(a) == TAG_STRING) {
            const char* s = magi_as_string(a);
            const char* sub = magi_as_string(b);
            return magi_make_bool(strstr(s, sub) != NULL);
        }
        return magi_make_bool(0);
    }

    // ── Array/Collection methods with callbacks ──
    case MAGI_RT_MAP: return magi_method_map(a, b);
    case MAGI_RT_FILTER: return magi_method_filter(a, b);
    case MAGI_RT_REDUCE: { int64_t c = argc > 2 ? args[2] : magi_make_null(); return magi_method_reduce(a, b, c); }
    case MAGI_RT_FOR_EACH: return magi_method_for_each(a, b);
    case MAGI_RT_FIND: return magi_method_find(a, b);
    case MAGI_RT_EVERY: return magi_method_every(a, b);
    case MAGI_RT_SOME: return magi_method_some(a, b);
    case MAGI_RT_FLAT_MAP: return magi_method_flat_map(a, b);
    case MAGI_RT_SORT_BY: {
        MagiArray* arr = magi_array_ptr(a);
        if (!arr || arr->len <= 1) return a;
        for (int i = 1; i < arr->len; i++) {
            int64_t key = arr->data[i];
            int j = i - 1;
            while (j >= 0) {
                int64_t cmp_args[2] = { arr->data[j], key };
                int64_t cmp = __magi_call_fn(b, 2, cmp_args);
                if (magi_as_int(cmp) <= 0) break;
                arr->data[j + 1] = arr->data[j];
                j--;
            }
            arr->data[j + 1] = key;
        }
        return a;
    }
    case MAGI_RT_REVERSE: {
        MagiArray* arr = magi_array_ptr(a);
        if (!arr) return a;
        for (int i = 0, j = arr->len - 1; i < j; i++, j--) {
            int64_t tmp = arr->data[i]; arr->data[i] = arr->data[j]; arr->data[j] = tmp;
        }
        return a;
    }
    case MAGI_RT_SORT: {
        MagiArray* arr = magi_array_ptr(a);
        if (!arr || arr->len <= 1) return a;
        for (int i = 1; i < arr->len; i++) {
            int64_t key = arr->data[i];
            double kv = magi_as_float(key);
            int j = i - 1;
            while (j >= 0 && magi_as_float(arr->data[j]) > kv) { arr->data[j+1] = arr->data[j]; j--; }
            arr->data[j + 1] = key;
        }
        return a;
    }
    case MAGI_RT_INDEX_OF: {
        if (magi_get_tag(a) == TAG_ARRAY) {
            MagiArray* arr = magi_array_ptr(a);
            if (!arr) return magi_make_int(-1);
            for (int i = 0; i < arr->len; i++) { if (arr->data[i] == b) return magi_make_int(i); }
            return magi_make_int(-1);
        }
        if (magi_get_tag(a) == TAG_STRING) {
            const char* s = magi_as_string(a);
            const char* sub = magi_as_string(b);
            const char* p = strstr(s, sub);
            return p ? magi_make_int(p - s) : magi_make_int(-1);
        }
        return magi_make_int(-1);
    }
    case MAGI_RT_JOIN: {
        MagiArray* arr = magi_array_ptr(a);
        if (!arr) return magi_make_string("");
        const char* sep = magi_as_string(b);
        size_t total = 0, seplen = strlen(sep);
        char** strs = (char**)malloc(sizeof(char*) * arr->len);
        for (int i = 0; i < arr->len; i++) {
            char buf[256]; magi_val_to_str(arr->data[i], buf, sizeof(buf));
            if (magi_get_tag(arr->data[i]) == TAG_STRING) {
                strs[i] = strdup(magi_as_string(arr->data[i]));
            } else {
                strs[i] = strdup(buf);
            }
            total += strlen(strs[i]);
        }
        total += seplen * (arr->len > 0 ? arr->len - 1 : 0);
        char* result = (char*)malloc(total + 1);
        result[0] = '\0';
        for (int i = 0; i < arr->len; i++) {
            if (i > 0) strcat(result, sep);
            strcat(result, strs[i]);
            free(strs[i]);
        }
        free(strs);
        return magi_make_string(result);
    }

    // ── Slice ──
    case MAGI_RT_SLICE: {
        int64_t start_v = argc > 1 ? args[1] : magi_make_int(0);
        int64_t end_v = argc > 2 ? args[2] : magi_make_null();
        int64_t start = magi_as_int(start_v);
        if (magi_get_tag(a) == TAG_ARRAY) {
            MagiArray* arr = magi_array_ptr(a);
            if (!arr) return magi_make_null();
            int64_t end = magi_is_tagged(end_v) && magi_get_tag(end_v) == TAG_NULL ? arr->len : magi_as_int(end_v);
            if (start < 0) start += arr->len;
            if (end < 0) end += arr->len;
            if (start < 0) start = 0;
            if (end > arr->len) end = arr->len;
            if (start >= end) return __magi_array_new(0, NULL);
            int64_t count = end - start;
            return __magi_array_new((int32_t)count, arr->data + start);
        }
        if (magi_get_tag(a) == TAG_STRING) {
            const char* s = magi_as_string(a);
            int64_t slen = (int64_t)strlen(s);
            int64_t end = magi_is_tagged(end_v) && magi_get_tag(end_v) == TAG_NULL ? slen : magi_as_int(end_v);
            if (start < 0) start += slen;
            if (end < 0) end += slen;
            if (start < 0) start = 0;
            if (end > slen) end = slen;
            if (start >= end) return magi_make_string("");
            int64_t count = end - start;
            char* result = (char*)malloc(count + 1);
            memcpy(result, s + start, count);
            result[count] = '\0';
            return magi_make_string(result);
        }
        return magi_make_null();
    }
    case MAGI_RT_REPEAT: {
        if (magi_get_tag(a) == TAG_STRING) {
            const char* s = magi_as_string(a);
            int64_t n = magi_as_int(b);
            if (n <= 0) return magi_make_string("");
            size_t slen = strlen(s);
            char* result = (char*)malloc(slen * n + 1);
            result[0] = '\0';
            for (int64_t i = 0; i < n; i++) memcpy(result + i * slen, s, slen);
            result[slen * n] = '\0';
            return magi_make_string(result);
        }
        return magi_make_null();
    }

    // ── Map operations ──
    case MAGI_RT_MAP_GET: return __magi_map_get(a, b);
    case MAGI_RT_MAP_SET: {
        int64_t val = argc > 2 ? args[2] : magi_make_null();
        __magi_map_set(a, b, val);
        return magi_make_null();
    }
    case MAGI_RT_KEYS: {
        MagiMap* map = magi_map_ptr(a);
        if (!map) return __magi_array_new(0, NULL);
        int64_t* elems = (int64_t*)malloc(sizeof(int64_t) * map->len);
        for (int i = 0; i < map->len; i++) elems[i] = magi_make_string(map->keys[i]);
        int64_t result = __magi_array_new(map->len, elems);
        free(elems);
        return result;
    }
    case MAGI_RT_VALUES: {
        MagiMap* map = magi_map_ptr(a);
        if (!map) return __magi_array_new(0, NULL);
        return __magi_array_new(map->len, map->values);
    }
    case MAGI_RT_ENTRIES: {
        MagiMap* map = magi_map_ptr(a);
        if (!map) return __magi_array_new(0, NULL);
        int64_t* pairs = (int64_t*)malloc(sizeof(int64_t) * map->len);
        for (int i = 0; i < map->len; i++) {
            int64_t pair_data[2] = { magi_make_string(map->keys[i]), map->values[i] };
            pairs[i] = __magi_array_new(2, pair_data);
        }
        int64_t result = __magi_array_new(map->len, pairs);
        free(pairs);
        return result;
    }
    case MAGI_RT_HAS_KEY: {
        MagiMap* map = magi_map_ptr(a);
        const char* key = magi_as_string(b);
        if (!map) return magi_make_bool(0);
        for (int i = 0; i < map->len; i++) { if (strcmp(map->keys[i], key) == 0) return magi_make_bool(1); }
        return magi_make_bool(0);
    }

    // ── String operations ──
    case MAGI_RT_PARSE_INT: {
        const char* s = magi_as_string(a);
        return magi_make_int((int64_t)atoll(s));
    }
    case MAGI_RT_PARSE_FLOAT: {
        const char* s = magi_as_string(a);
        return magi_make_float(atof(s));
    }
    case MAGI_RT_CONCAT: return __magi_string_concat(a, b);
    case MAGI_RT_SPLIT: {
        const char* s = magi_as_string(a);
        const char* delim = magi_as_string(b);
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = 0; arr->cap = 16;
        arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
        size_t dlen = strlen(delim);
        if (dlen == 0) {
            for (size_t i = 0; i < strlen(s); i++) {
                char* ch = (char*)malloc(2); ch[0] = s[i]; ch[1] = '\0';
                if (arr->len >= arr->cap) { arr->cap *= 2; arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap); }
                arr->data[arr->len++] = magi_make_string(ch);
            }
        } else {
            const char* p = s;
            while (1) {
                const char* found = strstr(p, delim);
                size_t part_len = found ? (size_t)(found - p) : strlen(p);
                char* part = (char*)malloc(part_len + 1);
                memcpy(part, p, part_len); part[part_len] = '\0';
                if (arr->len >= arr->cap) { arr->cap *= 2; arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap); }
                arr->data[arr->len++] = magi_make_string(part);
                if (!found) break;
                p = found + dlen;
            }
        }
        return magi_make_array_val(arr);
    }
    case MAGI_RT_TRIM: {
        const char* s = magi_as_string(a);
        size_t len = strlen(s);
        size_t start = 0, end = len;
        while (start < len && (s[start] == ' ' || s[start] == '\t' || s[start] == '\n' || s[start] == '\r')) start++;
        while (end > start && (s[end-1] == ' ' || s[end-1] == '\t' || s[end-1] == '\n' || s[end-1] == '\r')) end--;
        char* result = (char*)malloc(end - start + 1);
        memcpy(result, s + start, end - start); result[end - start] = '\0';
        return magi_make_string(result);
    }
    case MAGI_RT_UPPER: {
        const char* s = magi_as_string(a);
        size_t len = strlen(s);
        char* result = (char*)malloc(len + 1);
        for (size_t i = 0; i < len; i++) result[i] = (s[i] >= 'a' && s[i] <= 'z') ? s[i] - 32 : s[i];
        result[len] = '\0';
        return magi_make_string(result);
    }
    case MAGI_RT_LOWER: {
        const char* s = magi_as_string(a);
        size_t len = strlen(s);
        char* result = (char*)malloc(len + 1);
        for (size_t i = 0; i < len; i++) result[i] = (s[i] >= 'A' && s[i] <= 'Z') ? s[i] + 32 : s[i];
        result[len] = '\0';
        return magi_make_string(result);
    }
    case MAGI_RT_STARTS_WITH:
        return magi_make_bool(strncmp(magi_as_string(a), magi_as_string(b), strlen(magi_as_string(b))) == 0);
    case MAGI_RT_ENDS_WITH: {
        const char* s = magi_as_string(a), *suffix = magi_as_string(b);
        size_t sl = strlen(s), sufl = strlen(suffix);
        return magi_make_bool(sl >= sufl && strcmp(s + sl - sufl, suffix) == 0);
    }
    case MAGI_RT_REPLACE: {
        const char* s = magi_as_string(a);
        const char* from = magi_as_string(b);
        const char* to = argc > 2 ? magi_as_string(args[2]) : "";
        size_t slen = strlen(s), flen = strlen(from), tlen = strlen(to);
        if (flen == 0) return a;
        int count = 0;
        const char* p = s;
        while ((p = strstr(p, from))) { count++; p += flen; }
        char* result = (char*)malloc(slen + count * (tlen - flen) + 1);
        char* w = result;
        p = s;
        while (*p) {
            if (strncmp(p, from, flen) == 0) { memcpy(w, to, tlen); w += tlen; p += flen; }
            else { *w++ = *p++; }
        }
        *w = '\0';
        return magi_make_string(result);
    }
    case MAGI_RT_SUBSTRING: {
        const char* s = magi_as_string(a);
        int64_t slen = (int64_t)strlen(s);
        int64_t start = magi_as_int(b);
        int64_t end = argc > 2 ? magi_as_int(args[2]) : slen;
        if (start < 0) start += slen;
        if (end < 0) end += slen;
        if (start < 0) start = 0;
        if (end > slen) end = slen;
        if (start >= end) return magi_make_string("");
        int64_t cnt = end - start;
        char* result = (char*)malloc(cnt + 1);
        memcpy(result, s + start, cnt); result[cnt] = '\0';
        return magi_make_string(result);
    }
    case MAGI_RT_CHAR_AT: {
        const char* s = magi_as_string(a);
        int64_t idx = magi_as_int(b);
        size_t slen = strlen(s);
        if (idx < 0 || idx >= (int64_t)slen) return magi_make_string("");
        char* result = (char*)malloc(2);
        result[0] = s[idx]; result[1] = '\0';
        return magi_make_string(result);
    }

    // ── Math ──
    case MAGI_RT_ABS: {
        if (magi_get_tag(a) == TAG_I64) {
            int64_t v = magi_sext48(magi_get_payload(a));
            return magi_make_int(v < 0 ? -v : v);
        }
        return magi_make_float(fabs(magi_as_float(a)));
    }
    case MAGI_RT_FLOOR: return magi_make_float(floor(magi_as_float(a)));
    case MAGI_RT_CEIL: return magi_make_float(ceil(magi_as_float(a)));
    case MAGI_RT_SQRT: return magi_make_float(sqrt(magi_as_float(a)));
    case MAGI_RT_ROUND: return magi_make_float(round(magi_as_float(a)));
    case MAGI_RT_SIN: return magi_make_float(sin(magi_as_float(a)));
    case MAGI_RT_COS: return magi_make_float(cos(magi_as_float(a)));
    case MAGI_RT_TAN: return magi_make_float(tan(magi_as_float(a)));
    case MAGI_RT_LOG: return magi_make_float(log(magi_as_float(a)));
    case MAGI_RT_LOG2: return magi_make_float(log2(magi_as_float(a)));
    case MAGI_RT_LOG10: return magi_make_float(log10(magi_as_float(a)));
    case MAGI_RT_EXP: return magi_make_float(exp(magi_as_float(a)));
    case MAGI_RT_ATAN: return magi_make_float(atan(magi_as_float(a)));
    case MAGI_RT_ATAN2: return magi_make_float(atan2(magi_as_float(a), magi_as_float(b)));
    case MAGI_RT_ASIN: return magi_make_float(asin(magi_as_float(a)));
    case MAGI_RT_ACOS: return magi_make_float(acos(magi_as_float(a)));
    case MAGI_RT_FPOW: return magi_make_float(pow(magi_as_float(a), magi_as_float(b)));
    case MAGI_RT_FMOD: return magi_make_float(fmod(magi_as_float(a), magi_as_float(b)));
    case MAGI_RT_MIN: {
        double da = magi_as_float(a), db = magi_as_float(b);
        return da < db ? a : b;
    }
    case MAGI_RT_MAX: {
        double da = magi_as_float(a), db = magi_as_float(b);
        return da > db ? a : b;
    }
    case MAGI_RT_RANDOM: return magi_make_float((double)rand() / RAND_MAX);
    case MAGI_RT_IS_NAN: {
        if (!magi_is_tagged(a)) { double d; memcpy(&d, &a, sizeof(d)); return magi_make_bool(isnan(d)); }
        return magi_make_bool(0);
    }
    case MAGI_RT_IS_FINITE: {
        if (!magi_is_tagged(a)) { double d; memcpy(&d, &a, sizeof(d)); return magi_make_bool(isfinite(d)); }
        return magi_make_bool(1);
    }

    // ── I/O ──
    case MAGI_RT_PRINTLN: __magi_print(a); return magi_make_null();
    case MAGI_RT_PRINT: {
        arena_mode = 1;
        char* s = magi_val_to_dyn_str(a, 1);
        printf("%s", s);
        fflush(stdout);
        arena_mode = 0;
        return magi_make_null();
    }

    // ── Process/OS ──
    case MAGI_RT_PROCESS_ARGS: {
        extern int __magi_argc;
        extern char** __magi_argv;
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = __magi_argc > 1 ? __magi_argc - 1 : 0;
        arr->cap = arr->len > 8 ? arr->len : 8;
        arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
        for (int i = 1; i < __magi_argc; i++) {
            arr->data[i - 1] = magi_make_string(__magi_argv[i]);
        }
        return magi_make_array_val(arr);
    }
    case MAGI_RT_ENV_GET: {
        const char* key = magi_as_string(a);
        const char* val = getenv(key);
        return val ? magi_make_string(val) : magi_make_null();
    }
    case MAGI_RT_ENV_SET: {
        #ifdef _WIN32
        _putenv_s(magi_as_string(a), magi_as_string(b));
        #else
        setenv(magi_as_string(a), magi_as_string(b), 1);
        #endif
        return magi_make_null();
    }
    case MAGI_RT_ENV_HAS:
        return magi_make_bool(getenv(magi_as_string(a)) != NULL);
    case MAGI_RT_TIMESTAMP_MS: {
        #ifdef _WIN32
        return magi_make_int((int64_t)GetTickCount64());
        #else
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        return magi_make_int(ts.tv_sec * 1000 + ts.tv_nsec / 1000000);
        #endif
    }
    case MAGI_RT_EXIT: exit((int)magi_as_int(a));
    case MAGI_RT_PANIC: {
        const char* msg = magi_as_string(a);
        fprintf(stderr, "panic: %s\n", msg);
        exit(1);
    }
    case MAGI_RT_EXEC_CMD: {
        const char* cmd = magi_as_string(a);
        int r = system(cmd);
        return magi_make_int(r);
    }
    case MAGI_RT_CWD: {
        char buf[4096];
        if (getcwd(buf, sizeof(buf))) return magi_make_string(strdup(buf));
        return magi_make_string("/");
    }
    case MAGI_RT_OS_NAME: {
        #ifdef __linux__
        return magi_make_string("linux");
        #elif __APPLE__
        return magi_make_string("macos");
        #elif _WIN32
        return magi_make_string("windows");
        #else
        return magi_make_string("unknown");
        #endif
    }
    case MAGI_RT_PID: return magi_make_int(getpid());

    // ── Byte ──
    case MAGI_RT_BYTE_SLICE: {
        int64_t c = argc > 2 ? args[2] : magi_make_int(0);
        return __magi_byte_slice(a, b, c);
    }

    // ── File I/O ──
    case MAGI_RT_PATH_JOIN: {
        const char* p1 = magi_as_string(a);
        const char* p2 = magi_as_string(b);
        size_t l1 = strlen(p1), l2 = strlen(p2);
        char* result = (char*)malloc(l1 + l2 + 2);
        memcpy(result, p1, l1);
        if (l1 > 0 && p1[l1-1] != '/') { result[l1] = '/'; memcpy(result+l1+1, p2, l2+1); }
        else { memcpy(result+l1, p2, l2+1); }
        return magi_make_string(result);
    }
    case MAGI_RT_FS_READ_BYTES: {
        const char* path = magi_as_string(a);
        FILE* f = fopen(path, "rb");
        if (!f) return magi_make_null();
        fseek(f, 0, SEEK_END);
        long len = ftell(f);
        fseek(f, 0, SEEK_SET);
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = (int32_t)len;
        arr->cap = (int32_t)len;
        arr->data = (int64_t*)malloc(sizeof(int64_t) * len);
        unsigned char* buf = (unsigned char*)malloc(len);
        fread(buf, 1, len, f);
        fclose(f);
        for (long i = 0; i < len; i++) {
            arr->data[i] = magi_make_int(buf[i]);
        }
        free(buf);
        return magi_make_array_val(arr);
    }
    case MAGI_RT_FS_SIZE: {
        const char* path = magi_as_string(a);
        FILE* f = fopen(path, "rb");
        if (!f) return magi_make_int(0);
        fseek(f, 0, SEEK_END);
        long sz = ftell(f);
        fclose(f);
        return magi_make_int(sz);
    }
    case MAGI_RT_FS_WRITE: {
        const char* path = magi_as_string(a);
        const char* content = magi_as_string(b);
        FILE* f = fopen(path, "w");
        if (f) { fputs(content, f); fclose(f); return magi_make_string(path); }
        return magi_make_null();
    }
    case MAGI_RT_FS_READ: {
        const char* path = magi_as_string(a);
        FILE* f = fopen(path, "r");
        if (!f) return magi_make_null();
        fseek(f, 0, SEEK_END);
        long len = ftell(f);
        fseek(f, 0, SEEK_SET);
        char* buf = (char*)malloc(len + 1);
        fread(buf, 1, len, f);
        buf[len] = '\0';
        fclose(f);
        return magi_make_string(buf);
    }
    case MAGI_RT_FS_EXISTS: {
        const char* path = magi_as_string(a);
        FILE* f = fopen(path, "r");
        if (f) { fclose(f); return magi_make_bool(1); }
        return magi_make_bool(0);
    }
    case MAGI_RT_FS_DELETE:
        return magi_make_bool(remove(magi_as_string(a)) == 0);
    case MAGI_RT_FS_READ_LINES: {
        const char* path = magi_as_string(a);
        FILE* f = fopen(path, "r");
        if (!f) return __magi_array_new(0, NULL);
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = 0; arr->cap = 32;
        arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
        char line[4096];
        while (fgets(line, sizeof(line), f)) {
            size_t len = strlen(line);
            if (len > 0 && line[len-1] == '\n') line[--len] = '\0';
            if (arr->len >= arr->cap) { arr->cap *= 2; arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap); }
            arr->data[arr->len++] = magi_make_string(strdup(line));
        }
        fclose(f);
        return magi_make_array_val(arr);
    }
    case MAGI_RT_FS_MKDIR: {
        #ifdef _WIN32
        int r = _mkdir(magi_as_string(a));
        #else
        int r = mkdir(magi_as_string(a), 0755);
        #endif
        return magi_make_bool(r == 0 || errno == EEXIST);
    }
    case MAGI_RT_FILE_APPEND: {
        const char* path = magi_as_string(a);
        const char* content = magi_as_string(b);
        FILE* f = fopen(path, "a");
        if (f) { fputs(content, f); fclose(f); return magi_make_string(path); }
        return magi_make_null();
    }
    case MAGI_RT_LIST_DIR: {
        const char* path = magi_as_string(a);
        DIR* d = opendir(path);
        if (!d) return __magi_array_new(0, NULL);
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = 0; arr->cap = 32;
        arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
        struct dirent* entry;
        while ((entry = readdir(d)) != NULL) {
            if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
            if (arr->len >= arr->cap) { arr->cap *= 2; arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap); }
            arr->data[arr->len++] = magi_make_string(strdup(entry->d_name));
        }
        closedir(d);
        for (int i = 1; i < arr->len; i++) {
            int64_t key = arr->data[i];
            const char* ks = magi_as_string(key);
            int j = i - 1;
            while (j >= 0 && strcmp(magi_as_string(arr->data[j]), ks) > 0) { arr->data[j+1] = arr->data[j]; j--; }
            arr->data[j+1] = key;
        }
        return magi_make_array_val(arr);
    }

    // ── JSON ──
    case MAGI_RT_JSON_PARSE: {
        // Delegate to the string-based handler for the complex JSON parser
        return __magi_runtime_call("parse_json", argc, args);
    }
    case MAGI_RT_JSON_STRINGIFY:
        return magi_make_string(magi_to_json(a));

    // ── Renderers ──
    case MAGI_RT_RENDER_SEG_COLS:
        return __magi_runtime_call("__render_seg_cols", argc, args);
    case MAGI_RT_RENDER_WALL_COL:
        return __magi_runtime_call("__render_wall_col", argc, args);
    case MAGI_RT_RENDER_FLAT_COL:
        return __magi_runtime_call("__render_flat_col", argc, args);

    // ── Arena ──
    case MAGI_RT_ARENA_RESET:
        __magi_arena_reset();
        return magi_make_null();

    default:
        return magi_make_null();
    }
}

int64_t __magi_runtime_call(const char* name, int32_t argc, int64_t* args) {
    int64_t a = argc > 0 ? args[0] : magi_make_null();
    int64_t b = argc > 1 ? args[1] : magi_make_null();

    // Fast path: common arithmetic/comparisons (avoids strcmp chain)
    if (name[0] == '_' && name[1] == '_') {
        int64_t fast = __magi_fast_binop(name, a, b);
        if (fast != FAST_BINOP_SENTINEL) return fast;
    }

    // Arithmetic
    if (strcmp(name, "__add") == 0) {
        int ta = magi_get_tag(a), tb = magi_get_tag(b);
        if (ta == TAG_I64 && tb == TAG_I64)
            return magi_make_int(magi_sext48(magi_get_payload(a)) + magi_sext48(magi_get_payload(b)));
        if (ta == TAG_STRING && tb == TAG_STRING)
            return __magi_string_concat(a, b);
        if (ta == TAG_STRING || tb == TAG_STRING) {
            // String + other: convert other to string, then concat
            int64_t sa = (ta == TAG_STRING) ? a : __magi_to_string(a);
            int64_t sb = (tb == TAG_STRING) ? b : __magi_to_string(b);
            return __magi_string_concat(sa, sb);
        }
        return magi_make_float(magi_as_float(a) + magi_as_float(b));
    }
    if (strcmp(name, "__sub") == 0) {
        int ta = magi_get_tag(a), tb = magi_get_tag(b);
        if (ta == TAG_I64 && tb == TAG_I64)
            return magi_make_int(magi_sext48(magi_get_payload(a)) - magi_sext48(magi_get_payload(b)));
        return magi_make_float(magi_as_float(a) - magi_as_float(b));
    }
    if (strcmp(name, "__mul") == 0) {
        int ta = magi_get_tag(a), tb = magi_get_tag(b);
        if (ta == TAG_I64 && tb == TAG_I64)
            return magi_make_int(magi_sext48(magi_get_payload(a)) * magi_sext48(magi_get_payload(b)));
        // String repeat: "x" * n or n * "x"
        if (ta == TAG_STRING && tb == TAG_I64) {
            int64_t repeat_args[2] = { a, b };
            return __magi_runtime_call("__repeat", 2, repeat_args);
        }
        if (ta == TAG_I64 && tb == TAG_STRING) {
            int64_t repeat_args[2] = { b, a };
            return __magi_runtime_call("__repeat", 2, repeat_args);
        }
        return magi_make_float(magi_as_float(a) * magi_as_float(b));
    }
    if (strcmp(name, "__div") == 0) {
        int ta = magi_get_tag(a), tb = magi_get_tag(b);
        if (ta == TAG_I64 && tb == TAG_I64) {
            int64_t bv = magi_sext48(magi_get_payload(b));
            return (bv == 0) ? magi_make_int(0) : magi_make_int(magi_sext48(magi_get_payload(a)) / bv);
        }
        return magi_make_float(magi_as_float(a) / magi_as_float(b));
    }
    if (strcmp(name, "__mod") == 0 || strcmp(name, "__rem") == 0) {
        int ta = magi_get_tag(a), tb = magi_get_tag(b);
        if (ta == TAG_I64 && tb == TAG_I64) {
            int64_t bv = magi_sext48(magi_get_payload(b));
            return (bv == 0) ? magi_make_int(0) : magi_make_int(magi_sext48(magi_get_payload(a)) % bv);
        }
        return magi_make_float(fmod(magi_as_float(a), magi_as_float(b)));
    }
    if (strcmp(name, "__pow") == 0)
        return magi_make_float(pow(magi_as_float(a), magi_as_float(b)));
    if (strcmp(name, "__neg") == 0) {
        if (magi_get_tag(a) == TAG_I64) return magi_make_int(-magi_sext48(magi_get_payload(a)));
        return magi_make_float(-magi_as_float(a));
    }

    // Comparison
    if (strcmp(name, "__eq") == 0) {
        if (a == b) return magi_make_bool(1);
        // String content equality: two different string pointers may have same content
        if (magi_get_tag(a) == TAG_STRING && magi_get_tag(b) == TAG_STRING)
            return magi_make_bool(strcmp(magi_as_string(a), magi_as_string(b)) == 0);
        // Deep array equality
        if (magi_get_tag(a) == TAG_ARRAY && magi_get_tag(b) == TAG_ARRAY) {
            MagiArray* aa = magi_array_ptr(a);
            MagiArray* ab = magi_array_ptr(b);
            if (!aa && !ab) return magi_make_bool(1);
            if (!aa || !ab || aa->len != ab->len) return magi_make_bool(0);
            for (int i = 0; i < aa->len; i++) {
                int64_t eq_args[2] = {aa->data[i], ab->data[i]};
                int64_t eq = __magi_runtime_call("__eq", 2, eq_args);
                if (!magi_as_bool(eq)) return magi_make_bool(0);
            }
            return magi_make_bool(1);
        }
        // Deep map equality
        if (magi_get_tag(a) == TAG_MAP && magi_get_tag(b) == TAG_MAP) {
            MagiMap* ma = magi_map_ptr(a);
            MagiMap* mb = magi_map_ptr(b);
            if (!ma && !mb) return magi_make_bool(1);
            if (!ma || !mb || ma->len != mb->len) return magi_make_bool(0);
            for (int i = 0; i < ma->len; i++) {
                int found = 0;
                for (int j = 0; j < mb->len; j++) {
                    if (strcmp(ma->keys[i], mb->keys[j]) == 0) {
                        int64_t eq_args[2] = {ma->values[i], mb->values[j]};
                        int64_t eq = __magi_runtime_call("__eq", 2, eq_args);
                        if (!magi_as_bool(eq)) return magi_make_bool(0);
                        found = 1;
                        break;
                    }
                }
                if (!found) return magi_make_bool(0);
            }
            return magi_make_bool(1);
        }
        // Float equality: -0.0 == 0.0 (different bits but equal as doubles)
        if (!magi_is_tagged(a) && !magi_is_tagged(b)) {
            double da, db;
            memcpy(&da, &a, sizeof(da));
            memcpy(&db, &b, sizeof(db));
            return magi_make_bool(da == db);
        }
        return magi_make_bool(0);
    }
    if (strcmp(name, "__ne") == 0) {
        // Negate __eq for consistent deep equality
        int64_t eq_args[2] = {a, b};
        int64_t eq = __magi_runtime_call("__eq", 2, eq_args);
        return magi_make_bool(!magi_as_bool(eq));
    }
    if (strcmp(name, "__lt") == 0) {
        if (magi_get_tag(a) == TAG_I64 && magi_get_tag(b) == TAG_I64)
            return magi_make_bool(magi_sext48(magi_get_payload(a)) < magi_sext48(magi_get_payload(b)));
        return magi_make_bool(magi_as_float(a) < magi_as_float(b));
    }
    if (strcmp(name, "__gt") == 0) {
        if (magi_get_tag(a) == TAG_I64 && magi_get_tag(b) == TAG_I64)
            return magi_make_bool(magi_sext48(magi_get_payload(a)) > magi_sext48(magi_get_payload(b)));
        return magi_make_bool(magi_as_float(a) > magi_as_float(b));
    }
    if (strcmp(name, "__le") == 0) {
        if (magi_get_tag(a) == TAG_I64 && magi_get_tag(b) == TAG_I64)
            return magi_make_bool(magi_sext48(magi_get_payload(a)) <= magi_sext48(magi_get_payload(b)));
        return magi_make_bool(magi_as_float(a) <= magi_as_float(b));
    }
    if (strcmp(name, "__ge") == 0) {
        if (magi_get_tag(a) == TAG_I64 && magi_get_tag(b) == TAG_I64)
            return magi_make_bool(magi_sext48(magi_get_payload(a)) >= magi_sext48(magi_get_payload(b)));
        return magi_make_bool(magi_as_float(a) >= magi_as_float(b));
    }

    // Logical
    if (strcmp(name, "__and") == 0) return magi_make_bool(magi_as_bool(a) && magi_as_bool(b));
    if (strcmp(name, "__or") == 0) return magi_make_bool(magi_as_bool(a) || magi_as_bool(b));
    if (strcmp(name, "__not") == 0) return magi_make_bool(!magi_as_bool(a));

    // Bitwise
    if (strcmp(name, "__bit_and") == 0) return magi_make_int(magi_as_int(a) & magi_as_int(b));
    if (strcmp(name, "__bit_or") == 0) return magi_make_int(magi_as_int(a) | magi_as_int(b));
    if (strcmp(name, "__bit_xor") == 0) return magi_make_int(magi_as_int(a) ^ magi_as_int(b));
    if (strcmp(name, "__shl") == 0 || strcmp(name, "__bit_shl") == 0) return magi_make_int(magi_as_int(a) << (magi_as_int(b) & 63));
    if (strcmp(name, "__shr") == 0 || strcmp(name, "__bit_shr") == 0) return magi_make_int(magi_as_int(a) >> (magi_as_int(b) & 63));
    if (strcmp(name, "__bit_andnot") == 0) return magi_make_int(magi_as_int(a) & ~magi_as_int(b));
    if (strcmp(name, "__bit_not") == 0) return magi_make_int(~magi_as_int(a));

    // Range
    if (strcmp(name, "__range") == 0) {
        int64_t start = magi_as_int(a);
        int64_t end = magi_as_int(b);
        int inclusive = (argc > 2) ? magi_as_bool(args[2]) : 0;
        int64_t count = inclusive ? (end - start + 1) : (end - start);
        if (count < 0) count = 0;
        if (count > 10000000) count = 10000000;
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = (int32_t)count;
        arr->cap = (int32_t)(count > 8 ? count : 8);
        arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
        for (int64_t i = 0; i < count; i++) {
            arr->data[i] = magi_make_int(start + i);
        }
        return magi_make_array_val(arr);
    }

    // Collection operations
    if (strcmp(name, "len") == 0) {
        if (magi_get_tag(a) == TAG_ARRAY) return __magi_array_len(a);
        if (magi_get_tag(a) == TAG_STRING) return __magi_string_len(a);
        if (magi_get_tag(a) == TAG_MAP) {
            MagiMap* map = magi_map_ptr(a);
            return magi_make_int(map ? map->len : 0);
        }
        return magi_make_int(0);
    }
    if (strcmp(name, "to_string") == 0 || strcmp(name, "string") == 0) return __magi_to_string(a);
    if (strcmp(name, "typeof") == 0 || strcmp(name, "type_of") == 0) {
        int t = magi_get_tag(a);
        const char* tn;
        switch(t) {
            case TAG_NULL: tn = "null"; break;
            case TAG_BOOL: tn = "bool"; break;
            case TAG_I64: tn = "int"; break;
            case TAG_STRING: tn = "string"; break;
            case TAG_ARRAY: tn = "array"; break;
            case TAG_MAP: tn = "map"; break;
            case 8: tn = "float"; break;
            default: tn = "unknown"; break;
        }
        return magi_make_string(tn);
    }
    if (strcmp(name, "push") == 0 || strcmp(name, "array_push") == 0 || strcmp(name, "__array_push") == 0)
        return __magi_array_push(a, b);
    if (strcmp(name, "pop") == 0 || strcmp(name, "array_pop") == 0) {
        MagiArray* arr = magi_array_ptr(a);
        if (!arr || arr->len == 0) return magi_make_null();
        return arr->data[--arr->len];
    }
    if (strcmp(name, "has") == 0) {
        if (magi_get_tag(a) == TAG_MAP) {
            MagiMap* map = magi_map_ptr(a);
            const char* key = magi_as_string(b);
            if (!map || !key) return magi_make_bool(0);
            for (int i = 0; i < map->len; i++) {
                if (strcmp(map->keys[i], key) == 0) return magi_make_bool(1);
            }
            return magi_make_bool(0);
        }
        if (magi_get_tag(a) == TAG_ARRAY) {
            MagiArray* arr = magi_array_ptr(a);
            if (!arr) return magi_make_bool(0);
            for (int i = 0; i < arr->len; i++) {
                if (arr->data[i] == b) return magi_make_bool(1);
                if (magi_get_tag(arr->data[i]) == TAG_STRING && magi_get_tag(b) == TAG_STRING &&
                    strcmp(magi_as_string(arr->data[i]), magi_as_string(b)) == 0)
                    return magi_make_bool(1);
            }
            return magi_make_bool(0);
        }
        return magi_make_bool(0);
    }
    if (strcmp(name, "contains") == 0) {
        // Alias for has
        int64_t has_args[2] = { a, b };
        return __magi_runtime_call("has", 2, has_args);
    }

    // Array/Collection methods with callbacks
    if (strcmp(name, "map") == 0) return magi_method_map(a, b);
    if (strcmp(name, "filter") == 0) return magi_method_filter(a, b);
    if (strcmp(name, "reduce") == 0) { int64_t c = argc > 2 ? args[2] : magi_make_null(); return magi_method_reduce(a, b, c); }
    if (strcmp(name, "for_each") == 0 || strcmp(name, "forEach") == 0) return magi_method_for_each(a, b);
    if (strcmp(name, "find") == 0) return magi_method_find(a, b);
    if (strcmp(name, "every") == 0) return magi_method_every(a, b);
    if (strcmp(name, "some") == 0) return magi_method_some(a, b);
    if (strcmp(name, "flat_map") == 0 || strcmp(name, "flatMap") == 0) return magi_method_flat_map(a, b);
    if (strcmp(name, "sort_by") == 0 || strcmp(name, "sortBy") == 0) {
        MagiArray* arr = magi_array_ptr(a);
        if (!arr || arr->len <= 1) return a;
        // Simple insertion sort with comparator
        for (int i = 1; i < arr->len; i++) {
            int64_t key = arr->data[i];
            int j = i - 1;
            while (j >= 0) {
                int64_t cmp_args[2] = { arr->data[j], key };
                int64_t cmp = __magi_call_fn(b, 2, cmp_args);
                if (magi_as_int(cmp) <= 0) break;
                arr->data[j + 1] = arr->data[j];
                j--;
            }
            arr->data[j + 1] = key;
        }
        return a;
    }

    // Array operations without callbacks
    if (strcmp(name, "reverse") == 0) {
        MagiArray* arr = magi_array_ptr(a);
        if (!arr) return a;
        for (int i = 0, j = arr->len - 1; i < j; i++, j--) {
            int64_t tmp = arr->data[i]; arr->data[i] = arr->data[j]; arr->data[j] = tmp;
        }
        return a;
    }
    if (strcmp(name, "sort") == 0) {
        MagiArray* arr = magi_array_ptr(a);
        if (!arr || arr->len <= 1) return a;
        // Simple insertion sort by numeric value
        for (int i = 1; i < arr->len; i++) {
            int64_t key = arr->data[i];
            double kv = magi_as_float(key);
            int j = i - 1;
            while (j >= 0 && magi_as_float(arr->data[j]) > kv) { arr->data[j+1] = arr->data[j]; j--; }
            arr->data[j + 1] = key;
        }
        return a;
    }
    if (strcmp(name, "contains") == 0 || strcmp(name, "includes") == 0) {
        if (magi_get_tag(a) == TAG_ARRAY) {
            MagiArray* arr = magi_array_ptr(a);
            if (!arr) return magi_make_bool(0);
            for (int i = 0; i < arr->len; i++) { if (arr->data[i] == b) return magi_make_bool(1); }
            return magi_make_bool(0);
        }
        if (magi_get_tag(a) == TAG_STRING) {
            const char* s = magi_as_string(a);
            const char* sub = magi_as_string(b);
            return magi_make_bool(strstr(s, sub) != NULL);
        }
        return magi_make_bool(0);
    }
    if (strcmp(name, "index_of") == 0 || strcmp(name, "indexOf") == 0) {
        if (magi_get_tag(a) == TAG_ARRAY) {
            MagiArray* arr = magi_array_ptr(a);
            if (!arr) return magi_make_int(-1);
            for (int i = 0; i < arr->len; i++) { if (arr->data[i] == b) return magi_make_int(i); }
            return magi_make_int(-1);
        }
        if (magi_get_tag(a) == TAG_STRING) {
            const char* s = magi_as_string(a);
            const char* sub = magi_as_string(b);
            const char* p = strstr(s, sub);
            return p ? magi_make_int(p - s) : magi_make_int(-1);
        }
        return magi_make_int(-1);
    }
    if (strcmp(name, "join") == 0) {
        MagiArray* arr = magi_array_ptr(a);
        if (!arr) return magi_make_string("");
        const char* sep = magi_as_string(b);
        size_t total = 0, seplen = strlen(sep);
        char** strs = (char**)malloc(sizeof(char*) * arr->len);
        for (int i = 0; i < arr->len; i++) {
            char buf[256]; magi_val_to_str(arr->data[i], buf, sizeof(buf));
            // Strip quotes from string representation
            if (magi_get_tag(arr->data[i]) == TAG_STRING) {
                strs[i] = strdup(magi_as_string(arr->data[i]));
            } else {
                strs[i] = strdup(buf);
            }
            total += strlen(strs[i]);
        }
        total += seplen * (arr->len > 0 ? arr->len - 1 : 0);
        char* result = (char*)malloc(total + 1);
        result[0] = '\0';
        for (int i = 0; i < arr->len; i++) {
            if (i > 0) strcat(result, sep);
            strcat(result, strs[i]);
            free(strs[i]);
        }
        free(strs);
        return magi_make_string(result);
    }
    if (strcmp(name, "__slice") == 0) {
        // __slice(arr, start, end, step) or __slice(str, start, end, step)
        int64_t start_v = argc > 1 ? args[1] : magi_make_int(0);
        int64_t end_v = argc > 2 ? args[2] : magi_make_null();
        int64_t start = magi_as_int(start_v);
        if (magi_get_tag(a) == TAG_ARRAY) {
            MagiArray* arr = magi_array_ptr(a);
            if (!arr) return magi_make_null();
            int64_t end = magi_is_tagged(end_v) && magi_get_tag(end_v) == TAG_NULL ? arr->len : magi_as_int(end_v);
            if (start < 0) start += arr->len;
            if (end < 0) end += arr->len;
            if (start < 0) start = 0;
            if (end > arr->len) end = arr->len;
            if (start >= end) return __magi_array_new(0, NULL);
            int64_t count = end - start;
            return __magi_array_new((int32_t)count, arr->data + start);
        }
        if (magi_get_tag(a) == TAG_STRING) {
            const char* s = magi_as_string(a);
            int64_t slen = (int64_t)strlen(s);
            int64_t end = magi_is_tagged(end_v) && magi_get_tag(end_v) == TAG_NULL ? slen : magi_as_int(end_v);
            if (start < 0) start += slen;
            if (end < 0) end += slen;
            if (start < 0) start = 0;
            if (end > slen) end = slen;
            if (start >= end) return magi_make_string("");
            int64_t count = end - start;
            char* result = (char*)malloc(count + 1);
            memcpy(result, s + start, count);
            result[count] = '\0';
            return magi_make_string(result);
        }
        return magi_make_null();
    }
    if (strcmp(name, "__repeat") == 0) {
        // String repeat: "x" * n
        if (magi_get_tag(a) == TAG_STRING) {
            const char* s = magi_as_string(a);
            int64_t n = magi_as_int(b);
            if (n <= 0) return magi_make_string("");
            size_t slen = strlen(s);
            char* result = (char*)malloc(slen * n + 1);
            result[0] = '\0';
            for (int64_t i = 0; i < n; i++) memcpy(result + i * slen, s, slen);
            result[slen * n] = '\0';
            return magi_make_string(result);
        }
        return magi_make_null();
    }

    // Map operations
    if (strcmp(name, "map_get") == 0) return __magi_map_get(a, b);
    if (strcmp(name, "map_set") == 0) {
        int64_t val = argc > 2 ? args[2] : magi_make_null();
        __magi_map_set(a, b, val);
        return magi_make_null();
    }
    if (strcmp(name, "keys") == 0) {
        MagiMap* map = magi_map_ptr(a);
        if (!map) return __magi_array_new(0, NULL);
        int64_t* elems = (int64_t*)malloc(sizeof(int64_t) * map->len);
        for (int i = 0; i < map->len; i++) elems[i] = magi_make_string(map->keys[i]);
        int64_t result = __magi_array_new(map->len, elems);
        free(elems);
        return result;
    }
    if (strcmp(name, "values") == 0) {
        MagiMap* map = magi_map_ptr(a);
        if (!map) return __magi_array_new(0, NULL);
        return __magi_array_new(map->len, map->values);
    }
    if (strcmp(name, "entries") == 0) {
        MagiMap* map = magi_map_ptr(a);
        if (!map) return __magi_array_new(0, NULL);
        int64_t* pairs = (int64_t*)malloc(sizeof(int64_t) * map->len);
        for (int i = 0; i < map->len; i++) {
            int64_t pair_data[2] = { magi_make_string(map->keys[i]), map->values[i] };
            pairs[i] = __magi_array_new(2, pair_data);
        }
        int64_t result = __magi_array_new(map->len, pairs);
        free(pairs);
        return result;
    }
    if (strcmp(name, "has_key") == 0 || strcmp(name, "hasKey") == 0) {
        MagiMap* map = magi_map_ptr(a);
        const char* key = magi_as_string(b);
        if (!map) return magi_make_bool(0);
        for (int i = 0; i < map->len; i++) { if (strcmp(map->keys[i], key) == 0) return magi_make_bool(1); }
        return magi_make_bool(0);
    }

    // String operations
    if (strcmp(name, "parse_int") == 0) {
        const char* s = magi_as_string(a);
        return magi_make_int((int64_t)atoll(s));
    }
    if (strcmp(name, "parse_float") == 0) {
        const char* s = magi_as_string(a);
        return magi_make_float(atof(s));
    }
    if (strcmp(name, "concat") == 0) return __magi_string_concat(a, b);
    if (strcmp(name, "split") == 0) {
        const char* s = magi_as_string(a);
        const char* delim = magi_as_string(b);
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = 0; arr->cap = 16;
        arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
        size_t dlen = strlen(delim);
        if (dlen == 0) {
            // Split into characters
            for (size_t i = 0; i < strlen(s); i++) {
                char* ch = (char*)malloc(2); ch[0] = s[i]; ch[1] = '\0';
                if (arr->len >= arr->cap) { arr->cap *= 2; arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap); }
                arr->data[arr->len++] = magi_make_string(ch);
            }
        } else {
            const char* p = s;
            while (1) {
                const char* found = strstr(p, delim);
                size_t part_len = found ? (size_t)(found - p) : strlen(p);
                char* part = (char*)malloc(part_len + 1);
                memcpy(part, p, part_len); part[part_len] = '\0';
                if (arr->len >= arr->cap) { arr->cap *= 2; arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap); }
                arr->data[arr->len++] = magi_make_string(part);
                if (!found) break;
                p = found + dlen;
            }
        }
        return magi_make_array_val(arr);
    }
    if (strcmp(name, "trim") == 0) {
        const char* s = magi_as_string(a);
        size_t len = strlen(s);
        size_t start = 0, end = len;
        while (start < len && (s[start] == ' ' || s[start] == '\t' || s[start] == '\n' || s[start] == '\r')) start++;
        while (end > start && (s[end-1] == ' ' || s[end-1] == '\t' || s[end-1] == '\n' || s[end-1] == '\r')) end--;
        char* result = (char*)malloc(end - start + 1);
        memcpy(result, s + start, end - start); result[end - start] = '\0';
        return magi_make_string(result);
    }
    if (strcmp(name, "upper") == 0 || strcmp(name, "to_upper") == 0 || strcmp(name, "toUpperCase") == 0) {
        const char* s = magi_as_string(a);
        size_t len = strlen(s);
        char* result = (char*)malloc(len + 1);
        for (size_t i = 0; i < len; i++) result[i] = (s[i] >= 'a' && s[i] <= 'z') ? s[i] - 32 : s[i];
        result[len] = '\0';
        return magi_make_string(result);
    }
    if (strcmp(name, "lower") == 0 || strcmp(name, "to_lower") == 0 || strcmp(name, "toLowerCase") == 0) {
        const char* s = magi_as_string(a);
        size_t len = strlen(s);
        char* result = (char*)malloc(len + 1);
        for (size_t i = 0; i < len; i++) result[i] = (s[i] >= 'A' && s[i] <= 'Z') ? s[i] + 32 : s[i];
        result[len] = '\0';
        return magi_make_string(result);
    }
    if (strcmp(name, "starts_with") == 0 || strcmp(name, "startsWith") == 0) {
        return magi_make_bool(strncmp(magi_as_string(a), magi_as_string(b), strlen(magi_as_string(b))) == 0);
    }
    if (strcmp(name, "ends_with") == 0 || strcmp(name, "endsWith") == 0) {
        const char* s = magi_as_string(a), *suffix = magi_as_string(b);
        size_t sl = strlen(s), sufl = strlen(suffix);
        return magi_make_bool(sl >= sufl && strcmp(s + sl - sufl, suffix) == 0);
    }
    if (strcmp(name, "replace") == 0) {
        const char* s = magi_as_string(a);
        const char* from = magi_as_string(b);
        const char* to = argc > 2 ? magi_as_string(args[2]) : "";
        size_t slen = strlen(s), flen = strlen(from), tlen = strlen(to);
        if (flen == 0) return a;
        // Count occurrences
        int count = 0;
        const char* p = s;
        while ((p = strstr(p, from))) { count++; p += flen; }
        char* result = (char*)malloc(slen + count * (tlen - flen) + 1);
        char* w = result;
        p = s;
        while (*p) {
            if (strncmp(p, from, flen) == 0) { memcpy(w, to, tlen); w += tlen; p += flen; }
            else { *w++ = *p++; }
        }
        *w = '\0';
        return magi_make_string(result);
    }
    if (strcmp(name, "substring") == 0 || strcmp(name, "substr") == 0 || strcmp(name, "slice") == 0) {
        const char* s = magi_as_string(a);
        int64_t slen = (int64_t)strlen(s);
        int64_t start = magi_as_int(b);
        int64_t end = argc > 2 ? magi_as_int(args[2]) : slen;
        if (start < 0) start += slen;
        if (end < 0) end += slen;
        if (start < 0) start = 0;
        if (end > slen) end = slen;
        if (start >= end) return magi_make_string("");
        int64_t cnt = end - start;
        char* result = (char*)malloc(cnt + 1);
        memcpy(result, s + start, cnt); result[cnt] = '\0';
        return magi_make_string(result);
    }
    if (strcmp(name, "char_at") == 0 || strcmp(name, "charAt") == 0) {
        const char* s = magi_as_string(a);
        int64_t idx = magi_as_int(b);
        size_t slen = strlen(s);
        if (idx < 0 || idx >= (int64_t)slen) return magi_make_string("");
        char* result = (char*)malloc(2);
        result[0] = s[idx]; result[1] = '\0';
        return magi_make_string(result);
    }

    // Math
    if (strcmp(name, "abs") == 0) {
        if (magi_get_tag(a) == TAG_I64) {
            int64_t v = magi_sext48(magi_get_payload(a));
            return magi_make_int(v < 0 ? -v : v);
        }
        return magi_make_float(fabs(magi_as_float(a)));
    }
    if (strcmp(name, "floor") == 0) return magi_make_float(floor(magi_as_float(a)));
    if (strcmp(name, "ceil") == 0) return magi_make_float(ceil(magi_as_float(a)));
    if (strcmp(name, "sqrt") == 0) return magi_make_float(sqrt(magi_as_float(a)));
    if (strcmp(name, "round") == 0) return magi_make_float(round(magi_as_float(a)));
    if (strcmp(name, "sin") == 0) return magi_make_float(sin(magi_as_float(a)));
    if (strcmp(name, "cos") == 0) return magi_make_float(cos(magi_as_float(a)));
    if (strcmp(name, "tan") == 0) return magi_make_float(tan(magi_as_float(a)));
    if (strcmp(name, "log") == 0) return magi_make_float(log(magi_as_float(a)));
    if (strcmp(name, "log2") == 0) return magi_make_float(log2(magi_as_float(a)));
    if (strcmp(name, "log10") == 0) return magi_make_float(log10(magi_as_float(a)));
    if (strcmp(name, "exp") == 0) return magi_make_float(exp(magi_as_float(a)));
    if (strcmp(name, "atan") == 0) return magi_make_float(atan(magi_as_float(a)));
    if (strcmp(name, "atan2") == 0) return magi_make_float(atan2(magi_as_float(a), magi_as_float(b)));
    if (strcmp(name, "asin") == 0) return magi_make_float(asin(magi_as_float(a)));
    if (strcmp(name, "acos") == 0) return magi_make_float(acos(magi_as_float(a)));
    if (strcmp(name, "pow") == 0) return magi_make_float(pow(magi_as_float(a), magi_as_float(b)));
    if (strcmp(name, "fmod") == 0) return magi_make_float(fmod(magi_as_float(a), magi_as_float(b)));
    if (strcmp(name, "min") == 0) {
        double da = magi_as_float(a), db = magi_as_float(b);
        return da < db ? a : b;
    }
    if (strcmp(name, "max") == 0) {
        double da = magi_as_float(a), db = magi_as_float(b);
        return da > db ? a : b;
    }
    if (strcmp(name, "random") == 0) return magi_make_float((double)rand() / RAND_MAX);
    if (strcmp(name, "is_nan") == 0 || strcmp(name, "isNaN") == 0) {
        if (!magi_is_tagged(a)) { double d; memcpy(&d, &a, sizeof(d)); return magi_make_bool(isnan(d)); }
        return magi_make_bool(0);
    }
    if (strcmp(name, "is_finite") == 0 || strcmp(name, "isFinite") == 0) {
        if (!magi_is_tagged(a)) { double d; memcpy(&d, &a, sizeof(d)); return magi_make_bool(isfinite(d)); }
        return magi_make_bool(1);
    }

    // Path operations
    if (strcmp(name, "path_join") == 0) {
        const char* p1 = magi_as_string(a);
        const char* p2 = magi_as_string(b);
        size_t l1 = strlen(p1), l2 = strlen(p2);
        char* result = (char*)malloc(l1 + l2 + 2);
        memcpy(result, p1, l1);
        if (l1 > 0 && p1[l1-1] != '/') { result[l1] = '/'; memcpy(result+l1+1, p2, l2+1); }
        else { memcpy(result+l1, p2, l2+1); }
        return magi_make_string(result);
    }

    // Binary file read — returns array of byte values
    if (strcmp(name, "fs_read_bytes") == 0 || strcmp(name, "read_file_bytes") == 0) {
        const char* path = magi_as_string(a);
        FILE* f = fopen(path, "rb");
        if (!f) return magi_make_null();
        fseek(f, 0, SEEK_END);
        long len = ftell(f);
        fseek(f, 0, SEEK_SET);
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = (int32_t)len;
        arr->cap = (int32_t)len;
        arr->data = (int64_t*)malloc(sizeof(int64_t) * len);
        unsigned char* buf = (unsigned char*)malloc(len);
        fread(buf, 1, len, f);
        fclose(f);
        for (long i = 0; i < len; i++) {
            arr->data[i] = magi_make_int(buf[i]);
        }
        free(buf);
        return magi_make_array_val(arr);
    }
    if (strcmp(name, "fs_size") == 0 || strcmp(name, "file_size") == 0) {
        const char* path = magi_as_string(a);
        FILE* f = fopen(path, "rb");
        if (!f) return magi_make_int(0);
        fseek(f, 0, SEEK_END);
        long sz = ftell(f);
        fclose(f);
        return magi_make_int(sz);
    }

    // File I/O
    if (strcmp(name, "fs_write") == 0 || strcmp(name, "file_write") == 0 || strcmp(name, "write_file") == 0) {
        const char* path = magi_as_string(a);
        const char* content = magi_as_string(b);
        FILE* f = fopen(path, "w");
        if (f) { fputs(content, f); fclose(f); return magi_make_string(path); }
        return magi_make_null();
    }
    // (fs_read/fs_exists/fs_delete handle these above)
    if (strcmp(name, "fs_read") == 0 || strcmp(name, "file_read") == 0 || strcmp(name, "read_file") == 0) {
        const char* path = magi_as_string(a);
        FILE* f = fopen(path, "r");
        if (!f) return magi_make_null();
        fseek(f, 0, SEEK_END);
        long len = ftell(f);
        fseek(f, 0, SEEK_SET);
        char* buf = (char*)malloc(len + 1);
        fread(buf, 1, len, f);
        buf[len] = '\0';
        fclose(f);
        return magi_make_string(buf);
    }
    if (strcmp(name, "fs_exists") == 0 || strcmp(name, "file_exists") == 0) {
        const char* path = magi_as_string(a);
        FILE* f = fopen(path, "r");
        if (f) { fclose(f); return magi_make_bool(1); }
        return magi_make_bool(0);
    }
    if (strcmp(name, "fs_delete") == 0 || strcmp(name, "file_delete") == 0 || strcmp(name, "delete_file") == 0) {
        return magi_make_bool(remove(magi_as_string(a)) == 0);
    }
    if (strcmp(name, "fs_read_lines") == 0) {
        const char* path = magi_as_string(a);
        FILE* f = fopen(path, "r");
        if (!f) return __magi_array_new(0, NULL);
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = 0; arr->cap = 32;
        arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
        char line[4096];
        while (fgets(line, sizeof(line), f)) {
            size_t len = strlen(line);
            if (len > 0 && line[len-1] == '\n') line[--len] = '\0';
            if (arr->len >= arr->cap) { arr->cap *= 2; arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap); }
            arr->data[arr->len++] = magi_make_string(strdup(line));
        }
        fclose(f);
        return magi_make_array_val(arr);
    }
    if (strcmp(name, "fs_mkdir") == 0 || strcmp(name, "mkdir") == 0 || strcmp(name, "create_dir") == 0) {
        #ifdef _WIN32
        int r = _mkdir(magi_as_string(a));
        #else
        int r = mkdir(magi_as_string(a), 0755);
        #endif
        return magi_make_bool(r == 0 || errno == EEXIST);
    }
    if (strcmp(name, "file_append") == 0 || strcmp(name, "append_file") == 0) {
        const char* path = magi_as_string(a);
        const char* content = magi_as_string(b);
        FILE* f = fopen(path, "a");
        if (f) { fputs(content, f); fclose(f); return magi_make_string(path); }
        return magi_make_null();
    }
    if (strcmp(name, "list_dir") == 0 || strcmp(name, "read_dir") == 0 || strcmp(name, "fs_list_dir") == 0 || strcmp(name, "fs_list") == 0) {
        const char* path = magi_as_string(a);
        DIR* d = opendir(path);
        if (!d) return __magi_array_new(0, NULL);
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = 0; arr->cap = 32;
        arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
        struct dirent* entry;
        while ((entry = readdir(d)) != NULL) {
            if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
            if (arr->len >= arr->cap) { arr->cap *= 2; arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap); }
            arr->data[arr->len++] = magi_make_string(strdup(entry->d_name));
        }
        closedir(d);
        // Sort entries for deterministic output
        for (int i = 1; i < arr->len; i++) {
            int64_t key = arr->data[i];
            const char* ks = magi_as_string(key);
            int j = i - 1;
            while (j >= 0 && strcmp(magi_as_string(arr->data[j]), ks) > 0) { arr->data[j+1] = arr->data[j]; j--; }
            arr->data[j+1] = key;
        }
        return magi_make_array_val(arr);
    }

    // JSON parse/stringify (basic implementation)
    if (strcmp(name, "parse_json") == 0 || strcmp(name, "json_parse") == 0) {
        const char* json = magi_as_string(a);
        // Skip whitespace
        while (*json == ' ' || *json == '\n' || *json == '\t' || *json == '\r') json++;
        if (*json == '{') {
            // Parse JSON object into a MagiMap
            MagiMap* map = (MagiMap*)malloc(sizeof(MagiMap));
            map->len = 0; map->cap = 16;
            map->keys = (char**)malloc(sizeof(char*) * map->cap);
            map->values = (int64_t*)malloc(sizeof(int64_t) * map->cap);
            map->hashes = (uint32_t*)malloc(sizeof(uint32_t) * map->cap);
            map->bucket_count = 32;
            map->buckets = (int32_t*)malloc(32 * sizeof(int32_t));
            memset(map->buckets, -1, 32 * sizeof(int32_t));
            json++; // skip {
            while (*json) {
                while (*json == ' ' || *json == '\n' || *json == '\t' || *json == '\r' || *json == ',') json++;
                if (*json == '}') break;
                // Parse key
                if (*json == '"') {
                    json++;
                    const char* key_start = json;
                    while (*json && *json != '"') json++;
                    size_t klen = json - key_start;
                    char* key = (char*)malloc(klen + 1);
                    memcpy(key, key_start, klen); key[klen] = '\0';
                    if (*json == '"') json++;
                    while (*json == ' ' || *json == ':') json++;
                    // Parse value
                    int64_t val = magi_make_null();
                    while (*json == ' ') json++;
                    if (*json == '"') {
                        json++;
                        const char* vs = json;
                        while (*json && *json != '"') json++;
                        size_t vlen = json - vs;
                        char* vstr = (char*)malloc(vlen + 1);
                        memcpy(vstr, vs, vlen); vstr[vlen] = '\0';
                        val = magi_make_string(vstr);
                        if (*json == '"') json++;
                    } else if (*json == 't') { val = magi_make_bool(1); json += 4; }
                    else if (*json == 'f') { val = magi_make_bool(0); json += 5; }
                    else if (*json == 'n') { val = magi_make_null(); json += 4; }
                    else if (*json == '[') {
                        // Parse array
                        json++;
                        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
                        arr->len = 0; arr->cap = 16;
                        arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
                        while (*json) {
                            while (*json == ' ' || *json == '\n' || *json == ',') json++;
                            if (*json == ']') { json++; break; }
                            if (*json == '"') {
                                json++;
                                const char* es = json;
                                while (*json && *json != '"') json++;
                                size_t elen = json - es;
                                char* estr = (char*)malloc(elen + 1);
                                memcpy(estr, es, elen); estr[elen] = '\0';
                                if (arr->len >= arr->cap) { arr->cap *= 2; arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap); }
                                arr->data[arr->len++] = magi_make_string(estr);
                                if (*json == '"') json++;
                            } else if (*json >= '0' && *json <= '9' || *json == '-') {
                                char* end;
                                double d = strtod(json, &end);
                                if (d == (double)(int64_t)d) {
                                    if (arr->len >= arr->cap) { arr->cap *= 2; arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap); }
                                    arr->data[arr->len++] = magi_make_int((int64_t)d);
                                } else {
                                    if (arr->len >= arr->cap) { arr->cap *= 2; arr->data = (int64_t*)realloc(arr->data, sizeof(int64_t) * arr->cap); }
                                    arr->data[arr->len++] = magi_make_float(d);
                                }
                                json = end;
                            } else break;
                        }
                        val = magi_make_array_val(arr);
                    } else if ((*json >= '0' && *json <= '9') || *json == '-') {
                        char* end;
                        double d = strtod(json, &end);
                        if (d == (double)(int64_t)d) val = magi_make_int((int64_t)d);
                        else val = magi_make_float(d);
                        json = end;
                    }
                    if (map->len >= map->cap) { map->cap *= 2; map->keys = (char**)realloc(map->keys, sizeof(char*) * map->cap); map->values = (int64_t*)realloc(map->values, sizeof(int64_t) * map->cap); map->hashes = (uint32_t*)realloc(map->hashes, sizeof(uint32_t) * map->cap); }
                    uint32_t kh = fnv1a(key);
                    map->keys[map->len] = key;
                    map->values[map->len] = val;
                    map->hashes[map->len] = kh;
                    if (map->len * 4 > map->bucket_count * 3) {
                        map->len++;
                        magi_map_rehash(map);
                    } else {
                        uint32_t ks = kh & (uint32_t)(map->bucket_count - 1);
                        while (map->buckets[ks] != -1) ks = (ks + 1) & (uint32_t)(map->bucket_count - 1);
                        map->buckets[ks] = map->len;
                        map->len++;
                    }
                } else break;
            }
            return magi_make_map_val(map);
        }
        return magi_make_null();
    }
    if (strcmp(name, "stringify_json") == 0 || strcmp(name, "json_stringify") == 0 || strcmp(name, "to_json") == 0) {
        // Produce JSON format: {"key":"value"} with quoted keys and no spaces
        return magi_make_string(magi_to_json(a));
    }

    // Process/OS operations
    if (strcmp(name, "process_args") == 0 || strcmp(name, "args") == 0) {
        // Return command line args (stored by main)
        extern int __magi_argc;
        extern char** __magi_argv;
        MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
        arr->len = __magi_argc > 1 ? __magi_argc - 1 : 0;
        arr->cap = arr->len > 8 ? arr->len : 8;
        arr->data = (int64_t*)malloc(sizeof(int64_t) * arr->cap);
        for (int i = 1; i < __magi_argc; i++) {
            arr->data[i - 1] = magi_make_string(__magi_argv[i]);
        }
        return magi_make_array_val(arr);
    }
    if (strcmp(name, "env_get") == 0) {
        const char* key = magi_as_string(a);
        const char* val = getenv(key);
        return val ? magi_make_string(val) : magi_make_null();
    }
    if (strcmp(name, "env_set") == 0) {
        #ifdef _WIN32
        _putenv_s(magi_as_string(a), magi_as_string(b));
        #else
        setenv(magi_as_string(a), magi_as_string(b), 1);
        #endif
        return magi_make_null();
    }
    if (strcmp(name, "env_has") == 0) {
        return magi_make_bool(getenv(magi_as_string(a)) != NULL);
    }
    if (strcmp(name, "timestamp_ms") == 0 || strcmp(name, "time_ms") == 0) {
        #ifdef _WIN32
        return magi_make_int((int64_t)GetTickCount64());
        #else
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        return magi_make_int(ts.tv_sec * 1000 + ts.tv_nsec / 1000000);
        #endif
    }
    if (strcmp(name, "exit") == 0) {
        exit((int)magi_as_int(a));
    }
    if (strcmp(name, "panic") == 0) {
        const char* msg = magi_as_string(a);
        fprintf(stderr, "panic: %s\n", msg);
        exit(1);
    }
    if (strcmp(name, "exec_cmd") == 0) {
        const char* cmd = magi_as_string(a);
        int r = system(cmd);
        return magi_make_int(r);
    }
    if (strcmp(name, "cwd") == 0) {
        char buf[4096];
        if (getcwd(buf, sizeof(buf))) return magi_make_string(strdup(buf));
        return magi_make_string("/");
    }
    if (strcmp(name, "os_name") == 0) {
        #ifdef __linux__
        return magi_make_string("linux");
        #elif __APPLE__
        return magi_make_string("macos");
        #elif _WIN32
        return magi_make_string("windows");
        #else
        return magi_make_string("unknown");
        #endif
    }
    if (strcmp(name, "pid") == 0) {
        return magi_make_int(getpid());
    }

    if (strcmp(name, "__byte_slice") == 0) {
        int64_t c = argc > 2 ? args[2] : magi_make_int(0);
        return __magi_byte_slice(a, b, c);
    }

    if (strcmp(name, "__arena_reset") == 0) {
        __magi_arena_reset();
        return magi_make_null();
    }
    if (strcmp(name, "__arena_enter") == 0) {
        __magi_arena_enter();
        return magi_make_null();
    }
    if (strcmp(name, "__arena_leave") == 0) {
        __magi_arena_leave();
        return magi_make_null();
    }
    if (strcmp(name, "__heap_allocated") == 0) {
        return magi_make_int((int64_t)magi_total_malloc);
    }

    // Native seg renderer: processes an entire seg's column range in C
    // Args: fb, hor_ocl, floor_ocl, ceil_ocl,
    //       scr_x1, scr_x2, cx1, cy1, cx2, cy2,
    //       floor_h, ceil_h, back_floor_h, back_ceil_h,
    //       has_back, has_upper, has_lower,
    //       light_level, game_focus_x, aspect_ratio, cam_focus_x, cam_focus_y
    if (strcmp(name, "__render_seg_cols") == 0 && argc >= 22) {
        MagiArray* fb = magi_array_ptr(args[0]);
        MagiArray* hor_ocl = magi_array_ptr(args[1]);
        MagiArray* floor_ocl = magi_array_ptr(args[2]);
        MagiArray* ceil_ocl = magi_array_ptr(args[3]);
        if (!fb || !hor_ocl || !floor_ocl || !ceil_ocl) return magi_make_null();
        int scr_x1 = (int)magi_as_int(args[4]);
        int scr_x2 = (int)magi_as_int(args[5]);
        double cx1 = magi_as_float(args[6]);
        double cy1 = magi_as_float(args[7]);
        double cx2 = magi_as_float(args[8]);
        double cy2 = magi_as_float(args[9]);
        double floor_h = magi_as_float(args[10]);
        double ceil_h = magi_as_float(args[11]);
        double back_floor_h = magi_as_float(args[12]);
        double back_ceil_h = magi_as_float(args[13]);
        int has_back = (int)magi_as_int(args[14]);
        int has_upper = (int)magi_as_int(args[15]);
        int has_lower = (int)magi_as_int(args[16]);
        int light_level = (int)magi_as_int(args[17]);
        double gfx = magi_as_float(args[18]);
        double ar = magi_as_float(args[19]);
        double cfx = magi_as_float(args[20]);
        double cfy = magi_as_float(args[21]);
        int SW = 320, VH = 168;
        double x_range = (double)(scr_x2 - scr_x1);
        if (x_range <= 0) return magi_make_null();
        for (int ix = scr_x1; ix < scr_x2; ix++) {
            int x = ix;
            if (x < 0 || x >= SW) continue;
            if (magi_as_bool(hor_ocl->data[x])) continue;
            double t = (double)(ix - scr_x1) / x_range;
            double depth = cx1 + t * (cx2 - cx1);
            if (depth <= 0.1) continue;
            double bot_y_f = cfy - gfx * floor_h / depth;
            double top_y_f = cfy - gfx * ceil_h / depth;
            int bot_y = (int)bot_y_f;
            int top_y = (int)top_y_f;
            int fl_ocl = (int)magi_as_int(floor_ocl->data[x]);
            int cl_ocl = (int)magi_as_int(ceil_ocl->data[x]);
            int cb = bot_y < fl_ocl ? bot_y : fl_ocl;
            int ct = top_y > cl_ocl ? top_y : cl_ocl;
            if (cb <= ct) continue;
            // Light
            int cmap_idx = (32 - light_level / 8) + (int)(depth / 48.0);
            if (cmap_idx < 0) cmap_idx = 0;
            if (cmap_idx > 31) cmap_idx = 31;
            int color = has_back ? (has_upper ? 64 : (has_lower ? 104 : 80)) : 96;
            // Fill wall with solid color for now (texture handled by MAGI caller)
            for (int y = ct + 1; y < cb; y++) {
                if (y >= 0 && y < VH) {
                    int fi = y * SW + x;
                    if (fi >= 0 && fi < fb->len) fb->data[fi] = magi_make_int(color);
                }
            }
            // Occlude
            if (!has_back) {
                hor_ocl->data[x] = magi_make_bool(1);
                floor_ocl->data[x] = magi_make_int(VH / 2);
                ceil_ocl->data[x] = magi_make_int(VH / 2);
            } else {
                if (has_upper) {
                    int bty = (int)(cfy - gfx * back_ceil_h / depth);
                    if (bty > cl_ocl) ceil_ocl->data[x] = magi_make_int(bty);
                }
                if (has_lower) {
                    int bby = (int)(cfy - gfx * back_floor_h / depth);
                    if (bby < fl_ocl) floor_ocl->data[x] = magi_make_int(bby);
                }
            }
        }
        return magi_make_null();
    }

    // Native wall column renderer
    // Args: fb, column_data, x, y1, y2, col_h, scr_top, scr_h, tex_h, cmap
    if (strcmp(name, "__render_wall_col") == 0 && argc >= 10) {
        MagiArray* fb = magi_array_ptr(args[0]);
        MagiArray* col = magi_array_ptr(args[1]);
        if (!fb || !col) return magi_make_null();
        int x = (int)magi_as_int(args[2]);
        int y1 = (int)magi_as_int(args[3]);
        int y2 = (int)magi_as_int(args[4]);
        int col_h = (int)magi_as_int(args[5]);
        double scr_top = magi_as_float(args[6]);
        double scr_h = magi_as_float(args[7]);
        int tex_h = (int)magi_as_int(args[8]);
        MagiArray* cmap = magi_array_ptr(args[9]);
        if (col_h <= 0 || scr_h <= 0.0 || tex_h <= 0) return magi_make_null();
        int SW = 320;
        for (int y = y1; y <= y2; y++) {
            if (y < 0 || y >= 168) continue;
            double ay = ((double)y - scr_top) / scr_h;
            int ty = (int)(ay * tex_h);
            ty = ((ty % col_h) + col_h) % col_h;
            int px = 0;
            if (col->cap == -1) {
                const unsigned char* raw = (const unsigned char*)(uintptr_t)col->data;
                if (ty >= 0 && ty < col->len) px = raw[ty];
            } else if (ty >= 0 && ty < col->len) {
                px = (int)magi_as_int(col->data[ty]);
            }
            if (cmap && px >= 0 && px < 256 && px < cmap->len) {
                if (cmap->cap == -1) {
                    const unsigned char* raw = (const unsigned char*)(uintptr_t)cmap->data;
                    px = raw[px];
                } else {
                    px = (int)magi_as_int(cmap->data[px]);
                }
            }
            int fi = y * SW + x;
            if (fi >= 0 && fi < fb->len) fb->data[fi] = magi_make_int(px);
        }
        return magi_make_null();
    }

    // Native flat column renderer using Rust-style inverse perspective
    // Args: fb, flat, x, y1, y2, player_x, player_y, player_floor_h, player_angle_bam, plane_height, light_level, cmap
    // Uses float inverse perspective matching doom-rust-renderer visplanes.rs
    if (strcmp(name, "__render_flat_col") == 0 && argc >= 12) {
        MagiArray* fb = magi_array_ptr(args[0]);
        MagiArray* flat = magi_array_ptr(args[1]);
        if (!fb || !flat || flat->len < 4096) return magi_make_null();
        int x = (int)magi_as_int(args[2]);
        int y1 = (int)magi_as_int(args[3]);
        int y2 = (int)magi_as_int(args[4]);
        // Player position in map units (fixed-point >> 16)
        double player_x = (double)magi_as_int(args[5]) / 65536.0;
        double player_y = (double)magi_as_int(args[6]) / 65536.0;
        double player_floor_h = (double)magi_as_int(args[7]) / 65536.0;
        double player_angle_rad = (double)magi_as_int(args[8]) * 3.14159265358979323846 / 2147483648.0;
        double plane_height = (double)magi_as_int(args[9]) / 65536.0;
        int light_level = (int)magi_as_int(args[10]);
        MagiArray* cmap = magi_array_ptr(args[11]);

        int SW = 320, SH = 200, VH = 168;
        double ASPECT = 200.0 / 240.0;
        double GAME_FOCUS = (SW / ASPECT) / 2.0;
        double CAM_FX = SW / 2.0;
        double CAM_FY = SH / 2.0;
        double EYE_H = 41.0;
        double wz = plane_height - player_floor_h - EYE_H;

        double cos_a = cos(player_angle_rad);
        double sin_a = sin(player_angle_rad);

        double vx_base = (CAM_FX - (double)x) / ASPECT;

        for (int y = y1; y <= y2; y++) {
            if (y < 0 || y >= VH) continue;
            double vy = CAM_FY - (double)y;
            if (vy > -0.5 && vy < 0.5) continue; // skip horizon

            // Inverse perspective: screen -> viewport -> world
            double wx = GAME_FOCUS * wz / vy;
            double wy = wz * vx_base / vy;

            // Rotate back to world space
            double rx = wx * cos_a - wy * sin_a;
            double ry = wx * sin_a + wy * cos_a;

            // World position + player offset, then wrap to 64x64 flat tile
            int tx = ((int)floor(rx + player_x)) & 63;
            int ty = ((int)floor(ry + player_y)) & 63;

            int flat_idx = ty * 64 + tx;
            int px = 0;
            if (flat->cap == -1) {
                const unsigned char* raw = (const unsigned char*)(uintptr_t)flat->data;
                if (flat_idx >= 0 && flat_idx < flat->len) px = raw[flat_idx];
            } else if (flat_idx >= 0 && flat_idx < flat->len) {
                px = (int)magi_as_int(flat->data[flat_idx]);
            }

            // Light diminishing
            if (cmap && px >= 0 && px < 256 && px < cmap->len) {
                if (cmap->cap == -1) {
                    const unsigned char* raw = (const unsigned char*)(uintptr_t)cmap->data;
                    px = raw[px];
                } else {
                    px = (int)magi_as_int(cmap->data[px]);
                }
            }

            int fb_idx = y * SW + x;
            if (fb_idx >= 0 && fb_idx < fb->len) {
                fb->data[fb_idx] = magi_make_int(px);
            }
        }
        return magi_make_null();
    }

    // Unknown: return null
    return magi_make_null();
}

// embed() support — wrap raw embedded bytes as a MAGI array
// Uses a compact representation: stores the raw pointer + length,
// and the array_get function reads bytes on demand.
typedef struct { const unsigned char* data; int64_t len; } EmbedData;

// Global embed table (up to 64 embedded files)
static EmbedData __embed_table[64];
static int __embed_count = 0;

int64_t __magi_embed_array(const unsigned char* data, int64_t len) {
    if (!data || len <= 0) return magi_make_null();
    // Store in embed table — return a MagiArray backed by the raw data
    int idx = __embed_count++;
    __embed_table[idx].data = data;
    __embed_table[idx].len = len;
    // Create a MagiArray that points to a pre-built int64 array
    // But 4M * 8 bytes = 33MB is too much. Instead, use a special array
    // that lazily reads from the raw data.
    MagiArray* arr = (MagiArray*)malloc(sizeof(MagiArray));
    if (!arr) return magi_make_null();
    arr->len = (int32_t)len;
    arr->data = (int64_t*)(uintptr_t)data;
    arr->cap = -1;
    return magi_make_array_val(arr);
}

// magi_is_byte_array and magi_byte_array_get defined in forward declarations above

// Slice a byte array without copying — returns a new MagiArray pointing into the original data
int64_t __magi_byte_slice(int64_t arr_val, int64_t start_val, int64_t len_val) {
    MagiArray* src = magi_array_ptr(arr_val);
    if (!src) return magi_make_null();
    int64_t start = magi_sext48(magi_get_payload(start_val));
    int64_t slen = magi_sext48(magi_get_payload(len_val));
    if (start < 0 || slen <= 0 || start + slen > src->len) return magi_make_null();
    MagiArray* slice = (MagiArray*)malloc(sizeof(MagiArray));
    slice->len = (int32_t)slen;
    slice->cap = -1; // byte array marker
    if (magi_is_byte_array(src)) {
        const unsigned char* bytes = (const unsigned char*)(uintptr_t)src->data;
        slice->data = (int64_t*)(uintptr_t)(bytes + start);
    } else {
        // Regular array — can't slice without copy
        slice->data = src->data + start;
        slice->cap = (int32_t)slen;
    }
    return magi_make_array_val(slice);
}
