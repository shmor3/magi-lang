# Changelog

All notable changes to the MAGI language are documented in this file.

## [0.9.0] - 2026-03-21

### Language Features

#### Real Concurrency
- `spawn` creates OS threads for true parallel execution
- `await` joins spawned threads and retrieves results
- `channel()` creates unbounded channels returning `[sender, receiver]`
- `channel(capacity)` creates bounded channels
- Channel operations: `chan_send`, `chan_recv`, `chan_try_recv`, `chan_close`
- Spawned tasks capture outer scope variables by value
- New `std::concurrent` module exposes channel operations

#### Traits and Impl Blocks
- `impl Type { fn method(self) { ... } }` adds methods to structs
- `trait Name { fn method(self); }` defines interfaces
- `impl Trait for Type { ... }` implements traits for structs
- Operator overloading via dunder methods: `__add__`, `__sub__`, `__eq__`, `__lt__`, `__gt__`, `__mul__`, `__div__`, `__neg__`, `__not__`
- Methods on impl blocks receive `self` as first parameter

#### New Types
- `Set` type: `Set(1, 2, 3)` with `contains`, `union`, `intersection`, `difference`, `len`, `is_subset`, `is_superset`
- `Tuple` type: `Tuple(1, "hello", true)` with `typeof(t) == "tuple"`
- `Optional` pattern: `Some(value)`, `None`, `is_some`, `is_none`, `unwrap`, `unwrap_or`
- `Result` pattern: `Ok(value)`, `Err(msg)`, `is_ok`, `is_err`, `unwrap`

#### Control Flow Additions
- `do { ... } while condition;` loops (execute body at least once)
- `defer expr;` runs cleanup expressions at scope exit
- C-style `for` loops: `for (let mut i = 0; i < n; i += 1) { ... }`
- Labeled loops: `'outer: for`, `break 'outer`, `continue 'outer`
- `in` operator for membership testing on arrays, maps, and strings

#### Struct Enhancements
- Default field values: `struct Config { timeout: int64 = 30 }`
- Struct update syntax: `let c2 = Config { ...c1, timeout: 60 }`

#### Other Language Features
- Deprecation attributes: `#[deprecated] fn old_func() { ... }`
- String repetition: `"ha" * 3` produces `"hahaha"`
- Type annotations on function parameters and return types: `fn add(a: int64, b: int64) -> int64`
- Type inference and type narrowing in the type checker

### Standard Library

#### New Modules (40 total)
- `concurrent`: `channel`, `chan_send`, `chan_recv`, `chan_try_recv`, `chan_close`
- `collections`: `set_from`, `set_union`, `set_intersection`, `set_difference`, `set_symmetric_difference`, `counter`, `most_common`, `ordered_map`
- `sort`: `sort_asc`, `sort_desc`, `sort_by`, `sort_by_key`, `stable_sort`, `is_sorted`, `binary_search`, `binary_search_by`, `sort_reverse`
- `reflect`: `reflect_type_of`, `reflect_type_name`, `reflect_is_type`, `reflect_fields`, `reflect_has_field`, `reflect_callable`, `reflect_arity`, `reflect_inspect`
- `encode`: `html_escape`, `html_unescape`, `base32_encode`, `base32_decode`
- `text`: `text_wrap`, `text_dedent`, `text_indent`, `text_pad_left`, `text_pad_right`, `text_truncate`, `text_slug`, `text_camel_case`, `text_snake_case`, `text_title_case`
- `compress`: `compress_zstd`, `decompress_zstd`, `compress_lz4`, `decompress_lz4`
- `stats`: `stats_mean`, `stats_median`, `stats_mode`, `stats_variance`, `stats_std_dev`, `stats_min_by`, `stats_max_by`, `stats_sum`, `stats_percentile`, `stats_quantile`, `stats_covariance`, `stats_correlation`
- `fmt`: `fmt_number`, `fmt_bytes`, `fmt_duration`, `fmt_hex`, `fmt_binary`, `fmt_percent`

