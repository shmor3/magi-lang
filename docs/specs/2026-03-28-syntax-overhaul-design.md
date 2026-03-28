# MAGI Syntax Overhaul Design

**Date**: 2026-03-28
**Status**: Draft
**Goal**: Make MAGI easy to learn, write, read, understand, maintain, and ship.

## Design Principle

Every syntax choice must pass this test: can a developer coming from any mainstream language read this code and understand it without a manual?

## Summary of Changes

| Area | Before | After |
|------|--------|-------|
| Print | `output x;` (statement) | `print(x)` / `println(x)` (expression, returns argument) |
| Imports | `use std::math::*;` | `import std.math` — qualified: `math.sin()` |
| Functions | `fn` | `func` |
| Enum display | Shows `{__enum: Color, __variant: Red}` | Auto-formats: `Color::Red` |
| Enum numbering | No iota | `= 0` with auto-increment |
| Errors | try/catch/throw/Result/?/Option | Multi-return `val, err = f()` |
| Methods | `impl Type { fn method(self) }` | `func Type.method(self)` — dot receiver |
| Interfaces | `trait` + explicit `impl Trait for Type` | `interface` + implicit satisfaction |
| Mutability | `let mut x` | `const` immutable, `let` mutable |
| Operator overload | `__add__`, `__str__` dunders | Named interfaces: `Add`, `Display`, `Equal` |
| Composition | None | Removed — `\|>` pipe is sufficient |

## Removed

- `output` — replaced by `print()` / `println()`
- `throw` — replaced by `return null, "error"`
- `try` / `catch` / `finally` — no exception handling
- `impl` — dot receiver functions
- `trait` — replaced by `interface`
- `use` — replaced by `import`
- `let mut` — `let` is now mutable, `const` is immutable
- `fn` — replaced by `func`
- `__dunder__` methods — replaced by named interfaces
- `>>` / `<<` composition — `|>` pipe is sufficient

## Added

- `import` — module imports
- `interface` — replaces `trait`
- `func` — replaces `fn`

## 1. Print as Expression

```magi
println("hello")              // prints "hello\n", returns "hello"
print("no newline")           // prints, returns value
const x = println(compute())  // prints result, x holds the result
```

- `print(val)` — no newline, returns `val`
- `println(val)` — with newline, returns `val`
- Both accept any type

## 2. Import System

```magi
import std.math
import std.fs
import std.{net, json, yaml}    // multi-import
import std.math as m            // alias
import canvas                   // package
import ./util                   // relative
```

Rules:
- Last segment becomes the namespace: `math.sin()`, `fs.read()`
- No wildcard imports — always qualified
- No selective imports — import the module, use what you need

```magi
import std.math
import std.{fs, json}

const angle = math.sin(3.14)
let data, err = fs.read("config.json")
if err { return err }
```

## 3. Functions — `func`

```magi
func add(a, b) { a + b }

func greet(name string) -> string {
    f"Hello, {name}!"
}
```

`func` is familiar to Go, Swift, and Kotlin developers. Reads as English.

## 4. Dot Receiver Methods

```magi
struct Vec2 { x: float, y: float }

func Vec2.length(self) {
    math.sqrt(self.x * self.x + self.y * self.y)
}

func Vec2.scale(self, factor) {
    Vec2 { x: self.x * factor, y: self.y * factor }
}
```

Rules:
- `func Type.method(self, args)` — dot syntax, explicit `self`
- No `impl` blocks — each method is a standalone function
- Works on structs, enums, any type

```magi
const a = Vec2 { x: 3.0, y: 4.0 }
println(a.length())      // 5.0
println(a.scale(2.0))    // Vec2 { x: 6.0, y: 8.0 }
```

## 5. Operator Overloading via Interfaces

Operators are defined by implementing named interfaces. Every name is a complete English word.

```magi
interface Add {
    func add(self, other) -> self
}

interface Display {
    func display(self) -> string
}

interface Equal {
    func equal(self, other) -> bool
}

interface Compare {
    func compare(self, other) -> int
}

interface Iterable {
    func iterate(self) -> Iterator
}

interface Callable {
    func call(self, args) -> any
}

interface Index {
    func index(self, key) -> any
}

interface Length {
    func length(self) -> int
}

interface Contains {
    func contains(self, key) -> bool
}
```

To make `+` work on Vec2, implement `Add`:

```magi
func Vec2.add(self, other) {
    Vec2 { x: self.x + other.x, y: self.y + other.y }
}

func Vec2.display(self) {
    f"({self.x}, {self.y})"
}

func Vec2.equal(self, other) {
    self.x == other.x && self.y == other.y
}
```

Usage — the compiler maps operators to interface methods:

```magi
const a = Vec2 { x: 1.0, y: 2.0 }
const b = Vec2 { x: 3.0, y: 4.0 }
println(a + b)       // calls a.add(b) → (4.0, 6.0)
println(a == b)      // calls a.equal(b) → false
println(a)           // calls a.display() → (1.0, 2.0)
```

Operator → interface mapping (all plain English, no abbreviations):

