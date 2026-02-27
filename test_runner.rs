use magi_lang::syntax::lexer::tokenize;

fn main() {
    println!("=== Test: f-string with \\}} ===");
    let result = tokenize(r#"f"{\}""#);
    match result {
        Ok(tokens) => {
            println!("OK: {} tokens", tokens.len());
            for (i, t) in tokens.iter().enumerate() {
                println!("  {}: {:?} text='{}'", i, t.kind, t.text);
            }
        }
        Err(e) => {
            println!("ERROR: {} at line {} col {}", e.message, e.line, e.column);
        }
    }

    println!("\n=== Test: f-string with \\{{ ===");
    let result = tokenize(r#"f"\{test}""#);
    match result {
        Ok(tokens) => {
            println!("OK: {} tokens", tokens.len());
            for (i, t) in tokens.iter().enumerate() {
                println!("  {}: {:?} text='{}'", i, t.kind, t.text);
            }
        }
        Err(e) => {
            println!("ERROR: {} at line {} col {}", e.message, e.line, e.column);
        }
    }

    println!("\n=== Test: f-string simple ===");
    let result = tokenize(r#"f"hello {x}""#);
    match result {
        Ok(tokens) => {
            println!("OK: {} tokens", tokens.len());
            for (i, t) in tokens.iter().enumerate() {
                println!("  {}: {:?} text='{}'", i, t.kind, t.text);
            }
        }
        Err(e) => {
            println!("ERROR: {} at line {} col {}", e.message, e.line, e.column);
        }
    }
}
