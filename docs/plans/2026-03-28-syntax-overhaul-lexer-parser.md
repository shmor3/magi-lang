# Syntax Overhaul — Lexer, Parser, AST Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Update the MAGI lexer, parser, and AST to implement the new syntax spec (func, import, interface, arrow functions, no try/catch/impl/trait/output/fn/loop).

**Architecture:** Modify src/syntax/lexer.rs, src/syntax/parser.rs, src/syntax/ast.rs in place. The lexer adds/removes/renames tokens. The parser adds new parse rules and removes old ones. The AST adds new node types and removes old ones. Each task is a single token or syntax change, tested immediately.

**Tech Stack:** Rust, cargo test --lib

**Spec reference:** `docs/specs/2026-03-28-syntax-overhaul-design.md`

**Testing:** After each task, run `cargo build --bin magi 2>&1 | grep "^error"` to verify compilation. Full test suite runs on dedicated server after all tasks complete. Individual syntax tests added per task.

**Note:** This plan changes the LANGUAGE SYNTAX. After this plan, existing code using old syntax will NOT compile. Test migration is a separate plan.

---

## File Structure

```
Modify: src/syntax/lexer.rs      — Token additions/removals/renames
Modify: src/syntax/parser.rs     — New parse rules
Modify: src/syntax/ast.rs        — New AST node types
Modify: src/syntax/errors.rs     — Updated error messages
```

---

### Task 1: Rename `Fn` token to `Func`

**Files:**
- Modify: `src/syntax/lexer.rs`
- Modify: `src/syntax/parser.rs`
- Modify: All files that reference `TokenKind::Fn`

- [ ] **Step 1: Add `Func` token, keep `Fn` as alias**

In `src/syntax/lexer.rs`, add `Func` to TokenKind enum alongside `Fn`:
```rust
Func,     // func (new syntax)
```

In the keyword matching section, add:
```rust
"func" => TokenKind::Func,
```

- [ ] **Step 2: Update parser to accept both `Fn` and `Func`**

In `src/syntax/parser.rs`, everywhere that checks `TokenKind::Fn`, also accept `TokenKind::Func`:
```rust
TokenKind::Fn | TokenKind::Func => { ... }
```

- [ ] **Step 3: Build and verify**

Run: `cargo build --bin magi 2>&1 | grep "^error" | head -5`
Expected: Zero errors

- [ ] **Step 4: Add test**

Add to `tests/integration.rs`:
```rust
#[test]
fn test_func_keyword() {
    let result = run_eval_unique("func add(a, b) { a + b } println(add(1, 2))", "func_kw");
    assert!(result.contains("3"));
}
```

- [ ] **Step 5: Commit**
```bash
git add -A && git commit -m "syntax: add func keyword (fn still accepted)" && git push origin main
```

---

### Task 2: Add `Interface` token, keep `Trait` as alias

**Files:**
- Modify: `src/syntax/lexer.rs`
- Modify: `src/syntax/parser.rs`

- [ ] **Step 1: Add token**

In lexer.rs TokenKind enum:
```rust
Interface, // interface (replaces trait)
```

In keyword matching:
```rust
"interface" => TokenKind::Interface,
```

- [ ] **Step 2: Update parser to accept both**

Everywhere `TokenKind::Trait` is checked, also accept `TokenKind::Interface`.

- [ ] **Step 3: Build and verify**

- [ ] **Step 4: Add test**
```rust
#[test]
fn test_interface_keyword() {
    let result = run_eval_unique(r#"
        interface Greetable { func greet(self) -> string }
        struct Dog { name: string }
        func Dog.greet(self) { f"Woof! I'm {self.name}" }
        println("interface defined")
    "#, "interface_kw");
    assert!(result.contains("interface defined"));
}
```

- [ ] **Step 5: Commit**

---

### Task 3: Add `import` statement parsing (alongside `use`)

**Files:**
- Modify: `src/syntax/lexer.rs`
- Modify: `src/syntax/parser.rs`
- Modify: `src/syntax/ast.rs`

- [ ] **Step 1: Add Import AST node**

In `src/syntax/ast.rs`, add to StatementKind:
```rust
/// import std.math
/// import std.{fs, json}
/// import std.math as m
ImportModule {
    path: Vec<String>,       // ["std", "math"]
    alias: Option<String>,   // Some("m") for `as m`
    multi: Vec<String>,      // ["fs", "json"] for { } imports
},
```

- [ ] **Step 2: Parse `import` statements in parser**

In `src/syntax/parser.rs`, in the statement parsing section, add:
```rust
TokenKind::Import => {
    self.advance(); // consume 'import'
    // Parse dotted path: std.math or std.{fs, json}
    let mut path = vec![self.expect_ident()?];
    while self.check(TokenKind::Dot) {
        self.advance(); // consume '.'
        if self.check(TokenKind::LBrace) {
            // Multi-import: std.{fs, json}
            self.advance(); // consume '{'
            let mut names = vec![self.expect_ident()?];
            while self.check(TokenKind::Comma) {
                self.advance();
                names.push(self.expect_ident()?);
            }
            self.expect(TokenKind::RBrace)?;
            return Ok(Statement::new(StatementKind::ImportModule {
                path, alias: None, multi: names,
            }, span));
        }
        path.push(self.expect_ident()?);
    }
    // Check for alias: import std.math as m
    let alias = if self.check(TokenKind::As) {
        self.advance();
        Some(self.expect_ident()?)
    } else { None };
    Ok(Statement::new(StatementKind::ImportModule {
        path, alias, multi: vec![],
    }, span))
}
```

