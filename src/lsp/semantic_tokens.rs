//! Semantic token provider for the MAGI LSP.
//!
//! Provides semantic highlighting for keywords, functions, variables, types,
//! strings, numbers, operators, and comments.

use super::analysis::DocumentState;
use tower_lsp::lsp_types::*;

/// Token types used in semantic highlighting.
pub const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::KEYWORD,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::TYPE,
    SemanticTokenType::STRING,
    SemanticTokenType::NUMBER,
    SemanticTokenType::OPERATOR,
    SemanticTokenType::COMMENT,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::ENUM_MEMBER,
    SemanticTokenType::STRUCT,
    SemanticTokenType::ENUM,
];

/// Token modifiers for semantic highlighting.
pub const TOKEN_MODIFIERS: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION,
    SemanticTokenModifier::DEFINITION,
    SemanticTokenModifier::READONLY,
    SemanticTokenModifier::STATIC,
    SemanticTokenModifier::DEFAULT_LIBRARY,
];

const TT_KEYWORD: u32 = 0;
const TT_FUNCTION: u32 = 1;
const TT_VARIABLE: u32 = 2;
const TT_TYPE: u32 = 3;
const TT_STRING: u32 = 4;
const TT_NUMBER: u32 = 5;
const TT_OPERATOR: u32 = 6;
const TT_COMMENT: u32 = 7;
#[allow(dead_code)]
const TT_PARAMETER: u32 = 8;
const TT_PROPERTY: u32 = 9;
const TT_ENUM_MEMBER: u32 = 10;
const TT_STRUCT: u32 = 11;
const TT_ENUM: u32 = 12;

const MOD_DECLARATION: u32 = 1 << 0;
const MOD_DEFINITION: u32 = 1 << 1;
const MOD_READONLY: u32 = 1 << 2;

/// MAGI keywords for semantic highlighting.
const KEYWORDS: &[&str] = &[
    "let", "mut", "fn", "async", "if", "else", "for", "while", "loop", "match",
    "return", "break", "continue", "throw", "try", "catch", "finally", "output",
    "import", "use", "const", "type", "mod", "enum", "struct", "test", "true",
    "false", "null", "in", "as", "spawn", "await", "pub", "do",
];

/// Provide full semantic tokens for a document.
pub fn handle_semantic_tokens_full(state: &DocumentState) -> SemanticTokensResult {
    let tokens = tokenize_source(&state.source, state);
    SemanticTokensResult::Tokens(SemanticTokens {
        result_id: None,
        data: tokens,
    })
}

