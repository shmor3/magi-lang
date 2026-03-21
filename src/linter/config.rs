//! Lint configuration file support.
//!
//! Reads a `.magi-lint.toml` or `magi-lint.toml` configuration file to
//! control which lint rules are enabled/disabled.

use std::path::Path;

/// Load lint configuration from a TOML file.
///
/// Searches for `.magi-lint.toml` or `magi-lint.toml` in the given directory
/// and its parents. Returns the disabled rules list.
pub fn load_lint_config(dir: &Path) -> Vec<String> {
    let mut current = Some(dir);
    while let Some(d) = current {
        for name in &[".magi-lint.toml", "magi-lint.toml"] {
            let path = d.join(name);
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    return parse_lint_config(&content);
                }
            }
        }
        current = d.parent();
    }
    Vec::new()
}

/// Parse a lint config TOML string and extract disabled rules.
///
/// Expected format:
/// ```toml
/// [lint]
/// disabled = ["W200", "W201"]
/// ```
fn parse_lint_config(content: &str) -> Vec<String> {
    let table = match crate::util::toml_parse(content) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let lint = match table.get("lint").and_then(|v| v.as_table()) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let disabled = match lint.get("disabled").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    disabled
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_config() {
        assert!(parse_lint_config("").is_empty());
    }

    #[test]
    fn test_parse_no_lint_section() {
        assert!(parse_lint_config("[other]\nfoo = 1").is_empty());
    }

    #[test]
    fn test_parse_disabled_rules() {
        let config = r#"
[lint]
disabled = ["W200", "W201", "W206"]
"#;
        let rules = parse_lint_config(config);
        assert_eq!(rules, vec!["W200", "W201", "W206"]);
    }

    #[test]
    fn test_parse_empty_disabled() {
        let config = r#"
[lint]
disabled = []
"#;
        assert!(parse_lint_config(config).is_empty());
    }

    #[test]
    fn test_parse_invalid_toml() {
        assert!(parse_lint_config("not valid toml {{{").is_empty());
    }
}