- [ ] **Step 3: Build and verify**

- [ ] **Step 4: Add test**
```rust
#[test]
fn test_import_syntax() {
    let program = parse("import std.math");
    assert!(!program.statements.is_empty());
}

#[test]
fn test_import_multi() {
    let program = parse("import std.{fs, json}");
    assert!(!program.statements.is_empty());
}

#[test]
fn test_import_alias() {
    let program = parse("import std.math as m");
    assert!(!program.statements.is_empty());
}
```

- [ ] **Step 5: Commit**

---

### Task 4: Add arrow function (`=>`) parsing

**Files:**
- Modify: `src/syntax/parser.rs`
- Modify: `src/syntax/ast.rs`

Arrow functions use `=>` which already exists as `FatArrow` for match arms. The parser must disambiguate based on context.

- [ ] **Step 1: Add ArrowFunction to ExpressionKind**

In `src/syntax/ast.rs`:
```rust
/// Arrow function: x => x * 2, (a, b) => a + b
ArrowFunction {
    params: Vec<String>,
    body: Box<Expression>,
},
```

- [ ] **Step 2: Parse arrow functions in expression context**

In the expression parser, after parsing an identifier or parenthesized parameter list, check for `=>`:

```rust
// If we see ident => expr, it's an arrow function
// If we see (a, b) => expr, it's an arrow function
```

Key: Arrow functions are parsed at the LOWEST precedence level (per spec — precedence 15, just above assignment).

- [ ] **Step 3: Build and verify**

- [ ] **Step 4: Add test**
```rust
#[test]
fn test_arrow_function_single() {
    let program = parse("const f = x => x * 2");
    assert!(!program.statements.is_empty());
}

#[test]
fn test_arrow_function_multi_param() {
    let program = parse("const f = (a, b) => a + b");
    assert!(!program.statements.is_empty());
}

#[test]
fn test_arrow_function_block() {
    let program = parse("const f = (x) => { const y = x + 1; y }");
    assert!(!program.statements.is_empty());
}
```

- [ ] **Step 5: Commit**

---

### Task 5: Add dot receiver function parsing

**Files:**
- Modify: `src/syntax/parser.rs`
- Modify: `src/syntax/ast.rs`

- [ ] **Step 1: Add ReceiverFunction to AST**

In `src/syntax/ast.rs`, modify FunctionDef or add:
```rust
/// func Vec2.length(self) { ... }
ReceiverFunction {
    type_name: String,      // "Vec2"
    method_name: String,    // "length"
    params: Vec<FunctionParam>,  // includes self
    body: Block,
    type_params: Vec<String>,    // generics: <T>
},
```

- [ ] **Step 2: Parse `func Type.method(self)` syntax**

In the function definition parser, after consuming `func` (or `fn`), check if the next pattern is `Ident.Ident(`:
```rust
// func Vec2.length(self) { ... }
// func Stack<T>.push(self, item T) { ... }
if self.peek_is(TokenKind::Dot) {
    // Receiver function
    let type_name = name;
    self.advance(); // consume '.'
    let method_name = self.expect_ident()?;
    // Parse params, body as normal
}
```

- [ ] **Step 3: Build and verify**

- [ ] **Step 4: Add test**
```rust
#[test]
fn test_dot_receiver_parse() {
    let program = parse("struct Foo { x: int } func Foo.get_x(self) { self.x }");
    assert!(!program.statements.is_empty());
}
```

- [ ] **Step 5: Commit**

---

### Task 6: Add `println`/`print` as expression builtins

**Files:**
- Modify: `src/syntax/parser.rs` (recognize as function calls, not statements)
- Modify: `src/syntax/interpreter.rs` (dispatch print/println as builtins returning their argument)

- [ ] **Step 1: Add println/print to interpreter builtins**

In the function call dispatch of `src/syntax/interpreter.rs`, add:
```rust
"println" => {
    let val = if args.is_empty() { DataType::Null } else { self.eval_expr(&args[0])? };
    let s = val.to_string_lossy();
    self.logs.push(s.clone());
    println!("{}", s);
    return Ok(val); // returns the argument, not null
}
"print" => {
    let val = if args.is_empty() { DataType::Null } else { self.eval_expr(&args[0])? };
    let s = val.to_string_lossy();
    print!("{}", s);
    return Ok(val);
}
```

- [ ] **Step 2: Keep `output` working for now**

Don't remove `output` yet — keep both syntaxes working so existing tests don't break during migration.

- [ ] **Step 3: Build and verify**

- [ ] **Step 4: Add test**
```rust
#[test]
fn test_println_returns_value() {
    let r = run("const x = println(42); x");
    // println returns its argument
    assert_eq!(r, DataType::Int64(42));
}
```

