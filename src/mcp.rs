//! MCP (Model Context Protocol) server for the MAGI language.
//!
//! Exposes MAGI tools over JSON-RPC stdio transport.

use std::io::{self, BufRead, Write};
use std::collections::HashMap;
use crate::util::{JsonValue, OrderedMap, json_parse_value, json_to_string};
use crate::eval::{OperationEvaluator, EvalError};
use crate::types::{DataType, operations::OperationType};

struct McpEvaluator;
impl OperationEvaluator for McpEvaluator {
    fn eval_operation(&self, _op: OperationType, _inputs: &HashMap<String, DataType>, _config: &HashMap<String, DataType>) -> Result<DataType, EvalError> {
        Ok(DataType::Null)
    }
}

pub fn run_mcp_server() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 { break; }
        let content_length = if header.to_lowercase().starts_with("content-length:") {
            header.trim_start_matches(|c: char| !c.is_ascii_digit()).trim().parse::<usize>().unwrap_or(0)
        } else { continue; };

        let mut blank = String::new();
        let _ = reader.read_line(&mut blank);

        let mut body = vec![0u8; content_length];
        if io::Read::read_exact(&mut reader, &mut body).is_err() { break; }
        let body_str = String::from_utf8_lossy(&body);

        let request = match json_parse_value(&body_str) { Ok(v) => v, Err(_) => continue };
        let method = get_str(&request, "method").unwrap_or_default();
        let id = get_value(&request, "id");
        let params = get_value(&request, "params");

        let response = match method.as_str() {
            "initialize" => handle_initialize(id),
            "initialized" => continue,
            "tools/list" => handle_tools_list(id),
            "tools/call" => handle_tools_call(id, &params),
            "shutdown" => { send_response(&mut writer, &make_result(id, JsonValue::Null)); break; }
            _ => make_error(id, -32601, "Method not found"),
        };
        send_response(&mut writer, &response);
    }
}

fn handle_initialize(id: JsonValue) -> JsonValue {
    let result = JsonValue::Object(OrderedMap::from([
        ("protocolVersion".into(), JsonValue::String("2024-11-05".into())),
        ("capabilities".into(), JsonValue::Object(OrderedMap::from([
            ("tools".into(), JsonValue::Object(OrderedMap::new())),
        ]))),
        ("serverInfo".into(), JsonValue::Object(OrderedMap::from([
            ("name".into(), JsonValue::String("magi-mcp".into())),
            ("version".into(), JsonValue::String(crate::version::version_string())),
        ]))),
    ]));
    make_result(id, result)
}

fn handle_tools_list(id: JsonValue) -> JsonValue {
    let tools = vec![
        tool_def("magi_run", "Execute MAGI source code and return output", &[
            param("code", "string", "MAGI source code"),
        ]),
        tool_def("magi_check", "Type-check MAGI source and return diagnostics", &[
            param("code", "string", "MAGI source code"),
        ]),
        tool_def("magi_format", "Format MAGI source code", &[
            param("code", "string", "MAGI source code"),
        ]),
        tool_def("magi_lint", "Lint MAGI source and return warnings", &[
            param("code", "string", "MAGI source code"),
        ]),
        tool_def("magi_parse", "Parse MAGI source and return AST summary", &[
            param("code", "string", "MAGI source code"),
        ]),
        tool_def("magi_stdlib", "List stdlib modules and functions", &[
            param("module", "string", "Module name (optional)"),
        ]),
        tool_def("magi_version", "Return MAGI version info", &[]),
    ];
    make_result(id, JsonValue::Object(OrderedMap::from([
        ("tools".into(), JsonValue::Array(tools)),
    ])))
}

fn handle_tools_call(id: JsonValue, params: &JsonValue) -> JsonValue {
    let tool_name = get_str(params, "name").unwrap_or_default();
    let arguments = get_value(params, "arguments");
    let result = match tool_name.as_str() {
        "magi_run" => tool_run(&arguments),
        "magi_check" => tool_check(&arguments),
        "magi_format" => tool_format(&arguments),
        "magi_lint" => tool_lint(&arguments),
        "magi_parse" => tool_parse(&arguments),
        "magi_stdlib" => tool_stdlib(&arguments),
        "magi_version" => tool_version(),
        _ => tool_error(&format!("Unknown tool: {}", tool_name)),
    };
    make_result(id, result)
}

