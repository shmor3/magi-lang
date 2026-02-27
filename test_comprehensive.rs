fn main() {
    use magi_lang::syntax::parser::parse_v2;
    
    let test_cases = vec![
        ("let m = {\"key\": 1 for x in [1,2,3]};", "map comprehension"),
        ("let m = {};", "empty block or map"),
        ("5", "single expression"),
        ("let x = 1 + + 2;", "unary plus after binary plus"),
        ("let f = |x| |y| x + y;", "nested lambda"),
        ("let x = (((((1)))));", "deeply nested parens"),
    ];
    
    for (code, desc) in test_cases {
        println!("\nTesting: {}", desc);
        println!("Code: {}", code);
        match parse_v2(code) {
            Ok(prog) => println!("  ✓ Parsed OK ({} statements)", prog.statements.len()),
            Err(e) => println!("  ✗ Error: {}", e.message),
        }
    }
}
