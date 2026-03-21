# MAGI Language Deep Audit — 250+ Findings

**Date:** 2026-03-19
**Version:** 0.3.0-alpha
**Codebase:** ~74,000 lines across 40 files
**Methodology:** 10 focused audit rounds, each targeting a different aspect

---

## Round 1: Bugs & Correctness (Items 1–30)

### Lexer

1. **Lexer accepts `0x` without digits** — `0x` (hex prefix with no digits) may parse as a valid IntLiteral with value 0 rather than erroring. Should reject `0x`, `0o`, `0b` without trailing digits.

2. **Hex literal overflow not caught** — `0xFFFFFFFFFFFFFFFF1` (too many hex digits for u64) silently truncates rather than reporting an error.

3. **Underscore-only numeric literal** — `1___` may parse as valid; should require at least one digit after underscores.

4. **Block comment nesting counter overflow** — Deeply nested `/* /* /* ... */ */ */` could overflow the nesting counter if it's a u32, though practically unlikely.

5. **Raw string `r"..."` doesn't support `r#"..."#` syntax** — Only `r""` is supported; no way to include literal `"` inside a raw string.

### Parser

6. **`parse_expression_no_struct` flag leaks through lambdas** — If a lambda body is parsed while `no_struct_literal=true` (inside an `if` condition), struct literals inside the lambda body are also suppressed.

7. **Assignment to field/index not supported** — `arr[0] = 5` and `obj.field = 5` parse as expression statements, not assignments. Only simple variable names can be assigned to.

8. **Semicolons in if/else blocks ambiguous** — `if cond { a } else { b }` as an expression vs statement depends on whether a semicolon follows, but `eat(Semicolon)` is greedy.

9. **Enum variant with zero fields vs unit variant** — `Enum::Variant()` (empty parens) and `Enum::Variant` (no parens) both exist but may be confused during pattern matching.

10. **For loop pattern: no nested destructuring** — `for [[a, b], c] in nested_array` fails; only one level of destructuring is supported.

### Interpreter

11. **`for` loop applies MAX_LOOP_ITERATIONS to the array length** — If you iterate over a 10M element array that was created via `range()`, the loop cap at `i >= MAX_LOOP_ITERATIONS` means only 10M iterations. But the array was already fully materialized in memory.

12. **`DataType::Map` uses `BTreeMap` not `HashMap`** — Iteration order is alphabetical by key, not insertion order. `{"b": 1, "a": 2}` iterates as `a, b`. This diverges from JavaScript/Python expectations.

13. **String comparison uses byte ordering, not locale-aware** — `"ä" > "z"` is true because UTF-8 byte value of `ä` > `z`. No locale-aware string comparison available.

14. **Float equality uses `==` which fails for NaN** — `NaN == NaN` returns `false` (IEEE 754), but `DataType::Float64(NaN) == DataType::Float64(NaN)` uses Rust's `PartialEq` which also returns false. The `PartialEq` derive on DataType may silently fail for NaN-containing values.

15. **Closure capture is by-value snapshot** — Closures capture variables at definition time, not reference. `let mut x = 1; let f = || x; x = 2; f()` returns 1, not 2. This is undocumented.

16. **Recursive GC doesn't trace nested DataType values** — The GC only marks direct symbol table addresses as roots. If a heap value contains an array of heap-allocated values, the inner values aren't traced. However, since DataType is a value type (not reference), this is actually fine — but the GC/heap complexity is wasted.

17. **`map_get` returns null for missing keys** — No way to distinguish between a key whose value is null and a missing key. Should return an Option-like type or have a separate `map_has` check.

18. **`try/catch` doesn't catch panics** — If the evaluator panics (e.g., from a bug in a Rust dependency), the panic propagates uncaught.

19. **`output` captures to logs but CLI also prints** — In the CLI's `cmd_run`, output goes both to the interpreter's logs and to stdout. If running programmatically, the stdout side-effect is unavoidable.

20. **Spawn/await is synchronous** — `spawn compute(6, 7)` creates a `Future::Resolved` immediately. No actual concurrency. This is documented in the showcase but may confuse users.

### Type Checker

21. **Type checker doesn't analyze lambda bodies** — Lambda expressions are treated as opaque; no type checking of their bodies.

22. **Const definitions don't get const-propagation** — `const PI = 3.14; let r = PI * 2;` — the type checker doesn't know PI is Float64.

23. **Module-qualified function calls not type-checked** — `math::sqrt(4)` — the type checker doesn't resolve module functions.

24. **`OperationType::parse` comment mismatch** — `ops.rs` comment says "Null = polymorphic" but after the ChannelType::Any fix, these should say "Any = polymorphic".

### Compiler

25. **WASM indirect call table size is static** — The function table size equals the number of functions at compile time. Dynamically registered lambdas can't be called indirectly.

26. **No bounds checking on array access in WASM** — `arr[i]` doesn't emit bounds checks; accessing out of bounds reads garbage memory.

27. **`Return` instruction in WASM doesn't clean up locals** — Returning from a deeply nested block may leave stale values on the WASM stack.

28. **String data offset 1024 may collide with stack** — The WASM data section starts at byte 1024, but the implicit WASM stack also starts at 0. No explicit stack pointer management.

29. **`intern_string` uses linear scan** — `IrModule::intern_string` does `self.strings.iter().position(...)` which is O(n) per string. For programs with many strings, this is O(n²).

30. **Compiler `block_depth` tracking can desync** — If a compilation error occurs mid-block, the `block_depth` counter may not be decremented, leading to incorrect branch offsets.

---

## Round 2: Performance (Items 31–60)

### Critical Performance Issues

31. **O(n²) array building pattern** — `let mut arr = []; for i in range(0, n) { arr = array_push(arr, i); }` clones the entire array on each push. 10,000 iterations = 50M copies.

32. **`available_variable_names()` allocates HashSet on every call** — Called for error messages (suggest_variable), but creates a new HashSet of all variable names each time.

33. **`available_function_names()` clones all keys** — Returns `Vec<String>` by cloning every function name, only used for error suggestion.

34. **Token text is always owned String** — Every token gets a heap-allocated String, even for keywords/operators where a `&'static str` would suffice. For a 10K line file, that's ~50K unnecessary allocations.

35. **DataType::clone is deep for containers** — Cloning `DataType::Array(vec_of_1000_items)` copies all 1000 items recursively. Every variable read triggers a clone.

36. **HashMap<String, DataType> for operation inputs** — Every operation call allocates a new HashMap with string keys. A fixed-size struct or array would be faster.

37. **`op_input_ports` returns `&'static [&'static str]` but `op_input_types` returns `Vec`** — The ports function is zero-alloc but the types function allocates a Vec on every call. Only used in the type checker, but called for every expression.

