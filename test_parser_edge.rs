fn main() {
    use magi_lang::syntax::parser::parse_v2;
    
    // Test case: trailing tokens that shouldn't be there
    let result = parse_v2("5 garbage");
    println!("Result: {:?}", result);
    match result {
        Ok(prog) => println!("Parsed: {} statements", prog.statements.len()),
        Err(e) => println!("Error: {}", e.message),
    }
}
