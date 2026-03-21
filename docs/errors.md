# MAGI Error Code Reference

## Error Codes (Exx)

| Code | Category | Description |
|------|----------|-------------|
| E100 | Error | Type mismatch: expected X, got Y |
| E101 | Error | Expected Bool in condition (if/while/assert) |
| E102 | Error | Expected Array for iteration (for..in) |
| E103 | Error | Arithmetic overflow or invalid argument type |
| E104 | Error | Division/modulo by zero |
| E105 | Error | Negative array index |
| E106 | Error | Index out of bounds on empty array literal |
| E107 | Error | Duplicate map keys |
| E200 | Error | Undefined variable |
| E201 | Error | Undefined function |
| E202 | Error | Unknown operation |
| E203 | Error | Module not found |
| E300 | Error | `break` outside loop |
| E301 | Error | `continue` outside loop |
| E302 | Error | `return` outside function |
| E303 | Error | Placeholder `_` outside pipe |
| E304 | Error | Invalid pipe stage (not a function call) |
| E400 | Error | Max loop iterations exceeded |
| E401 | Error | Max call depth exceeded (recursion) |
| E402 | Error | Assertion failed |
| E403 | Error | Uncaught user-thrown error (throw) |
| E404 | Error | Assignment to immutable variable |
| E405 | Error | Arity mismatch (wrong number of arguments) |
| E406 | Error | Eval/operation error |
| E407 | Error | Execution cancelled |
| E408 | Error | Feature not implemented |
| E409 | Error | Resource limit exceeded (string/array size) |
| ErrorCode::E100 => "E100" | Error |  |
| ErrorCode::E101 => "E101" | Error |  |
| ErrorCode::E102 => "E102" | Error |  |
| ErrorCode::E103 => "E103" | Error |  |
| ErrorCode::E104 => "E104" | Error |  |
| ErrorCode::E105 => "E105" | Error |  |
| ErrorCode::E106 => "E106" | Error |  |
| ErrorCode::E107 => "E107" | Error |  |
| ErrorCode::E200 => "E200" | Error |  |
| ErrorCode::E201 => "E201" | Error |  |
| ErrorCode::E202 => "E202" | Error |  |
| ErrorCode::E203 => "E203" | Error |  |
| ErrorCode::E300 => "E300" | Error |  |
| ErrorCode::E301 => "E301" | Error |  |
| ErrorCode::E302 => "E302" | Error |  |
| ErrorCode::E303 => "E303" | Error |  |
| ErrorCode::E304 => "E304" | Error |  |
| ErrorCode::E400 => "E400" | Error |  |
| ErrorCode::E401 => "E401" | Error |  |
| ErrorCode::E402 => "E402" | Error |  |
| ErrorCode::E403 => "E403" | Error |  |
| ErrorCode::E404 => "E404" | Error |  |
| ErrorCode::E405 => "E405" | Error |  |
| ErrorCode::E406 => "E406" | Error |  |
| ErrorCode::E407 => "E407" | Error |  |
| ErrorCode::E408 => "E408" | Error |  |
| ErrorCode::E409 => "E409" | Error |  |
| ErrorCode::W100 => "W100" | Error |  |
| ErrorCode::W101 => "W101" | Error |  |
| ErrorCode::W103 => "W103" | Error |  |
| ErrorCode::W106 => "W106" | Error |  |
| ErrorCode::W107 => "W107" | Error |  |
| ErrorCode::W108 => "W108" | Error |  |
| ErrorCode::W109 => "W109" | Error |  |
| ErrorCode::W110 => "W110" | Error |  |
| ErrorCode::W111 => "W111" | Error |  |
| ErrorCode::W112 => "W112" | Error |  |
| ErrorCode::W113 => "W113" | Error |  |
| ErrorCode::W200 => "W200" | Error |  |
| ErrorCode::W201 => "W201" | Error |  |
| ErrorCode::W202 => "W202" | Error |  |
| ErrorCode::W203 => "W203" | Error |  |
| ErrorCode::W204 => "W204" | Error |  |
| ErrorCode::W205 => "W205" | Error |  |
| ErrorCode::W206 => "W206" | Error |  |
| ErrorCode::W207 => "W207" | Error |  |
| ErrorCode::W208 => "W208" | Error |  |
| ErrorCode::W209 => "W209" | Error |  |
| ErrorCode::W212 => "W212" | Error |  |
| ErrorCode::W214 => "W214" | Error |  |
| ErrorCode::W215 => "W215" | Error |  |
| ErrorCode::W216 => "W216" | Error |  |
| ErrorCode::W229 => "W229" | Error |  |
| ErrorCode::W230 => "W230" | Error |  |
| ErrorCode::W231 => "W231" | Error |  |
| ErrorCode::W233 => "W233" | Error |  |
| ErrorCode::W234 => "W234" | Error |  |
| ErrorCode::W235 => "W235" | Error |  |
| ErrorCode::W236 => "W236" | Error |  |
| ErrorCode::W237 => "W237" | Error |  |
| ErrorCode::E100 => "A value of the wrong type was used where a specific type was expected. Check the types of your variables and ensure they match what the operation or function expects." | Error | Human-readable help text explaining the error and how to fix it. |
| ErrorCode::E101 => "Conditions in `if`, `while`, `!`, and match guards must be boolean (`true`/`false`). If you have a number or string, compare it explicitly: `x != 0` or `s != \"\"`. Note: `&&` and `||` accept any value via truthiness." | Error |  |
| ErrorCode::E102 => "The `for..in` loop requires an iterable (array, map, or string). Use `range(start, end)` for numeric loops, or ensure the value is iterable." | Error |  |
| ErrorCode::E103 => "An arithmetic operation overflowed or received an argument of the wrong type. Check values are within bounds." | Error |  |
| ErrorCode::E104 => "Division or modulo by zero is undefined. Check that your divisor is not zero before the operation." | Error |  |
| ErrorCode::E105 => "Array indices must be non-negative integers. Use `len(arr) - 1` to access the last element, or use slice syntax `arr[-1..]` for negative offsets." | Error |  |
| ErrorCode::E106 => "Attempted to index into an empty array literal. Ensure the array has elements before indexing." | Error |  |
| ErrorCode::E107 => "Map literals cannot have duplicate keys. Remove or rename the duplicate key." | Error |  |
| ErrorCode::E200 => "The variable has not been declared in this scope. Declare it with `let name = value;` before using it." | Error |  |
| ErrorCode::E201 => "The function has not been defined. Check spelling and ensure the function is defined before it is called." | Error |  |
| ErrorCode::E202 => "The operation or method name is not recognized. Check spelling, verify the method exists on the receiver type, or use `use std::module::*` to import standard library functions." | Error |  |
| ErrorCode::E203 => "The module does not exist. Available standard library modules: math, cmp, logic, bits, str, convert, array, map, bytes, json, time, hash, io, control, rand, fs, env, net, tcp, udp, ws, sse, http_server, path, yaml, csv, toml, regex, uuid, crypto, compress, fmt, stats, text, encode, reflect, collections, sort, cert." | Error |  |
| ErrorCode::E300 => "`break` can only be used inside a `for`, `while`, or `loop` block." | Error |  |
| ErrorCode::E301 => "`continue` can only be used inside a `for`, `while`, or `loop` block." | Error |  |
| ErrorCode::E302 => "`return` can only be used inside a function body (`fn` or `async fn`)." | Error |  |
| ErrorCode::E303 => "The placeholder `_` is only valid inside pipe expressions (`|>`)." | Error |  |
| ErrorCode::E304 => "Each stage of a pipe expression must be a function or operation call." | Error |  |
| ErrorCode::E400 => "The loop has run for too many iterations (limit: 10,000). This usually indicates an infinite loop. Check your loop condition." | Error |  |
| ErrorCode::E401 => "Function call depth exceeded the limit (48 levels). This usually indicates infinite recursion. Add a base case to your recursive function." | Error |  |
| ErrorCode::E402 => "An assertion failed. The condition evaluated to `false`. Check the expected values." | Error |  |
| ErrorCode::E403 => "An error was thrown with `throw` and not caught by a `try`/`catch` block." | Error |  |
| ErrorCode::E404 => "Cannot assign to an immutable variable. Declare it with `let mut` to allow reassignment." | Error |  |
| ErrorCode::E405 => "The function was called with the wrong number of arguments. Check the function signature." | Error |  |
| ErrorCode::E406 => "An operation failed during evaluation. Check the input types and values." | Error |  |
| ErrorCode::E407 => "Execution was cancelled by the user or system." | Error |  |
| ErrorCode::E408 => "This feature is not yet implemented in the current version." | Error |  |
| ErrorCode::E409 => "A resource limit was exceeded (e.g. string or array grew too large). Check for unbounded growth in string concatenation, array construction, or similar operations." | Error |  |
| ErrorCode::W100 => "This variable is declared but never used. Prefix it with `_` to suppress this warning, or remove it." | Error |  |
| ErrorCode::W101 => "This import is not used anywhere in the code. Remove the unused import." | Error |  |
| ErrorCode::W103 => "This function is defined but never called. Remove it if it's not needed." | Error |  |
| ErrorCode::W106 => "This operation is redundant (e.g., double negation `--x`, comparing to a boolean literal `x == true`). Simplify the expression." | Error |  |
| ErrorCode::W107 => "This arithmetic operation has a suspicious pattern (e.g., modulo by 1 always returns 0, multiply by 0 always returns 0)." | Error |  |
| ErrorCode::W108 => "The `return` keyword is unnecessary in tail position. The last expression in a block is already the return value." | Error |  |
| ErrorCode::W109 => "This function parameter is never used. Prefix it with `_` to suppress this warning, or remove it." | Error |  |
| ErrorCode::W110 => "This variable is declared as `let mut` but is never reassigned. Use `let` instead." | Error |  |
| ErrorCode::W111 => "This name is a reserved keyword in MAGI. Using it as an identifier may cause issues in future versions." | Error |  |
| ErrorCode::W112 => "The default value type does not match the parameter's type annotation. This may cause unexpected behavior." | Error |  |
| ErrorCode::W113 => "All alternatives in an or-pattern must bind the same set of variable names." | Error |  |
| ErrorCode::W200 => "Function and variable names should use snake_case. Rename `myFunc` to `my_func`." | Error |  |
| ErrorCode::W201 => "Enum and struct names should use PascalCase. Rename `my_enum` to `MyEnum`." | Error |  |
| ErrorCode::W202 => "Code after `return`, `break`, `continue`, or `throw` is unreachable and will never execute. Remove the dead code." | Error |  |
| ErrorCode::W203 => "This match expression may not cover all possible cases. Add missing arms or a wildcard `_` arm." | Error |  |
| ErrorCode::W204 => "The condition is always `true` or `false`. This makes the branch unconditional or dead code." | Error |  |
| ErrorCode::W205 => "Comparing a value to itself is always `true` (for `==`) or `false` (for `!=`, `<`, `>`). This is likely a bug — did you mean to compare to a different value?" | Error |  |
| ErrorCode::W206 => "This block body is empty. Add statements or remove the block." | Error |  |
| ErrorCode::W207 => "This match arm is unreachable because a previous wildcard or variable pattern already matches all values." | Error |  |
| ErrorCode::W208 => "This import path has already been imported. Remove the duplicate import." | Error |  |
| ErrorCode::W209 => "A variable with the same name is already declared in this scope. This shadows the previous binding. Use a different name or remove the redundant declaration." | Error |  |
| ErrorCode::W212 => "Using `return`, `break`, `continue`, or `throw` in a `finally` block overrides the result from `try`/`catch`. This is almost always a bug." | Error |  |
| ErrorCode::W214 => "This `loop` has no `break` statement, so it will run forever. Add a `break` condition or use `while` with a termination condition." | Error |  |
| ErrorCode::W215 => "`if cond { true } else { false }` can be simplified to just `cond`." | Error |  |
| ErrorCode::W216 => "An enum with no variants can never be constructed. Add variants or remove the enum." | Error |  |
| ErrorCode::W229 => "This match arm has an empty body. Add an expression or use `null` explicitly." | Error |  |
| ErrorCode::W230 => "Assigning a variable to itself has no effect. This is likely a bug." | Error |  |
| ErrorCode::W231 => "This `if/else` returns boolean literals that match the condition. Simplify to just the condition expression." | Error |  |
| ErrorCode::W233 => "This code is deeply nested (5+ levels). Consider extracting inner blocks into functions for readability." | Error |  |
| ErrorCode::W234 => "This struct has duplicate field names. Each field name must be unique within a struct definition." | Error |  |
| ErrorCode::W235 => "This enum has duplicate variant names. Each variant name must be unique within an enum definition." | Error |  |
| ErrorCode::W236 => "A TODO or FIXME comment was found. These are reminders of incomplete work." | Error |  |
| ErrorCode::W237 => "A magic number was used directly in code. Consider extracting it into a named constant for clarity." | Error |  |
| ErrorCode::E100, ErrorCode::E101, ErrorCode::E102, ErrorCode::E103 | Error | Returns `Some("did you mean 'closest'?")` if a close match (distance ≤ 3) is found. |
| ErrorCode::E104, ErrorCode::E105, ErrorCode::E106, ErrorCode::E107 | Error |  |
| ErrorCode::E200, ErrorCode::E201, ErrorCode::E202, ErrorCode::E203 | Error |  |
| ErrorCode::E300, ErrorCode::E301, ErrorCode::E302, ErrorCode::E303 | Error |  |
| ErrorCode::E304 | Error |  |
| ErrorCode::E400, ErrorCode::E401, ErrorCode::E402, ErrorCode::E403 | Error |  |
| ErrorCode::E404, ErrorCode::E405, ErrorCode::E406, ErrorCode::E407 | Error |  |
| ErrorCode::E408, ErrorCode::E409 | Error |  |
| ErrorCode::W100, ErrorCode::W101, ErrorCode::W103 | Error |  |
| ErrorCode::W106, ErrorCode::W107 | Error |  |
| ErrorCode::W108, ErrorCode::W109, ErrorCode::W110, ErrorCode::W111 | Error |  |
| ErrorCode::W112, ErrorCode::W113 | Error |  |
| ErrorCode::W200, ErrorCode::W201, ErrorCode::W202, ErrorCode::W203 | Error |  |
| ErrorCode::W204, ErrorCode::W205, ErrorCode::W206, ErrorCode::W207 | Error |  |
| ErrorCode::W208, ErrorCode::W209, ErrorCode::W212 | Error |  |
| ErrorCode::W214, ErrorCode::W215, ErrorCode::W216 | Error |  |
| ErrorCode::W229, ErrorCode::W230, ErrorCode::W231, ErrorCode::W233 | Error |  |
| ErrorCode::W234, ErrorCode::W235 | Error |  |

