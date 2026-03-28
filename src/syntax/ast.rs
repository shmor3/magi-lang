//! AST node types for the MAGI v2 language.
//!
//! This module defines a proper abstract syntax tree that sits between the
//! text representation and the `GraphDef` visual graph. It enables loops,
//! infix operators, blocks, and proper type checking.

use std::fmt;
pub use super::type_ann::TypeAnnotation;

// Span — source location tracking

/// Source location span for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    /// Byte offset of the span start in the source text (0-based).
    pub start_byte: u32,
    /// Byte offset of the span end in the source text (0-based).
    pub end_byte: u32,
    /// Whether this expression is in tail-call position (set by optimizer).
    pub tail_call: bool,
}

impl Span {
    pub fn new(start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> Self {
        Self { start_line, start_col, end_line, end_col, start_byte: 0, end_byte: 0, tail_call: false }
    }

    /// Create a span with byte offset information.
    pub fn with_bytes(start_line: u32, start_col: u32, end_line: u32, end_col: u32, start_byte: u32, end_byte: u32) -> Self {
        Self { start_line, start_col, end_line, end_col, start_byte, end_byte, tail_call: false }
    }

    /// Create a span covering a single point.
    pub fn point(line: u32, col: u32) -> Self {
        Self { start_line: line, start_col: col, end_line: line, end_col: col, start_byte: 0, end_byte: 0, tail_call: false }
    }

    /// Create a span covering a single point with byte offset.
    pub fn point_with_byte(line: u32, col: u32, byte_offset: u32) -> Self {
        Self { start_line: line, start_col: col, end_line: line, end_col: col, start_byte: byte_offset, end_byte: byte_offset, tail_call: false }
    }

    /// Merge two spans into one covering both.
    pub fn merge(self, other: Span) -> Span {
        Span {
            start_line: self.start_line.min(other.start_line),
            start_col: if self.start_line < other.start_line { self.start_col }
                else if self.start_line > other.start_line { other.start_col }
                else { self.start_col.min(other.start_col) },
            end_line: self.end_line.max(other.end_line),
            end_col: if self.end_line > other.end_line { self.end_col }
                else if self.end_line < other.end_line { other.end_col }
                else { self.end_col.max(other.end_col) },
            start_byte: self.start_byte.min(other.start_byte),
            end_byte: self.end_byte.max(other.end_byte),
            tail_call: false,
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}-{}:{}",
            self.start_line, self.start_col, self.end_line, self.end_col
        )
    }
}

// Program — top-level AST node

/// A complete program is a sequence of statements.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
    pub span: Span,
    /// Comments at the end of the file (after the last statement).
    pub trailing_comments: Vec<String>,
}


/// A statement in the program.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
    /// Comments appearing on lines before this statement.
    pub leading_comments: Vec<String>,
    /// A comment appearing on the same line after the statement.
    pub trailing_comment: Option<String>,
}

impl Statement {
    /// Create a new statement with no attached comments.
    pub fn new(kind: StatementKind, span: Span) -> Self {
        Self {
            kind,
            span,
            leading_comments: Vec::new(),
            trailing_comment: None,
        }
    }
}

