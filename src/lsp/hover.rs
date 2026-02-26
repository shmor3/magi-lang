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
        if var.mutable {
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

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "let" | "mut" | "fn" | "async" | "if" | "else" | "for" | "while" | "loop"
            | "match" | "return" | "break" | "continue" | "throw" | "try" | "catch"
            | "finally" | "output" | "import" | "use" | "const" | "type" | "mod"
            | "enum" | "struct" | "test" | "true" | "false" | "null" | "in" | "as"
            | "spawn" | "await"
    )
}