| Operator | Interface | Method |
|----------|-----------|--------|
| `+` | `Add` | `add(self, other)` |
| `-` | `Subtract` | `subtract(self, other)` |
| `*` | `Multiply` | `multiply(self, other)` |
| `/` | `Divide` | `divide(self, other)` |
| `%` | `Modulo` | `modulo(self, other)` |
| `==` | `Equal` | `equal(self, other)` |
| `<` `>` `<=` `>=` | `Compare` | `compare(self, other) -> int` |
| `-x` (unary) | `Negate` | `negate(self)` |
| `[]` | `Index` | `index(self, key)` |
| `[]=` | `SetIndex` | `set_index(self, key, value)` |
| `for..in` | `Iterable` | `iterate(self)` |
| `println()` | `Display` | `display(self)` |
| `()` | `Callable` | `call(self, args)` |
| `len()` | `Length` | `length(self)` |
| `in` | `Contains` | `contains(self, key)` |

## 6. Interfaces (Implicit Satisfaction)

Any type with matching methods satisfies the interface automatically.

```magi
interface Shape {
    func area(self) -> float
    func perimeter(self) -> float
}

struct Circle { radius: float }

func Circle.area(self) { 3.14159 * self.radius * self.radius }
func Circle.perimeter(self) { 2.0 * 3.14159 * self.radius }

// Circle satisfies Shape — no declaration needed

func print_shape(s Shape) {
    println(f"area={s.area()}, perimeter={s.perimeter()}")
}

print_shape(Circle { radius: 5.0 })
```

Embedding:
```magi
interface Reader {
    func read(self, n int) -> ([]byte, string)
}

interface Writer {
    func write(self, data []byte) -> (int, string)
}

interface ReadWriter {
    Reader
    Writer
}
```

## 7. Enums

### Auto-Display

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
    East,
    South,
    West,
}
```

### Methods on Enums

```magi
func Color.display(self) {
    match self {
        Color::Red => "red",
        Color::Green => "green",
        Color::Blue => "blue",
        Color::Rgb(r, g, b) => f"#{r:02x}{g:02x}{b:02x}",
    }
}
```

## 8. Error Handling — Errors Are Values

```magi
func read_config(path) {
    let text, err = fs.read(path)
    if err { return null, f"read failed: {err}" }

    let config, err = json.parse(text)
    if err { return null, f"parse failed: {err}" }

    config, null
}

func main() {
    let config, err = read_config("app.json")
    if err {
        println(f"error: {err}")
        return
    }
    println(config)
}
```

Rules:
- Fallible functions return `value, error`
- Error is `null` on success
- Check with `if err` (truthiness)
- No try/catch/throw/Result/Option/?
- `??` and `?.` stay for null convenience

## 9. Mutability

```magi
const x = 5        // immutable — cannot reassign
let count = 0      // mutable — can reassign
count = count + 1  // ok
x = 10             // compile error
```

Matches JavaScript: `const` is frozen, `let` is mutable.

## 10. What Stays Unchanged

- `const` / `let` for bindings
- `struct` for types
- `enum` for enumerated types
- `if` / `else` as expressions
- `for` / `while` / `loop`
- `match` with pattern matching (exhaustive enforcement)
- `break` / `continue` / `return`
- `f"..."` string interpolation
- `|>` pipe operator
- `[x for x in arr if cond]` comprehensions
- `..` and `..=` range operators
- `??` null coalesce and `?.` optional chain
- `async` / `await` / `spawn` / `select`
- `defer`
- `...` spread operator
- Generics with `<T>` syntax
- Lambdas `|x| x * 2`
- `_` partial application in function calls

## 11. Full Example

```magi
import std.{math, fs, json}

struct Config {
    host: string,
    port: int,
    debug: bool,
}

func Config.address(self) {
    f"{self.host}:{self.port}"
}

func Config.display(self) {
    f"Config({self.address()})"
}

func load_config(path) {
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

func LogLevel.display(self) {
    match self {
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    }
}

func log(level LogLevel, msg string) {
    if level == LogLevel::Debug { return }
    println(f"[{level}] {msg}")
}

func main() {
    let config, err = load_config("config.json")
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

## 12. Implementation Order

1. **Lexer** — Add `import`, `interface`, `func` tokens. Remove `output`, `throw`, `try`, `catch`, `finally`, `impl`, `trait`, `use`, `fn` tokens.
2. **Parser** — Parse `import std.path`, `func` keyword, dot receivers, `interface`, multiple return values.
3. **AST** — New nodes for imports, dot receivers, interfaces. Remove try/catch/throw/impl/trait.
4. **Interpreter** — `print()`/`println()` builtins. Multi-return. Module namespacing. Dot receiver dispatch. Implicit interface satisfaction. Named operator interfaces. Clean enum display. Remove exception machinery.
5. **Type checker** — Interface satisfaction. Receiver validation. Multi-return types. Exhaustive match enforcement.
6. **Tests** — Update all tests to new syntax. Add new tests.
7. **Docs** — Update spec.md, stdlib.md, examples.