## Warning Codes (Wxx)

| Code | Category | Description |
|------|----------|-------------|
| W100 | Warning | Unused variable |
| W101 | Warning | Unused import |
| W103 | Warning | Unused function |
| W106 | Warning | Redundant operation (double negation, boolean literal comparison) |
| W107 | Warning | Suspicious arithmetic (modulo by 1, multiply by 0, etc.) |
| W108 | Warning | Unnecessary return in tail position |
| W109 | Warning | Unused function parameter |
| W110 | Warning | Unnecessary `let mut` — variable is never reassigned |
| W111 | Warning | Reserved keyword used as identifier |
| W112 | Warning | Default parameter type mismatch |
| W113 | Warning | Or-pattern alternatives bind different variables |
| W200 | Warning | Naming convention: functions/variables should be snake_case |
| W201 | Warning | Naming convention: enums/structs should be PascalCase |
| W202 | Warning | Dead code after return/break/continue/throw |
| W203 | Warning | Non-exhaustive match (missing enum variants, no wildcard) |
| W204 | Warning | Constant condition in if/while |
| W205 | Warning | Self-comparison (comparing a value to itself) |
| W206 | Warning | Empty block body |
| W207 | Warning | Unreachable match arm after wildcard |
| W208 | Warning | Duplicate import |
| W209 | Warning | Shadowed variable in same scope |
| W212 | Warning | Return/break/continue/throw in finally block |
| W214 | Warning | Infinite loop (loop without break) |
| W215 | Warning | Negated if condition with else branch |
| W216 | Warning | Empty enum definition |
| W229 | Warning | Empty match arm body |
| W230 | Warning | Self-assignment (x = x) |
| W231 | Warning | Redundant boolean if-else |
| W233 | Warning | Deeply nested code |
| W234 | Warning | Duplicate struct field name |
| W235 | Warning | Duplicate enum variant name |
| W236 | Warning | TODO/FIXME comment found |
| W237 | Warning | Magic number in code |

