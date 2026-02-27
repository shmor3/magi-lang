//! Completion provider for the MAGI LSP.

use super::analysis::{find_dot_receiver_at_position, find_variable_struct_type, utf16_to_char_col, DocumentState};
use tower_lsp::lsp_types::*;

/// MAGI language keywords.
const KEYWORDS: &[&str] = &[
    "let", "mut", "fn", "async", "if", "else", "for", "while", "loop", "match",
    "return", "break", "continue", "throw", "try", "catch", "finally", "output",
    "import", "use", "const", "type", "mod", "enum", "struct", "test", "true",
    "false", "null", "in", "as", "spawn", "await", "pub",
];

/// Built-in functions available without import.
const BUILTINS: &[&str] = &[
    "len", "range", "assert", "assert_eq", "assert_ne", "assert_throws",
    "print", "println", "debug_log", "typeof",
    "to_string", "to_int64", "to_float64", "to_bool", "to_json",
    "parse_int", "parse_float",
    "abs", "round", "floor", "ceil", "sqrt", "pow", "min", "max", "clamp",
    "sin", "cos", "tan", "ln", "log2", "log10", "exp",
    "is_null", "is_string", "is_number", "is_array", "is_map", "is_bool", "is_bytes",
];

/// Find the word prefix (text before cursor only) at a given position.
/// `character` is a 0-based UTF-16 code unit offset (per LSP spec).
fn find_prefix_at_position(source: &str, line: u32, character: u32) -> Option<String> {
    let target_line = source.lines().nth(line as usize)?;
    let chars: Vec<char> = target_line.chars().collect();
    let col = utf16_to_char_col(target_line, character) as usize;

    if col > chars.len() {
        return None;
    }

    // Scan backwards only for start of identifier
    let mut start = col;
    while start > 0 && (chars[start - 1].is_ascii_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }

    if start == col {
        return None;
    }

    Some(chars[start..col].iter().collect())
}

/// Check if cursor is right after `EnumName::` and return enum variants for completion.
fn find_double_colon_context(source: &str, line: u32, character: u32) -> Option<String> {
    let target_line = source.lines().nth(line as usize)?;
    let chars: Vec<char> = target_line.chars().collect();
    let col = utf16_to_char_col(target_line, character) as usize;

    if col > chars.len() {
        return None;
    }

    // Walk backwards: skip any partial identifier the user is typing
    let mut pos = col;
    while pos > 0 && (chars[pos - 1].is_ascii_alphanumeric() || chars[pos - 1] == '_') {
        pos -= 1;
    }

    // Check for `::`
    if pos < 2 || chars[pos - 1] != ':' || chars[pos - 2] != ':' {
        return None;
    }
    pos -= 2;

    // Scan backwards for the enum name
    let name_end = pos;
    let mut name_start = name_end;
    while name_start > 0 && (chars[name_start - 1].is_ascii_alphanumeric() || chars[name_start - 1] == '_') {
        name_start -= 1;
    }

    if name_start == name_end {
        return None;
    }

    Some(chars[name_start..name_end].iter().collect())
}

/// Common method names for array types.
const ARRAY_METHODS: &[(&str, &str)] = &[
    ("len", "Returns the length of the array"),
    ("push", "Appends an element to the array"),
    ("pop", "Removes and returns the last element"),
    ("shift", "Removes and returns the first element"),
    ("insert", "Inserts an element at an index"),
    ("remove", "Removes an element at an index"),
    ("get", "Returns element at an index"),
    ("set", "Sets element at an index"),
    ("map", "Transforms each element"),
    ("filter", "Filters elements by predicate"),
    ("reduce", "Reduces the array to a single value"),
    ("find", "Finds the first matching element"),
    ("find_index", "Finds the index of the first matching element"),
    ("any", "Returns true if any element matches"),
    ("all", "Returns true if all elements match"),
    ("each", "Iterates over each element"),
    ("sort", "Returns a sorted copy"),
    ("sort_by", "Sorts by a comparator function"),
    ("reverse", "Returns a reversed copy"),
    ("contains", "Checks if array contains a value"),
    ("join", "Joins elements into a string"),
    ("slice", "Returns a sub-array"),
    ("concat", "Concatenates two arrays"),
    ("flatten", "Flattens nested arrays one level"),
    ("unique", "Returns array with duplicates removed"),
    ("first", "Returns the first element or null"),
    ("last", "Returns the last element or null"),
    ("is_empty", "Returns true if the array is empty"),
    ("sum", "Returns the sum of all numeric elements"),
    ("product", "Returns the product of all numeric elements"),
    ("min", "Returns the minimum element"),
    ("max", "Returns the maximum element"),
    ("flat_map", "Maps and flattens results"),
    ("enumerate", "Returns [index, value] pairs"),
    ("chunk", "Splits into chunks of n elements"),
    ("zip", "Zips two arrays together"),
    ("group_by", "Groups elements by key function"),
    ("min_by", "Finds minimum by comparator function"),
    ("max_by", "Finds maximum by comparator function"),
    ("take_while", "Takes elements while predicate is true"),
    ("skip_while", "Skips elements while predicate is true"),
    ("partition", "Splits into [matches, non-matches]"),
    ("scan", "Like reduce but returns intermediate results"),
    ("filter_nulls", "Removes null elements"),
    ("to_string", "Converts to string representation"),
    ("to_json", "Converts to JSON string"),
    ("typeof", "Returns the type name"),
];

