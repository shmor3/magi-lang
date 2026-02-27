//! Tokenizer for the MAGI v2 language.
//!
//! Converts source text into a stream of tokens with span information.
//! Supports keywords, operators, delimiters, literals, comments, and identifiers.

use super::ast::Span;
use super::SyntaxError;
use std::fmt;

// =============================================================================
// Token types
// =============================================================================

/// A token with its kind and source location.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    /// The raw text of the token (for identifiers, literals, etc.)
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    // Keywords
    Let,
    Mut,
    Import,
    Output,
    If,
    Else,
    For,
    In,
    While,
    True,
    False,
    Null,
    Fn,
    Async,
    Await,
    Spawn,
    Break,
    Continue,
    Return,
    // New keywords (promoted from reserved)
    Match,
    Use,
    Mod,
    Const,
    Type,
    As,
    Pub,
    Loop,
    // New keywords (error handling)
    Try,
    Catch,
    Finally,
    Throw,
    // Testing
    Test,
    // Promoted keywords (enum, struct)
    Enum,
    Struct,
    /// A reserved keyword that cannot be used as an identifier.
    Reserved,

    // Literals
    IntLiteral,
    FloatLiteral,
    StringLiteral,
    /// f"string interpolation" prefix
    FStringStart,

    // Identifier (variable names, function names)
    Ident,

    // Operators
    Plus,             // +
    Minus,            // -
    Star,             // *
    Slash,            // /
    Percent,          // %
    EqEq,             // ==
    NotEq,            // !=
    Gt,               // >
    Lt,               // <
    GtEq,             // >=
    LtEq,             // <=
    AndAnd,           // &&
    PipePipe,         // ||
    Bang,             // !
    Eq,               // =
    Pipe,             // |> (pipe operator)
    Bar,              // | (lambda params, match or-patterns)
    Arrow,            // -> (return type)
    FatArrow,         // => (match arms)
    PlusEq,           // +=
    MinusEq,          // -=
    StarEq,           // *=
    SlashEq,          // /=
    PercentEq,        // %=
    QuestionQuestion, // ??
    QuestionDot,      // ?.
    DotDot,           // ..
    DotDotEq,         // ..=
    DotDotDot,        // ...
    Question,         // ? (standalone, error propagation)

    // Delimiters
    LParen,     // (
    RParen,     // )
    LBracket,   // [
    RBracket,   // ]
    LBrace,     // {
    RBrace,     // }
    Colon,      // :
    ColonColon, // :: (module path separator)
    Semicolon,  // ;
    Comma,      // ,
    Dot,        // .
    Underscore, // _ (standalone placeholder)

    // End of file
    Eof,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TokenKind::Let => "let",
            TokenKind::Mut => "mut",
            TokenKind::Import => "import",
            TokenKind::Output => "output",
            TokenKind::If => "if",
            TokenKind::Else => "else",
            TokenKind::For => "for",
            TokenKind::In => "in",
            TokenKind::While => "while",
            TokenKind::True => "true",
            TokenKind::False => "false",
            TokenKind::Null => "null",
            TokenKind::Fn => "fn",
            TokenKind::Async => "async",
            TokenKind::Await => "await",
            TokenKind::Spawn => "spawn",
            TokenKind::Break => "break",
            TokenKind::Continue => "continue",
            TokenKind::Return => "return",
            TokenKind::Match => "match",
            TokenKind::Use => "use",
            TokenKind::Mod => "mod",
            TokenKind::Const => "const",
            TokenKind::Type => "type",
            TokenKind::As => "as",
            TokenKind::Pub => "pub",
            TokenKind::Loop => "loop",
            TokenKind::Try => "try",
            TokenKind::Catch => "catch",
            TokenKind::Finally => "finally",
            TokenKind::Throw => "throw",
            TokenKind::Test => "test",
            TokenKind::Enum => "enum",
            TokenKind::Struct => "struct",
            TokenKind::Reserved => "reserved keyword",
            TokenKind::IntLiteral => "integer",
            TokenKind::FloatLiteral => "float",
            TokenKind::StringLiteral => "string",
            TokenKind::FStringStart => "f-string",
            TokenKind::Ident => "identifier",
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Percent => "%",
            TokenKind::EqEq => "==",
            TokenKind::NotEq => "!=",
            TokenKind::Gt => ">",
            TokenKind::Lt => "<",
            TokenKind::GtEq => ">=",
            TokenKind::LtEq => "<=",
            TokenKind::AndAnd => "&&",
            TokenKind::PipePipe => "||",
            TokenKind::Bang => "!",
            TokenKind::Eq => "=",
            TokenKind::Pipe => "|>",
            TokenKind::Bar => "|",
            TokenKind::Arrow => "->",
            TokenKind::FatArrow => "=>",
            TokenKind::PlusEq => "+=",
            TokenKind::MinusEq => "-=",
            TokenKind::StarEq => "*=",
            TokenKind::SlashEq => "/=",
            TokenKind::PercentEq => "%=",
            TokenKind::QuestionQuestion => "??",
            TokenKind::QuestionDot => "?.",
            TokenKind::DotDot => "..",
            TokenKind::DotDotEq => "..=",
            TokenKind::DotDotDot => "...",
            TokenKind::Question => "?",
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::LBrace => "{",
            TokenKind::RBrace => "}",
            TokenKind::Colon => ":",
            TokenKind::ColonColon => "::",
            TokenKind::Semicolon => ";",
            TokenKind::Comma => ",",
            TokenKind::Dot => ".",
            TokenKind::Underscore => "_",
            TokenKind::Eof => "EOF",
        };
        write!(f, "{}", s)
    }
}

// =============================================================================
// Reserved keywords
// =============================================================================

