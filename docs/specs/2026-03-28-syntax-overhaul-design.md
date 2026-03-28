# MAGI Syntax Overhaul Design

**Date**: 2026-03-28
**Status**: Draft
**Goal**: Comprehensive syntax overhaul to give MAGI a clean, distinct identity before self-hosting.

## Summary of Changes

| Area | Before | After |
|------|--------|-------|
| Print | `output x;` (statement) | `print(x)` / `println(x)` (expression, returns argument) |
| Imports | `use std::math::*;` | `import std.math` — qualified access: `math.sin()` |
| Enum display | Shows `{__enum: Color, __variant: Red}` | Auto-formats: `Color::Red` |
| Enum numbering | No iota | Simple enums support `= 0` with auto-increment |
| Errors | try/catch/throw/Result/?/Option | Multi-return `val, err = f()` — errors are values |
| Methods | `impl Type { fn method(self) }` | `fn Type.method(self)` — dot receiver syntax |
| Interfaces | `trait` + explicit `impl Trait for Type` | `interface` + implicit satisfaction |
| Mutability | `let mut x` | `const` immutable, `let` mutable |
| Composition | None | `>>` and `<<` function composition |
| Partial apply | `_` only in pipes | `_` placeholder in any function call |
| Match | Warning on non-exhaustive | Error on non-exhaustive |

## Removed Keywords

- `output` — replaced by `print()` / `println()` builtin functions
- `throw` — replaced by `return null, "error message"`
- `try` — removed (no exception handling)
- `catch` — removed
- `finally` — removed
- `impl` — removed (dot receiver functions replace impl blocks)
- `trait` — replaced by `interface`
- `use` — replaced by `import`
- `let mut` — `let` is now mutable by default, `const` for immutable

## Added Keywords

- `import` — module imports
- `interface` — replaces `trait`

## 1. Print as Expression

`print()` and `println()` are builtin functions that print their argument and return it.

```magi
println("hello")              // prints "hello\n", returns "hello"
print("no newline")           // prints "no newline", returns "no newline"
let x = println(compute())   // prints result, x holds the result
```

- `print(val)` — prints without newline, returns `val`
- `println(val)` — prints with newline, returns `val`
- Both accept any type, convert to string automatically
- `output` keyword is removed from the language entirely

## 2. Import System

Modules are imported by dotted path. The last segment becomes the namespace.

```magi
import std.math
import std.fs
import std.{net, json, yaml}    // multi-import
import std.math as m            // alias: m.sin(3.14)
import canvas                   // package import
import ./util                   // relative import
```

### Rules

- `import path.to.module` — imports module, last segment is the name
- `import path.{a, b, c}` — multi-import from same base
- `import path.module as alias` — aliased import
- No wildcard imports — always qualified access
- No selective function imports — import the module, use what you need
- Standard library: `import std.math`, `import std.fs`, etc.
- Packages: `import canvas`, `import keypress`
- Relative: `import ./util`, `import ../types`

### Usage

```magi
import std.math
import std.fs
import std.json

const angle = math.sin(3.14)

let data, err = fs.read("config.json")
if err { return err }

let config, err = json.parse(data)
if err { return err }
```

## 3. Enum Improvements

### Auto-Display

Enums display their variant name cleanly.

```magi
enum Color {
    Red,
    Green,
    Blue,
    Rgb(int, int, int),
}

println(Color::Red)              // Color::Red
println(Color::Rgb(255, 0, 0))   // Color::Rgb(255, 0, 0)
```

### Iota-Style Numbering

```magi
enum Direction {
    North = 0,
    East,       // 1
    South,      // 2
    West,       // 3
}

enum HttpStatus {
    Ok = 200,
    NotFound = 404,
    ServerError = 500,
}
```

### Receiver Methods on Enums

```magi
fn Color.__str__(self) {
    match self {
        Color::Red => "red",
        Color::Green => "green",
        Color::Blue => "blue",
        Color::Rgb(r, g, b) => f"#{r:02x}{g:02x}{b:02x}",
    }
}
```

