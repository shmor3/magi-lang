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
    pub is_async: bool,
    pub line: u32,
    pub col: u32,
}

/// Extracted symbol information for a variable.
#[derive(Debug, Clone)]
pub struct VariableSymbol {
    pub name: String,
    pub mutable: bool,
    pub constant: bool,
    pub is_type_alias: bool,
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
        extract_symbols(prog, source, &mut functions, &mut variables, &mut enums, &mut structs);
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

/// Find the 1-based column of `name` in the given 1-based source line.
/// Uses word-boundary matching to avoid matching substrings of other identifiers.
fn find_name_col(source: &str, line: u32, name: &str) -> Option<u32> {
    let line_text = source.lines().nth(line.saturating_sub(1) as usize)?;
    let name_bytes = name.as_bytes();
    let mut start = 0;
    while let Some(offset) = line_text[start..].find(name) {
        let abs_offset = start + offset;
        let before_ok = abs_offset == 0
            || !line_text.as_bytes().get(abs_offset - 1)
                .map_or(false, |&b| b.is_ascii_alphanumeric() || b == b'_');
        let after_pos = abs_offset + name_bytes.len();
        let after_ok = after_pos >= line_text.len()
            || !line_text.as_bytes().get(after_pos)
                .map_or(false, |&b| b.is_ascii_alphanumeric() || b == b'_');
        if before_ok && after_ok {
            let char_col = line_text[..abs_offset].chars().count() as u32;
            return Some(char_col + 1);
        }
        // Advance past current match start, ensuring we land on a UTF-8 char boundary
        start = abs_offset + name_bytes.len().max(1);
        // Ensure we're on a char boundary
        while start < line_text.len() && !line_text.is_char_boundary(start) {
            start += 1;
        }
    }
    // Fallback to substring match if no word-boundary match found
    let byte_offset = line_text.find(name)?;
    let char_col = line_text[..byte_offset].chars().count() as u32;
    Some(char_col + 1)
}

/// Extract top-level symbols from a program.
pub fn extract_symbols(
    program: &Program,
    source: &str,
    functions: &mut HashMap<String, FunctionSymbol>,
    variables: &mut HashMap<String, VariableSymbol>,
    enums: &mut HashMap<String, EnumSymbol>,
    structs: &mut HashMap<String, StructSymbol>,
) {
    for stmt in &program.statements {
        match &stmt.kind {
            StatementKind::FunctionDef(fdef) | StatementKind::AsyncFunctionDef(fdef) => {
                let is_async = matches!(&stmt.kind, StatementKind::AsyncFunctionDef(_));
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

                let name_col = find_name_col(source, fdef.span.start_line, &fdef.name)
                    .unwrap_or(fdef.span.start_col);
                functions.insert(fdef.name.clone(), FunctionSymbol {
                    name: fdef.name.clone(),
                    params,
                    return_type: fdef.return_type.clone(),
                    is_async,
                    line: fdef.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::Let { name, type_annotation, .. } => {
                let name_col = find_name_col(source, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: false,
                    constant: false,
                    is_type_alias: false,
                    type_annotation: type_annotation.clone(),
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::LetMut { name, type_annotation, .. } => {
                let name_col = find_name_col(source, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: true,
                    constant: false,
                    is_type_alias: false,
                    type_annotation: type_annotation.clone(),
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::ConstDef { name, type_annotation, .. } => {
                let name_col = find_name_col(source, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: false,
                    constant: true,
                    is_type_alias: false,
                    type_annotation: type_annotation.clone(),
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::EnumDef { name, variants } => {
                let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
                let name_col = find_name_col(source, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                enums.insert(name.clone(), EnumSymbol {
                    name: name.clone(),
                    variants: variant_names,
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::StructDef { name, fields } => {
                let field_info: Vec<(String, Option<String>)> = fields.iter().map(|f| {
                    (f.name.clone(), f.type_annotation.clone())
                }).collect();
                let name_col = find_name_col(source, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                structs.insert(name.clone(), StructSymbol {
                    name: name.clone(),
                    fields: field_info,
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::TypeAlias { name, target } => {
                let name_col = find_name_col(source, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: false,
                    constant: false,
                    is_type_alias: true,
                    type_annotation: Some(target.clone()),
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::ModuleDef { name, body } => {
                let name_col = find_name_col(source, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: false,
                    constant: false,
                    is_type_alias: false,
                    type_annotation: Some("module".to_string()),
                    line: stmt.span.start_line,
                    col: name_col,
                });
                // Also extract symbols from module body
                let module_program = crate::syntax::ast::Program {
                    statements: body.statements.clone(),
                    span: body.span,
                };
                extract_symbols(&module_program, source, functions, variables, enums, structs);
            }
            StatementKind::LetDestructure { pattern, mutable, .. } => {
                // Extract variable names from destructure patterns
                let names = destructure_names(pattern);
                for name in names {
                    let name_col = find_name_col(source, stmt.span.start_line, &name)
                        .unwrap_or(stmt.span.start_col);
                    variables.insert(name.clone(), VariableSymbol {
                        name,
                        mutable: *mutable,
                        constant: false,
                        is_type_alias: false,
                        type_annotation: None,
                        line: stmt.span.start_line,
                        col: name_col,
                    });
                }
            }
            StatementKind::Use { path, alias, glob } => {
                if !glob {
                    // For `use foo::bar` or `use foo::bar as baz`, register the local name
                    let local_name = if let Some(a) = alias {
                        a.clone()
                    } else if let Some(last) = path.last() {
                        last.clone()
                    } else {
                        continue;
                    };
                    let name_col = if let Some(a) = alias {
                        find_name_col(source, stmt.span.start_line, a)
                            .unwrap_or(stmt.span.start_col)
                    } else if let Some(last) = path.last() {
                        find_name_col(source, stmt.span.start_line, last)
                            .unwrap_or(stmt.span.start_col)
                    } else {
                        stmt.span.start_col
                    };
                    variables.insert(local_name.clone(), VariableSymbol {
                        name: local_name,
                        mutable: false,
                        constant: false,
                        is_type_alias: false,
                        type_annotation: Some(format!("import({})", path.join("::"))),
                        line: stmt.span.start_line,
                        col: name_col,
                    });
                }
            }
            _ => {}
        }
    }
}

/// Extract variable names from a destructure pattern.
fn destructure_names(pattern: &DestructurePattern) -> Vec<String> {
    match pattern {
        DestructurePattern::Array(elems) => {
            let mut names = Vec::new();
            for elem in elems {
                match elem {
                    DestructureElement::Name(n) => names.push(n.clone()),
                    DestructureElement::Rest(n) => names.push(n.clone()),
                }
            }
            names
        }
        DestructurePattern::Map(entries) => {
            entries.iter().map(|(key, alias)| {
                alias.as_ref().unwrap_or(key).clone()
            }).collect()
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
            let char_len = chars.len() as u32;
            // Clamp start to within the line
            let start = (col as usize).min(chars.len());
            let end_char = if start < chars.len() && is_ident_start(chars[start]) {
                // Identifier: scan alphanumeric + underscore
                let mut end = start;
                while end < chars.len() && is_ident_char_unicode(chars[end]) {
                    end += 1;
                }
                end as u32
            } else if start < chars.len() && chars[start].is_ascii_digit() {
                // Numeric literal: scan digits, dots, hex chars
                let mut end = start;
                while end < chars.len()
                    && (chars[end].is_ascii_alphanumeric() || chars[end] == '.' || chars[end] == '_')
                {
                    end += 1;
                }
                end as u32
            } else if start < chars.len() && (chars[start] == '"' || chars[start] == '\'') {
                // String literal: scan to matching close quote
                let quote = chars[start];
                let mut end = start + 1;
                while end < chars.len() && chars[end] != quote {
                    if chars[end] == '\\' {
                        end += 1; // skip escaped char
                    }
                    end += 1;
                }
                if end < chars.len() {
                    end += 1; // include closing quote
                }
                end as u32
            } else if start < chars.len() {
                // Operator or other single char — highlight at least one char
                (start as u32 + 1).min(char_len)
            } else {
                // Column past end of line — clamp to line end, produce zero-width
                char_len
            };
            let clamped_start = (col).min(char_len);
            (char_col_to_utf16(src_line, clamped_start), char_col_to_utf16(src_line, end_char))
        } else {
            // Line not found in source — col is char-based, use as-is (no conversion possible)
            (col, col.saturating_add(1))
        }
    } else {
        // No source available — col is char-based, use as-is
        (col, col.saturating_add(1))
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

/// Detect if cursor is on an `EnumName::Variant` pattern.
/// Returns `(enum_name, variant_name)` if found, or `None` if cursor is on a plain identifier.
/// `character` is a 0-based UTF-16 code unit offset (per LSP spec).
pub fn find_enum_variant_at_position(source: &str, line: u32, character: u32) -> Option<(String, String)> {
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

    let word: String = chars[start..end].iter().collect();

    // Check if there's `::` before this identifier (cursor is on variant)
    if start >= 2 && chars[start - 1] == ':' && chars[start - 2] == ':' {
        // Scan backwards from `::` to find the enum name
        let enum_end = start - 2;
        let mut enum_start = enum_end;
        while enum_start > 0 && is_ident_char_unicode(chars[enum_start - 1]) {
            enum_start -= 1;
        }
        if enum_start < enum_end {
            let enum_name: String = chars[enum_start..enum_end].iter().collect();
            return Some((enum_name, word));
        }
    }

    // Check if there's `::` after this identifier (cursor is on enum name)
    if end + 1 < chars.len() && chars[end] == ':' && chars[end + 1] == ':' {
        // Scan forwards from `::` to find the variant name
        let variant_start = end + 2;
        let mut variant_end = variant_start;
        while variant_end < chars.len() && is_ident_char_unicode(chars[variant_end]) {
            variant_end += 1;
        }
        if variant_start < variant_end {
            let variant: String = chars[variant_start..variant_end].iter().collect();
            return Some((word, variant));
        }
    }

    None
}

/// Detect if the cursor is in a dot-access context (e.g., `point.`).
/// Returns the identifier before the dot if found.
/// `character` is a 0-based UTF-16 code unit offset (per LSP spec).
pub fn find_dot_receiver_at_position(source: &str, line: u32, character: u32) -> Option<String> {
    let target_line = source.lines().nth(line as usize)?;
    let chars: Vec<char> = target_line.chars().collect();
    let col = utf16_to_char_col(target_line, character) as usize;

    if col == 0 || col > chars.len() {
        return None;
    }

    // Walk backwards from cursor to find the dot
    let mut pos = col;
    // Skip any partial identifier the user is typing after the dot
    while pos > 0 && is_ident_char_unicode(chars[pos - 1]) {
        pos -= 1;
    }

    // Check for dot
    if pos == 0 || chars[pos - 1] != '.' {
        return None;
    }
    pos -= 1; // skip the dot

    // Now scan backwards for the receiver identifier
    let recv_end = pos;
    let mut recv_start = recv_end;
    while recv_start > 0 && is_ident_char_unicode(chars[recv_start - 1]) {
        recv_start -= 1;
    }

    if recv_start == recv_end {
        return None;
    }

    Some(chars[recv_start..recv_end].iter().collect())
}

/// Find the function name and argument index at a call site.
/// Used for signature help. Returns `(function_name, active_param_index)`.
/// `character` is a 0-based UTF-16 code unit offset (per LSP spec).
pub fn find_call_context_at_position(source: &str, line: u32, character: u32) -> Option<(String, u32)> {
    let target_line = source.lines().nth(line as usize)?;
    let chars: Vec<char> = target_line.chars().collect();
    let col = utf16_to_char_col(target_line, character) as usize;

    if col > chars.len() {
        return None;
    }

    // Scan backwards to find the matching `(`
    let mut depth = 0i32;
    let mut pos = col;
    let mut commas = 0u32;

    // Walk backwards from cursor
    while pos > 0 {
        pos -= 1;
        match chars[pos] {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    // Found our opening paren. The function name is before it.
                    let mut name_end = pos;
                    // Skip whitespace
                    while name_end > 0 && chars[name_end - 1] == ' ' {
                        name_end -= 1;
                    }
                    let mut name_start = name_end;
                    while name_start > 0 && is_ident_char_unicode(chars[name_start - 1]) {
                        name_start -= 1;
                    }
                    if name_start < name_end {
                        let name: String = chars[name_start..name_end].iter().collect();
                        return Some((name, commas));
                    }
                    return None;
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                commas += 1;
            }
            _ => {}
        }
    }

    None
}

/// Try to determine the struct type of a variable by looking at its initializer in the AST.
/// Returns the struct name if the variable is initialized with a struct constructor.
pub fn find_variable_struct_type(state: &DocumentState, var_name: &str) -> Option<String> {
    let program = state.program.as_ref()?;
    for stmt in &program.statements {
        match &stmt.kind {
            StatementKind::Let { name, value, type_annotation, .. }
            | StatementKind::LetMut { name, value, type_annotation, .. }
            | StatementKind::ConstDef { name, value, type_annotation, .. } => {
                if name == var_name {
                    // Check type annotation first
                    if let Some(ta) = type_annotation {
                        if state.structs.contains_key(ta) {
                            return Some(ta.clone());
                        }
                    }
                    // Check if RHS is a struct constructor
                    if let ExpressionKind::StructConstruct { name: sname, .. } = &value.kind {
                        return Some(sname.clone());
                    }
                }
            }
            _ => {}
        }
    }
    None
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

    #[test]
    fn test_find_name_col_utf8_no_panic() {
        // Source with multi-byte characters before an identifier
        let source = "let αβγ = 1;\nlet foo = αβγ;";
        // "foo" is on line 2
        let result = find_name_col(source, 2, "foo");
        assert!(result.is_some(), "Should find 'foo' on line 2");
        assert_eq!(result.unwrap(), 5); // "let " = 4 chars + 1
    }

    #[test]
    fn test_find_name_col_repeated_substring_utf8() {
        // Ensure we don't panic when skipping past multi-byte chars
        let source = "let café_x = 1; let café_y = 2;";
        let result = find_name_col(source, 1, "café_y");
        assert!(result.is_some(), "Should find 'café_y'");
    }

    #[test]
    fn test_find_enum_variant_cursor_on_variant() {
        let source = "let c = Color::Red;";
        // Cursor on "Red" (col 15)
        let result = find_enum_variant_at_position(source, 0, 15);
        assert_eq!(result, Some(("Color".to_string(), "Red".to_string())));
    }

    #[test]
    fn test_find_enum_variant_cursor_on_enum_name() {
        let source = "let c = Color::Red;";
        // Cursor on "Color" (col 8)
        let result = find_enum_variant_at_position(source, 0, 8);
        assert_eq!(result, Some(("Color".to_string(), "Red".to_string())));
    }

    #[test]
    fn test_find_enum_variant_no_double_colon() {
        let source = "let c = foo;";
        let result = find_enum_variant_at_position(source, 0, 8);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_dot_receiver() {
        let source = "point.x";
        // Cursor after the dot at col 6 (on "x")
        let result = find_dot_receiver_at_position(source, 0, 6);
        assert_eq!(result, Some("point".to_string()));
    }

    #[test]
    fn test_find_dot_receiver_no_dot() {
        let source = "point";
        let result = find_dot_receiver_at_position(source, 0, 3);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_call_context_first_param() {
        let source = "foo(x, y)";
        // Cursor inside parens, after "x"
        let result = find_call_context_at_position(source, 0, 4);
        assert_eq!(result, Some(("foo".to_string(), 0)));
    }

    #[test]
    fn test_find_call_context_second_param() {
        let source = "foo(x, y)";
        // Cursor after comma
        let result = find_call_context_at_position(source, 0, 7);
        assert_eq!(result, Some(("foo".to_string(), 1)));
    }

    #[test]
    fn test_find_call_context_no_call() {
        let source = "let x = 5;";
        let result = find_call_context_at_position(source, 0, 5);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_variable_struct_type() {
        let source = "struct Point { x: float64, y: float64 }\nlet p = Point { x: 1.0, y: 2.0 };";
        let (state, _) = analyze_document(source);
        let result = find_variable_struct_type(&state, "p");
        assert_eq!(result, Some("Point".to_string()));
    }
}
