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
            // Verify word boundary (check for alphanumeric or underscore)
            let before_ok = byte_offset == 0
                || !lines[line_idx].as_bytes().get(byte_offset - 1)
                    .map_or(false, |&b| b.is_ascii_alphanumeric() || b == b'_');
            let after_pos = byte_offset + name.len();
            let after_ok = after_pos >= lines[line_idx].len()
                || !lines[line_idx].as_bytes().get(after_pos)
                    .map_or(false, |&b| b.is_ascii_alphanumeric() || b == b'_');
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze_document;

    fn test_uri() -> Url {
        Url::parse("file:///test.magi").unwrap()
    }

    #[test]
    fn test_document_symbols_functions() {
        let source = "fn foo() { null }\nfn bar(x: int64) -> int64 { x }";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let result = handle_document_symbols(&state, &uri);
        assert!(result.is_some());
        if let Some(DocumentSymbolResponse::Flat(symbols)) = result {
            let fn_symbols: Vec<_> = symbols.iter()
                .filter(|s| s.kind == SymbolKind::FUNCTION)
                .collect();
            assert_eq!(fn_symbols.len(), 2);
            let names: Vec<&str> = fn_symbols.iter().map(|s| s.name.as_str()).collect();
            assert!(names.contains(&"foo"));
            assert!(names.contains(&"bar"));
        } else {
            panic!("expected flat symbols");
        }
    }

    #[test]
    fn test_document_symbols_enum_with_variants() {
        let source = "enum Color { Red, Green, Blue }";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let result = handle_document_symbols(&state, &uri);
        assert!(result.is_some());
        if let Some(DocumentSymbolResponse::Flat(symbols)) = result {
            let enum_sym: Vec<_> = symbols.iter()
                .filter(|s| s.kind == SymbolKind::ENUM)
                .collect();
            assert_eq!(enum_sym.len(), 1);
            assert_eq!(enum_sym[0].name, "Color");

            let variant_syms: Vec<_> = symbols.iter()
                .filter(|s| s.kind == SymbolKind::ENUM_MEMBER)
                .collect();
            assert_eq!(variant_syms.len(), 3);
            // Variants should have container_name set
            for v in &variant_syms {
                assert_eq!(v.container_name, Some("Color".to_string()));
            }
        }
    }

    #[test]
    fn test_document_symbols_struct_with_fields() {
        let source = "struct Point { x: float64, y: float64 }";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let result = handle_document_symbols(&state, &uri);
        assert!(result.is_some());
        if let Some(DocumentSymbolResponse::Flat(symbols)) = result {
            let struct_sym: Vec<_> = symbols.iter()
                .filter(|s| s.kind == SymbolKind::STRUCT)
                .collect();
            assert_eq!(struct_sym.len(), 1);
            assert_eq!(struct_sym[0].name, "Point");

            let field_syms: Vec<_> = symbols.iter()
                .filter(|s| s.kind == SymbolKind::FIELD)
                .collect();
            assert_eq!(field_syms.len(), 2);
            for f in &field_syms {
                assert_eq!(f.container_name, Some("Point".to_string()));
            }
        }
    }

    #[test]
    fn test_document_symbols_variables_and_constants() {
        let source = "let x = 5;\nconst PI = 3.14;\nlet mut counter = 0;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let result = handle_document_symbols(&state, &uri);
        assert!(result.is_some());
        if let Some(DocumentSymbolResponse::Flat(symbols)) = result {
            let const_sym: Vec<_> = symbols.iter()
                .filter(|s| s.kind == SymbolKind::CONSTANT)
                .collect();
            assert_eq!(const_sym.len(), 1);
            assert_eq!(const_sym[0].name, "PI");

            let var_syms: Vec<_> = symbols.iter()
                .filter(|s| s.kind == SymbolKind::VARIABLE)
                .collect();
            assert!(var_syms.len() >= 2); // x and counter
        }
    }

    #[test]
    fn test_document_symbols_type_alias() {
        let source = "type MyInt = int64;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let result = handle_document_symbols(&state, &uri);
        assert!(result.is_some());
        if let Some(DocumentSymbolResponse::Flat(symbols)) = result {
            let type_sym: Vec<_> = symbols.iter()
                .filter(|s| s.kind == SymbolKind::TYPE_PARAMETER)
                .collect();
            assert_eq!(type_sym.len(), 1);
            assert_eq!(type_sym[0].name, "MyInt");
        }
    }

    #[test]
    fn test_document_symbols_empty_document() {
        let source = "";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let result = handle_document_symbols(&state, &uri);
        assert!(result.is_some());
        if let Some(DocumentSymbolResponse::Flat(symbols)) = result {
            assert!(symbols.is_empty());
        }
    }

    #[test]
    fn test_document_symbols_sorted_by_position() {
        let source = "let z = 1;\nfn a() { null }\nlet m = 2;";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let result = handle_document_symbols(&state, &uri);
        if let Some(DocumentSymbolResponse::Flat(symbols)) = result {
            // Verify sorted by (line, character)
            for i in 1..symbols.len() {
                let prev = &symbols[i - 1].location.range.start;
                let curr = &symbols[i].location.range.start;
                assert!(
                    (prev.line, prev.character) <= (curr.line, curr.character),
                    "symbols not sorted: {:?} > {:?}", prev, curr
                );
            }
        }
    }

    #[test]
    fn test_document_symbols_parse_error_document() {
        // Document with parse error -- no AST, no symbols
        let source = "fn {";
        let (state, _) = analyze_document(source);
        let uri = test_uri();
        let result = handle_document_symbols(&state, &uri);
        assert!(result.is_some());
        if let Some(DocumentSymbolResponse::Flat(symbols)) = result {
            assert!(symbols.is_empty());
        }
    }

    #[test]
    fn test_make_range_line_0() {
        let source = "hello";
        // line=0, col=0 tests saturating_sub behavior
        let range = make_range(source, 0, 0, 5);
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
    }

    #[test]
    fn test_find_variant_range_not_found() {
        let source = "enum Color { Red }";
        // Search for a variant that doesn't exist
        let range = find_variant_range(source, 1, "NotExist");
        // Should fallback to definition line
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
    }

    #[test]
    fn test_find_variant_range_multiline_enum() {
        let source = "enum Direction {\n    North,\n    South,\n    East,\n    West,\n}";
        let range = find_variant_range(source, 1, "South");
        assert_eq!(range.start.line, 2); // "South" is on line 2 (0-based)
    }
}
