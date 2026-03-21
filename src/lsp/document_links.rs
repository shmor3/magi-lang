//! Document link provider for the MAGI LSP.
//!
//! For `use pkg::name` statements, resolves the path to a file and returns
//! it as a clickable link in the editor.

use super::analysis::{char_col_to_utf16, DocumentState};
use crate::syntax::ast::*;
use tower_lsp::lsp_types::*;

/// Handle a document link request.
///
/// Walks the AST looking for `use` statements and returns clickable links
/// for those that resolve to files on disk.
pub fn handle_document_links(
    state: &DocumentState,
    uri: &Url,
) -> Vec<DocumentLink> {
    let program = match &state.program {
        Some(p) => p,
        None => return Vec::new(),
    };

    let mut links = Vec::new();

    // Resolve the workspace root from the document URI
    let doc_dir = uri.to_file_path().ok().and_then(|p| {
        p.parent().map(|d| d.to_path_buf())
    });

    for stmt in &program.statements {
        if let StatementKind::Use { path, glob, .. } = &stmt.kind {
            if path.is_empty() {
                continue;
            }

            // Skip std library imports (they don't resolve to files)
            if path.first().map(|s| s.as_str()) == Some("std") {
                continue;
            }

            // Try to resolve the path to a file
            let resolved = if let Some(ref dir) = doc_dir {
                resolve_use_path(dir, path)
            } else {
                None
            };

            if let Some(file_path) = resolved {
                let target_uri = Url::from_file_path(&file_path).ok();
                if let Some(target) = target_uri {
                    // Build the range covering the use path in source
                    let lsp_line = stmt.span.start_line.saturating_sub(1);
                    let line_text = state.source.lines().nth(lsp_line as usize).unwrap_or("");

                    // Find the path portion in the line
                    let path_str = if *glob {
                        format!("{}::*", path.join("::"))
                    } else {
                        path.join("::")
                    };

                    if let Some(byte_start) = line_text.find(&path_str) {
                        let char_start = line_text[..byte_start].chars().count() as u32;
                        let char_end = char_start + path_str.chars().count() as u32;
                        let start_utf16 = char_col_to_utf16(line_text, char_start);
                        let end_utf16 = char_col_to_utf16(line_text, char_end);

                        links.push(DocumentLink {
                            range: Range {
                                start: Position {
                                    line: lsp_line,
                                    character: start_utf16,
                                },
                                end: Position {
                                    line: lsp_line,
                                    character: end_utf16,
                                },
                            },
                            target: Some(target),
                            tooltip: Some(format!("Open {}", file_path.display())),
                            data: None,
                        });
                    }
                }
            }
        }
    }

    links
}

/// Try to resolve a `use` path to an actual file.
///
/// Tries several patterns:
/// - `<dir>/<path>.magi` (e.g., `use foo::bar` -> `foo/bar.magi`)
/// - `<dir>/<path>/mod.magi` (e.g., `use foo::bar` -> `foo/bar/mod.magi`)
/// - `<dir>/<first_segment>.magi` (e.g., `use mylib::func` -> `mylib.magi`)
fn resolve_use_path(dir: &std::path::Path, path: &[String]) -> Option<std::path::PathBuf> {
    if path.is_empty() {
        return None;
    }

    // Convert path segments to a file path: foo::bar -> foo/bar
    let relative: std::path::PathBuf = path.iter().collect();

    // Try <dir>/<path>.magi
    let candidate = dir.join(&relative).with_extension("magi");
    if candidate.is_file() {
        return Some(candidate);
    }

    // Try <dir>/<path>/mod.magi
    let candidate = dir.join(&relative).join("mod.magi");
    if candidate.is_file() {
        return Some(candidate);
    }

    // Try just the first segment: <dir>/<first>.magi
    if path.len() > 1 {
        let candidate = dir.join(&path[0]).with_extension("magi");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze_document;

    fn test_uri() -> Url {
        Url::parse("file:///test/main.magi").unwrap()
    }

    #[test]
    fn test_no_links_for_std_imports() {
        let source = "use std::math::*;";
        let (state, _) = analyze_document(source);
        let links = handle_document_links(&state, &test_uri());
        assert!(links.is_empty(), "std imports should not produce document links");
    }

    #[test]
    fn test_no_links_without_program() {
        let state = DocumentState {
            source: String::new(),
            program: None,
            functions: std::collections::HashMap::new(),
            variables: std::collections::HashMap::new(),
            enums: std::collections::HashMap::new(),
            structs: std::collections::HashMap::new(),
        };
        let links = handle_document_links(&state, &test_uri());
        assert!(links.is_empty());
    }

    #[test]
    fn test_no_links_for_nonexistent_files() {
        let source = "use nonexistent::module;";
        let (state, _) = analyze_document(source);
        let links = handle_document_links(&state, &test_uri());
        // File doesn't exist, so no link
        assert!(links.is_empty());
    }

    #[test]
    fn test_resolve_use_path_none_for_empty() {
        let dir = std::path::Path::new("/tmp");
        assert!(resolve_use_path(dir, &[]).is_none());
    }

    #[test]
    fn test_resolve_use_path_nonexistent() {
        let dir = std::path::Path::new("/tmp/nonexistent_magi_dir");
        let path = vec!["foo".to_string(), "bar".to_string()];
        assert!(resolve_use_path(dir, &path).is_none());
    }
}