fn tool_run(args: &JsonValue) -> JsonValue {
    let code = get_str(args, "code").unwrap_or_default();
    match crate::syntax::parser::parse_v2(&code) {
        Ok(program) => {
            let ev = McpEvaluator;
            let mut interp = crate::syntax::interpreter::Interpreter::new(&ev);
            match interp.execute(&program) {
                Ok(result) => {
                    let mut output = String::new();
                    for log in &interp.logs { output.push_str(&log.message); output.push('\n'); }
                    output.push_str(&result.to_string_lossy());
                    tool_text(&output)
                }
                Err(e) => tool_error(&format!("{}", e)),
            }
        }
        Err(e) => tool_error(&e.message),
    }
}

fn tool_check(args: &JsonValue) -> JsonValue {
    let code = get_str(args, "code").unwrap_or_default();
    match crate::syntax::parser::parse_v2(&code) {
        Ok(program) => {
            let imports = std::collections::HashSet::new();
            let analysis = crate::syntax::type_checker::check_types(&program, &imports);
            let diags: Vec<String> = analysis.diagnostics.iter()
                .map(|d| format!("{}:{}: {} [{}]", d.line, d.column, d.message, d.code.as_deref().unwrap_or("")))
                .collect();
            if diags.is_empty() { tool_text("No issues found.") } else { tool_text(&diags.join("\n")) }
        }
        Err(e) => tool_error(&e.message),
    }
}

fn tool_format(args: &JsonValue) -> JsonValue {
    let code = get_str(args, "code").unwrap_or_default();
    match crate::syntax::parser::parse_v2(&code) {
        Ok(program) => {
            let formatted = crate::formatter::format_program(&program, &crate::formatter::FormatConfig::default());
            tool_text(&formatted)
        }
        Err(e) => tool_error(&e.message),
    }
}

fn tool_lint(args: &JsonValue) -> JsonValue {
    let code = get_str(args, "code").unwrap_or_default();
    match crate::syntax::parser::parse_v2(&code) {
        Ok(program) => {
            let result = crate::linter::lint(&program, &crate::linter::LintConfig::default());
            let diags: Vec<String> = result.diagnostics.iter()
                .map(|d| format!("{}:{}: {} [{}]", d.line, d.column, d.message, d.code.as_deref().unwrap_or("")))
                .collect();
            if diags.is_empty() { tool_text("No warnings.") } else { tool_text(&diags.join("\n")) }
        }
        Err(e) => tool_error(&e.message),
    }
}

fn tool_parse(args: &JsonValue) -> JsonValue {
    let code = get_str(args, "code").unwrap_or_default();
    match crate::syntax::parser::parse_v2(&code) {
        Ok(program) => {
            let mut summary = format!("{} statements\n", program.statements.len());
            for stmt in &program.statements {
                let kind = match &stmt.kind {
                    crate::syntax::ast::StatementKind::FunctionDef(f) => format!("fn {}", f.name),
                    crate::syntax::ast::StatementKind::StructDef { name, .. } => format!("struct {}", name),
                    crate::syntax::ast::StatementKind::EnumDef { name, .. } => format!("enum {}", name),
                    crate::syntax::ast::StatementKind::TraitDef { name, .. } => format!("trait {}", name),
                    crate::syntax::ast::StatementKind::ImplBlock { type_name, .. } => format!("impl {}", type_name),
                    crate::syntax::ast::StatementKind::TestDef { name, .. } => format!("test {}", name),
                    crate::syntax::ast::StatementKind::Let { name, .. } => format!("let {}", name),
                    crate::syntax::ast::StatementKind::ConstDef { name, .. } => format!("const {}", name),
                    crate::syntax::ast::StatementKind::Use { path, .. } => format!("use {}", path.join("::")),
                    _ => "stmt".to_string(),
                };
                summary.push_str(&format!("  line {}: {}\n", stmt.span.start_line, kind));
            }
            tool_text(&summary)
        }
        Err(e) => tool_error(&e.message),
    }
}

