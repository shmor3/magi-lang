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
                    type_annotation: type_annotation.clone(),
                    line: stmt.span.start_line,
                    col: stmt.span.start_col,
                });
            }
            StatementKind::LetMut { name, type_annotation, .. } => {
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: true,
                    type_annotation: type_annotation.clone(),
                    line: stmt.span.start_line,
                    col: stmt.span.start_col,
                });
            }
            StatementKind::ConstDef { name, type_annotation, .. } => {
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: false,
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
pub fn to_lsp_diagnostic(d: &AstDiagnostic) -> Diagnostic {
    let line = d.line.saturating_sub(1);
    let col = d.column.saturating_sub(1);

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
            start: Position { line, character: col },
            end: Position { line, character: col },
        },
        severity: Some(severity),
        code: d.code.as_ref().map(|c| NumberOrString::String(c.clone())),
        source: Some("magi".to_string()),
        message,
        ..Default::default()
    }
}

/// Find the word (identifier) at a given cursor position in source text.
/// Uses char indices (not byte indices) for correct Unicode handling.
pub fn find_word_at_position(source: &str, line: u32, character: u32) -> Option<String> {
    let target_line = source.lines().nth(line as usize)?;
    let chars: Vec<char> = target_line.chars().collect();
    let col = character as usize;

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
        assert_eq!(lsp_d.severity, Some(DiagnosticSeverity::ERROR));
    }
}