### Construction

Enum construction uses `::` — unambiguous since imports use dot syntax.

```magi
const c = Color::Red
const c2 = Color::Rgb(255, 128, 0)

match c2 {
    Color::Rgb(r, g, b) => println(f"rgb({r}, {g}, {b})"),
    _ => println("solid color"),
}
```

## 4. Error Handling — Errors Are Values

Functions return `value, error` tuples. No exceptions, no magic.

### Basic Pattern

```magi
fn read_config(path) {
    let text, err = fs.read(path)
    if err { return null, f"read failed: {err}" }

    let config, err = json.parse(text)
    if err { return null, f"parse failed: {err}" }

    config, null
}

fn main() {
    let config, err = read_config("app.json")
    if err {
        println(f"error: {err}")
        return
    }
    println(f"loaded: {config}")
}
```

### Rules

- Functions that can fail return `value, error`
- Error is `null` on success, a string or value on failure
- Caller checks with `if err` (truthiness — non-null is truthy)
- Multiple return values are native via comma separation
- No `try`/`catch`/`finally`/`throw`
- No `Result`/`Ok`/`Err`
- No `Some`/`None`
- No `?` operator
- `??` (null coalesce) and `?.` (optional chain) stay for null convenience

### Stdlib Functions

All fallible stdlib functions use multi-return:

```magi
let data, err = fs.read("file.txt")
let conn, err = net.connect("localhost:8080")
let parsed, err = json.parse(text)
```

Infallible functions return a single value:

```magi
const n = math.sqrt(16.0)
const s = text.to_upper("hello")
const id = uuid.v4()
```

## 5. Dot Receiver Functions (No `impl`)

Methods are functions with `Type.name` syntax and `self` parameter.

```magi
struct Vec2 { x: float, y: float }

fn Vec2.length(self) {
    math.sqrt(self.x * self.x + self.y * self.y)
}

fn Vec2.add(self, other) {
    Vec2 { x: self.x + other.x, y: self.y + other.y }
}

fn Vec2.__str__(self) {
    f"({self.x}, {self.y})"
}

fn Vec2.__add__(self, other) {
    Vec2 { x: self.x + other.x, y: self.y + other.y }
}
```

### Rules

- `fn Type.method(self, args) { body }` — defines a method
- `self` refers to the receiver instance
- Methods on any type: structs, enums, even basic types
- No `impl` blocks — each method is a standalone function
- Operator overloading uses dunder methods: `__add__`, `__sub__`, `__eq__`, `__str__`, `__index__`, `__iter__`, `__call__`

### Usage

```magi
const a = Vec2 { x: 3.0, y: 4.0 }
println(a.length())      // 5.0
println(a + a)           // (6.0, 8.0)
```

### Methods on Enums

```magi
enum Shape {
    Circle(float),
    Rect(float, float),
}

fn Shape.area(self) {
    match self {
        Shape::Circle(r) => 3.14159 * r * r,
        Shape::Rect(w, h) => w * h,
    }
}
```

## 6. Interfaces (Implicit Satisfaction)

Interfaces declare method signatures. Any type with matching methods satisfies the interface automatically.

```magi
interface Stringer {
    fn string(self) -> string
}

interface Shape {
    fn area(self) -> float
    fn perimeter(self) -> float
}

struct Circle { radius: float }

fn Circle.area(self) { 3.14159 * self.radius * self.radius }
fn Circle.perimeter(self) { 2.0 * 3.14159 * self.radius }
fn Circle.string(self) { f"Circle(r={self.radius})" }

// Circle satisfies both Shape and Stringer — no declaration needed

fn print_area(s Shape) {
    println(f"area = {s.area()}")
}

print_area(Circle { radius: 5.0 })   // area = 78.53975
```

### Rules

