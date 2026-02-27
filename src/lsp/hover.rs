//! Hover provider for the MAGI LSP.

use super::analysis::{find_word_at_position, DocumentState};
use tower_lsp::lsp_types::*;

/// Handle a hover request. Looks up the word under the cursor in symbol maps.
pub fn handle_hover(state: &DocumentState, params: &HoverParams) -> Option<Hover> {
    let pos = params.text_document_position_params.position;
    let word = find_word_at_position(&state.source, pos.line, pos.character)?;

    // Look up in functions
    if let Some(func) = state.functions.get(&word) {
        let params_str = func.params.join(", ");
        let ret = func
            .return_type
            .as_deref()
            .map_or(String::new(), |r| format!(" -> {}", r));
        let info = format!("```magi\nfn {}({}){}\n```", func.name, params_str, ret);
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info,
            }),
            range: None,
        });
    }

    // Look up in variables
    if let Some(var) = state.variables.get(&word) {
        let mut info = String::from("```magi\n");
        if var.constant {
            info.push_str("const ");
        } else if var.mutable {
            info.push_str("let mut ");
        } else {
            info.push_str("let ");
        }
        info.push_str(&var.name);
        if let Some(ty) = &var.type_annotation {
            info.push_str(": ");
            info.push_str(ty);
        }
        info.push_str("\n```");
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info,
            }),
            range: None,
        });
    }

    // Look up in enums
    if let Some(en) = state.enums.get(&word) {
        let variants = en.variants.join(", ");
        let info = format!("```magi\nenum {} {{ {} }}\n```", en.name, variants);
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info,
            }),
            range: None,
        });
    }

    // Look up in structs
    if let Some(st) = state.structs.get(&word) {
        let fields: Vec<String> = st
            .fields
            .iter()
            .map(|(name, ty)| {
                if let Some(t) = ty {
                    format!("{}: {}", name, t)
                } else {
                    name.clone()
                }
            })
            .collect();
        let info = format!("```magi\nstruct {} {{ {} }}\n```", st.name, fields.join(", "));
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info,
            }),
            range: None,
        });
    }

    // Check if it's a builtin function
    if let Some(desc) = builtin_description(&word) {
        let info = format!("```magi\nfn {}(...)\n```\n{}", word, desc);
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info,
            }),
            range: None,
        });
    }

    // Check if it's a keyword
    if is_keyword(&word) {
        let info = format!("`{}` — MAGI keyword", word);
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info,
            }),
            range: None,
        });
    }

    None
}

fn builtin_description(name: &str) -> Option<&'static str> {
    match name {
        "len" => Some("Returns the length of an array, string, or map."),
        "range" => Some("Creates an array of integers from start to end."),
        "assert" => Some("Asserts a condition is true; throws on failure."),
        "print" => Some("Prints a value to stdout (no newline)."),
        "println" => Some("Prints a value to stdout with a newline."),
        "debug_log" => Some("Logs a debug message."),
        "typeof" => Some("Returns the type name of a value as a string."),
        "to_string" => Some("Converts a value to its string representation."),
        "to_int" => Some("Converts a value to an integer."),
        "to_float" => Some("Converts a value to a float."),
        "to_int64" => Some("Converts a value to a 64-bit integer."),
        "to_float64" => Some("Converts a value to a 64-bit float."),
        "to_bool" => Some("Converts a value to a boolean."),
        "to_json" => Some("Converts a value to a JSON string."),
        "parse_int" => Some("Parses a string as an integer."),
        "parse_float" => Some("Parses a string as a float."),
        "abs" => Some("Returns the absolute value of a number."),
        "round" => Some("Rounds a number to the nearest integer."),
        "floor" => Some("Rounds a number down to the nearest integer."),
        "ceil" => Some("Rounds a number up to the nearest integer."),
        "sqrt" => Some("Returns the square root of a number."),
        "pow" => Some("Raises a number to a power."),
        "min" => Some("Returns the smaller of two values."),
        "max" => Some("Returns the larger of two values."),
        "clamp" => Some("Clamps a value between a minimum and maximum."),
        "sin" => Some("Returns the sine of an angle in radians."),
        "cos" => Some("Returns the cosine of an angle in radians."),
        "tan" => Some("Returns the tangent of an angle in radians."),
        "ln" => Some("Returns the natural logarithm."),
        "log2" => Some("Returns the base-2 logarithm."),
        "log10" => Some("Returns the base-10 logarithm."),
        "exp" => Some("Returns e raised to a power."),
        "output" => Some("Outputs a value as the program result."),
        _ => None,
    }
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "let" | "mut" | "fn" | "async" | "if" | "else" | "for" | "while" | "loop"
            | "match" | "return" | "break" | "continue" | "throw" | "try" | "catch"
            | "finally" | "output" | "import" | "use" | "const" | "type" | "mod"
            | "enum" | "struct" | "test" | "true" | "false" | "null" | "in" | "as"
            | "spawn" | "await" | "pub"
    )
}