/// Simple lexer-based semantic token generation.
/// Walks the source character by character, classifying tokens.
fn tokenize_source(source: &str, state: &DocumentState) -> Vec<SemanticToken> {
    let mut raw_tokens: Vec<(u32, u32, u32, u32, u32)> = Vec::new(); // (line, col, len, type, mods)
    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line) in lines.iter().enumerate() {
        let line_num = line_idx as u32;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let ch = chars[i];

            // Skip whitespace
            if ch.is_whitespace() {
                i += 1;
                continue;
            }

            // Line comment
            if ch == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
                let start = i as u32;
                let len = (chars.len() - i) as u32;
                raw_tokens.push((line_num, start, len, TT_COMMENT, 0));
                break;
            }

            // String literal
            if ch == '"' || (ch == 'f' && i + 1 < chars.len() && chars[i + 1] == '"')
                || (ch == 'r' && i + 1 < chars.len() && chars[i + 1] == '"')
            {
                let start = i as u32;
                if ch == 'f' || ch == 'r' {
                    i += 1;
                }
                i += 1; // skip opening quote
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        i += 1; // skip escaped char
                    }
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // skip closing quote
                }
                let len = i as u32 - start;
                raw_tokens.push((line_num, start, len, TT_STRING, 0));
                continue;
            }

            // Triple-quoted string
            if ch == '"' && i + 2 < chars.len() && chars[i + 1] == '"' && chars[i + 2] == '"' {
                let start = i as u32;
                i += 3;
                loop {
                    if i + 2 < chars.len()
                        && chars[i] == '"'
                        && chars[i + 1] == '"'
                        && chars[i + 2] == '"'
                    {
                        i += 3;
                        break;
                    }
                    i += 1;
                    if i >= chars.len() {
                        break;
                    }
                }
                let len = i as u32 - start;
                raw_tokens.push((line_num, start, len, TT_STRING, 0));
                continue;
            }

            // Number literal
            if ch.is_ascii_digit()
                || (ch == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
            {
                let start = i as u32;
                // Handle 0x, 0o, 0b prefixes
                if ch == '0' && i + 1 < chars.len() {
                    let next = chars[i + 1];
                    if next == 'x' || next == 'o' || next == 'b' || next == 'X' || next == 'O' || next == 'B' {
                        i += 2;
                    }
                }
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
                {
                    i += 1;
                }
                let len = i as u32 - start;
                raw_tokens.push((line_num, start, len, TT_NUMBER, 0));
                continue;
            }

            // Identifier or keyword
            if ch.is_alphabetic() || ch == '_' {
                let start = i as u32;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start as usize..i].iter().collect();
                let len = i as u32 - start;

                // Check context: preceded by `.` → property, preceded by `::` → enum member
                let preceded_by_dot = start > 0 && chars[start as usize - 1] == '.';
                let preceded_by_colon_colon = start >= 2
                    && chars[start as usize - 1] == ':'
                    && chars[start as usize - 2] == ':';

                if preceded_by_dot {
                    // Check if followed by ( -> method call
                    let mut j = i;
                    while j < chars.len() && chars[j].is_whitespace() { j += 1; }
                    if j < chars.len() && chars[j] == '(' {
                        raw_tokens.push((line_num, start, len, TT_FUNCTION, 0));
                    } else {
                        raw_tokens.push((line_num, start, len, TT_PROPERTY, 0));
                    }
                } else if preceded_by_colon_colon {
                    raw_tokens.push((line_num, start, len, TT_ENUM_MEMBER, 0));
                } else if KEYWORDS.contains(&word.as_str()) {
                    raw_tokens.push((line_num, start, len, TT_KEYWORD, 0));
                } else if state.functions.contains_key(&word) {
                    raw_tokens.push((line_num, start, len, TT_FUNCTION, MOD_DECLARATION | MOD_DEFINITION));
                } else if state.structs.contains_key(&word) {
                    raw_tokens.push((line_num, start, len, TT_STRUCT, MOD_DECLARATION));
                } else if state.enums.contains_key(&word) {
                    raw_tokens.push((line_num, start, len, TT_ENUM, MOD_DECLARATION));
                } else if let Some(var) = state.variables.get(&word) {
                    let mods = if var.constant { MOD_READONLY } else { 0 };
                    if var.is_type_alias {
                        raw_tokens.push((line_num, start, len, TT_TYPE, mods));
                    } else {
                        raw_tokens.push((line_num, start, len, TT_VARIABLE, mods));
                    }
                } else if is_type_name(&word) {
                    raw_tokens.push((line_num, start, len, TT_TYPE, 0));
                } else {
                    // Check if followed by ( -> function call
                    let mut j = i;
                    while j < chars.len() && chars[j].is_whitespace() {
                        j += 1;
                    }
                    if j < chars.len() && chars[j] == '(' {
                        raw_tokens.push((line_num, start, len, TT_FUNCTION, 0));
                    } else {
                        raw_tokens.push((line_num, start, len, TT_VARIABLE, 0));
                    }
                }
                continue;
            }

            // Operators (multi-char)
            if is_operator_start(ch) {
                let start = i as u32;
                i += 1;
                // consume additional operator chars
                while i < chars.len() && is_operator_continuation(chars[i]) {
                    i += 1;
                }
                let len = i as u32 - start;
                raw_tokens.push((line_num, start, len, TT_OPERATOR, 0));
                continue;
            }

            // Skip other characters (braces, parens, etc.)
            i += 1;
        }
    }

    // Convert absolute positions to LSP delta encoding
    encode_tokens(&raw_tokens)
}