## Detailed Help

### E100: Type mismatch: expected X, got Y

A value of the wrong type was used where a specific type was expected. Check the types of your variables and ensure they match what the operation or function expects.

### E101: Expected Bool in condition (if/while/assert)

Conditions in `if`, `while`, `!`, and match guards must be boolean (`true`/`false`). If you have a number or string, compare it explicitly: `x != 0` or `s != \

### E102: Expected Array for iteration (for..in)

The `for..in` loop requires an iterable (array, map, or string). Use `range(start, end)` for numeric loops, or ensure the value is iterable.

### E103: Arithmetic overflow or invalid argument type

An arithmetic operation overflowed or received an argument of the wrong type. Check values are within bounds.

### E104: Division/modulo by zero

Division or modulo by zero is undefined. Check that your divisor is not zero before the operation.

### E105: Negative array index

Array indices must be non-negative integers. Use `len(arr) - 1` to access the last element, or use slice syntax `arr[-1..]` for negative offsets.

### E106: Index out of bounds on empty array literal

Attempted to index into an empty array literal. Ensure the array has elements before indexing.

### E107: Duplicate map keys

Map literals cannot have duplicate keys. Remove or rename the duplicate key.

### E200: Undefined variable

The variable has not been declared in this scope. Declare it with `let name = value;` before using it.

### E201: Undefined function

The function has not been defined. Check spelling and ensure the function is defined before it is called.

### E202: Unknown operation

The operation or method name is not recognized. Check spelling, verify the method exists on the receiver type, or use `use std::module::*` to import standard library functions.

### E203: Module not found

The module does not exist. Available standard library modules: math, cmp, logic, bits, str, convert, array, map, bytes, json, time, hash, io, control, rand, fs, env, net, tcp, udp, ws, sse, http_server, path, yaml, csv, toml, regex, uuid, crypto, compress, fmt, stats, text, encode, reflect, collections, sort, cert.

### E300: `break` outside loop

`break` can only be used inside a `for`, `while`, or `loop` block.

### E301: `continue` outside loop

`continue` can only be used inside a `for`, `while`, or `loop` block.

### E302: `return` outside function

`return` can only be used inside a function body (`fn` or `async fn`).

### E303: Placeholder `_` outside pipe

The placeholder `_` is only valid inside pipe expressions (`|>`).

### E304: Invalid pipe stage (not a function call)

Each stage of a pipe expression must be a function or operation call.

### E400: Max loop iterations exceeded

The loop has run for too many iterations (limit: 10,000). This usually indicates an infinite loop. Check your loop condition.

### E401: Max call depth exceeded (recursion)

Function call depth exceeded the limit (48 levels). This usually indicates infinite recursion. Add a base case to your recursive function.

### E402: Assertion failed

An assertion failed. The condition evaluated to `false`. Check the expected values.

### E403: Uncaught user-thrown error (throw)

An error was thrown with `throw` and not caught by a `try`/`catch` block.

### E404: Assignment to immutable variable

Cannot assign to an immutable variable. Declare it with `let mut` to allow reassignment.

### E405: Arity mismatch (wrong number of arguments)

The function was called with the wrong number of arguments. Check the function signature.

### E406: Eval/operation error

An operation failed during evaluation. Check the input types and values.

### E407: Execution cancelled

Execution was cancelled by the user or system.

### E408: Feature not implemented

This feature is not yet implemented in the current version.

### E409: Resource limit exceeded (string/array size)

A resource limit was exceeded (e.g. string or array grew too large). Check for unbounded growth in string concatenation, array construction, or similar operations.

### W100: Unused variable

This variable is declared but never used. Prefix it with `_` to suppress this warning, or remove it.

### W101: Unused import

This import is not used anywhere in the code. Remove the unused import.

### W103: Unused function

This function is defined but never called. Remove it if it's not needed.

### W106: Redundant operation (double negation, boolean literal comparison)

This operation is redundant (e.g., double negation `--x`, comparing to a boolean literal `x == true`). Simplify the expression.

### W107: Suspicious arithmetic (modulo by 1, multiply by 0, etc.)

This arithmetic operation has a suspicious pattern (e.g., modulo by 1 always returns 0, multiply by 0 always returns 0).

### W108: Unnecessary return in tail position

The `return` keyword is unnecessary in tail position. The last expression in a block is already the return value.

### W109: Unused function parameter

This function parameter is never used. Prefix it with `_` to suppress this warning, or remove it.

### W110: Unnecessary `let mut` — variable is never reassigned

This variable is declared as `let mut` but is never reassigned. Use `let` instead.

### W111: Reserved keyword used as identifier

This name is a reserved keyword in MAGI. Using it as an identifier may cause issues in future versions.

### W112: Default parameter type mismatch

The default value type does not match the parameter's type annotation. This may cause unexpected behavior.

### W113: Or-pattern alternatives bind different variables

All alternatives in an or-pattern must bind the same set of variable names.

### W200: Naming convention: functions/variables should be snake_case

Function and variable names should use snake_case. Rename `myFunc` to `my_func`.

### W201: Naming convention: enums/structs should be PascalCase

Enum and struct names should use PascalCase. Rename `my_enum` to `MyEnum`.

### W202: Dead code after return/break/continue/throw

Code after `return`, `break`, `continue`, or `throw` is unreachable and will never execute. Remove the dead code.

### W203: Non-exhaustive match (missing enum variants, no wildcard)

This match expression may not cover all possible cases. Add missing arms or a wildcard `_` arm.

### W204: Constant condition in if/while

The condition is always `true` or `false`. This makes the branch unconditional or dead code.

### W205: Self-comparison (comparing a value to itself)

Comparing a value to itself is always `true` (for `==`) or `false` (for `!=`, `<`, `>`). This is likely a bug — did you mean to compare to a different value?

### W206: Empty block body

This block body is empty. Add statements or remove the block.

### W207: Unreachable match arm after wildcard

This match arm is unreachable because a previous wildcard or variable pattern already matches all values.

### W208: Duplicate import

This import path has already been imported. Remove the duplicate import.

### W209: Shadowed variable in same scope

A variable with the same name is already declared in this scope. This shadows the previous binding. Use a different name or remove the redundant declaration.

### W212: Return/break/continue/throw in finally block

Using `return`, `break`, `continue`, or `throw` in a `finally` block overrides the result from `try`/`catch`. This is almost always a bug.

### W214: Infinite loop (loop without break)

This `loop` has no `break` statement, so it will run forever. Add a `break` condition or use `while` with a termination condition.

### W215: Negated if condition with else branch

`if cond { true } else { false }` can be simplified to just `cond`.

### W216: Empty enum definition

An enum with no variants can never be constructed. Add variants or remove the enum.

### W229: Empty match arm body

This match arm has an empty body. Add an expression or use `null` explicitly.

### W230: Self-assignment (x = x)

Assigning a variable to itself has no effect. This is likely a bug.

### W231: Redundant boolean if-else

This `if/else` returns boolean literals that match the condition. Simplify to just the condition expression.

### W233: Deeply nested code

This code is deeply nested (5+ levels). Consider extracting inner blocks into functions for readability.

### W234: Duplicate struct field name

This struct has duplicate field names. Each field name must be unique within a struct definition.

### W235: Duplicate enum variant name

This enum has duplicate variant names. Each variant name must be unique within an enum definition.

### W236: TODO/FIXME comment found

A TODO or FIXME comment was found. These are reminders of incomplete work.

### W237: Magic number in code

A magic number was used directly in code. Consider extracting it into a named constant for clarity.