fn tool_stdlib(args: &JsonValue) -> JsonValue {
    let module = get_str(args, "module").unwrap_or_default();
    if module.is_empty() {
        let modules: Vec<&str> = crate::syntax::interpreter::STD_MODULE_NAMES.to_vec();
        tool_text(&format!("{} modules: {}", modules.len(), modules.join(", ")))
    } else {
        let ops = crate::syntax::interpreter::std_module_ops(&module);
        if ops.is_empty() { tool_text(&format!("Module '{}' not found.", module)) }
        else { tool_text(&format!("{} ({} functions): {}", module, ops.len(), ops.join(", "))) }
    }
}

fn tool_version() -> JsonValue {
    tool_text(&format!("MAGI v{} ({}/{})", crate::version::version_string(), std::env::consts::ARCH, std::env::consts::OS))
}

fn tool_text(text: &str) -> JsonValue {
    JsonValue::Object(OrderedMap::from([
        ("content".into(), JsonValue::Array(vec![
            JsonValue::Object(OrderedMap::from([
                ("type".into(), JsonValue::String("text".into())),
                ("text".into(), JsonValue::String(text.into())),
            ])),
        ])),
    ]))
}

fn tool_error(msg: &str) -> JsonValue {
    JsonValue::Object(OrderedMap::from([
        ("content".into(), JsonValue::Array(vec![
            JsonValue::Object(OrderedMap::from([
                ("type".into(), JsonValue::String("text".into())),
                ("text".into(), JsonValue::String(msg.into())),
            ])),
        ])),
        ("isError".into(), JsonValue::Bool(true)),
    ]))
}

fn tool_def(name: &str, desc: &str, params: &[JsonValue]) -> JsonValue {
    let mut schema = OrderedMap::from([("type".into(), JsonValue::String("object".into()))]);
    if !params.is_empty() {
        let mut props = OrderedMap::new();
        for p in params {
            if let JsonValue::Object(m) = p {
                if let Some(JsonValue::String(n)) = m.get("name") { props.insert(n.clone(), p.clone()); }
            }
        }
        schema.insert("properties".into(), JsonValue::Object(props));
    }
    JsonValue::Object(OrderedMap::from([
        ("name".into(), JsonValue::String(name.into())),
        ("description".into(), JsonValue::String(desc.into())),
        ("inputSchema".into(), JsonValue::Object(schema)),
    ]))
}

fn param(name: &str, typ: &str, desc: &str) -> JsonValue {
    JsonValue::Object(OrderedMap::from([
        ("name".into(), JsonValue::String(name.into())),
        ("type".into(), JsonValue::String(typ.into())),
        ("description".into(), JsonValue::String(desc.into())),
    ]))
}

fn make_result(id: JsonValue, result: JsonValue) -> JsonValue {
    JsonValue::Object(OrderedMap::from([
        ("jsonrpc".into(), JsonValue::String("2.0".into())),
        ("id".into(), id),
        ("result".into(), result),
    ]))
}

fn make_error(id: JsonValue, code: i64, message: &str) -> JsonValue {
    JsonValue::Object(OrderedMap::from([
        ("jsonrpc".into(), JsonValue::String("2.0".into())),
        ("id".into(), id),
        ("error".into(), JsonValue::Object(OrderedMap::from([
            ("code".into(), JsonValue::Number(crate::util::JsonNumber::Int(code))),
            ("message".into(), JsonValue::String(message.into())),
        ]))),
    ]))
}

fn send_response(writer: &mut impl Write, response: &JsonValue) {
    let body = json_to_string(response);
    let _ = write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = writer.flush();
}

fn get_str(val: &JsonValue, key: &str) -> Option<String> {
    if let JsonValue::Object(m) = val { if let Some(JsonValue::String(s)) = m.get(key) { return Some(s.clone()); } }
    None
}

fn get_value(val: &JsonValue, key: &str) -> JsonValue {
    if let JsonValue::Object(m) = val { if let Some(v) = m.get(key) { return v.clone(); } }
    JsonValue::Null
}
