//! Error code registry for the MAGI language.
//!
//! Provides stable error codes, help text, and "did you mean?" suggestions.
//! Codes are grouped by category:
//!   E1xx = Type errors
//!   E2xx = Name resolution errors
//!   E3xx = Control flow errors
//!   E4xx = Runtime errors
//!   W1xx = Warnings

use std::fmt;

// =============================================================================
// Error codes
// =============================================================================

/// Stable error code for MAGI diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    // Type errors (E1xx)
    /// Type mismatch: expected X, got Y
    E100,
    /// Expected Bool in condition (if/while/assert)
    E101,
    /// Expected Array for iteration (for..in)
    E102,
    /// Invalid argument type for operation
    E103,
    /// Division/modulo by zero
    E104,
    /// Negative array index
    E105,
    /// Index out of bounds on empty array literal
    E106,
    /// Duplicate map keys
    E107,

    // Name resolution (E2xx)
    /// Undefined variable
    E200,
    /// Undefined function
    E201,
    /// Unknown operation
    E202,
    /// Module not found
    E203,
    /// Item not found in module
    E204,

    // Control flow (E3xx)
    /// `break` outside loop
    E300,
    /// `continue` outside loop
    E301,
    /// `return` outside function
    E302,
    /// Placeholder `_` outside pipe
    E303,
    /// Invalid pipe stage (not a function call)
    E304,

    // Runtime (E4xx)
    /// Max loop iterations exceeded
    E400,
    /// Max call depth exceeded (recursion)
    E401,
    /// Assertion failed
    E402,
    /// Uncaught user-thrown error (throw)
    E403,
    /// Assignment to immutable variable
    E404,
    /// Arity mismatch (wrong number of arguments)
    E405,
    /// Eval/operation error
    E406,
    /// Execution cancelled
    E407,
    /// Feature not implemented
    E408,

    // Warnings (W1xx)
    /// Unused variable
    W100,
    /// Unused import
    W101,
    /// Variable shadowing
    W102,
    /// Unused function
    W103,
    /// Empty loop body
    W104,
    /// Infinite loop (literal true)
    W105,
    /// Redundant operation (self-comparison, double negation, etc.)
    W106,
    /// Suspicious arithmetic (modulo by 1, multiply by 0, etc.)
    W107,
    /// Unnecessary return in tail position
    W108,
    /// Unused function parameter
    W109,
    /// Unnecessary `let mut` — variable is never reassigned
    W110,
    /// Reserved keyword used as identifier
    W111,
    /// Default parameter type mismatch
    W112,
    /// Or-pattern alternatives bind different variables
    W113,

    // Lint warnings (W2xx)
    /// Naming convention: functions/variables should be snake_case
    W200,
    /// Naming convention: enums/structs should be PascalCase
    W201,
    /// Dead code after return/break/continue/throw
    W202,
    /// Non-exhaustive match (missing enum variants, no wildcard)
    W203,
    /// Constant condition in if/while
    W204,
    /// Unnecessary parentheses (reserved)
    W205,
    /// Empty block body
    W206,
    /// Unreachable match arm after wildcard
    W207,
    /// Duplicate import
    W208,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self {
            ErrorCode::E100 => "E100",
            ErrorCode::E101 => "E101",
            ErrorCode::E102 => "E102",
            ErrorCode::E103 => "E103",
            ErrorCode::E104 => "E104",
            ErrorCode::E105 => "E105",
            ErrorCode::E106 => "E106",
            ErrorCode::E107 => "E107",
            ErrorCode::E200 => "E200",
            ErrorCode::E201 => "E201",
            ErrorCode::E202 => "E202",
            ErrorCode::E203 => "E203",
            ErrorCode::E204 => "E204",
            ErrorCode::E300 => "E300",
            ErrorCode::E301 => "E301",
            ErrorCode::E302 => "E302",
            ErrorCode::E303 => "E303",
            ErrorCode::E304 => "E304",
            ErrorCode::E400 => "E400",
            ErrorCode::E401 => "E401",
            ErrorCode::E402 => "E402",
            ErrorCode::E403 => "E403",
            ErrorCode::E404 => "E404",
            ErrorCode::E405 => "E405",
            ErrorCode::E406 => "E406",
            ErrorCode::E407 => "E407",
            ErrorCode::E408 => "E408",
            ErrorCode::W100 => "W100",
            ErrorCode::W101 => "W101",
            ErrorCode::W102 => "W102",
            ErrorCode::W103 => "W103",
            ErrorCode::W104 => "W104",
            ErrorCode::W105 => "W105",
            ErrorCode::W106 => "W106",
            ErrorCode::W107 => "W107",
            ErrorCode::W108 => "W108",
            ErrorCode::W109 => "W109",
            ErrorCode::W110 => "W110",
            ErrorCode::W111 => "W111",
            ErrorCode::W112 => "W112",
            ErrorCode::W113 => "W113",
            ErrorCode::W200 => "W200",
            ErrorCode::W201 => "W201",
            ErrorCode::W202 => "W202",
            ErrorCode::W203 => "W203",
            ErrorCode::W204 => "W204",
            ErrorCode::W205 => "W205",
            ErrorCode::W206 => "W206",
            ErrorCode::W207 => "W207",
            ErrorCode::W208 => "W208",
        };
        write!(f, "{}", code)
    }
}

