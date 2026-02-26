//! Completion provider for the MAGI LSP.

use super::analysis::{utf16_to_char_col, DocumentState};
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
    "len", "range", "assert", "print", "typeof", "to_string", "to_int",
    "to_float", "abs", "round", "floor", "ceil", "sqrt",
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

/// Handle a completion request.
pub fn handle_completion(
    state: &DocumentState,
    params: &CompletionParams,
) -> CompletionResponse {
    let mut items = Vec::new();

    // Extract the prefix at cursor position for filtering (only text before cursor)
    let prefix = find_prefix_at_position(
        &state.source,
        params.text_document_position.position.line,
        params.text_document_position.position.character,
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
        let detail = if var.constant {
            format!("const {}", name)
        } else if var.mutable {
            format!("let mut {}", name)
        } else {
            format!("let {}", name)
        };
        let kind = if var.constant {
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
        // Also suggest enum variants
        for variant in &en.variants {
            items.push(CompletionItem {
                label: format!("{}::{}", name, variant),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                detail: Some(format!("{}::{}", name, variant)),
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
        items.retain(|item| item.label.to_lowercase().starts_with(&prefix_lower));
    }

    CompletionResponse::Array(items)
}
