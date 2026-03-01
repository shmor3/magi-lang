//! Signature help provider for the MAGI LSP.
//!
//! Shows function parameter names and types when the cursor is inside a call.

use super::analysis::{find_call_context_at_position, DocumentState};
use tower_lsp::lsp_types::*;

/// Handle a signature help request.
pub fn handle_signature_help(
    state: &DocumentState,
    params: &SignatureHelpParams,
) -> Option<SignatureHelp> {
    let pos = params.text_document_position_params.position;

    let (fn_name, active_param) =
        find_call_context_at_position(&state.source, pos.line, pos.character)?;

    // Look up user-defined functions
    if let Some(func) = state.functions.get(&fn_name) {
        let parameters: Vec<ParameterInformation> = func
            .params
            .iter()
            .map(|p| ParameterInformation {
                label: ParameterLabel::Simple(p.clone()),
                documentation: None,
            })
            .collect();

        let params_str = func.params.join(", ");
        let ret = func
            .return_type
            .as_deref()
            .map_or(String::new(), |r| format!(" -> {}", r));
        let prefix = if func.is_async { "async fn" } else { "fn" };
        let label = format!("{} {}({}){}", prefix, func.name, params_str, ret);

        return Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label,
                documentation: None,
                parameters: Some(parameters),
                active_parameter: Some(active_param),
            }],
            active_signature: Some(0),
            active_parameter: Some(active_param),
        });
    }

    // Look up builtin functions with known signatures
    if let Some(sig) = builtin_signature(&fn_name, active_param) {
        return Some(sig);
    }

    None
}