/// The kind of a statement node, determining its semantics and contained data.
#[derive(Debug, Clone, PartialEq)]
pub enum StatementKind {
    /// `import "plugin-id";` — legacy plugin import (deprecated, use `Use` instead)
    Import(String),
    /// `let name = expr;` or `let name: type = expr;`
    Let {
        name: String,
        type_annotation: Option<TypeAnnotation>,
        value: Expression,
    },
    /// `let mut name = expr;` or `let mut name: type = expr;`
    LetMut {
        name: String,
        type_annotation: Option<TypeAnnotation>,
        value: Expression,
    },
    /// `let [a, b] = expr;` or `let {x, y} = expr;` — destructuring bind
    LetDestructure {
        pattern: DestructurePattern,
        mutable: bool,
        value: Expression,
    },
    /// `name = expr;` — assignment to mutable variable
    Assignment { name: String, value: Expression },
    /// `name += expr;`, `name -= expr;`, etc. — compound assignment
    CompoundAssign {
        name: String,
        op: BinOp,
        value: Expression,
    },
    /// `obj.field = expr;` — field assignment (#7)
    FieldAssignment {
        object: Expression,
        field: String,
        value: Expression,
    },
    /// `obj[index] = expr;` — index assignment (#7)
    IndexAssignment {
        object: Expression,
        index: Expression,
        value: Expression,
    },
    /// `for item in iterable { body }` / `for [a, b] in pairs { }` / `for {k, v} in map { }`
    /// Optionally labeled: `'label: for item in iterable { body }`
    ForLoop {
        label: Option<String>,
        pattern: ForPattern,
        iterable: Expression,
        body: Block,
    },
    /// `while condition { body }`
    /// Optionally labeled: `'label: while condition { body }`
    WhileLoop {
        label: Option<String>,
        condition: Expression,
        body: Block,
    },
    /// `output expr;`
    Output(Expression),
    /// Expression used as a statement (e.g. function call for side effects)
    ExprStatement(Expression),
    /// `fn name(params) -> type { body }`
    FunctionDef(FunctionDef),
    /// `async fn name(params) -> type { body }`
    AsyncFunctionDef(FunctionDef),
    /// `break;` or `break expr;` or `break 'label;` or `break 'label expr;`
    Break {
        label: Option<String>,
        value: Option<Expression>,
    },
    /// `continue;` or `continue 'label;`
    Continue {
        label: Option<String>,
    },
    /// `return;` or `return expr;`
    Return(Option<Expression>),
    /// `try { ... } catch err { ... } finally { ... }`
    TryCatch {
        try_block: Block,
        catch_var: Option<String>,
        catch_block: Block,
        finally_block: Option<Block>,
    },
    /// `throw expr;`
    Throw(Expression),
    /// `const NAME = expr;` or `const NAME: type = expr;`
    ConstDef {
        name: String,
        type_annotation: Option<TypeAnnotation>,
        value: Expression,
    },
    /// `static name: Type = value;` — module-level mutable binding
    StaticDef {
        name: String,
        type_annotation: Option<TypeAnnotation>,
        value: Expression,
        mutable: bool,
    },
    /// `type Name = target;`
    TypeAlias { name: String, target: TypeAnnotation },
    /// `mod name { body }`
    ModuleDef { name: String, body: Block },
    /// `use path::to::item;` or `use path::to::item as alias;` or `use path::to::*;`
    /// When `is_pub` is true, re-exports the item so importers of this module can access it.
    Use {
        path: Vec<String>,
        alias: Option<String>,
        glob: bool,
        is_pub: bool,
    },
    /// `test "description" { body }`
    TestDef { name: String, body: Block },
    /// `enum Name { Variant, Variant(field1, field2) }`
    EnumDef {
        name: String,
        /// Generic type parameters: `enum Option<T> { ... }`
        type_params: Vec<String>,
        variants: Vec<EnumVariant>,
        deprecated: bool,
    },
    /// `struct Name { field: type, ... }`
    StructDef {
        name: String,
        /// Generic type parameters: `struct Box<T> { ... }`
        type_params: Vec<String>,
        fields: Vec<StructField>,
        deprecated: bool,
    },
    /// `impl TypeName { fn method(self, ...) { ... } ... }`
    ImplBlock {
        type_name: String,
        methods: Vec<FunctionDef>,
    },
    /// `trait Name { fn method(self, ...); ... }` or `trait Name: Super1 + Super2 { ... }`
    TraitDef {
        name: String,
        methods: Vec<TraitMethod>,
        supertraits: Vec<String>,
    },
    /// `impl Trait for Type { fn method(self, ...) { ... } ... }`
    ImplTrait {
        trait_name: String,
        type_name: String,
        methods: Vec<FunctionDef>,
    },
    /// `do { body } while condition;`
    DoWhileLoop {
        label: Option<String>,
        body: Block,
        condition: Expression,
    },
    /// `defer expr;` or `defer { block }` -- runs when enclosing scope exits
    Defer(Expression),
    /// C-style for loop: `for (init; condition; update) { body }`
    CStyleFor {
        init: Box<Statement>,
        condition: Expression,
        update: Box<Statement>,
        body: Block,
    },
    /// `(a, b) = (expr1, expr2);` — tuple/swap assignment
    TupleAssignment {
        names: Vec<String>,
        value: Expression,
    },
    /// `name++;` — post-increment statement (desugars to `name += 1`)
    Increment { name: String },
    /// `name--;` — post-decrement statement (desugars to `name -= 1`)
    Decrement { name: String },
    /// `import std.math` / `import std.{fs, json}` / `import std.math as m`
    ImportModule {
        path: Vec<String>,
        alias: Option<String>,
        multi: Vec<String>,
    },
}

