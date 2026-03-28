# MAGI Syntax — Open Gaps

All unresolved syntax questions. Every one must be answered before implementation.

## From Initial Review

### 1. For Loop Syntax
Keep C-style `for`? Or only `for x in iterable` + `for i in 0..10`?

### 2. Destructuring
Does destructuring work with `const` and `let`? In function params? In for loops?

### 3. Map Type Syntax
`map[string]int`, `{string: int}`, or `Map<string, int>`?

### 4. Array Type Syntax
`[int]`, `[]int`, or `Array<int>`?

### 5. Multi-Return Type Annotation
`func f() -> (int, string)` or `func f() -> int, string`?

### 6. Primitive Type Names
Lowercase `int`/`float`/`string`/`bool` or capitalized?

### 7. Number Type Granularity
All six (Int32/Int64/Uint32/Uint64/Float32/Float64) or just `int` + `float`?

### 8. Match Arm vs Arrow Function Ambiguity
`=>` used for both. Is `x => x + 1` inside a match a pattern or lambda?

### 9. Comments
`//` and `/* */` stay? Doc comments `///`?

### 10. Entry Point
Explicit `main()` call or auto-invoke if `func main()` defined?

### 11. Struct Update Syntax
`Config { ...default, port: 443 }` — confirm spread in structs stays.

### 12. Function Parameter Mutability
Are params `const` (immutable) or `let` (mutable) by default?

## Dropped Features — Keep or Confirm Removal

### 13. Do-While Loops
Current: `do { } while cond`. Keep or remove?

### 14. Labeled Break/Continue
Current: `'outer: for x in xs { break 'outer }`. Keep or remove?

### 15. Static Globals
Current: `static COUNTER: int = 0`. Keep or remove?

### 16. Sets, Tuples, Bytes
First-class types in current MAGI. Staying?

### 17. Attributes/Decorators
Current: `#[test]`, `#[deprecated]`, `#[ignore]`, `#[cfg(...)]`. Staying?

### 18. Unsafe Blocks
Current: `unsafe { }`. Keep or remove?

### 19. Yield/Generators
Current: `yield value`. Keep or remove?

### 20. Ref/Move/Dyn Keywords
Current: `ref`, `move`, `dyn`. Keep or remove?

### 21. Where Clauses
Current: `func f<T>() where T: Display`. Keep or remove? (Overhaul only shows `<T: Interface>`)

### 22. Multiline and Raw Strings
Current: `"""multiline"""` and `r"raw\n"`. Staying?

### 23. Mod Blocks
Current: `mod utils { }`. Removed since we have `import`?

## Edge Cases

### 24. Pipe with Multi-Return
`getData()` returns `(data, err)`. How does `|>` work? Only first value piped?

### 25. Print Return Value
`println(x)` returns `x`. Is it the original value or the string representation?

### 26. Methods on Primitive Types
Can you do `func int.is_even(self) { self % 2 == 0 }`?

### 27. Multiple Generic Bounds
`<T: Display & Compare>` or `<T: Display, T: Compare>` or `<T: (Display, Compare)>`?

### 28. Struct Field Mutability
`const v = Vec2{x: 3.0, y: 4.0}` — can you do `v.x = 5.0`? Binding-level or field-level?

### 29. Null vs Uninitialized
`let x: int` — is x null? Zero? Compile error?

### 30. Interface Method Resolution
If Reader and Writer both have `close()`, and ReadWriter embeds both, which `close()` is called?

### 31. Spread in Arrays
`const arr = [...a, ...b, 3]` — spread works in array literals?

### 32. Interface Case Sensitivity
`func read()` satisfies `interface { func read() }` but `func Read()` does not?

### 33. Operator Precedence with `=>`
Is `a > b => x` parsed as `(a > b) => x` or `a > (b => x)`?

### 34. Empty Return with Multi-Return
`func f() -> (int, string) { return }` — error or defaults to `(null, null)`?

### 35. Defer with Error State
`defer` runs on function exit but can't see if an error occurred. Is this a problem?

### 36. Async Multi-Return
`await fetch()` where fetch returns `(data, err)` — does await unwrap the future AND give you the tuple?

### 37. Comprehensions with Multi-Return
`[x for x in getData()]` where `getData()` returns `(arr, err)` — error or auto-unwrap first value?

### 38. Receiver on Interface Types
Can you write `func Shape.display(self) { }`? Shape is an interface, not a concrete type.

### 39. Recursive Type Aliases
`type Node = { value: int, children: [Node] }` — allowed?
