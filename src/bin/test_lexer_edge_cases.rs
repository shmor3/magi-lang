use magi_lang::syntax::lexer::tokenize;

fn main() {
    println!("=== Test: .5 (leading dot) ===");
    let result = tokenize(".5");
    match result {
        Ok(tokens) => {
            println!("Success: {} tokens", tokens.len());
            for (i, t) in tokens.iter().enumerate() {
                println!("  Token {}: {:?} text='{}'", i, t.kind, t.text);
            }
        },
        Err(e) => println!("Error: {}", e.message),
    }

    println!("\n=== Test: 5. (trailing dot) ===");
    let result = tokenize("5.");
    match result {
        Ok(tokens) => {
            println!("Success: {} tokens", tokens.len());
            for (i, t) in tokens.iter().enumerate() {
                println!("  Token {}: {:?} text='{}'", i, t.kind, t.text);
            }
        },
        Err(e) => println!("Error: {}", e.message),
    }

    println!("\n=== Test: 1E10 (uppercase exponent) ===");
    let tokens = tokenize("1E10").unwrap();
    println!("1E10: kind={:?}, text={}", tokens[0].kind, tokens[0].text);

    println!("\n=== Test: 00 (leading zeros) ===");
    let tokens = tokenize("00").unwrap();
    println!("00: kind={:?}, text={}", tokens[0].kind, tokens[0].text);

    println!("\n=== Test: \\u{{110000}} invalid unicode ===");
    let result = tokenize(r#""\u{110000}""#);
    println!("Invalid unicode: {:?}", result);

    println!("\n=== Test: >> (double greater) ===");
    let tokens = tokenize(">>").unwrap();
    println!(">>: {} tokens", tokens.len());
    for (i, t) in tokens.iter().enumerate() {
        println!("  Token {}: {:?}", i, t.kind);
    }

    println!("\n=== Test: CRLF ===");
    let tokens = tokenize("let x\r\nlet y").unwrap();
    println!("CRLF: {} tokens", tokens.len());
    println!("Token 0 (let): line={}, col={}", tokens[0].span.start_line, tokens[0].span.start_col);
    if tokens.len() > 3 {
        println!("Token 3 (let on line 2): line={}, col={}", tokens[3].span.start_line, tokens[3].span.start_col);
    }

    println!("\n=== Test: Empty unicode escape ===");
    let result = tokenize(r#""\u{}""#);
    println!("Empty unicode escape: {:?}", result);

    println!("\n=== Test: Multi-byte UTF-8 in identifier ===");
    let tokens = tokenize("αβγ").unwrap();
    println!("αβγ: {} tokens, kind={:?}", tokens.len(), tokens[0].kind);

    println!("\n=== Test: Multi-byte UTF-8 in string ===");
    let tokens = tokenize(r#""hello 世界""#).unwrap();
    println!("String with multi-byte: text='{}'", tokens[0].text);

    println!("\n=== Test: f-string with escaped braces ===");
    let tokens = tokenize(r#"f"test\{value}""#).unwrap();
    println!("f-string escaped braces: kind={:?}, text='{}'", tokens[0].kind, tokens[0].text);

    println!("\n=== Test: Nested block comments ===");
    let tokens = tokenize("let /* outer /* inner */ end */ x").unwrap();
    println!("Nested comments: {} tokens", tokens.len());

    println!("\n=== Test: Very long float (scientific notation) ===");
    let tokens = tokenize("1e308").unwrap();
    println!("1e308: kind={:?}, text={}", tokens[0].kind, tokens[0].text);

    println!("\n=== Test: Hex escape with invalid digits ===");
    let result = tokenize(r#""test\xGH""#);
    println!("Hex escape invalid: {:?}", result);

    println!("\n=== Test: Incomplete hex escape EOF ===");
    let result = tokenize(r#""test\x""#);
    println!("Incomplete hex escape: {:?}", result);
}
