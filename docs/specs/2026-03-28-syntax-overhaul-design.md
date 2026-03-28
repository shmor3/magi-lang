# MAGI Syntax — Final Specification

**Date**: 2026-03-28
**Status**: Final
**Goal**: Make MAGI easy to learn, write, read, understand, maintain, and ship.

**Design Principle**: Can a developer from any mainstream language read this code and understand it without a manual?

---

## Keywords

```
const  let  func  struct  enum  interface  type  import
if  else  for  in  while  match  select
break  continue  return  defer  spawn  await  async
true  false  null
```

**Removed from language**: `output`, `fn`, `throw`, `try`, `catch`, `finally`, `impl`, `trait`, `use`, `loop`, `unsafe`, `yield`, `move`, `ref`, `dyn`, `where`, `mod`, `pub`, `mut`, `Some`, `None`, `Ok`, `Err`

---

## Types

### Primitives

All lowercase:

```
int       // 64-bit signed integer (default)
float     // 64-bit floating point (default)
string    // UTF-8 string
bool      // true or false
byte      // unsigned 8-bit

int32     // 32-bit signed
uint32    // 32-bit unsigned
uint64    // 64-bit unsigned
float32   // 32-bit float
any       // any type (dynamic, no type checking)
```

### Composite Types

```
[]int                    // array of int
[]any                    // array of mixed types
map[string]int           // map with string keys, int values
map[string]any           // map with string keys, any values
(int, string)            // tuple
set[int]                 // set
[]byte                   // byte array
```

### Tuple Literals

```magi
const point = (3, 4)                // tuple of (int, int)
const pair = ("hello", 42)          // tuple of (string, int)
const (x, y) = point                // destructure
```

Tuples use parentheses. Multi-return values are tuples.

### Error Convention

Fallible functions return `(T, string)` — value and error string. Error is `null` on success.

```magi
func divide(a int, b int) -> (int, string) {
    if b == 0 { return 0, "division by zero" }
    a / b, null
}
```

The error type is always `string` (or `null`). Not a custom type.

### Type Aliases

```magi
type ID = int
type Handler = func(Request) -> Response
type Matrix = [][]float
type Node = { value: int, children: []Node }   // recursive allowed
```

---

## Bindings

```magi
const x = 5             // immutable — cannot reassign
let count = 0           // mutable — can reassign
count = count + 1       // ok
x = 10                  // compile error
```

- `const` — immutable (default, encouraged)
- `let` — mutable
- All variables must be initialized. `let x: int` without a value is a compile error.

### Destructuring

```magi
const [a, b, c] = [1, 2, 3]
let [head, ...rest] = [1, 2, 3, 4]
const {name, age} = person
const (x, y) = get_point()
```

Works with `const`, `let`, `for` loops, and function parameters.

---

## Functions

```magi
func add(a int, b int) -> int {
    a + b
}

func greet(name string) -> string {
    f"Hello, {name}!"
}

// Type annotations optional for dynamic usage:
func double(x) { x * 2 }
```

### Parameters

- Immutable by default
- Prefix with `let` to make mutable: `func process(let items []int) { items.push(42) }`

### Multiple Return Values

```magi
func divide(a int, b int) -> (int, string) {
    if b == 0 { return 0, "division by zero" }
    a / b, null
}

let result, err = divide(10, 3)
if err { return null, err }
```

### Arrow Functions (Lambdas)

```magi
const double = x => x * 2
const add = (a, b) => a + b
const process = (x) => {
    const cleaned = x.trim()
    parse_int(cleaned) * 2
}

numbers |> filter(x => x % 2 == 0) |> map(x => x * x)
```

---

## Methods — Dot Receivers

```magi
struct Vec2 { x: float, y: float }

func Vec2.length(self) -> float {
    math.sqrt(self.x * self.x + self.y * self.y)
}

func Vec2.scale(self, factor float) -> Vec2 {
    Vec2 { x: self.x * factor, y: self.y * factor }
}
```

- `func Type.method(self)` — dot syntax, explicit `self`
- Works on structs, enums, and primitives
- No `impl` blocks

### Methods on Primitives

```magi
func int.abs(self) -> int {
    if self < 0 { -self } else { self }
}

func string.words(self) -> []string {
    self.split(" ")
}

println((-5).abs())       // 5
println("hello world".words())  // ["hello", "world"]
```

---

## Interfaces

Implicit satisfaction. No `implements` declaration.

