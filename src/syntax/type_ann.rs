//! Type annotation AST nodes for the MAGI language.
//!
//! Supports generic types (`array<int64>`), union types (`int64 | string`),
//! optional types (`?int64`), function types (`fn(int64) -> int64`),
//! and tuple types (`(int64, string)`).

use std::fmt;

/// A type annotation in the source code.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeAnnotation {
    /// A simple named type: `int64`, `string`, `bool`
    Simple(String),
    /// A generic/parameterized type: `array<int64>`, `map<string, bool>`
    Generic {
        base: String,
        params: Vec<TypeAnnotation>,
    },
    /// A union type: `int64 | string`
    Union(Vec<TypeAnnotation>),
    /// An optional type: `?int64` (shorthand for `int64 | null`)
    Optional(Box<TypeAnnotation>),
    /// A function type: `fn(int64, int64) -> int64`
    Function {
        params: Vec<TypeAnnotation>,
        return_type: Box<TypeAnnotation>,
    },
    /// A tuple type: `(int64, string)`
    Tuple(Vec<TypeAnnotation>),
}

impl fmt::Display for TypeAnnotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeAnnotation::Simple(name) => write!(f, "{}", name),
            TypeAnnotation::Generic { base, params } => {
                write!(f, "{}<", base)?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ">")
            }
            TypeAnnotation::Union(types) => {
                for (i, t) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", t)?;
                }
                Ok(())
            }
            TypeAnnotation::Optional(inner) => write!(f, "?{}", inner),
            TypeAnnotation::Function {
                params,
                return_type,
            } => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", return_type)
            }
            TypeAnnotation::Tuple(types) => {
                write!(f, "(")?;
                for (i, t) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", t)?;
                }
                write!(f, ")")
            }
        }
    }
}