38. **`datatype_to_display` allocates for simple types** — Converting `DataType::Int64(42)` to display string allocates. Could use `write!` to a shared buffer.

39. **Scope push/pop allocates new HashMap** — Every block, loop iteration, and function call creates a new `HashMap::new()`. Pre-sized or arena-allocated scope maps would be faster.

40. **`BTreeMap` for DataType::Map is slower than HashMap** — BTreeMap provides sorted keys but O(log n) access vs HashMap's O(1). Most map usage doesn't need sorted keys.

### Medium Performance Issues

41. **String interpolation allocates per-segment** — `f"Hello {name}, you are {age}"` evaluates each segment separately and concatenates. A single-pass builder would be faster.

42. **`merge_sort_by` clones arrays for each split** — `items[..mid].to_vec()` copies half the array at each recursion level: O(n log n) copies.

43. **`try_eval_hof_method` matches on string method names** — 30+ string comparisons per method call. A perfect hash or enum dispatch would be faster.

44. **`try_eval_direct_method` has separate match arms for each numeric type** — Int32, Int64, Uint32, Uint64, Float32, Float64 all have near-identical method implementations. Massive code duplication (lines 1860-2400).

45. **Regex compilation not cached** — Every `regex_match`, `regex_replace`, etc. recompiles the regex pattern. Should cache recently used patterns.

46. **`FullEvaluator` allocates HashMap for every operation** — Even simple operations like `Add(3, 5)` allocate a HashMap with "a" and "b" entries.

47. **String method `replace` scans the string twice** — First `s.matches(&from).count()` for bounds checking, then `s.replace(&from, &to)` for the actual replacement.

48. **Heap free list is never compacted** — Freed allocations accumulate in `free_list: Vec<(MemAddr, u64)>` but are never merged into contiguous regions.

49. **`format!` used for simple string concatenation** — Throughout the codebase, `format!("{}::{}", a, b)` is used where `format!` with concat would be clearer but still allocates.

50. **LSP analyzes entire document on every keystroke** — `TextDocumentSyncKind::FULL` means the entire document text is re-sent and re-parsed on each change. Incremental sync would be better.

### Low Performance Issues

51. **`suggest_name` computes Levenshtein distance against all available names** — For large scopes with many variables, this is O(n × m²) where m is name length.

52. **Error formatting allocates strings eagerly** — Error messages like `format!("Expected '{}', got '{}'", kind, tok.kind)` are always allocated even if the error is caught by try/catch.

53. **Parser clones tokens on `advance().clone()`** — Several parser methods clone the current token even when only the text or span is needed.

54. **`to_i128_numeric` helper called repeatedly in method chains** — The numeric conversion helper is called fresh each time; could be cached.

55. **`exec_block` always pushes scope even for empty blocks** — `{}` allocates a new HashMap scope.

56. **Formatter string escaping scans character-by-character** — `escape_string_contents` iterates character by character even for ASCII-only strings.

57. **CSV parsing uses string-based matching** — The `csv` crate is used for parsing but results are converted through serde_json intermediate format.

58. **`wasmparser` only used in dev-dependencies** — Correct, but still compiled if you run tests.

59. **`ordered-float` dependency unused in runtime** — Only used in the type checker for potential future ordered comparison. Could be removed.

60. **`textwrap` dependency for a single function** — Used only for `TextWrap` operation. The algorithm is simple enough to inline.

---

## Round 3: Missing Language Features (Items 61–95)

### Type System

61. **No generic/parameterized types** — `array<int64>`, `map<string, bool>` not supported.
62. **No union types** — `int64 | string` not expressible.
63. **No optional type syntax** — `?int64` or `Option<int64>` not available; must use null.
64. **No function types** — `fn(int64, int64) -> int64` not expressible in type annotations.
65. **No tuple types** — `(int64, string)` not available; must use arrays.
66. **No type inference for return types** — Functions always return `Any` to the type checker.
67. **No type narrowing in conditionals** — `if typeof(x) == "string" { x.len() }` doesn't narrow x's type.
68. **No const generics** — No `array<int64, 3>` (fixed-size array types).

### Control Flow

69. **No `else if` as first-class construct** — Must nest: `if a { } else { if b { } }` works but `else if` is parsed as else-then-if.
70. **No labeled loops** — `break 'outer` for breaking from nested loops not supported.
71. **No `do-while` loop** — Must use `loop { ... if !cond { break; } }`.
72. **No `for (let i = 0; i < n; i++)` C-style loops** — Only `for x in collection` and `while` loops.
73. **No `switch` statement** — Only `match` expression. No fall-through semantics.
74. **No `yield` / generators** — Can't create lazy iterators.
75. **No `defer` statement** — No Go-style deferred cleanup.

### Data Types

76. **No Set type** — `SetFrom` operation exists but there's no `set` literal or `Set` type.
77. **No Tuple type** — Arrays serve as tuples but lose type information.
78. **No Optional/Result type** — Null-checking replaces proper Option/Result.
79. **No BigInt** — Integers limited to i64/u64 range.
80. **No Decimal type** — No arbitrary-precision decimal for financial calculations.
81. **No Date/DateTime type** — Only unix timestamps (i64). No date arithmetic API.
82. **No Duration type** — Sleep and time operations use raw millisecond integers.
83. **No Regex type** — Regex patterns are plain strings, compiled on every use.

### Functions

84. **No method definitions on types** — Can't define `fn Point.distance(other)`.
85. **No trait/interface system** — No shared behavior contracts.
86. **No operator overloading** — Custom types can't define `+`, `-`, `==`, etc.
87. **No property getters/setters** — Struct fields are always direct access.
88. **No variadic keyword arguments** — `**kwargs` pattern not supported.
89. **No function overloading** — Can't have `fn add(a: int64, b: int64)` and `fn add(a: string, b: string)`.
90. **No named return values** — Can't name what a function returns for documentation.

### Modules

91. **No circular import detection for file-level imports** — Package imports check for circular deps but `use` of modules doesn't.
92. **No re-exports** — `pub use sub::item` to re-export from a module not supported.
93. **No conditional imports** — No `#[cfg]` or `if env("DEBUG")` compile-time conditionals.
94. **No import groups** — `use std::math::{sqrt, sin, cos}` not supported; must import one at a time.
95. **No wildcard re-exports** — Can't `pub use sub::*` from a module.

---

## Round 4: Missing Standard Library Operations (Items 96–130)

### String Operations

96. **No `string.pad_center`** — Center-alignment padding not available.
97. **No `string.truncate_ellipsis`** — Truncate with `...` suffix not built-in.
98. **No `string.count_words`** — Word count requires manual split + len.
99. **No `string.capitalize`** — Capitalize first letter only (vs `to_upper` which uppercases all).
100. **No `string.uncapitalize`** — Lowercase first letter.
101. **No `string.strip_prefix` / `strip_suffix`** — Remove a specific prefix/suffix if present.
102. **No `string.byte_length`** — Get byte length vs character length.
103. **No `string.encode(encoding)`** — Only UTF-8 encoding supported.