```magi
interface Shape {
    func area(self) -> float
    func perimeter(self) -> float
}

struct Circle { radius: float }

func Circle.area(self) -> float { 3.14159 * self.radius * self.radius }
func Circle.perimeter(self) -> float { 2.0 * 3.14159 * self.radius }

// Circle satisfies Shape automatically

func print_shape(s Shape) {
    println(f"area={s.area()}")
}
```

### Embedding

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

If embedded interfaces have conflicting methods, it's a compile error. Define your own to resolve.

### Multiple Bounds

```magi
func process<T: Display + Compare>(x T) { ... }
```

Use `+` to combine interface requirements.

---

## Operator Interfaces

Operators map to named interface methods. All plain English.

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
| `println()` | `Display` | `display(self) -> string` |
| `()` | `Callable` | `call(self, args)` |
| `len()` | `Length` | `length(self) -> int` |
| `in` | `Contains` | `contains(self, key) -> bool` |

```magi
func Vec2.add(self, other Vec2) -> Vec2 {
    Vec2 { x: self.x + other.x, y: self.y + other.y }
}

func Vec2.display(self) -> string {
    f"({self.x}, {self.y})"
}

const a = Vec2 { x: 1.0, y: 2.0 }
const b = Vec2 { x: 3.0, y: 4.0 }
println(a + b)     // (4.0, 6.0)
```

---

## Enums

### Simple

```magi
enum Direction {
    North = 0,
    East,       // 1
    South,      // 2
    West,       // 3
}
```

### With Data

```magi
enum Token {
    Number(float),
    String(string),
    Ident(string),
    Plus,
    Eof,
}
```

### Auto-Display

```magi
println(Direction::North)          // Direction::North
println(Token::Number(3.14))       // Token::Number(3.14)
```

Numbered enums don't show the number in display. Override with `display()`.

### Methods

```magi
func Token.display(self) -> string {
    match self {
        Token::Number(n) => f"{n}",
        Token::String(s) => f"\"{s}\"",
        Token::Ident(name) => name,
        Token::Plus => "+",
        Token::Eof => "EOF",
    }
}
```

---

## Control Flow

### If/Else (Expression)

```magi
const max = if a > b { a } else { b }
```

### For Loop

```magi
for x in [1, 2, 3] { println(x) }
for i in 0..10 { println(i) }
for i in 0..=10 { println(i) }         // inclusive
for [key, val] in entries { println(f"{key}: {val}") }
```

No C-style for loops. Use `for i in 0..n` instead.

### Labeled Break/Continue

```magi
'outer: for row in matrix {
    for cell in row {
        if cell == 0 { break 'outer }
    }
}
```

### While

```magi
while condition { ... }
while true { ... }      // infinite loop (replaces `loop`)
```

### Match (Exhaustive)

```magi
const name = match direction {
    Direction::North => "north",
    Direction::East => "east",
    Direction::South => "south",
    Direction::West => "west",
}
```

All enum variants must be covered or a `_` wildcard present. Enforced as compile error.

### Match Guards

```magi
const label = match n {
    _ if n % 15 == 0 => "FizzBuzz",
    _ if n % 3 == 0 => "Fizz",
    _ if n % 5 == 0 => "Buzz",
    _ => to_string(n),
}
```

Guards add conditions to match arms with `if`. A `_` with a guard is not exhaustive — a plain `_` must follow.

---

## Error Handling

Errors are values. Multi-return. No exceptions.

```magi
func read_config(path string) -> (Config, string) {
    let text, err = fs.read(path)
    if err { return null, f"read: {err}" }

    let data, err = json.parse(text)
    if err { return null, f"parse: {err}" }

    const config = Config {
        host: data.host ?? "localhost",
        port: data.port ?? 8080,
    }
    config, null
}

let config, err = read_config("app.json")
if err {
    println(f"error: {err}")
    return
}
```

Rules:
- Fallible functions return `(value, error)` — error is `null` on success
- Check with `if err` (truthiness: non-null is truthy)
- `??` null coalesce and `?.` optional chain stay for convenience
- Pipe `|>` only works on single values — handle errors first, then pipe

---

## Imports

```magi
import std.math
import std.{fs, json, net}
import std.math as m
import canvas
import ./util
```

- Last segment becomes namespace: `math.sin()`, `fs.read()`
- No wildcard imports
- Files are modules — no `mod` blocks

---

## Concurrency

```magi
const task = spawn fetch_data(url)
const result = await task

select {
    msg from inbox => println(msg),
    tick from timer => update(),
    _ => println("timeout"),
}
```

