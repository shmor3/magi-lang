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
} MagiMap;

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

// ===== Print =====
void __magi_print(int64_t val) {
    char* s = magi_val_to_dyn_str(val, 1);
    printf("%s\n", s);
    fflush(stdout);
    free(s);
}

// Dynamic string builder for value formatting
static char* magi_val_to_dyn_str(int64_t val, int for_display) {
    if (!magi_is_tagged(val)) {
        double d;
        memcpy(&d, &val, sizeof(d));
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
            if (for_display) return strdup(s);
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
                if (i > 0) { buf[pos++] = ','; buf[pos++] = ' '; }
                // In display mode, array elements also use display mode (no quotes on strings)
                char* elem = magi_val_to_dyn_str(arr->data[i], for_display);
                size_t elen = strlen(elem);
                while (pos + elen + 10 > cap) { cap *= 2; buf = (char*)realloc(buf, cap); }
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
                if (i > 0) { buf[pos++] = ','; buf[pos++] = ' '; }
                char* v = magi_val_to_dyn_str(map->values[i], for_display);
                size_t klen = strlen(map->keys[i]), vlen = strlen(v);
                while (pos + klen + vlen + 10 > cap) { cap *= 2; buf = (char*)realloc(buf, cap); }
                // Keys without quotes (matching interpreter format)
                memcpy(buf+pos, map->keys[i], klen); pos += klen;
                buf[pos++] = ':'; buf[pos++] = ' ';
                memcpy(buf+pos, v, vlen); pos += vlen;
                free(v);
            }
            if (pos + 2 > cap) { cap += 4; buf = (char*)realloc(buf, cap); }
            buf[pos++] = '}'; buf[pos] = '\0';
            return buf;
        }
        default: return strdup("<unknown>");
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
    int64_t idx = magi_as_int(idx_val);
    if (!arr || idx < 0 || idx >= arr->len) return;
    if (magi_is_byte_array(arr)) return;
    arr->data[idx] = val;
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
    return magi_make_map_val(map);
}

int64_t __magi_map_get(int64_t map_val, int64_t key_val) {
    MagiMap* map = magi_map_ptr(map_val);
    const char* key = magi_as_string(key_val);
    if (!map || !key) return magi_make_null();
    for (int i = 0; i < map->len; i++) {
        if (strcmp(map->keys[i], key) == 0) return map->values[i];
    }
    return magi_make_null();
}

void __magi_map_set(int64_t map_val, int64_t key_val, int64_t val) {
    MagiMap* map = magi_map_ptr(map_val);
    const char* key = magi_as_string(key_val);
    if (!map || !key) return;
    for (int i = 0; i < map->len; i++) {
        if (strcmp(map->keys[i], key) == 0) { map->values[i] = val; return; }
    }
    if (map->len >= map->cap) {
        map->cap = map->cap < 8 ? 8 : map->cap * 2;
        map->keys = (char**)realloc(map->keys, sizeof(char*) * map->cap);
        map->values = (int64_t*)realloc(map->values, sizeof(int64_t) * map->cap);
    }
    map->keys[map->len] = strdup(key);
    map->values[map->len] = val;
    map->len++;
}

// ===== String Operations =====
int64_t __magi_string_concat(int64_t a_val, int64_t b_val) {
    const char* a = magi_as_string(a_val);
    const char* b = magi_as_string(b_val);
    size_t la = strlen(a), lb = strlen(b);
    char* result = (char*)malloc(la + lb + 1);
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
    char* s = magi_val_to_dyn_str(val, 1); // display mode (no quotes on strings)
    return magi_make_string(s); // s is already heap-allocated
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
int64_t __magi_runtime_call(const char* name, int32_t argc, int64_t* args) {
    int64_t a = argc > 0 ? args[0] : magi_make_null();
    int64_t b = argc > 1 ? args[1] : magi_make_null();

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
        return magi_make_bool(0);
    }
    if (strcmp(name, "__ne") == 0) {
        if (a == b) return magi_make_bool(0);
        if (magi_get_tag(a) == TAG_STRING && magi_get_tag(b) == TAG_STRING)
            return magi_make_bool(strcmp(magi_as_string(a), magi_as_string(b)) != 0);
        return magi_make_bool(1);
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
                    if (map->len >= map->cap) { map->cap *= 2; map->keys = (char**)realloc(map->keys, sizeof(char*) * map->cap); map->values = (int64_t*)realloc(map->values, sizeof(int64_t) * map->cap); }
                    map->keys[map->len] = key;
                    map->values[map->len] = val;
                    map->len++;
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