### Array Operations

104. **No `array.flatten_depth(n)`** — Only `flatten` (infinite depth).
105. **No `array.rotate_left/right`** — Rotate array elements.
106. **No `array.transpose`** — Transpose 2D array (matrix).
107. **No `array.interleave`** — Interleave two arrays `[1,2,3].interleave([a,b,c])` → `[1,a,2,b,3,c]`.
108. **No `array.combinations(n)`** — Generate n-element combinations.
109. **No `array.permutations`** — Generate permutations.
110. **No `array.sliding_window`** — `window(n)` exists but `sliding_window(n, step)` doesn't.
111. **No `array.dedup`** — Remove consecutive duplicates (not same as unique).
112. **No `array.binary_search_by`** — Only value-based binary search.

### Map Operations

113. **No `map.invert`** — Swap keys and values.
114. **No `map.defaults`** — Merge with defaults (only overwrite missing keys).
115. **No `map.pick(keys)`** — Select subset of keys.
116. **No `map.omit(keys)`** — Remove subset of keys.
117. **No `map.map_entries`** — Transform both key and value simultaneously.
118. **No `map.group_values_by_key`** — Group multiple maps by shared keys.
119. **No `map.deep_merge`** — Recursive merge of nested maps.
120. **No `map.flatten_keys`** — Flatten `{"a": {"b": 1}}` to `{"a.b": 1}`.

### Math Operations

121. **No `math.factorial`** — Must implement recursively.
122. **No `math.fibonacci`** — Must implement manually.
123. **No `math.prime_check`** — No primality testing.
124. **No `math.combinations`** — nCr not built-in.
125. **No `math.permutations`** — nPr not built-in.
126. **No complex number support** — No `Complex` type or operations.
127. **No matrix operations** — No matrix multiply, determinant, inverse, etc.
128. **No interpolation beyond lerp** — No cubic, bezier, or spline interpolation.

### I/O Operations

129. **No stdin read** — Can't read from stdin (only environment variables).
130. **No file watch** — No `fs_watch(path)` for file change notifications.

---

## Round 5: Error Handling & Diagnostics (Items 131–160)

131. **Error codes have gaps** — W102, W104, W105 are skipped; no documentation on why.
132. **No error recovery in the interpreter** — A single runtime error aborts the entire program.
133. **Stack traces missing** — Runtime errors don't include a call stack trace.
134. **No source file information in errors** — Errors show line:column but not the filename.
135. **No multi-file error aggregation** — Errors from imported packages are reported individually.
136. **`InterpError` has 20+ variants** — Complex error type; some variants duplicate information.
137. **`EvalError` maps to error codes via string matching** — `error_code()` checks if `msg.contains("exceeds")` — fragile.
138. **Type checker warnings not suppressible per-line** — No `// @suppress W100` or similar mechanism.
139. **No `--max-errors` flag** — Can't limit the number of errors reported.
140. **Linter and type checker produce separate diagnostic lists** — Must be merged by the caller; no unified diagnostic pipeline.
141. **No fix-it hints in diagnostics** — LSP diagnostics have `suggestion` field but no structured edit.
142. **`SyntaxError` doesn't carry an error code** — Only `line`, `column`, `message`; no stable code.
143. **Parser error recovery may skip valid code** — `synchronize()` advances until it finds a statement boundary, potentially skipping recoverable code.
144. **No warning for unreachable code after `loop { ... }` without break** — Only `while true` without break is detected.
145. **No warning for empty match arms** — `match x { 1 => {} }` — the empty body may be unintentional.
146. **No warning for `let x = x`** — Self-assignment that does nothing.
147. **No warning for redundant parentheses** — `(((x)))` is valid but probably unintentional.
148. **No warning for `if x { true } else { false }`** — Should simplify to just `x`.
149. **No warning for `if !x { a } else { b }`** — Could simplify to `if x { b } else { a }`.
150. **No warning for unused `output` result** — `let x = output "hello"` — the return value of output is captured but likely unused.
151. **No warning for TODO/FIXME comments** — Common in development but should be flagged.
152. **No warning for magic numbers** — `if x > 42` — the 42 should probably be a named constant.
153. **No warning for deeply nested code** — Functions with 5+ levels of nesting are hard to read.
154. **No suggestion for common operation name typos** — `arry.push(1)` doesn't suggest `array_push`.
155. **Error messages inconsistent in capitalization** — Some start with uppercase, some lowercase.
156. **No structured error output (JSON)** — CLI only outputs human-readable errors, no machine-parseable format.
157. **No `--json` flag for diagnostics** — Would help IDE integrations beyond LSP.
158. **Span tracking is line:column only** — No byte offset tracking, which some tools need.
159. **No error for duplicate struct field names** — `struct P { x: int64, x: int64 }` silently uses last.
160. **No error for duplicate enum variant names** — `enum E { A, A }` silently duplicates.

---

## Round 6: WASM Compiler Gaps (Items 161–195)

### Missing AST Node Compilation

161. **No closure compilation with captured variables** — Lambdas that capture outer variables emit `Unsupported`.
162. **No try/catch compilation** — `try {} catch {}` emits unsupported.
163. **No async/await compilation** — `async fn` and `await` unsupported in WASM.
164. **No optional chaining compilation** — `obj?.field` unsupported.
165. **No null coalescing compilation** — `x ?? default` unsupported.
166. **No comprehension compilation** — `[x * 2 for x in arr]` unsupported.
167. **No map comprehension compilation** — `{k: v for [k, v] in entries}` unsupported.
168. **No enum construction compilation** — `Result::Ok(42)` unsupported.
169. **No struct construction compilation** — `Point { x: 1, y: 2 }` unsupported.
170. **No destructuring compilation** — `let [a, b] = arr` unsupported.
171. **No string interpolation compilation** — `f"hello {name}"` unsupported.
172. **No method call compilation** — `arr.push(5)` unsupported.
173. **No pipe expression compilation** — `x |> f(_)` unsupported.
174. **No throw/try-propagate compilation** — `throw "error"` and `expr?` unsupported.
175. **No module/use compilation** — `mod utils { ... }` unsupported.
176. **No test block compilation** — `test "name" { ... }` unsupported.
177. **No const definition compilation** — `const PI = 3.14` unsupported.
178. **No type alias compilation** — `type Id = int64` unsupported (should be no-op).
179. **No loop expression compilation** — `loop { ... break value }` unsupported.

### WASM Runtime Gaps