impl ErrorCode {
    /// Human-readable help text explaining the error and how to fix it.
    pub fn help(&self) -> &'static str {
        match self {
            // Type errors
            ErrorCode::E100 => "A value of the wrong type was used where a specific type was expected. Check the types of your variables and ensure they match what the operation or function expects.",
            ErrorCode::E101 => "Conditions in `if`, `while`, and `assert` must be boolean (`true`/`false`). If you have a number or string, compare it explicitly: `x != 0` or `s != \"\"`.",
            ErrorCode::E102 => "The `for..in` loop requires an iterable (array). Use `range(start, end)` for numeric loops, or ensure the value is an array.",
            ErrorCode::E103 => "The operation received an argument of the wrong type. Check the operation's expected input types.",
            ErrorCode::E104 => "Division or modulo by zero is undefined. Check that your divisor is not zero before the operation.",
            ErrorCode::E105 => "Array indices must be non-negative integers. Use `len(arr) - 1` to access the last element.",
            ErrorCode::E106 => "Attempted to index into an empty array literal. Ensure the array has elements before indexing.",
            ErrorCode::E107 => "Map literals cannot have duplicate keys. Remove or rename the duplicate key.",

            // Name resolution
            ErrorCode::E200 => "The variable has not been declared in this scope. Declare it with `let name = value;` before using it.",
            ErrorCode::E201 => "The function has not been defined. Check spelling and ensure the function is defined before it is called.",
            ErrorCode::E202 => "The operation name is not recognized. Check spelling or use `use std::module::*` to import standard library functions.",
            ErrorCode::E203 => "The module does not exist. Available standard library modules: math, cmp, logic, bits, str, convert, array, map, bytes, json, time, hash, io, control, rand, fs, env, net, tcp, udp, ws, sse, http_server, path, yaml, csv, toml, regex, uuid, crypto, compress, fmt, stats, text, encode, reflect, collections, sort, cert.",
            ErrorCode::E204 => "The item was not found in the specified module. Check the module's available exports.",

            // Control flow
            ErrorCode::E300 => "`break` can only be used inside a `for`, `while`, or `loop` block.",
            ErrorCode::E301 => "`continue` can only be used inside a `for`, `while`, or `loop` block.",
            ErrorCode::E302 => "`return` can only be used inside a function body (`fn` or `async fn`).",
            ErrorCode::E303 => "The placeholder `_` is only valid inside pipe expressions (`|>`).",
            ErrorCode::E304 => "Each stage of a pipe expression must be a function or operation call.",

            // Runtime
            ErrorCode::E400 => "The loop has run for too many iterations (limit: 10,000). This usually indicates an infinite loop. Check your loop condition.",
            ErrorCode::E401 => "Function call depth exceeded the limit (48 levels). This usually indicates infinite recursion. Add a base case to your recursive function.",
            ErrorCode::E402 => "An assertion failed. The condition evaluated to `false`. Check the expected values.",
            ErrorCode::E403 => "An error was thrown with `throw` and not caught by a `try`/`catch` block.",
            ErrorCode::E404 => "Cannot assign to an immutable variable. Declare it with `let mut` to allow reassignment.",
            ErrorCode::E405 => "The function was called with the wrong number of arguments. Check the function signature.",
            ErrorCode::E406 => "An operation failed during evaluation. Check the input types and values.",
            ErrorCode::E407 => "Execution was cancelled by the user or system.",
            ErrorCode::E408 => "This feature is not yet implemented in the current version.",

            // Warnings
            ErrorCode::W100 => "This variable is declared but never used. Prefix it with `_` to suppress this warning, or remove it.",
            ErrorCode::W101 => "This import is not used anywhere in the code. Remove the unused import.",
            ErrorCode::W102 => "A variable with the same name already exists in this scope. The new declaration shadows the previous one, which may be unintentional.",
            ErrorCode::W103 => "This function is defined but never called. Remove it if it's not needed.",
            ErrorCode::W104 => "The loop body is empty. This loop will either run forever or do nothing.",
            ErrorCode::W105 => "This loop has a literal `true` condition and no `break` statement, creating an infinite loop.",
            ErrorCode::W106 => "This operation is redundant (e.g., comparing a value to itself, double negation). Simplify the expression.",
            ErrorCode::W107 => "This arithmetic operation has a suspicious pattern (e.g., modulo by 1 always returns 0, multiply by 0 always returns 0).",
            ErrorCode::W108 => "The `return` keyword is unnecessary in tail position. The last expression in a block is already the return value.",
            ErrorCode::W109 => "This function parameter is never used. Prefix it with `_` to suppress this warning, or remove it.",
            ErrorCode::W110 => "This variable is declared as `let mut` but is never reassigned. Use `let` instead.",
            ErrorCode::W111 => "This name is a reserved keyword in MAGI. Using it as an identifier may cause issues in future versions.",
            ErrorCode::W112 => "The default value type does not match the parameter's type annotation. This may cause unexpected behavior.",
            ErrorCode::W113 => "All alternatives in an or-pattern must bind the same set of variable names.",

            // Lint warnings
            ErrorCode::W200 => "Function and variable names should use snake_case. Rename `myFunc` to `my_func`.",
            ErrorCode::W201 => "Enum and struct names should use PascalCase. Rename `my_enum` to `MyEnum`.",
            ErrorCode::W202 => "Code after `return`, `break`, `continue`, or `throw` is unreachable and will never execute. Remove the dead code.",
            ErrorCode::W203 => "This match expression may not cover all possible cases. Add missing arms or a wildcard `_` arm.",
            ErrorCode::W204 => "The condition is always `true` or `false`. This makes the branch unconditional or dead code.",
            ErrorCode::W205 => "These parentheses are unnecessary and can be removed for cleaner code.",
            ErrorCode::W206 => "This block body is empty. Add statements or remove the block.",
            ErrorCode::W207 => "This match arm is unreachable because a previous wildcard or variable pattern already matches all values.",
            ErrorCode::W208 => "This import path has already been imported. Remove the duplicate import.",
        }
    }
}

