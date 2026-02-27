#[cfg(test)]
mod fstring_bug_tests {
    use magi_lang::syntax::lexer::tokenize;

    #[test]
    fn test_fstring_with_escaped_closing_brace() {
        // f"{\}" should either:
        // 1. Succeed and parse as f-string with an incomplete expression
        // 2. Fail with a clear error about unmatched braces
        // It should NOT try to read past the closing quote
        let result = tokenize(r#"f"{\}""#);
        match result {
            Ok(tokens) => {
                println!("Tokens:");
                for (i, t) in tokens.iter().enumerate() {
                    println!("  {}: {:?} text='{}'", i, t.kind, t.text);
                }
            }
            Err(e) => {
                println!("Error: {} at line {} col {}", e.message, e.line, e.column);
            }
        }
    }

    #[test]
    fn test_fstring_with_escaped_opening_brace() {
        let result = tokenize(r#"f"\{test}""#);
        match result {
            Ok(tokens) => {
                println!("Tokens:");
                for (i, t) in tokens.iter().enumerate() {
                    println!("  {}: {:?} text='{}'", i, t.kind, t.text);
                }
            }
            Err(e) => {
                println!("Error: {} at line {} col {}", e.message, e.line, e.column);
            }
        }
    }
}
