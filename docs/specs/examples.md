# MAGI — Example Programs (New Syntax)

## Hello World

```magi
println("Hello, World!")
```

## FizzBuzz

```magi
for n in 1..=20 {
    const label = match n {
        _ if n % 15 == 0 => "FizzBuzz",
        _ if n % 3 == 0 => "Fizz",
        _ if n % 5 == 0 => "Buzz",
        _ => to_string(n),
    }
    println(f"{n}: {label}")
}
```

## Fibonacci

```magi
func fib(n int) -> int {
    if n <= 1 { return n }
    let a = 0
    let b = 1
    for i in 2..=n {
        const temp = a + b
        a = b
        b = temp
    }
    b
}

for i in 0..15 {
    println(f"fib({i}) = {fib(i)}")
}
```

## HTTP Client

```magi
import std.{net, json}

let resp, err = net.get("https://httpbin.org/get")
if err {
    println(f"request failed: {err}")
    return
}

println(f"status: {resp.status}")

let data, err = json.parse(resp.body)
if err {
    println(f"parse failed: {err}")
    return
}

println(f"origin: {data.origin}")
```

## File Processing

```magi
import std.{fs, json}

func load_config(path string) -> (map[string]any, string) {
    let text, err = fs.read(path)
    if err { return null, f"read: {err}" }

    let config, err = json.parse(text)
    if err { return null, f"parse: {err}" }

    config, null
}

let config, err = load_config("config.json")
if err {
    println(f"error: {err}")
    return
}

println(f"host: {config.host}")
println(f"port: {config.port}")
```

## Structs and Methods

```magi
import std.math

struct Vec2 { x: float, y: float }

func Vec2.length(self) -> float {
    math.sqrt(self.x * self.x + self.y * self.y)
}

func Vec2.add(self, other Vec2) -> Vec2 {
    Vec2 { x: self.x + other.x, y: self.y + other.y }
}

func Vec2.display(self) -> string {
    f"({self.x}, {self.y})"
}

const a = Vec2 { x: 3.0, y: 4.0 }
const b = Vec2 { x: 1.0, y: 2.0 }

println(f"a = {a}")                    // (3.0, 4.0)
println(f"b = {b}")                    // (1.0, 2.0)
println(f"a + b = {a + b}")           // (4.0, 6.0)
println(f"|a| = {a.length()}")        // 5.0
```

## Interfaces

```magi
import std.math

interface Shape {
    func area(self) -> float
    func perimeter(self) -> float
}

struct Circle { radius: float }
struct Rect { width: float, height: float }

func Circle.area(self) -> float { math.pi * self.radius * self.radius }
func Circle.perimeter(self) -> float { 2.0 * math.pi * self.radius }
func Circle.display(self) -> string { f"Circle(r={self.radius})" }

func Rect.area(self) -> float { self.width * self.height }
func Rect.perimeter(self) -> float { 2.0 * (self.width + self.height) }
func Rect.display(self) -> string { f"Rect({self.width}x{self.height})" }

func describe(s Shape) {
    println(f"{s}: area={s.area()}, perimeter={s.perimeter()}")
}

describe(Circle { radius: 5.0 })
describe(Rect { width: 3.0, height: 4.0 })
```

## Enums and Pattern Matching

```magi
enum Token {
    Number(float),
    String(string),
    Ident(string),
    Plus,
    Minus,
    Star,
    Eof,
}

func Token.display(self) -> string {
    match self {
        Token::Number(n) => f"NUM({n})",
        Token::String(s) => f"STR(\"{s}\")",
        Token::Ident(name) => f"ID({name})",
        Token::Plus => "+",
        Token::Minus => "-",
        Token::Star => "*",
        Token::Eof => "EOF",
    }
}

const tokens = [
    Token::Ident("x"),
    Token::Plus,
    Token::Number(42.0),
    Token::Star,
    Token::Ident("y"),
    Token::Eof,
]

for tok in tokens {
    print(f"{tok} ")
}
println("")
// ID(x) + NUM(42) * ID(y) EOF
```

## Error Handling

```magi
import std.fs

func read_lines(path string) -> ([]string, string) {
    let text, err = fs.read(path)
    if err { return null, err }

    const lines = text.split("\n")
        |> filter(line => line.trim().length() > 0)
    lines, null
}

func count_words(path string) -> (int, string) {
    let lines, err = read_lines(path)
    if err { return 0, err }

    const total = lines
        |> map(line => line.split(" ").length())
        |> reduce(0, (acc, n) => acc + n)
    total, null
}

let count, err = count_words("document.txt")
if err {
    println(f"error: {err}")
} else {
    println(f"word count: {count}")
}
```