/// Return signature help for well-known builtin functions.
fn builtin_signature(name: &str, active_param: u32) -> Option<SignatureHelp> {
    let (label, params): (&str, Vec<&str>) = match name {
        "len" => ("fn len(value) -> int64", vec!["value"]),
        "range" => ("fn range(start: int64, end: int64) -> [int64]", vec!["start", "end"]),
        "assert" => ("fn assert(condition: bool)", vec!["condition"]),
        "assert_eq" => ("fn assert_eq(left, right)", vec!["left", "right"]),
        "assert_ne" => ("fn assert_ne(left, right)", vec!["left", "right"]),
        "assert_throws" => ("fn assert_throws(fn_to_call)", vec!["fn_to_call"]),
        "print" => ("fn print(value)", vec!["value"]),
        "println" => ("fn println(value)", vec!["value"]),
        "debug_log" => ("fn debug_log(value)", vec!["value"]),
        "typeof" => ("fn typeof(value) -> string", vec!["value"]),
        "to_string" => ("fn to_string(value) -> string", vec!["value"]),
        "to_int64" => ("fn to_int64(value) -> int64", vec!["value"]),
        "to_float64" => ("fn to_float64(value) -> float64", vec!["value"]),
        "to_bool" => ("fn to_bool(value) -> bool", vec!["value"]),
        "to_json" => ("fn to_json(value) -> string", vec!["value"]),
        "parse_int" => ("fn parse_int(s: string) -> int64", vec!["s"]),
        "parse_float" => ("fn parse_float(s: string) -> float64", vec!["s"]),
        "abs" => ("fn abs(n: number) -> number", vec!["n"]),
        "round" => ("fn round(n: float64) -> float64", vec!["n"]),
        "floor" => ("fn floor(n: float64) -> float64", vec!["n"]),
        "ceil" => ("fn ceil(n: float64) -> float64", vec!["n"]),
        "sqrt" => ("fn sqrt(n: number) -> float64", vec!["n"]),
        "pow" => ("fn pow(base: number, exp: number) -> number", vec!["base", "exp"]),
        "min" => ("fn min(a, b)", vec!["a", "b"]),
        "max" => ("fn max(a, b)", vec!["a", "b"]),
        "clamp" => ("fn clamp(value, min, max)", vec!["value", "min", "max"]),
        "sin" => ("fn sin(radians: float64) -> float64", vec!["radians"]),
        "cos" => ("fn cos(radians: float64) -> float64", vec!["radians"]),
        "tan" => ("fn tan(radians: float64) -> float64", vec!["radians"]),
        "ln" => ("fn ln(n: float64) -> float64", vec!["n"]),
        "log2" => ("fn log2(n: float64) -> float64", vec!["n"]),
        "log10" => ("fn log10(n: float64) -> float64", vec!["n"]),
        "exp" => ("fn exp(n: float64) -> float64", vec!["n"]),
        "asin" => ("fn asin(n: float64) -> float64", vec!["n"]),
        "acos" => ("fn acos(n: float64) -> float64", vec!["n"]),
        "atan" => ("fn atan(n: float64) -> float64", vec!["n"]),
        "atan2" => ("fn atan2(y: float64, x: float64) -> float64", vec!["y", "x"]),
        "sinh" => ("fn sinh(n: float64) -> float64", vec!["n"]),
        "cosh" => ("fn cosh(n: float64) -> float64", vec!["n"]),
        "tanh" => ("fn tanh(n: float64) -> float64", vec!["n"]),
        "is_null" => ("fn is_null(value) -> bool", vec!["value"]),
        "is_string" => ("fn is_string(value) -> bool", vec!["value"]),
        "is_number" => ("fn is_number(value) -> bool", vec!["value"]),
        "is_array" => ("fn is_array(value) -> bool", vec!["value"]),
        "is_map" => ("fn is_map(value) -> bool", vec!["value"]),
        "is_bool" => ("fn is_bool(value) -> bool", vec!["value"]),
        "is_bytes" => ("fn is_bytes(value) -> bool", vec!["value"]),
        "is_finite" => ("fn is_finite(value) -> bool", vec!["value"]),
        "to_float32" => ("fn to_float32(value) -> float32", vec!["value"]),
        _ => return None,
    };

    let parameters: Vec<ParameterInformation> = params
        .iter()
        .map(|p| ParameterInformation {
            label: ParameterLabel::Simple(p.to_string()),
            documentation: None,
        })
        .collect();

    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label: label.to_string(),
            documentation: None,
            parameters: Some(parameters),
            active_parameter: Some(active_param),
        }],
        active_signature: Some(0),
        active_parameter: Some(active_param),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze_document;

    fn make_sig_params(line: u32, character: u32) -> SignatureHelpParams {
        SignatureHelpParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse("file:///test.magi").unwrap(),
                },
                position: Position { line, character },
            },
            context: None,
            work_done_progress_params: Default::default(),
        }
    }

    #[test]
    fn test_signature_help_user_function() {
        let source = "fn add(a: int64, b: int64) -> int64 { a }\nadd(1, 2)";
        let (state, _) = analyze_document(source);
        // Cursor on first arg (col 4 on line 1)
        let params = make_sig_params(1, 4);
        let result = handle_signature_help(&state, &params);
        assert!(result.is_some());
        let sig = result.unwrap();
        assert_eq!(sig.signatures.len(), 1);
        assert_eq!(sig.active_parameter, Some(0));
        assert!(sig.signatures[0].label.contains("add"));
    }

    #[test]
    fn test_signature_help_user_function_second_param() {
        let source = "fn add(a: int64, b: int64) -> int64 { a }\nadd(1, 2)";
        let (state, _) = analyze_document(source);
        // Cursor on second arg (col 7 on line 1)
        let params = make_sig_params(1, 7);
        let result = handle_signature_help(&state, &params);
        assert!(result.is_some());
        let sig = result.unwrap();
        assert_eq!(sig.active_parameter, Some(1));
    }

    #[test]
    fn test_signature_help_builtin_function() {
        let source = "range(0, 10)";
        let (state, _) = analyze_document(source);
        let params = make_sig_params(0, 6);
        let result = handle_signature_help(&state, &params);
        assert!(result.is_some());
        let sig = result.unwrap();
        assert!(sig.signatures[0].label.contains("range"));
        assert_eq!(sig.active_parameter, Some(0));
    }

    #[test]
    fn test_signature_help_builtin_second_param() {
        let source = "range(0, 10)";
        let (state, _) = analyze_document(source);
        // Cursor after comma (col 9 on "10")
        let params = make_sig_params(0, 9);
        let result = handle_signature_help(&state, &params);
        assert!(result.is_some());
        let sig = result.unwrap();
        assert_eq!(sig.active_parameter, Some(1));
    }

    #[test]
    fn test_signature_help_no_call() {
        let source = "let x = 5;";
        let (state, _) = analyze_document(source);
        let params = make_sig_params(0, 5);
        let result = handle_signature_help(&state, &params);
        assert!(result.is_none());
    }

    #[test]
    fn test_signature_help_empty_document() {
        let source = "";
        let (state, _) = analyze_document(source);
        let params = make_sig_params(0, 0);
        let result = handle_signature_help(&state, &params);
        assert!(result.is_none());
    }

    #[test]
    fn test_signature_help_unknown_function() {
        let source = "unknown_fn(x)";
        let (state, _) = analyze_document(source);
        let params = make_sig_params(0, 12);
        let result = handle_signature_help(&state, &params);
        // "unknown_fn" is not defined and not a builtin
        assert!(result.is_none());
    }

    #[test]
    fn test_signature_help_clamp_third_param() {
        let source = "clamp(x, 0, 100)";
        let (state, _) = analyze_document(source);
        // Cursor on third arg
        let params = make_sig_params(0, 13);
        let result = handle_signature_help(&state, &params);
        assert!(result.is_some());
        let sig = result.unwrap();
        assert_eq!(sig.active_parameter, Some(2));
    }
}
