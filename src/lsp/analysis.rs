//! Document analysis — parse, type-check, lint, and extract symbols.

use crate::linter;
use crate::syntax::ast::*;
use crate::syntax::parser::parse_v2;
use crate::syntax::type_checker::{self, AstDiagnostic};
use std::collections::{HashMap, HashSet};
use tower_lsp::lsp_types::*;

/// Extracted symbol information for a function.
#[derive(Debug, Clone)]
pub struct FunctionSymbol {
    pub name: String,
    pub params: Vec<String>,
    pub return_type: Option<String>,
    pub line: u32,
    pub col: u32,
}

/// Extracted symbol information for a variable.
#[derive(Debug, Clone)]
pub struct VariableSymbol {
    pub name: String,
    pub mutable: bool,
    pub constant: bool,
    pub type_annotation: Option<String>,
    pub line: u32,
    pub col: u32,
}

/// Extracted symbol information for an enum.
#[derive(Debug, Clone)]
pub struct EnumSymbol {
    pub name: String,
    pub variants: Vec<String>,
    pub line: u32,
    pub col: u32,
}

/// Extracted symbol information for a struct.
#[derive(Debug, Clone)]
pub struct StructSymbol {
    pub name: String,
    pub fields: Vec<(String, Option<String>)>,
    pub line: u32,
    pub col: u32,
}

/// The complete analysis state for a single document.
#[derive(Debug, Clone)]
pub struct DocumentState {
    pub source: String,
    pub program: Option<Program>,
    pub functions: HashMap<String, FunctionSymbol>,
    pub variables: HashMap<String, VariableSymbol>,
    pub enums: HashMap<String, EnumSymbol>,
    pub structs: HashMap<String, StructSymbol>,
}

/// Analyze a document: parse + type check + lint + extract symbols.
/// Returns the document state and all diagnostics.
pub fn analyze_document(source: &str) -> (DocumentState, Vec<AstDiagnostic>) {
    let mut all_diagnostics = Vec::new();

    let program = match parse_v2(source) {
        Ok(p) => Some(p),
        Err(e) => {
            all_diagnostics.push(AstDiagnostic {
                line: e.line as u32,
                column: e.column as u32,
                message: e.message.clone(),
                severity: crate::eval::DiagnosticSeverity::Error,
                code: None,
                help: None,
                suggestion: None,
            });
            None
        }
    };

    let mut functions = HashMap::new();
    let mut variables = HashMap::new();
    let mut enums = HashMap::new();
    let mut structs = HashMap::new();

    if let Some(ref prog) = program {
        // Type check
        let imports = HashSet::new();
        let analysis = type_checker::check_types(prog, &imports);
        all_diagnostics.extend(analysis.diagnostics);

        // Lint
        let lint_config = linter::LintConfig::default();
        let lint_result = linter::lint(prog, &lint_config);
        all_diagnostics.extend(lint_result.diagnostics);

        // Extract symbols
        extract_symbols(prog, &mut functions, &mut variables, &mut enums, &mut structs);
    }

    let state = DocumentState {
        source: source.to_string(),
        program,
        functions,
        variables,
        enums,
        structs,
    };

    (state, all_diagnostics)
}

/// Extract top-level symbols from a program.
pub fn extract_symbols(
    program: &Program,
    functions: &mut HashMap<String, FunctionSymbol>,
    variables: &mut HashMap<String, VariableSymbol>,
    enums: &mut HashMap<String, EnumSymbol>,
    structs: &mut HashMap<String, StructSymbol>,
) {
    for stmt in &program.statements {
        match &stmt.kind {
            StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
                let params: Vec<String> = fdef.params.iter().map(|p| {
                    let mut s = String::new();
                    if p.rest {
                        s.push_str("...");
                    }
                    s.push_str(&p.name);
                    if let Some(ty) = &p.type_annotation {
                        s.push_str(": ");
                        s.push_str(ty);
                    }
                    s
                }).collect();

                functions.insert(fdef.name.clone(), FunctionSymbol {
                    name: fdef.name.clone(),
                    params,
                    return_type: fdef.return_type.clone(),
                    line: fdef.span.start_line,
                    col: fdef.span.start_col,
                });
            }
            StatementKind::Let { name, type_annotation, .. } => {
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: false,
                    constant: false,
                    type_annotation: type_annotation.clone(),
                    line: stmt.span.start_line,
                    col: stmt.span.start_col,
                });
            }
            StatementKind::LetMut { name, type_annotation, .. } => {
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: true,
                    constant: false,
                    type_annotation: type_annotation.clone(),
                    line: stmt.span.start_line,
                    col: stmt.span.start_col,
                });
            }
            StatementKind::ConstDef { name, type_annotation, .. } => {
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: false,
                    constant: true,
                    type_annotation: type_annotation.clone(),
                    line: stmt.span.start_line,
                    col: stmt.span.start_col,
                });
            }
            StatementKind::EnumDef { name, variants } => {
                let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
                enums.insert(name.clone(), EnumSymbol {
                    name: name.clone(),
                    variants: variant_names,
                    line: stmt.span.start_line,
                    col: stmt.span.start_col,
                });
            }
            StatementKind::StructDef { name, fields } => {
                let field_info: Vec<(String, Option<String>)> = fields.iter().map(|f| {
                    (f.name.clone(), f.type_annotation.clone())
                }).collect();
                structs.insert(name.clone(), StructSymbol {
                    name: name.clone(),
                    fields: field_info,
                    line: stmt.span.start_line,
                    col: stmt.span.start_col,
                });
            }
            _ => {}
        }
    }
}

