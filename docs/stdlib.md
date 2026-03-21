# MAGI Standard Library Reference

Complete reference for all 39 standard library modules and their operations.

## Table of Contents

- [math](#math) -- Arithmetic, trigonometry, and numeric utilities
- [cmp](#cmp) -- Comparison operations
- [logic](#logic) -- Boolean logic
- [bits](#bits) -- Bitwise operations
- [str](#str) -- String manipulation
- [convert](#convert) -- Type conversion and type checking
- [array](#array) -- Array manipulation
- [map](#map) -- Map (dictionary) operations
- [bytes](#bytes) -- Binary data operations
- [json](#json) -- JSON manipulation
- [time](#time) -- Timestamps, durations, and time utilities
- [hash](#hash) -- Hashing and encoding
- [io](#io) -- Debug logging and assertions
- [control](#control) -- Control flow primitives
- [rand](#rand) -- Random number generation
- [fs](#fs) -- Filesystem operations
- [env](#env) -- Environment variables and OS info
- [net](#net) -- HTTP client and URL utilities
- [tcp](#tcp) -- TCP socket operations
- [udp](#udp) -- UDP socket operations
- [ws](#ws) -- WebSocket client
- [sse](#sse) -- Server-Sent Events client
- [http_server](#http_server) -- HTTP server
- [cert](#cert) -- TLS certificate operations
- [path](#path) -- File path manipulation
- [yaml](#yaml) -- YAML parsing and serialization
- [csv](#csv) -- CSV parsing and serialization
- [toml](#toml) -- TOML parsing and serialization
- [regex](#regex) -- Regular expression utilities
- [uuid](#uuid) -- UUID generation and parsing
- [crypto](#crypto) -- Cryptographic hashing
- [compress](#compress) -- Compression and decompression
- [fmt](#fmt) -- Value formatting
- [stats](#stats) -- Statistical functions
- [text](#text) -- Text transformation utilities
- [encode](#encode) -- HTML and Base32 encoding
- [reflect](#reflect) -- Runtime type reflection
- [collections](#collections) -- Sets, counters, and ordered maps
- [sort](#sort) -- Sorting algorithms

---

## Importing Modules

```magi
// Import all operations from a module into scope
use std::math::*

// Import a specific operation
use std::math::sqrt

// Import module as a namespace
use std::math
let result = math.sqrt(16.0)

// Import with alias
use std::math as m
let result = m.sqrt(16.0)
```

---

## math

Arithmetic, trigonometry, logarithms, and numeric utilities.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `add` | `(a: any, b: any) -> number` | Adds two values. |
| `subtract` | `(a: any, b: any) -> number` | Subtracts b from a. |
| `multiply` | `(a: any, b: any) -> number` | Multiplies two values. |
| `divide` | `(a: any, b: any) -> number` | Divides a by b. |
| `modulo` | `(a: any, b: any) -> number` | Returns the remainder of a divided by b. |
| `power` | `(a: any, b: any) -> number` | Raises a to the power of b. |
| `sqrt` | `(value: any) -> float64` | Returns the square root of a number. |
| `abs` | `(value: any) -> number` | Returns the absolute value of a number. |
| `negate` | `(value: any) -> number` | Negates a numeric value. |
| `min` | `(a: any, b: any) -> number` | Returns the smaller of two values. |
| `max` | `(a: any, b: any) -> number` | Returns the larger of two values. |
| `round` | `(value: any) -> number` | Rounds a number to the nearest integer. |
| `floor` | `(value: any) -> number` | Rounds a number down to the nearest integer. |
| `ceil` | `(value: any) -> number` | Rounds a number up to the nearest integer. |
| `sin` | `(value: any) -> float64` | Returns the sine of an angle in radians. |
| `cos` | `(value: any) -> float64` | Returns the cosine of an angle in radians. |
| `tan` | `(value: any) -> float64` | Returns the tangent of an angle in radians. |
| `asin` | `(value: any) -> float64` | Returns the arcsine (inverse sine) in radians. |
| `acos` | `(value: any) -> float64` | Returns the arccosine (inverse cosine) in radians. |
| `atan` | `(value: any) -> float64` | Returns the arctangent (inverse tangent) in radians. |
| `atan2` | `(a: any, b: any) -> float64` | Returns the arctangent of y/x, using signs to determine the quadrant. |
| `sinh` | `(value: any) -> float64` | Returns the hyperbolic sine. |
| `cosh` | `(value: any) -> float64` | Returns the hyperbolic cosine. |
| `tanh` | `(value: any) -> float64` | Returns the hyperbolic tangent. |
| `log` | `(value: any, base: any) -> float64` | Returns the logarithm of a value with the given base. |
| `ln` | `(value: any) -> float64` | Returns the natural logarithm. |
| `log2` | `(value: any) -> float64` | Returns the base-2 logarithm. |
| `log10` | `(value: any) -> float64` | Returns the base-10 logarithm. |
| `exp` | `(value: any) -> float64` | Computes e^x (exponential function). |
| `to_radians` | `(value: any) -> float64` | Converts degrees to radians. |
| `to_degrees` | `(value: any) -> float64` | Converts radians to degrees. |
| `clamp` | `(value: any, min: any, max: any) -> any` | Clamps a value between a minimum and maximum. |
| `lerp` | `(a: any, b: any, t: any) -> float64` | Linearly interpolates between a and b by factor t. |
| `remap` | `(value: any, in_min: any, in_max: any, out_min: any, out_max: any) -> float64` | Remaps a value from one range to another. |
| `sign` | `(value: any) -> any` | Returns the sign of a number (-1, 0, or 1). |
| `gcd` | `(a: any, b: any) -> int64` | Returns the greatest common divisor of two integers. |
| `lcm` | `(a: any, b: any) -> int64` | Returns the least common multiple of two integers. |
| `is_nan` | `(value: any) -> bool` | Returns true if the value is NaN. |
| `is_infinite` | `(value: any) -> bool` | Returns true if the value is infinite. |
| `is_finite` | `(value: any) -> bool` | Returns true if the value is finite (not NaN or Infinity). |
| `approx_eq` | `(a: any, b: any, epsilon: any) -> bool` | Returns true if two values are approximately equal within epsilon. |
| `math_sum` | `(array: array) -> any` | Returns the sum of all numeric elements in an array. |
| `math_product` | `(array: array) -> any` | Returns the product of all numeric elements in an array. |
| `math_average` | `(array: array) -> float64` | Returns the arithmetic mean of an array of numbers. |
| `math_min_of` | `(array: array) -> any` | Returns the minimum value in an array. |
| `math_max_of` | `(array: array) -> any` | Returns the maximum value in an array. |
| `math_count` | `(array: array) -> int64` | Returns the number of elements in an array. |

```magi
use std::math::*

let x = sqrt(16.0)       // 4.0
let y = abs(-42)          // 42
let c = clamp(15, 0, 10)  // 10
let r = lerp(0.0, 100.0, 0.5)  // 50.0
let g = gcd(12, 8)        // 4
let avg = math_average([1, 2, 3, 4, 5])  // 3.0
```

---

## cmp

Comparison operations that return boolean results.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `equal` | `(a: any, b: any) -> bool` | Returns true if a equals b. |
| `not_equal` | `(a: any, b: any) -> bool` | Returns true if a does not equal b. |
| `greater` | `(a: any, b: any) -> bool` | Returns true if a is greater than b. |
| `less` | `(a: any, b: any) -> bool` | Returns true if a is less than b. |
| `greater_eq` | `(a: any, b: any) -> bool` | Returns true if a is greater than or equal to b. |
| `less_eq` | `(a: any, b: any) -> bool` | Returns true if a is less than or equal to b. |

```magi
use std::cmp::*

let a = equal(1, 1)       // true
let b = greater(5, 3)     // true
let c = less_eq(2, 2)     // true
```

---

## logic

Boolean logic operations.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `and` | `(a: bool, b: bool) -> bool` | Logical AND of two booleans. |
| `or` | `(a: bool, b: bool) -> bool` | Logical OR of two booleans. |
| `not` | `(value: bool) -> bool` | Logical NOT of a boolean. |
| `xor` | `(a: bool, b: bool) -> bool` | Logical exclusive OR of two booleans. |

```magi
use std::logic::*

let a = and(true, false)   // false
let b = or(true, false)    // true
let c = not(true)          // false
let d = xor(true, false)   // true
```

---

## bits

Bitwise operations on integer values.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `bit_and` | `(a: any, b: any) -> any` | Bitwise AND of two integers. |
| `bit_or` | `(a: any, b: any) -> any` | Bitwise OR of two integers. |
| `bit_xor` | `(a: any, b: any) -> any` | Bitwise XOR of two integers. |
| `bit_not` | `(value: any) -> any` | Bitwise NOT (complement) of an integer. |
| `bit_shift_left` | `(a: any, b: any) -> any` | Shifts bits of a left by b positions. |
| `bit_shift_right` | `(a: any, b: any) -> any` | Shifts bits of a right by b positions. |

```magi
use std::bits::*

let a = bit_and(0xFF, 0x0F)       // 15
let b = bit_or(0x0F, 0xF0)        // 255
let c = bit_shift_left(1, 4)      // 16
let d = bit_shift_right(16, 2)    // 4
```

---

## str

String manipulation and pattern matching.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `concat` | `(a: string, b: string) -> string` | Concatenates two strings. |
| `split` | `(input: string, delimiter: string) -> array` | Splits a string by a delimiter. Returns an array. |
| `substring` | `(input: string) -> string` | Returns a substring by start and end index. |
| `length` | `(input: string) -> int64` | Returns the length of a string. |
| `replace` | `(input: string, search: string, replace: string) -> string` | Replaces all occurrences of a substring. |
| `to_upper` | `(input: string) -> string` | Converts the string to uppercase. |
| `to_lower` | `(input: string) -> string` | Converts the string to lowercase. |
| `trim` | `(input: string) -> string` | Removes leading and trailing whitespace. |
| `trim_start` | `(input: string) -> string` | Removes leading whitespace. |
| `trim_end` | `(input: string) -> string` | Removes trailing whitespace. |
| `contains` | `(input: string, search: string) -> bool` | Returns true if the string contains the search substring. |
| `starts_with` | `(input: string, prefix: string) -> bool` | Returns true if the string starts with the given prefix. |
| `ends_with` | `(input: string, suffix: string) -> bool` | Returns true if the string ends with the given suffix. |
| `char_at` | `(input: string) -> string` | Returns the character at the given index, or null. |
| `index_of` | `(input: string, search: string) -> int64` | Returns the index of the first occurrence, or null if not found. |
| `pad_start` | `(input: string) -> string` | Pads the start of the string to a given width. |
| `pad_end` | `(input: string) -> string` | Pads the end of the string to a given width. |
| `string_repeat` | `(input: string) -> string` | Repeats the string n times. |
| `string_reverse` | `(input: string) -> string` | Returns the reversed string. |
| `string_lines` | `(input: string) -> array` | Splits the string into lines. Returns an array. |
| `string_words` | `(input: string) -> array` | Splits the string into words. Returns an array. |
| `string_count` | `(input: string, search: string) -> int64` | Counts occurrences of a substring. |
| `string_chars` | `(input: string) -> array` | Returns an array of individual characters. |
| `string_join` | `(array: array) -> string` | Joins array elements into a string with a separator. |
| `string_template` | `(template: string, values: array) -> string` | Applies positional template substitution. |
| `string_format` | `(template: string, values: map) -> string` | Formats a string with named placeholders from a map. |
| `regex_match` | `(input: string, pattern: string) -> bool` | Returns true if the string matches the regex pattern. |
| `regex_replace` | `(input: string, replacement: string, pattern: string) -> string` | Replaces regex matches with a replacement string. |
| `regex_extract` | `(input: string, pattern: string) -> array` | Extracts all regex capture groups from the string. |

```magi
use std::str::*

let s = "Hello, World!"
let upper = to_upper(s)                  // "HELLO, WORLD!"
let parts = split("a,b,c", ",")          // ["a", "b", "c"]
let has = contains(s, "World")           // true
let joined = string_join(["a", "b"], "-") // "a-b"
let lines = string_lines("line1\nline2") // ["line1", "line2"]
let reversed = string_reverse("abc")     // "cba"
```

---

## convert

Type conversion, type parsing, and type checking utilities.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `to_string` | `(input: any) -> string` | Converts a value to its string representation. |
| `to_int64` | `(input: any) -> int64` | Converts a value to a 64-bit integer. |
| `to_float64` | `(input: any) -> float64` | Converts a value to a 64-bit float. |
| `to_bool` | `(input: any) -> bool` | Converts a value to a boolean. |
| `to_bytes` | `(input: any) -> bytes` | Converts a value to bytes. |
| `from_bytes` | `(input: bytes) -> string` | Converts bytes to a UTF-8 string. |
| `parse_json` | `(input: string) -> any` | Parses a JSON string into a value. |
| `to_json` | `(input: any) -> string` | Converts a value to a JSON string. |
| `parse_int` | `(input: string) -> int64` | Parses a string as an integer. |
| `parse_float` | `(input: string) -> float64` | Parses a string as a float. |
| `typeof` | `(input: any) -> string` | Returns the type name of a value as a string. |
| `default` | `(input: any, fallback: any) -> any` | Returns input if non-null, otherwise returns fallback. |
| `is_null` | `(input: any) -> bool` | Returns true if the value is null. |
| `is_string` | `(input: any) -> bool` | Returns true if the value is a string. |
| `is_number` | `(input: any) -> bool` | Returns true if the value is a number (int or float). |
| `is_array` | `(input: any) -> bool` | Returns true if the value is an array. |
| `is_map` | `(input: any) -> bool` | Returns true if the value is a map. |
| `is_bool` | `(input: any) -> bool` | Returns true if the value is a boolean. |
| `is_bytes` | `(input: any) -> bool` | Returns true if the value is a bytes value. |

```magi
use std::convert::*

let s = to_string(42)          // "42"
let n = to_int64("123")        // 123
let f = to_float64("3.14")     // 3.14
let t = typeof([1, 2, 3])      // "array"
let d = default(null, "fallback")  // "fallback"
let check = is_string("hello") // true
```

---

## array

Array creation, access, and manipulation.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `array_get` | `(array: array, index: any) -> any` | Returns the element at the given index. |
| `array_set` | `(array: array, index: any, value: any) -> array` | Sets the element at an index. Returns the updated array. |
| `array_push` | `(array: array, value: any) -> array` | Appends an element to the end of the array. |
| `array_pop` | `(array: array) -> any` | Removes and returns the last element. |
| `array_shift` | `(array: array) -> any` | Removes and returns the first element. |
| `array_length` | `(array: array) -> int64` | Returns the number of elements in the array. |
| `array_slice` | `(array: array) -> array` | Returns a sub-array by index range. |
| `array_concat` | `(a: array, b: array) -> array` | Concatenates two arrays. |
| `array_contains` | `(array: array, value: any) -> bool` | Returns true if the array contains the value. |
| `array_sort` | `(array: array) -> array` | Returns a sorted copy of the array. |
| `array_reverse` | `(array: array) -> array` | Returns a reversed copy of the array. |
| `array_flatten` | `(array: array) -> array` | Flattens nested arrays one level deep. |
| `array_filter_nulls` | `(array: array) -> array` | Removes null elements from the array. |
| `array_join` | `(array: array) -> string` | Joins array elements into a string with a separator. |
| `array_unique` | `(array: array) -> array` | Returns the array with duplicate elements removed. |
| `array_insert` | `(array: array, index: any, value: any) -> array` | Inserts an element at the given index. |
| `array_remove` | `(array: array, index: any) -> array` | Removes the element at the given index. |
| `array_from_map` | `(map: map) -> array` | Converts a map to an array of [key, value] pairs. |
| `reduce` | `(array: array, initial: any) -> any` | Reduces the array to a single value using an accumulator function. |
| `range` | `(start: any, end: any) -> array` | Creates an array of integers from start (inclusive) to end (exclusive). |

```magi
use std::array::*

let arr = [1, 2, 3, 4, 5]
let pushed = array_push(arr, 6)        // [1, 2, 3, 4, 5, 6]
let elem = array_get(arr, 2)           // 3
let has = array_contains(arr, 3)       // true
let sorted = array_sort([3, 1, 2])     // [1, 2, 3]
let flat = array_flatten([[1, 2], [3]]) // [1, 2, 3]
let nums = range(1, 6)                 // [1, 2, 3, 4, 5]
```

---

## map

Map (dictionary) operations for key-value data.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `map_get` | `(map: map, key: string) -> any` | Returns the value for the given key, or null. |
| `map_set` | `(map: map, key: string, value: any) -> map` | Sets a key-value pair. Returns the updated map. |
| `map_delete` | `(map: map, key: string) -> map` | Removes a key from the map. Returns the updated map. |
| `map_has` | `(map: map, key: string) -> bool` | Returns true if the map contains the given key. |
| `map_keys` | `(map: map) -> array` | Returns an array of all keys in the map. |
| `map_values` | `(map: map) -> array` | Returns an array of all values in the map. |
| `map_entries` | `(map: map) -> array` | Returns an array of [key, value] pairs. |
| `map_merge` | `(a: map, b: map) -> map` | Merges two maps. Keys in b override keys in a. |
| `map_size` | `(map: map) -> int64` | Returns the number of entries in the map. |
| `map_from_entries` | `(array: array) -> map` | Creates a map from an array of [key, value] pairs. |
| `map_update` | `(map: map, key: string, value: any) -> map` | Updates a key-value pair. Returns the updated map. |

```magi
use std::map::*

let m = {"name": "Alice", "age": 30}
let name = map_get(m, "name")          // "Alice"
let updated = map_set(m, "city", "NYC") // {"name": "Alice", "age": 30, "city": "NYC"}
let has = map_has(m, "age")            // true
let keys = map_keys(m)                 // ["name", "age"]
let merged = map_merge(m, {"role": "dev"})
```

---

## bytes

Binary data manipulation and Base64 encoding.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `bytes_length` | `(input: bytes) -> int64` | Returns the length of the byte sequence. |
| `bytes_slice` | `(input: bytes) -> bytes` | Returns a slice of the byte sequence. |
| `bytes_concat` | `(a: bytes, b: bytes) -> bytes` | Concatenates two byte sequences. |
| `bytes_contains` | `(input: bytes, search: bytes) -> bool` | Returns true if the byte sequence contains the search bytes. |
| `base64_encode` | `(input: bytes) -> string` | Encodes bytes as a Base64 string. |
| `base64_decode` | `(input: string) -> bytes` | Decodes a Base64 string to bytes. |

```magi
use std::bytes::*

let data = to_bytes("hello")
let len = bytes_length(data)
let encoded = base64_encode(data)      // "aGVsbG8="
let decoded = base64_decode(encoded)
```

---

## json

JSON parsing, manipulation, and querying.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `json_get` | `(value: any, path: string) -> any` | Gets a value at a JSON path. |
| `json_set` | `(value: any, path: string, item: any) -> any` | Sets a value at a JSON path. |
| `json_delete` | `(value: any, path: string) -> any` | Deletes a value at a JSON path. |
| `json_flatten` | `(input: any) -> map` | Flattens nested JSON into a flat map with dot-notation keys. |
| `json_merge` | `(a: any, b: any) -> any` | Deep-merges two JSON values. |
| `json_type` | `(input: any) -> string` | Returns the JSON type of a value (e.g., "object", "array", "string"). |
| `json_validate` | `(input: any) -> bool` | Returns true if the input is valid JSON. |
| `json_pretty_print` | `(input: any) -> string` | Formats JSON with indentation for readability. |
| `json_compact` | `(input: any) -> string` | Formats JSON as a compact single-line string. |
| `json_query` | `(value: any, path: string) -> any` | Queries a JSON value using a path expression. |

```magi
use std::json::*

let data = {"user": {"name": "Alice", "age": 30}}
let name = json_get(data, "user.name")     // "Alice"
let updated = json_set(data, "user.city", "NYC")
let pretty = json_pretty_print(data)
let valid = json_validate("{\"key\": 1}")   // true
```

---

## time

Timestamps, durations, and time arithmetic.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `now_timestamp` | `() -> int64` | Returns the current Unix timestamp in milliseconds. |
| `format_timestamp` | `(input: any) -> string` | Formats a timestamp as a human-readable string. |
| `parse_timestamp` | `(input: any) -> int64` | Parses a date/time string into a Unix timestamp. |
| `timestamp_add` | `(input: any, amount: any) -> int64` | Adds milliseconds to a timestamp. |
| `timestamp_diff` | `(a: any, b: any) -> int64` | Returns the difference between two timestamps in milliseconds. |
| `sleep` | `(duration: any) -> any` | Pauses execution for the given duration in milliseconds. |
| `duration` | `() -> int64` | Creates a duration value (milliseconds). |
| `elapsed` | `(timestamp: any) -> int64` | Returns milliseconds elapsed since the given timestamp. |
| `time_sleep` | `(duration: any) -> any` | Pauses execution for the given duration in milliseconds. |
| `add_duration` | `(timestamp: any, duration: any) -> int64` | Adds a duration to a timestamp. |
| `sub_duration` | `(timestamp: any, duration: any) -> int64` | Subtracts a duration from a timestamp. |
| `time_diff` | `(a: any, b: any) -> int64` | Returns the difference between two timestamps. |
| `start_of` | `(input: any) -> int64` | Returns the start of a time period (day, hour, etc.). |
| `end_of` | `(input: any) -> int64` | Returns the end of a time period (day, hour, etc.). |

```magi
use std::time::*

let ts = now_timestamp()
let formatted = format_timestamp(ts)
let future = timestamp_add(ts, 3600000)  // 1 hour later
let diff = timestamp_diff(future, ts)    // 3600000
```

---

## hash

Cryptographic hashing and URL/hex encoding.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `hash_sha256` | `(input: string) -> string` | Returns the SHA-256 hash of the input as a hex string. |
| `hash_blake3` | `(input: string) -> string` | Returns the BLAKE3 hash of the input as a hex string. |
| `hash_md5` | `(input: string) -> string` | Returns the MD5 hash of the input as a hex string. |
| `url_encode` | `(input: string) -> string` | URL-encodes a string (percent-encoding). |
| `url_decode` | `(input: string) -> string` | Decodes a URL-encoded string. |
| `hex_encode` | `(input: bytes) -> string` | Encodes bytes as a hexadecimal string. |
| `hex_decode` | `(input: string) -> bytes` | Decodes a hexadecimal string to bytes. |
| `hash_sha512` | `(input: string) -> string` | Returns the SHA-512 hash of the input as a hex string. |
| `hmac_sha256` | `(input: string, key: string) -> string` | Computes an HMAC-SHA256 of input with the given key. |
| `hash_crc32` | `(input: string) -> int64` | Returns the CRC-32 checksum of the input. |
| `constant_time_eq` | `(a: any, b: any) -> bool` | Compares two values in constant time (timing-attack safe). |

```magi
use std::hash::*

let h = hash_sha256("hello")
let encoded = url_encode("hello world")   // "hello%20world"
let decoded = url_decode("hello%20world") // "hello world"
let mac = hmac_sha256("message", "secret-key")
```

---

## io

Debug logging and runtime assertions.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `debug_log` | `(input: any) -> any` | Logs a debug message and returns the input. |
| `assert` | `(condition: bool, message: string) -> bool` | Asserts a condition is true; throws on failure with the given message. |
| `error` | `(message: string) -> any` | Throws a runtime error with the given message. |

```magi
use std::io::*

debug_log("checkpoint reached")
assert(x > 0, "x must be positive")
```

---

## control

Control flow primitives.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `if_else` | `(condition: bool, then: any, else: any) -> any` | Returns `then` if condition is true, otherwise `else`. |
| `switch` | `(value: any, default: any) -> any` | Multi-way branch based on value matching. |
| `coalesce` | `(a: any, b: any) -> any` | Returns a if non-null, otherwise b (null coalescing). |
| `try_catch` | `(input: any, fallback: any) -> any` | Evaluates input; returns fallback if an error occurs. |
| `error` | `(message: string) -> any` | Throws a runtime error with the given message. |

```magi
use std::control::*

let result = coalesce(null, "default")    // "default"
let safe = try_catch(risky_fn(), "error fallback")
```

---

## rand

Random number generation and sampling.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `random_int` | `() -> int64` | Returns a random 64-bit integer. |
| `random_float` | `() -> float64` | Returns a random float between 0.0 and 1.0. |
| `random_bool` | `() -> bool` | Returns a random boolean. |
| `random_bytes` | `() -> bytes` | Returns random bytes. |
| `random_range` | `(a: any, b: any) -> any` | Returns a random number in the range [a, b). |
| `random_choice` | `(array: array) -> any` | Returns a random element from the array. |
| `random_shuffle` | `(array: array) -> array` | Returns a randomly shuffled copy of the array. |
| `random_sample` | `(array: array) -> array` | Returns a random sample from the array. |
| `random_uuid` | `() -> string` | Returns a random UUID v4 string. |
| `random_string` | `() -> string` | Returns a random alphanumeric string. |

```magi
use std::rand::*

let n = random_range(1, 100)
let coin = random_bool()
let pick = random_choice(["red", "green", "blue"])
let id = random_uuid()
let shuffled = random_shuffle([1, 2, 3, 4, 5])
```

---

## fs

Filesystem operations for reading, writing, and managing files and directories.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `fs_read` | `(path: string) -> string` | Reads the contents of a file as a UTF-8 string. |
| `fs_write` | `(path: string, content: any) -> bool` | Writes content to a file. Returns true on success. |
| `fs_append` | `(path: string, content: any) -> bool` | Appends content to a file. Returns true on success. |
| `fs_exists` | `(path: string) -> bool` | Returns true if the path exists. |
| `fs_remove` | `(path: string) -> bool` | Removes a file or directory. Returns true on success. |
| `fs_list` | `(path: string) -> array` | Lists entries in a directory. Returns an array of names. |
| `fs_mkdir` | `(path: string) -> bool` | Creates a directory (and parents). Returns true on success. |
| `fs_copy` | `(source: string, destination: string) -> int64` | Copies a file. Returns the number of bytes copied. |
| `fs_move` | `(source: string, destination: string) -> bool` | Moves/renames a file. Returns true on success. |
| `fs_size` | `(path: string) -> int64` | Returns the file size in bytes. |
| `fs_is_file` | `(path: string) -> bool` | Returns true if the path is a regular file. |
| `fs_is_dir` | `(path: string) -> bool` | Returns true if the path is a directory. |

```magi
use std::fs::*

let content = fs_read("config.toml")
fs_write("output.txt", "hello world")
let exists = fs_exists("output.txt")         // true
let files = fs_list("./src")
let size = fs_size("output.txt")
fs_mkdir("./new_dir")
```

---

## env

Environment variables and operating system information.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `env_get` | `(key: string) -> any` | Returns the value of an environment variable, or null. |
| `env_has` | `(key: string) -> bool` | Returns true if the environment variable exists. |
| `env_keys` | `() -> array` | Returns an array of all environment variable names. |
| `os_name` | `() -> string` | Returns the operating system name (e.g., "linux", "macos"). |
| `os_arch` | `() -> string` | Returns the CPU architecture (e.g., "x86_64", "aarch64"). |
| `process_pid` | `() -> int64` | Returns the current process ID. |
| `current_dir` | `() -> string` | Returns the current working directory. |

```magi
use std::env::*

let home = env_get("HOME")
let has_path = env_has("PATH")    // true
let os = os_name()                // "linux"
let arch = os_arch()              // "x86_64"
let pid = process_pid()
let cwd = current_dir()
```

---

## net

HTTP client operations and URL utilities.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `http_get` | `(url: string) -> string` | Sends an HTTP GET request. Returns the response body. |
| `http_post` | `(url: string, body: any) -> string` | Sends an HTTP POST request. Returns the response body. |
| `http_put` | `(url: string, body: any) -> string` | Sends an HTTP PUT request. Returns the response body. |
| `http_delete` | `(url: string) -> string` | Sends an HTTP DELETE request. Returns the response body. |
| `http_request` | `(method: string, url: string, body: any, headers: map) -> map` | Sends a custom HTTP request. Returns a map with status, headers, and body. |
| `http_head` | `(url: string) -> map` | Sends an HTTP HEAD request. Returns response headers as a map. |
| `http_options` | `(url: string) -> map` | Sends an HTTP OPTIONS request. Returns response headers as a map. |
| `http_patch` | `(url: string, body: any) -> string` | Sends an HTTP PATCH request. Returns the response body. |
| `url_parse` | `(input: string) -> map` | Parses a URL into its components (scheme, host, path, etc.). |
| `url_join` | `(base: string, path: string) -> string` | Joins a base URL with a relative path. |

```magi
use std::net::*

let body = http_get("https://api.example.com/data")
let resp = http_post("https://api.example.com/submit", {"key": "value"})
let parts = url_parse("https://example.com:8080/path?q=1")
let full = url_join("https://example.com", "/api/v1")
```

---

## tcp

TCP socket operations for low-level networking.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `tcp_connect` | `(host: string, port: any) -> string` | Connects to a TCP server. Returns a connection ID. |
| `tcp_write` | `(conn_id: string, data: any) -> int64` | Writes data to a TCP connection. Returns bytes written. |
| `tcp_read` | `(conn_id: string) -> bytes` | Reads data from a TCP connection. |
| `tcp_close` | `(conn_id: string) -> any` | Closes a TCP connection. |
| `tcp_bind` | `(address: string, port: any) -> string` | Binds a TCP listener to an address. Returns a listener ID. |
| `tcp_accept` | `(listener_id: string) -> map` | Accepts an incoming TCP connection. Returns connection info. |
| `tcp_server_close` | `(listener_id: string) -> any` | Closes a TCP listener. |

```magi
use std::tcp::*

let conn = tcp_connect("localhost", 8080)
tcp_write(conn, "GET / HTTP/1.1\r\n\r\n")
let response = tcp_read(conn)
tcp_close(conn)
```

---

## udp

UDP socket operations.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `udp_bind` | `(address: string, port: any) -> string` | Binds a UDP socket. Returns a socket ID. |
| `udp_send_to` | `(socket_id: string, data: any, address: string, port: any) -> int64` | Sends data to a UDP address. Returns bytes sent. |
| `udp_recv_from` | `(socket_id: string) -> map` | Receives data from the UDP socket. Returns a map with data and sender info. |
| `udp_close` | `(socket_id: string) -> any` | Closes a UDP socket. |

```magi
use std::udp::*

let sock = udp_bind("0.0.0.0", 9000)
udp_send_to(sock, "hello", "127.0.0.1", 9001)
let msg = udp_recv_from(sock)
udp_close(sock)
```

---

## ws

WebSocket client operations.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `ws_connect` | `(url: string) -> string` | Connects to a WebSocket server. Returns a connection ID. |
| `ws_send` | `(conn_id: string, message: any) -> any` | Sends a message over the WebSocket connection. |
| `ws_receive` | `(conn_id: string) -> string` | Receives a message from the WebSocket connection. |
| `ws_close` | `(conn_id: string) -> any` | Closes the WebSocket connection. |

```magi
use std::ws::*

let conn = ws_connect("wss://echo.websocket.org")
ws_send(conn, "hello")
let reply = ws_receive(conn)
ws_close(conn)
```

---

## sse

Server-Sent Events (SSE) client.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `sse_connect` | `(url: string) -> string` | Connects to an SSE endpoint. Returns a connection ID. |
| `sse_read_event` | `(conn_id: string) -> map` | Reads the next event. Returns a map with event type and data. |
| `sse_close` | `(conn_id: string) -> any` | Closes the SSE connection. |

```magi
use std::sse::*

let conn = sse_connect("https://api.example.com/events")
let event = sse_read_event(conn)
sse_close(conn)
```

---

## http_server

HTTP server for handling incoming requests.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `http_server_start` | `(address: string, port: any) -> string` | Starts an HTTP server. Returns a server ID. |
| `http_server_receive` | `(server_id: string) -> map` | Waits for and receives an incoming HTTP request. Returns request details. |
| `http_server_respond` | `(client_id: string, status: any, body: any) -> any` | Sends an HTTP response to a client. |
| `http_server_stop` | `(server_id: string) -> any` | Stops the HTTP server. |

```magi
use std::http_server::*

let server = http_server_start("0.0.0.0", 3000)
let req = http_server_receive(server)
http_server_respond(req.client_id, 200, "OK")
http_server_stop(server)
```

---

## cert

TLS certificate generation, parsing, and verification.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `cert_generate` | `(cn: string) -> map` | Generates a TLS certificate for the given common name. Returns a map with cert and key. |
| `cert_parse` | `(pem: string) -> map` | Parses a PEM-encoded certificate. Returns certificate details. |
| `cert_info` | `(pem: string) -> map` | Returns detailed information about a PEM-encoded certificate. |
| `cert_verify` | `(pem: string) -> map` | Verifies a PEM-encoded certificate. Returns verification result. |
| `key_generate` | `() -> map` | Generates a new private key. Returns key details. |
| `cert_self_signed` | `(cn: string) -> map` | Generates a self-signed certificate. Returns cert and key. |

```magi
use std::cert::*

let pair = cert_self_signed("localhost")
let info = cert_info(pair.cert)
let valid = cert_verify(pair.cert)
```

---

## path

File path manipulation utilities.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `path_join` | `(a: string, b: string) -> string` | Joins two path components. |
| `path_basename` | `(input: string) -> string` | Returns the file name component of a path. |
| `path_dirname` | `(input: string) -> string` | Returns the directory component of a path. |
| `path_extension` | `(input: string) -> string` | Returns the file extension (e.g., "txt"). |
| `path_stem` | `(input: string) -> string` | Returns the file name without extension. |
| `path_is_absolute` | `(input: string) -> bool` | Returns true if the path is absolute. |
| `path_normalize` | `(input: string) -> string` | Normalizes a path (resolves `.` and `..`). |
| `path_split` | `(input: string) -> array` | Splits a path into its components. |
| `path_with_extension` | `(input: string, extension: string) -> string` | Replaces or adds a file extension. |
| `path_parent` | `(input: string) -> string` | Returns the parent directory of a path. |

```magi
use std::path::*

let full = path_join("/home/user", "docs/file.txt")
let name = path_basename("/home/user/file.txt")   // "file.txt"
let dir = path_dirname("/home/user/file.txt")      // "/home/user"
let ext = path_extension("file.tar.gz")            // "gz"
let stem = path_stem("file.txt")                   // "file"
let parts = path_split("/home/user/file.txt")      // ["/", "home", "user", "file.txt"]
```

---

## yaml

YAML parsing and serialization.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `yaml_parse` | `(input: any) -> any` | Parses a YAML string into a value. |
| `yaml_stringify` | `(input: any) -> string` | Serializes a value to a YAML string. |
| `yaml_validate` | `(input: any) -> bool` | Returns true if the input is valid YAML. |
| `yaml_to_json` | `(input: any) -> string` | Converts a YAML string to a JSON string. |
| `yaml_from_json` | `(input: any) -> string` | Converts a JSON string to a YAML string. |
| `yaml_merge` | `(a: string, b: string) -> string` | Deep-merges two YAML documents. |

```magi
use std::yaml::*

let data = yaml_parse("name: Alice\nage: 30")
let yml = yaml_stringify({"key": "value"})
let json = yaml_to_json("items:\n  - one\n  - two")
let valid = yaml_validate("key: value")  // true
```

---

## csv

CSV parsing and serialization.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `csv_parse` | `(input: any) -> array` | Parses a CSV string into an array of maps (using headers as keys). |
| `csv_stringify` | `(input: any) -> string` | Serializes an array of maps to a CSV string. |
| `csv_headers` | `(input: any) -> array` | Extracts the header row from a CSV string. |
| `csv_parse_rows` | `(input: any) -> array` | Parses a CSV string into an array of arrays (raw rows). |

```magi
use std::csv::*

let data = csv_parse("name,age\nAlice,30\nBob,25")
// [{"name": "Alice", "age": "30"}, {"name": "Bob", "age": "25"}]

let headers = csv_headers("name,age\nAlice,30")  // ["name", "age"]
let csv_str = csv_stringify([{"a": "1", "b": "2"}])
```

---

## toml

TOML parsing and serialization.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `toml_parse` | `(input: any) -> any` | Parses a TOML string into a value. |
| `toml_stringify` | `(input: any) -> string` | Serializes a value to a TOML string. |

```magi
use std::toml::*

let config = toml_parse("[server]\nport = 8080")
let toml_str = toml_stringify({"server": {"port": 8080}})
```

---

## regex

Regular expression utilities (beyond the basic regex ops in `str`).

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `regex_split` | `(input: string, pattern: string) -> array` | Splits a string by a regex pattern. |
| `regex_escape` | `(input: string) -> string` | Escapes special regex characters in a string. |
| `regex_test` | `(input: string, pattern: string) -> bool` | Returns true if the string matches the regex pattern. |
| `regex_captures` | `(input: string, pattern: string) -> array` | Returns all capture groups from a regex match. |
| `regex_find_all` | `(input: string, pattern: string) -> array` | Returns all non-overlapping matches of the pattern. |

```magi
use std::regex::*

let parts = regex_split("one1two2three", "\\d+")  // ["one", "two", "three"]
let test = regex_test("hello123", "\\d+")          // true
let matches = regex_find_all("a1b2c3", "\\d")      // ["1", "2", "3"]
let escaped = regex_escape("hello.world")          // "hello\\.world"
let caps = regex_captures("2024-03-20", "(\\d{4})-(\\d{2})-(\\d{2})")
```

---

## uuid

UUID generation and validation.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `uuid_v4` | `() -> string` | Generates a random UUID v4 string. |
| `uuid_parse` | `(input: string) -> map` | Parses a UUID string into its components (version, variant, etc.). |
| `uuid_is_valid` | `(input: string) -> bool` | Returns true if the string is a valid UUID. |
| `uuid_nil` | `() -> string` | Returns the nil UUID (all zeros). |

```magi
use std::uuid::*

let id = uuid_v4()                              // "550e8400-e29b-41d4-..."
let valid = uuid_is_valid(id)                   // true
let nil = uuid_nil()                            // "00000000-0000-0000-0000-000000000000"
let parts = uuid_parse(id)                      // {"version": 4, ...}
```

---

## crypto

Extended cryptographic hashing operations.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `hash_sha512` | `(input: string) -> string` | Returns the SHA-512 hash as a hex string. |
| `hmac_sha256` | `(input: string, key: string) -> string` | Computes HMAC-SHA256 of input with the given key. |
| `hash_crc32` | `(input: string) -> int64` | Returns the CRC-32 checksum. |
| `constant_time_eq` | `(a: any, b: any) -> bool` | Compares two values in constant time (timing-attack safe). |

```magi
use std::crypto::*

let h = hash_sha512("hello")
let mac = hmac_sha256("message", "key")
let crc = hash_crc32("data")
let eq = constant_time_eq("secret1", "secret2")  // false
```

---

## compress

Compression and decompression using Zstandard and LZ4.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `compress_zstd` | `(input: any) -> bytes` | Compresses data using Zstandard. |
| `decompress_zstd` | `(input: any) -> bytes` | Decompresses Zstandard-compressed data. |
| `compress_lz4` | `(input: any) -> bytes` | Compresses data using LZ4. |
| `decompress_lz4` | `(input: any) -> bytes` | Decompresses LZ4-compressed data. |

```magi
use std::compress::*

let data = to_bytes("hello world, hello world, hello world")
let compressed = compress_zstd(data)
let original = decompress_zstd(compressed)
```

---

## fmt

Value formatting utilities for human-readable output.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `fmt_number` | `(value: any) -> string` | Formats a number with locale-aware separators. |
| `fmt_bytes` | `(value: any) -> string` | Formats a byte count as a human-readable string (e.g., "1.5 MB"). |
| `fmt_duration` | `(value: any) -> string` | Formats a duration in milliseconds as a human-readable string. |
| `fmt_hex` | `(value: any) -> string` | Formats a number as a hexadecimal string. |
| `fmt_binary` | `(value: any) -> string` | Formats a number as a binary string. |
| `fmt_percent` | `(value: any) -> string` | Formats a number as a percentage string. |

```magi
use std::fmt::*

let n = fmt_number(1234567)       // "1,234,567"
let b = fmt_bytes(1536000)        // "1.5 MB"
let d = fmt_duration(3661000)     // "1h 1m 1s"
let h = fmt_hex(255)              // "ff"
let bin = fmt_binary(42)          // "101010"
let pct = fmt_percent(0.75)       // "75%"
```

---

## stats

Statistical functions for numeric data analysis.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `stats_mean` | `(array: array) -> float64` | Returns the arithmetic mean of the array. |
| `stats_median` | `(array: array) -> float64` | Returns the median value of the array. |
| `stats_mode` | `(array: array) -> any` | Returns the most frequent value in the array. |
| `stats_variance` | `(array: array) -> float64` | Returns the variance of the array. |
| `stats_std_dev` | `(array: array) -> float64` | Returns the standard deviation of the array. |
| `stats_min_by` | `(array: array, key: string) -> any` | Returns the element with the minimum value for the given key. |
| `stats_max_by` | `(array: array, key: string) -> any` | Returns the element with the maximum value for the given key. |
| `stats_sum` | `(array: array) -> any` | Returns the sum of all elements in the array. |
| `stats_percentile` | `(array: array, percentile: any) -> float64` | Returns the value at the given percentile (0-100). |
| `stats_quantile` | `(array: array, quantile: any) -> float64` | Returns the value at the given quantile (0.0-1.0). |
| `stats_covariance` | `(a: array, b: array) -> float64` | Returns the covariance of two arrays. |
| `stats_correlation` | `(a: array, b: array) -> float64` | Returns the Pearson correlation coefficient of two arrays. |

```magi
use std::stats::*

let data = [2, 4, 4, 4, 5, 5, 7, 9]
let mean = stats_mean(data)           // 5.0
let med = stats_median(data)          // 4.5
let mode = stats_mode(data)           // 4
let sd = stats_std_dev(data)
let p90 = stats_percentile(data, 90)
let r = stats_correlation([1, 2, 3], [2, 4, 6])  // 1.0
```

---

## text

Text transformation and formatting utilities.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `text_wrap` | `(input: string) -> string` | Wraps text to a specified line width. |
| `text_dedent` | `(input: string) -> string` | Removes common leading whitespace from all lines. |
| `text_indent` | `(input: string) -> string` | Adds indentation to each line of text. |
| `text_pad_left` | `(input: string) -> string` | Pads each line on the left to a given width. |
| `text_pad_right` | `(input: string) -> string` | Pads each line on the right to a given width. |
| `text_truncate` | `(input: string) -> string` | Truncates text to a maximum length, adding an ellipsis. |
| `text_slug` | `(input: string) -> string` | Converts text to a URL-friendly slug (lowercase, hyphens). |
| `text_camel_case` | `(input: string) -> string` | Converts text to camelCase. |
| `text_snake_case` | `(input: string) -> string` | Converts text to snake_case. |
| `text_title_case` | `(input: string) -> string` | Converts text to Title Case. |

```magi
use std::text::*

let slug = text_slug("Hello World!")           // "hello-world"
let camel = text_camel_case("hello_world")     // "helloWorld"
let snake = text_snake_case("helloWorld")      // "hello_world"
let title = text_title_case("hello world")     // "Hello World"
let dedented = text_dedent("    line1\n    line2")
```

---

## encode

HTML and Base32 encoding/decoding.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `html_escape` | `(input: any) -> string` | Escapes HTML special characters (`<`, `>`, `&`, `"`, `'`). |
| `html_unescape` | `(input: any) -> string` | Unescapes HTML entities back to their original characters. |
| `base32_encode` | `(input: any) -> string` | Encodes data as a Base32 string. |
| `base32_decode` | `(input: any) -> bytes` | Decodes a Base32 string to bytes. |

```magi
use std::encode::*

let safe = html_escape("<script>alert('xss')</script>")
// "&lt;script&gt;alert(&#x27;xss&#x27;)&lt;/script&gt;"

let original = html_unescape("&lt;b&gt;bold&lt;/b&gt;")
let b32 = base32_encode("hello")
```

---

## reflect

Runtime type reflection and introspection.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `reflect_type_of` | `(input: any) -> string` | Returns the type name of a value (e.g., "int64", "string", "array"). |
| `reflect_type_name` | `(input: any) -> string` | Returns the human-readable type name of a value. |
| `reflect_is_type` | `(input: any, type_name: string) -> bool` | Returns true if the value matches the given type name. |
| `reflect_fields` | `(input: any) -> array` | Returns the field names (keys) of a map or struct. |
| `reflect_has_field` | `(input: any, field: string) -> bool` | Returns true if the value has the given field. |
| `reflect_callable` | `(input: any) -> bool` | Returns true if the value is a callable function. |
| `reflect_arity` | `(input: any) -> int64` | Returns the number of parameters a callable expects. |
| `reflect_inspect` | `(input: any) -> string` | Returns a detailed string representation of the value for debugging. |

```magi
use std::reflect::*

let t = reflect_type_of(42)                    // "int64"
let is_str = reflect_is_type("hello", "string") // true
let fields = reflect_fields({"a": 1, "b": 2})  // ["a", "b"]
let has = reflect_has_field({"x": 1}, "x")      // true
let info = reflect_inspect([1, "two", null])
```

---

## collections

Set operations, frequency counting, and ordered maps.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `set_from` | `(array: array) -> array` | Creates a set (array of unique elements) from an array. |
| `set_union` | `(a: array, b: array) -> array` | Returns the union of two sets. |
| `set_intersection` | `(a: array, b: array) -> array` | Returns the intersection of two sets. |
| `set_difference` | `(a: array, b: array) -> array` | Returns elements in a that are not in b. |
| `set_symmetric_difference` | `(a: array, b: array) -> array` | Returns elements in either set but not both. |
| `counter` | `(array: array) -> map` | Counts the frequency of each element. Returns a map of element to count. |
| `most_common` | `(array: array) -> array` | Returns elements sorted by frequency (most common first). |
| `ordered_map` | `(array: array) -> map` | Creates an ordered map from an array of [key, value] pairs. |

```magi
use std::collections::*

let a = [1, 2, 3, 4]
let b = [3, 4, 5, 6]
let union = set_union(a, b)             // [1, 2, 3, 4, 5, 6]
let inter = set_intersection(a, b)      // [3, 4]
let diff = set_difference(a, b)         // [1, 2]

let freq = counter(["a", "b", "a", "c", "a", "b"])
// {"a": 3, "b": 2, "c": 1}

let common = most_common(["a", "b", "a", "c", "a"])
```

---

## sort

Sorting algorithms and search.

| Operation | Signature | Description |
|-----------|-----------|-------------|
| `sort_asc` | `(array: array) -> array` | Sorts an array in ascending order. |
| `sort_desc` | `(array: array) -> array` | Sorts an array in descending order. |
| `sort_by` | `(array: array) -> array` | Sorts by a comparator function `(a, b) -> number`. |
| `sort_by_key` | `(array: array) -> array` | Sorts by a key extraction function. |
| `stable_sort` | `(array: array) -> array` | Sorts with guaranteed stability (equal elements preserve order). |
| `is_sorted` | `(array: array) -> bool` | Returns true if the array is sorted in ascending order. |
| `binary_search` | `(array: array, value: any) -> int64` | Searches a sorted array for a value. Returns the index, or -1. |
| `sort_reverse` | `(array: array) -> array` | Reverses the order of elements in the array. |

```magi
use std::sort::*

let asc = sort_asc([3, 1, 4, 1, 5])       // [1, 1, 3, 4, 5]
let desc = sort_desc([3, 1, 4, 1, 5])      // [5, 4, 3, 1, 1]
let sorted = is_sorted([1, 2, 3])          // true
let idx = binary_search([1, 2, 3, 4, 5], 3) // 2
let rev = sort_reverse([1, 2, 3])          // [3, 2, 1]
```

---

## Type Reference

Types used in signatures:

| Type | Description |
|------|-------------|
| `any` | Polymorphic -- accepts any type. Output type depends on input. |
| `bool` | Boolean (`true` or `false`). |
| `int64` | 64-bit signed integer. |
| `float64` | 64-bit floating-point number. |
| `string` | UTF-8 string. |
| `bytes` | Binary byte sequence. |
| `array` | Ordered collection of values. |
| `map` | Key-value dictionary (string keys). |
| `number` | Either `int64` or `float64`, depending on inputs. |