fn is_type_name(name: &str) -> bool {
    matches!(
        name,
        "int32" | "int64" | "uint32" | "uint64" | "float32" | "float64"
            | "string" | "bool" | "null" | "any" | "Array" | "Map"
            | "Bytes" | "Int32" | "Int64" | "Uint32" | "Uint64"
            | "Float32" | "Float64" | "String" | "Bool"
    )
}

fn is_operator_start(ch: char) -> bool {
    matches!(
        ch,
        '+' | '-' | '*' | '/' | '%' | '=' | '!' | '<' | '>' | '&' | '|' | '^' | '~' | '?'
    )
}

fn is_operator_continuation(ch: char) -> bool {
    matches!(ch, '=' | '>' | '|' | '&' | '?' | '.')
}

/// Encode raw tokens into LSP delta-encoded SemanticTokens.
fn encode_tokens(raw: &[(u32, u32, u32, u32, u32)]) -> Vec<SemanticToken> {
    let mut result = Vec::with_capacity(raw.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;

    for &(line, start, length, token_type, token_modifiers) in raw {
        let delta_line = line - prev_line;
        let delta_start = if delta_line == 0 {
            start - prev_start
        } else {
            start
        };

        result.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type,
            token_modifiers_bitset: token_modifiers,
        });

        prev_line = line;
        prev_start = start;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze_document;

    #[test]
    fn test_semantic_tokens_keywords() {
        let source = "let x = 5";
        let (state, _) = analyze_document(source);
        let result = handle_semantic_tokens_full(&state);
        let SemanticTokensResult::Tokens(tokens) = result else {
            panic!("expected full tokens");
        };
        assert!(!tokens.data.is_empty(), "should have tokens");
        // First token should be 'let' keyword
        assert_eq!(tokens.data[0].token_type, TT_KEYWORD);
        assert_eq!(tokens.data[0].length, 3);
    }

    #[test]
    fn test_semantic_tokens_comments() {
        let source = "// comment\nlet x = 1";
        let (state, _) = analyze_document(source);
        let result = handle_semantic_tokens_full(&state);
        let SemanticTokensResult::Tokens(tokens) = result else {
            panic!("expected full tokens");
        };
        // First token is the comment
        assert_eq!(tokens.data[0].token_type, TT_COMMENT);
    }

    #[test]
    fn test_semantic_tokens_strings() {
        let source = r#"let s = "hello""#;
        let (state, _) = analyze_document(source);
        let result = handle_semantic_tokens_full(&state);
        let SemanticTokensResult::Tokens(tokens) = result else {
            panic!("expected full tokens");
        };
        let has_string = tokens.data.iter().any(|t| t.token_type == TT_STRING);
        assert!(has_string, "should have string token");
    }

    #[test]
    fn test_semantic_tokens_numbers() {
        let source = "let n = 42";
        let (state, _) = analyze_document(source);
        let result = handle_semantic_tokens_full(&state);
        let SemanticTokensResult::Tokens(tokens) = result else {
            panic!("expected full tokens");
        };
        let has_number = tokens.data.iter().any(|t| t.token_type == TT_NUMBER);
        assert!(has_number, "should have number token");
    }

    #[test]
    fn test_semantic_tokens_function_def() {
        let source = "fn greet() { null }";
        let (state, _) = analyze_document(source);
        let result = handle_semantic_tokens_full(&state);
        let SemanticTokensResult::Tokens(tokens) = result else {
            panic!("expected full tokens");
        };
        let has_fn = tokens.data.iter().any(|t| t.token_type == TT_FUNCTION);
        assert!(has_fn, "should have function token");
    }

    #[test]
    fn test_semantic_tokens_enum() {
        let source = "enum Color { Red, Green }";
        let (state, _) = analyze_document(source);
        let result = handle_semantic_tokens_full(&state);
        let SemanticTokensResult::Tokens(tokens) = result else {
            panic!("expected full tokens");
        };
        let has_enum = tokens.data.iter().any(|t| t.token_type == TT_ENUM);
        assert!(has_enum, "should have enum token");
    }
}
