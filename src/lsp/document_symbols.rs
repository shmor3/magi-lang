//! Document symbol provider for the MAGI LSP.
//!
//! Provides function/enum/struct/variable symbols for the editor outline view.

use super::analysis::{char_col_to_utf16, DocumentState};
use tower_lsp::lsp_types::*;

/// Handle a document symbol request.
/// Returns a flat list of SymbolInformation (not hierarchical DocumentSymbol)
/// for broadest editor compatibility.
#[allow(deprecated)] // SymbolInformation::location field
pub fn handle_document_symbols(
    state: &DocumentState,
    uri: &Url,
) -> Option<DocumentSymbolResponse> {
    let mut symbols: Vec<SymbolInformation> = Vec::new();

    // Functions
    for func in state.functions.values() {
        let range = make_range(&state.source, func.line, func.col, func.name.chars().count());
        #[allow(deprecated)]
        symbols.push(SymbolInformation {
            name: func.name.clone(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            deprecated: None,
            location: Location {
                uri: uri.clone(),
                range,
            },
            container_name: None,
        });
    }

    // Enums (with variants as children)
    for en in state.enums.values() {
        let range = make_range(&state.source, en.line, en.col, en.name.chars().count());
        #[allow(deprecated)]
        symbols.push(SymbolInformation {
            name: en.name.clone(),
            kind: SymbolKind::ENUM,
            tags: None,
            deprecated: None,
            location: Location {
                uri: uri.clone(),
                range,
            },
            container_name: None,
        });

        // Add variants as members of the enum
        for variant in &en.variants {
            // Variants don't have their own tracked positions, so use enum's line
            // and try to find the variant name on that line or nearby
            let variant_range = find_variant_range(&state.source, en.line, variant);
            #[allow(deprecated)]
            symbols.push(SymbolInformation {
                name: format!("{}::{}", en.name, variant),
                kind: SymbolKind::ENUM_MEMBER,
                tags: None,
                deprecated: None,
                location: Location {
                    uri: uri.clone(),
                    range: variant_range,
                },
                container_name: Some(en.name.clone()),
            });
        }
    }

    // Structs (with fields as children)
    for st in state.structs.values() {
        let range = make_range(&state.source, st.line, st.col, st.name.chars().count());
        #[allow(deprecated)]
        symbols.push(SymbolInformation {
            name: st.name.clone(),
            kind: SymbolKind::STRUCT,
            tags: None,
            deprecated: None,
            location: Location {
                uri: uri.clone(),
                range,
            },
            container_name: None,
        });

        // Add fields
        for (field_name, field_type) in &st.fields {
            let field_label = if let Some(ty) = field_type {
                format!("{}: {}", field_name, ty)
            } else {
                field_name.clone()
            };
            let field_range = find_variant_range(&state.source, st.line, field_name);
            #[allow(deprecated)]
            symbols.push(SymbolInformation {
                name: field_label,
                kind: SymbolKind::FIELD,
                tags: None,
                deprecated: None,
                location: Location {
                    uri: uri.clone(),
                    range: field_range,
                },
                container_name: Some(st.name.clone()),
            });
        }
    }

    // Variables and constants
    for var in state.variables.values() {
        let kind = if var.is_type_alias {
            SymbolKind::TYPE_PARAMETER
        } else if var.type_annotation.as_deref() == Some("module") {
            SymbolKind::MODULE
        } else if var.constant {
            SymbolKind::CONSTANT
        } else {
            SymbolKind::VARIABLE
        };
        let range = make_range(&state.source, var.line, var.col, var.name.chars().count());
        #[allow(deprecated)]
        symbols.push(SymbolInformation {
            name: var.name.clone(),
            kind,
            tags: None,
            deprecated: None,
            location: Location {
                uri: uri.clone(),
                range,
            },
            container_name: None,
        });
    }

    // Sort by position for a natural reading order
    symbols.sort_by_key(|s| (s.location.range.start.line, s.location.range.start.character));

    Some(DocumentSymbolResponse::Flat(symbols))
}

/// Build a 0-based LSP Range from 1-based line/col and a name length in chars.
fn make_range(source: &str, line: u32, col: u32, name_char_len: usize) -> Range {
    let lsp_line = line.saturating_sub(1);
    let char_col = col.saturating_sub(1);
    let line_text = source.lines().nth(lsp_line as usize).unwrap_or("");
    let start_utf16 = char_col_to_utf16(line_text, char_col);
    let end_utf16 = char_col_to_utf16(line_text, char_col.saturating_add(name_char_len as u32));
    Range {
        start: Position {
            line: lsp_line,
            character: start_utf16,
        },
        end: Position {
            line: lsp_line,
            character: end_utf16,
        },
    }
}

/// Try to find a name (variant or field) within a few lines of a definition.
/// Returns a best-effort range.
fn find_variant_range(source: &str, def_line: u32, name: &str) -> Range {
    let lines: Vec<&str> = source.lines().collect();
    let start_line = def_line.saturating_sub(1) as usize; // 0-based
    // Search within 20 lines after the definition start
    for offset in 0..20 {
        let line_idx = start_line + offset;
        if line_idx >= lines.len() {
            break;
        }
        if let Some(byte_offset) = lines[line_idx].find(name) {
            // Verify word boundary
            let before_ok = byte_offset == 0
                || !lines[line_idx].as_bytes()[byte_offset - 1].is_ascii_alphanumeric();
            let after_pos = byte_offset + name.len();
            let after_ok = after_pos >= lines[line_idx].len()
                || !lines[line_idx].as_bytes()[after_pos].is_ascii_alphanumeric();
            if before_ok && after_ok {
                let char_col = lines[line_idx][..byte_offset].chars().count() as u32;
                let start_utf16 = char_col_to_utf16(lines[line_idx], char_col);
                let end_utf16 = char_col_to_utf16(lines[line_idx], char_col + name.chars().count() as u32);
                return Range {
                    start: Position {
                        line: line_idx as u32,
                        character: start_utf16,
                    },
                    end: Position {
                        line: line_idx as u32,
                        character: end_utf16,
                    },
                };
            }
        }
    }

    // Fallback: point to the definition line
    let lsp_line = def_line.saturating_sub(1);
    Range {
        start: Position { line: lsp_line, character: 0 },
        end: Position { line: lsp_line, character: 0 },
    }
}
