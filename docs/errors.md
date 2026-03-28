# MAGI Error Code Reference

## Error Codes

| Code | Description |
|------|-------------|
| E100 | A value of the wrong type was used where a specific type was expected. |
| E101 | Conditions in `if`, `while`, `!`, and match guards must be boolean. Compare explicitly: `x != 0`. |
| E102 | The `for..in` loop requires an iterable (array, map, or string). |
| E103 | An arithmetic operation overflowed or received an argument of the wrong type. |
| E104 | Division or modulo by zero. |
| E105 | Array index out of bounds. Indices must be non-negative integers. |
| E106 | Attempted to index into an empty array literal. |
| E107 | Map literals cannot have duplicate keys. |
| E200 | Variable not declared in this scope. Declare with `let name = value;`. |
| E201 | Function not defined. Check spelling and ensure definition precedes call. |
| E202 | Unrecognized operation or method. Use `use std::module::*` to import. |
| E203 | Module does not exist. Available: math, cmp, logic, bits, str, convert, array, map, bytes, json, time, hash, io, control, rand, fs, env, net, tcp, udp, ws, sse, http_server, path, yaml, csv, toml, regex, uuid, crypto, compress, fmt, stats, text, encode, reflect, collections, sort, cert, platform. |
| E300 | `break` can only be used inside a loop. |
| E301 | `continue` can only be used inside a loop. |
| E302 | `return` can only be used inside a function body. |
| E303 | The placeholder `_` is only valid inside pipe expressions (`|>`). |
| E304 | Each stage of a pipe expression must be a function or operation call. |
| E400 | Loop exceeded iteration limit (100,000,000). Likely an infinite loop. |
| E401 | Call depth exceeded limit (512 levels). Likely infinite recursion. |
| E402 | Assertion failed. |
| E403 | Uncaught throw — not caught by a `try`/`catch` block. |
| E404 | Cannot assign to an immutable variable. Declare with `let mut`. |
| E405 | Wrong number of arguments in function call. |
| E406 | Operation evaluation failed. |
| E407 | Execution cancelled. |
| E408 | Feature not yet implemented. |
| E409 | Resource limit exceeded (string/array too large). |

## Warning Codes

| Code | Description |
|------|-------------|
| W100 | Unused variable. Prefix with `_` to suppress. |
| W101 | Unused import. |
| W103 | Unused function. |
| W106 | Redundant operation (e.g., `--x`, `x == true`). |
| W107 | Suspicious arithmetic (e.g., `x % 1`, `x * 0`). |
| W108 | Unnecessary `return` in tail position. |
| W109 | Unused function parameter. Prefix with `_`. |
| W110 | Variable declared `let mut` but never reassigned. |
| W111 | Name is a reserved keyword. |
| W112 | Default value type mismatch with type annotation. |
| W113 | Or-pattern alternatives must bind the same variable names. |
| W114 | Item is `#[deprecated]`. |
| W200 | Name should use snake_case. |
| W201 | Type name should use PascalCase. |
| W202 | Unreachable code after `return`/`break`/`continue`/`throw`. |
| W203 | Non-exhaustive match. Add missing arms or `_`. |
| W204 | Condition is always `true` or `false`. |
| W205 | Self-comparison (e.g., `x == x`). Likely a bug. |
| W206 | Empty block body. |
| W207 | Unreachable match arm (shadowed by wildcard). |
| W208 | Duplicate import. |
| W209 | Variable shadows a previous binding in the same scope. |
| W212 | Control flow in `finally` block overrides `try`/`catch` result. |
| W214 | Loop without `break` — runs forever. |
| W215 | `if cond { true } else { false }` — simplify to `cond`. |
| W216 | Empty enum (no variants). |
| W229 | Match arm with empty body. |
| W230 | Self-assignment (`x = x`). |
| W231 | `if/else` returning boolean literals matching condition. |
| W233 | Deep nesting (5+ levels). |
| W234 | Duplicate struct field names. |
| W235 | Duplicate enum variant names. |
| W236 | TODO/FIXME comment found. |
| W237 | Magic number — extract to named constant. |
| W238 | Unused variable binding. |
| W239 | Unused `mut` keyword. |
| W240 | Unnecessary trailing `return`. |
| W241 | Redundant boolean comparison (`x == true`). |
| W242 | Nested `if` can be collapsed with `&&`. |
| W243 | Too many parameters (> 7). |
| W244 | TODO/FIXME/HACK comment. |
| W245 | Deprecated function or method. |
| W246 | Match with no arms. |
| W247 | Unused import. |
| W248 | Unused function definition. |
| W249 | Single-arm match — use `if let`. |
| W250 | High cognitive complexity. |
| W251 | Long function body (> 100 lines). |
| W252 | Loop implements map/filter — use `.map()`/`.filter()`. |
