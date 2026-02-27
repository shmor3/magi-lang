fn main() {
    use magi_lang::syntax::parser::parse_v2;
    use magi_lang::syntax::ast::*;
    
    let code = r#"let m = {"key": 1 for x in [1,2,3]};"#;
    let prog = parse_v2(code).unwrap();
    
    if let StatementKind::Let { value, .. } = &prog.statements[0].kind {
        if let ExpressionKind::MapComprehension { key_expr, value_expr, pattern, .. } = &value.kind {
            println!("Key expression: {:?}", key_expr.kind);
            println!("Value expression: {:?}", value_expr.kind);
            println!("Pattern: {:?}", pattern);
        }
    }
}