### Async Functions

```magi
async func fetch(url string) -> (string, string) {
    let resp, err = net.get(url)
    if err { return null, err }
    resp.body, null
}

let body, err = await fetch("https://api.example.com")
if err { return null, err }
```

`await` unwraps the future. Multi-return passes through.

### Defer

```magi
func process_file(path string) -> (string, string) {
    let file, err = fs.open(path)
    if err { return null, err }
    defer fs.close(file)      // unconditional cleanup on function exit

    let data, err = fs.read_all(file)
    if err { return null, err }
    data, null
}
```

Defer is for unconditional cleanup. It doesn't see error state.

---

## Generics

```magi
func max<T: Compare>(a T, b T) -> T {
    if a.compare(b) > 0 { a } else { b }
}

func filter_map<T, U>(items []T, f func(T) -> (U, bool)) -> []U {
    let result = []
    for item in items {
        const (val, ok) = f(item)
        if ok { result.push(val) }
    }
    result
}

struct Stack<T> { items: []T }

func Stack<T>.push(self, item T) { self.items.push(item) }
func Stack<T>.pop(self) -> T { self.items.pop() }
```

---

## Strings

```magi
const s = "hello"                     // regular string
const f = f"value: {x + 1}"          // interpolation
const m = """
    multiline
    string
"""                                    // multiline
const r = r"no\escape\here"          // raw string
```

---

## Comments

```magi
// line comment
/* block comment */
/// doc comment — generates documentation
```

---

## Semicolons

Optional. Newlines end statements.

```magi
const x = 5
const y = 10
println(x + y)

const a = 1; const b = 2; println(a + b)   // one-liner ok
```

---

## Visibility

Everything is public. Modules are the privacy boundary.

```magi
func parse(input string) { ... }       // public
func _tokenize(input string) { ... }   // convention: internal
```

---

## Attributes

```magi
#[test]
func test_addition() {
    assert(1 + 1 == 2)
}

#[deprecated("use new_func instead")]
func old_func() { ... }

#[ignore]
func slow_test() { ... }
```

---

## Struct Update (Spread)

```magi
const default = Config { host: "localhost", port: 8080, debug: false }
const prod = Config { ...default, port: 443 }
```

Spread works in arrays too:

```magi
const a = [1, 2, 3]
const b = [...a, 4, 5]    // [1, 2, 3, 4, 5]
```

---

## Mutability Rules

| Context | Default | Override |
|---------|---------|---------|
| `const x = ...` | Immutable binding, frozen fields | — |
| `let x = ...` | Mutable binding, mutable fields | — |
| Function params | Immutable | `func f(let x int)` for mutable |
| Struct fields | Follow the binding | `const s` = frozen, `let s` = mutable |

---

## Operator Precedence (high to low)

1. `()` `.` `[]` — call, field access, index
2. `-x` `!x` — unary
3. `*` `/` `%` — multiplicative
4. `+` `-` — additive
5. `..` `..=` — range
6. `<<` `>>` — bitshift
7. `&` — bitwise and
8. `^` — bitwise xor
9. `|` — bitwise or
10. `==` `!=` `<` `>` `<=` `>=` — comparison
11. `&&` — logical and
12. `||` — logical or
13. `??` — null coalesce
14. `|>` — pipe
15. `=>` — arrow function (lowest)
16. `=` `+=` `-=` etc. — assignment

---

## Implicit Returns

The last expression in a function body is the return value. No `return` keyword needed.

```magi
func add(a int, b int) -> int { a + b }          // returns a + b
func max(a int, b int) -> int {
    if a > b { a } else { b }                     // returns if-expression
}
```

Use `return` for early exits:

```magi
func divide(a int, b int) -> (int, string) {
    if b == 0 { return 0, "division by zero" }    // early return
    a / b, null                                     // implicit return
}
```

---

## Multi-Return Destructuring

No parentheses needed. Comma-separated names create separate bindings:

```magi
let result, err = divide(10, 3)     // two separate bindings
const a, b, c = get_three()         // three separate bindings
let _, err = do_something()         // discard first value
```

Parentheses allowed for clarity but not required:

```magi
const (x, y) = get_point()          // same as: const x, y = get_point()
```

---

## Pipe Operator

`|>` passes the left value as the first argument to the right function:

```magi
const result = data |> filter(x => x > 0) |> map(x => x * 2)
// equivalent to: map(filter(data, x => x > 0), x => x * 2)
```

