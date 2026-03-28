# MAGI Language Specification

Version: 0.9.0

---

## 1. Lexical Structure

### Keywords
```
let mut const fn return if else for while loop match break continue
struct enum impl trait use mod pub async await spawn yield
try catch finally throw defer output type static unsafe asm
ref move dyn where super
```

### Literals
- **Integers**: `42`, `0xFF`, `0o77`, `0b1010`, `1_000_000`
- **Floats**: `3.14`, `1.0e10`, `2.5E-3`
- **Strings**: `"hello"`, `f"name is {name}"` (string interpolation), `"""multiline"""`, `r"raw\n"`
- **Booleans**: `true`, `false`
- **Null**: `null`
- **Characters**: `'a'`, `'\n'`
- **Arrays**: `[1, 2, 3]`
- **Maps**: `{"key": "value"}`
- **Sets**: `Set(1, 2, 3)`
- **Tuples**: `Tuple(1, "hello", true)`

### Operators
```
+  -  *  /  %  **           Arithmetic
== != < > <= >=             Comparison
&& || !                     Logical
& | ^ << >> &^              Bitwise
+= -= *= /= %= &= |= ^=   Compound assignment
|>                          Pipe
.. ..=                      Range
?.                          Optional chain
??                          Null coalesce
?                           Try propagate
...                         Spread
in                          Membership
```

---

## 2. Types

### Primitive Types
`int64`, `float64`, `int32`, `uint32`, `uint64`, `float32`, `bool`, `string`, `null`

### Composite Types
- **Array**: ordered collection
- **Map**: insertion-order preserving key-value pairs
- **Set**: unordered unique values
- **Tuple**: fixed-size heterogeneous collection
- **Bytes**: raw binary data
- **Optional**: `Some(value)` / `None`
- **Result**: `Ok(value)` / `Err(msg)`

### User-Defined Types
```magi
struct Point { x: float64, y: float64 }
enum Shape { Circle(float64), Rect(float64, float64) }
type UserId = int64  // type alias
```

### Generics
```magi
fn<T, U: Display>(x: T) -> U { ... }
struct Box<T> { value: T }
enum Option<T> { Some(T), None }
```

---

## 3. Expressions

### Control Flow (expression-based)
```magi
let x = if cond { a } else { b };
let y = match val { 1 => "one", _ => "other" };
let z = loop { if done { break result; } };
```

### Closures
```magi
let add = |a, b| a + b;
let transform = |x| { let y = x * 2; y + 1 };
```

### Pipe Operator
```magi
data |> filter(|x| x > 0) |> map(|x| x * 2) |> sum()
```

### Destructuring
```magi
let [a, b, ...rest] = [1, 2, 3, 4, 5];
let {name, age} = {"name": "Alice", "age": 30};
for [key, value] in map.entries() { ... }
```

---

## 4. Statements

### Variable Binding
```magi
let x = 42;
let mut counter = 0;
const MAX = 100;
static GLOBAL: int64 = 0;
```

### Functions
```magi
fn greet(name: string) -> string { f"Hello, {name}!" }
async fn fetch(url: string) { await http_get(url) }
fn<T>(items: [T], pred: fn(T) -> bool) -> [T] { items.filter(pred) }
```

### Control Flow
```magi
for item in collection { ... }
for (let mut i = 0; i < n; i += 1) { ... }
'outer: for x in xs { break 'outer; }
while condition { ... }
do { ... } while condition;
loop { break; }
defer cleanup();
try { risky() } catch err { handle(err) } finally { cleanup() }
```

### Pattern Matching
```magi
match value {
    0 => "zero",
    1..=9 => "digit",
    n if n < 0 => "negative",
    Some(x) => f"got {x}",
    [first, .., last] => f"{first}..{last}",
    _ => "other",
}
```

### Structs, Enums, Traits
```magi
struct Point { x: float64, y: float64 }
impl Point { fn distance(self, other: Point) -> float64 { ... } }
trait HasArea { fn area(self) -> float64; }
impl HasArea for Point { fn area(self) -> float64 { 0.0 } }
enum Color { Red, Green, Blue, Custom(int64, int64, int64) }
```

### Modules
```magi
mod utils { pub fn square(x) { x * x } }
use utils::*;
use std::math::*;
```

### Operator Overloading
```magi
impl Point {
    fn __add__(self, other) { Point { x: self.x + other.x, y: self.y + other.y } }
    fn __eq__(self, other) { self.x == other.x && self.y == other.y }
    fn __str__(self) { f"({self.x}, {self.y})" }
    fn __index__(self, i) { if i == 0 { self.x } else { self.y } }
    fn __iter__(self) { [self.x, self.y] }
}
```

---

## 5. Concurrency

```magi
spawn { expensive_computation() };
let (tx, rx) = channel();
chan_send(tx, value);
let msg = chan_recv(rx);
select { msg from rx1 => handle(msg), msg from rx2 => handle(msg) }
```

---

## 6. Error Handling

```magi
try { fs_read("file.txt") } catch err { output err; }
fn divide(a, b) { if b == 0 { Err("zero") } else { Ok(a / b) } }
let value = risky()?;
```

---

## 7. Attributes

```magi
#[test] fn test_add() { assert_eq(1 + 1, 2); }
#[deprecated] fn old() { ... }
#[ignore] fn skip() { ... }
#[cfg(target_os = "linux")] fn linux_only() { ... }
```