/// Keywords reserved for future use. Using these as identifiers is an error.
/// Note: match, use, mod, const, type, as, pub, loop are now active keywords.
pub const RESERVED_KEYWORDS: &[&str] = &[
    "trait", "impl", "static", "ref", "move", "yield", "self", "super", "where", "dyn",
];

/// Check if a name is a reserved keyword.
pub fn is_reserved_keyword(name: &str) -> bool {
    RESERVED_KEYWORDS.contains(&name)
}

// =============================================================================
// Lexer
// =============================================================================

/// Tokenize source code into a vector of tokens.
pub fn tokenize(source: &str) -> Result<Vec<Token>, SyntaxError> {
    let mut lexer = Lexer::new(source);
    let mut done = false;
    std::iter::from_fn(|| {
        if done { return None; }
        let token = lexer.next_token();
        if matches!(&token, Ok(t) if t.kind == TokenKind::Eof) {
            done = true;
        }
        Some(token)
    })
    .collect()
}

struct Lexer<'a> {
    source: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source: source.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.source.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let ch = self.source.get(self.pos).copied()?;
        self.pos += 1;
        if ch == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    /// Advance and decode one full UTF-8 character from the source.
    /// Used inside string literals to correctly handle multi-byte characters.
    /// Always advances at least one byte to prevent infinite loops.
    fn advance_char(&mut self) -> Option<char> {
        let first = self.source.get(self.pos).copied()?;
        // Determine UTF-8 sequence length from the first byte
        let byte_len = if first < 0x80 { 1 }
            else if first < 0xE0 { 2 }
            else if first < 0xF0 { 3 }
            else { 4 };
        let end = (self.pos + byte_len).min(self.source.len());
        let slice = &self.source[self.pos..end];
        match std::str::from_utf8(slice).ok().and_then(|s| s.chars().next()) {
            Some(ch) => {
                self.pos += byte_len;
                if ch == '\n' {
                    self.line += 1;
                    self.col = 1;
                } else {
                    self.col += 1;
                }
                Some(ch)
            }
            None => {
                // Invalid or incomplete UTF-8: advance one byte as replacement char
                self.pos += 1;
                self.col += 1;
                Some('\u{FFFD}')
            }
        }
    }

    fn skip_whitespace_and_comments(&mut self) -> Result<(), SyntaxError> {
        loop {
            // Skip whitespace
            while let Some(ch) = self.peek() {
                if ch == b' ' || ch == b'\t' || ch == b'\r' || ch == b'\n' {
                    self.advance();
                } else {
                    break;
                }
            }

            // Skip line comments
            if self.peek() == Some(b'/') && self.peek_at(1) == Some(b'/') {
                while let Some(ch) = self.advance() {
                    if ch == b'\n' {
                        break;
                    }
                }
                continue;
            }

            // Skip block comments (nested: /* ... /* ... */ ... */)
            if self.peek() == Some(b'/') && self.peek_at(1) == Some(b'*') {
                let comment_line = self.line;
                let comment_col = self.col;
                self.advance(); // consume /
                self.advance(); // consume *
                let mut depth: u32 = 1;
                while depth > 0 {
                    match self.advance() {
                        None => {
                            return Err(SyntaxError {
                                line: comment_line as usize,
                                column: comment_col as usize,
                                message: "Unterminated block comment".to_string(),
                            });
                        }
                        Some(b'/') if self.peek() == Some(b'*') => {
                            self.advance();
                            depth += 1;
                        }
                        Some(b'*') if self.peek() == Some(b'/') => {
                            self.advance();
                            depth -= 1;
                        }
                        _ => {}
                    }
                }
                continue;
            }

            break;
        }
        Ok(())
    }

    fn next_token(&mut self) -> Result<Token, SyntaxError> {
        self.skip_whitespace_and_comments()?;

        let start_line = self.line;
        let start_col = self.col;

        let ch = match self.peek() {
            Some(ch) => ch,
            None => {
                return Ok(Token {
                    kind: TokenKind::Eof,
                    span: Span::point(start_line, start_col),
                    text: String::new(),
                });
            }
        };

        // Simple single-character tokens (no multi-char variants)
        match ch {
            b'(' => return self.single_char_token(TokenKind::LParen, start_line, start_col),
            b')' => return self.single_char_token(TokenKind::RParen, start_line, start_col),
            b'[' => return self.single_char_token(TokenKind::LBracket, start_line, start_col),
            b']' => return self.single_char_token(TokenKind::RBracket, start_line, start_col),
            b'{' => return self.single_char_token(TokenKind::LBrace, start_line, start_col),
            b'}' => return self.single_char_token(TokenKind::RBrace, start_line, start_col),
            b';' => return self.single_char_token(TokenKind::Semicolon, start_line, start_col),
            b',' => return self.single_char_token(TokenKind::Comma, start_line, start_col),
            _ => {}
        }

        // Multi-character operators that start with characters that could be single-char tokens
        match ch {
            b'+' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::PlusEq,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "+=".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Plus,
                    span: Span::point(start_line, start_col),
                    text: "+".to_string(),
                });
            }
            b'*' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::StarEq,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "*=".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Star,
                    span: Span::point(start_line, start_col),
                    text: "*".to_string(),
                });
            }
            b'/' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::SlashEq,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "/=".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Slash,
                    span: Span::point(start_line, start_col),
                    text: "/".to_string(),
                });
            }
            b'%' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::PercentEq,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "%=".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Percent,
                    span: Span::point(start_line, start_col),
                    text: "%".to_string(),
                });
            }
            b':' => {
                self.advance();
                if self.peek() == Some(b':') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::ColonColon,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "::".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Colon,
                    span: Span::point(start_line, start_col),
                    text: ":".to_string(),
                });
            }
            b'.' => {
                self.advance();
                if self.peek() == Some(b'.') {
                    self.advance();
                    if self.peek() == Some(b'.') {
                        self.advance();
                        return Ok(Token {
                            kind: TokenKind::DotDotDot,
                            span: Span::new(start_line, start_col, self.line, self.col - 1),
                            text: "...".to_string(),
                        });
                    }
                    if self.peek() == Some(b'=') {
                        self.advance();
                        return Ok(Token {
                            kind: TokenKind::DotDotEq,
                            span: Span::new(start_line, start_col, self.line, self.col - 1),
                            text: "..=".to_string(),
                        });
                    }
                    return Ok(Token {
                        kind: TokenKind::DotDot,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "..".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Dot,
                    span: Span::point(start_line, start_col),
                    text: ".".to_string(),
                });
            }
            b'?' => {
                self.advance();
                if self.peek() == Some(b'?') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::QuestionQuestion,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "??".to_string(),
                    });
                }
                if self.peek() == Some(b'.') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::QuestionDot,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "?.".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Question,
                    span: Span::point(start_line, start_col),
                    text: "?".to_string(),
                });
            }
            _ => {}
        }

        // Multi-character operators (existing)
        match ch {
            b'=' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::EqEq,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "==".to_string(),
                    });
                }
                if self.peek() == Some(b'>') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::FatArrow,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "=>".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Eq,
                    span: Span::point(start_line, start_col),
                    text: "=".to_string(),
                });
            }
            b'!' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::NotEq,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "!=".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Bang,
                    span: Span::point(start_line, start_col),
                    text: "!".to_string(),
                });
            }
            b'>' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::GtEq,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: ">=".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Gt,
                    span: Span::point(start_line, start_col),
                    text: ">".to_string(),
                });
            }
            b'<' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::LtEq,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "<=".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Lt,
                    span: Span::point(start_line, start_col),
                    text: "<".to_string(),
                });
            }
            b'&' => {
                self.advance();
                if self.peek() == Some(b'&') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::AndAnd,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "&&".to_string(),
                    });
                }
                return Err(SyntaxError {
                    line: start_line as usize,
                    column: start_col as usize,
                    message: "Expected '&&', got single '&'".to_string(),
                });
            }
            b'|' => {
                self.advance();
                if self.peek() == Some(b'>') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::Pipe,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "|>".to_string(),
                    });
                }
                if self.peek() == Some(b'|') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::PipePipe,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "||".to_string(),
                    });
                }
                // Single | for lambda params and or-patterns
                return Ok(Token {
                    kind: TokenKind::Bar,
                    span: Span::point(start_line, start_col),
                    text: "|".to_string(),
                });
            }
            b'-' => {
                self.advance();
                if self.peek() == Some(b'>') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::Arrow,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "->".to_string(),
                    });
                }
                if self.peek() == Some(b'=') {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::MinusEq,
                        span: Span::new(start_line, start_col, self.line, self.col - 1),
                        text: "-=".to_string(),
                    });
                }
                return Ok(Token {
                    kind: TokenKind::Minus,
                    span: Span::point(start_line, start_col),
                    text: "-".to_string(),
                });
            }
            _ => {}
        }

        // Raw string: r"no\escape"
        if ch == b'r' && self.peek_at(1) == Some(b'"') {
            self.advance(); // consume 'r'
            return self.lex_raw_string(start_line, start_col);
        }

        // f-string: f"hello {name}"
        if ch == b'f' && self.peek_at(1) == Some(b'"') {
            self.advance(); // consume 'f'
            return self.lex_fstring(start_line, start_col);
        }

        // String literal (triple-quoted or regular)
        if ch == b'"' {
            if self.peek_at(1) == Some(b'"') && self.peek_at(2) == Some(b'"') {
                return self.lex_triple_string(start_line, start_col);
            }
            return self.lex_string(start_line, start_col);
        }

        // Number literal (digit or negative sign handled by parser as unary minus)
        if ch.is_ascii_digit() {
            return self.lex_number(start_line, start_col);
        }

        // Identifier or keyword
        if ch.is_ascii_alphabetic() || ch == b'_' {
            return self.lex_identifier(start_line, start_col);
        }

        // Decode the actual Unicode character for a better error message
        let display_char = if ch.is_ascii() {
            (ch as char).to_string()
        } else {
            // Try to decode a UTF-8 character starting at current position
            let remaining = &self.source[self.pos..];
            std::str::from_utf8(remaining)
                .ok()
                .and_then(|s| s.chars().next())
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("0x{:02X}", ch))
        };
        Err(SyntaxError {
            line: start_line as usize,
            column: start_col as usize,
            message: format!("Unexpected character: '{}'", display_char),
        })
    }

    fn single_char_token(
        &mut self,
        kind: TokenKind,
        start_line: u32,
        start_col: u32,
    ) -> Result<Token, SyntaxError> {
        let ch = self.advance().ok_or_else(|| SyntaxError {
            line: start_line as usize,
            column: start_col as usize,
            message: "Unexpected end of input".to_string(),
        })?;
        Ok(Token {
            kind,
            span: Span::point(start_line, start_col),
            text: String::from(ch as char),
        })
    }

    /// Parse an escape sequence after consuming the backslash.
    /// Returns the resulting character.
    fn parse_escape_sequence(&mut self) -> Result<char, SyntaxError> {
        match self.advance() {
            Some(b'n') => Ok('\n'),
            Some(b't') => Ok('\t'),
            Some(b'r') => Ok('\r'),
            Some(b'\\') => Ok('\\'),
            Some(b'"') => Ok('"'),
            Some(b'0') => Ok('\0'),
            Some(b'{') => Ok('{'),
            Some(b'}') => Ok('}'),
            Some(b'x') => {
                // \xHH — two hex digits
                let mut hex = String::with_capacity(2);
                for _ in 0..2 {
                    match self.advance() {
                        Some(ch) if (ch as char).is_ascii_hexdigit() => hex.push(ch as char),
                        _ => {
                            return Err(SyntaxError {
                                line: self.line as usize,
                                column: self.col as usize,
                                message: "Expected two hex digits after \\x".to_string(),
                            });
                        }
                    }
                }
                let code = u8::from_str_radix(&hex, 16).map_err(|_| SyntaxError {
                    line: self.line as usize,
                    column: self.col as usize,
                    message: format!("Invalid hex escape: \\x{}", hex),
                })?;
                Ok(code as char)
            }
            Some(b'u') => {
                // \u{HHHH} — 1-6 hex digits in braces
                if self.peek() != Some(b'{') {
                    return Err(SyntaxError {
                        line: self.line as usize,
                        column: self.col as usize,
                        message: "Expected '{' after \\u".to_string(),
                    });
                }
                self.advance(); // consume {
                let mut hex = String::with_capacity(6);
                loop {
                    match self.peek() {
                        Some(b'}') => {
                            self.advance();
                            break;
                        }
                        Some(ch) if (ch as char).is_ascii_hexdigit() && hex.len() < 6 => {
                            hex.push(ch as char);
                            self.advance();
                        }
                        _ => {
                            return Err(SyntaxError {
                                line: self.line as usize,
                                column: self.col as usize,
                                message: "Invalid unicode escape: expected hex digits and '}'".to_string(),
                            });
                        }
                    }
                }
                if hex.is_empty() {
                    return Err(SyntaxError {
                        line: self.line as usize,
                        column: self.col as usize,
                        message: "Empty unicode escape \\u{}".to_string(),
                    });
                }
                let code = u32::from_str_radix(&hex, 16).map_err(|_| SyntaxError {
                    line: self.line as usize,
                    column: self.col as usize,
                    message: format!("Invalid unicode escape: \\u{{{}}}", hex),
                })?;
                char::from_u32(code).ok_or_else(|| SyntaxError {
                    line: self.line as usize,
                    column: self.col as usize,
                    message: format!("Invalid unicode code point: U+{:04X}", code),
                })
            }
            Some(ch) => {
                Err(SyntaxError {
                    line: self.line as usize,
                    column: (self.col - 1) as usize,
                    message: format!("Invalid escape sequence: \\{}", ch as char),
                })
            }
            None => {
                Err(SyntaxError {
                    line: self.line as usize,
                    column: self.col as usize,
                    message: "Unterminated escape sequence".to_string(),
                })
            }
        }
    }

    fn lex_string(&mut self, start_line: u32, start_col: u32) -> Result<Token, SyntaxError> {
        self.advance(); // consume opening "
        let mut value = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(SyntaxError {
                        line: start_line as usize,
                        column: start_col as usize,
                        message: "Unterminated string literal".to_string(),
                    });
                }
                Some(b'"') => { self.advance(); break; }
                Some(b'\\') => {
                    self.advance(); // consume backslash
                    let ch = self.parse_escape_sequence()?;
                    value.push(ch);
                }
                _ => {
                    if let Some(ch) = self.advance_char() {
                        value.push(ch);
                    }
                }
            }
        }
        Ok(Token {
            kind: TokenKind::StringLiteral,
            span: Span::new(start_line, start_col, self.line, self.col - 1),
            text: value,
        })
    }

    fn lex_triple_string(&mut self, start_line: u32, start_col: u32) -> Result<Token, SyntaxError> {
        self.advance(); // consume first "
        self.advance(); // consume second "
        self.advance(); // consume third "
        let mut value = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(SyntaxError {
                        line: start_line as usize,
                        column: start_col as usize,
                        message: "Unterminated triple-quoted string".to_string(),
                    });
                }
                Some(b'"') if self.peek_at(1) == Some(b'"') && self.peek_at(2) == Some(b'"') => {
                    self.advance(); // consume first "
                    self.advance(); // consume second "
                    self.advance(); // consume third "
                    break;
                }
                Some(b'\\') => {
                    self.advance(); // consume backslash
                    let ch = self.parse_escape_sequence()?;
                    value.push(ch);
                }
                _ => {
                    if let Some(ch) = self.advance_char() {
                        value.push(ch);
                    }
                }
            }
        }
        Ok(Token {
            kind: TokenKind::StringLiteral,
            span: Span::new(start_line, start_col, self.line, self.col - 1),
            text: value,
        })
    }

    fn lex_raw_string(&mut self, start_line: u32, start_col: u32) -> Result<Token, SyntaxError> {
        self.advance(); // consume opening "
        let mut value = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(SyntaxError {
                        line: start_line as usize,
                        column: start_col as usize,
                        message: "Unterminated raw string literal".to_string(),
                    });
                }
                Some(b'"') => { self.advance(); break; }
                _ => {
                    if let Some(ch) = self.advance_char() {
                        value.push(ch);
                    }
                }
            }
        }
        Ok(Token {
            kind: TokenKind::StringLiteral,
            span: Span::new(start_line, start_col, self.line, self.col - 1),
            text: value,
        })
    }

    fn lex_fstring(&mut self, start_line: u32, start_col: u32) -> Result<Token, SyntaxError> {
        self.advance(); // consume opening "
        let mut value = String::new();
        let mut brace_depth = 0;
        loop {
            match self.peek() {
                None => {
                    return Err(SyntaxError {
                        line: start_line as usize,
                        column: start_col as usize,
                        message: "Unterminated f-string literal".to_string(),
                    });
                }
                Some(b'"') if brace_depth == 0 => { self.advance(); break; }
                Some(b'{') => {
                    self.advance();
                    brace_depth += 1;
                    value.push('{');
                }
                Some(b'}') => {
                    self.advance();
                    if brace_depth > 0 {
                        brace_depth -= 1;
                    } else {
                        return Err(SyntaxError {
                            line: self.line as usize,
                            column: self.col as usize,
                            message: "Unmatched '}' in f-string; use '\\}' for a literal brace".to_string(),
                        });
                    }
                    value.push('}');
                }
                Some(b'\\') if brace_depth == 0 => {
                    self.advance(); // consume backslash
                    // In f-strings outside interpolation, \{ and \} must not produce
                    // literal braces (which would be confused with interpolation markers).
                    // Use sentinel chars that the parser converts back.
                    match self.peek() {
                        Some(b'{') => { self.advance(); value.push('\u{FFF0}'); }
                        Some(b'}') => { self.advance(); value.push('\u{FFF1}'); }
                        _ => {
                            let ch = self.parse_escape_sequence()?;
                            value.push(ch);
                        }
                    }
                }
                Some(b'\\') => {
                    // Inside interpolation: keep backslash escapes verbatim
                    // so the inner tokenizer can process them.
                    self.advance();
                    value.push('\\');
                    if let Some(ch) = self.advance_char() {
                        value.push(ch);
                    }
                }
                Some(q @ b'"') | Some(q @ b'\'') if brace_depth > 0 => {
                    // Inside interpolation: skip over string literals so braces within
                    // them don't affect our depth tracking.
                    let quote = q;
                    self.advance();
                    value.push(quote as char);
                    loop {
                        match self.peek() {
                            None => break,
                            Some(b'\\') => {
                                self.advance();
                                value.push('\\');
                                if let Some(ch) = self.advance_char() {
                                    value.push(ch);
                                }
                            }
                            Some(c) if c == quote => {
                                self.advance();
                                value.push(quote as char);
                                break;
                            }
                            _ => {
                                if let Some(ch) = self.advance_char() {
                                    value.push(ch);
                                }
                            }
                        }
                    }
                }
                _ => {
                    if let Some(ch) = self.advance_char() {
                        value.push(ch);
                    }
                }
            }
        }
        Ok(Token {
            kind: TokenKind::FStringStart,
            span: Span::new(start_line, start_col, self.line, self.col - 1),
            text: value,
        })
    }

    fn lex_number(&mut self, start_line: u32, start_col: u32) -> Result<Token, SyntaxError> {
        let start = self.pos;

        // Check for base prefixes: 0x, 0o, 0b
        if self.peek() == Some(b'0') {
            if let Some(prefix) = self.peek_at(1) {
                match prefix {
                    b'x' | b'X' => return self.lex_int_with_base(start, start_line, start_col, 16, |c| c.is_ascii_hexdigit()),
                    b'o' | b'O' => return self.lex_int_with_base(start, start_line, start_col, 8, |c| matches!(c, b'0'..=b'7')),
                    b'b' | b'B' => return self.lex_int_with_base(start, start_line, start_col, 2, |c| c == b'0' || c == b'1'),
                    _ => {}
                }
            }
        }

        let mut is_float = false;

        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                self.advance();
            } else if ch == b'_' && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
                // Underscore separator: only valid between digits
                self.advance();
            } else if ch == b'.' && !is_float {
                // Check if next char after dot is a digit (to avoid 1.method())
                if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
                    is_float = true;
                    self.advance(); // consume .
                } else {
                    break;
                }
            } else if ch == b'e' || ch == b'E' {
                // Require at least one digit after the exponent (with optional sign)
                let next = self.peek_at(1);
                let has_digit_after = next.is_some_and(|c| c.is_ascii_digit())
                    || ((next == Some(b'+') || next == Some(b'-'))
                        && self.peek_at(2).is_some_and(|c| c.is_ascii_digit()));
                if !has_digit_after {
                    break; // Reject malformed exponents like `1e+`, `1e`
                }
                is_float = true;
                self.advance();
                // Optional sign after exponent
                if self.peek() == Some(b'+') || self.peek() == Some(b'-') {
                    self.advance();
                }
            } else {
                break;
            }
        }

        // Strip underscores from the token text for downstream parsing
        let raw = String::from_utf8_lossy(&self.source[start..self.pos]).to_string();
        let text = raw.replace('_', "");
        let kind = if is_float {
            TokenKind::FloatLiteral
        } else {
            TokenKind::IntLiteral
        };

        Ok(Token {
            kind,
            span: Span::new(start_line, start_col, self.line, self.col - 1),
            text,
        })
    }

    fn lex_int_with_base(&mut self, _start: usize, start_line: u32, start_col: u32, base: u32, is_valid_digit: fn(u8) -> bool) -> Result<Token, SyntaxError> {
        self.advance(); // consume '0'
        self.advance(); // consume prefix letter (x/o/b)

        let digit_start = self.pos;
        while let Some(ch) = self.peek() {
            if is_valid_digit(ch) {
                self.advance();
            } else if ch == b'_' && self.peek_at(1).is_some_and(|c| is_valid_digit(c)) {
                self.advance(); // underscore between digits
            } else {
                break;
            }
        }

        if self.pos == digit_start {
            return Err(SyntaxError {
                line: start_line as usize,
                column: start_col as usize,
                message: format!("Expected digits after base prefix"),
            });
        }

        let digits: String = String::from_utf8_lossy(&self.source[digit_start..self.pos])
            .chars()
            .filter(|c| *c != '_')
            .collect();
        let value = i64::from_str_radix(&digits, base).map_err(|_| SyntaxError {
            line: start_line as usize,
            column: start_col as usize,
            message: format!("Invalid numeric literal"),
        })?;

        Ok(Token {
            kind: TokenKind::IntLiteral,
            span: Span::new(start_line, start_col, self.line, self.col - 1),
            text: value.to_string(), // store decimal value for downstream parsers
        })
    }

    fn lex_identifier(&mut self, start_line: u32, start_col: u32) -> Result<Token, SyntaxError> {
        let start = self.pos;

        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                self.advance();
            } else if ch == b'-' {
                // Allow hyphens in identifiers only if followed by an alphabetic
                // char (for plugin IDs like "text-llm"), not digits (x-5 is subtraction)
                if self.peek_at(1).is_some_and(|c| c.is_ascii_alphabetic()) {
                    self.advance();
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        let text = String::from_utf8_lossy(&self.source[start..self.pos]).to_string();

        // Check for standalone underscore placeholder
        if text == "_" {
            return Ok(Token {
                kind: TokenKind::Underscore,
                span: Span::new(start_line, start_col, self.line, self.col - 1),
                text,
            });
        }

        let kind = match text.as_str() {
            "let" => TokenKind::Let,
            "mut" => TokenKind::Mut,
            "import" => TokenKind::Import,
            "output" => TokenKind::Output,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "while" => TokenKind::While,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "null" => TokenKind::Null,
            "fn" => TokenKind::Fn,
            "async" => TokenKind::Async,
            "await" => TokenKind::Await,
            "spawn" => TokenKind::Spawn,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "return" => TokenKind::Return,
            "match" => TokenKind::Match,
            "use" => TokenKind::Use,
            "mod" => TokenKind::Mod,
            "const" => TokenKind::Const,
            "type" => TokenKind::Type,
            "as" => TokenKind::As,
            "pub" => TokenKind::Pub,
            "loop" => TokenKind::Loop,
            "try" => TokenKind::Try,
            "catch" => TokenKind::Catch,
            "finally" => TokenKind::Finally,
            "throw" => TokenKind::Throw,
            "test" => TokenKind::Test,
            "enum" => TokenKind::Enum,
            "struct" => TokenKind::Struct,
            s if is_reserved_keyword(s) => TokenKind::Reserved,
            _ => TokenKind::Ident,
        };

        Ok(Token {
            kind,
            span: Span::new(start_line, start_col, self.line, self.col - 1),
            text,
        })
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tok_kinds(source: &str) -> Vec<TokenKind> {
        tokenize(source)
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    fn tok_texts(source: &str) -> Vec<String> {
        tokenize(source)
            .unwrap()
            .into_iter()
            .map(|t| t.text)
            .collect()
    }

    // --- Keywords ---

    #[test]
    fn test_keywords() {
        let kinds = tok_kinds("let mut import output if else for in while true false null");
        assert_eq!(
            kinds,
            vec![
                TokenKind::Let,
                TokenKind::Mut,
                TokenKind::Import,
                TokenKind::Output,
                TokenKind::If,
                TokenKind::Else,
                TokenKind::For,
                TokenKind::In,
                TokenKind::While,
                TokenKind::True,
                TokenKind::False,
                TokenKind::Null,
                TokenKind::Eof,
            ]
        );
    }

    // --- Identifiers ---

    #[test]
    fn test_identifiers() {
        let tokens = tokenize("foo bar_baz _x x123").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Ident);
        assert_eq!(tokens[0].text, "foo");
        assert_eq!(tokens[1].text, "bar_baz");
        assert_eq!(tokens[2].text, "_x");
        assert_eq!(tokens[3].text, "x123");
    }

    #[test]
    fn test_hyphenated_identifier() {
        let tokens = tokenize("text-llm").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Ident);
        assert_eq!(tokens[0].text, "text-llm");
    }

    #[test]
    fn test_identifier_minus_number() {
        // `x-5` should be identifier `x`, minus, int `5`
        let kinds = tok_kinds("x-5");
        assert_eq!(
            kinds,
            vec![
                TokenKind::Ident,
                TokenKind::Minus,
                TokenKind::IntLiteral,
                TokenKind::Eof
            ]
        );
    }

    // --- Operators ---

    #[test]
    fn test_arithmetic_operators() {
        let kinds = tok_kinds("+ - * / %");
        assert_eq!(
            kinds,
            vec![
                TokenKind::Plus,
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Percent,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_comparison_operators() {
        let kinds = tok_kinds("== != > < >= <=");
        assert_eq!(
            kinds,
            vec![
                TokenKind::EqEq,
                TokenKind::NotEq,
                TokenKind::Gt,
                TokenKind::Lt,
                TokenKind::GtEq,
                TokenKind::LtEq,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_logical_operators() {
        let kinds = tok_kinds("&& || !");
        assert_eq!(
            kinds,
            vec![
                TokenKind::AndAnd,
                TokenKind::PipePipe,
                TokenKind::Bang,
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_assignment_vs_equality() {
        let kinds = tok_kinds("= ==");
        assert_eq!(kinds, vec![TokenKind::Eq, TokenKind::EqEq, TokenKind::Eof]);
    }

    #[test]
    fn test_pipe_operator() {
        let kinds = tok_kinds("|>");
        assert_eq!(kinds, vec![TokenKind::Pipe, TokenKind::Eof]);
    }

    // --- Delimiters ---

    #[test]
    fn test_delimiters() {
        let kinds = tok_kinds("( ) [ ] { } : ; , .");
        assert_eq!(
            kinds,
            vec![
                TokenKind::LParen,
                TokenKind::RParen,
                TokenKind::LBracket,
                TokenKind::RBracket,
                TokenKind::LBrace,
                TokenKind::RBrace,
                TokenKind::Colon,
                TokenKind::Semicolon,
                TokenKind::Comma,
                TokenKind::Dot,
                TokenKind::Eof,
            ]
        );
    }

    // --- Number literals ---

    #[test]
    fn test_integer_literal() {
        let tokens = tokenize("42").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::IntLiteral);
        assert_eq!(tokens[0].text, "42");
    }

    #[test]
    fn test_float_literal() {
        let tokens = tokenize("3.14").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::FloatLiteral);
        assert_eq!(tokens[0].text, "3.14");
    }

    #[test]
    fn test_float_exponent() {
        let tokens = tokenize("1e10").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::FloatLiteral);
        assert_eq!(tokens[0].text, "1e10");
    }

    #[test]
    fn test_float_exponent_neg() {
        let tokens = tokenize("2.5e-3").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::FloatLiteral);
        assert_eq!(tokens[0].text, "2.5e-3");
    }

    #[test]
    fn test_multiple_numbers() {
        let texts = tok_texts("10 20.5 300");
        assert_eq!(texts, vec!["10", "20.5", "300", ""]);
    }

    // --- String literals ---

    #[test]
    fn test_string_literal() {
        let tokens = tokenize(r#""hello world""#).unwrap();
        assert_eq!(tokens[0].kind, TokenKind::StringLiteral);
        assert_eq!(tokens[0].text, "hello world");
    }

    #[test]
    fn test_string_escapes() {
        let tokens = tokenize(r#""line1\nline2\ttab\\slash\"quote""#).unwrap();
        assert_eq!(tokens[0].kind, TokenKind::StringLiteral);
        assert_eq!(tokens[0].text, "line1\nline2\ttab\\slash\"quote");
    }

    #[test]
    fn test_empty_string() {
        let tokens = tokenize(r#""""#).unwrap();
        assert_eq!(tokens[0].kind, TokenKind::StringLiteral);
        assert_eq!(tokens[0].text, "");
    }

    #[test]
    fn test_unterminated_string() {
        let err = tokenize(r#""unclosed"#).unwrap_err();
        assert!(err.message.contains("Unterminated"));
    }

    // --- Comments ---

    #[test]
    fn test_line_comment() {
        let kinds = tok_kinds("let x // this is a comment\nlet y");
        assert_eq!(
            kinds,
            vec![
                TokenKind::Let,
                TokenKind::Ident,
                TokenKind::Let,
                TokenKind::Ident,
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_comment_only() {
        let kinds = tok_kinds("// just a comment");
        assert_eq!(kinds, vec![TokenKind::Eof]);
    }

    // --- Underscore placeholder ---

    #[test]
    fn test_underscore_placeholder() {
        let kinds = tok_kinds("_");
        assert_eq!(kinds, vec![TokenKind::Underscore, TokenKind::Eof]);
    }

    #[test]
    fn test_underscore_prefix_ident() {
        let tokens = tokenize("_foo").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Ident);
        assert_eq!(tokens[0].text, "_foo");
    }

    // --- Spans ---

    #[test]
    fn test_span_single_line() {
        let tokens = tokenize("let x = 42;").unwrap();
        assert_eq!(tokens[0].span.start_line, 1);
        assert_eq!(tokens[0].span.start_col, 1);
    }

    #[test]
    fn test_span_multi_line() {
        let tokens = tokenize("let x = 10;\nlet y = 20;").unwrap();
        // 'let' on line 2
        // Tokens: let(0) x(1) =(2) 10(3) ;(4) let(5) y(6) =(7) 20(8) ;(9) EOF(10)
        assert_eq!(tokens[5].span.start_line, 2); // 'let' on line 2
        assert_eq!(tokens[5].span.start_col, 1);
    }

    // --- Compound expressions ---

    #[test]
    fn test_full_let_statement() {
        let kinds = tok_kinds("let x: int64 = 42;");
        assert_eq!(
            kinds,
            vec![
                TokenKind::Let,
                TokenKind::Ident,
                TokenKind::Colon,
                TokenKind::Ident,
                TokenKind::Eq,
                TokenKind::IntLiteral,
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_infix_expression() {
        let kinds = tok_kinds("a + b * c");
        assert_eq!(
            kinds,
            vec![
                TokenKind::Ident,
                TokenKind::Plus,
                TokenKind::Ident,
                TokenKind::Star,
                TokenKind::Ident,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_if_else_expression() {
        let kinds = tok_kinds("if x > 0 { x } else { 0 }");
        assert_eq!(
            kinds,
            vec![
                TokenKind::If,
                TokenKind::Ident,
                TokenKind::Gt,
                TokenKind::IntLiteral,
                TokenKind::LBrace,
                TokenKind::Ident,
                TokenKind::RBrace,
                TokenKind::Else,
                TokenKind::LBrace,
                TokenKind::IntLiteral,
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_for_loop() {
        let kinds = tok_kinds("for item in items { }");
        assert_eq!(
            kinds,
            vec![
                TokenKind::For,
                TokenKind::Ident,
                TokenKind::In,
                TokenKind::Ident,
                TokenKind::LBrace,
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_import_statement() {
        let kinds = tok_kinds(r#"import "capture";"#);
        assert_eq!(
            kinds,
            vec![
                TokenKind::Import,
                TokenKind::StringLiteral,
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_pipe_expression() {
        let kinds = tok_kinds("x |> add(_, 5)");
        assert_eq!(
            kinds,
            vec![
                TokenKind::Ident,
                TokenKind::Pipe,
                TokenKind::Ident,
                TokenKind::LParen,
                TokenKind::Underscore,
                TokenKind::Comma,
                TokenKind::IntLiteral,
                TokenKind::RParen,
                TokenKind::Eof,
            ]
        );
    }

    // --- Error cases ---

    #[test]
    fn test_invalid_char() {
        let err = tokenize("let x = @;").unwrap_err();
        assert!(err.message.contains("Unexpected character"));
    }

    #[test]
    fn test_unicode_error_message() {
        let err = tokenize("let é = 5;").unwrap_err();
        assert!(err.message.contains('é'), "Expected Unicode char in error, got: {}", err.message);
    }

    #[test]
    fn test_single_ampersand() {
        let err = tokenize("&x").unwrap_err();
        assert!(err.message.contains("&&"));
    }

    #[test]
    fn test_invalid_escape() {
        let err = tokenize(r#""\q""#).unwrap_err();
        assert!(err.message.contains("Invalid escape"));
    }

    // --- Function tokens ---

    #[test]
    fn test_fn_keyword() {
        let kinds = tok_kinds("fn");
        assert_eq!(kinds, vec![TokenKind::Fn, TokenKind::Eof]);
    }

    #[test]
    fn test_fn_name_stays_ident() {
        let tokens = tokenize("fn_name").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Ident);
        assert_eq!(tokens[0].text, "fn_name");
    }

    #[test]
    fn test_arrow_token() {
        let kinds = tok_kinds("->");
        assert_eq!(kinds, vec![TokenKind::Arrow, TokenKind::Eof]);
    }

    #[test]
    fn test_minus_alone_stays_minus() {
        let kinds = tok_kinds("- x");
        assert_eq!(
            kinds,
            vec![TokenKind::Minus, TokenKind::Ident, TokenKind::Eof]
        );
    }

    // --- Async/Await/Spawn tokens ---

    #[test]
    fn test_async_token() {
        let kinds = tok_kinds("async");
        assert_eq!(kinds, vec![TokenKind::Async, TokenKind::Eof]);
    }

    #[test]
    fn test_await_token() {
        let kinds = tok_kinds("await");
        assert_eq!(kinds, vec![TokenKind::Await, TokenKind::Eof]);
    }

    #[test]
    fn test_spawn_token() {
        let kinds = tok_kinds("spawn");
        assert_eq!(kinds, vec![TokenKind::Spawn, TokenKind::Eof]);
    }

    #[test]
    fn test_reserved_keyword_produces_reserved_token() {
        for &kw in RESERVED_KEYWORDS {
            let kinds = tok_kinds(kw);
            assert_eq!(
                kinds,
                vec![TokenKind::Reserved, TokenKind::Eof],
                "Expected Reserved for '{}'",
                kw
            );
        }
    }

    #[test]
    fn test_non_reserved_ident_stays_ident() {
        let tokens = tokenize("spawn_task").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Ident);
        assert_eq!(tokens[0].text, "spawn_task");
    }

    // --- Malformed float exponent rejection (Task #13 fix) ---

    #[test]
    fn test_float_valid_exponent() {
        let tokens = tokenize("1e10").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::FloatLiteral);
        assert_eq!(tokens[0].text, "1e10");
    }

    #[test]
    fn test_float_valid_positive_exponent() {
        let tokens = tokenize("2.5e+3").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::FloatLiteral);
        assert_eq!(tokens[0].text, "2.5e+3");
    }

    #[test]
    fn test_float_valid_negative_exponent() {
        let tokens = tokenize("3.14e-2").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::FloatLiteral);
        assert_eq!(tokens[0].text, "3.14e-2");
    }

    #[test]
    fn test_float_malformed_exponent_no_digits() {
        // `1e` should lex as IntLiteral "1" followed by Ident "e"
        let tokens = tokenize("1e").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::IntLiteral);
        assert_eq!(tokens[0].text, "1");
        assert_eq!(tokens[1].kind, TokenKind::Ident);
        assert_eq!(tokens[1].text, "e");
    }

    #[test]
    fn test_float_malformed_exponent_sign_no_digits() {
        // `1e+` should lex as IntLiteral "1" followed by Ident "e" and Plus
        let tokens = tokenize("1e+").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::IntLiteral);
        assert_eq!(tokens[0].text, "1");
        assert_eq!(tokens[1].kind, TokenKind::Ident);
        assert_eq!(tokens[1].text, "e");
        assert_eq!(tokens[2].kind, TokenKind::Plus);
    }

    #[test]
    fn test_float_malformed_exponent_negative_sign_no_digits() {
        // `2.5e-` should NOT include the malformed exponent
        let tokens = tokenize("2.5e-").unwrap();
        // "2.5" should be a float, then "e" ident, then "-" minus
        assert_eq!(tokens[0].kind, TokenKind::FloatLiteral);
        assert_eq!(tokens[0].text, "2.5");
    }

    #[test]
    fn test_fn_definition_tokens() {
        let kinds = tok_kinds("fn double(x: int64) -> int64 { x * 2 }");
        assert_eq!(
            kinds,
            vec![
                TokenKind::Fn,
                TokenKind::Ident, // double
                TokenKind::LParen,
                TokenKind::Ident, // x
                TokenKind::Colon,
                TokenKind::Ident, // int64
                TokenKind::RParen,
                TokenKind::Arrow,
                TokenKind::Ident, // int64
                TokenKind::LBrace,
                TokenKind::Ident, // x
                TokenKind::Star,
                TokenKind::IntLiteral,
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_test_keyword_token() {
        let kinds = tok_kinds("test");
        assert_eq!(kinds, vec![TokenKind::Test, TokenKind::Eof]);
    }

    #[test]
    fn test_test_keyword_in_context() {
        // "test" followed by a string literal — the expected usage
        let tokens = tokenize(r#"test "my test" { }"#).unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Test);
        assert_eq!(tokens[1].kind, TokenKind::StringLiteral);
        assert_eq!(tokens[1].text, "my test");
        assert_eq!(tokens[2].kind, TokenKind::LBrace);
        assert_eq!(tokens[3].kind, TokenKind::RBrace);
    }
}
