# Plan C: Self-Hosting — Rewrite MAGI in MAGI

## Goal
Rewrite the MAGI interpreter and compiler in the MAGI language itself, achieving
self-hosting — the hallmark of a mature programming language. Go achieved this in
Go 1.5 (2015), Rust achieved this before 1.0.

## Prerequisites (must be complete before starting)

1. **v1.0 feature depth** — Plan A must be complete
2. **File I/O** — MAGI must read/write files efficiently ✓
3. **String processing** — robust string manipulation ✓
4. **Data structures** — maps, arrays, sets with good performance ✓
5. **Error handling** — try/catch + Result pattern ✓
6. **Module system** — package imports working ✓
7. **Performance** — interpreter must be fast enough to run itself in reasonable time

## Architecture: Bootstrap Chain

```
Stage 0: Current Rust implementation (magi-rs)
    ↓ compiles
Stage 1: MAGI lexer/parser written in MAGI (magi-stage1.magi)
    ↓ run by magi-rs
Stage 2: MAGI type checker written in MAGI (magi-stage2.magi)
    ↓ run by magi-rs
Stage 3: MAGI interpreter written in MAGI (magi-stage3.magi)
    ↓ run by magi-rs → produces self-hosting interpreter
Stage 4: MAGI compiler written in MAGI (magi-stage4.magi)
    ↓ run by Stage 3 → compiles itself to WASM
Stage 5: Self-hosting complete
    ↓ magi-stage3 can run magi-stage3 (interpreter interprets itself)
    ↓ magi-stage4 can compile magi-stage4 (compiler compiles itself)
```

## Phase 1: Lexer in MAGI (~2,000 lines)

### Files to Create
- `bootstrap/lexer.magi` — tokenizer

### Implementation
```magi
struct Token {
    kind: string,
    text: string,
    line: int64,
    column: int64,
}

struct Lexer {
    source: string,
    pos: int64,
    line: int64,
    column: int64,
}

impl Lexer {
    fn new(source: string) -> Lexer { ... }
    fn tokenize() -> array<Token> { ... }
    fn advance() -> string { ... }
    fn peek() -> string { ... }
    fn skip_whitespace() { ... }
    fn lex_number() -> Token { ... }
    fn lex_string() -> Token { ... }
    fn lex_identifier() -> Token { ... }
}
```

### Validation
- Lex the MAGI test suite and compare token output to Rust lexer
- Must produce identical token sequences for all 2,900+ test programs

### Estimated effort: ~2,000 lines of MAGI, ~2 weeks

## Phase 2: Parser in MAGI (~4,000 lines)

### Files to Create
- `bootstrap/ast.magi` — AST node definitions
- `bootstrap/parser.magi` — recursive descent parser

### Implementation
- Port all StatementKind and ExpressionKind variants as enums
- Port recursive descent parser with Pratt precedence climbing
- Port error recovery (parse_v2_recovering)

### Validation
- Parse the MAGI test suite and compare AST output to Rust parser
- Must produce identical ASTs for all valid programs

### Estimated effort: ~4,000 lines of MAGI, ~3 weeks

## Phase 3: Type Checker in MAGI (~3,000 lines)

### Files to Create
- `bootstrap/type_checker.magi` — static analysis
- `bootstrap/errors.magi` — error codes and diagnostics

### Implementation
- Port scope tracking, variable resolution, function signatures
- Port diagnostic emission with error codes
- Port exhaustive match checking

### Estimated effort: ~3,000 lines of MAGI, ~2 weeks

## Phase 4: Interpreter in MAGI (~6,000 lines)

### Files to Create
- `bootstrap/interpreter.magi` — tree-walking interpreter
- `bootstrap/heap.magi` — virtual heap and GC
- `bootstrap/eval.magi` — operation evaluation

### Implementation
- Port the eval_expr / exec_statement dispatch
- Port the virtual heap with mark-and-sweep GC
- Port all 396 stdlib operations (most delegate to built-in functions)
- Port closure capture, scope management, function calls

### Critical challenge: The interpreter must be fast enough to run itself.
A tree-walking interpreter running a tree-walking interpreter will be
~100-1000x slower than the Rust version. This is acceptable for
bootstrapping but not for production use.

### Estimated effort: ~6,000 lines of MAGI, ~4 weeks

## Phase 5: WASM Compiler in MAGI (~5,000 lines)

### Files to Create
- `bootstrap/compiler.magi` — AST to IR compilation
- `bootstrap/ir.magi` — intermediate representation
- `bootstrap/wasm_codegen.magi` — IR to WASM binary

### Implementation
- Port IR instruction set
- Port AST-to-IR compilation passes
- Port WASM binary encoding (wasm-encoder equivalent)
- Port NaN-boxing tagged value scheme

### Estimated effort: ~5,000 lines of MAGI, ~4 weeks

## Phase 6: Self-Hosting Validation

### Tests
1. Stage 3 interpreter runs the full test suite → all tests pass
2. Stage 3 interpreter runs Stage 3 interpreter → produces correct output
3. Stage 4 compiler compiles Stage 4 compiler → produces valid WASM
4. Compiled WASM of Stage 4 produces identical output to interpreted Stage 4

### Performance Benchmarks
- Measure overhead: self-hosted vs Rust implementation
- Target: < 50x slowdown for interpreter-on-interpreter
- Target: identical performance for WASM-compiled output

## Summary

| Phase | Component | Lines (MAGI) | Duration |
|-------|-----------|-------------|----------|
| 1 | Lexer | ~2,000 | 2 weeks |
| 2 | Parser | ~4,000 | 3 weeks |
| 3 | Type Checker | ~3,000 | 2 weeks |
| 4 | Interpreter | ~6,000 | 4 weeks |
| 5 | WASM Compiler | ~5,000 | 4 weeks |
| 6 | Validation | ~1,000 | 1 week |
| **Total** | **Self-hosting** | **~21,000** | **~16 weeks** |

## Milestones

- **M1**: MAGI lexer can lex itself ← proof of concept
- **M2**: MAGI parser can parse itself ← viability confirmed
- **M3**: MAGI type checker can check itself ← semantic correctness
- **M4**: MAGI interpreter can run "hello world" ← bootstrap achieved
- **M5**: MAGI interpreter passes full test suite ← feature parity
- **M6**: MAGI interpreter runs itself ← self-hosting achieved
- **M7**: MAGI compiler compiles itself to WASM ← full bootstrap

## Dependencies on Plan A

Self-hosting requires these Plan A features to be complete first:
- Struct methods and impl blocks ✓
- Pattern matching with exhaustiveness ✓
- Error handling (try/catch + Result) ✓
- File I/O ✓
- String methods (all 30+) ✓
- Array methods (all 37+) ✓
- Map methods (all 13+) ✓
- Enum discriminants (Phase 1.6 of Plan A) — needed for AST node types
- Const evaluation (Phase 1.5 of Plan A) — needed for constants
- Iterator depth (Phase 2 of Plan A) — needed for efficient data processing

## Why Self-Host?

1. **Language maturity proof** — if the language can implement itself, it's complete
2. **Dogfooding** — using MAGI to build MAGI exposes every rough edge
3. **Independence** — no longer dependent on Rust toolchain for development
4. **Compilation story** — MAGI-compiled-to-WASM runs anywhere, no Rust needed
5. **Community signal** — self-hosting is the standard bar for serious languages