## Functional Pipelines

```magi
const data = [
    { name: "Alice", age: 30, score: 95 },
    { name: "Bob", age: 25, score: 87 },
    { name: "Carol", age: 35, score: 92 },
    { name: "Dave", age: 28, score: 78 },
    { name: "Eve", age: 32, score: 96 },
]

const top_scorers = data
    |> filter(p => p.score >= 90)
    |> sort_by(p => -p.score)
    |> map(p => f"{p.name}: {p.score}")

for line in top_scorers {
    println(line)
}
// Eve: 96
// Alice: 95
// Carol: 92

const avg_age = data
    |> map(p => p.age)
    |> reduce(0, (acc, age) => acc + age)
    |> (total => total / data.length())

println(f"average age: {avg_age}")
```

## Concurrency

```magi
import std.net

async func fetch(url string) -> (string, string) {
    let resp, err = net.get(url)
    if err { return null, err }
    resp.body, null
}

const urls = [
    "https://api.example.com/users",
    "https://api.example.com/posts",
    "https://api.example.com/comments",
]

// Spawn all requests concurrently
const tasks = urls |> map(url => spawn fetch(url))

// Await all results
for task in tasks {
    let body, err = await task
    if err {
        println(f"failed: {err}")
    } else {
        println(f"got {body.length()} bytes")
    }
}
```

## Generics

```magi
func filter_map<T, U>(items []T, f func(T) -> (U, bool)) -> []U {
    let result = []
    for item in items {
        const (val, ok) = f(item)
        if ok { result.push(val) }
    }
    result
}

const numbers = ["1", "abc", "3", "def", "5"]
const parsed = filter_map(numbers, (s) => {
    const n = parse_int(s)
    if n != null { (n, true) } else { (0, false) }
})
println(parsed)  // [1, 3, 5]
```

## Defer

```magi
import std.fs

func process_file(path string) -> (string, string) {
    let file, err = fs.open(path)
    if err { return null, err }
    defer fs.close(file)

    let content, err = fs.read_all(file)
    if err { return null, err }

    content, null
}

let data, err = process_file("input.txt")
if err {
    println(f"error: {err}")
} else {
    println(f"read {len(data)} bytes")
}
```

## Labeled Break

```magi
const matrix = [[1, 2, 3], [4, 0, 6], [7, 8, 9]]

let found = false
'search: for row in matrix {
    for cell in row {
        if cell == 0 {
            found = true
            break 'search
        }
    }
}

println(f"found zero: {found}")
```

## Attributes

```magi
#[test]
func test_addition() {
    assert(1 + 1 == 2)
}

#[test]
func test_string_length() {
    assert(len("hello") == 5)
}

#[deprecated("use parse_config instead")]
func load_config(path string) -> (map[string]string, string) {
    let text, err = fs.read(path)
    if err { return null, err }
    json.parse(text)
}
```

## Complete Program — Todo App

```magi
import std.{fs, json, time}

struct Todo {
    id: int,
    title: string,
    done: bool,
    created: int,
}

func Todo.display(self) -> string {
    const status = if self.done { "✓" } else { " " }
    f"[{status}] {self.title}"
}

func load_todos(path string) -> ([]map[string]string, string) {
    let text, err = fs.read(path)
    if err { return [], null }

    let data, err = json.parse(text)
    if err { return [], null }

    data, null
}

func save_todos(path string, todos []Todo) -> string {
    const text = json.stringify(todos)
    let _, err = fs.write(path, text)
    err
}

func add_todo(todos []Todo, title string) -> []Todo {
    const todo = Todo {
        id: len(todos) + 1,
        title: title,
        done: false,
        created: time.now(),
    }
    [...todos, todo]
}

func complete_todo(todos []Todo, id int) -> []Todo {
    todos |> map(t => {
        if t.id == id { Todo { ...t, done: true } } else { t }
    })
}

// Main
let todos = []Todo{}

todos = add_todo(todos, "Write MAGI spec")
todos = add_todo(todos, "Implement syntax overhaul")
todos = add_todo(todos, "Self-host the compiler")
todos = complete_todo(todos, 1)

for todo in todos {
    println(todo)
}
// [✓] Write MAGI spec
// [ ] Implement syntax overhaul
// [ ] Self-host the compiler

const err = save_todos("todos.json", todos)
if err { println(f"save failed: {err}") }
```
