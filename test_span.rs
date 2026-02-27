fn main() {
    use magi_lang::syntax::parser::parse_v2;
    
    let code = "let x = 42;";
    let prog = parse_v2(code).unwrap();
    let stmt = &prog.statements[0];
    
    println!("Statement span: {}", stmt.span);
    println!("Expected: 1:1-1:11");
}
