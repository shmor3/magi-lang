//! Hover provider for the MAGI LSP.

use super::analysis::{find_enum_variant_at_position, find_word_at_position, DocumentState};
use tower_lsp::lsp_types::*;

/// Handle a hover request. Looks up the word under the cursor in symbol maps.
pub fn handle_hover(state: &DocumentState, params: &HoverParams) -> Option<Hover> {
    let pos = params.text_document_position_params.position;

    // Check for enum variant pattern (EnumName::Variant) first
    if let Some((enum_name, variant_name)) = find_enum_variant_at_position(&state.source, pos.line, pos.character) {
        if let Some(en) = state.enums.get(&enum_name) {
            if en.variants.contains(&variant_name) {
                let all_variants = en.variants.join(", ");
                let info = format!(
                    "```magi\n{}::{}\n```\nVariant of `enum {} {{ {} }}`",
                    enum_name, variant_name, enum_name, all_variants
                );
                return Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: info,
                    }),
                    range: None,
                });
            }
        }
    }

    let word = find_word_at_position(&state.source, pos.line, pos.character)?;

    // Look up in functions
    if let Some(func) = state.functions.get(&word) {
        let params_str = func.params.join(", ");
        let ret = func
            .return_type
            .as_deref()
            .map_or(String::new(), |r| format!(" -> {}", r));
        let prefix = if func.is_async { "async fn" } else { "fn" };
        let info = format!("```magi\n{} {}({}){}\n```", prefix, func.name, params_str, ret);
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
        if var.is_type_alias {
            info.push_str("type ");
            info.push_str(&var.name);
            if let Some(ty) = &var.type_annotation {
                info.push_str(" = ");
                info.push_str(ty);
            }
        } else if var.type_annotation.as_deref() == Some("module") {
            info.push_str("mod ");
            info.push_str(&var.name);
        } else if let Some(ref ta) = var.type_annotation {
            if let Some(inner) = ta.strip_prefix("import(").and_then(|s| s.strip_suffix(')')) {
                info.push_str("use ");
                info.push_str(inner);
            } else if ta.starts_with("import(") {
                info.push_str("use ");
                info.push_str(ta);
            } else {
                if var.constant {
                    info.push_str("const ");
                } else if var.mutable {
                    info.push_str("let mut ");
                } else {
                    info.push_str("let ");
                }
                info.push_str(&var.name);
                info.push_str(": ");
                info.push_str(ta);
            }
        } else {
            if var.constant {
                info.push_str("const ");
            } else if var.mutable {
                info.push_str("let mut ");
            } else {
                info.push_str("let ");
            }
            info.push_str(&var.name);
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

    // Check if it's a known method name
    if let Some(desc) = method_description(&word) {
        let info = format!("```magi\n.{}(...)\n```\n{}", word, desc);
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
        "assert_eq" => Some("Asserts two values are equal; throws on failure."),
        "assert_ne" => Some("Asserts two values are not equal; throws on failure."),
        "assert_throws" => Some("Asserts that a function throws an error when called."),
        "print" => Some("Prints a value to stdout (no newline)."),
        "println" => Some("Prints a value to stdout with a newline."),
        "debug_log" => Some("Logs a debug message."),
        "typeof" => Some("Returns the type name of a value as a string."),
        "to_string" => Some("Converts a value to its string representation."),
        "to_int64" => Some("Converts a value to a 64-bit integer."),
        "to_float64" => Some("Converts a value to a 64-bit float."),
        "to_float32" => Some("Converts a value to a 32-bit float."),
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
        "asin" => Some("Returns the arcsine (inverse sine) in radians."),
        "acos" => Some("Returns the arccosine (inverse cosine) in radians."),
        "atan" => Some("Returns the arctangent (inverse tangent) in radians."),
        "atan2" => Some("Returns the arctangent of y/x, using signs to determine the quadrant."),
        "sinh" => Some("Returns the hyperbolic sine."),
        "cosh" => Some("Returns the hyperbolic cosine."),
        "tanh" => Some("Returns the hyperbolic tangent."),
        "ln" => Some("Returns the natural logarithm."),
        "log2" => Some("Returns the base-2 logarithm."),
        "log10" => Some("Returns the base-10 logarithm."),
        "exp" => Some("Computes e^x (exponential function)"),
        "is_null" => Some("Returns true if the value is null."),
        "is_string" => Some("Returns true if the value is a string."),
        "is_number" => Some("Returns true if the value is a number (int or float)."),
        "is_array" => Some("Returns true if the value is an array."),
        "is_map" => Some("Returns true if the value is a map."),
        "is_bool" => Some("Returns true if the value is a boolean."),
        "is_bytes" => Some("Returns true if the value is a bytes value."),
        "is_finite" => Some("Returns true if the value is finite (not NaN or Infinity)."),
        _ => None,
    }
}

fn method_description(name: &str) -> Option<&'static str> {
    match name {
        // Generic methods (aliases not covered by builtin_description)
        "length" => Some("Returns the length (alias for `len`)."),
        "size" => Some("Returns the number of entries in a map (alias for `len`)."),
        // Array methods
        "push" => Some("Appends an element to the array and returns the new array."),
        "pop" => Some("Removes and returns the last element of the array."),
        "shift" => Some("Removes and returns the first element of the array."),
        "insert" => Some("Inserts an element at the given index."),
        "remove" => Some("Removes the element at the given index and returns the array."),
        "get" => Some("Returns the element at an index (array) or the value for a key (map)."),
        "set" => Some("Sets the element at an index (array) or a key-value pair (map). Returns the updated collection."),
        "map" => Some("Transforms each element using a function. Returns a new array."),
        "filter" => Some("Filters elements by a predicate function. Returns a new array."),
        "reduce" => Some("Reduces the array to a single value using an accumulator function."),
        "find" => Some("Returns the first element matching the predicate, or null."),
        "find_index" => Some("Returns the index of the first matching element, or null."),
        "any" => Some("Returns true if any element matches the predicate."),
        "all" => Some("Returns true if all elements match the predicate."),
        "each" => Some("Iterates over each element (for side effects). Returns null."),
        "sort" => Some("Returns a sorted copy of the array."),
        "sort_by" => Some("Sorts by a comparator function (a, b) -> number."),
        "reverse" => Some("Returns a reversed copy."),
        "contains" => Some("Checks if the collection contains a value."),
        "join" => Some("Joins array elements into a string with a separator."),
        "slice" => Some("Returns a sub-array or substring by index range."),
        "concat" => Some("Concatenates two arrays or byte sequences."),
        "flatten" => Some("Flattens nested arrays one level deep."),
        "unique" => Some("Returns the array with duplicate elements removed."),
        "first" => Some("Returns the first element, or null if empty."),
        "last" => Some("Returns the last element, or null if empty."),
        "is_empty" => Some("Returns true if the collection is empty."),
        "sum" => Some("Returns the sum of all numeric elements."),
        "product" => Some("Returns the product of all numeric elements."),
        "flat_map" => Some("Maps each element and flattens the results."),
        "enumerate" => Some("Returns [index, value] pairs."),
        "chunk" => Some("Splits the array into chunks of n elements."),
        "zip" => Some("Zips two arrays into pairs."),
        "group_by" => Some("Groups elements by a key function. Returns a map."),
        "min_by" => Some("Finds the minimum element using a comparator."),
        "max_by" => Some("Finds the maximum element using a comparator."),
        "take_while" => Some("Takes elements while the predicate is true."),
        "skip_while" => Some("Skips elements while the predicate is true."),
        "partition" => Some("Splits into [matches, non-matches]."),
        "scan" => Some("Like reduce, but returns all intermediate accumulator values."),
        "filter_nulls" => Some("Removes null elements from the array."),
        // String methods
        "split" => Some("Splits the string by a delimiter. Returns an array."),
        "replace" => Some("Replaces all occurrences of a substring."),
        "trim" => Some("Removes leading and trailing whitespace."),
        "trim_start" => Some("Removes leading whitespace."),
        "trim_end" => Some("Removes trailing whitespace."),
        "to_uppercase" | "to_upper" => Some("Converts the string to uppercase."),
        "to_lowercase" | "to_lower" => Some("Converts the string to lowercase."),
        "starts_with" => Some("Returns true if the string starts with the given prefix."),
        "ends_with" => Some("Returns true if the string ends with the given suffix."),
        "index_of" => Some("Returns the character index of the first occurrence, or -1."),
        "chars" => Some("Returns an array of individual characters."),
        "lines" => Some("Splits the string into lines. Returns an array."),
        "words" => Some("Splits the string into words. Returns an array."),
        "repeat" => Some("Repeats the string n times."),
        "count" => Some("Counts occurrences of a substring."),
        "pad_start" => Some("Pads the start of the string to a given width."),
        "pad_end" => Some("Pads the end of the string to a given width."),
        "char_at" => Some("Returns the character at the given index, or null."),
        "substring" => Some("Returns a substring by start and end index."),
        "is_numeric" => Some("Returns true if the string is a valid number."),
        "is_alphabetic" => Some("Returns true if all characters are alphabetic."),
        "to_int" => Some("Parses the string as an integer. Returns null on failure."),
        "to_float" => Some("Parses the string as a float. Returns null on failure."),
        // Map methods
        "has" => Some("Returns true if the map contains the given key."),
        "delete" => Some("Removes a key from the map. Returns the updated map."),
        "keys" => Some("Returns an array of all keys in the map."),
        "values" => Some("Returns an array of all values in the map."),
        "entries" => Some("Returns an array of [key, value] pairs."),
        "merge" => Some("Merges another map into this one. Returns the merged map."),
        "filter_entries" => Some("Filters map entries by a predicate (key, value) -> bool."),
        "map_values" => Some("Transforms map values with a function. Returns a new map."),
        "map_keys" => Some("Transforms map keys with a function. Returns a new map."),
        // Bytes methods
        "base64_encode" => Some("Encodes bytes as a base64 string."),
        "base64_decode" => Some("Decodes a base64 string to bytes."),
        // Numeric methods
        "sign" => Some("Returns the sign of the number (-1, 0, or 1)."),
        "is_nan" => Some("Returns true if the value is NaN."),
        "is_infinite" => Some("Returns true if the value is infinite."),
        "is_finite" => Some("Returns true if the value is finite (not NaN or Infinity)."),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze_document;

    fn make_hover_params(line: u32, character: u32) -> HoverParams {
        HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse("file:///test.magi").unwrap(),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        }
    }

    #[test]
    fn test_hover_function() {
        let source = "fn greet(name: string) -> string { name }";
        let (state, _) = analyze_document(source);
        let params = make_hover_params(0, 3);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("fn greet"));
            assert!(content.value.contains("name: string"));
            assert!(content.value.contains("-> string"));
        } else {
            panic!("expected markup content");
        }
    }

    #[test]
    fn test_hover_variable() {
        let source = "let x: int64 = 42;";
        let (state, _) = analyze_document(source);
        let params = make_hover_params(0, 4);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("let x"));
            assert!(content.value.contains("int64"));
        }
    }

    #[test]
    fn test_hover_constant() {
        let source = "const PI = 3.14;";
        let (state, _) = analyze_document(source);
        let params = make_hover_params(0, 6);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("const PI"));
        }
    }

    #[test]
    fn test_hover_enum() {
        let source = "enum Color { Red, Green, Blue }";
        let (state, _) = analyze_document(source);
        // Hover on "Color"
        let params = make_hover_params(0, 5);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("enum Color"));
            assert!(content.value.contains("Red"));
        }
    }

    #[test]
    fn test_hover_struct() {
        let source = "struct Point { x: float64, y: float64 }";
        let (state, _) = analyze_document(source);
        let params = make_hover_params(0, 7);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("struct Point"));
            assert!(content.value.contains("x: float64"));
        }
    }

    #[test]
    fn test_hover_builtin() {
        let source = "len([1, 2, 3])";
        let (state, _) = analyze_document(source);
        let params = make_hover_params(0, 1);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("len"));
            assert!(content.value.contains("length"));
        }
    }

    #[test]
    fn test_hover_keyword() {
        let source = "let x = 5;";
        let (state, _) = analyze_document(source);
        // Hover on "let" keyword
        let params = make_hover_params(0, 0);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("keyword"));
        }
    }

    #[test]
    fn test_hover_on_space_returns_none() {
        let source = "let x = 5;";
        let (state, _) = analyze_document(source);
        // Col 3 is the space after "let" -- backward scan finds "let" keyword
        let params = make_hover_params(0, 3);
        let _result = handle_hover(&state, &params);
        // Col 6 is space between = and 5 (no adjacent identifier chars)
        let params2 = make_hover_params(0, 6);
        let result2 = handle_hover(&state, &params2);
        assert!(result2.is_none());
    }

    #[test]
    fn test_hover_empty_document() {
        let source = "";
        let (state, _) = analyze_document(source);
        let params = make_hover_params(0, 0);
        let result = handle_hover(&state, &params);
        assert!(result.is_none());
    }

    #[test]
    fn test_hover_method_name() {
        let source = "let arr = [1, 2, 3];\narr.map(|x| x + 1)";
        let (state, _) = analyze_document(source);
        // Hover on "map" (line 1, col 4)
        let params = make_hover_params(1, 4);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("map"));
        }
    }

    #[test]
    fn test_hover_enum_variant() {
        let source = "enum Color { Red, Green, Blue }\nlet c = Color::Red;";
        let (state, _) = analyze_document(source);
        // Hover on "Red" in "Color::Red" (line 1, col 15)
        let params = make_hover_params(1, 15);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("Color::Red"));
            assert!(content.value.contains("Variant"));
        }
    }

    #[test]
    fn test_hover_type_alias() {
        let source = "type MyInt = int64;";
        let (state, _) = analyze_document(source);
        let params = make_hover_params(0, 5);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("type MyInt"));
            assert!(content.value.contains("int64"));
        }
    }

    #[test]
    fn test_hover_use_import() {
        let source = "use std::io;";
        let (state, _) = analyze_document(source);
        let params = make_hover_params(0, 9);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("use"));
            assert!(content.value.contains("std::io"));
        }
    }

    #[test]
    fn test_hover_async_function() {
        let source = "async fn fetch() { null }";
        let (state, _) = analyze_document(source);
        let params = make_hover_params(0, 9);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("async fn fetch"));
        }
    }

    #[test]
    fn test_hover_method_length() {
        // "length" is an alias method — should have hover description
        let source = "let arr = [1, 2, 3];\narr.length()";
        let (state, _) = analyze_document(source);
        // Hover on "length" (line 1, col 4)
        let params = make_hover_params(1, 4);
        let result = handle_hover(&state, &params);
        assert!(result.is_some(), "hovering on 'length' should show method description");
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("length"), "should mention 'length': {}", content.value);
        }
    }

    #[test]
    fn test_hover_method_size() {
        // "size" is a map alias method — should have hover description
        let source = "let m = { a: 1 };\nm.size()";
        let (state, _) = analyze_document(source);
        // Hover on "size" (line 1, col 2)
        let params = make_hover_params(1, 2);
        let result = handle_hover(&state, &params);
        assert!(result.is_some(), "hovering on 'size' should show method description");
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("size"), "should mention 'size': {}", content.value);
        }
    }

    #[test]
    fn test_hover_method_get_description() {
        // "get" works on both arrays and maps — description should reflect this
        let source = "let arr = [1, 2, 3];\narr.get(0)";
        let (state, _) = analyze_document(source);
        let params = make_hover_params(1, 4);
        let result = handle_hover(&state, &params);
        assert!(result.is_some());
        let hover = result.unwrap();
        if let HoverContents::Markup(content) = &hover.contents {
            assert!(content.value.contains("index") || content.value.contains("key"),
                "get description should mention index or key: {}", content.value);
        }
    }
}