- `interface Name { fn method(self) -> ReturnType }` — declares an interface
- Types satisfy interfaces implicitly by having matching methods
- Interface values work as function parameters (dynamic dispatch)
- Empty interface `interface{}` matches any type
- No explicit `implements` declaration

### Embedding (Interface Composition)

```magi
interface Reader {
    fn read(self, n int) -> ([]byte, error)
}

interface Writer {
    fn write(self, data []byte) -> (int, error)
}

interface ReadWriter {
    Reader
    Writer
}
```

## 7. What Stays Unchanged

- `const` for immutable bindings (default, encouraged)
- `let` for mutable bindings
- `fn` for function definitions
- `struct` for type definitions
- `enum` for enumerated types (with improvements above)
- `if` / `else` as expressions
- `for` / `while` / `loop` loops
- `match` expressions with pattern matching
- `break` / `continue` / `return`
- `f"..."` string interpolation
- `|>` pipe operator
- `[x for x in arr if cond]` comprehensions
- `..` and `..=` range operators
- `??` null coalesce and `?.` optional chain
- `async` / `await` / `spawn` / `select`
- `defer`
- `...` spread operator
- Operator overloading via dunder methods
- Generics with `<T>` syntax

## 8. Functional Features

### Immutable by Default

`const` is immutable. `let` is mutable. Matches JavaScript/TypeScript.

```magi
const x = 5        // immutable — cannot reassign
let count = 0      // mutable — can reassign
count = count + 1  // ok
x = 10             // compile error
```

- `const` bindings are frozen after assignment — use by default
- `let` signals "this will change" — clear intent
- Function parameters are immutable by default

### Function Composition Operator `>>`

Compose functions into pipelines:

```magi
const process = parse >> validate >> transform
const result = process(input)

// Equivalent to:
const result = transform(validate(parse(input)))
```

- `f >> g` returns a new function that calls `f` then `g`
- Works with any single-argument functions
- Composes left to right (like `|>` but creates a reusable function)
- `<<` for right-to-left composition: `transform << validate << parse`

### Partial Application with `_`

Use `_` as a placeholder to create partially applied functions:

```magi
const add = fn(a, b) { a + b }
const add5 = add(5, _)       // returns fn(b) { 5 + b }
const double = mul(2, _)     // returns fn(b) { 2 * b }

println(add5(3))            // 8
println(double(7))          // 14

// Works with any function:
const is_even = mod(_, 2) >> eq(_, 0)
const evens = numbers |> filter(is_even)

// Multiple placeholders create multi-arg functions:
const between = fn(lo, x, hi) { x >= lo && x <= hi }
const teen = between(13, _, 19)    // fn(x) { x >= 13 && x <= 19 }
```

- `_` in a function call creates a new function with that argument open
- Multiple `_` create a function with multiple parameters (left to right)
- Works with `|>` pipe naturally: `data |> map(add(1, _))`
- Already exists in pipe expressions — now generalized to all calls

### Anonymous Functions

Two syntaxes — short lambdas and block functions:

```magi
// Short lambda (existing):
const double = |x| x * 2

// Block anonymous function (new):
const process = fn(x) {
    const cleaned = x.trim()
    const parsed = parse_int(cleaned)
    parsed * 2
}

// In higher-order functions:
items |> map(|x| x * 2)
items |> filter(fn(x) {
    const valid = x > 0
    const even = x % 2 == 0
    valid && even
})
```

### Exhaustive Match Enforcement

The compiler enforces that `match` covers all enum variants:

```magi
enum Direction { North, East, South, West }

let name = match dir {
    Direction::North => "north",
    Direction::East => "east",
    // compile error: non-exhaustive match — missing South, West
}

// Fix with wildcard or all variants:
let name = match dir {
    Direction::North => "north",
    Direction::East => "east",
    _ => "other",              // wildcard covers remaining
}
```

- All enum variants must be covered or a `_` wildcard must be present
- Compiler warns on redundant match arms
- Already partially implemented — now enforced as an error, not a warning