180. **No string concatenation in WASM** — No `__string_concat` runtime function.
181. **No string comparison in WASM** — No `__string_eq`, `__string_lt`, etc.
182. **No array indexing in WASM** — `ArrayGet` instruction exists in IR but runtime support limited.
183. **No map operations in WASM** — Maps can be constructed but not accessed.
184. **No garbage collection in WASM** — Bump allocator only; memory never freed.
185. **No stack overflow protection in WASM** — Deep recursion can corrupt memory.
186. **No error reporting from WASM** — Runtime errors (div by zero, OOB) trap without message.
187. **No I/O beyond print in WASM** — Only `env.print` host function; no file/network access.
188. **No type tags for complex values** — NaN-boxing only handles null, bool, int, float, string ref. No array/map tags.
189. **No memory growth** — Initial 16 pages, max 256, but no `memory.grow` instruction emitted.

### Code Quality

190. **Compiler has no optimization passes** — No dead code elimination, constant folding, or strength reduction.
191. **No debug info in WASM** — No DWARF sections, no name section.
192. **No WASM validation before output** — Generated WASM isn't validated for correctness.
193. **WASM tests only check binary structure** — Don't execute and verify output.
194. **No incremental compilation** — Entire module recompiled on any change.
195. **No compilation cache** — Repeated compilations of the same source start from scratch.

---

## Round 7: LSP & Tooling Gaps (Items 196–225)

### LSP

196. **No semantic tokens** — No syntax highlighting beyond basic TextMate grammars.
197. **No inlay hints** — No type annotations shown inline.
198. **No code lens** — No "N references" or "Run test" annotations.
199. **No workspace symbols** — Can't search symbols across files.
200. **No folding ranges** — IDE can't fold blocks, functions, etc.
201. **No selection ranges** — No smart selection expansion.
202. **No linked editing ranges** — No synchronized editing of matching identifiers.
203. **No document links** — `use pkg::shared` doesn't become a clickable link.
204. **No call hierarchy** — Can't see who calls a function or what a function calls.
205. **No type hierarchy** — No enum/struct inheritance visualization (N/A currently).
206. **No diagnostic tags** — Unused variable warnings not tagged as `unnecessary`.
207. **No code actions for quick fixes** — No "add missing import", "rename to snake_case", etc.
208. **No organize imports** — No automatic import sorting/cleanup.
209. **Completion doesn't include snippets** — No `fn $1($2) { $0 }` snippet completion.
210. **Hover doesn't show function signatures** — Only shows operation names, not full signatures.
211. **Signature help is basic** — Shows parameter names but not types.
212. **No incremental document sync** — Full document re-sent on every change.

### CLI Tooling

213. **No `magi doc` command** — Can't generate documentation from source.
214. **No `magi bench` command** — No benchmarking support.
215. **No `magi fix` command** — No auto-fix for lint warnings.
216. **No `magi new` / `magi init`** — No project scaffolding.
217. **No `magi publish` command** — No package publishing to registry.
218. **No `magi install` command** — No dependency installation.
219. **No `magi update` command** — No dependency update checking.
220. **No `magi tree` command** — No dependency tree visualization.
221. **No `magi eval` command** — Can't evaluate an expression from CLI args.
222. **No `magi run --watch`** — No file watching with auto-restart.
223. **No `magi run --timeout`** — No execution timeout flag.
224. **No `magi run --sandbox`** — No sandboxed execution mode.
225. **No profiler integration** — No `magi run --profile` for performance analysis.

---

## Round 8: Code Quality & Maintenance (Items 226–255)

### Architecture

226. **`magi.rs` binary still 8K+ lines** — Even after extracting eval functions, the binary is huge.
227. **Interpreter and FullEvaluator duplicate method dispatch** — Direct methods (`.abs()`, `.trim()`) are implemented both in the interpreter AND in the FullEvaluator.
228. **No shared method registry** — String/array/map methods are hardcoded in multiple match statements.
229. **`ops.rs` has 3 parallel match statements** — `op_output_type`, `op_input_types`, `op_input_ports` must all be kept in sync.
230. **OperationType has 374+ variants** — The enum is massive; could be split into sub-enums.
231. **No operation categorization enum** — Operations have implicit categories (arithmetic, string, etc.) but no formal grouping.
232. **Test file is 19K lines** — `integration.rs` is a single massive file; should be split.

### Code Duplication

233. **Numeric method implementations duplicated 6 times** — `abs`, `pow`, `min`, `max`, `clamp`, `sign` implemented separately for Int32, Int64, Uint32, Uint64, Float32, Float64.
234. **Port extraction code duplicated** — `get_port` and `get_bind_port` in `magi.rs` are near-identical.
235. **Error construction is verbose** — `InterpError::TypeError { expected: ..., actual: ..., context: ..., span }` repeated hundreds of times.
236. **HOF methods all have identical cancellation/limit checks** — Every method starts with `if self.is_cancelled() { return Err(...); }`.
237. **`to_i128_numeric` helper could be a `DataType` method** — Currently a free function used only by Uint32/Uint64 methods.
238. **Connection registry helpers (`conn_store`, `conn_with`, `conn_remove`) are `#[allow(dead_code)]`** — Suggesting they may not actually be used in all builds.

### Testing

239. **No property-based testing** — No `proptest` or `quickcheck` for fuzzing AST/parser.
240. **No snapshot testing** — Formatter output not tested via snapshots.
241. **No mutation testing** — No verification that tests actually catch bugs.
242. **No coverage reporting** — No `cargo-tarpaulin` or `llvm-cov` integration.
243. **No regression test for each bug fix** — Bug fixes should have minimal reproduction tests.
244. **Integration tests use `StubEvaluator`** — Doesn't test the real `FullEvaluator` operations.
245. **No cross-compilation tests** — WASM target not tested in CI.
246. **No memory leak tests** — No tests that verify the GC actually collects garbage.
247. **E2E tests don't verify WASM execution output** — Only check binary structure.
248. **No benchmark regression tests** — No way to detect performance regressions.

### Documentation

249. **No inline doc comments on public API** — Many `pub fn` lack `///` doc comments.
250. **No module-level documentation** — Some modules have `//!` docs, others don't.
251. **No changelog** — No CHANGELOG.md tracking version changes.
252. **No contribution guide** — No CONTRIBUTING.md for new contributors.
253. **No architecture decision records** — Design decisions not documented.
254. **No error code reference page** — Error codes defined in code but not in user docs.
255. **No API stability guarantees** — No `#[stable]` / `#[unstable]` annotations.

---

## Round 9: Security & Robustness (Items 256–280)