/// Pattern for for-loop iteration variable binding.
#[derive(Debug, Clone, PartialEq)]
pub enum ForPattern {
    /// `for x in ...` — single variable
    Single(String),
    /// `for [a, b] in ...` — array destructuring
    ArrayDestructure(Vec<DestructureElement>),
    /// `for {k, v} in ...` — map destructuring
    MapDestructure(Vec<(String, Option<String>)>),
}

/// A variant in an enum definition.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<String>,
    pub span: Span,
}

/// A field in a struct definition.
#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub name: String,
    pub type_annotation: Option<TypeAnnotation>,
    pub default: Option<Expression>,
    pub span: Span,
}

/// A where clause constraint: `T: Display`
#[derive(Debug, Clone, PartialEq)]
pub struct WhereClause {
    pub type_param: String,
    pub bounds: Vec<String>,
}

/// A method signature in a trait definition (no body).
#[derive(Debug, Clone, PartialEq)]
pub struct TraitMethod {
    pub name: String,
    pub params: Vec<FunctionParam>,
    pub return_type: Option<TypeAnnotation>,
    pub span: Span,
}


/// A function parameter with an optional type annotation and optional default value.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionParam {
    pub name: String,
    pub type_annotation: Option<TypeAnnotation>,
    pub default: Option<Expression>,
    pub rest: bool,
    /// `**name` — collects extra named arguments into a Map.
    pub kwargs: bool,
    pub span: Span,
}

/// A user-defined function definition.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDef {
    pub name: String,
    /// Generic type parameters: `fn<T, U>(...)`
    pub type_params: Vec<String>,
    pub params: Vec<FunctionParam>,
    pub return_type: Option<TypeAnnotation>,
    pub body: Block,
    pub span: Span,
    /// Whether this method is a property getter (`get field() { ... }`).
    pub is_getter: bool,
    /// Whether this method is a property setter (`set field(value) { ... }`).
    pub is_setter: bool,
    /// Where clause constraints: `where T: Display, U: Clone`
    pub where_clauses: Vec<WhereClause>,
    /// Whether this function is marked with `#[deprecated]`.
    pub deprecated: bool,
}


/// An expression that produces a value.
#[derive(Debug, Clone, PartialEq)]
pub struct Expression {
    pub kind: ExpressionKind,
    pub span: Span,
}

