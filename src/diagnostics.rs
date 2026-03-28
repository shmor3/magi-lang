//! Rich diagnostic rendering for the MAGI language.

/// Render a diagnostic with source context to stderr.
pub fn render_error(
    filename: &str,
    source: &str,
    line: u32,
    column: u32,
    message: &str,
    code: Option<&str>,
    help: Option<&str>,
    suggestion: Option<&str>,
) {
    crate::util::render_diagnostic(filename, source, line, column, message, code, help, suggestion, false);
}

/// Render a warning with source context to stderr.
pub fn render_warning(
    filename: &str,
    source: &str,
    line: u32,
    column: u32,
    message: &str,
    code: Option<&str>,
    help: Option<&str>,
) {
    crate::util::render_diagnostic(filename, source, line, column, message, code, help, None, true);
}