256. **No filesystem sandboxing** — Scripts can read/write/delete any file.
257. **No network sandboxing** — Scripts can make HTTP requests to any host (except blocked IPs).
258. **No resource quotas** — No CPU time limit, memory limit, or file handle limit.
259. **No capability-based permissions** — All operations available to all scripts.
260. **Regex ReDoS vulnerability** — User-provided regex patterns not checked for catastrophic backtracking. No timeout on regex execution.
261. **No size limit on parsed AST** — A deeply nested expression `(((((...))))` with 128 levels of nesting is allowed. Could be used for DoS.
262. **No limit on number of variables** — A script can define millions of variables, consuming unbounded memory.
263. **No limit on function definitions** — Millions of function definitions allowed.
264. **No limit on string literal length in source** — A single string literal can be gigabytes.
265. **TOML/YAML/CSV parsing may consume unbounded memory** — Large input files parsed without size limits.
266. **Decompression bomb protection is per-operation** — Compress then decompress 10 times bypasses the 64MB limit.
267. **WebSocket receive has no message size limit** — A malicious server could send a multi-GB message.
268. **TCP read has no size limit** — `TcpRead` reads all available data without bounding.
269. **HTTP server receive has no body size limit** — `HttpServerReceive` reads the entire request body.
270. **No TLS certificate validation opt-out** — Can't disable cert validation for testing (debatable).
271. **Connection registry uses global mutable state** — `CONNECTIONS` is a `LazyLock<Mutex<HashMap>>`. Panics in one operation can poison the mutex for all subsequent operations.
272. **Sleep operation has no maximum** — `sleep(999999999)` blocks the thread for 31 years.
273. **No rate limiting on operations** — A script can make unlimited HTTP requests per second.
274. **File write has no size limit** — Can write gigabytes to disk.
275. **`env_get` can read sensitive environment variables** — Passwords, API keys, etc.
276. **`process_pid` leaks process information** — Minor information disclosure.
277. **Path traversal in `fs_read`** — `fs_read("../../etc/passwd")` works if not sandboxed.
278. **No audit log** — No record of what operations a script performed.
279. **Mutex poisoning recovery swallows errors** — `unwrap_or_else(|e| e.into_inner())` loses the panic backtrace.
280. **No constant-time comparison for non-crypto paths** — `ConstantTimeEq` exists as an operation but `==` on strings is not constant-time.

---

## Round 10: UX & Ergonomics (Items 281–310)

### Error Messages

281. **"Unknown operation" is a catch-all** — Many different failures produce this error; should be more specific.
282. **No "expected N arguments, got M" for method calls** — Method arity errors show the method name but not the actual vs expected count clearly.
283. **Type annotations in error messages use internal names** — "expected Null" should say "expected any type" (after the Any/Null fix).
284. **No color in non-REPL error output** — `magi run` errors are plain text; `magi test` has color.
285. **No caret pointing to error location** — Just `line 5:10: error` — no source code snippet.

### Language Ergonomics

