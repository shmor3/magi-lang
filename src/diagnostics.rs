//! Rich diagnostic rendering for the MAGI language using ariadne.

use ariadne::{Color, Label, Report, ReportKind, Source};

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
    // Convert line:col to byte offset
    let offset = line_col_to_offset(source, line as usize, column as usize);
    let span_end = next_char_boundary(source, offset);

    let mut report = Report::build(ReportKind::Error, (filename, offset..span_end));

    if let Some(code) = code {
        report = report.with_code(code);
    }

    report = report.with_message(message);
    report = report.with_label(
        Label::new((filename, offset..span_end))
            .with_message(message)
            .with_color(Color::Red),
    );

    if let Some(help) = help {
        report = report.with_help(help);
    }
    if let Some(suggestion) = suggestion {
        report = report.with_note(suggestion);
    }

    let _ = report
        .finish()
        .eprint((filename, Source::from(source)));
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
    let offset = line_col_to_offset(source, line as usize, column as usize);
    let span_end = next_char_boundary(source, offset);

    let mut report = Report::build(ReportKind::Warning, (filename, offset..span_end));

    if let Some(code) = code {
        report = report.with_code(code);
    }

    report = report.with_message(message);
    report = report.with_label(
        Label::new((filename, offset..span_end))
            .with_message(message)
            .with_color(Color::Yellow),
    );

    if let Some(help) = help {
        report = report.with_help(help);
    }

    let _ = report
        .finish()
        .eprint((filename, Source::from(source)));
}

/// Convert a 1-based line and 1-based column to a byte offset.
fn line_col_to_offset(source: &str, line: usize, col: usize) -> usize {
    let mut current_line = 1;
    for (i, ch) in source.char_indices() {
        if current_line == line {
            // Count columns (1-based)
            let mut current_col = 1;
            for (j, _) in source[i..].char_indices() {
                if current_col == col {
                    return i + j;
                }
                current_col += 1;
                if source.as_bytes().get(i + j) == Some(&b'\n') {
                    break;
                }
            }
            return i; // fallback to start of line
        }
        if ch == '\n' {
            current_line += 1;
        }
    }
    source.len() // fallback to end of source
}

/// Return the byte offset of the next character boundary after `offset`,
/// so that we always have a non-empty span for ariadne labels.
fn next_char_boundary(source: &str, offset: usize) -> usize {
    if offset >= source.len() {
        return source.len();
    }
    let rest = &source[offset..];
    let mut chars = rest.chars();
    match chars.next() {
        Some(ch) => offset + ch.len_utf8(),
        None => source.len(),
    }
}