/// Common method names for string types.
const STRING_METHODS: &[(&str, &str)] = &[
    ("len", "Returns the length of the string"),
    ("is_empty", "Returns true if the string is empty"),
    ("is_numeric", "Returns true if the string is a valid number"),
    ("is_alphabetic", "Returns true if all characters are alphabetic"),
    ("contains", "Checks if string contains a substring"),
    ("starts_with", "Checks if string starts with a prefix"),
    ("ends_with", "Checks if string ends with a suffix"),
    ("trim", "Removes leading and trailing whitespace"),
    ("trim_start", "Removes leading whitespace"),
    ("trim_end", "Removes trailing whitespace"),
    ("to_uppercase", "Converts to uppercase"),
    ("to_lowercase", "Converts to lowercase"),
    ("split", "Splits the string by delimiter"),
    ("replace", "Replaces all occurrences of a substring"),
    ("substring", "Returns a substring by index range"),
    ("slice", "Returns a substring by index range"),
    ("index_of", "Finds the first index of a substring"),
    ("chars", "Returns array of characters"),
    ("lines", "Splits into lines"),
    ("words", "Splits into words"),
    ("reverse", "Returns a reversed string"),
    ("repeat", "Repeats the string n times"),
    ("count", "Counts occurrences of a substring"),
    ("pad_start", "Pads the start to a given length"),
    ("pad_end", "Pads the end to a given length"),
    ("char_at", "Returns the character at an index"),
    ("to_int", "Parses as integer"),
    ("to_float", "Parses as float"),
    ("to_string", "Returns the string itself"),
    ("to_int64", "Converts to 64-bit integer"),
    ("to_float64", "Converts to 64-bit float"),
    ("to_json", "Converts to JSON string"),
    ("typeof", "Returns the type name"),
];

/// Common method names for map types.
const MAP_METHODS: &[(&str, &str)] = &[
    ("get", "Returns the value for a key"),
    ("set", "Sets a key-value pair"),
    ("delete", "Removes a key-value pair"),
    ("has", "Checks if a key exists"),
    ("len", "Returns the number of entries"),
    ("keys", "Returns array of keys"),
    ("values", "Returns array of values"),
    ("entries", "Returns array of [key, value] pairs"),
    ("merge", "Merges another map into this one"),
    ("filter_entries", "Filters entries by predicate"),
    ("map_values", "Transforms values with a function"),
    ("map_keys", "Transforms keys with a function"),
    ("to_string", "Converts to string representation"),
    ("to_json", "Converts to JSON string"),
    ("typeof", "Returns the type name"),
];

/// Common method names for bytes types.
const BYTES_METHODS: &[(&str, &str)] = &[
    ("len", "Returns the byte length"),
    ("slice", "Returns a sub-slice of bytes"),
    ("concat", "Concatenates two byte sequences"),
    ("contains", "Checks if bytes contain a subsequence"),
    ("base64_encode", "Encodes bytes as a base64 string"),
    ("base64_decode", "Decodes base64 string to bytes"),
    ("to_string", "Converts to string representation"),
    ("to_json", "Converts to JSON string"),
    ("typeof", "Returns the type name"),
];

