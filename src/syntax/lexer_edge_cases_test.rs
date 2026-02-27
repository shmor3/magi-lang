// Test: .5 (leading dot)
fn test_leading_dot() {
    use crate::syntax::lexer::*;
    let result = tokenize(".5");
    match result {
        Ok(tokens) => {
            for (i, t) in tokens.iter().enumerate() {
                println!("Token {}: {:?} text='{}'", i, t.kind, t.text);
            }
        },
        Err(e) => println!("Error: {}", e.message),
    }
}

// Test: 5. (trailing dot)
fn test_trailing_dot() {
    use crate::syntax::lexer::*;
    let result = tokenize("5.");
    match result {
        Ok(tokens) => {
            for (i, t) in tokens.iter().enumerate() {
                println!("Token {}: {:?} text='{}'", i, t.kind, t.text);
            }
        },
        Err(e) => println!("Error: {}", e.message),
    }
}

// Test: 1E10 (uppercase exponent)
fn test_uppercase_exp() {
    use crate::syntax::lexer::*;
    let tokens = tokenize("1E10").unwrap();
    println!("1E10: kind={:?}, text={}", tokens[0].kind, tokens[0].text);
}

// Test: 00 (leading zeros)
fn test_leading_zeros() {
    use crate::syntax::lexer::*;
    let tokens = tokenize("00").unwrap();
    println!("00: kind={:?}, text={}", tokens[0].kind, tokens[0].text);
}

// Test: \u{110000} invalid unicode
fn test_invalid_unicode() {
    use crate::syntax::lexer::*;
    let result = tokenize(r#""\u{110000}""#);
    println!("Invalid unicode \\u{{110000}}: {:?}", result);
}

// Test: >> 
fn test_double_greater() {
    use crate::syntax::lexer::*;
    let tokens = tokenize(">>").unwrap();
    println!(">>: {} tokens", tokens.len());
    for (i, t) in tokens.iter().enumerate() {
        println!("  Token {}: {:?}", i, t.kind);
    }
}

// Test: \r\n
fn test_crlf() {
    use crate::syntax::lexer::*;
    let tokens = tokenize("let x\r\nlet y").unwrap();
    println!("CRLF: {} tokens", tokens.len());
    println!("Token 0: line={}, col={}", tokens[0].span.start_line, tokens[0].span.start_col);
    println!("Token 2: line={}, col={}", tokens[2].span.start_line, tokens[2].span.start_col);
}

// Test: null byte
fn test_null_byte() {
    use crate::syntax::lexer::*;
    let source = "let\0x";
    let result = tokenize(source);
    println!("Null byte: {:?}", result);
}

fn main() {
    println!("=== Testing .5 ===");
    test_leading_dot();
    println!("\n=== Testing 5. ===");
    test_trailing_dot();
    println!("\n=== Testing 1E10 ===");
    test_uppercase_exp();
    println!("\n=== Testing 00 ===");
    test_leading_zeros();
    println!("\n=== Testing \\u{{110000}} ===");
    test_invalid_unicode();
    println!("\n=== Testing >> ===");
    test_double_greater();
    println!("\n=== Testing CRLF ===");
    test_crlf();
    println!("\n=== Testing null byte ===");
    test_null_byte();
}
