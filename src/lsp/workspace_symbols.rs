//! Workspace symbol provider for the MAGI LSP.
//!
//! Scans all `.magi` files in the workspace root, parses each, and collects
//! top-level function, struct, and enum definitions.

use crate::lsp::analysis::{analyze_document, char_col_to_utf16};
use super::types::*;

/// Handle a workspace symbol request.
///
/// Walks all `.magi` files under `workspace_root`, parses each, and returns
/// top-level function, struct, and enum symbols that match the query filter.
#[allow(deprecated)] // SymbolInformation::deprecated field
pub fn handle_workspace_symbols(
    workspace_root: &str,
    query: &str,
) -> Vec<SymbolInformation> {
    let mut symbols = Vec::new();

    let root = std::path::Path::new(workspace_root);
    if !root.is_dir() {
        return symbols;
    }

    let query_lower = query.to_lowercase();

    collect_magi_files(root, &query_lower, &mut symbols);

    // Sort by file path then position for deterministic output.
    symbols.sort_by(|a, b| {
        a.location
            .uri
            .as_str()
            .cmp(b.location.uri.as_str())
            .then_with(|| {
                a.location
                    .range
                    .start
                    .line
                    .cmp(&b.location.range.start.line)
            })
    });

    symbols
}

/// Recursively find `.magi` files and extract symbols.
#[allow(deprecated)]
fn collect_magi_files(
    dir: &std::path::Path,
    query_lower: &str,
    symbols: &mut Vec<SymbolInformation>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // Skip hidden directories and common non-source directories
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist"
            {
                continue;
            }
        }

        if path.is_dir() {
            collect_magi_files(&path, query_lower, symbols);
        } else if path.extension().and_then(|e| e.to_str()) == Some("magi") {
            let source = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let uri = match Url::from_file_path(&path) {
                Ok(u) => u,
                Err(_) => continue,
            };

            let (state, _) = analyze_document(&source);

            for func in state.functions.values() {
                // Skip internal test symbols (registered as "test:name")
                if func.name.starts_with("test \"") {
                    continue;
                }
                if !query_lower.is_empty() && !func.name.to_lowercase().contains(query_lower) {
                    continue;
                }
                let range =
                    make_range(&state.source, func.line, func.col, func.name.chars().count());
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

            for st in state.structs.values() {
                if !query_lower.is_empty() && !st.name.to_lowercase().contains(query_lower) {
                    continue;
                }
                let range =
                    make_range(&state.source, st.line, st.col, st.name.chars().count());
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
            }

            for en in state.enums.values() {
                if !query_lower.is_empty() && !en.name.to_lowercase().contains(query_lower) {
                    continue;
                }
                let range =
                    make_range(&state.source, en.line, en.col, en.name.chars().count());
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
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Create a unique temporary directory for each test.
    fn make_temp_dir() -> std::path::PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("magi_ws_test_{}_{}", pid, id));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setup_workspace(files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = make_temp_dir();
        for (name, content) in files {
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, content).unwrap();
        }
        dir
    }

    #[test]
    fn test_workspace_symbols_functions() {
        let dir = setup_workspace(&[
            ("main.magi", "fn hello() { null }\nfn world() { null }"),
            ("lib.magi", "fn helper() { null }"),
        ]);
        let symbols = handle_workspace_symbols(dir.to_str().unwrap(), "");
        let fn_names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(fn_names.contains(&"hello"));
        assert!(fn_names.contains(&"world"));
        assert!(fn_names.contains(&"helper"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_workspace_symbols_structs_and_enums() {
        let dir = setup_workspace(&[
            ("types.magi", "struct Point { x: float64, y: float64 }\nenum Color { Red, Green, Blue }"),
        ]);
        let symbols = handle_workspace_symbols(dir.to_str().unwrap(), "");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Point"));
        assert!(names.contains(&"Color"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_workspace_symbols_query_filter() {
        let dir = setup_workspace(&[
            ("main.magi", "fn hello() { null }\nfn world() { null }"),
        ]);
        let symbols = handle_workspace_symbols(dir.to_str().unwrap(), "hel");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_workspace_symbols_empty_workspace() {
        let dir = setup_workspace(&[]);
        let symbols = handle_workspace_symbols(dir.to_str().unwrap(), "");
        assert!(symbols.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_workspace_symbols_invalid_root() {
        let symbols = handle_workspace_symbols("/nonexistent/path/magi_ws_test_xxx", "");
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_workspace_symbols_nested_files() {
        let dir = setup_workspace(&[
            ("src/lib.magi", "fn nested_fn() { null }"),
        ]);
        let symbols = handle_workspace_symbols(dir.to_str().unwrap(), "");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"nested_fn"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_workspace_symbols_case_insensitive_query() {
        let dir = setup_workspace(&[
            ("main.magi", "fn MyFunc() { null }"),
        ]);
        let symbols = handle_workspace_symbols(dir.to_str().unwrap(), "myfunc");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "MyFunc");
        let _ = fs::remove_dir_all(&dir);
    }
}