/// The kind of an expression node, determining how it evaluates to a value.
#[derive(Debug, Clone, PartialEq)]
pub enum ExpressionKind {
    /// Literal value: integer, float, string, bool, null, array, map
    Literal(Literal),
    /// Variable reference
    Variable(String),
    /// Binary operation: `a + b`, `a && b`, etc.
    BinaryOp {
        op: BinOp,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    /// Unary operation: `!x`, `-x`
    UnaryOp { op: UnOp, operand: Box<Expression> },
    /// Function/operation call: `add(x, y)`, `split(text, ",")`
    Call {
        name: String,
        args: Vec<Expression>,
        kwargs: Vec<(String, Expression)>,
    },
    /// Method call: `arr.push(5)`, `str.split(",")`
    MethodCall {
        object: Box<Expression>,
        method: String,
        args: Vec<Expression>,
        kwargs: Vec<(String, Expression)>,
    },
    /// Pipe expression: `expr |> f(_, b)`
    Pipe {
        left: Box<Expression>,
        right: Box<Expression>,
    },
    /// If/else expression: `if cond { a } else { b }`
    IfElse {
        condition: Box<Expression>,
        then_block: Block,
        else_block: Option<Block>,
    },
    /// Block expression: `{ stmts; expr }`
    Block(Block),
    /// Array index: `arr[i]`
    Index {
        object: Box<Expression>,
        index: Box<Expression>,
    },
    /// Field access: `obj.field`
    FieldAccess {
        object: Box<Expression>,
        field: String,
    },
    /// Placeholder for pipe: `_`
    Placeholder,
    /// Range expression: `0..10` or `0..=10`
    Range {
        start: Box<Expression>,
        end: Box<Expression>,
        inclusive: bool,
    },
    /// `await expr`
    Await(Box<Expression>),
    /// `spawn expr` or `spawn { block }`
    Spawn(Box<Expression>),
    /// `yield expr` — yield a value from a generator/iterator
    Yield(Box<Expression>),
    /// `unsafe { body }` — allows raw pointer operations
    UnsafeBlock(Block),
    /// `asm!(template, operands...)` — inline assembly expression
    InlineAsm {
        template: String,
        operands: Vec<Expression>,
    },
    /// `ref x` — reference binding (creates a reference to a value)
    Ref(Box<Expression>),
    /// `move |x| expr` — closure that captures by move
    MoveClosure {
        params: Vec<FunctionParam>,
        body: Box<Expression>,
    },
    /// `dyn Trait` — dynamic trait object type expression
    DynTrait(String),
    /// Lambda/closure: `|x, y| x + y` or `|x| { body }`
    Lambda {
        params: Vec<FunctionParam>,
        body: Box<Expression>,
    },
    /// Match expression: `match value { pattern => body, ... }`
    Match {
        value: Box<Expression>,
        arms: Vec<MatchArm>,
    },
    /// String interpolation: `f"hello {name}"`
    StringInterpolation { parts: Vec<StringPart> },
    /// Null coalescing: `x ?? default` (short-circuit)
    NullCoalesce {
        left: Box<Expression>,
        right: Box<Expression>,
    },
    /// Optional chaining: `obj?.field`
    OptionalChain {
        object: Box<Expression>,
        field: String,
    },
    /// Spread: `...expr` (in array/map literals)
    Spread(Box<Expression>),
    /// `loop { body }` — infinite loop, exits with `break value`
    /// Optionally labeled: `'label: loop { body }`
    Loop {
        label: Option<String>,
        body: Block,
    },
    /// `try { expr } catch var { expr }` — expression form of try/catch
    TryCatchExpr {
        try_block: Block,
        catch_var: Option<String>,
        catch_block: Block,
        finally_block: Option<Block>,
    },
    /// List comprehension: `[expr for pattern in iterable if condition]`
    ListComprehension {
        expr: Box<Expression>,
        pattern: ForPattern,
        iterable: Box<Expression>,
        condition: Option<Box<Expression>>,
    },
    /// Map comprehension: `{key: value for pattern in iterable if condition}`
    MapComprehension {
        key_expr: Box<Expression>,
        value_expr: Box<Expression>,
        pattern: ForPattern,
        iterable: Box<Expression>,
        condition: Option<Box<Expression>>,
    },
    /// Enum construction: `Result::Ok(42)`
    EnumConstruct {
        enum_name: String,
        variant: String,
        args: Vec<Expression>,
    },
    /// Struct construction: `Point { x: 1.0, y: 2.0 }`
    StructConstruct {
        name: String,
        fields: Vec<(String, Expression)>,
    },
    /// Try-propagate: `expr?` — early return on error/null
    TryPropagate(Box<Expression>),
    /// Tuple literal: `a, b` or `a, b, c` — comma-separated values (multi-return)
    TupleLiteral(Vec<Expression>),
}


/// A block of statements with an optional trailing expression (the block's value).
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Statement>,
    /// The final expression (without `;`) that becomes the block's value.
    pub tail_expr: Option<Box<Expression>>,
    pub span: Span,
    /// Comments appearing before the tail expression (if any).
    pub tail_comments: Vec<String>,
}


/// Binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Gt,
    Lt,
    GtEq,
    LtEq,
    And,
    Or,
    In,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    AndNot,
}

impl BinOp {
    /// Return the MAGI operation type name this operator desugars to.
    pub fn operation_name(self) -> &'static str {
        match self {
            BinOp::Add => "add",
            BinOp::Sub => "subtract",
            BinOp::Mul => "multiply",
            BinOp::Div => "divide",
            BinOp::Mod => "modulo",
            BinOp::Eq => "equal",
            BinOp::NotEq => "not_equal",
            BinOp::Gt => "greater",
            BinOp::Lt => "less",
            BinOp::GtEq => "greater_eq",
            BinOp::LtEq => "less_eq",
            BinOp::And => "and",
            BinOp::Or => "or",
            BinOp::In => "contains",
            BinOp::BitAnd => "bit_and",
            BinOp::BitOr => "bit_or",
            BinOp::BitXor => "bit_xor",
            BinOp::Shl => "bit_shift_left",
            BinOp::Shr => "bit_shift_right",
            BinOp::AndNot => "bit_and_not",
        }
    }

    /// Operator precedence (higher = tighter binding).
    pub fn magic_method_name(self) -> &'static str {
        match self {
            BinOp::Add => "__add__", BinOp::Sub => "__sub__", BinOp::Mul => "__mul__",
            BinOp::Div => "__div__", BinOp::Mod => "__mod__", BinOp::Eq => "__eq__",
            BinOp::NotEq => "__ne__", BinOp::Gt => "__gt__", BinOp::Lt => "__lt__",
            BinOp::GtEq => "__ge__", BinOp::LtEq => "__le__", BinOp::And => "__and__",
            BinOp::Or => "__or__", BinOp::In => "__contains__",
            BinOp::BitAnd => "__band__", BinOp::BitOr => "__bor__",
            BinOp::BitXor => "__bxor__", BinOp::Shl => "__shl__",
            BinOp::Shr => "__shr__", BinOp::AndNot => "__bandnot__",
        }
    }

    pub fn precedence(self) -> u8 {
        match self {
            BinOp::Or => 1,
            BinOp::And => 2,
            BinOp::In => 3,
            BinOp::Eq | BinOp::NotEq => 3,
            BinOp::Gt | BinOp::Lt | BinOp::GtEq | BinOp::LtEq => 4,
            BinOp::BitOr => 3,
            BinOp::BitXor => 4,
            BinOp::BitAnd | BinOp::AndNot => 4,
            BinOp::Shl | BinOp::Shr => 5,
            BinOp::Add | BinOp::Sub => 5,
            BinOp::Mul | BinOp::Div | BinOp::Mod => 6,
        }
    }
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Eq => "==",
            BinOp::NotEq => "!=",
            BinOp::Gt => ">",
            BinOp::Lt => "<",
            BinOp::GtEq => ">=",
            BinOp::LtEq => "<=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::In => "in",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::AndNot => "&^",
        };
        write!(f, "{}", s)
    }
}

/// Unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
}

impl UnOp {
    /// Return the MAGI operation type name this operator desugars to.
    pub fn operation_name(self) -> &'static str {
        match self {
            UnOp::Not => "not",
            UnOp::Neg => "negate",
        }
    }
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            UnOp::Not => "!",
            UnOp::Neg => "-",
        };
        write!(f, "{}", s)
    }
}


/// A literal value.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int64(i64),
    Float64(f64),
    String(String),
    Bool(bool),
    Null,
    Array(Vec<Expression>),
    Map(Vec<(String, Expression)>),
    /// `Set(expr, expr, ...)` -- set literal
    Set(Vec<Expression>),
}

// Match arms and patterns

/// A single arm in a match expression.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expression>,
    pub body: Block,
    pub span: Span,
}

/// A pattern for match expressions and destructuring.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// Matches a literal value: `42`, `"hello"`, `true`, `null`
    Literal(Literal),
    /// Binds the matched value to a variable: `x`
    Variable(String),
    /// Matches anything, discards the value: `_`
    Wildcard,
    /// Matches an array: `[a, b, c]` or `[first, ...rest]`
    Array(Vec<Pattern>),
    /// Matches a map: `{x, y}` or `{key: pattern}`
    Map(Vec<(String, Pattern)>),
    /// Matches any of several patterns: `1 | 2 | 3`
    Or(Vec<Pattern>),
    /// Rest/spread pattern in arrays: `...rest`
    Rest(Option<String>),
    /// Enum pattern: `Result::Ok(value)` or `Color::Red`
    EnumPattern {
        enum_name: String,
        variant: String,
        bindings: Vec<Pattern>,
    },
    /// Type pattern: `n: int64` — matches if value is of given type, binds to name
    TypePattern {
        name: String,
        type_name: String,
    },
    /// Binding pattern: `name @ pattern` — binds the matched value to name while also matching pattern
    Binding {
        name: String,
        pattern: Box<Pattern>,
    },
    /// Range pattern: `0..10` or `0..=10` — matches if value is in range
    RangePattern {
        start: Box<Expression>,
        end: Box<Expression>,
        inclusive: bool,
    },
}


/// A part of a string interpolation expression.
#[derive(Debug, Clone, PartialEq)]
pub enum StringPart {
    /// Literal string text between interpolations
    Literal(String),
    /// An embedded expression: `{expr}`
    Expr(Expression),
}

// Destructuring patterns for let bindings

/// Pattern for destructuring let bindings.
#[derive(Debug, Clone, PartialEq)]
pub enum DestructurePattern {
    /// `let [a, b, c] = expr;` or `let [first, ...rest] = expr;`
    Array(Vec<DestructureElement>),
    /// `let {x, y} = expr;` or `let {x: alias} = expr;`
    Map(Vec<(String, Option<String>)>),
    /// `let (a, b) = expr;` — tuple destructuring (desugars same as array)
    Tuple(Vec<DestructureElement>),
}

/// An element in an array destructuring pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum DestructureElement {
    /// A named binding: `a`
    Name(String),
    /// A rest/spread binding: `...rest`
    Rest(String),
}
