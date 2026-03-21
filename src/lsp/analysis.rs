//! Document analysis — parse, type-check, lint, and extract symbols.

use crate::linter;
use crate::syntax::ast::*;
use crate::syntax::parser::parse_v2_recovering;
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

    let (parsed_program, parse_errors) = parse_v2_recovering(source);

    // Add all parse errors as diagnostics
    for e in &parse_errors {
        all_diagnostics.push(AstDiagnostic {
            line: e.line as u32,
            column: e.column as u32,
            message: e.message.clone(),
            severity: crate::eval::DiagnosticSeverity::Error,
            code: None,
            help: None,
            suggestion: None,
        source_file: None,
        });
    }

    // Use the partial program even if there were errors (for symbol extraction).
    // Only run type checker and linter if there were no parse errors, since
    // a partial AST may cause confusing secondary diagnostics.
    let program = if parse_errors.is_empty() {
        Some(parsed_program)
    } else if parsed_program.statements.is_empty() {
        None
    } else {
        Some(parsed_program)
    };

    let mut functions = HashMap::new();
    let mut variables = HashMap::new();
    let mut enums = HashMap::new();
    let mut structs = HashMap::new();

    if let Some(ref prog) = program {
        if parse_errors.is_empty() {
            // Only run type checker and linter on a complete (error-free) AST.
            // Partial ASTs from error recovery would produce confusing secondary diagnostics.
            let imports: HashSet<String> = prog.statements.iter()
                .filter_map(|s| if let StatementKind::Import(id) = &s.kind { Some(id.clone()) } else { None })
                .collect();
            let analysis = type_checker::check_types(prog, &imports);
            all_diagnostics.extend(analysis.diagnostics);

            let lint_config = linter::LintConfig::default();
            let lint_result = linter::lint(prog, &lint_config);
            all_diagnostics.extend(lint_result.diagnostics);

            // Deduplicate diagnostics that may overlap between type checker and linter.
            // Key: (line, column, code) — if two diagnostics share the same location and
            // error code, keep only the first one (type checker takes priority since it
            // runs first).
            deduplicate_diagnostics(&mut all_diagnostics);
        }

        // Extract symbols even from partial programs for IDE features
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

/// Deduplicate diagnostics by (line, column, code).
/// When two diagnostics share the same location and error code, the first one wins
/// (type checker diagnostics are added first, so they take priority).
/// Also deduplicates diagnostics at the same location with no code.
fn deduplicate_diagnostics(diagnostics: &mut Vec<AstDiagnostic>) {
    let mut seen = HashSet::new();
    diagnostics.retain(|d| {
        let key = (d.line, d.column, d.code.clone().unwrap_or_default(), d.message.clone());
        seen.insert(key)
    });
}

/// Build a byte-offset index of line starts for O(1) line lookup.
fn build_line_index(source: &str) -> Vec<usize> {
    let mut offsets = vec![0]; // line 1 starts at byte 0
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' && i < source.len() {
            offsets.push(i + 1);
        }
    }
    offsets
}

/// Get the text of a 1-based line using a pre-built line index.
fn get_line<'a>(source: &'a str, line_index: &[usize], line: u32) -> Option<&'a str> {
    let idx = line.saturating_sub(1) as usize;
    let start = *line_index.get(idx)?;
    let end = line_index.get(idx + 1).map(|&e| e.saturating_sub(1)).unwrap_or(source.len());
    // Trim trailing \r for CRLF
    let line_text = &source[start..end];
    Some(line_text.trim_end_matches('\r'))
}

/// Find the 1-based column of `name` in the given 1-based source line.
/// Uses word-boundary matching to avoid matching substrings of other identifiers.
#[cfg(test)]
fn find_name_col(source: &str, line: u32, name: &str) -> Option<u32> {
    let line_text = source.lines().nth(line.saturating_sub(1) as usize)?;
    find_name_col_in_line(line_text, name)
}

/// Find the 1-based column of `name` in a line using a pre-built line index.
fn find_name_col_indexed(source: &str, line_index: &[usize], line: u32, name: &str) -> Option<u32> {
    let line_text = get_line(source, line_index, line)?;
    find_name_col_in_line(line_text, name)
}

