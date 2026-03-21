# MAGI Language Specification

Version: 0.3.0-alpha

This document is a semi-formal specification of the MAGI programming language. It covers
lexical structure, types, expressions, statements, pattern matching, scoping, and error
handling. Grammar rules use EBNF-like notation where appropriate.

---

## Table of Contents

1. [Lexical Structure](#1-lexical-structure)
2. [Types](#2-types)
3. [Expressions](#3-expressions)
4. [Statements](#4-statements)
5. [Operator Precedence](#5-operator-precedence)
6. [Scoping Rules](#6-scoping-rules)
7. [Pattern Matching](#7-pattern-matching)
8. [Standard Library](#8-standard-library)
9. [Error Handling](#9-error-handling)

---

## 1. Lexical Structure

### 1.1 Source Encoding

Source files are UTF-8 encoded. The lexer processes input byte-by-byte for ASCII tokens
and decodes full UTF-8 code points inside string literals, comments, and identifiers.

### 1.2 Whitespace

Whitespace characters (space `0x20`, tab `0x09`, carriage return `0x0D`, newline `0x0A`)
are insignificant outside of string literals and serve only to separate tokens. Semicolons
are optional statement terminators; the parser infers statement boundaries from context.

### 1.3 Comments

```ebnf
line_comment  = "//" { any_char - newline } newline ;
block_comment = "/*" { any_char | block_comment } "*/" ;
```

- **Line comments** begin with `//` and extend to the end of the line.
- **Block comments** begin with `/*` and end with `*/`. Block comments nest: `/* outer /* inner */ still outer */` is valid. Maximum nesting depth is 256.

### 1.4 Keywords

The following identifiers are keywords and cannot be used as variable or function names:

| Category | Keywords |
|---|---|
| Bindings | `let`, `mut`, `const` |
| Control flow | `if`, `else`, `for`, `in`, `while`, `loop`, `break`, `continue`, `return` |
| Functions | `fn`, `async`, `await`, `spawn` |
| Values | `true`, `false`, `null` |
| Declarations | `enum`, `struct`, `type`, `mod`, `pub` |
| Imports | `import`, `use`, `as` |
| Error handling | `try`, `catch`, `finally`, `throw` |
| Pattern matching | `match` |
| Testing | `test` |
| I/O | `output` |

#### Reserved Keywords

The following identifiers are reserved for future use and produce a compile error if used
as identifiers:

```
trait  impl  static  ref  move  yield  self  super  where  dyn
```

### 1.5 Identifiers

```ebnf
ident_start = ascii_letter | "_" | unicode_alphabetic ;
ident_cont  = ident_start | ascii_digit | unicode_alphanumeric | "-" (* see note *) ;
identifier  = ident_start { ident_cont } ;
```

Identifiers may begin with an ASCII letter, underscore, or any Unicode alphabetic character.
Subsequent characters may also include digits and Unicode alphanumerics. Hyphens (`-`) are
permitted inside identifiers only when immediately followed by an alphabetic character (to
support plugin IDs like `text-llm`); otherwise a hyphen is parsed as the subtraction operator.

The standalone underscore `_` is a special **placeholder** token used in pipe expressions, not
a valid variable name.

### 1.6 Literals

#### Integer Literals

```ebnf
decimal_int = digit { digit | "_" digit } ;
hex_int     = "0" ("x" | "X") hex_digit { hex_digit | "_" hex_digit } ;
octal_int   = "0" ("o" | "O") octal_digit { octal_digit | "_" octal_digit } ;
binary_int  = "0" ("b" | "B") bin_digit { bin_digit | "_" bin_digit } ;
int_literal = decimal_int | hex_int | octal_int | binary_int ;
```

- Underscores may appear between digits as visual separators: `1_000_000`.
- Hex, octal, and binary literals are interpreted as unsigned bit patterns and stored
  as `i64` via two's complement reinterpretation (values up to `u64::MAX` are accepted).
- Decimal literals are parsed as signed `i64`.

#### Float Literals

```ebnf
float_literal = digit { digit | "_" digit } "." digit { digit | "_" digit }
                [ ("e" | "E") ["+" | "-"] digit { digit } ] ;
```

- The decimal point must be followed by at least one digit (to disambiguate `1.method()`).
- Scientific notation with `e`/`E` is supported. The exponent must contain at least one digit.
- All floats are stored as `f64` (64-bit IEEE 754).

#### String Literals

```ebnf
string_literal       = '"' { char | escape_sequence } '"' ;
triple_string        = '"""' { any_char | escape_sequence } '"""' ;
raw_string           = 'r"' { any_char - '"' } '"' ;
fstring              = 'f"' { char | escape_sequence | interpolation } '"' ;
interpolation        = "{" expression "}" ;
escape_sequence      = "\" ( "n" | "t" | "r" | "\\" | '"' | "0" | "{" | "}"
                           | "x" hex_digit hex_digit
                           | "u" "{" hex_digit{1,6} "}" ) ;
```

- **Regular strings** (`"..."`) support escape sequences.
- **Triple-quoted strings** (`"""..."""`) may span multiple lines and also support escapes.
- **Raw strings** (`r"..."`) treat backslashes as literal characters; no escape processing.
- **Interpolated strings** (`f"..."`) embed expressions inside `{...}` braces. Use `\{` and
  `\}` to produce literal braces. Nested braces inside interpolation expressions are tracked
  automatically.

#### Boolean Literals

```
true | false
```

#### Null Literal

```
null
```

### 1.7 Operators and Punctuation

| Token | Symbol | Token | Symbol |
|---|---|---|---|
| `+` | Plus | `+=` | Plus-assign |
| `-` | Minus | `-=` | Minus-assign |
| `*` | Star | `*=` | Star-assign |
| `/` | Slash | `/=` | Slash-assign |
| `%` | Percent | `%=` | Percent-assign |
| `==` | Equal | `!=` | Not-equal |
| `>` | Greater | `>=` | Greater-equal |
| `<` | Less | `<=` | Less-equal |
| `&&` | Logical AND | `\|\|` | Logical OR |
| `!` | Logical NOT | `=` | Assignment |
| `\|>` | Pipe | `\|` | Bar (lambdas, or-patterns) |
| `->` | Arrow (return type) | `=>` | Fat arrow (match arms) |
| `??` | Null coalesce | `?.` | Optional chain |
| `..` | Exclusive range | `..=` | Inclusive range |
| `...` | Spread/rest | `?` | Try-propagate |
| `(` `)` | Parentheses | `[` `]` | Brackets |
| `{` `}` | Braces | `:` | Colon |
| `::` | Path separator | `;` | Semicolon |
| `,` | Comma | `.` | Dot |
| `_` | Placeholder | | |

---

## 2. Types

MAGI uses a strict type system based on the `DataType` enum. All runtime values are one of
the following types.

### 2.1 Primitive Types

| Type name | Description | Literal syntax |
|---|---|---|
| `null` | Absence of a value; default type | `null` |
| `bool` | Boolean | `true`, `false` |
| `int32` | 32-bit signed integer | (via API/runtime; literals parse as int64) |
| `int64` | 64-bit signed integer | `42`, `0xFF`, `0o77`, `0b1010`, `1_000` |
| `uint32` | 32-bit unsigned integer | (via API/runtime) |
| `uint64` | 64-bit unsigned integer | (via API/runtime) |
| `float32` | 32-bit IEEE 754 float | (via API/runtime) |
| `float64` | 64-bit IEEE 754 float | `3.14`, `1.5e10` |
| `string` | UTF-8 text | `"hello"`, `r"raw"`, `f"interp {x}"` |
| `bytes` | Raw binary data | (via API/runtime) |

Integer literals in source code are always parsed as `int64`. The narrower integer types
(`int32`, `uint32`, `uint64`) and `float32` exist in the runtime type system and are
produced by API operations, plugin interactions, and explicit conversions.

### 2.2 Collection Types

| Type name | Description | Literal syntax |
|---|---|---|
| `array` | Ordered, heterogeneous sequence | `[1, "two", true]` |
| `map` | Insertion-ordered string-keyed map | `{"key": value, ...}` |

- **Arrays** may contain values of any type, including nested arrays and maps.
- **Maps** use string keys and preserve insertion order (backed by `IndexMap`).
  Map keys in literals must be string literals: `{"name": "Alice", "age": 30}`.

### 2.3 Future Type

```
future<pending | resolved(value) | rejected(error)>
```

The `future` type represents an asynchronous computation in one of three states:

- `Pending` -- computation has not yet completed.
- `Resolved(value)` -- computation completed successfully with a value.
- `Rejected(error)` -- computation failed with an error message.

Futures are created with `spawn` and consumed with `await`.

### 2.4 Type Annotations

Type annotations are optional and appear after a colon:

```magi
let x: int64 = 42;
let name: string = "Alice";
const MAX: int64 = 100;
fn add(a: int64, b: int64) -> int64 { a + b }
```

### 2.5 Type Aliases

```magi
type UserId = int64;
type Name = string;
type Scores = array;
```

Type aliases provide semantic names for existing types. At runtime, they are treated as
their base type.

### 2.6 Truthiness

All values have a boolean interpretation used in conditions (`if`, `while`, `&&`, `||`):

| Type | Falsy values | Truthy values |
|---|---|---|
| `bool` | `false` | `true` |
| `int32`, `int64` | `0` | non-zero |
| `uint32`, `uint64` | `0` | non-zero |
| `float32`, `float64` | `0.0`, `NaN` | all other values |
| `string` | `""` (empty) | non-empty |
| `null` | always falsy | -- |
| `bytes` | empty | non-empty |
| `array` | empty | non-empty |
| `map` | empty | non-empty |
| `future` | -- | always truthy |

---

## 3. Expressions

All expressions produce a value. Many constructs that are statements in other languages
(if/else, match, try/catch, loop, blocks) are expressions in MAGI.

### 3.1 Literal Expressions

```ebnf
literal = int_literal | float_literal | string_literal | "true" | "false" | "null"
        | array_literal | map_literal ;

array_literal = "[" [ expression { "," expression } [","] ] "]" ;
map_literal   = "{" [ string_literal ":" expression { "," string_literal ":" expression } [","] ] "}" ;
```

### 3.2 Variable References

```ebnf
variable = identifier ;
```

Looks up a name in the current scope chain (innermost to outermost).

### 3.3 Binary Operations

```ebnf
binary_expr = expression binop expression ;
binop = "+" | "-" | "*" | "/" | "%" | "==" | "!=" | ">" | "<" | ">=" | "<=" | "&&" | "||" ;
```

Binary operators are left-associative. See [Section 5](#5-operator-precedence) for the
complete precedence table.

- Arithmetic: `+`, `-`, `*`, `/`, `%`
- Comparison: `==`, `!=`, `>`, `<`, `>=`, `<=`
- Logical: `&&` (short-circuit AND), `||` (short-circuit OR)

### 3.4 Unary Operations

```ebnf
unary_expr = unop expression ;
unop = "!" | "-" ;
```

- `!expr` -- logical NOT
- `-expr` -- arithmetic negation

Unary operators bind tighter than all binary operators.

### 3.5 Function Calls

```ebnf
call_expr = identifier "(" [ args_and_kwargs ] ")" ;
args_and_kwargs = positional_args [ "," keyword_args ] | keyword_args ;
positional_args = expression { "," expression } ;
keyword_args    = identifier "=" expression { "," identifier "=" expression } ;
```

Functions are called by name with positional and optional keyword arguments:

```magi
add(3, 4)
greet("World", greeting="Hi")
```

### 3.6 Method Calls

```ebnf
method_call = expression "." identifier "(" [ args_and_kwargs ] ")" ;
```

Method calls desugar to function calls with the receiver as the first argument:

```magi
"hello".to_upper()      // string method
[1, 2, 3].map(|x| x * 2)  // array method
```

### 3.7 Pipe Operator

```ebnf
pipe_expr = expression "|>" expression { "|>" expression } ;
```

The pipe operator passes the left-hand value into the right-hand expression. The
placeholder `_` marks where the piped value is inserted:

```magi
"hello" |> to_upper(_) |> add_prefix(_)
[1, 2, 3] |> len(_)
```

Pipes are left-associative and have the lowest precedence of all operators.

### 3.8 If/Else Expressions

```ebnf
if_expr = "if" expression block [ "else" ( block | if_expr ) ] ;
```

If/else is an expression -- the last expression in each branch is the value:

```magi
let max = if a > b { a } else { b };
```

The condition is parsed with struct literal suppression (a `{` after the condition starts
a block, not a struct).

### 3.9 Match Expressions

```ebnf
match_expr = "match" expression "{" { match_arm } "}" ;
match_arm  = pattern [ "if" expression ] "=>" ( block | expression ) [ "," ] ;
```

Match evaluates patterns top-to-bottom and executes the first matching arm. An optional
guard (`if condition`) adds a boolean check after the pattern matches:

```magi
match value {
    0 => "zero",
    1 | 2 | 3 => "small",
    n: int64 if n > 100 => "large",
    _ => "other",
}
```

See [Section 7](#7-pattern-matching) for all pattern forms.

### 3.10 Lambda Expressions

```ebnf
lambda = "|" [ params ] "|" ( expression | block ) ;
lambda = "||" ( expression | block ) ;   (* zero-parameter form *)
```

Lambdas capture their enclosing scope by value at the time of creation:

```magi
let double = |x| x * 2;
let clamp = |val, lo, hi| {
    if val < lo { lo } else if val > hi { hi } else { val }
};
let thunk = || 42;
```

Parameters support type annotations, default values, and rest parameters with the same
syntax as function definitions.

### 3.11 Range Expressions

```ebnf
range = expression ".." expression ;       (* exclusive end *)
range = expression "..=" expression ;      (* inclusive end *)
```

Ranges produce an array of integers:

```magi
1..5     // [1, 2, 3, 4]
1..=5    // [1, 2, 3, 4, 5]
```

### 3.12 Comprehensions

#### List Comprehension

```ebnf
list_comp = "[" expression "for" for_pattern "in" expression [ "if" expression ] "]" ;
```

```magi
[n * n for n in 1..=6]
[n * n for n in 1..=10 if n % 2 == 0]
```

#### Map Comprehension

```ebnf
map_comp = "{" string_literal ":" expression "for" for_pattern "in" expression [ "if" expression ] "}" ;
```

### 3.13 Optional Chaining

```ebnf
optional_chain = expression "?." identifier ;
optional_chain = expression "?." identifier "(" [ args ] ")" ;
optional_chain = expression "?" "[" expression "]" ;
```

Optional chaining returns `null` if the receiver is null, instead of raising an error.
Chains propagate through subsequent `.field`, `.method()`, and `[index]` accesses:

```magi
config?.database?.host       // null if any intermediate is null
config?.cache?.port ?? 6379  // combined with null coalescing
```

### 3.14 Null Coalescing

```ebnf
null_coalesce = expression "??" expression ;
```

Returns the left-hand value if it is not null; otherwise evaluates and returns the
right-hand value (short-circuit):

```magi
let host = cache_host ?? "127.0.0.1";
```

### 3.15 Spread

```ebnf
spread = "..." expression ;
```

Spread expands an array inline within array literals or function call arguments:

```magi
let combined = [...head, 3, ...tail];
sum_all(...args);
```

### 3.16 String Interpolation

```ebnf
fstring = 'f"' { text | "{" expression "}" } '"' ;
```

F-strings embed arbitrary expressions:

```magi
f"Hello, {name}! You are {age + 1} years old."
```

### 3.17 Await and Spawn

```ebnf
await_expr = "await" expression ;
spawn_expr = "spawn" expression ;
spawn_expr = "spawn" block ;
```

- `spawn` creates a `future` from an expression (typically an async function call).
- `await` blocks until a future resolves and returns the resolved value.

```magi
async fn compute(a, b) { a * b + 1 }
let future = spawn compute(6, 7);
let result = await future;
```

### 3.18 Try-Propagate

```ebnf
try_propagate = expression "?" ;
```

The `?` postfix operator provides early return on null/error values. If the expression
evaluates to `null`, execution returns early from the enclosing function with `null`:

```magi
fn get_user_name(id) {
    let user = find_user(id)?;  // returns null early if find_user returns null
    user.name
}
```

### 3.19 Block Expressions

```ebnf
block = "{" { statement } [ expression ] "}" ;
```

A block is a sequence of statements with an optional trailing expression (without a
semicolon). The trailing expression becomes the block's value. If there is no trailing
expression, the block evaluates to the result of the last expression statement, or `null`
if the block is empty.

```magi
let result = {
    let x = 10;
    let y = 20;
    x + y       // trailing expression -- block evaluates to 30
};
```

### 3.20 Loop Expression

```ebnf
loop_expr = "loop" block ;
```

An infinite loop that can be exited with `break`. If `break` carries a value, the loop
expression evaluates to that value:

```magi
let found = loop {
    if condition { break value; }
};
```

### 3.21 Enum Construction

```ebnf
enum_construct = identifier "::" identifier [ "(" [ expression { "," expression } ] ")" ] ;
```

```magi
Shape::Circle(5.0)
Result::Ok(42)
Option::None
```

### 3.22 Struct Construction

```ebnf
struct_construct = identifier "{" identifier ":" expression { "," identifier ":" expression } [","] "}" ;
```

```magi
Point { x: 1.0, y: 2.0 }
```

Struct construction is suppressed in `if`/`while`/`for` conditions and match guards where
a `{` starts a block. Use parentheses to disambiguate: `if (Point { x: 1, y: 2 }).x > 0`.

### 3.23 Index Expression

```ebnf
index = expression "[" expression "]" ;
```

Indexes into arrays (by integer) or maps (by string key). Supports range indexing for
slicing:

```magi
arr[0]
arr[1..4]
map["key"]
"Hello"[0..5]
```

### 3.24 Field Access

```ebnf
field_access = expression "." identifier ;
```

Accesses a named field on a map or struct value.

### 3.25 Try/Catch Expression

```ebnf
try_catch_expr = "try" block "catch" [ identifier ] block [ "finally" block ] ;
```

Try/catch can be used as an expression; the value is the result of the try block or the
catch block:

```magi
let result = try { parse_positive("42") } catch err { f"Error: {err}" };
```

---

## 4. Statements

Statements are executed for their side effects. Semicolons are optional.

### 4.1 Let Bindings

```ebnf
let_stmt     = "let" [ "mut" ] identifier [ ":" type ] "=" expression ";" ;
let_destr    = "let" [ "mut" ] destructure_pattern "=" expression ";" ;
destructure  = "[" { ident | "..." ident } "]"     (* array destructuring *)
             | "{" { ident [ ":" ident ] } "}" ;   (* map destructuring *)
```

`let` creates an immutable binding. `let mut` creates a mutable binding that can be
reassigned.

```magi
let name = "Alice";
let mut counter = 0;
let radius: float64 = 5.0;
let [first, second, ...rest] = [10, 20, 30, 40, 50];
let {city, population} = {"city": "Tokyo", "population": 14000000};
```

### 4.2 Const

```ebnf
const_stmt = "const" identifier [ ":" type ] "=" expression ";" ;
```

Declares a constant binding. Constants cannot be reassigned.

```magi
const PI = 3.14159265;
const MAX_ITEMS: int64 = 100;
```

### 4.3 Assignment

```ebnf
assignment       = identifier "=" expression ";" ;
field_assignment = identifier "." identifier "=" expression ";" ;
index_assignment = identifier "[" expression "]" "=" expression ";" ;
```

Assignment requires the target to have been declared with `let mut`.

### 4.4 Compound Assignment

```ebnf
compound_assign = identifier ( "+=" | "-=" | "*=" | "/=" | "%=" ) expression ";" ;
```

Equivalent to `x = x op expr`:

```magi
counter += 1;
total *= 2;
```

### 4.5 For Loop

```ebnf
for_stmt = "for" for_pattern "in" expression block ;
for_pattern = identifier
            | "[" { ident | "..." ident } "]"
            | "{" { ident [ ":" ident ] } "}" ;
```

Iterates over arrays, ranges, or maps:

```magi
for n in 1..=10 { output n; }
for [x, y] in points { output f"({x}, {y})"; }
for {name, score} in records { output f"{name}: {score}"; }
```

### 4.6 While Loop

```ebnf
while_stmt = "while" expression block ;
```

### 4.7 Loop

```ebnf
loop_stmt = "loop" block ;
```

Infinite loop. Exit with `break`. See also the loop expression form in [Section 3.20](#320-loop-expression).

### 4.8 Break, Continue, Return

```ebnf
break_stmt    = "break" [ expression ] ";" ;
continue_stmt = "continue" ";" ;
return_stmt   = "return" [ expression ] ";" ;
```

- `break` exits the innermost loop. An optional value becomes the loop expression's result.
- `continue` skips to the next iteration of the innermost loop.
- `return` exits the current function with an optional value.

The optional value expression is only parsed if the next token is on the same line as the
keyword (to avoid accidentally consuming the next statement).

### 4.9 Output

```ebnf
output_stmt = "output" expression ";" ;
```

Evaluates the expression, converts it to a string, and emits it as program output.

### 4.10 Import (Legacy)

```ebnf
import_stmt = "import" string_literal ";" ;
```

Imports a plugin by ID. Deprecated in favor of `use`.

### 4.11 Use

```ebnf
use_stmt = "use" path [ "as" identifier ] ";" ;
use_stmt = "use" path "::" "*" ";" ;
path     = identifier { "::" identifier } ;
```

Imports items from modules or packages:

```magi
use utils::double;
use utils::triple as t;
use std::math::*;
```

Glob imports (`*`) cannot have an alias.

### 4.12 Function Definition

```ebnf
fn_def = "fn" identifier "(" [ params ] ")" [ "->" type ] block ;
params = param { "," param } ;
param  = [ "..." ] identifier [ ":" type ] [ "=" expression ] ;
```

- Parameters may have optional type annotations and default values.
- A `...` prefix marks a rest parameter (variadic); it must be the last parameter.
- The return type annotation (`-> type`) is optional.
- Duplicate parameter names are a parse error.

```magi
fn add(a, b) { a + b }
fn greet(who, greeting = "Hello") { f"{greeting}, {who}!" }
fn sum_all(first, ...rest) { /* ... */ }
fn typed(x: int64) -> int64 { x * 2 }
```

### 4.13 Async Function Definition

```ebnf
async_fn_def = "async" "fn" identifier "(" [ params ] ")" [ "->" type ] block ;
```

Async functions return a future when invoked via `spawn`:

```magi
async fn compute(a, b) { a * b + 1 }
let future = spawn compute(6, 7);
let result = await future;
```

### 4.14 Enum Definition

```ebnf
enum_def    = "enum" identifier "{" { variant [ "," ] } "}" ;
variant     = identifier [ "(" identifier { "," identifier } ")" ] ;
```

Defines a tagged union with named variants. Variants may carry named fields:

```magi
enum Shape {
    Circle(radius),
    Rectangle(width, height),
    Triangle(a, b, c)
}

enum Option {
    Some(value),
    None
}
```

Duplicate variant names are a parse error. Variant names may not start with `__`.

### 4.15 Struct Definition

```ebnf
struct_def = "struct" identifier "{" { field [ "," ] } "}" ;
field      = identifier [ ":" type ] ;
```

Defines a record type with named fields:

```magi
struct Point {
    x: float64,
    y: float64
}
```

Duplicate field names are a parse error. Field names may not start with `__`.

### 4.16 Module Definition

```ebnf
mod_def = "mod" identifier block ;
```

Defines an inline module:

```magi
mod utils {
    fn double(x) { x * 2 }
    fn triple(x) { x * 3 }
}
use utils::double;
```

### 4.17 Type Alias

```ebnf
type_alias = "type" identifier "=" identifier ";" ;
```

Creates a type alias:

```magi
type UserId = int64;
```

### 4.18 Test Definition

```ebnf
test_def = "test" string_literal block ;
```

Defines an inline test:

```magi
test "addition works" {
    let result = add(2, 3);
    assert(result == 5);
}
```

### 4.19 Try/Catch Statement

```ebnf
try_catch_stmt = "try" block "catch" [ identifier ] block [ "finally" block ] ;
```

See [Section 9](#9-error-handling) for details.

### 4.20 Throw Statement

```ebnf
throw_stmt = "throw" expression ";" ;
```

Throws an error value. See [Section 9](#9-error-handling).

### 4.21 Pub Modifier

The `pub` keyword can precede `fn`, `async fn`, `mod`, `enum`, `struct`, `const`, `type`,
or `use` declarations to mark them as public:

```magi
pub fn helper(x) { x + 1 }
pub const VERSION = "1.0";
```

### 4.22 Expression Statements

Any expression can appear as a statement. If followed by a semicolon, it is an expression
statement whose value is discarded.

---

## 5. Operator Precedence

From **lowest** (loosest binding) to **highest** (tightest binding):

| Precedence | Operators | Associativity | Description |
|:---:|---|---|---|
| 1 | `\|>` | Left | Pipe |
| 2 | `??` | Left | Null coalescing |
| 3 | `..` `..=` | Non-associative | Range |
| 4 | `\|\|` | Left | Logical OR |
| 5 | `&&` | Left | Logical AND |
| 6 | `==` `!=` | Left | Equality |
| 7 | `>` `<` `>=` `<=` | Left | Comparison |
| 8 | `+` `-` | Left | Addition, subtraction |
| 9 | `*` `/` `%` | Left | Multiplication, division, modulo |
| 10 | `!` `-` (unary) `await` `spawn` | Right (prefix) | Unary NOT, negation, await, spawn |
| 11 | `()` `[]` `.` `?.` `?` | Left (postfix) | Call, index, field, optional chain, try-propagate |

The parser uses precedence climbing (Pratt parsing) for binary operators. Within the binary
operator group, the BinOp precedence values are:

| Level | Operators |
|:---:|---|
| 1 | `\|\|` |
| 2 | `&&` |
| 3 | `==` `!=` |
| 4 | `>` `<` `>=` `<=` |
| 5 | `+` `-` |
| 6 | `*` `/` `%` |

All binary operators are **left-associative** (the parser recurses with `prec + 1` on the
right side).

---

## 6. Scoping Rules

### 6.1 Lexical Scoping

MAGI uses lexical (static) scoping. The scope chain is a stack of hash maps, searched
from innermost to outermost when resolving a name.

- **Global scope** -- the top-level program scope. All top-level `let`, `const`, `fn`,
  `enum`, and `struct` definitions live here.
- **Block scope** -- each `{ ... }` block pushes a new scope. Variables defined inside a
  block are not visible outside it.
- **Function scope** -- each function call pushes a fresh scope containing the function's
  parameters. The function body's statements execute in this scope.

### 6.2 Variable Shadowing

A binding in an inner scope may shadow a binding with the same name in an outer scope. The
outer binding is restored when the inner scope exits.

### 6.3 Mutability

Variables declared with `let` are immutable; reassignment is a runtime error. Variables
declared with `let mut` may be reassigned via `=` or compound assignment operators.
Constants declared with `const` are also immutable.

### 6.4 Closure Capture Semantics

Lambdas capture their enclosing scope **by value** at the moment of lambda creation. All
visible variables in the scope chain are snapshot-copied into the closure. This means:

- Mutations to outer variables after a lambda is created are **not** reflected inside the
  lambda.
- The lambda holds its own independent copy of captured values.
- Captured variables retain their mutability flag, so mutable captures can be mutated
  inside the closure (but those mutations do not propagate back to the outer scope).

### 6.5 Module Scope

Module definitions (`mod name { ... }`) create an isolated scope. Items inside a module are
accessed via `use module_name::item_name` or `use module_name::*`.

### 6.6 Function Hoisting

Function definitions (`fn`, `async fn`) and enum/struct definitions are registered before
statement execution begins (within the module or test that defines them, when executing
that scope). Top-level function definitions are available throughout the program.

---

## 7. Pattern Matching

Patterns are used in `match` arms. They are tested top-to-bottom; the first matching arm
executes.

### 7.1 Pattern Forms

```ebnf
pattern = literal_pattern
        | variable_pattern
        | wildcard_pattern
        | array_pattern
        | map_pattern
        | or_pattern
        | rest_pattern
        | enum_pattern
        | type_pattern
        | range_pattern ;
```

#### Literal Pattern

Matches if the value equals the literal:

```magi
42          // matches integer 42
3.14        // matches float 3.14
"hello"     // matches string "hello"
true        // matches boolean true
null        // matches null
-5          // negative literal
```

#### Variable Pattern

Binds the matched value to a new variable name in the arm's scope:

```magi
x           // binds value to x
n           // binds value to n
```

Any identifier that is not a keyword or wildcard is treated as a variable pattern.

#### Wildcard Pattern

Matches any value without binding:

```magi
_           // matches anything, discards the value
```

#### Array Pattern

Matches an array value element-by-element:

```magi
[a, b, c]           // matches 3-element array
[first, ...rest]    // first element + rest
[_, _, third]       // skip first two
```

#### Map Pattern

Matches a map value by key:

```magi
{x, y}              // shorthand: binds map["x"] to x, map["y"] to y
{name: n, age: a}   // binds map["name"] to n, map["age"] to a
```

#### Or Pattern

Matches if any of the sub-patterns match:

```magi
1 | 2 | 3           // matches 1, 2, or 3
"yes" | "y"         // matches either string
```

#### Rest Pattern

Used inside array patterns to match remaining elements:

```magi
[first, ...rest]    // rest gets remaining elements
[head, ...]         // anonymous rest (discard remaining)
```

#### Enum Pattern

Matches an enum variant and optionally destructures its fields:

```magi
Result::Ok(value)           // binds the Ok payload to value
Result::Err(_)              // matches Err, discards payload
Shape::Circle(r)            // binds radius to r
Option::None                // matches unit variant
```

#### Type Pattern

Matches if the value is of the specified type, and binds it:

```magi
n: int64            // matches if value is int64, binds to n
s: string           // matches if value is string, binds to s
_n: int64           // underscore-prefixed to indicate unused
```

#### Range Pattern

Matches if the value falls within the specified range:

```magi
0..10               // matches integers 0 through 9
0..=10              // matches integers 0 through 10
-10..10             // matches -10 through 9
0.0..1.0            // matches floats in [0.0, 1.0)
```

### 7.2 Guards

Each match arm may have an optional guard -- a boolean expression evaluated after the
pattern matches. The arm only executes if the guard is truthy:

```magi
match value {
    n: int64 if n > 100 => "large",
    n: int64 if n > 0 => "positive",
    _ => "other",
}
```

Guards are parsed with struct literal suppression to avoid ambiguity with the arm body
block.

---

## 8. Standard Library

MAGI includes a built-in standard library of operations accessible via `use std::*`
imports or direct function calls. The standard library includes:

- **String operations** -- `len`, `trim`, `to_upper`, `to_lower`, `split`, `join`,
  `contains`, `starts_with`, `ends_with`, `replace`, `chars`, `reverse`, `repeat`,
  `substring`, `to_string`, etc.
- **Numeric operations** -- `abs`, `round`, `floor`, `ceil`, `sqrt`, `min`, `max`,
  `pow`, `to_int64`, `to_float64`, `parse_int`, `parse_float`, etc.
- **Array operations** -- `len`, `push`, `pop`, `map`, `filter`, `reduce`, `find`,
  `any`, `all`, `sort`, `sort_by`, `reverse`, `unique`, `flat_map`, `group_by`,
  `partition`, `chunk`, `enumerate`, `zip`, `take_while`, `skip_while`, `scan`,
  `min_by`, `max_by`, `slice`, `concat`, `flatten`, etc.
- **Map operations** -- `keys`, `values`, `entries`, `has_key`, `merge`, etc.
- **Type operations** -- `type_of`, `is_null`, `is_string`, `is_int`, `is_float`,
  `is_bool`, `is_array`, `is_map`, etc.
- **I/O and conversion** -- `to_json`, `from_json`, `to_string`, `output`, etc.
- **Assertion** -- `assert`, `assert_eq` (for test blocks).

For the complete standard library reference, see [stdlib.md](stdlib.md).

---

## 9. Error Handling

MAGI provides structured error handling through `try`/`catch`/`finally`, `throw`, and the
try-propagate operator `?`.

### 9.1 Try/Catch/Finally

```ebnf
try_catch = "try" block "catch" [ identifier ] block [ "finally" block ] ;
```

- The `try` block executes normally.
- If an error is thrown (via `throw` or a runtime error), execution transfers to the
  `catch` block. The optional identifier binds the error value (a string).
- The `finally` block, if present, always executes regardless of whether an error occurred.
- Try/catch can be used as both a statement and an expression.

```magi
try {
    let result = risky_operation();
    output result;
} catch err {
    output f"Error: {err}";
} finally {
    cleanup();
}
```

### 9.2 Throw

```ebnf
throw_stmt = "throw" expression ";" ;
```

Throws an error. The expression is evaluated and its string representation becomes the
error message. Throw immediately transfers control to the nearest enclosing `catch` block.
If no `catch` block is present, the error propagates up the call stack.

```magi
throw "division by zero";
throw f"invalid input: {value}";
```

### 9.3 Try-Propagate (`?`)

```ebnf
try_propagate = expression "?" ;
```

The `?` postfix operator provides concise early-return on null values. If the expression
evaluates to `null`, the enclosing function immediately returns `null`. Otherwise the
non-null value is returned:

```magi
fn get_name(id) {
    let user = find_user(id)?;   // returns null if find_user returns null
    let profile = user.profile?; // returns null if profile is null
    profile.name
}
```

For complete error message reference and error codes, see [errors.md](errors.md).

---

## Appendix: Grammar Summary

The following is a condensed grammar for the MAGI language. Productions use EBNF notation:
`{ x }` means zero or more repetitions, `[ x ]` means optional, `( x | y )` means
alternation.

```ebnf
(* Top level *)
program    = { statement } EOF ;

(* Statements *)
statement  = let_stmt | const_stmt | assignment | compound_assign
           | field_assignment | index_assignment
           | for_stmt | while_stmt
           | fn_def | async_fn_def
           | break_stmt | continue_stmt | return_stmt
           | output_stmt | import_stmt | use_stmt
           | mod_def | type_alias | test_def
           | enum_def | struct_def
           | try_catch_stmt | throw_stmt
           | [ "pub" ] statement
           | expr_stmt ;

(* Expressions -- ordered by precedence, lowest first *)
expression     = pipe_expr ;
pipe_expr      = null_coalesce { "|>" null_coalesce } ;
null_coalesce  = range_expr { "??" range_expr } ;
range_expr     = binary_expr [ (".." | "..=") binary_expr ] ;
binary_expr    = unary_expr { binop unary_expr } ;
unary_expr     = ("!" | "-" | "await" | "spawn") unary_expr | postfix_expr ;
postfix_expr   = primary { call | index | field | method | opt_chain | "?" } ;
primary        = literal | variable | "(" expression ")" | block
               | if_expr | match_expr | loop_expr | try_catch_expr
               | lambda | fstring | enum_construct | struct_construct
               | array_literal | map_literal | list_comp | map_comp
               | spread | "_" ;

(* Patterns *)
pattern        = single_pattern { "|" single_pattern } ;
single_pattern = literal | variable | "_" | array_pattern | map_pattern
               | enum_pattern | type_pattern | range_pattern
               | rest_pattern | "-" (int | float) ;
```