## 9. MAGI Identity

What makes MAGI distinct from any other language:

| Feature | MAGI | Go | Rust | Python |
|---------|------|-----|------|--------|
| Pattern matching | `match` with destructuring | No | Yes | Limited |
| Enums with data | Yes | No | Yes | No |
| String interpolation | `f"..."` | No | No | Yes |
| Pipe operator | `\|>` | No | No | No |
| Comprehensions | `[x for x in arr]` | No | No | Yes |
| Null operators | `??`, `?.` | No | No | No |
| Operator overload | Dunder methods | No | Traits | Dunder |
| Expressions everywhere | if/match/loop return values | No | Yes | Limited |
| Errors | Multi-return + truthiness | Multi-return | Result type | Exceptions |
| Methods | `fn Type.method(self)` | `fn (t T) method()` | `impl T { fn }` | `def method(self)` |
| Interfaces | Implicit satisfaction | Implicit | Explicit traits | Duck typing |
| Imports | `import std.math` | `import "path"` | `use path::*` | `import module` |
| Composition | `>>` / `<<` | No | No | No |
| Partial apply | `f(1, _)` | No | No | `functools.partial` |
| Immutability | `const` immutable, `let` mutable | `var` / `:=` | `let` / `let mut` | All mutable |

MAGI borrows the best ideas but combines them in its own way. The dot receiver syntax, expression-based control flow, pipe operator, and comprehensions are the signature features.

## 9. Full Example — New Syntax

```magi
import std.{math, fs, json}
import std.fmt

struct Config {
    host: string,
    port: int,
    debug: bool,
}

fn Config.address(self) {
    f"{self.host}:{self.port}"
}

fn Config.__str__(self) {
    f"Config({self.address()})"
}

interface Loadable {
    fn load(path string) -> (self, string)
}

fn load_json(path) {
    let text, err = fs.read(path)
    if err { return null, f"read: {err}" }

    let data, err = json.parse(text)
    if err { return null, f"parse: {err}" }

    const config = Config {
        host: data.host ?? "localhost",
        port: data.port ?? 8080,
        debug: data.debug ?? false,
    }
    config, null
}

enum LogLevel {
    Debug = 0,
    Info,
    Warn,
    Error,
}

fn LogLevel.__str__(self) {
    match self {
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    }
}

fn log(level LogLevel, msg string) {
    if level == LogLevel::Debug { return }
    println(f"[{level}] {msg}")
}

fn main() {
    let config, err = load_json("config.json")
    if err {
        log(LogLevel::Error, err)
        return
    }

    log(LogLevel::Info, f"starting {config}")

    const numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    const result = numbers
        |> filter(|n| n % 2 == 0)
        |> map(|n| n * n)
        |> reduce(0, |acc, n| acc + n)

    println(f"sum of even squares: {result}")
}

main()
```

## 10. Implementation Order

1. **Lexer** — Add `import`, `interface` tokens. Remove `output`, `throw`, `try`, `catch`, `finally`, `impl`, `trait`, `use` tokens. Parse dot receiver syntax `fn Type.method`.
2. **Parser** — Parse `import std.path`, multi-import `import std.{a, b}`, dot receiver functions, `interface` declarations, multiple return values.
3. **AST** — New node types for imports, dot receiver functions, interfaces. Remove try/catch/throw/impl/trait nodes.
4. **Interpreter** — `print()`/`println()` builtins returning their argument. Multiple return values. Module namespacing from imports. Dot receiver dispatch. Implicit interface satisfaction. Clean enum display. Remove exception machinery.
5. **Type checker** — Interface satisfaction checking. Receiver function validation. Multiple return value types.
6. **Evaluator (magi.rs)** — Update FullEvaluator for new syntax.
7. **Tests** — Update all 3,263 tests. Add new tests for new features.
8. **Docs** — Update spec.md, stdlib.md, examples, CLAUDE.md.