Pipe to an arrow function for inline transforms:

```magi
const avg = items |> reduce(0, (a, b) => a + b) |> (sum => sum / items.length())
```

Pipe only works on single values. Handle errors before piping:

```magi
let data, err = fs.read("file.txt")
if err { return null, err }
const result = data |> parse() |> transform()
```

---

## Built-in Functions

Always available, no import needed:

```magi
println(value)           // print with newline, returns value
print(value)             // print without newline, returns value
len(collection)          // length of array, string, map, bytes
to_string(value)         // convert any value to string
parse_int(s)             // parse string to int (returns null on failure)
parse_float(s)           // parse string to float (returns null on failure)
typeof(value)            // returns type name as string
assert(condition)        // panic if false
```

---

## Built-in Methods on Primitives

### string

```magi
s.length()               // character count
s.split(delim)           // split into []string
s.trim()                 // strip whitespace
s.to_upper()             // uppercase
s.to_lower()             // lowercase
s.contains(sub)          // true if sub found
s.starts_with(prefix)    // prefix check
s.ends_with(suffix)      // suffix check
s.replace(old, new)      // replace all occurrences
s.chars()                // split into individual characters
s.bytes()                // convert to []byte
s.substring(start, end)  // slice
```

### array

```magi
arr.length()             // element count
arr.push(item)           // append (mutates if let, error if const)
arr.pop()                // remove last (mutates)
arr.map(f)               // transform each element
arr.filter(f)            // keep elements where f returns true
arr.reduce(init, f)      // fold into single value
arr.sort()               // sort in place
arr.sort_by(f)           // sort by key function
arr.reverse()            // reverse in place
arr.contains(item)       // true if item found
arr.find(f)              // first element where f is true
arr.flatten()            // flatten nested arrays
arr.join(sep)            // join into string
arr.slice(start, end)    // sub-array
arr.enumerate()          // returns [(index, value)]
arr.zip(other)           // pair elements
arr.chunk(n)             // split into groups of n
arr.group_by(f)          // group into map by key function
```

### map

```magi
m.keys()                 // []string of keys
m.values()               // []any of values
m.entries()              // [](string, any) pairs
m.contains(key)          // true if key exists
m.remove(key)            // remove key
m.merge(other)           // combine maps
```

---

## Standard Library Modules

Each module is imported with `import std.module`. Key modules:

```
std.math     — math.sqrt(), math.sin(), math.cos(), math.pi, math.e
std.fs       — fs.read(), fs.write(), fs.open(), fs.close(), fs.exists(), fs.list()
std.json     — json.parse(), json.stringify()
std.net      — net.get(), net.post(), net.listen(), net.connect()
std.time     — time.now(), time.sleep(), time.format()
std.crypto   — crypto.sha256(), crypto.aes_encrypt()
std.yaml     — yaml.parse(), yaml.stringify()
std.csv      — csv.parse(), csv.stringify()
std.toml     — toml.parse(), toml.stringify()
std.regex    — regex.match(), regex.test(), regex.replace()
std.uuid     — uuid.v4(), uuid.parse()
std.path     — path.join(), path.basename(), path.extension()
std.rand     — rand.int(), rand.float(), rand.bool()
std.compress — compress.gzip(), compress.gunzip()
std.encode   — encode.base64(), encode.hex()
std.sort     — sort.asc(), sort.desc(), sort.by()
std.fmt      — fmt.number(), fmt.bytes()
std.text     — text.camel(), text.snake(), text.slug()
std.platform — platform.raw_mode(), platform.sdl_init()
```

All stdlib functions that can fail return `(value, string)` — value and error string.
Infallible functions return a single value.

---

## Empty Collections

```magi
const empty_arr = []int{}            // empty typed array
const empty_map = map[string]int{}   // empty typed map
const empty = []                     // empty array, type inferred from context
```

---

## Method vs Function

Array methods like `.map()`, `.filter()`, `.reduce()` can also be called as functions in pipes:

```magi
// Method call:
const result = arr.filter(x => x > 0)

// Pipe call (equivalent):
const result = arr |> filter(x => x > 0)

// Both .length() method and len() builtin work:
const n = arr.length()
const n = len(arr)
```

The pipe form calls the method on the piped value. They are interchangeable.

---

## Entry Point

Top-level code executes directly. No `main()` required.

```magi
// This runs immediately:
println("hello world")

// Or use main for structure:
func main() {
    println("hello world")
}
main()
```