- [ ] **Step 5: Commit**

---

### Task 7: Add multi-return value support

**Files:**
- Modify: `src/syntax/parser.rs`
- Modify: `src/syntax/ast.rs`
- Modify: `src/syntax/interpreter.rs`

- [ ] **Step 1: Add MultiReturn to ExpressionKind**

```rust
/// a, b   (bare multi-return expression)
MultiReturn(Vec<Expression>),
```

- [ ] **Step 2: Add MultiAssign to StatementKind**

```rust
/// let a, b = expr
/// const x, y = func_call()
MultiAssign {
    names: Vec<String>,
    mutable: Vec<bool>,  // which are let vs const
    value: Expression,
},
```

- [ ] **Step 3: Parse multi-assignment**

When parsing `let` or `const` and seeing `ident, ident = ...`, parse as multi-assign.

- [ ] **Step 4: Interpret multi-return**

In the interpreter, when a function returns a Tuple or Array, destructure into multiple bindings.

- [ ] **Step 5: Build and verify**

- [ ] **Step 6: Add test**
```rust
#[test]
fn test_multi_return() {
    let result = run_eval_unique(r#"
        func divide(a, b) {
            if b == 0 { return 0, "division by zero" }
            a / b, null
        }
        let result, err = divide(10, 2)
        println(result)
        println(err)
    "#, "multi_return");
    assert!(result.contains("5"));
    assert!(result.contains("null"));
}
```

- [ ] **Step 7: Commit**

---

### Task 8: Remove old tokens (make them parse errors)

**Files:**
- Modify: `src/syntax/lexer.rs`

After Tasks 1-7 are working with new syntax:

- [ ] **Step 1: Remove keyword tokens**

Change these keywords to emit `Reserved` token (which produces a helpful error):
- `output` → Reserved (use println)
- `throw` → Reserved
- `try` → Reserved
- `catch` → Reserved
- `finally` → Reserved
- `loop` → Reserved (use while true)
- `unsafe` → Reserved
- `yield` → Reserved
- `ref` → Reserved
- `move` → Reserved
- `where` → Reserved
- `dyn` → Reserved
- `pub` → Reserved
- `mod` → Reserved

Keep as aliases (transition period):
- `fn` → still works, same as `func`
- `trait` → still works, same as `interface`
- `use` → still works, same as `import` (different syntax though)
- `impl` → Reserved (no replacement)

- [ ] **Step 2: Build and verify**

- [ ] **Step 3: Commit**

---

### Task 9: Optional semicolons

**Files:**
- Modify: `src/syntax/parser.rs`

- [ ] **Step 1: Make semicolons optional**

In the statement terminator logic, don't require `;`. Treat newlines as statement terminators. Allow `;` but don't require it.

The parser already handles most cases. Key change: don't emit "expected semicolon" errors when a newline follows a complete statement.

- [ ] **Step 2: Build and verify**

- [ ] **Step 3: Add test**
```rust
#[test]
fn test_no_semicolons() {
    let result = run_eval_unique(r#"
        const x = 5
        const y = 10
        println(x + y)
    "#, "no_semi");
    assert!(result.contains("15"));
}
```

- [ ] **Step 4: Commit**

---

### Task 10: Clean enum display

**Files:**
- Modify: `src/types/mod.rs` (DataType Display impl)
- Modify: `src/syntax/interpreter.rs` (enum value display)

- [ ] **Step 1: Update enum display format**

When printing an enum value, instead of `{__enum: Color, __variant: Red, __data: []}`, output `Color::Red`. For data variants, output `Color::Rgb(255, 0, 0)`.

Find the `to_string_lossy()` or `Display` implementation for enum DataType values and update the formatting.

- [ ] **Step 2: Build and verify**

- [ ] **Step 3: Add test**
```rust
#[test]
fn test_enum_display() {
    let result = run_eval_unique(r#"
        enum Color { Red, Green, Rgb(int, int, int) }
        println(Color::Red)
        println(Color::Rgb(255, 0, 0))
    "#, "enum_display");
    assert!(result.contains("Color::Red"));
    assert!(result.contains("Color::Rgb(255, 0, 0)"));
}
```

- [ ] **Step 4: Commit**

---

## Acceptance Criteria

After all 10 tasks:
1. `func` keyword works for function definitions
2. `interface` keyword works for interface declarations
3. `import std.math` syntax parses correctly
4. Arrow functions `x => x * 2` parse and execute
5. Dot receiver `func Vec2.length(self)` parses
6. `println()`/`print()` work as expressions returning their argument
7. Multi-return `let a, b = f()` works
8. Old keywords (`output`, `throw`, `try`, `catch`, `loop`, etc.) produce errors
9. Semicolons are optional
10. Enums display as `Type::Variant` not `{__enum: ...}`
11. `cargo build --bin magi` produces zero errors, zero warnings

## Next Plans

After this plan completes:
- **Plan 2**: Interpreter + Evaluator changes (module namespacing, receiver dispatch, interface satisfaction)
- **Plan 3**: Type checker updates
- **Plan 4**: Test migration (3,263 tests)
- **Plan 5**: Documentation updates