/// Convert an AstDiagnostic (1-based) to an LSP Diagnostic (0-based).
/// Uses source text to give diagnostics a non-zero-width range when possible.
pub fn to_lsp_diagnostic(d: &AstDiagnostic) -> Diagnostic {
    to_lsp_diagnostic_with_source(d, None)
}

/// Convert an AstDiagnostic (1-based) to an LSP Diagnostic (0-based),
/// optionally using source text to compute a better end position.
pub fn to_lsp_diagnostic_with_source(d: &AstDiagnostic, source: Option<&str>) -> Diagnostic {
    let line = d.line.saturating_sub(1);
    let col = d.column.saturating_sub(1);

    // Try to compute a non-zero-width end position and convert to UTF-16
    let (start_utf16, end_utf16) = if let Some(src) = source {
        if let Some(src_line) = src.lines().nth(line as usize) {
            let chars: Vec<char> = src_line.chars().collect();
            let start = col as usize;
            let end_char = if start < chars.len() && is_ident_start(chars[start]) {
                let mut end = start;
                while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
                    end += 1;
                }
                end as u32
            } else {
                (col + 1).min(chars.len() as u32)
            };
            (char_col_to_utf16(src_line, col), char_col_to_utf16(src_line, end_char))
        } else {
            (col, col + 1)
        }
    } else {
        (col, col + 1)
    };

    let severity = match d.severity {
        crate::eval::DiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
        crate::eval::DiagnosticSeverity::Warning => DiagnosticSeverity::WARNING,
        crate::eval::DiagnosticSeverity::Info => DiagnosticSeverity::INFORMATION,
    };

    let mut message = d.message.clone();
    if let Some(ref help) = d.help {
        message.push_str("\n\n");
        message.push_str(help);
    }
    if let Some(ref suggestion) = d.suggestion {
        message.push_str("\n\n");
        message.push_str(suggestion);
    }

    Diagnostic {
        range: Range {
            start: Position { line, character: start_utf16 },
            end: Position { line, character: end_utf16 },
        },
        severity: Some(severity),
        code: d.code.as_ref().map(|c| NumberOrString::String(c.clone())),
        source: Some("magi".to_string()),
        message,
        ..Default::default()
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// Convert a 0-based char column to a 0-based UTF-16 code unit offset.
/// For ASCII-only lines this is identity. For non-BMP characters, each
/// character may occupy 2 UTF-16 code units.
pub fn char_col_to_utf16(line_text: &str, char_col: u32) -> u32 {
    let mut utf16_offset: u32 = 0;
    for (i, ch) in line_text.chars().enumerate() {
        if i as u32 >= char_col {
            break;
        }
        utf16_offset += ch.len_utf16() as u32;
    }
    utf16_offset
}

/// Convert a 0-based UTF-16 code unit offset to a 0-based char column.
pub fn utf16_to_char_col(line_text: &str, utf16_col: u32) -> u32 {
    let mut utf16_offset: u32 = 0;
    for (i, ch) in line_text.chars().enumerate() {
        if utf16_offset >= utf16_col {
            return i as u32;
        }
        utf16_offset += ch.len_utf16() as u32;
    }
    line_text.chars().count() as u32
}

/// Find the word (identifier) at a given cursor position in source text.
/// `character` is a 0-based UTF-16 code unit offset (per LSP spec).
pub fn find_word_at_position(source: &str, line: u32, character: u32) -> Option<String> {
    let target_line = source.lines().nth(line as usize)?;
    let chars: Vec<char> = target_line.chars().collect();
    let col = utf16_to_char_col(target_line, character) as usize;

    if col > chars.len() {
        return None;
    }

    // Scan backwards for start of identifier
    let mut start = col;
    while start > 0 && is_ident_char_unicode(chars[start - 1]) {
        start -= 1;
    }

    // Scan forwards for end of identifier
    let mut end = col;
    while end < chars.len() && is_ident_char_unicode(chars[end]) {
        end += 1;
    }

    if start == end {
        return None;
    }

    Some(chars[start..end].iter().collect())
}

fn is_ident_char_unicode(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_valid_document() {
        let source = "let x = 5;\nfn foo() { x }";
        let (state, diagnostics) = analyze_document(source);
        assert!(state.program.is_some());
        assert!(state.variables.contains_key("x"));
        assert!(state.functions.contains_key("foo"));
        // May have warnings but should parse successfully
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|d| matches!(d.severity, crate::eval::DiagnosticSeverity::Error))
            .collect();
        // No parse errors expected
        assert!(errors.is_empty() || !errors.iter().any(|d| d.code.is_none()), "unexpected parse errors: {:?}", errors);
    }

    #[test]
    fn test_analyze_invalid_document() {
        let source = "let = ;";
        let (state, diagnostics) = analyze_document(source);
        assert!(state.program.is_none());
        assert!(!diagnostics.is_empty());
    }

    #[test]
    fn test_extract_enum_symbol() {
        let source = "enum Color { Red, Green, Blue }";
        let (state, _) = analyze_document(source);
        let e = state.enums.get("Color").unwrap();
        assert_eq!(e.variants, vec!["Red", "Green", "Blue"]);
    }

    #[test]
    fn test_extract_struct_symbol() {
        let source = "struct Point { x: float64, y: float64 }";
        let (state, _) = analyze_document(source);
        let s = state.structs.get("Point").unwrap();
        assert_eq!(s.fields.len(), 2);
        assert_eq!(s.fields[0].0, "x");
    }

    #[test]
    fn test_find_word_at_position() {
        let source = "let my_var = 42;";
        assert_eq!(find_word_at_position(source, 0, 4), Some("my_var".to_string()));
        assert_eq!(find_word_at_position(source, 0, 7), Some("my_var".to_string()));
        assert_eq!(find_word_at_position(source, 0, 0), Some("let".to_string()));
        assert_eq!(find_word_at_position(source, 0, 13), Some("42".to_string()));
    }

    #[test]
    fn test_find_word_at_position_spaces() {
        let source = "let x = 5;";
        assert_eq!(find_word_at_position(source, 0, 6), None); // space between = and 5
    }

    #[test]
    fn test_to_lsp_diagnostic() {
        let d = AstDiagnostic {
            line: 1,
            column: 5,
            message: "test error".to_string(),
            severity: crate::eval::DiagnosticSeverity::Error,
            code: Some("E200".to_string()),
            help: None,
            suggestion: None,
        };
        let lsp_d = to_lsp_diagnostic(&d);
        assert_eq!(lsp_d.range.start.line, 0);
        assert_eq!(lsp_d.range.start.character, 4);
        // Without source, end is start + 1
        assert_eq!(lsp_d.range.end.character, 5);
        assert_eq!(lsp_d.severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn test_to_lsp_diagnostic_with_source_ident_range() {
        let d = AstDiagnostic {
            line: 1,
            column: 5,
            message: "unknown variable".to_string(),
            severity: crate::eval::DiagnosticSeverity::Error,
            code: Some("E100".to_string()),
            help: None,
            suggestion: None,
        };
        let source = "let my_var = 42;";
        let lsp_d = to_lsp_diagnostic_with_source(&d, Some(source));
        assert_eq!(lsp_d.range.start.line, 0);
        assert_eq!(lsp_d.range.start.character, 4); // 'm' in my_var
        assert_eq!(lsp_d.range.end.character, 10); // end of 'my_var'
    }

    #[test]
    fn test_char_col_to_utf16_ascii() {
        assert_eq!(char_col_to_utf16("hello", 3), 3);
    }

    #[test]
    fn test_utf16_to_char_col_ascii() {
        assert_eq!(utf16_to_char_col("hello", 3), 3);
    }
}