#### Enhanced Existing Modules
- `math`: added `gcd`, `lcm`, `is_nan`, `is_infinite`, `is_finite`, `approx_eq`, `math_sum`, `math_product`, `math_average`, `math_min_of`, `math_max_of`, `math_count`, `factorial`, `fibonacci`, `is_prime`, `ncr`, `npr`, `combinations`, `permutations`, `lerp`, `remap`, `sign`
- `time`: added `date_now`, `date_parse`, `date_format`, `date_add`, `date_diff`, `duration_ms`, `duration_secs`, `duration_mins`, `duration_hours`, `time_sleep`, `add_duration`, `sub_duration`, `start_of`, `end_of`
- `hash`: added `hash_sha512`, `hmac_sha256`, `hash_crc32`, `constant_time_eq`
- `crypto`: `hash_sha512`, `hmac_sha256`, `hash_crc32`, `constant_time_eq`
- `cert`: added `cert_generate`, `cert_parse`, `cert_info`, `cert_verify`, `key_generate`, `cert_self_signed`
- `path`: added `path_normalize`, `path_split`, `path_with_extension`, `path_parent`
- `regex`: added `regex_split`, `regex_escape`, `regex_test`, `regex_captures`, `regex_find_all`
- `uuid`: added `uuid_parse`, `uuid_is_valid`, `uuid_nil`
- `csv`: added `csv_parse_rows`
- `yaml`: added `yaml_validate`, `yaml_to_json`, `yaml_from_json`, `yaml_merge`
- `json`: added `json_query`, `json_compact`
- `rand`: added `random_sample`, `random_uuid`, `random_string`

### CLI

- Added `magi doc file.magi` command to generate Markdown from `///` doc comments
- Added `magi bench file.magi` command for benchmarking execution
- Workspace-level `magi test` runs tests across all workspace members
- Improved error messages with ariadne-based rich diagnostics

### Type Checker
- Type inference for variables, function returns, and expressions
- Type narrowing in conditional branches
- SyntaxError error codes for structured diagnostics
- Dead code and unused variable detection via linter

### Performance
- `DataType::Map` migrated from `BTreeMap` to `IndexMap` for insertion-order preservation
- `suggest_variable` and `suggest_function` optimized to avoid unnecessary allocations
- Thread-local LRU cache for compiled regexes
- SSRF and TOCTOU vulnerability fixes
- Reduced numeric type duplication

### Compiler
- WASM compiler generates valid WASM binaries (~5740 lines)
- AST -> IR compilation (~3200 lines)
- `wasm-runtime` feature flag gates optional wasmtime dependency

### LSP Server
- Full Language Server Protocol implementation
- 15 LSP capabilities: completion, hover, go-to-definition, references, rename, document symbols, workspace symbols, call hierarchy, semantic tokens, folding ranges, selection ranges, code actions, code lens, inlay hints, linked editing, signature help, document links

### Testing
- 1386 library tests
- 1506 integration tests
- 2892+ total tests
- Comprehensive coverage for all new features including concurrency, traits, channels, labeled loops, do-while, defer, C-style for, Set/Tuple types, Optional/Result patterns, struct defaults, operator overloading

### Examples
- `examples/hello.magi` -- hello world with main function and FizzBuzz
- `examples/fibonacci.magi` -- recursive and iterative fibonacci
- `examples/channels.magi` -- spawn + channel producer-consumer patterns
- `examples/http_client.magi` -- HTTP GET/POST with JSON parsing
- `examples/file_ops.magi` -- read, write, list, and path utilities
- `examples/json_api.magi` -- parse, construct, query, and transform JSON
- `examples/traits.magi` -- trait definition, impl blocks, operator overloading
- `examples/error_handling.magi` -- try/catch, Result/Optional patterns
- `examples/iterators.magi` -- map/filter/reduce chains and comprehensions
- `examples/concurrency.magi` -- parallel spawns with channels

### Internal
- Total codebase: ~62,560 lines of Rust across 40+ source files
- ~20,410 lines of integration tests
- AST optimizer module added (~1580 lines)
- Operation dispatch module (`src/ops.rs`, ~930 lines)