/// Core implementation: find the 1-based column of `name` in a line string.
fn find_name_col_in_line(line_text: &str, name: &str) -> Option<u32> {
    let name_bytes = name.as_bytes();
    let mut start = 0;
    while let Some(offset) = line_text[start..].find(name) {
        let abs_offset = start + offset;
        // Check character before match is not an identifier char (Unicode-aware)
        let before_ok = abs_offset == 0
            || !line_text[..abs_offset].chars().next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let after_pos = abs_offset + name_bytes.len();
        // Check character after match is not an identifier char (Unicode-aware)
        let after_ok = after_pos >= line_text.len()
            || !line_text[after_pos..].chars().next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
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
    // No word-boundary match found — return None rather than a potentially
    // incorrect substring match position.
    None
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
    let line_index = build_line_index(source);
    extract_symbols_indexed(program, source, &line_index, functions, variables, enums, structs);
}

/// Extract top-level symbols using a pre-built line index for O(1) line lookup.
fn extract_symbols_indexed(
    program: &Program,
    source: &str,
    line_index: &[usize],
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
                        s.push_str(&ty.to_string());
                    }
                    s
                }).collect();

                let name_col = find_name_col_indexed(source, line_index, fdef.span.start_line, &fdef.name)
                    .unwrap_or(fdef.span.start_col);
                functions.insert(fdef.name.clone(), FunctionSymbol {
                    name: fdef.name.clone(),
                    params,
                    return_type: fdef.return_type.as_ref().map(|t| t.to_string()),
                    is_async,
                    line: fdef.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::Let { name, type_annotation, .. } => {
                let name_col = find_name_col_indexed(source, line_index, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: false,
                    constant: false,
                    is_type_alias: false,
                    type_annotation: type_annotation.as_ref().map(|t| t.to_string()),
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::LetMut { name, type_annotation, .. } => {
                let name_col = find_name_col_indexed(source, line_index, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: true,
                    constant: false,
                    is_type_alias: false,
                    type_annotation: type_annotation.as_ref().map(|t| t.to_string()),
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::ConstDef { name, type_annotation, .. } => {
                let name_col = find_name_col_indexed(source, line_index, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: false,
                    constant: true,
                    is_type_alias: false,
                    type_annotation: type_annotation.as_ref().map(|t| t.to_string()),
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::EnumDef { name, variants, .. } => {
                let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
                let name_col = find_name_col_indexed(source, line_index, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                enums.insert(name.clone(), EnumSymbol {
                    name: name.clone(),
                    variants: variant_names,
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::StructDef { name, fields, .. } => {
                let field_info: Vec<(String, Option<String>)> = fields.iter().map(|f| {
                    (f.name.clone(), f.type_annotation.as_ref().map(|t| t.to_string()))
                }).collect();
                let name_col = find_name_col_indexed(source, line_index, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                structs.insert(name.clone(), StructSymbol {
                    name: name.clone(),
                    fields: field_info,
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::TypeAlias { name, target } => {
                let name_col = find_name_col_indexed(source, line_index, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                variables.insert(name.clone(), VariableSymbol {
                    name: name.clone(),
                    mutable: false,
                    constant: false,
                    is_type_alias: true,
                    type_annotation: Some(target.to_string()),
                    line: stmt.span.start_line,
                    col: name_col,
                });
            }
            StatementKind::ModuleDef { name, body } => {
                let name_col = find_name_col_indexed(source, line_index, stmt.span.start_line, name)
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
                // Also extract symbols from module body (reuse same line index)
                let module_program = crate::syntax::ast::Program {
                    statements: body.statements.clone(),
                    span: body.span,
                };
                extract_symbols_indexed(&module_program, source, line_index, functions, variables, enums, structs);
            }
            StatementKind::LetDestructure { pattern, mutable, .. } => {
                // Extract variable names from destructure patterns
                let names = destructure_names(pattern);
                for name in names {
                    let name_col = find_name_col_indexed(source, line_index, stmt.span.start_line, &name)
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
            StatementKind::Use { path, alias, glob, .. } => {
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
                        find_name_col_indexed(source, line_index, stmt.span.start_line, a)
                            .unwrap_or(stmt.span.start_col)
                    } else if let Some(last) = path.last() {
                        find_name_col_indexed(source, line_index, stmt.span.start_line, last)
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
            StatementKind::TestDef { name, .. } => {
                let name_col = find_name_col_indexed(source, line_index, stmt.span.start_line, name)
                    .unwrap_or(stmt.span.start_col);
                functions.insert(format!("test:{}", name), FunctionSymbol {
                    name: format!("test \"{}\"", name),
                    params: Vec::new(),
                    is_async: false,
                    line: stmt.span.start_line,
                    col: name_col,
                    return_type: None,
                });
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
                while end < chars.len() && is_ident_char(chars[end]) {
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
                        if end >= chars.len() { break; }
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

    // #206: Tag unused/deprecated diagnostics so IDEs can render them with fade-out
    let tags = d.code.as_deref().and_then(|code| match code {
        "W100" | "W101" | "W103" | "W109" | "W110" | "W202" => {
            Some(vec![DiagnosticTag::UNNECESSARY])
        }
        "W111" => Some(vec![DiagnosticTag::DEPRECATED]),
        _ => None,
    });

    Diagnostic {
        range: Range {
            start: Position { line, character: start_utf16 },
            end: Position { line, character: end_utf16 },
        },
        severity: Some(severity),
        code: d.code.as_ref().map(|c| NumberOrString::String(c.clone())),
        source: Some("magi".to_string()),
        message,
        tags,
        ..Default::default()
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
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
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }

    // Scan forwards for end of identifier
    let mut end = col;
    while end < chars.len() && is_ident_char(chars[end]) {
        end += 1;
    }

    if start == end {
        return None;
    }

    Some(chars[start..end].iter().collect())
}

pub fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
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
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }

    // Scan forwards for end of identifier
    let mut end = col;
    while end < chars.len() && is_ident_char(chars[end]) {
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
        while enum_start > 0 && is_ident_char(chars[enum_start - 1]) {
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
        while variant_end < chars.len() && is_ident_char(chars[variant_end]) {
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
    while pos > 0 && is_ident_char(chars[pos - 1]) {
        pos -= 1;
    }

    // Check for dot (also handles `?.` optional chain since `.` is still the last char)
    if pos == 0 || chars[pos - 1] != '.' {
        return None;
    }
    pos -= 1; // skip the dot
    // Skip `?` for optional chain (`?.`)
    if pos > 0 && chars[pos - 1] == '?' {
        pos -= 1;
    }

    // Now scan backwards for the receiver identifier
    let recv_end = pos;
    let mut recv_start = recv_end;
    while recv_start > 0 && is_ident_char(chars[recv_start - 1]) {
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
/// Supports multiline calls by scanning backwards across lines.
pub fn find_call_context_at_position(source: &str, line: u32, character: u32) -> Option<(String, u32)> {
    // Build a flat character buffer from line 0 through the cursor position,
    // then scan backwards from the cursor to find the matching `(`.
    let lines: Vec<&str> = source.lines().collect();
    if (line as usize) >= lines.len() {
        return None;
    }

    // Collect chars near the cursor position (limited to ~2000 chars to avoid O(file_size) allocation)
    const SCAN_LIMIT: usize = 2000;
    let mut all_chars: Vec<char> = Vec::with_capacity(SCAN_LIMIT + 200);
    let cursor_line = line as usize;
    // Start collecting from the cursor line backwards, up to SCAN_LIMIT chars
    {
        let col = utf16_to_char_col(lines[cursor_line], character) as usize;
        let line_chars: Vec<char> = lines[cursor_line].chars().collect();
        let end = col.min(line_chars.len());
        // Prepend lines from cursor line backwards until we have enough chars
        let mut collected_lines: Vec<Vec<char>> = vec![line_chars[..end].to_vec()];
        let mut total_chars = end;
        let mut li = cursor_line;
        while total_chars < SCAN_LIMIT && li > 0 {
            li -= 1;
            let lc: Vec<char> = lines[li].chars().collect();
            total_chars += lc.len() + 1; // +1 for newline
            collected_lines.push(lc);
        }
        // Build all_chars in forward order
        for (idx, lc) in collected_lines.iter().rev().enumerate() {
            if idx > 0 {
                all_chars.push('\n');
            }
            all_chars.extend(lc);
        }
    }

    // Apply final limit
    let scan_start = all_chars.len().saturating_sub(SCAN_LIMIT);
    let chars = &all_chars[scan_start..];

    let mut depth = 0i32;
    let mut pos = chars.len();
    let mut commas = 0u32;

    // Walk backwards from cursor, skipping string literals
    while pos > 0 {
        pos -= 1;
        // Skip string literals when scanning backwards
        if chars[pos] == '"' || chars[pos] == '\'' {
            let quote = chars[pos];
            if pos > 0 {
                pos -= 1;
                while pos > 0 {
                    if chars[pos] == quote {
                        let mut backslash_count = 0;
                        let mut bp = pos;
                        while bp > 0 && chars[bp - 1] == '\\' {
                            backslash_count += 1;
                            bp -= 1;
                        }
                        if backslash_count % 2 == 0 {
                            break;
                        }
                    }
                    pos -= 1;
                }
            }
            continue;
        }
        // Skip newlines
        if chars[pos] == '\n' || chars[pos] == '\r' {
            continue;
        }
        match chars[pos] {
            ')' | ']' | '}' => depth += 1,
            '[' | '{' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            '(' => {
                if depth == 0 {
                    // Found our opening paren. The function name is before it.
                    let mut name_end = pos;
                    // Skip whitespace and newlines
                    while name_end > 0 && (chars[name_end - 1] == ' ' || chars[name_end - 1] == '\n' || chars[name_end - 1] == '\r' || chars[name_end - 1] == '\t') {
                        name_end -= 1;
                    }
                    let mut name_start = name_end;
                    while name_start > 0 && is_ident_char(chars[name_start - 1]) {
                        name_start -= 1;
                    }
                    if name_start < name_end {
                        let name: String = chars[name_start..name_end].iter().collect();
                        return Some((name, commas));
                    }
                    return None;
                }
                // Only decrement if depth > 0 (consistent with `[`/`{` handling).
                // Prevents depth going negative from unbalanced `)` in incomplete source.
                if depth > 0 {
                    depth -= 1;
                }
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
                        let ta_str = ta.to_string(); if state.structs.contains_key(&ta_str) {
                            return Some(ta_str);
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
            source_file: None,
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
            source_file: None,
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

    // =========================================================================
    // Empty/whitespace document edge cases
    // =========================================================================

    #[test]
    fn test_analyze_empty_document() {
        let source = "";
        let (state, diagnostics) = analyze_document(source);
        // Empty source should parse to an empty program (no parse error)
        assert!(state.program.is_some());
        assert!(state.functions.is_empty());
        assert!(state.variables.is_empty());
        assert!(state.enums.is_empty());
        assert!(state.structs.is_empty());
        // No parse errors for empty file
        let errors: Vec<_> = diagnostics.iter()
            .filter(|d| d.code.is_none())
            .collect();
        assert!(errors.is_empty(), "empty doc should not produce parse errors: {:?}", errors);
    }

    #[test]
    fn test_analyze_whitespace_only_document() {
        let source = "   \n  \n\n  \t  ";
        let (state, _) = analyze_document(source);
        assert!(state.program.is_some());
        assert!(state.functions.is_empty());
    }

    #[test]
    fn test_analyze_document_with_comments_only() {
        let source = "// just a comment\n// another line";
        let (state, _) = analyze_document(source);
        assert!(state.program.is_some());
        assert!(state.functions.is_empty());
    }

    // =========================================================================
    // find_word_at_position edge cases
    // =========================================================================

    #[test]
    fn test_find_word_empty_source() {
        assert_eq!(find_word_at_position("", 0, 0), None);
    }

    #[test]
    fn test_find_word_nonexistent_line() {
        let source = "let x = 5;";
        // Line 1 doesn't exist (source has only line 0)
        assert_eq!(find_word_at_position(source, 1, 0), None);
        // Very large line number
        assert_eq!(find_word_at_position(source, 999, 0), None);
    }

    #[test]
    fn test_find_word_col_past_end_of_line() {
        let source = "hi";
        // col 2 is at len, should return "hi" (backward scan works)
        assert_eq!(find_word_at_position(source, 0, 2), Some("hi".to_string()));
        // col 100 is past end — utf16_to_char_col clamps to char count,
        // backward scan still finds the word at end of line
        assert_eq!(find_word_at_position(source, 0, 100), Some("hi".to_string()));
    }

    #[test]
    fn test_find_word_at_position_end_of_identifier() {
        let source = "foo bar";
        // col 3 is the space between foo and bar
        assert_eq!(find_word_at_position(source, 0, 3), Some("foo".to_string()));
    }

    #[test]
    fn test_find_word_at_position_unicode() {
        let source = "let αβγ = 42;";
        // cursor on 'α' (col 4)
        // Unicode alphanumeric characters are treated as identifier chars
        let word = find_word_at_position(source, 0, 4);
        assert_eq!(word, Some("αβγ".to_string()));
    }

    #[test]
    fn test_find_word_on_special_chars() {
        let source = "a + b";
        // col 2 is on '+'
        assert_eq!(find_word_at_position(source, 0, 2), None);
    }

    #[test]
    fn test_find_word_at_position_multiline() {
        let source = "let x = 5;\nlet y = 10;";
        assert_eq!(find_word_at_position(source, 1, 4), Some("y".to_string()));
    }

    // =========================================================================
    // find_call_context edge cases
    // =========================================================================

    #[test]
    fn test_find_call_context_nested_calls() {
        let source = "foo(bar(x), y)";
        // Cursor after "y" (col 13) -- inside outer foo(), after comma
        let result = find_call_context_at_position(source, 0, 13);
        assert_eq!(result, Some(("foo".to_string(), 1)));
    }

    #[test]
    fn test_find_call_context_nested_inner() {
        let source = "foo(bar(x), y)";
        // Cursor on "x" (col 8) -- inside inner bar()
        let result = find_call_context_at_position(source, 0, 8);
        assert_eq!(result, Some(("bar".to_string(), 0)));
    }

    #[test]
    fn test_find_call_context_unclosed_paren() {
        let source = "foo(x, y";
        // No closing paren -- cursor at col 7 (on "y")
        let result = find_call_context_at_position(source, 0, 7);
        assert_eq!(result, Some(("foo".to_string(), 1)));
    }

    #[test]
    fn test_find_call_context_empty_parens() {
        let source = "foo()";
        // Cursor inside empty parens (col 4)
        let result = find_call_context_at_position(source, 0, 4);
        assert_eq!(result, Some(("foo".to_string(), 0)));
    }

    #[test]
    fn test_find_call_context_string_with_comma() {
        let source = r#"foo("a,b", y)"#;
        // Cursor on "y" (after real comma) -- the comma inside the string should be skipped
        let result = find_call_context_at_position(source, 0, 12);
        assert_eq!(result, Some(("foo".to_string(), 1)));
    }

    #[test]
    fn test_find_call_context_string_with_paren() {
        let source = r#"foo("(x)", y)"#;
        // Cursor on "y" -- parens inside string should be skipped
        let result = find_call_context_at_position(source, 0, 12);
        assert_eq!(result, Some(("foo".to_string(), 1)));
    }

    #[test]
    fn test_find_call_context_array_arg() {
        let source = "foo([1, 2], y)";
        // Cursor on "y" -- comma inside array should be skipped via bracket tracking
        let result = find_call_context_at_position(source, 0, 13);
        assert_eq!(result, Some(("foo".to_string(), 1)));
    }

    #[test]
    fn test_find_call_context_no_function_name() {
        let source = "(x, y)";
        // Opening paren has no identifier before it
        let result = find_call_context_at_position(source, 0, 4);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_call_context_empty_source() {
        assert_eq!(find_call_context_at_position("", 0, 0), None);
    }

    #[test]
    fn test_find_call_context_unbalanced_close_paren() {
        // Unbalanced `)` before the actual call -- should not prevent finding `foo`
        // Source simulates editing: ") foo(x, y)" with cursor on "y"
        let source = ") foo(x, y)";
        let result = find_call_context_at_position(source, 0, 10);
        // Should still find foo with active_param=1 despite leading unbalanced `)`
        assert_eq!(result, Some(("foo".to_string(), 1)));
    }

    #[test]
    fn test_find_call_context_multiple_unbalanced_close_parens() {
        // Multiple unbalanced `)` -- depth should not go negative
        let source = "))) bar(a, b, c)";
        let result = find_call_context_at_position(source, 0, 14);
        assert_eq!(result, Some(("bar".to_string(), 2)));
    }

    // =========================================================================
    // find_dot_receiver edge cases
    // =========================================================================

    #[test]
    fn test_find_dot_receiver_partial_method() {
        // User is typing "point.le" (partial method name)
        let source = "point.le";
        let result = find_dot_receiver_at_position(source, 0, 8);
        assert_eq!(result, Some("point".to_string()));
    }

    #[test]
    fn test_find_dot_receiver_at_dot() {
        // Cursor right after dot (col 6 in "point." where dot is at col 5)
        let source = "point.";
        let result = find_dot_receiver_at_position(source, 0, 6);
        assert_eq!(result, Some("point".to_string()));
    }

    #[test]
    fn test_find_dot_receiver_col_0() {
        let source = ".method";
        let result = find_dot_receiver_at_position(source, 0, 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_dot_receiver_empty() {
        assert_eq!(find_dot_receiver_at_position("", 0, 0), None);
    }

    // =========================================================================
    // find_enum_variant edge cases
    // =========================================================================

    #[test]
    fn test_find_enum_variant_empty() {
        assert_eq!(find_enum_variant_at_position("", 0, 0), None);
    }

    #[test]
    fn test_find_enum_variant_col_past_end() {
        let source = "Color::Red";
        // col 100 is past end — utf16_to_char_col clamps to char count,
        // backward scan still finds the variant at end of line
        assert_eq!(find_enum_variant_at_position(source, 0, 100), Some(("Color".to_string(), "Red".to_string())));
    }

    // =========================================================================
    // UTF-16 encoding edge cases
    // =========================================================================

    #[test]
    fn test_char_col_to_utf16_with_bmp_chars() {
        // BMP characters take 1 UTF-16 code unit each
        let line = "hello\u{00E9}world"; // "helloéworld" -- é is U+00E9 (BMP)
        assert_eq!(char_col_to_utf16(line, 0), 0);
        assert_eq!(char_col_to_utf16(line, 5), 5); // before é
        assert_eq!(char_col_to_utf16(line, 6), 6); // after é
    }

    #[test]
    fn test_char_col_to_utf16_with_supplementary_chars() {
        // U+1F600 (grinning face) takes 2 UTF-16 code units
        let line = "ab\u{1F600}cd";
        assert_eq!(char_col_to_utf16(line, 0), 0);
        assert_eq!(char_col_to_utf16(line, 2), 2); // before emoji
        assert_eq!(char_col_to_utf16(line, 3), 4); // after emoji (2 UTF-16 units)
        assert_eq!(char_col_to_utf16(line, 4), 5); // 'c'
    }

    #[test]
    fn test_utf16_to_char_col_with_supplementary_chars() {
        let line = "ab\u{1F600}cd";
        assert_eq!(utf16_to_char_col(line, 0), 0);
        assert_eq!(utf16_to_char_col(line, 2), 2); // before emoji
        assert_eq!(utf16_to_char_col(line, 4), 3); // after emoji
        assert_eq!(utf16_to_char_col(line, 5), 4); // 'c' (corrected — 'd' is at char 4)
    }

    #[test]
    fn test_utf16_to_char_col_past_end() {
        let line = "abc";
        assert_eq!(utf16_to_char_col(line, 100), 3); // clamps to char count
    }

    #[test]
    fn test_char_col_to_utf16_empty() {
        assert_eq!(char_col_to_utf16("", 0), 0);
        assert_eq!(char_col_to_utf16("", 5), 0); // past end returns 0
    }

    // =========================================================================
    // to_lsp_diagnostic edge cases
    // =========================================================================

    #[test]
    fn test_to_lsp_diagnostic_line_0_col_0() {
        // line 0 / col 0 saturating_sub should not underflow
        let d = AstDiagnostic {
            line: 0,
            column: 0,
            message: "err".to_string(),
            severity: crate::eval::DiagnosticSeverity::Error,
            code: None,
            help: None,
            suggestion: None,
            source_file: None,
        };
        let lsp_d = to_lsp_diagnostic(&d);
        assert_eq!(lsp_d.range.start.line, 0);
        assert_eq!(lsp_d.range.start.character, 0);
    }

    #[test]
    fn test_to_lsp_diagnostic_with_help_and_suggestion() {
        let d = AstDiagnostic {
            line: 1,
            column: 1,
            message: "error".to_string(),
            severity: crate::eval::DiagnosticSeverity::Warning,
            code: Some("W100".to_string()),
            help: Some("try this".to_string()),
            suggestion: Some("fix it".to_string()),
            source_file: None,
        };
        let lsp_d = to_lsp_diagnostic(&d);
        assert!(lsp_d.message.contains("error"));
        assert!(lsp_d.message.contains("try this"));
        assert!(lsp_d.message.contains("fix it"));
        assert_eq!(lsp_d.severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn test_to_lsp_diagnostic_col_past_end_of_line() {
        let d = AstDiagnostic {
            line: 1,
            column: 100, // way past end
            message: "err".to_string(),
            severity: crate::eval::DiagnosticSeverity::Error,
            code: None,
            help: None,
            suggestion: None,
            source_file: None,
        };
        let source = "short";
        let lsp_d = to_lsp_diagnostic_with_source(&d, Some(source));
        // Should not panic, range should be clamped
        assert_eq!(lsp_d.range.start.line, 0);
    }

    #[test]
    fn test_to_lsp_diagnostic_line_past_end_of_source() {
        let d = AstDiagnostic {
            line: 100,
            column: 1,
            message: "err".to_string(),
            severity: crate::eval::DiagnosticSeverity::Error,
            code: None,
            help: None,
            suggestion: None,
            source_file: None,
        };
        let source = "let x = 5;";
        let lsp_d = to_lsp_diagnostic_with_source(&d, Some(source));
        // Line not found — falls back to col-based range
        assert_eq!(lsp_d.range.start.line, 99);
    }

    // =========================================================================
    // Diagnostic tags (#206)
    // =========================================================================

    #[test]
    fn test_unused_variable_has_unnecessary_tag() {
        let d = AstDiagnostic {
            line: 1, column: 1,
            message: "unused variable 'x'".to_string(),
            severity: crate::eval::DiagnosticSeverity::Warning,
            code: Some("W100".to_string()),
            help: None, suggestion: None,
            source_file: None,
        };
        let lsp_d = to_lsp_diagnostic(&d);
        assert_eq!(lsp_d.tags, Some(vec![DiagnosticTag::UNNECESSARY]));
    }

    #[test]
    fn test_unused_import_has_unnecessary_tag() {
        let d = AstDiagnostic {
            line: 1, column: 1,
            message: "unused import".to_string(),
            severity: crate::eval::DiagnosticSeverity::Warning,
            code: Some("W101".to_string()),
            help: None, suggestion: None,
            source_file: None,
        };
        let lsp_d = to_lsp_diagnostic(&d);
        assert_eq!(lsp_d.tags, Some(vec![DiagnosticTag::UNNECESSARY]));
    }

    #[test]
    fn test_dead_code_has_unnecessary_tag() {
        let d = AstDiagnostic {
            line: 1, column: 1,
            message: "dead code".to_string(),
            severity: crate::eval::DiagnosticSeverity::Warning,
            code: Some("W202".to_string()),
            help: None, suggestion: None,
            source_file: None,
        };
        let lsp_d = to_lsp_diagnostic(&d);
        assert_eq!(lsp_d.tags, Some(vec![DiagnosticTag::UNNECESSARY]));
    }

    #[test]
    fn test_reserved_keyword_has_deprecated_tag() {
        let d = AstDiagnostic {
            line: 1, column: 1,
            message: "reserved keyword".to_string(),
            severity: crate::eval::DiagnosticSeverity::Warning,
            code: Some("W111".to_string()),
            help: None, suggestion: None,
            source_file: None,
        };
        let lsp_d = to_lsp_diagnostic(&d);
        assert_eq!(lsp_d.tags, Some(vec![DiagnosticTag::DEPRECATED]));
    }

    #[test]
    fn test_normal_error_has_no_tags() {
        let d = AstDiagnostic {
            line: 1, column: 1,
            message: "type error".to_string(),
            severity: crate::eval::DiagnosticSeverity::Error,
            code: Some("E100".to_string()),
            help: None, suggestion: None,
            source_file: None,
        };
        let lsp_d = to_lsp_diagnostic(&d);
        assert_eq!(lsp_d.tags, None);
    }

    // =========================================================================
    // find_name_col edge cases
    // =========================================================================

    #[test]
    fn test_find_name_col_nonexistent_line() {
        let source = "let x = 5;";
        assert_eq!(find_name_col(source, 5, "x"), None);
    }

    #[test]
    fn test_find_name_col_name_not_found() {
        let source = "let x = 5;";
        assert_eq!(find_name_col(source, 1, "zzz"), None);
    }

    #[test]
    fn test_find_name_col_word_boundary() {
        // "ax" should not match as word "x"
        let source = "let ax = 5; let x = 10;";
        let result = find_name_col(source, 1, "x");
        // Should find standalone "x", not the "x" in "ax"
        assert!(result.is_some());
        let col = result.unwrap();
        assert_eq!(col, 17); // "let ax = 5; let " = 16 chars, then "x" at 17
    }

    // =========================================================================
    // analyze_document with various structures
    // =========================================================================

    #[test]
    fn test_analyze_document_with_type_alias() {
        let source = "type MyInt = int64;";
        let (state, _) = analyze_document(source);
        let var = state.variables.get("MyInt");
        assert!(var.is_some());
        assert!(var.unwrap().is_type_alias);
        assert_eq!(var.unwrap().type_annotation, Some("int64".to_string()));
    }

    #[test]
    fn test_analyze_document_with_const() {
        let source = "const PI = 3.14;";
        let (state, _) = analyze_document(source);
        let var = state.variables.get("PI");
        assert!(var.is_some());
        assert!(var.unwrap().constant);
    }

    #[test]
    fn test_analyze_document_with_let_mut() {
        let source = "let mut counter = 0;";
        let (state, _) = analyze_document(source);
        let var = state.variables.get("counter");
        assert!(var.is_some());
        assert!(var.unwrap().mutable);
    }

    #[test]
    fn test_analyze_document_with_async_fn() {
        let source = "async fn fetch() { null }";
        let (state, _) = analyze_document(source);
        let func = state.functions.get("fetch");
        assert!(func.is_some());
        assert!(func.unwrap().is_async);
    }

    #[test]
    fn test_analyze_document_with_use_statement() {
        let source = "use std::io;";
        let (state, _) = analyze_document(source);
        let var = state.variables.get("io");
        assert!(var.is_some());
        assert_eq!(var.unwrap().type_annotation, Some("import(std::io)".to_string()));
    }

    #[test]
    fn test_analyze_document_with_destructure() {
        let source = "let [a, b] = [1, 2];";
        let (state, _) = analyze_document(source);
        assert!(state.variables.contains_key("a"));
        assert!(state.variables.contains_key("b"));
    }

    // ── Deduplication tests ──

    #[test]
    fn test_no_duplicate_shadowing_warnings() {
        // Both type checker (W102) and linter (W209) used to warn about shadowing.
        // After cleanup, only the linter should emit W209.
        let source = "let x = 1;\nlet x = 2;\noutput x;";
        let (_, diagnostics) = analyze_document(source);
        let shadow_diags: Vec<_> = diagnostics.iter()
            .filter(|d| d.message.contains("shadow"))
            .collect();
        assert_eq!(shadow_diags.len(), 1,
            "expected exactly 1 shadowing diagnostic (W209), got {}: {:?}",
            shadow_diags.len(), shadow_diags);
        assert_eq!(shadow_diags[0].code.as_deref(), Some("W209"));
    }

    #[test]
    fn test_no_duplicate_empty_block_warnings() {
        // Both type checker (W104) and linter (W206) used to warn about empty blocks.
        // After cleanup, only the linter should emit W206.
        let source = "for _x in [1, 2, 3] {}";
        let (_, diagnostics) = analyze_document(source);
        let empty_diags: Vec<_> = diagnostics.iter()
            .filter(|d| d.message.to_lowercase().contains("empty"))
            .collect();
        assert_eq!(empty_diags.len(), 1,
            "expected exactly 1 empty block diagnostic (W206), got {}: {:?}",
            empty_diags.len(), empty_diags);
        assert_eq!(empty_diags[0].code.as_deref(), Some("W206"));
    }

    #[test]
    fn test_no_duplicate_unused_param_warnings() {
        // Both type checker (W109) and linter (W211) used to warn about unused params.
        // After cleanup, only the type checker should emit W109.
        let source = "fn foo(x, y) { output x; }";
        let (_, diagnostics) = analyze_document(source);
        let unused_param_diags: Vec<_> = diagnostics.iter()
            .filter(|d| d.message.to_lowercase().contains("unused") && d.message.to_lowercase().contains("parameter"))
            .collect();
        assert_eq!(unused_param_diags.len(), 1,
            "expected exactly 1 unused param diagnostic (W109), got {}: {:?}",
            unused_param_diags.len(), unused_param_diags);
        assert_eq!(unused_param_diags[0].code.as_deref(), Some("W109"));
    }

    #[test]
    fn test_no_duplicate_while_true_warnings() {
        // Both type checker (W105) and linter (W204) used to warn about while true.
        // After cleanup, only the linter should emit W204.
        let source = "while true { output 1; }";
        let (_, diagnostics) = analyze_document(source);
        let const_cond_diags: Vec<_> = diagnostics.iter()
            .filter(|d| d.message.contains("always") && d.code.as_deref().is_some_and(|c| c == "W204"))
            .collect();
        assert_eq!(const_cond_diags.len(), 1,
            "expected exactly 1 constant condition diagnostic (W204), got {}: {:?}",
            const_cond_diags.len(), const_cond_diags);
        assert_eq!(const_cond_diags[0].code.as_deref(), Some("W204"));
    }

    #[test]
    fn test_deduplicate_diagnostics_function() {
        let mut diags = vec![
            AstDiagnostic {
                line: 1, column: 1,
                message: "unused variable".to_string(),
                severity: crate::eval::DiagnosticSeverity::Warning,
                code: Some("W100".to_string()),
                help: None, suggestion: None,
                source_file: None,
            },
            AstDiagnostic {
                line: 1, column: 1,
                message: "unused variable".to_string(),
                severity: crate::eval::DiagnosticSeverity::Warning,
                code: Some("W100".to_string()),
                help: None, suggestion: None,
                source_file: None,
            },
            AstDiagnostic {
                line: 1, column: 1,
                message: "different warning".to_string(),
                severity: crate::eval::DiagnosticSeverity::Warning,
                code: Some("W200".to_string()),
                help: None, suggestion: None,
                source_file: None,
            },
            AstDiagnostic {
                line: 2, column: 1,
                message: "different location".to_string(),
                severity: crate::eval::DiagnosticSeverity::Warning,
                code: Some("W100".to_string()),
                help: None, suggestion: None,
                source_file: None,
            },
        ];
        deduplicate_diagnostics(&mut diags);
        assert_eq!(diags.len(), 3, "expected 3 after dedup, got {}", diags.len());
        assert_eq!(diags[0].message, "unused variable");
        assert_eq!(diags[1].message, "different warning");
        assert_eq!(diags[2].message, "different location");
    }
}