/// Common method names for integer types (Int64, Int32, Uint32, Uint64).
const INT_METHODS: &[(&str, &str)] = &[
    ("abs", "Returns the absolute value"),
    ("sign", "Returns the sign (-1, 0, or 1)"),
    ("pow", "Raises to a power"),
    ("min", "Returns the smaller of two values"),
    ("max", "Returns the larger of two values"),
    ("clamp", "Clamps between a minimum and maximum"),
    ("to_string", "Converts to string representation"),
    ("to_int64", "Converts to 64-bit integer"),
    ("to_float64", "Converts to 64-bit float"),
    ("to_json", "Converts to JSON string"),
    ("typeof", "Returns the type name"),
];

/// Common method names for float types (Float64, Float32).
const FLOAT_METHODS: &[(&str, &str)] = &[
    ("abs", "Returns the absolute value"),
    ("round", "Rounds to the nearest integer"),
    ("floor", "Rounds down to the nearest integer"),
    ("ceil", "Rounds up to the nearest integer"),
    ("sqrt", "Returns the square root"),
    ("sign", "Returns the sign (-1.0, 0.0, or 1.0)"),
    ("pow", "Raises to a power"),
    ("min", "Returns the smaller of two values"),
    ("max", "Returns the larger of two values"),
    ("clamp", "Clamps between a minimum and maximum"),
    ("is_nan", "Returns true if the value is NaN"),
    ("is_infinite", "Returns true if the value is infinite"),
    ("ln", "Returns the natural logarithm"),
    ("log2", "Returns the base-2 logarithm"),
    ("log10", "Returns the base-10 logarithm"),
    ("sin", "Returns the sine"),
    ("cos", "Returns the cosine"),
    ("tan", "Returns the tangent"),
    ("to_string", "Converts to string representation"),
    ("to_int64", "Converts to 64-bit integer"),
    ("to_float64", "Converts to 64-bit float"),
    ("to_json", "Converts to JSON string"),
    ("typeof", "Returns the type name"),
];

/// Generic methods available on all types.
const GENERIC_METHODS: &[(&str, &str)] = &[
    ("to_string", "Converts to string representation"),
    ("to_json", "Converts to JSON string"),
    ("to_int64", "Converts to 64-bit integer"),
    ("to_float64", "Converts to 64-bit float"),
    ("to_bool", "Converts to boolean"),
    ("typeof", "Returns the type name"),
];