// =============================================================================
// "Did you mean?" suggestions via Levenshtein distance
// =============================================================================

/// Compute the Levenshtein edit distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev: Vec<usize> = (0..=b_len).collect();
    let mut curr = vec![0usize; b_len + 1];

    for (i, ca) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b_chars.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j] + cost).min(curr[j] + 1).min(prev[j + 1] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[b_len]
}

/// Suggest the closest matching name from a list of available names.
///
/// Returns `Some("did you mean 'closest'?")` if a close match (distance ≤ 3) is found.
pub fn suggest_name(name: &str, available: &[&str]) -> Option<String> {
    let name_char_len = name.chars().count();
    let max_distance = 3.min(name_char_len / 2 + 1);

    let mut best: Option<(&str, usize)> = None;

    for &candidate in available {
        // Quick skip: length difference too large
        let cand_char_len = candidate.chars().count();
        let len_diff = if name_char_len > cand_char_len {
            name_char_len - cand_char_len
        } else {
            cand_char_len - name_char_len
        };
        if len_diff > max_distance {
            continue;
        }

        let dist = levenshtein(name, candidate);
        if dist > 0 && dist <= max_distance {
            match best {
                None => best = Some((candidate, dist)),
                Some((_, best_dist)) if dist < best_dist => {
                    best = Some((candidate, dist));
                }
                _ => {}
            }
        }
    }

    best.map(|(suggestion, _)| format!("did you mean '{}'?", suggestion))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_display() {
        assert_eq!(ErrorCode::E100.to_string(), "E100");
        assert_eq!(ErrorCode::E200.to_string(), "E200");
        assert_eq!(ErrorCode::W100.to_string(), "W100");
    }

    #[test]
    fn test_error_code_help_non_empty() {
        let codes = [
            ErrorCode::E100, ErrorCode::E101, ErrorCode::E102, ErrorCode::E103,
            ErrorCode::E104, ErrorCode::E105, ErrorCode::E106, ErrorCode::E107,
            ErrorCode::E200, ErrorCode::E201, ErrorCode::E202, ErrorCode::E203,
            ErrorCode::E204,
            ErrorCode::E300, ErrorCode::E301, ErrorCode::E302, ErrorCode::E303,
            ErrorCode::E304,
            ErrorCode::E400, ErrorCode::E401, ErrorCode::E402, ErrorCode::E403,
            ErrorCode::E404, ErrorCode::E405, ErrorCode::E406, ErrorCode::E407,
            ErrorCode::E408,
            ErrorCode::W100, ErrorCode::W101, ErrorCode::W102, ErrorCode::W103,
            ErrorCode::W104, ErrorCode::W105, ErrorCode::W106, ErrorCode::W107,
            ErrorCode::W108, ErrorCode::W109, ErrorCode::W110, ErrorCode::W111,
            ErrorCode::W112, ErrorCode::W113,
            ErrorCode::W200, ErrorCode::W201, ErrorCode::W202, ErrorCode::W203,
            ErrorCode::W204, ErrorCode::W205, ErrorCode::W206, ErrorCode::W207,
            ErrorCode::W208,
        ];
        for code in codes {
            let help = code.help();
            assert!(!help.is_empty(), "{} has empty help text", code);
            assert!(help.len() > 20, "{} help text too short: {}", code, help);
        }
    }

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein("hello", "hello"), 0);
    }

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", ""), 0);
    }

    #[test]
    fn test_levenshtein_one_edit() {
        assert_eq!(levenshtein("cat", "hat"), 1); // substitution
        assert_eq!(levenshtein("cat", "cats"), 1); // insertion
        assert_eq!(levenshtein("cats", "cat"), 1); // deletion
    }

    #[test]
    fn test_levenshtein_multiple_edits() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn test_suggest_name_close_match() {
        let available = ["count", "counter", "total", "sum"];
        let result = suggest_name("counr", &available);
        assert_eq!(result, Some("did you mean 'count'?".to_string()));
    }

    #[test]
    fn test_suggest_name_no_match() {
        let available = ["alpha", "beta", "gamma"];
        let result = suggest_name("xyz", &available);
        assert!(result.is_none());
    }

    #[test]
    fn test_suggest_name_exact_match_not_suggested() {
        let available = ["count"];
        let result = suggest_name("count", &available);
        assert!(result.is_none(), "exact match should not be suggested");
    }

    #[test]
    fn test_suggest_name_prefers_closest() {
        let available = ["item", "items", "itemize"];
        let result = suggest_name("itm", &available);
        assert_eq!(result, Some("did you mean 'item'?".to_string()));
    }

    #[test]
    fn test_suggest_name_single_char_typo() {
        let available = ["println", "print", "debug_log"];
        let result = suggest_name("printl", &available);
        assert_eq!(result, Some("did you mean 'println'?".to_string()));
    }

    #[test]
    fn test_suggest_name_empty_available() {
        let available: Vec<&str> = vec![];
        let result = suggest_name("anything", &available);
        assert!(result.is_none());
    }
}
