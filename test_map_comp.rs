fn main() {
    use magi_lang::syntax::parser::parse_v2;
    
    // Test map comprehension
    let code = r#"let m = {"x": 1 for i in [1,2,3]};"#;
    let result = parse_v2(code);
    match result {
        Ok(prog) => {
            println!("Parsed successfully");
            // Try to inspect the AST
            println!("Statements: {}", prog.statements.len());
        },
        Err(e) => println!("Error: {}", e.message),
    }
}
