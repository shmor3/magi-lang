# MAGI Language Specification

Version: 1.0.0

---

## 1. Lexical Structure

### Keywords
```
const let func struct enum interface type import
if else for in while match select
break continue return defer spawn await async
true false null
```

### Literals
- **Integers**: `42`, `0xFF`, `0o77`, `0b1010`, `1_000_000`
- **Floats**: `3.14`, `1.0e10`, `2.5E-3`
- **Strings**: `"hello"`, `f"name is {name}"`, `"""multiline"""`, `r"raw\n"`
- **Booleans**: `true`, `false`
- **Null**: `null`
- **Characters**: `'a'`, `'\n'`
- **Arrays**: `[1, 2, 3]`
- **Maps**: `{"key": "value"}`
- **Tuples**: `(1, "hello", true)`

### Operators
```
+  -  *  /  %  **           Arithmetic
== != < > <= >=             Comparison
&& || !                     Logical
& | ^ << >> &^              Bitwise
+= -= *= /= %= &= |= ^=   Compound assignment
|>                          Pipe
=>                          Arrow function
.. ..=                      Range
?.                          Optional chain
??                          Null coalesce
...                         Spread
in                          Membership
```

### Comments
```magi
// line comment
/* block comment */
/// doc comment
```

### Semicolons
Optional. Newlines end statements. Semicolons allowed for multiple statements on one line.

---

## 2. Types

### Primitive Types
```
int       // 64-bit signed integer (default)
float     // 64-bit floating point (default)
string    // UTF-8 string
bool      // true or false
byte      // unsigned 8-bit
any       // any type (dynamic)
int32     // 32-bit signed
uint32    // 32-bit unsigned
uint64    // 64-bit unsigned
float32   // 32-bit float
```

### Composite Types
```
[]int                    // array of int
map[string]int           // map with string keys, int values
(int, string)            // tuple
set[int]                 // set
[]byte                   // byte array
```

### User-Defined Types
```magi
struct Point { x: float, y: float }
enum Shape { Circle(float), Rect(float, float) }
type UserId = int
```

### Generics
```magi
func max<T: Compare>(a T, b T) -> T { if a.compare(b) > 0 { a } else { b } }
struct Stack<T> { items: []T }
```

---

## 3. Bindings

```magi
const x = 42          // immutable (default, encouraged)
let count = 0         // mutable
count = count + 1     // ok
```

All variables must be initialized. Destructuring works with both:
```magi
const [a, b, c] = [1, 2, 3]
let [head, ...rest] = [1, 2, 3, 4]
const {name, age} = person
let result, err = divide(10, 3)
```

---

## 4. Functions

```magi
func add(a int, b int) -> int { a + b }
func greet(name string) -> string { f"Hello, {name}!" }
func double(x) { x * 2 }    // type annotations optional
```

### Arrow Functions
```magi
const double = x => x * 2
const add = (a, b) => a + b
numbers |> filter(x => x > 0) |> map(x => x * x)
```

### Dot Receiver Methods
```magi
struct Vec2 { x: float, y: float }

func Vec2.length(self) -> float {
    math.sqrt(self.x * self.x + self.y * self.y)
}

func Vec2.add(self, other Vec2) -> Vec2 {
    Vec2 { x: self.x + other.x, y: self.y + other.y }
}
```

### Multi-Return
```magi
func divide(a int, b int) -> (int, string) {
    if b == 0 { return 0, "division by zero" }
    return a / b, null
}

let result, err = divide(10, 3)
if err { return null, err }
```

---

## 5. Interfaces

Implicit satisfaction — no `implements` declaration needed.

```magi
interface Shape {
    func area(self) -> float
    func perimeter(self) -> float
}

struct Circle { radius: float }
func Circle.area(self) -> float { 3.14159 * self.radius * self.radius }
func Circle.perimeter(self) -> float { 2.0 * 3.14159 * self.radius }
// Circle satisfies Shape automatically
```

### Operator Interfaces
```
+       Add.add(self, other)
-       Subtract.subtract(self, other)
*       Multiply.multiply(self, other)
/       Divide.divide(self, other)
==      Equal.equal(self, other)
<><=>=  Compare.compare(self, other) -> int
[]      Index.index(self, key)
for..in Iterable.iterate(self)
print   Display.display(self) -> string
len()   Length.length(self) -> int
in      Contains.contains(self, key) -> bool
```

---

## 6. Enums

```magi
enum Color { Red, Green, Blue, Rgb(int, int, int) }
enum Direction { North = 0, East, South, West }

println(Color::Red)              // Color::Red
println(Color::Rgb(255, 0, 0))   // Color::Rgb(255, 0, 0)

func Color.display(self) -> string {
    match self {
        Color::Red => "red",
        Color::Rgb(r, g, b) => f"#{r:02x}{g:02x}{b:02x}",
        _ => "color",
    }
}
```

---

## 7. Control Flow

```magi
// If/else (expression)
const max = if a > b { a } else { b }

// For loop (ranges and iterables only — no C-style)
for i in 0..10 { println(i) }
for item in collection { println(item) }
'outer: for row in matrix { for cell in row { if cell == 0 { break 'outer } } }

// While
while condition { ... }

// Match (exhaustive — all variants must be covered)
match direction {
    Direction::North => "north",
    Direction::East => "east",
    _ => "other",
}

// Match guards
match n {
    _ if n % 15 == 0 => "FizzBuzz",
    _ if n % 3 == 0 => "Fizz",
    _ => to_string(n),
}
```

---

## 8. Error Handling

Errors are values. No exceptions.

```magi
func read_config(path string) -> (Config, string) {
    let text, err = fs.read(path)
    if err { return null, f"read: {err}" }

    let data, err = json.parse(text)
    if err { return null, f"parse: {err}" }

    data, null
}

let config, err = read_config("app.json")
if err { println(f"error: {err}"); return }
```

---

## 9. Imports

```magi
import std.math              // math.sqrt(), math.sin()
import std.{fs, json, net}   // multi-import
import std.math as m         // alias: m.sqrt()
import canvas                // package
import ./util                // relative
```

---

## 10. Concurrency

```magi
const task = spawn fetch_data(url)
const result = await task

select {
    msg from inbox => println(msg),
    tick from timer => update(),
    _ => println("timeout"),
}

defer fs.close(file)    // runs on function exit
```

---

## 11. Built-in Functions

```
println(value)    // print with newline, returns value
print(value)      // print without newline, returns value
len(collection)   // length
to_string(value)  // convert to string
parse_int(s)      // string to int (null on failure)
parse_float(s)    // string to float (null on failure)
typeof(value)     // type name as string
assert(condition) // panic if false
```

---

## 12. Attributes

```magi
#[test]
func test_add() { assert(1 + 1 == 2) }

#[deprecated("use new_func instead")]
func old_func() { ... }
```