286. **No `else if` keyword** — Must use `else { if ... }` or rely on parser sugar.
287. **Can't chain comparisons** — `1 < x < 10` parses as `(1 < x) < 10` which compares bool to int.
288. **No ternary operator** — Must use `if cond { a } else { b }` for inline conditionals.
289. **No string multiplication** — `"ha" * 3` doesn't work; must use `"ha".repeat(3)`.
290. **No `in` operator for containment** — `if x in arr` not supported; must use `arr.contains(x)` or `arr.any(|i| i == x)`.
291. **Map literal keys must be strings** — `{1: "one"}` not supported; only `{"1": "one"}`.
292. **No computed map keys** — `{[expr]: value}` not supported.
293. **No shorthand field names in struct construction** — `Point { x, y }` doesn't work; must use `Point { x: x, y: y }`.
294. **No default values in struct definitions** — `struct Config { timeout: int64 = 30 }` not supported.
295. **No struct update syntax** — `{ ...old_point, x: 5 }` not supported for structs.
296. **No chained method calls with newlines** — Method chains must be on one line or use explicit backslash continuation (which doesn't exist).
297. **Array.sort() returns a new array** — Sorting is non-mutating which requires `arr = arr.sort()`. No in-place sort.
298. **No implicit return without tail expression** — `fn f() { let x = 5; }` returns null, not 5. Must omit semicolon: `fn f() { let x = 5; x }` or use explicit return.
299. **Semicolon sensitivity in blocks** — `{ expr; }` vs `{ expr }` determines whether the block returns the expression value. Easy to get wrong.
300. **No named arguments in function calls** — Can't do `greet(who: "World", greeting: "Hi")`.

### Developer Experience

301. **No syntax highlighting for any editor** — No VSCode extension, no TreeSitter grammar.
302. **No language server auto-install** — Must manually build and configure `magi lsp`.
303. **No playground/REPL website** — No online MAGI playground.
304. **No package registry** — `magispace.toml` supports local paths only.
305. **No standard project layout convention** — No convention for `src/`, `tests/`, `lib/`.
306. **REPL has no history persistence** — History lost when REPL exits.
307. **REPL has no tab completion** — No tab-completion for keywords/functions.
308. **REPL has no syntax highlighting** — Input is plain text.
309. **No debug adapter protocol (DAP)** — Debugging infrastructure exists but no DAP server.
310. **No profiling output** — No way to see which functions/operations take the most time.

---

## Round 11: Deep Code-Level Bugs (Items 311–340)

### Interpreter Bugs

311. **`call_function` replaces entire symbol table** — `std::mem::replace(&mut self.symbols, vec![HashMap::new()])` discards the entire scope chain. Functions cannot access global variables unless captured via `closure_captures` hack. This means `const PI = 3.14; fn f() { PI }` fails unless `f` is `main()`.

312. **Lambda captures are by-value at definition time only** — `let mut x = 1; let f = || x; x = 2; f()` returns 1. Closures snapshot, never observe mutation. Undocumented and surprising for users coming from JS/Python.

313. **`assert_throws` only calls with zero arguments** — `assert_throws(fn_name)` calls the function with `&[]`, so you can't test that a function throws with specific arguments.

314. **`f64::NAN == f64::NAN` returns false in DataType::Equal** — `OperationType::Equal` uses `a == b` which follows IEEE 754 (NaN != NaN). But `DataType::PartialEq` derive also returns false for NaN. This means `let x = 0.0/0.0; x == x` is false, which is correct but undocumented.

315. **Short-circuit `&&`/`||` requires Bool on left side** — `0 && true` errors with "expected Bool, got int64" instead of short-circuiting on falsy 0. This differs from JS/Python where `&&`/`||` work on any truthy/falsy value.

316. **`exec_block` doesn't push/pop scope** — Block execution in `exec_block` doesn't push a new scope. Variables declared inside a bare block `{ let x = 1; }` leak into the enclosing scope (unless called from IfElse/ForLoop/etc. which do push scope).

317. **EnumDef and StructDef registered twice** — Both in `execute()` pass 1 and in `exec_statement()`. If a struct is defined inside an if-block, it gets registered in pass 1 (unconditionally) AND again when the if-block executes.

318. **`map_get` on struct returns field by string key** — FieldAccess dispatches to `MapGet` operation, which means structs are just maps with string keys at runtime. No struct-specific field validation.

319. **`use` of non-existent module gives "Unknown operation" error** — `use math::nonexistent` fails with "Unknown operation: math::nonexistent" instead of "Module item not found".

320. **No validation that rest parameter is actually last** — The parser checks for this, but if AST is constructed programmatically, a rest parameter in the middle would cause argument binding bugs.

### Parser Bugs

321. **`parse_type_alias` only accepts single-identifier target** — `type Callback = fn(int64) -> string;` fails because the parser expects a single `Ident` token for the target.

322. **Map literal keys can't be expressions** — `{["key_" + i]: value}` fails; only string literal keys are supported.

323. **No compound assignment for field/index access** — `arr[0] += 1` and `obj.count += 1` are not supported; only `name += expr`.

324. **Implicit semicolons after blocks are inconsistent** — `if cond { a } else { b }` doesn't need a semicolon, but `let x = if cond { a } else { b }` does (optional).

325. **`pub` modifier is silently ignored** — `pub fn f() {}` is identical to `fn f() {}` at runtime. No visibility enforcement.

### Type Checker Bugs

326. **Duplicate parameter names not caught by type checker** — The parser catches this, but if the type checker receives a malformed AST (e.g., from a buggy tool), it won't detect duplicate params.

327. **`use std::nonexistent::*` silently does nothing** — No error or warning when importing from a non-existent std module.

328. **Method calls on any type default to "unknown operation"** — `42.nonexistent()` produces E202 but with no suggestion of available methods for int64.

329. **Type checker tracks `used` flag but never reads it for constants** — `const X = 1;` is never flagged as unused even if X is never referenced.

330. **Struct field type annotations are not validated** — `struct P { x: nonexistent_type }` produces no error.

### Compiler Bugs

331. **`register_builtins` hardcodes function indices** — If the user defines a function named "println", it shadows the builtin registration, creating duplicate indices.

332. **`compile_if_else` without else branch pushes Null** — `if cond { value }` compiles to push null in the else branch, but the WASM `if` instruction expects matching stack types for both branches.

333. **`compile_for_loop` uses `LocalGet` for array length** — But the array may have been modified during iteration. The length is captured once at loop start, which is correct for immutable arrays but confusing if mutability is ever added.

334. **Lambda compilation uses `lambda_counter` but doesn't reset per-module** — In a multi-module compilation, lambda names could collide across modules.

335. **`compile_match` wildcard arm emits `Drop` + `PushNull`** — Discards the matched value even if the arm body references it. Wildcard arms that bind a variable (not `_`) would lose the value.

336. **WASM `Call` instruction uses function index directly** — No indirection through the function table. This means lambdas passed as values can't be called via `call_indirect`.

337. **No overflow checking for integer arithmetic in WASM** — `I64Add` wraps silently; the interpreter uses `checked_add` and reports overflow.

338. **`emit_function` creates temp locals for every instruction** — `count_temp_locals_needed` scans all instructions and allocates the max temps. This overestimates; many temps could be shared.

339. **String pool deduplication is O(n²)** — `intern_string` searches linearly through all existing strings for each new string.

340. **No WASM validation before writing to disk** — The generated WASM binary is not validated by wasmparser before saving. Invalid modules can be produced.

---

## Round 12: Interpreter Method Dispatch (Items 341–365)

341. **6 identical `pow` implementations for numeric types** — Int32, Int64, Uint32, Uint64, Float32, Float64 each have separate `pow` methods (~20 lines each) that differ only in type conversion.

342. **6 identical `min` implementations** — Same pattern: parse arg, coerce, compute min, return typed result.

343. **6 identical `max` implementations** — Same as min, just `max` instead.

344. **6 identical `clamp` implementations** — Same pattern with 2 args.

345. **6 identical `sign` implementations** — `n.signum()` duplicated per type.

346. **6 identical `to_string` implementations** — `n.to_string()` duplicated per type.

347. **Float32 methods cast through f64 and back** — `(*n as f64).sin() as f32` loses precision unnecessarily. Should use `f32::sin` directly.

348. **No `abs` method on Uint32/Uint64** — Wait, it exists but just returns self. Could skip the match arm.

349. **String method `is_numeric` allows leading/trailing whitespace** — `" 42 ".is_numeric()` returns true due to `.trim()`. This may surprise users.

350. **String `index_of` returns -1 for not-found** — Inconsistent with `find` on arrays which returns null. Should be null for consistency.

351. **String `replace` with empty pattern inserts between every character** — `"abc".replace("", "-")` returns `"-a-b-c-"`. This is Rust's behavior but surprising. Should either document or disallow.

352. **Array `sum` silently upgrades to float on overflow** — If integer sum overflows, it switches to float accumulation. The intermediate value (int_sum) is added to float_sum, losing precision for large integers.

353. **Array `product` has same overflow-to-float issue** — Silent precision loss.

354. **Array `min`/`max` mixes comparison types** — Comparing `Int64(5)` with `Float64(5.1)` works via numeric fallback, but comparing `Int64(1)` with `String("a")` fails. Heterogeneous arrays can partially work.

355. **Array `sort` (in try_eval_direct_method) is not defined** — The `sort` method exists as an OperationType but isn't in `try_eval_direct_method`. It falls through to the OperationEvaluator which may handle it differently.

356. **Array `join` defaults to comma separator** — `[1,2,3].join()` uses `","`. This differs from JS (`""`) and Python (`""`). Should document.

357. **Map methods `map_keys` uses `to_string_lossy` for non-string returns** — If the lambda returns an int, it's silently coerced to string.

358. **HOF methods clone the array before iterating** — `arr.map(|x| x)` — the `arr` is already cloned when extracted from DataType. Then each element is cloned again in the callback.

359. **`each` method returns Null, not the original array** — Unlike `Array.forEach` in JS which returns undefined, `each` could usefully return the original array for chaining.

360. **`enumerate` returns `[index, item]` arrays** — Should return `[int64, T]` tuples, but MAGI has no tuple type. The inner arrays are always 2-element.

361. **`chunk(0)` and `chunk(-1)` — chunk size validation** — chunk size ≤ 0 returns an error, which is correct. But the error message says "positive integer" without specifying the constraint.

362. **`window` method is missing** — `chunk` exists but `window` (sliding window) is not in `try_eval_hof_method`. It's listed as an OperationType but not a direct method.

363. **`flatten` method is missing from direct methods** — Must use `flat_map(|x| x)` instead.

364. **`unique` method is missing from direct methods** — Must use OperationType dispatch.

365. **`reverse` method is missing from direct array methods** — Must use OperationType dispatch.

---

## Round 13: FullEvaluator Specific Issues (Items 366–400)

366. **`eval_string` scans for matches twice in replace** — `s.matches(&from).count()` then `s.replace(&from, &to)`. Could do a single pass.

367. **`eval_array` ArraySort uses `partial_cmp` which fails on NaN** — Sorting `[1.0, NaN, 2.0]` produces undefined ordering since NaN has no partial order.

368. **`eval_regex` compiles regex on every call** — `regex::Regex::new(pattern)` is called fresh each time. A simple LRU cache (e.g., 64 entries) would dramatically speed up repeated regex operations.

369. **`eval_http_client` has no connection pooling** — Each HTTP request creates a new `ureq::Agent`. The agent should be cached or shared.

370. **`eval_compression` decompression bomb limit is per-call** — Compress → decompress → compress → decompress bypasses the 64MB limit by splitting into multiple steps.

371. **`eval_cert` CertGenerate creates a new CA for each call** — Self-signed cert generation is expensive; caching the CA key would speed up batch cert generation.

372. **`eval_network` TCP operations use blocking I/O** — `TcpStream::connect` blocks the entire thread. No timeout configuration.

373. **`eval_network` WebSocket uses `native-tls`** — Not `rustls`. Pulls in OpenSSL dependency on Linux.

374. **`eval_time` Sleep blocks the interpreter thread** — `std::thread::sleep()` halts all execution. Should use the cancel token to allow interruption.

375. **`eval_random` uses `rand::rng()` (thread-local RNG)** — Not cryptographically secure for `RandomBytes` used in security contexts.

376. **`eval_filesystem` FsRead has no encoding parameter** — Always reads as UTF-8. Binary files fail.

377. **`eval_filesystem` FsWrite creates parent directories** — Wait, it doesn't! `FsWrite` to a non-existent directory fails instead of creating parents.

378. **`eval_env_and_path` PathJoin doesn't normalize** — `path_join("/a/b", "../c")` returns `/a/b/../c` instead of `/a/c`.

379. **`eval_json` JsonGet supports dot-notation paths** — `json_get(data, "a.b.c")` navigates nested objects. But what if a key literally contains a dot?

380. **`eval_serialization` TomlParse doesn't handle TOML dates** — TOML datetime types are converted to strings, losing type information.

381. **`eval_stats` Percentile calculation may panic** — Division by zero if the array is empty and percentile is requested.

382. **`eval_fmt` FmtBytes uses SI prefixes** — 1024 bytes = "1.00 KB" but some contexts expect "1.00 KiB" (IEC). No configuration.

383. **`eval_sort_and_collection` SetIntersection clones both sets** — Creates two HashSets from arrays, then intersects. O(n+m) space.

384. **`eval_uuid` UuidParse is lenient** — Accepts non-standard UUID formats without validation.

385. **`eval_encoding` HmacSha256 doesn't constant-time compare** — The operation computes HMAC but comparing the result uses standard `==`.

386. **`eval_text` TextWrap may split mid-word** — Uses `textwrap::fill` which can split words if they exceed the line width.

387. **`num_binop` helper doesn't preserve type for mixed operations** — `Int32(5) + Float64(3.0)` returns Float64. But `Uint32(5) + Int32(3)` returns Int64 (widens both to i64). The widening rules are implicit.

388. **`num_cmp` helper has three separate closures** — `num_cmp(&a, &b, |x, y| x > y, |x, y| x > y, |x, y| x > y)` — the three closures are identical for most comparisons. Redundant.

389. **`is_truthy` helper duplicates `DataType::to_bool`** — There's `is_truthy()` in magi.rs AND `DataType::to_bool()` in types/mod.rs. They may diverge.

390. **`read_http_body` truncates without notification** — If the response exceeds MAX_STRING_OUTPUT, it's truncated silently.

391. **`validate_url_with_dns` does synchronous DNS resolution** — Blocks the thread on DNS lookup. No timeout.

392. **Connection registry values are `Box<dyn Any + Send>`** — No `Sync` bound, so connections can't be shared across threads.

393. **`conn_store` doesn't check for existing ID** — If the same UUID is generated twice (astronomically unlikely), it silently overwrites.

394. **`get_port` duplicates logic for Int32, Int64, Uint32, Uint64** — Four nearly-identical match arms for port validation.

395. **HTTP agent timeout is hardcoded** — No way for MAGI scripts to configure request timeout.

396. **`eval_reflect` ReflectInspect has MAX_INSPECT_OUTPUT = 1MB** — But this constant is defined in magi.rs, not shared with the lib. Inconsistent limits.

397. **`eval_control_flow` Assert uses `is_truthy` not strict Bool** — `assert(1)` passes because 1 is truthy. But the interpreter's built-in `assert()` requires strict Bool. Inconsistent.

398. **`eval_bitwise` operations only work on Int64** — `BitAnd` on Int32 values first converts to i64, losing the original type.

399. **`eval_math` Power with float exponent on integer base** — `pow(2, 0.5)` converts 2 to float and returns float. But `2 ** 0.5` in the interpreter might dispatch differently.

400. **Sleep with cancel token not checked** — `std::thread::sleep(duration)` in eval_time ignores the interpreter's cancel token. A long sleep can't be interrupted.

---

## Round 14: Edge Cases & Corner Cases (Items 401–430)

401. **Empty program execution** — `magi run empty.magi` where the file is empty. Should succeed with null output, but may behave differently with/without a main function.

402. **File with only comments** — `// just a comment` should parse successfully with zero statements.

403. **Unicode in string interpolation** — `f"emoji: {'\u{1F600}'}"` — does the interpolation handle multi-byte characters correctly?

404. **Very long identifier names** — A 10MB identifier name could exhaust memory during parsing.

405. **Deeply nested match expressions** — `match x { 1 => match y { 2 => match z { ... } } }` — stack overflow in compilation.

406. **Circular module definitions** — `mod a { use b::*; } mod b { use a::*; }` — infinite loop in module resolution.

407. **Self-referential struct** — `struct Node { value: int64, next: Node }` — no way to express this since structs are just maps.

408. **Empty enum** — `enum Empty {}` — should this be allowed? What does `match e { }` do?

409. **Single-variant enum** — `enum Single { Only }` — match is trivially exhaustive.

410. **Map with integer-like string keys** — `{"1": "one"}["1"]` works, but `{"1": "one"}[1]` may fail since index is Int64 not String.

411. **Negative zero** — `-0.0 == 0.0` is true in IEEE 754. Does MAGI handle this consistently?

412. **Integer overflow in range** — `0..9223372036854775807` creates a 9.2 quintillion element array. The 10M limit catches this, but the error message shows the huge number.

413. **String with null bytes** — `"hello\0world"` — null bytes in strings could cause issues with C-interop or WASM string handling.

414. **Map key ordering after serialization roundtrip** — `{"b": 1, "a": 2}` serialized to JSON and back loses BTreeMap ordering (JSON objects are unordered).

415. **Float64 precision in display** — `0.1 + 0.2` displays as `0.30000000000000004`. No formatting control.

416. **Large integer in float context** — `to_float64(9007199254740993)` loses precision (> 2^53). No warning.

417. **Recursive data structures** — `let mut x = []; x = array_push(x, x);` — works because x is cloned, creating `[[]]` not a circular reference. But it's confusing.

418. **Empty string split** — `"".split(",")` returns `[""]` (one empty element), not `[]`. Matches Rust behavior but may surprise users.

419. **Type alias shadowing built-in** — `type string = int64;` — does the type checker accept this?

420. **Variable named same as type** — `let int64 = "hello"` — is `int64` a valid variable name?

421. **Function named same as built-in** — `fn len(x) { x }` — shadows built-in `len`. Is there a warning?

422. **Test name with special characters** — `test "a \"quoted\" test" { ... }` — does the string escaping work in test names?

423. **Break with value in for loop** — `for x in arr { break 42; }` — the for loop returns 42.

424. **Continue in while loop doesn't re-evaluate condition in same iteration** — `while cond { if x { continue; } ... }` — skips to next iteration check.

425. **Return from try/catch finally block** — `try { } catch { } finally { return 42; }` — the return overrides both try and catch results. Linter warns (W212) but it's allowed.

426. **Throw non-string value** — `throw 42` — the value is wrapped in InterpError::ThrownError. Catch receives it as-is (Int64(42)), not as a string.

427. **Pattern matching on map with extra keys** — `match m { {x, y} => ... }` — does this match maps with additional keys beyond x and y?

428. **Spread in map literal** — `{...old_map, key: value}` — is this supported? Not in the AST definition (Literal::Map only has Vec<(String, Expression)>).

429. **Multi-line string with interpolation** — `f"""hello {name}"""` — does interpolation work in triple-quoted strings?

430. **REPL state persistence with errors** — If an error occurs mid-expression in the REPL, is the state rolled back or partially committed?

---

## Round 15: Ecosystem & Integration Gaps (Items 431–460)

431. **No package version constraints** — `magi.toml` dependency declarations have `path` but no `version` field for compatibility checking.
432. **No remote package dependencies** — Only local path dependencies supported. No `git` or `registry` sources.
433. **No lockfile for dependencies** — No `magi.lock` to pin dependency versions.
434. **No semver-aware dependency resolution** — Version module exists but isn't used in dependency resolution.
435. **No build scripts** — No pre-build or post-build hooks in `magi.toml`.
436. **No conditional dependencies** — No `[target.'cfg(...)'.dependencies]` equivalent.
437. **No workspace-level commands** — `magi test` only works on single files, not entire workspaces.
438. **No code coverage for MAGI scripts** — No way to measure which lines of a MAGI program are executed.
439. **No dead code detection across files** — Functions only used by external packages aren't detected as used.
440. **No cross-compilation for WASM from non-Linux** — `.cargo/config.toml` only configures Linux cross-compilation targets.
441. **No editor plugin for any editor** — No VSCode extension, Neovim plugin, or Emacs mode.
442. **No tree-sitter grammar** — No tree-sitter parser for syntax highlighting in modern editors.
443. **No language server auto-discovery** — `magi lsp` must be manually configured in editor settings.
444. **No debug adapter protocol (DAP) server** — Debug state/commands exist but no DAP transport.
445. **No MAGI-to-JavaScript transpiler** — Only WASM compilation target.
446. **No C FFI** — Can't call C libraries from MAGI.
447. **No WASM host function binding** — Can't define custom host functions for WASM plugins.
448. **No documentation generation** — No `magi doc` command to generate HTML/Markdown docs.
449. **No code formatting CI check** — No `magi fmt --check` integration with CI pipelines.
450. **No pre-commit hooks** — No `.pre-commit-config.yaml` for MAGI projects.
451. **No GitHub Actions** — No CI workflow for building/testing MAGI projects.
452. **No Docker image** — No official Docker image for running MAGI.
453. **No Homebrew formula** — No `brew install magi`.
454. **No npm/cargo publish workflow** — No publish pipeline for releases.
455. **No telemetry/crash reporting** — No opt-in crash reports for improving the language.
456. **No backward compatibility tests** — No test suite that verifies old MAGI programs still work on new versions.
457. **No deprecation mechanism** — No `#[deprecated]` attribute or runtime warning system.
458. **No edition system** — No way to opt into new syntax while maintaining old behavior.
459. **No feature flags in MAGI source** — No `#[feature(name)]` or similar.
460. **No stable ABI for WASM modules** — WASM module format may change between versions without warning.

---

## Summary

| Round | Focus | Items |
|-------|-------|-------|
| 1 | Bugs & Correctness | 1–30 |
| 2 | Performance | 31–60 |
| 3 | Missing Language Features | 61–95 |
| 4 | Missing Stdlib Operations | 96–130 |
| 5 | Error Handling & Diagnostics | 131–160 |
| 6 | WASM Compiler Gaps | 161–195 |
| 7 | LSP & Tooling | 196–225 |
| 8 | Code Quality & Maintenance | 226–255 |
| 9 | Security & Robustness | 256–280 |
| 10 | UX & Ergonomics | 281–310 |
| 11 | Deep Code-Level Bugs | 311–340 |
| 12 | Interpreter Method Dispatch | 341–365 |
| 13 | FullEvaluator Issues | 366–400 |
| 14 | Edge Cases & Corner Cases | 401–430 |
| 15 | Ecosystem & Integration | 431–460 |

**Total: 460 findings across 15 audit rounds.**

### Top 15 Most Impactful Items

1. **#311** `call_function` replaces entire scope — global vars inaccessible in functions
2. **#31** O(n²) array building pattern — fundamental performance issue
3. **#161** No closure compilation in WASM — blocks real WASM usage
4. **#256** No filesystem sandboxing — security gap
5. **#260** Regex ReDoS vulnerability — security issue
6. **#35** DataType::clone is deep for containers — pervasive perf issue
7. **#315** Short-circuit &&/|| requires strict Bool — diverges from most dynamic languages
8. **#368** Regex compiled fresh on every call — massive perf hit
9. **#341-347** 6x code duplication per numeric method — maintenance burden
10. **#133** No stack traces in errors — debugging is hard
11. **#316** exec_block doesn't push scope — variable leaking
12. **#46** HashMap allocation per operation — hot path perf
13. **#180** No string operations in WASM — blocks WASM usability
14. **#397** Inconsistent assert behavior between interpreter and evaluator
15. **#389** Duplicated `is_truthy`/`to_bool` — divergence risk
