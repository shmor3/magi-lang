# MAGI Syntax — Open Gaps

Unresolved syntax questions that must be answered before implementation.

## 1. For Loop Syntax

What forms of `for` loop exist?

```magi
// For-in (confirmed):
for x in [1, 2, 3] { println(x) }

// C-style — keep or remove?
for (let i = 0; i < 10; i += 1) { println(i) }

// Range:
for i in 0..10 { println(i) }

// Destructuring in for:
for [key, value] in entries { println(f"{key}: {value}") }
```

**Decision needed:** Keep C-style for? Or only `for x in iterable`?

## 2. Destructuring

Does destructuring work with `const` and `let`?

```magi
const [a, b, c] = [1, 2, 3]
let [x, ...rest] = [1, 2, 3, 4]
const {name, age} = person
let {host, port} = config
```

**Decision needed:** Confirm destructuring works with both `const` and `let`.

## 3. Map Literal and Type Syntax

```magi
// Literal:
const m = {"key": "value", "count": 42}

// Type annotation — which?
func process(data map[string]int) { ... }
func process(data {string: int}) { ... }
func process(data Map<string, int>) { ... }
```

**Decision needed:** What is the map type syntax?

## 4. Array Type Syntax

```magi
// Literal:
const arr = [1, 2, 3]

// Type annotation — which?
func sum(items [int]) -> int { ... }
func sum(items []int) -> int { ... }
func sum(items Array<int>) -> int { ... }
```

**Decision needed:** `[int]`, `[]int`, or `Array<int>`?

## 5. Multi-Return Type Annotation

```magi
// Which syntax?
func divide(a int, b int) -> (int, string) { ... }
func divide(a int, b int) -> int, string { ... }
```

**Decision needed:** Parens or no parens for multi-return types?

## 6. Primitive Type Names

```magi
// Lowercase (like Go):
func add(a int, b int) -> int
func pi() -> float
func name() -> string
func ok() -> bool

// Or mixed (like current MAGI):
func add(a Int64, b Int64) -> Int64
func pi() -> Float64
```

**Decision needed:** Lowercase `int`/`float`/`string`/`bool` or capitalized?

## 7. Number Type Granularity

Current MAGI has: `Int32`, `Int64`, `Uint32`, `Uint64`, `Float32`, `Float64`.

Options:
- **A)** Keep all six — explicit control
- **B)** Just `int` (64-bit) and `float` (64-bit) — simple, with `int32`/`float32` available when needed
- **C)** Just `int` and `float` — no 32-bit types

**Decision needed:** How many number types?

## 8. Match Arm vs Arrow Function Ambiguity

`=>` is used for both:

```magi
// Arrow function:
const double = x => x * 2

// Match arm:
match color {
    Color::Red => "red",
    Color::Blue => "blue",
}
```

Is there ambiguity? `x => x * 2` inside a match arm — is `x` a pattern or a lambda parameter?

**Decision needed:** Confirm no ambiguity, or change one of the syntaxes.

## 9. Comments

```magi
// Line comment
/* Block comment */
```

**Decision needed:** Confirm both stay. Doc comments? `///` or `/** */`?

## 10. Entry Point

```magi
// Current — explicit call:
func main() { ... }
main()

// Alternative — auto-main:
func main() { ... }
// main() called automatically if defined
```

**Decision needed:** Explicit `main()` call or auto-invoke?

## 11. Struct Update Syntax

```magi
const default_config = Config { host: "localhost", port: 8080, debug: false }
const prod = Config { ...default_config, port: 443, debug: false }
```

**Decision needed:** Confirm `...spread` in struct literals stays.

## 12. Function Parameter Mutability

```magi
func process(items [int]) {
    // Can I mutate items here?
    items.push(42)  // allowed or error?
}
```

**Decision needed:** Are function parameters `const` (immutable) or `let` (mutable) by default?