/// Handle a completion request.
pub fn handle_completion(
    state: &DocumentState,
    params: &CompletionParams,
) -> CompletionResponse {
    let pos = params.text_document_position.position;

    // Check for double-colon context: `EnumName::` -> suggest variants
    if let Some(enum_name) = find_double_colon_context(&state.source, pos.line, pos.character) {
        if let Some(en) = state.enums.get(&enum_name) {
            let items: Vec<CompletionItem> = en.variants.iter().map(|variant| {
                CompletionItem {
                    label: variant.clone(),
                    kind: Some(CompletionItemKind::ENUM_MEMBER),
                    detail: Some(format!("{}::{}", enum_name, variant)),
                    ..Default::default()
                }
            }).collect();
            return CompletionResponse::Array(items);
        }
    }

    // Check for dot-access context: `expr.` -> suggest fields/methods
    if let Some(receiver) = find_dot_receiver_at_position(&state.source, pos.line, pos.character) {
        let mut items = Vec::new();

        // Check if receiver is a variable with a known struct type
        if let Some(struct_name) = find_variable_struct_type(state, &receiver) {
            if let Some(st) = state.structs.get(&struct_name) {
                for (field_name, field_type) in &st.fields {
                    let detail = if let Some(ty) = field_type {
                        format!("{}: {}", field_name, ty)
                    } else {
                        field_name.clone()
                    };
                    items.push(CompletionItem {
                        label: field_name.clone(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(detail),
                        ..Default::default()
                    });
                }
            }
        }

        // Also add generic methods for all types
        for &(method, desc) in GENERIC_METHODS {
            items.push(CompletionItem {
                label: method.to_string(),
                kind: Some(CompletionItemKind::METHOD),
                detail: Some(desc.to_string()),
                ..Default::default()
            });
        }

        // Add type-specific methods based on variable type annotation
        let type_methods: Option<&[(&str, &str)]> = if let Some(var) = state.variables.get(&receiver) {
            match var.type_annotation.as_deref() {
                Some(t) if t.starts_with("[") || t == "Array" => Some(ARRAY_METHODS),
                Some("string" | "String") => Some(STRING_METHODS),
                Some("Map") => Some(MAP_METHODS),
                Some("Bytes" | "bytes") => Some(BYTES_METHODS),
                Some("int64" | "Int64" | "int32" | "Int32" | "uint32" | "Uint32" | "uint64" | "Uint64") => Some(INT_METHODS),
                Some("float64" | "Float64" | "float32" | "Float32") => Some(FLOAT_METHODS),
                _ => None,
            }
        } else {
            None
        };

        if let Some(methods) = type_methods {
            for &(method, desc) in methods {
                // Avoid duplicates with generic methods
                if !items.iter().any(|i| i.label == method) {
                    items.push(CompletionItem {
                        label: method.to_string(),
                        kind: Some(CompletionItemKind::METHOD),
                        detail: Some(desc.to_string()),
                        ..Default::default()
                    });
                }
            }
        }

        // Filter by any partial text after the dot
        let prefix = find_prefix_at_position(&state.source, pos.line, pos.character)
            .unwrap_or_default();
        if !prefix.is_empty() {
            let prefix_lower = prefix.to_lowercase();
            items.retain(|item| item.label.to_lowercase().contains(&prefix_lower));
        }

        return CompletionResponse::Array(items);
    }

    let mut items = Vec::new();

    // Extract the prefix at cursor position for filtering (only text before cursor)
    let prefix = find_prefix_at_position(
        &state.source,
        pos.line,
        pos.character,
    )
    .unwrap_or_default();

    // Keywords
    for kw in KEYWORDS {
        items.push(CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("keyword".to_string()),
            ..Default::default()
        });
    }

    // Builtins
    for bi in BUILTINS {
        items.push(CompletionItem {
            label: bi.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("built-in".to_string()),
            ..Default::default()
        });
    }

    // User-defined functions
    for (name, func) in &state.functions {
        let params_str = func.params.join(", ");
        items.push(CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(format!("fn {}({})", name, params_str)),
            ..Default::default()
        });
    }

    // User-defined variables
    for (name, var) in &state.variables {
        let detail = if var.is_type_alias {
            if let Some(ref ta) = var.type_annotation {
                format!("type {} = {}", name, ta)
            } else {
                format!("type {}", name)
            }
        } else if var.type_annotation.as_deref() == Some("module") {
            format!("mod {}", name)
        } else if let Some(ref ta) = var.type_annotation {
            if ta.starts_with("import(") {
                format!("use {}", &ta[7..ta.len().saturating_sub(1)])
            } else if var.constant {
                format!("const {}: {}", name, ta)
            } else if var.mutable {
                format!("let mut {}: {}", name, ta)
            } else {
                format!("let {}: {}", name, ta)
            }
        } else if var.constant {
            format!("const {}", name)
        } else if var.mutable {
            format!("let mut {}", name)
        } else {
            format!("let {}", name)
        };
        let kind = if var.is_type_alias {
            CompletionItemKind::TYPE_PARAMETER
        } else if var.type_annotation.as_deref() == Some("module") {
            CompletionItemKind::MODULE
        } else if var.type_annotation.as_ref().map_or(false, |t| t.starts_with("import(")) {
            CompletionItemKind::REFERENCE
        } else if var.constant {
            CompletionItemKind::CONSTANT
        } else {
            CompletionItemKind::VARIABLE
        };
        items.push(CompletionItem {
            label: name.clone(),
            kind: Some(kind),
            detail: Some(detail),
            ..Default::default()
        });
    }

    // User-defined enums
    for (name, en) in &state.enums {
        items.push(CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::ENUM),
            detail: Some(format!("enum {} ({} variants)", name, en.variants.len())),
            ..Default::default()
        });
        // Also suggest enum variants (filterable by variant name or full path)
        for variant in &en.variants {
            let full_name = format!("{}::{}", name, variant);
            items.push(CompletionItem {
                label: full_name.clone(),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                detail: Some(full_name.clone()),
                filter_text: Some(format!("{} {}", full_name, variant)),
                ..Default::default()
            });
        }
    }

    // User-defined structs
    for (name, _) in &state.structs {
        items.push(CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::STRUCT),
            detail: Some(format!("struct {}", name)),
            ..Default::default()
        });
    }

    // Filter by prefix if one exists
    if !prefix.is_empty() {
        let prefix_lower = prefix.to_lowercase();
        items.retain(|item| {
            let text = item.filter_text.as_ref().unwrap_or(&item.label);
            text.to_lowercase().contains(&prefix_lower)
        });
    }

    CompletionResponse::Array(items)
}
