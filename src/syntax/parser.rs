//! Recursive descent parser for the MAGI v2 language.
//!
//! Parses a token stream into an AST. Uses precedence climbing for
//! infix operator expressions.

use super::ast::*;
use super::lexer::{is_reserved_keyword, Token, TokenKind};
use super::SyntaxError;

// =============================================================================
// Parser
// =============================================================================

/// Maximum nesting depth for expressions (prevents stack overflow on pathological input).
/// Each parenthesized sub-expression adds ~2 depth units (binary_expr + unary_expr),
/// so this effectively limits paren nesting to ~64 levels.
const MAX_PARSE_DEPTH: usize = 128;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
        }
    }

    fn enter_depth(&mut self) -> Result<(), SyntaxError> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            let tok = self.peek();
            Err(SyntaxError {
                line: tok.span.start_line as usize,
                column: tok.span.start_col as usize,
                message: format!(
                    "Expression nesting exceeds maximum depth ({})",
                    MAX_PARSE_DEPTH
                ),
            })
        } else {
            Ok(())
        }
    }

    fn exit_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // =========================================================================
    // Token navigation
    // =========================================================================

    fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.peek_kind() == kind
    }

    /// Peek at the token after the current one.
    fn peek_next_kind(&self) -> &TokenKind {
        let next = (self.pos + 1).min(self.tokens.len() - 1);
        &self.tokens[next].kind
    }

    /// Check if the current position starts an assignment or compound assignment.
    /// Looks for `ident =` (not `==`) or `ident +=/-=/etc.`
    fn is_assignment_start(&self) -> bool {
        if !matches!(self.peek_kind(), TokenKind::Ident) {
            return false;
        }
        matches!(
            self.peek_next_kind(),
            TokenKind::Eq
                | TokenKind::PlusEq
                | TokenKind::MinusEq
                | TokenKind::StarEq
                | TokenKind::SlashEq
                | TokenKind::PercentEq
        )
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos.min(self.tokens.len() - 1)];
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<Token, SyntaxError> {
        if self.peek_kind() == kind {
            Ok(self.advance().clone())
        } else {
            let tok = self.peek();
            Err(SyntaxError {
                line: tok.span.start_line as usize,
                column: tok.span.start_col as usize,
                message: format!("Expected '{}', got '{}'", kind, tok.kind),
            })
        }
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn error(&self, msg: &str) -> SyntaxError {
        let tok = self.peek();
        SyntaxError {
            line: tok.span.start_line as usize,
            column: tok.span.start_col as usize,
            message: msg.to_string(),
        }
    }

    /// Expect an identifier token and validate it is not a reserved keyword.
    fn expect_identifier(&mut self) -> Result<Token, SyntaxError> {
        let tok = self.peek().clone();
        if tok.kind == TokenKind::Reserved {
            return Err(SyntaxError {
                line: tok.span.start_line as usize,
                column: tok.span.start_col as usize,
                message: format!(
                    "'{}' is a reserved keyword and cannot be used as an identifier",
                    tok.text
                ),
            });
        }
        if tok.kind == TokenKind::Async
            || tok.kind == TokenKind::Await
            || tok.kind == TokenKind::Spawn
        {
            return Err(SyntaxError {
                line: tok.span.start_line as usize,
                column: tok.span.start_col as usize,
                message: format!(
                    "'{}' is a keyword and cannot be used as an identifier",
                    tok.text
                ),
            });
        }
        let ident = self.expect(&TokenKind::Ident)?;
        if is_reserved_keyword(&ident.text) {
            return Err(SyntaxError {
                line: ident.span.start_line as usize,
                column: ident.span.start_col as usize,
                message: format!(
                    "'{}' is a reserved keyword and cannot be used as an identifier",
                    ident.text
                ),
            });
        }
        Ok(ident)
    }

    // =========================================================================
    // Program
    // =========================================================================

    pub fn parse_program(&mut self) -> Result<Program, SyntaxError> {
        let start = self.peek().span;
        let mut statements = Vec::new();

        while !self.at(&TokenKind::Eof) {
            statements.push(self.parse_statement()?);
        }

        let end = self.peek().span;
        Ok(Program {
            statements,
            span: start.merge(end),
        })
    }

    // =========================================================================
    // Statements
    // =========================================================================

    fn parse_statement(&mut self) -> Result<Statement, SyntaxError> {
        let start = self.peek().span;

        match self.peek_kind().clone() {
            TokenKind::Import => self.parse_import_statement(start),
            TokenKind::Output => self.parse_output_statement(start),
            TokenKind::Let => self.parse_let_statement(start),
            TokenKind::Const => self.parse_const_statement(start),
            TokenKind::Fn => self.parse_function_def(start),
            TokenKind::Async => self.parse_async_function_def(start),
            TokenKind::For => self.parse_for_loop(start),
            TokenKind::While => self.parse_while_loop(start),
            TokenKind::Break => self.parse_break_statement(start),
            TokenKind::Continue => self.parse_continue_statement(start),
            TokenKind::Return => self.parse_return_statement(start),
            TokenKind::Try => self.parse_try_catch_statement(start),
            TokenKind::Throw => self.parse_throw_statement(start),
            TokenKind::Mod => self.parse_mod_statement(start),
            TokenKind::Use => self.parse_use_statement(start),
            TokenKind::Type => self.parse_type_alias(start),
            TokenKind::Test => self.parse_test_def(start),
            TokenKind::Enum => self.parse_enum_def(start),
            TokenKind::Struct => self.parse_struct_def(start),
            TokenKind::Pub => {
                // pub fn / pub mod — consume pub, then parse inner
                self.advance(); // consume 'pub'
                self.parse_statement()
            }
            TokenKind::Ident => {
                // Could be assignment (x = ...), compound assign (x += ...), or expression
                self.parse_assignment_or_expr_statement(start)
            }
            _ => self.parse_expr_statement(start),
        }
    }

    fn parse_import_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'import'
        let name_tok = self.expect(&TokenKind::StringLiteral)?;
        if name_tok.text.is_empty() {
            return Err(SyntaxError {
                line: name_tok.span.start_line as usize,
                column: name_tok.span.start_col as usize,
                message: "Import plugin ID cannot be empty".to_string(),
            });
        }
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon); // optional semicolon
        Ok(Statement {
            kind: StatementKind::Import(name_tok.text),
            span: start.merge(end),
        })
    }

    fn parse_output_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'output'
        let expr = self.parse_expression()?;
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon); // optional semicolon
        Ok(Statement {
            kind: StatementKind::Output(expr),
            span: start.merge(end),
        })
    }

    fn parse_let_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'let'

        let is_mut = self.eat(&TokenKind::Mut);

        // Check for destructuring: let [a, b] = expr; or let {x, y} = expr;
        if self.at(&TokenKind::LBracket) {
            return self.parse_let_destructure_array(start, is_mut);
        }
        if self.at(&TokenKind::LBrace) {
            return self.parse_let_destructure_map(start, is_mut);
        }

        let name_tok = self.expect_identifier()?;
        let name = name_tok.text;

        // Optional type annotation
        let type_annotation = if self.eat(&TokenKind::Colon) {
            let type_tok = self.expect(&TokenKind::Ident)?;
            Some(type_tok.text)
        } else {
            None
        };

        self.expect(&TokenKind::Eq)?;
        let value = self.parse_expression()?;
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon); // optional semicolon

        let kind = if is_mut {
            StatementKind::LetMut {
                name,
                type_annotation,
                value,
            }
        } else {
            StatementKind::Let {
                name,
                type_annotation,
                value,
            }
        };

        Ok(Statement {
            kind,
            span: start.merge(end),
        })
    }

    fn parse_let_destructure_array(
        &mut self,
        start: Span,
        mutable: bool,
    ) -> Result<Statement, SyntaxError> {
        self.advance(); // consume '['
        let mut elements = Vec::new();
        while !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
            if self.at(&TokenKind::DotDotDot) {
                self.advance(); // consume '...'
                let rest_name = self.expect_identifier()?;
                elements.push(DestructureElement::Rest(rest_name.text));
                break; // rest must be last element
            } else {
                let name = self.expect_identifier()?;
                elements.push(DestructureElement::Name(name.text));
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBracket)?;
        self.expect(&TokenKind::Eq)?;
        let value = self.parse_expression()?;
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon);
        Ok(Statement {
            kind: StatementKind::LetDestructure {
                pattern: DestructurePattern::Array(elements),
                mutable,
                value,
            },
            span: start.merge(end),
        })
    }

    fn parse_let_destructure_map(
        &mut self,
        start: Span,
        mutable: bool,
    ) -> Result<Statement, SyntaxError> {
        self.advance(); // consume '{'
        let mut entries = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let key = self.expect_identifier()?;
            // Optional alias: {x: alias}
            let alias = if self.eat(&TokenKind::Colon) {
                let alias_tok = self.expect_identifier()?;
                Some(alias_tok.text)
            } else {
                None
            };
            entries.push((key.text, alias));
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBrace)?;
        self.expect(&TokenKind::Eq)?;
        let value = self.parse_expression()?;
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon);
        Ok(Statement {
            kind: StatementKind::LetDestructure {
                pattern: DestructurePattern::Map(entries),
                mutable,
                value,
            },
            span: start.merge(end),
        })
    }

    fn parse_for_loop(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'for'

        let pattern = if self.at(&TokenKind::LBracket) {
            // Array destructure: for [a, b] in ...
            self.advance(); // consume [
            let mut elements = Vec::new();
            while !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
                if self.at(&TokenKind::DotDotDot) {
                    self.advance();
                    let rest_tok = self.expect_identifier()?;
                    elements.push(DestructureElement::Rest(rest_tok.text));
                    // Rest must be last
                    break;
                }
                let name_tok = self.expect_identifier()?;
                elements.push(DestructureElement::Name(name_tok.text));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RBracket)?;
            ForPattern::ArrayDestructure(elements)
        } else if self.at(&TokenKind::LBrace) {
            // Map destructure: for {k, v} in ... or for {k: alias} in ...
            self.advance(); // consume {
            let mut entries = Vec::new();
            while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                let key_tok = self.expect_identifier()?;
                let alias = if self.eat(&TokenKind::Colon) {
                    let alias_tok = self.expect_identifier()?;
                    Some(alias_tok.text)
                } else {
                    None
                };
                entries.push((key_tok.text, alias));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RBrace)?;
            ForPattern::MapDestructure(entries)
        } else {
            let var_tok = self.expect_identifier()?;
            ForPattern::Single(var_tok.text)
        };

        self.expect(&TokenKind::In)?;
        let iterable = self.parse_expression()?;
        let body = self.parse_block()?;

        Ok(Statement {
            span: start.merge(body.span),
            kind: StatementKind::ForLoop {
                pattern,
                iterable,
                body,
            },
        })
    }

    fn parse_while_loop(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'while'
        let condition = self.parse_expression()?;
        let body = self.parse_block()?;

        Ok(Statement {
            span: start.merge(body.span),
            kind: StatementKind::WhileLoop { condition, body },
        })
    }

    fn parse_break_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'break'
                        // Optional value expression (e.g. `break 42;`)
        let value = if !self.at(&TokenKind::Semicolon) && !self.at(&TokenKind::RBrace) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon);
        Ok(Statement {
            kind: StatementKind::Break(value),
            span: start.merge(end),
        })
    }

    fn parse_continue_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'continue'
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon);
        Ok(Statement {
            kind: StatementKind::Continue,
            span: start.merge(end),
        })
    }

    fn parse_return_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'return'
                        // Optional return value
        let value = if !self.at(&TokenKind::Semicolon) && !self.at(&TokenKind::RBrace) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon);
        Ok(Statement {
            kind: StatementKind::Return(value),
            span: start.merge(end),
        })
    }

    fn parse_const_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'const'
        let name_tok = self.expect_identifier()?;
        let name = name_tok.text;

        let type_annotation = if self.eat(&TokenKind::Colon) {
            let type_tok = self.expect(&TokenKind::Ident)?;
            Some(type_tok.text)
        } else {
            None
        };

        self.expect(&TokenKind::Eq)?;
        let value = self.parse_expression()?;
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon);
        Ok(Statement {
            kind: StatementKind::ConstDef {
                name,
                type_annotation,
                value,
            },
            span: start.merge(end),
        })
    }

    fn parse_try_catch_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'try'
        let try_block = self.parse_block()?;

        self.expect(&TokenKind::Catch)?;

        // Optional catch variable: catch err { ... } or catch { ... }
        let catch_var = if self.at(&TokenKind::Ident) {
            let tok = self.advance().clone();
            Some(tok.text)
        } else {
            None
        };

        let catch_block = self.parse_block()?;

        let finally_block = if self.eat(&TokenKind::Finally) {
            Some(self.parse_block()?)
        } else {
            None
        };

        let end = finally_block
            .as_ref()
            .map(|b| b.span)
            .unwrap_or(catch_block.span);

        Ok(Statement {
            kind: StatementKind::TryCatch {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            },
            span: start.merge(end),
        })
    }

    fn parse_throw_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'throw'
        let expr = self.parse_expression()?;
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon);
        Ok(Statement {
            kind: StatementKind::Throw(expr),
            span: start.merge(end),
        })
    }

    fn parse_mod_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'mod'
        let name_tok = self.expect_identifier()?;
        let body = self.parse_block()?;
        let end_span = body.span;
        Ok(Statement {
            kind: StatementKind::ModuleDef {
                name: name_tok.text,
                body,
            },
            span: start.merge(end_span),
        })
    }

    fn parse_use_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'use'

        // Parse path: std::math::sqrt or std::array::*
        let mut path = Vec::new();
        let first = self.expect_identifier()?;
        path.push(first.text);

        let mut glob = false;
        while self.eat(&TokenKind::ColonColon) {
            if self.eat(&TokenKind::Star) {
                glob = true;
                break;
            }
            let seg = self.expect_identifier()?;
            path.push(seg.text);
        }

        // Optional alias: `use std::math::sqrt as s;`
        let alias = if self.eat(&TokenKind::As) {
            let alias_tok = self.expect_identifier()?;
            Some(alias_tok.text)
        } else {
            None
        };

        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon);
        Ok(Statement {
            kind: StatementKind::Use { path, alias, glob },
            span: start.merge(end),
        })
    }

    fn parse_type_alias(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'type'
        let name_tok = self.expect_identifier()?;
        self.expect(&TokenKind::Eq)?;
        let target_tok = self.expect(&TokenKind::Ident)?;
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon);
        Ok(Statement {
            kind: StatementKind::TypeAlias {
                name: name_tok.text,
                target: target_tok.text,
            },
            span: start.merge(end),
        })
    }

    fn parse_test_def(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'test'

        // Expect a string literal for the test name
        let name_tok = self.expect(&TokenKind::StringLiteral)?;
        let name = name_tok.text;

        // Parse the test body block
        let body = self.parse_block()?;
        let end = self.peek().span;

        Ok(Statement {
            kind: StatementKind::TestDef { name, body },
            span: start.merge(end),
        })
    }

    fn parse_function_params(&mut self, end_token: &TokenKind) -> Result<Vec<FunctionParam>, SyntaxError> {
        let mut params = Vec::new();
        while !self.at(end_token) && !self.at(&TokenKind::Eof) {
            let param_start = self.peek().span;
            let is_rest = self.eat(&TokenKind::DotDotDot);
            let param_tok = self.expect_identifier()?;
            let type_annotation = if self.eat(&TokenKind::Colon) {
                let type_tok = self.expect(&TokenKind::Ident)?;
                Some(type_tok.text)
            } else {
                None
            };
            let default = if !is_rest && self.eat(&TokenKind::Eq) {
                Some(self.parse_expression()?)
            } else {
                None
            };
            let param_end = self.peek().span;
            params.push(FunctionParam {
                name: param_tok.text,
                type_annotation,
                default,
                rest: is_rest,
                span: param_start.merge(param_end),
            });
            if is_rest {
                // Rest param must be last
                break;
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        Ok(params)
    }

    fn parse_function_def(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'fn'
        let name_tok = self.expect_identifier()?;
        let name = name_tok.text;

        self.expect(&TokenKind::LParen)?;
        let params = self.parse_function_params(&TokenKind::RParen)?;
        self.expect(&TokenKind::RParen)?;

        let return_type = if self.eat(&TokenKind::Arrow) {
            let type_tok = self.expect(&TokenKind::Ident)?;
            Some(type_tok.text)
        } else {
            None
        };

        let body = self.parse_block()?;
        let full_span = start.merge(body.span);

        Ok(Statement {
            span: full_span,
            kind: StatementKind::FunctionDef(FunctionDef {
                name,
                params,
                return_type,
                body,
                span: full_span,
            }),
        })
    }

    fn parse_async_function_def(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'async'
        if !self.at(&TokenKind::Fn) {
            return Err(SyntaxError {
                line: self.peek().span.start_line as usize,
                column: self.peek().span.start_col as usize,
                message: "Expected 'fn' after 'async'".to_string(),
            });
        }
        self.advance(); // consume 'fn'
        let name_tok = self.expect_identifier()?;
        let name = name_tok.text;

        self.expect(&TokenKind::LParen)?;
        let params = self.parse_function_params(&TokenKind::RParen)?;
        self.expect(&TokenKind::RParen)?;

        let return_type = if self.eat(&TokenKind::Arrow) {
            let type_tok = self.expect(&TokenKind::Ident)?;
            Some(type_tok.text)
        } else {
            None
        };

        let body = self.parse_block()?;
        let full_span = start.merge(body.span);

        Ok(Statement {
            span: full_span,
            kind: StatementKind::AsyncFunctionDef(FunctionDef {
                name,
                params,
                return_type,
                body,
                span: full_span,
            }),
        })
    }

    fn parse_assignment_or_expr_statement(
        &mut self,
        start: Span,
    ) -> Result<Statement, SyntaxError> {
        // Peek ahead: if it's `ident =` (but not `==`), it's assignment
        // or if it's `ident +=/-=/etc`, it's compound assignment
        if self.peek_kind() == &TokenKind::Ident {
            let saved_pos = self.pos;
            let name_tok = self.advance().clone();

            if self.at(&TokenKind::Eq) {
                // Assignment: name = expr
                self.advance(); // consume '='
                let value = self.parse_expression()?;
                let end = self.peek().span;
                self.eat(&TokenKind::Semicolon);
                return Ok(Statement {
                    kind: StatementKind::Assignment {
                        name: name_tok.text,
                        value,
                    },
                    span: start.merge(end),
                });
            }

            // Compound assignment: name += expr, name -= expr, etc.
            let compound_op = match self.peek_kind() {
                TokenKind::PlusEq => Some(BinOp::Add),
                TokenKind::MinusEq => Some(BinOp::Sub),
                TokenKind::StarEq => Some(BinOp::Mul),
                TokenKind::SlashEq => Some(BinOp::Div),
                TokenKind::PercentEq => Some(BinOp::Mod),
                _ => None,
            };
            if let Some(op) = compound_op {
                self.advance(); // consume the compound operator
                let value = self.parse_expression()?;
                let end = self.peek().span;
                self.eat(&TokenKind::Semicolon);
                return Ok(Statement {
                    kind: StatementKind::CompoundAssign {
                        name: name_tok.text,
                        op,
                        value,
                    },
                    span: start.merge(end),
                });
            }

            // Not assignment — backtrack and parse as expression
            self.pos = saved_pos;
        }

        self.parse_expr_statement(start)
    }

    fn parse_expr_statement(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        let expr = self.parse_expression()?;
        let end = self.peek().span;
        self.eat(&TokenKind::Semicolon);
        Ok(Statement {
            kind: StatementKind::ExprStatement(expr),
            span: start.merge(end),
        })
    }

    // =========================================================================
    // Block
    // =========================================================================

    fn parse_block(&mut self) -> Result<Block, SyntaxError> {
        let start_tok = self.expect(&TokenKind::LBrace)?;
        let start = start_tok.span;
        let mut statements = Vec::new();
        let mut tail_expr = None;

        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let saved_pos = self.pos;

            // If the next thing is a keyword statement, parse it as a statement
            match self.peek_kind() {
                TokenKind::Let
                | TokenKind::Import
                | TokenKind::Output
                | TokenKind::For
                | TokenKind::While
                | TokenKind::Fn
                | TokenKind::Async
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Return
                | TokenKind::Try
                | TokenKind::Throw
                | TokenKind::Const
                | TokenKind::Mod
                | TokenKind::Use
                | TokenKind::Type
                | TokenKind::Pub
                | TokenKind::Enum
                | TokenKind::Struct
                | TokenKind::Test => {
                    statements.push(self.parse_statement()?);
                    continue;
                }
                // Identifiers followed by `=` or `+=`/etc. are assignments
                TokenKind::Ident if self.is_assignment_start() => {
                    statements.push(self.parse_statement()?);
                    continue;
                }
                _ => {}
            }

            // Try parsing as expression
            let expr = self.parse_expression()?;

            if self.eat(&TokenKind::Semicolon) {
                // It's an expression statement
                statements.push(Statement {
                    span: expr.span,
                    kind: StatementKind::ExprStatement(expr),
                });
            } else if self.at(&TokenKind::RBrace) {
                // It's the tail expression (no semicolon before closing brace)
                tail_expr = Some(Box::new(expr));
            } else if matches!(
                self.peek_kind(),
                TokenKind::Eq
                    | TokenKind::PlusEq
                    | TokenKind::MinusEq
                    | TokenKind::StarEq
                    | TokenKind::SlashEq
                    | TokenKind::PercentEq
            ) && matches!(expr.kind, ExpressionKind::Variable(_))
            {
                // It's actually an assignment — backtrack and parse as statement
                self.pos = saved_pos;
                statements.push(self.parse_statement()?);
            } else {
                // Treat as expression statement with implicit semicolon
                statements.push(Statement {
                    span: expr.span,
                    kind: StatementKind::ExprStatement(expr),
                });
            }
        }

        let end_tok = self.expect(&TokenKind::RBrace)?;
        Ok(Block {
            statements,
            tail_expr,
            span: start.merge(end_tok.span),
        })
    }

    // =========================================================================
    // Expressions — Pratt / precedence climbing
    // =========================================================================

    fn parse_expression(&mut self) -> Result<Expression, SyntaxError> {
        self.parse_pipe_expr()
    }

    /// Pipe expression: `expr |> expr |> expr`
    fn parse_pipe_expr(&mut self) -> Result<Expression, SyntaxError> {
        let mut left = self.parse_null_coalesce_expr()?;

        while self.at(&TokenKind::Pipe) {
            self.advance(); // consume |>
            let right = self.parse_null_coalesce_expr()?;
            let span = left.span.merge(right.span);
            left = Expression {
                kind: ExpressionKind::Pipe {
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            };
        }

        Ok(left)
    }

    /// Null coalescing: `expr ?? expr` (lower precedence than binary ops)
    fn parse_null_coalesce_expr(&mut self) -> Result<Expression, SyntaxError> {
        let mut left = self.parse_range_expr()?;

        while self.at(&TokenKind::QuestionQuestion) {
            self.advance(); // consume ??
            let right = self.parse_range_expr()?;
            let span = left.span.merge(right.span);
            left = Expression {
                kind: ExpressionKind::NullCoalesce {
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            };
        }

        Ok(left)
    }

    /// Range expression: `expr..expr` or `expr..=expr`
    fn parse_range_expr(&mut self) -> Result<Expression, SyntaxError> {
        let left = self.parse_binary_expr(0)?;

        if self.at(&TokenKind::DotDot) || self.at(&TokenKind::DotDotEq) {
            let inclusive = self.at(&TokenKind::DotDotEq);
            self.advance();
            let right = self.parse_binary_expr(0)?;
            let span = left.span.merge(right.span);
            return Ok(Expression {
                kind: ExpressionKind::Range {
                    start: Box::new(left),
                    end: Box::new(right),
                    inclusive,
                },
                span,
            });
        }

        Ok(left)
    }

    /// Binary expression with precedence climbing.
    fn parse_binary_expr(&mut self, min_prec: u8) -> Result<Expression, SyntaxError> {
        self.enter_depth()?;
        let result = self.parse_binary_expr_inner(min_prec);
        self.exit_depth();
        result
    }

    fn parse_binary_expr_inner(&mut self, min_prec: u8) -> Result<Expression, SyntaxError> {
        let mut left = self.parse_unary_expr()?;

        loop {
            let op = match self.peek_kind() {
                TokenKind::PipePipe => BinOp::Or,
                TokenKind::AndAnd => BinOp::And,
                TokenKind::EqEq => BinOp::Eq,
                TokenKind::NotEq => BinOp::NotEq,
                TokenKind::Gt => BinOp::Gt,
                TokenKind::Lt => BinOp::Lt,
                TokenKind::GtEq => BinOp::GtEq,
                TokenKind::LtEq => BinOp::LtEq,
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Mod,
                _ => break,
            };

            let prec = op.precedence();
            if prec < min_prec {
                break;
            }

            self.advance(); // consume operator
                            // Right-associative would use `prec`, left-associative uses `prec + 1`
            let right = self.parse_binary_expr(prec + 1)?;
            let span = left.span.merge(right.span);
            left = Expression {
                kind: ExpressionKind::BinaryOp {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            };
        }

        Ok(left)
    }

    /// Unary: `!expr`, `-expr`
    fn parse_unary_expr(&mut self) -> Result<Expression, SyntaxError> {
        self.enter_depth()?;
        let result = self.parse_unary_expr_inner();
        self.exit_depth();
        result
    }

    fn parse_unary_expr_inner(&mut self) -> Result<Expression, SyntaxError> {
        let start = self.peek().span;

        if self.at(&TokenKind::Await) {
            self.advance();
            let operand = self.parse_unary_expr()?;
            let span = start.merge(operand.span);
            return Ok(Expression {
                kind: ExpressionKind::Await(Box::new(operand)),
                span,
            });
        }

        if self.at(&TokenKind::Spawn) {
            self.advance();
            let operand = if self.at(&TokenKind::LBrace) {
                // spawn { block }
                let block = self.parse_block()?;
                Expression {
                    span: block.span,
                    kind: ExpressionKind::Block(block),
                }
            } else {
                self.parse_unary_expr()?
            };
            let span = start.merge(operand.span);
            return Ok(Expression {
                kind: ExpressionKind::Spawn(Box::new(operand)),
                span,
            });
        }

        if self.at(&TokenKind::Bang) {
            self.advance();
            let operand = self.parse_unary_expr()?;
            let span = start.merge(operand.span);
            return Ok(Expression {
                kind: ExpressionKind::UnaryOp {
                    op: UnOp::Not,
                    operand: Box::new(operand),
                },
                span,
            });
        }

        if self.at(&TokenKind::Minus) {
            // Distinguish unary minus from binary minus:
            // Unary if: at start, after operator, after '(', after ',', after '='
            // We handle this by checking what came before. In a Pratt parser the
            // unary case is only reached if we haven't started an expression yet.
            self.advance();
            let operand = self.parse_unary_expr()?;
            let span = start.merge(operand.span);
            return Ok(Expression {
                kind: ExpressionKind::UnaryOp {
                    op: UnOp::Neg,
                    operand: Box::new(operand),
                },
                span,
            });
        }

        self.parse_postfix_expr()
    }

    /// Postfix: call `f()`, index `a[i]`, field `a.b`, method `a.b()`, optional chain `a?.b`
    fn parse_postfix_expr(&mut self) -> Result<Expression, SyntaxError> {
        let mut expr = self.parse_primary()?;

        loop {
            if self.at(&TokenKind::LParen) {
                // Function call
                expr = self.parse_call_expr(expr)?;
            } else if self.at(&TokenKind::LBracket) {
                // Index
                self.advance();
                let index = self.parse_expression()?;
                let end = self.expect(&TokenKind::RBracket)?;
                let span = expr.span.merge(end.span);
                expr = Expression {
                    kind: ExpressionKind::Index {
                        object: Box::new(expr),
                        index: Box::new(index),
                    },
                    span,
                };
            } else if self.at(&TokenKind::Question) {
                // Error propagation: expr?
                let q = self.advance();
                let span = expr.span.merge(q.span);
                expr = Expression {
                    kind: ExpressionKind::TryPropagate(Box::new(expr)),
                    span,
                };
            } else if self.at(&TokenKind::QuestionDot) {
                // Optional chaining: obj?.field — propagates through subsequent .field/.method()
                self.advance();
                let field_tok = self.expect(&TokenKind::Ident)?;
                let span = expr.span.merge(field_tok.span);
                // Check if this is a direct optional method call: obj?.method(args)
                if self.at(&TokenKind::LParen) {
                    self.advance(); // consume '('
                    let mut args = Vec::new();
                    let mut kwargs = Vec::new();
                    while !self.at(&TokenKind::RParen) && !self.at(&TokenKind::Eof) {
                        if self.peek_kind() == &TokenKind::Ident {
                            let saved = self.pos;
                            let name_tok = self.advance().clone();
                            if self.at(&TokenKind::Eq) {
                                self.advance();
                                let value = self.parse_expression()?;
                                kwargs.push((name_tok.text, value));
                                if !self.eat(&TokenKind::Comma) { break; }
                                continue;
                            }
                            self.pos = saved;
                        }
                        args.push(self.parse_expression()?);
                        if !self.eat(&TokenKind::Comma) { break; }
                    }
                    let end = self.expect(&TokenKind::RParen)?;
                    let method_span = expr.span.merge(end.span);
                    // Wrap expr in OptionalChain as a marker for null propagation
                    // The interpreter detects OptionalChain as MethodCall object → returns null if base is null
                    let optional_marker = Expression {
                        kind: ExpressionKind::OptionalChain {
                            object: Box::new(expr),
                            field: String::new(), // empty field = method call marker
                        },
                        span: method_span,
                    };
                    expr = Expression {
                        kind: ExpressionKind::MethodCall {
                            object: Box::new(optional_marker),
                            method: field_tok.text,
                            args,
                            kwargs,
                        },
                        span: method_span,
                    };
                } else {
                    expr = Expression {
                        kind: ExpressionKind::OptionalChain {
                            object: Box::new(expr),
                            field: field_tok.text,
                        },
                        span,
                    };
                }
                // Phase 14: propagate optional chaining through subsequent .field and .method()
                while self.at(&TokenKind::Dot) {
                    self.advance();
                    let next_field = self.expect(&TokenKind::Ident)?;
                    if self.at(&TokenKind::LParen) {
                        // Optional method call: a?.b.method() → optional wrap
                        self.advance();
                        let mut args = Vec::new();
                        let mut kwargs = Vec::new();
                        while !self.at(&TokenKind::RParen) && !self.at(&TokenKind::Eof) {
                            if self.peek_kind() == &TokenKind::Ident {
                                let saved = self.pos;
                                let name_tok = self.advance().clone();
                                if self.at(&TokenKind::Eq) {
                                    self.advance();
                                    let value = self.parse_expression()?;
                                    kwargs.push((name_tok.text, value));
                                    if !self.eat(&TokenKind::Comma) { break; }
                                    continue;
                                }
                                self.pos = saved;
                            }
                            args.push(self.parse_expression()?);
                            if !self.eat(&TokenKind::Comma) { break; }
                        }
                        let end = self.expect(&TokenKind::RParen)?;
                        let method_span = expr.span.merge(end.span);
                        // Wrap: if expr is null, stay null; else call method
                        let inner_call = Expression {
                            kind: ExpressionKind::MethodCall {
                                object: Box::new(expr),
                                method: next_field.text,
                                args,
                                kwargs,
                            },
                            span: method_span,
                        };
                        expr = inner_call;
                    } else {
                        let new_span = expr.span.merge(next_field.span);
                        expr = Expression {
                            kind: ExpressionKind::OptionalChain {
                                object: Box::new(expr),
                                field: next_field.text,
                            },
                            span: new_span,
                        };
                    }
                }
            } else if self.at(&TokenKind::Dot) {
                self.advance();
                let field_tok = self.expect(&TokenKind::Ident)?;

                // Check if followed by `(` — that makes it a method call
                if self.at(&TokenKind::LParen) {
                    self.advance(); // consume '('
                    let mut args = Vec::new();
                    let mut kwargs = Vec::new();

                    while !self.at(&TokenKind::RParen) && !self.at(&TokenKind::Eof) {
                        // Check for keyword argument
                        if self.peek_kind() == &TokenKind::Ident {
                            let saved = self.pos;
                            let name_tok = self.advance().clone();
                            if self.at(&TokenKind::Eq) {
                                self.advance();
                                let value = self.parse_expression()?;
                                kwargs.push((name_tok.text, value));
                                if !self.eat(&TokenKind::Comma) {
                                    break;
                                }
                                continue;
                            }
                            self.pos = saved;
                        }

                        let arg = self.parse_expression()?;
                        args.push(arg);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    let end = self.expect(&TokenKind::RParen)?;
                    let span = expr.span.merge(end.span);
                    expr = Expression {
                        kind: ExpressionKind::MethodCall {
                            object: Box::new(expr),
                            method: field_tok.text,
                            args,
                            kwargs,
                        },
                        span,
                    };
                } else {
                    let span = expr.span.merge(field_tok.span);
                    expr = Expression {
                        kind: ExpressionKind::FieldAccess {
                            object: Box::new(expr),
                            field: field_tok.text,
                        },
                        span,
                    };
                }
            } else {
                break;
            }
        }

        Ok(expr)
    }

    /// Parse call arguments when we've already parsed the callee.
    fn parse_call_expr(&mut self, callee: Expression) -> Result<Expression, SyntaxError> {
        self.advance(); // consume '('
        let mut args = Vec::new();
        let mut kwargs = Vec::new();

        while !self.at(&TokenKind::RParen) && !self.at(&TokenKind::Eof) {
            // Check for keyword argument: `name=value`
            if self.peek_kind() == &TokenKind::Ident {
                let saved = self.pos;
                let name_tok = self.advance().clone();
                if self.at(&TokenKind::Eq) {
                    self.advance(); // consume '='
                    let value = self.parse_expression()?;
                    kwargs.push((name_tok.text, value));
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                    continue;
                }
                // Not a kwarg — backtrack
                self.pos = saved;
            }

            let arg = self.parse_expression()?;
            args.push(arg);

            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }

        let end = self.expect(&TokenKind::RParen)?;

        // Extract function name from callee
        let name = match &callee.kind {
            ExpressionKind::Variable(name) => name.clone(),
            _ => {
                return Err(SyntaxError {
                    line: callee.span.start_line as usize,
                    column: callee.span.start_col as usize,
                    message: "Expected function name".to_string(),
                });
            }
        };

        let span = callee.span.merge(end.span);
        Ok(Expression {
            kind: ExpressionKind::Call { name, args, kwargs },
            span,
        })
    }

    // =========================================================================
    // Primary expressions
    // =========================================================================

    fn parse_primary(&mut self) -> Result<Expression, SyntaxError> {
        let tok = self.peek().clone();

        match &tok.kind {
            // Integer literal
            TokenKind::IntLiteral => {
                self.advance();
                let val: i64 = tok.text.parse().map_err(|_| SyntaxError {
                    line: tok.span.start_line as usize,
                    column: tok.span.start_col as usize,
                    message: format!("Invalid integer: {}", tok.text),
                })?;
                Ok(Expression {
                    kind: ExpressionKind::Literal(Literal::Int64(val)),
                    span: tok.span,
                })
            }

            // Float literal
            TokenKind::FloatLiteral => {
                self.advance();
                let val: f64 = tok.text.parse().map_err(|_| SyntaxError {
                    line: tok.span.start_line as usize,
                    column: tok.span.start_col as usize,
                    message: format!("Invalid float: {}", tok.text),
                })?;
                Ok(Expression {
                    kind: ExpressionKind::Literal(Literal::Float64(val)),
                    span: tok.span,
                })
            }

            // String literal
            TokenKind::StringLiteral => {
                self.advance();
                Ok(Expression {
                    kind: ExpressionKind::Literal(Literal::String(tok.text.clone())),
                    span: tok.span,
                })
            }

            // Boolean true
            TokenKind::True => {
                self.advance();
                Ok(Expression {
                    kind: ExpressionKind::Literal(Literal::Bool(true)),
                    span: tok.span,
                })
            }

            // Boolean false
            TokenKind::False => {
                self.advance();
                Ok(Expression {
                    kind: ExpressionKind::Literal(Literal::Bool(false)),
                    span: tok.span,
                })
            }

            // Null
            TokenKind::Null => {
                self.advance();
                Ok(Expression {
                    kind: ExpressionKind::Literal(Literal::Null),
                    span: tok.span,
                })
            }

            // Underscore placeholder (for pipe expressions)
            TokenKind::Underscore => {
                self.advance();
                Ok(Expression {
                    kind: ExpressionKind::Placeholder,
                    span: tok.span,
                })
            }

            // Identifier (variable, function call, enum construct, struct construct)
            TokenKind::Ident => {
                self.advance();

                // Enum construction: Name::Variant or Name::Variant(args)
                if self.at(&TokenKind::ColonColon) {
                    let enum_name = tok.text.clone();
                    self.advance(); // consume ::
                    let variant_tok = self.expect_identifier()?;
                    let mut args = Vec::new();
                    if self.eat(&TokenKind::LParen) {
                        while !self.at(&TokenKind::RParen) && !self.at(&TokenKind::Eof) {
                            args.push(self.parse_expression()?);
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                        }
                        let end = self.expect(&TokenKind::RParen)?;
                        return Ok(Expression {
                            kind: ExpressionKind::EnumConstruct {
                                enum_name,
                                variant: variant_tok.text,
                                args,
                            },
                            span: tok.span.merge(end.span),
                        });
                    }
                    return Ok(Expression {
                        kind: ExpressionKind::EnumConstruct {
                            enum_name,
                            variant: variant_tok.text,
                            args,
                        },
                        span: tok.span.merge(variant_tok.span),
                    });
                }

                // Struct construction: Name { field: value, ... }
                // Disambiguate from block: check Ident { Ident :
                if self.at(&TokenKind::LBrace) && self.is_struct_literal(&tok.text) {
                    let name = tok.text.clone();
                    self.advance(); // consume {
                    let mut fields = Vec::new();
                    while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                        let field_tok = self.expect_identifier()?;
                        self.expect(&TokenKind::Colon)?;
                        let value = self.parse_expression()?;
                        fields.push((field_tok.text, value));
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    let end = self.expect(&TokenKind::RBrace)?;
                    return Ok(Expression {
                        kind: ExpressionKind::StructConstruct { name, fields },
                        span: tok.span.merge(end.span),
                    });
                }

                Ok(Expression {
                    kind: ExpressionKind::Variable(tok.text.clone()),
                    span: tok.span,
                })
            }

            // Parenthesized expression
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression()?;
                let end = self.expect(&TokenKind::RParen)?;
                // Preserve the expression but update span
                Ok(Expression {
                    span: tok.span.merge(end.span),
                    ..expr
                })
            }

            // Array literal: [a, b, c]
            TokenKind::LBracket => self.parse_array_literal(),

            // Block or map literal
            TokenKind::LBrace => {
                // Disambiguate: map literal `{"key": val}` vs block `{ stmt; expr }`
                // If next token is string followed by colon, it's a map
                if self.is_map_literal() {
                    self.parse_map_literal()
                } else {
                    let block = self.parse_block()?;
                    let span = block.span;
                    Ok(Expression {
                        kind: ExpressionKind::Block(block),
                        span,
                    })
                }
            }

            // If expression
            TokenKind::If => self.parse_if_expr(),

            // Match expression
            TokenKind::Match => self.parse_match_expr(),

            // Loop expression
            TokenKind::Loop => {
                self.advance();
                let block = self.parse_block()?;
                let span = tok.span.merge(block.span);
                Ok(Expression {
                    kind: ExpressionKind::Loop(block),
                    span,
                })
            }

            // Try expression: try { ... } catch e { ... }
            TokenKind::Try => self.parse_try_expr(),

            // F-string interpolation
            TokenKind::FStringStart => self.parse_fstring_expr(),

            // Lambda: |params| expr
            TokenKind::Bar => self.parse_lambda_expr(),

            // Zero-parameter lambda: || expr
            TokenKind::PipePipe => {
                let start = self.advance().span; // consume '||'
                let body = if self.at(&TokenKind::LBrace) {
                    let block = self.parse_block()?;
                    Expression {
                        span: block.span,
                        kind: ExpressionKind::Block(block),
                    }
                } else {
                    self.parse_expression()?
                };
                let span = start.merge(body.span);
                Ok(Expression {
                    kind: ExpressionKind::Lambda {
                        params: Vec::new(),
                        body: Box::new(body),
                    },
                    span,
                })
            }

            // Spread: ...expr
            TokenKind::DotDotDot => {
                self.advance();
                let inner = self.parse_unary_expr()?;
                let span = tok.span.merge(inner.span);
                Ok(Expression {
                    kind: ExpressionKind::Spread(Box::new(inner)),
                    span,
                })
            }

            _ => Err(self.error(&format!(
                "Unexpected token '{}', expected expression",
                tok.kind
            ))),
        }
    }

    fn parse_array_literal(&mut self) -> Result<Expression, SyntaxError> {
        let start = self.expect(&TokenKind::LBracket)?;

        if self.at(&TokenKind::RBracket) {
            let end = self.expect(&TokenKind::RBracket)?;
            return Ok(Expression {
                kind: ExpressionKind::Literal(Literal::Array(Vec::new())),
                span: start.span.merge(end.span),
            });
        }

        let first = self.parse_expression()?;

        // List comprehension: [expr for pattern in iterable]
        if self.at(&TokenKind::For) {
            self.advance(); // consume 'for'
            let pattern = self.parse_comprehension_pattern()?;
            self.expect(&TokenKind::In)?;
            let iterable = self.parse_expression()?;
            let condition = if self.at(&TokenKind::If) {
                self.advance();
                Some(Box::new(self.parse_expression()?))
            } else {
                None
            };
            let end = self.expect(&TokenKind::RBracket)?;
            return Ok(Expression {
                kind: ExpressionKind::ListComprehension {
                    expr: Box::new(first),
                    pattern,
                    iterable: Box::new(iterable),
                    condition,
                },
                span: start.span.merge(end.span),
            });
        }

        let mut elements = vec![first];
        while self.eat(&TokenKind::Comma) {
            if self.at(&TokenKind::RBracket) {
                break; // trailing comma
            }
            elements.push(self.parse_expression()?);
        }

        let end = self.expect(&TokenKind::RBracket)?;
        Ok(Expression {
            kind: ExpressionKind::Literal(Literal::Array(elements)),
            span: start.span.merge(end.span),
        })
    }

    fn is_map_literal(&self) -> bool {
        // Look ahead: `{` then StringLiteral then `:` means map
        if self.pos + 2 < self.tokens.len() {
            self.tokens[self.pos].kind == TokenKind::LBrace
                && self.tokens[self.pos + 1].kind == TokenKind::StringLiteral
                && self.tokens[self.pos + 2].kind == TokenKind::Colon
        } else {
            false
        }
    }

    fn parse_map_literal(&mut self) -> Result<Expression, SyntaxError> {
        let start = self.expect(&TokenKind::LBrace)?;

        if self.at(&TokenKind::RBrace) {
            let end = self.expect(&TokenKind::RBrace)?;
            return Ok(Expression {
                kind: ExpressionKind::Literal(Literal::Map(Vec::new())),
                span: start.span.merge(end.span),
            });
        }

        // First entry
        let key_tok = self.expect(&TokenKind::StringLiteral)?;
        self.expect(&TokenKind::Colon)?;
        let first_value = self.parse_expression()?;

        // Map comprehension: {"key_expr": value_expr for pattern in iterable}
        if self.at(&TokenKind::For) {
            self.advance();
            let pattern = self.parse_comprehension_pattern()?;
            self.expect(&TokenKind::In)?;
            let iterable = self.parse_expression()?;
            let condition = if self.at(&TokenKind::If) {
                self.advance();
                Some(Box::new(self.parse_expression()?))
            } else {
                None
            };
            let end = self.expect(&TokenKind::RBrace)?;
            return Ok(Expression {
                kind: ExpressionKind::MapComprehension {
                    key_expr: Box::new(Expression {
                        kind: ExpressionKind::Literal(Literal::String(key_tok.text)),
                        span: key_tok.span,
                    }),
                    value_expr: Box::new(first_value),
                    pattern,
                    iterable: Box::new(iterable),
                    condition,
                },
                span: start.span.merge(end.span),
            });
        }

        let mut entries = vec![(key_tok.text, first_value)];
        while self.eat(&TokenKind::Comma) {
            if self.at(&TokenKind::RBrace) {
                break;
            }
            let key_tok = self.expect(&TokenKind::StringLiteral)?;
            self.expect(&TokenKind::Colon)?;
            let value = self.parse_expression()?;
            entries.push((key_tok.text, value));
        }

        let end = self.expect(&TokenKind::RBrace)?;
        Ok(Expression {
            kind: ExpressionKind::Literal(Literal::Map(entries)),
            span: start.span.merge(end.span),
        })
    }

    fn parse_if_expr(&mut self) -> Result<Expression, SyntaxError> {
        let start = self.advance().span; // consume 'if'
        let condition = self.parse_expression()?;
        let then_block = self.parse_block()?;

        let else_block = if self.eat(&TokenKind::Else) {
            if self.at(&TokenKind::If) {
                // else if — parse nested if as a block containing a single tail expression
                let nested_if = self.parse_if_expr()?;
                let span = nested_if.span;
                Some(Block {
                    statements: Vec::new(),
                    tail_expr: Some(Box::new(nested_if)),
                    span,
                })
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };

        let end_span = else_block
            .as_ref()
            .map(|b| b.span)
            .unwrap_or(then_block.span);

        Ok(Expression {
            kind: ExpressionKind::IfElse {
                condition: Box::new(condition),
                then_block,
                else_block,
            },
            span: start.merge(end_span),
        })
    }

    fn parse_match_expr(&mut self) -> Result<Expression, SyntaxError> {
        let start = self.advance().span; // consume 'match'
        let value = self.parse_expression()?;
        self.expect(&TokenKind::LBrace)?;

        let mut arms = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let arm_start = self.peek().span;
            let pattern = self.parse_pattern()?;

            // Optional guard: `if condition`
            let guard = if self.eat(&TokenKind::If) {
                Some(self.parse_expression()?)
            } else {
                None
            };

            self.expect(&TokenKind::FatArrow)?;

            // Arm body: either a block or a single expression
            let body = if self.at(&TokenKind::LBrace) {
                self.parse_block()?
            } else {
                let expr = self.parse_expression()?;
                let span = expr.span;
                Block {
                    statements: Vec::new(),
                    tail_expr: Some(Box::new(expr)),
                    span,
                }
            };

            let arm_end = body.span;
            arms.push(MatchArm {
                pattern,
                guard,
                body,
                span: arm_start.merge(arm_end),
            });

            // Arms separated by commas (optional before closing brace)
            self.eat(&TokenKind::Comma);
        }

        let end = self.expect(&TokenKind::RBrace)?;
        Ok(Expression {
            kind: ExpressionKind::Match {
                value: Box::new(value),
                arms,
            },
            span: start.merge(end.span),
        })
    }

    fn parse_pattern(&mut self) -> Result<Pattern, SyntaxError> {
        let mut pattern = self.parse_single_pattern()?;

        // Or patterns: `1 | 2 | 3`
        if self.at(&TokenKind::Bar) {
            let mut patterns = vec![pattern];
            while self.eat(&TokenKind::Bar) {
                patterns.push(self.parse_single_pattern()?);
            }
            pattern = Pattern::Or(patterns);
        }

        Ok(pattern)
    }

    fn parse_single_pattern(&mut self) -> Result<Pattern, SyntaxError> {
        let tok = self.peek().clone();
        match &tok.kind {
            TokenKind::IntLiteral => {
                self.advance();
                let val: i64 = tok
                    .text
                    .parse()
                    .map_err(|_| self.error("Invalid integer"))?;
                let start_expr = Expression {
                    kind: ExpressionKind::Literal(Literal::Int64(val)),
                    span: tok.span,
                };
                // Range pattern: 0..10 or 0..=10
                if self.at(&TokenKind::DotDot) || self.at(&TokenKind::DotDotEq) {
                    let inclusive = self.at(&TokenKind::DotDotEq);
                    self.advance();
                    let end_tok = self.peek().clone();
                    if end_tok.kind == TokenKind::IntLiteral {
                        self.advance();
                        let end_val: i64 = end_tok.text.parse().map_err(|_| self.error("Invalid integer"))?;
                        return Ok(Pattern::RangePattern {
                            start: Box::new(start_expr),
                            end: Box::new(Expression {
                                kind: ExpressionKind::Literal(Literal::Int64(end_val)),
                                span: end_tok.span,
                            }),
                            inclusive,
                        });
                    }
                }
                Ok(Pattern::Literal(Literal::Int64(val)))
            }
            TokenKind::FloatLiteral => {
                self.advance();
                let val: f64 = tok.text.parse().map_err(|_| self.error("Invalid float"))?;
                Ok(Pattern::Literal(Literal::Float64(val)))
            }
            TokenKind::StringLiteral => {
                self.advance();
                Ok(Pattern::Literal(Literal::String(tok.text.clone())))
            }
            TokenKind::True => {
                self.advance();
                Ok(Pattern::Literal(Literal::Bool(true)))
            }
            TokenKind::False => {
                self.advance();
                Ok(Pattern::Literal(Literal::Bool(false)))
            }
            TokenKind::Null => {
                self.advance();
                Ok(Pattern::Literal(Literal::Null))
            }
            TokenKind::Underscore => {
                self.advance();
                Ok(Pattern::Wildcard)
            }
            TokenKind::DotDotDot => {
                self.advance();
                // ...rest or just ...
                if self.at(&TokenKind::Ident) {
                    let name_tok = self.advance().clone();
                    Ok(Pattern::Rest(Some(name_tok.text)))
                } else {
                    Ok(Pattern::Rest(None))
                }
            }
            TokenKind::LBracket => {
                // Array pattern: [a, b, c]
                self.advance();
                let mut elements = Vec::new();
                while !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
                    elements.push(self.parse_pattern()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(&TokenKind::RBracket)?;
                Ok(Pattern::Array(elements))
            }
            TokenKind::Ident => {
                self.advance();
                // Enum pattern: Name::Variant or Name::Variant(bindings)
                if self.at(&TokenKind::ColonColon) {
                    let enum_name = tok.text.clone();
                    self.advance(); // consume ::
                    let variant_tok = self.expect_identifier()?;
                    let mut bindings = Vec::new();
                    if self.eat(&TokenKind::LParen) {
                        while !self.at(&TokenKind::RParen) && !self.at(&TokenKind::Eof) {
                            bindings.push(self.parse_pattern()?);
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                        }
                        self.expect(&TokenKind::RParen)?;
                    }
                    return Ok(Pattern::EnumPattern {
                        enum_name,
                        variant: variant_tok.text,
                        bindings,
                    });
                }
                // Type pattern: name: type_name (in match context)
                if self.at(&TokenKind::Colon) {
                    let saved = self.pos;
                    self.advance(); // consume :
                    if self.at(&TokenKind::Ident) {
                        let type_tok = self.advance().clone();
                        return Ok(Pattern::TypePattern {
                            name: tok.text.clone(),
                            type_name: type_tok.text,
                        });
                    }
                    self.pos = saved;
                }
                Ok(Pattern::Variable(tok.text.clone()))
            }
            TokenKind::Minus => {
                // Negative literal: -42
                self.advance();
                if self.at(&TokenKind::IntLiteral) {
                    let num_tok = self.advance().clone();
                    let val: i64 = num_tok
                        .text
                        .parse()
                        .map_err(|_| self.error("Invalid integer"))?;
                    Ok(Pattern::Literal(Literal::Int64(-val)))
                } else if self.at(&TokenKind::FloatLiteral) {
                    let num_tok = self.advance().clone();
                    let val: f64 = num_tok
                        .text
                        .parse()
                        .map_err(|_| self.error("Invalid float"))?;
                    Ok(Pattern::Literal(Literal::Float64(-val)))
                } else {
                    Err(self.error("Expected number after '-' in pattern"))
                }
            }
            _ => Err(self.error(&format!("Unexpected token '{}' in pattern", tok.kind))),
        }
    }

    fn parse_try_expr(&mut self) -> Result<Expression, SyntaxError> {
        let start = self.advance().span; // consume 'try'
        let try_block = self.parse_block()?;
        self.expect(&TokenKind::Catch)?;

        let catch_var = if self.at(&TokenKind::Ident) {
            let tok = self.advance().clone();
            Some(tok.text)
        } else {
            None
        };

        let catch_block = self.parse_block()?;
        let span = start.merge(catch_block.span);

        Ok(Expression {
            kind: ExpressionKind::TryCatchExpr {
                try_block,
                catch_var,
                catch_block,
            },
            span,
        })
    }

    fn parse_fstring_expr(&mut self) -> Result<Expression, SyntaxError> {
        let tok = self.advance().clone(); // consume FStringStart
        let raw = &tok.text;
        let mut parts = Vec::new();
        let mut current_lit = String::new();
        let mut chars = raw.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '{' {
                // Start of expression interpolation
                if !current_lit.is_empty() {
                    parts.push(StringPart::Literal(std::mem::take(&mut current_lit)));
                }
                // Collect until matching '}'
                let mut expr_str = String::new();
                let mut depth = 1;
                for inner in chars.by_ref() {
                    if inner == '{' {
                        depth += 1;
                    } else if inner == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    expr_str.push(inner);
                }
                if depth > 0 {
                    return Err(SyntaxError {
                        line: tok.span.start_line as usize,
                        column: tok.span.start_col as usize,
                        message: "Unclosed interpolation brace in f-string".to_string(),
                    });
                }
                // Parse the inner expression
                let inner_tokens = super::lexer::tokenize(&expr_str).map_err(|e| SyntaxError {
                    line: tok.span.start_line as usize,
                    column: tok.span.start_col as usize,
                    message: format!("Error in f-string expression: {}", e.message),
                })?;
                let mut inner_parser = Parser::new(inner_tokens);
                let expr = inner_parser.parse_expression().map_err(|e| SyntaxError {
                    line: tok.span.start_line as usize,
                    column: tok.span.start_col as usize,
                    message: format!("Error in f-string expression: {}", e.message),
                })?;
                parts.push(StringPart::Expr(expr));
            } else {
                current_lit.push(ch);
            }
        }

        if !current_lit.is_empty() {
            parts.push(StringPart::Literal(current_lit));
        }

        Ok(Expression {
            kind: ExpressionKind::StringInterpolation { parts },
            span: tok.span,
        })
    }

    fn parse_lambda_expr(&mut self) -> Result<Expression, SyntaxError> {
        let start = self.advance().span; // consume opening '|'

        // Parse parameters
        let params = self.parse_function_params(&TokenKind::Bar)?;
        self.expect(&TokenKind::Bar)?;

        // Body: either a block or a single expression
        let body = if self.at(&TokenKind::LBrace) {
            let block = self.parse_block()?;
            Expression {
                span: block.span,
                kind: ExpressionKind::Block(block),
            }
        } else {
            self.parse_expression()?
        };

        let span = start.merge(body.span);
        Ok(Expression {
            kind: ExpressionKind::Lambda {
                params,
                body: Box::new(body),
            },
            span,
        })
    }

    // =========================================================================
    // Enum / Struct definitions
    // =========================================================================

    fn parse_enum_def(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'enum'
        let name_tok = self.expect_identifier()?;
        let name = name_tok.text;
        self.expect(&TokenKind::LBrace)?;

        let mut variants = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let var_start = self.peek().span;
            let var_tok = self.expect_identifier()?;
            let mut fields = Vec::new();
            if self.eat(&TokenKind::LParen) {
                while !self.at(&TokenKind::RParen) && !self.at(&TokenKind::Eof) {
                    let field_tok = self.expect_identifier()?;
                    fields.push(field_tok.text);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(&TokenKind::RParen)?;
            }
            let var_end = self.peek().span;
            variants.push(EnumVariant {
                name: var_tok.text,
                fields,
                span: var_start.merge(var_end),
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }

        let end = self.expect(&TokenKind::RBrace)?;
        Ok(Statement {
            span: start.merge(end.span),
            kind: StatementKind::EnumDef { name, variants },
        })
    }

    fn parse_struct_def(&mut self, start: Span) -> Result<Statement, SyntaxError> {
        self.advance(); // consume 'struct'
        let name_tok = self.expect_identifier()?;
        let name = name_tok.text;
        self.expect(&TokenKind::LBrace)?;

        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let field_start = self.peek().span;
            let field_tok = self.expect_identifier()?;
            let type_annotation = if self.eat(&TokenKind::Colon) {
                let type_tok = self.expect(&TokenKind::Ident)?;
                Some(type_tok.text)
            } else {
                None
            };
            let field_end = self.peek().span;
            fields.push(StructField {
                name: field_tok.text,
                type_annotation,
                span: field_start.merge(field_end),
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }

        let end = self.expect(&TokenKind::RBrace)?;
        Ok(Statement {
            span: start.merge(end.span),
            kind: StatementKind::StructDef { name, fields },
        })
    }

    // =========================================================================
    // Helpers
    // =========================================================================

    /// Check if current position looks like a struct literal: `Name { ident :`
    /// (not `Name { stmt...`)
    fn is_struct_literal(&self, _name: &str) -> bool {
        // We're at LBrace. Check if: LBrace Ident Colon
        if self.pos + 2 < self.tokens.len() {
            self.tokens[self.pos].kind == TokenKind::LBrace
                && self.tokens[self.pos + 1].kind == TokenKind::Ident
                && self.tokens[self.pos + 2].kind == TokenKind::Colon
        } else {
            false
        }
    }

    /// Parse a pattern used in comprehensions (for x in ..., for [a,b] in ...)
    fn parse_comprehension_pattern(&mut self) -> Result<ForPattern, SyntaxError> {
        if self.at(&TokenKind::LBracket) {
            self.advance();
            let mut elements = Vec::new();
            while !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
                if self.at(&TokenKind::DotDotDot) {
                    self.advance();
                    let rest_tok = self.expect_identifier()?;
                    elements.push(DestructureElement::Rest(rest_tok.text));
                    break;
                }
                let name_tok = self.expect_identifier()?;
                elements.push(DestructureElement::Name(name_tok.text));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RBracket)?;
            Ok(ForPattern::ArrayDestructure(elements))
        } else if self.at(&TokenKind::LBrace) {
            self.advance();
            let mut entries = Vec::new();
            while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                let key_tok = self.expect_identifier()?;
                let alias = if self.eat(&TokenKind::Colon) {
                    let alias_tok = self.expect_identifier()?;
                    Some(alias_tok.text)
                } else {
                    None
                };
                entries.push((key_tok.text, alias));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RBrace)?;
            Ok(ForPattern::MapDestructure(entries))
        } else {
            let var_tok = self.expect_identifier()?;
            Ok(ForPattern::Single(var_tok.text))
        }
    }
}

// =============================================================================
// Public entry point
// =============================================================================

/// Parse v2 source code into an AST.
pub fn parse_v2(source: &str) -> Result<Program, SyntaxError> {
    let tokens = super::lexer::tokenize(source)?;
    let mut parser = Parser::new(tokens);
    parser.parse_program()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(code: &str) -> Program {
        parse_v2(code).unwrap()
    }

    fn parse_err(code: &str) -> SyntaxError {
        parse_v2(code).unwrap_err()
    }

    // --- Import ---

    #[test]
    fn test_parse_import() {
        let prog = parse(r#"import "capture";"#);
        assert_eq!(prog.statements.len(), 1);
        match &prog.statements[0].kind {
            StatementKind::Import(name) => assert_eq!(name, "capture"),
            other => panic!("Expected Import, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_import_no_semicolon() {
        let prog = parse(r#"import "capture""#);
        assert_eq!(prog.statements.len(), 1);
    }

    #[test]
    fn test_parse_import_empty() {
        let err = parse_err(r#"import "";"#);
        assert!(err.message.contains("empty"));
    }

    // --- Let ---

    #[test]
    fn test_parse_let_literal() {
        let prog = parse("let x = 42;");
        match &prog.statements[0].kind {
            StatementKind::Let {
                name,
                type_annotation,
                value,
            } => {
                assert_eq!(name, "x");
                assert!(type_annotation.is_none());
                assert!(matches!(
                    value.kind,
                    ExpressionKind::Literal(Literal::Int64(42))
                ));
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_let_with_type() {
        let prog = parse("let x: int64 = 42;");
        match &prog.statements[0].kind {
            StatementKind::Let {
                type_annotation, ..
            } => {
                assert_eq!(type_annotation.as_deref(), Some("int64"));
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_let_mut() {
        let prog = parse("let mut total = 0;");
        match &prog.statements[0].kind {
            StatementKind::LetMut { name, .. } => assert_eq!(name, "total"),
            other => panic!("Expected LetMut, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_let_string() {
        let prog = parse(r#"let name = "hello";"#);
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(
                    &value.kind,
                    ExpressionKind::Literal(Literal::String(s)) if s == "hello"
                ));
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_let_bool() {
        let prog = parse("let flag = true;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(
                    value.kind,
                    ExpressionKind::Literal(Literal::Bool(true))
                ));
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_let_null() {
        let prog = parse("let nothing = null;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(value.kind, ExpressionKind::Literal(Literal::Null)));
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_let_array() {
        let prog = parse("let items = [1, 2, 3];");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::Literal(Literal::Array(elems)) = &value.kind {
                    assert_eq!(elems.len(), 3);
                } else {
                    panic!("Expected array literal");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_let_float() {
        let prog = parse("let pi = 3.14;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(
                    value.kind,
                    ExpressionKind::Literal(Literal::Float64(f)) if (f - 3.14).abs() < f64::EPSILON
                ));
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    // --- Assignment ---

    #[test]
    fn test_parse_assignment() {
        let prog = parse("total = total + 1;");
        match &prog.statements[0].kind {
            StatementKind::Assignment { name, .. } => assert_eq!(name, "total"),
            other => panic!("Expected Assignment, got {:?}", other),
        }
    }

    // --- Binary operators ---

    #[test]
    fn test_parse_add() {
        let prog = parse("let sum = a + b;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(
                    value.kind,
                    ExpressionKind::BinaryOp { op: BinOp::Add, .. }
                ));
            }
            other => panic!("Expected Let with BinaryOp, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_comparison() {
        let prog = parse("let cmp = x > 10;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(
                    value.kind,
                    ExpressionKind::BinaryOp { op: BinOp::Gt, .. }
                ));
            }
            other => panic!("Expected Let with comparison, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_logical() {
        let prog = parse("let flag = a && b;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(
                    value.kind,
                    ExpressionKind::BinaryOp { op: BinOp::And, .. }
                ));
            }
            other => panic!("Expected Let with logical, got {:?}", other),
        }
    }

    // --- Operator precedence ---

    #[test]
    fn test_precedence_mul_over_add() {
        // 1 + 2 * 3 should parse as 1 + (2 * 3)
        let prog = parse("let r = 1 + 2 * 3;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::BinaryOp { op, left, right } = &value.kind {
                    assert_eq!(*op, BinOp::Add);
                    assert!(matches!(
                        left.kind,
                        ExpressionKind::Literal(Literal::Int64(1))
                    ));
                    if let ExpressionKind::BinaryOp {
                        op: inner_op,
                        left: inner_left,
                        right: inner_right,
                    } = &right.kind
                    {
                        assert_eq!(*inner_op, BinOp::Mul);
                        assert!(matches!(
                            inner_left.kind,
                            ExpressionKind::Literal(Literal::Int64(2))
                        ));
                        assert!(matches!(
                            inner_right.kind,
                            ExpressionKind::Literal(Literal::Int64(3))
                        ));
                    } else {
                        panic!("Right side should be Mul");
                    }
                } else {
                    panic!("Expected BinaryOp");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_precedence_and_over_or() {
        // a || b && c should parse as a || (b && c)
        let prog = parse("let r = a || b && c;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::BinaryOp { op, .. } = &value.kind {
                    assert_eq!(*op, BinOp::Or);
                } else {
                    panic!("Expected BinaryOp(Or)");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_precedence_comparison_over_logical() {
        // a > 5 && b < 10 should parse as (a > 5) && (b < 10)
        let prog = parse("let r = a > 5 && b < 10;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::BinaryOp { op, left, right } = &value.kind {
                    assert_eq!(*op, BinOp::And);
                    assert!(matches!(
                        left.kind,
                        ExpressionKind::BinaryOp { op: BinOp::Gt, .. }
                    ));
                    assert!(matches!(
                        right.kind,
                        ExpressionKind::BinaryOp { op: BinOp::Lt, .. }
                    ));
                } else {
                    panic!("Expected And at top level");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_parens_override_precedence() {
        // (1 + 2) * 3 should parse as (1 + 2) * 3
        let prog = parse("let r = (1 + 2) * 3;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::BinaryOp { op, left, .. } = &value.kind {
                    assert_eq!(*op, BinOp::Mul);
                    assert!(matches!(
                        left.kind,
                        ExpressionKind::BinaryOp { op: BinOp::Add, .. }
                    ));
                } else {
                    panic!("Expected Mul at top level");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    // --- Unary operators ---

    #[test]
    fn test_parse_unary_not() {
        let prog = parse("let r = !flag;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(
                    value.kind,
                    ExpressionKind::UnaryOp { op: UnOp::Not, .. }
                ));
            }
            other => panic!("Expected Let with UnaryOp, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_unary_neg() {
        let prog = parse("let r = -x;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(
                    value.kind,
                    ExpressionKind::UnaryOp { op: UnOp::Neg, .. }
                ));
            }
            other => panic!("Expected Let with UnaryOp, got {:?}", other),
        }
    }

    // --- Function calls ---

    #[test]
    fn test_parse_call_no_args() {
        let prog = parse("let r = foo();");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::Call {
                    name, args, kwargs, ..
                } = &value.kind
                {
                    assert_eq!(name, "foo");
                    assert!(args.is_empty());
                    assert!(kwargs.is_empty());
                } else {
                    panic!("Expected Call");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_call_with_args() {
        let prog = parse("let r = add(x, y);");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::Call { name, args, .. } = &value.kind {
                    assert_eq!(name, "add");
                    assert_eq!(args.len(), 2);
                } else {
                    panic!("Expected Call");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_call_with_kwargs() {
        let prog = parse(r#"let r = capture(mode="stream");"#);
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::Call {
                    name, args, kwargs, ..
                } = &value.kind
                {
                    assert_eq!(name, "capture");
                    assert!(args.is_empty());
                    assert_eq!(kwargs.len(), 1);
                    assert_eq!(kwargs[0].0, "mode");
                } else {
                    panic!("Expected Call");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_call_mixed_args() {
        let prog = parse(r#"let r = process(input, mode="fast", verbose=true);"#);
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::Call { args, kwargs, .. } = &value.kind {
                    assert_eq!(args.len(), 1);
                    assert_eq!(kwargs.len(), 2);
                } else {
                    panic!("Expected Call");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    // --- If/else ---

    #[test]
    fn test_parse_if_else() {
        let prog = parse("let r = if x > 0 { x } else { 0 };");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::IfElse {
                    then_block,
                    else_block,
                    ..
                } = &value.kind
                {
                    assert!(then_block.tail_expr.is_some());
                    assert!(else_block.is_some());
                } else {
                    panic!("Expected IfElse");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_if_without_else() {
        let prog = parse("if cond { x; }");
        match &prog.statements[0].kind {
            StatementKind::ExprStatement(expr) => {
                if let ExpressionKind::IfElse { else_block, .. } = &expr.kind {
                    assert!(else_block.is_none());
                } else {
                    panic!("Expected IfElse");
                }
            }
            other => panic!("Expected ExprStatement, got {:?}", other),
        }
    }

    // --- For loop ---

    #[test]
    fn test_parse_for_loop() {
        let prog = parse("for item in items { x; }");
        match &prog.statements[0].kind {
            StatementKind::ForLoop {
                pattern,
                iterable,
                body,
            } => {
                assert!(matches!(pattern, ForPattern::Single(name) if name == "item"));
                assert!(matches!(iterable.kind, ExpressionKind::Variable(_)));
                assert_eq!(body.statements.len(), 1);
            }
            other => panic!("Expected ForLoop, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_for_range() {
        let prog = parse("for i in range(0, 10) { x; }");
        match &prog.statements[0].kind {
            StatementKind::ForLoop { iterable, .. } => {
                assert!(matches!(iterable.kind, ExpressionKind::Call { .. }));
            }
            other => panic!("Expected ForLoop, got {:?}", other),
        }
    }

    // --- While loop ---

    #[test]
    fn test_parse_while_loop() {
        let prog = parse("while total < 100 { total = total * 2; }");
        match &prog.statements[0].kind {
            StatementKind::WhileLoop { condition, body } => {
                assert!(matches!(
                    condition.kind,
                    ExpressionKind::BinaryOp { op: BinOp::Lt, .. }
                ));
                assert_eq!(body.statements.len(), 1);
            }
            other => panic!("Expected WhileLoop, got {:?}", other),
        }
    }

    // --- Pipe expressions ---

    #[test]
    fn test_parse_pipe() {
        let prog = parse("let r = x |> add(_, 5);");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(value.kind, ExpressionKind::Pipe { .. }));
            }
            other => panic!("Expected Let with Pipe, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_pipe_chain() {
        let prog = parse("let r = x |> add(_, 5) |> mul(_, 2);");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                // Should be Pipe { Pipe { x, add }, mul }
                if let ExpressionKind::Pipe { left, right } = &value.kind {
                    assert!(matches!(left.kind, ExpressionKind::Pipe { .. }));
                    assert!(matches!(right.kind, ExpressionKind::Call { .. }));
                } else {
                    panic!("Expected nested Pipe");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    // --- Output ---

    #[test]
    fn test_parse_output() {
        let prog = parse("output result;");
        match &prog.statements[0].kind {
            StatementKind::Output(expr) => {
                assert!(matches!(expr.kind, ExpressionKind::Variable(ref s) if s == "result"));
            }
            other => panic!("Expected Output, got {:?}", other),
        }
    }

    // --- Block expression ---

    #[test]
    fn test_parse_block_with_tail() {
        let prog = parse("let r = { let x = 5; x + 1 };");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::Block(block) = &value.kind {
                    assert_eq!(block.statements.len(), 1);
                    assert!(block.tail_expr.is_some());
                } else {
                    panic!("Expected Block");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    // --- Map literal ---

    #[test]
    fn test_parse_map_literal() {
        let prog = parse(r#"let m = {"name": "test", "count": 42};"#);
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::Literal(Literal::Map(entries)) = &value.kind {
                    assert_eq!(entries.len(), 2);
                    assert_eq!(entries[0].0, "name");
                } else {
                    panic!("Expected Map literal");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    // --- Index expression ---

    #[test]
    fn test_parse_index() {
        let prog = parse("let r = arr[0];");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(value.kind, ExpressionKind::Index { .. }));
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    // --- Field access ---

    #[test]
    fn test_parse_field_access() {
        let prog = parse("let r = obj.field;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::FieldAccess { field, .. } = &value.kind {
                    assert_eq!(field, "field");
                } else {
                    panic!("Expected FieldAccess");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    // --- Comments ---

    #[test]
    fn test_comments_ignored() {
        let prog = parse(
            r#"
// This is a comment
let x = 42;
// Another comment
let y = 10;
"#,
        );
        assert_eq!(prog.statements.len(), 2);
    }

    // --- Multi-statement program ---

    #[test]
    fn test_full_program() {
        let prog = parse(
            r#"
import "capture";
import "text-llm";

let name = "hello";
let count: int64 = 42;
let items = [1, 2, 3];

let sum = x + y;
let bigger = x > 10;
let flag = a && b;

let parts = split(text, ",");

let result = data |> split(_, ",") |> length(_);

let value = if condition {
    x + 1
} else {
    x - 1
};

let mut total = 0;
for item in items {
    total = total + item;
}

while total < 100 {
    total = total * 2;
}

output result;
"#,
        );
        // Count expected statements
        assert!(prog.statements.len() >= 12);
    }

    // --- Error cases ---

    #[test]
    fn test_error_missing_semicolon_before_let() {
        // The parser should still handle this since semicolons are optional
        let prog = parse("let x = 42\nlet y = 10");
        assert_eq!(prog.statements.len(), 2);
    }

    #[test]
    fn test_error_unclosed_brace() {
        let err = parse_err("let r = { x + 1");
        assert!(err.message.contains("Expected '}'"));
    }

    #[test]
    fn test_error_unclosed_paren() {
        let err = parse_err("let r = add(x, y");
        assert!(err.message.contains("Expected ')'"));
    }

    // --- Negative number expression ---

    #[test]
    fn test_negative_number() {
        let prog = parse("let x = -5;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                assert!(matches!(
                    value.kind,
                    ExpressionKind::UnaryOp { op: UnOp::Neg, .. }
                ));
            }
            other => panic!("Expected Let with UnaryOp, got {:?}", other),
        }
    }

    // --- Hyphenated plugin call ---

    #[test]
    fn test_parse_hyphenated_plugin_call() {
        let prog = parse(r#"let r = text-llm(prompt, model="gpt-4");"#);
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                if let ExpressionKind::Call {
                    name, args, kwargs, ..
                } = &value.kind
                {
                    assert_eq!(name, "text-llm");
                    assert_eq!(args.len(), 1);
                    assert_eq!(kwargs.len(), 1);
                } else {
                    panic!("Expected Call");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    // --- Multiple operators in expression ---

    #[test]
    fn test_chained_arithmetic() {
        let prog = parse("let r = a + b - c;");
        match &prog.statements[0].kind {
            StatementKind::Let { value, .. } => {
                // (a + b) - c — left-associative
                if let ExpressionKind::BinaryOp { op, left, .. } = &value.kind {
                    assert_eq!(*op, BinOp::Sub);
                    assert!(matches!(
                        left.kind,
                        ExpressionKind::BinaryOp { op: BinOp::Add, .. }
                    ));
                } else {
                    panic!("Expected BinaryOp");
                }
            }
            other => panic!("Expected Let, got {:?}", other),
        }
    }

    #[test]
    fn test_all_comparison_ops() {
        for (op_str, expected_op) in &[
            ("==", BinOp::Eq),
            ("!=", BinOp::NotEq),
            (">", BinOp::Gt),
            ("<", BinOp::Lt),
            (">=", BinOp::GtEq),
            ("<=", BinOp::LtEq),
        ] {
            let code = format!("let r = a {} b;", op_str);
            let prog = parse(&code);
            match &prog.statements[0].kind {
                StatementKind::Let { value, .. } => {
                    if let ExpressionKind::BinaryOp { op, .. } = &value.kind {
                        assert_eq!(op, expected_op, "Failed for operator {}", op_str);
                    } else {
                        panic!("Expected BinaryOp for {}", op_str);
                    }
                }
                _ => panic!("Expected Let for {}", op_str),
            }
        }
    }

    #[test]
    fn test_nested_if_else() {
        let prog = parse("let r = if a { if b { 1 } else { 2 } } else { 3 };");
        assert_eq!(prog.statements.len(), 1);
    }

    // --- Function definitions ---

    #[test]
    fn test_parse_fn_basic() {
        let prog = parse("fn double(x: int64) -> int64 { x * 2 }");
        assert_eq!(prog.statements.len(), 1);
        match &prog.statements[0].kind {
            StatementKind::FunctionDef(def) => {
                assert_eq!(def.name, "double");
                assert_eq!(def.params.len(), 1);
                assert_eq!(def.params[0].name, "x");
                assert_eq!(def.params[0].type_annotation.as_deref(), Some("int64"));
                assert_eq!(def.return_type.as_deref(), Some("int64"));
                assert!(def.body.tail_expr.is_some());
            }
            other => panic!("Expected FunctionDef, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_fn_no_params() {
        let prog = parse("fn greet() { output 42; }");
        match &prog.statements[0].kind {
            StatementKind::FunctionDef(def) => {
                assert_eq!(def.name, "greet");
                assert!(def.params.is_empty());
                assert!(def.return_type.is_none());
            }
            other => panic!("Expected FunctionDef, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_fn_no_return_type() {
        let prog = parse("fn process(items: array) { output items; }");
        match &prog.statements[0].kind {
            StatementKind::FunctionDef(def) => {
                assert_eq!(def.name, "process");
                assert_eq!(def.params.len(), 1);
                assert!(def.return_type.is_none());
            }
            other => panic!("Expected FunctionDef, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_fn_multiple_params() {
        let prog = parse("fn add_nums(a: int64, b: int64) -> int64 { a + b }");
        match &prog.statements[0].kind {
            StatementKind::FunctionDef(def) => {
                assert_eq!(def.params.len(), 2);
                assert_eq!(def.params[0].name, "a");
                assert_eq!(def.params[1].name, "b");
            }
            other => panic!("Expected FunctionDef, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_fn_with_loop_body() {
        let prog = parse("fn sum_arr(items: array) -> int64 { let mut total = 0; for item in items { total = total + item; } total }");
        match &prog.statements[0].kind {
            StatementKind::FunctionDef(def) => {
                assert_eq!(def.name, "sum_arr");
                assert!(def.body.tail_expr.is_some());
            }
            other => panic!("Expected FunctionDef, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_fn_main() {
        let prog = parse("fn main() { let x = 42; output x; }");
        match &prog.statements[0].kind {
            StatementKind::FunctionDef(def) => {
                assert_eq!(def.name, "main");
                assert!(def.params.is_empty());
                assert!(def.return_type.is_none());
            }
            other => panic!("Expected FunctionDef, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_multiple_functions() {
        let prog = parse(
            "fn double(x: int64) -> int64 { x * 2 }\nfn main() { let r = double(21); output r; }",
        );
        assert_eq!(prog.statements.len(), 2);
        assert!(matches!(
            &prog.statements[0].kind,
            StatementKind::FunctionDef(_)
        ));
        assert!(matches!(
            &prog.statements[1].kind,
            StatementKind::FunctionDef(_)
        ));
    }

    #[test]
    fn test_parse_fn_untyped_params() {
        let prog = parse("fn identity(x) { x }");
        match &prog.statements[0].kind {
            StatementKind::FunctionDef(def) => {
                assert_eq!(def.params[0].name, "x");
                assert!(def.params[0].type_annotation.is_none());
            }
            other => panic!("Expected FunctionDef, got {:?}", other),
        }
    }

    // --- Reserved keyword rejection ---

    #[test]
    fn test_reserved_keyword_in_let() {
        let err = parse_err("let trait = 5;");
        assert!(err.message.contains("reserved keyword"));
    }

    #[test]
    fn test_reserved_keyword_in_fn_name() {
        let err = parse_err("fn trait() { 42; }");
        assert!(err.message.contains("reserved keyword"));
    }

    #[test]
    fn test_reserved_keyword_in_fn_param() {
        let err = parse_err("fn foo(yield: int64) { 42; }");
        assert!(err.message.contains("reserved keyword"));
    }

    #[test]
    fn test_reserved_keyword_in_for_variable() {
        let err = parse_err("for yield in items { 0; }");
        assert!(err.message.contains("reserved keyword"));
    }

    #[test]
    fn test_non_exact_keyword_ok() {
        // "spawn_task" is not "spawn" — should work fine
        let prog = parse("let spawn_task = 5;");
        assert_eq!(prog.statements.len(), 1);
    }

    #[test]
    fn test_async_keyword_in_let() {
        let err = parse_err("let async = 5;");
        assert!(err.message.contains("keyword"));
    }

    #[test]
    fn test_parse_fn_error_missing_brace() {
        let err = parse_err("fn broken()");
        assert!(err.message.contains("Expected '{'"));
    }

    // --- Async/Await/Spawn ---

    #[test]
    fn test_parse_async_fn() {
        let prog = parse("async fn fetch() -> int64 { 42 }");
        assert_eq!(prog.statements.len(), 1);
        assert!(matches!(
            prog.statements[0].kind,
            StatementKind::AsyncFunctionDef(_)
        ));
    }

    #[test]
    fn test_parse_async_fn_with_params() {
        let prog = parse("async fn compute(x: int64, y: int64) -> int64 { x + y }");
        if let StatementKind::AsyncFunctionDef(def) = &prog.statements[0].kind {
            assert_eq!(def.name, "compute");
            assert_eq!(def.params.len(), 2);
        } else {
            panic!("Expected AsyncFunctionDef");
        }
    }

    #[test]
    fn test_parse_await_expr() {
        let prog = parse("let v = await f;");
        if let StatementKind::Let { value, .. } = &prog.statements[0].kind {
            assert!(matches!(value.kind, ExpressionKind::Await(_)));
        } else {
            panic!("Expected Let");
        }
    }

    #[test]
    fn test_parse_spawn_expr() {
        let prog = parse("let f = spawn compute();");
        if let StatementKind::Let { value, .. } = &prog.statements[0].kind {
            assert!(matches!(value.kind, ExpressionKind::Spawn(_)));
        } else {
            panic!("Expected Let");
        }
    }

    #[test]
    fn test_parse_spawn_block() {
        let prog = parse("let f = spawn { 42 };");
        if let StatementKind::Let { value, .. } = &prog.statements[0].kind {
            if let ExpressionKind::Spawn(inner) = &value.kind {
                assert!(matches!(inner.kind, ExpressionKind::Block(_)));
            } else {
                panic!("Expected Spawn");
            }
        } else {
            panic!("Expected Let");
        }
    }

    #[test]
    fn test_parse_await_spawn_combined() {
        let prog = parse("let v = await spawn 42;");
        assert_eq!(prog.statements.len(), 1);
    }

    #[test]
    fn test_parse_async_requires_fn() {
        let err = parse_err("async 42;");
        assert!(err.message.contains("Expected 'fn' after 'async'"));
    }

    // --- Parser recursion depth limit (Task #11 fix) ---

    #[test]
    fn test_parse_deeply_nested_parens_within_limit() {
        // 30 levels of nesting — well within the 128 depth limit (~64 paren levels)
        let mut code = String::new();
        for _ in 0..30 {
            code.push('(');
        }
        code.push('1');
        for _ in 0..30 {
            code.push(')');
        }
        code.push(';');
        let prog = parse(&code);
        assert_eq!(prog.statements.len(), 1);
    }

    #[test]
    fn test_parse_exceeds_max_depth() {
        // Run in a thread with a larger stack to avoid stack overflow in debug mode
        let handle = std::thread::Builder::new()
            .stack_size(4 * 1024 * 1024) // 4MB stack
            .spawn(|| {
                let mut code = String::new();
                for _ in 0..70 {
                    code.push('(');
                }
                code.push('1');
                for _ in 0..70 {
                    code.push(')');
                }
                code.push(';');
                let err = parse_err(&code);
                assert!(
                    err.message.contains("nesting exceeds maximum depth"),
                    "Expected depth error, got: {}",
                    err.message
                );
            })
            .unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn test_parse_deeply_nested_binary_ops() {
        // Chained binary ops with increasing precedence trigger recursion
        // via parse_binary_expr(prec+1) calls. 200 chained adds should be fine
        // since same-precedence ops use a loop, not recursion.
        let mut code = "1".to_string();
        for _ in 0..200 {
            code.push_str(" + 1");
        }
        code.push(';');
        // Same-precedence binary ops use a loop, so this should parse successfully
        let prog = parse(&code);
        assert_eq!(prog.statements.len(), 1);
    }

    #[test]
    fn test_parse_fn_with_if_body() {
        let prog = parse("fn abs(x: int64) -> int64 { if x > 0 { x } else { negate(x) } }");
        match &prog.statements[0].kind {
            StatementKind::FunctionDef(def) => {
                assert_eq!(def.name, "abs");
                assert!(def.body.tail_expr.is_some());
            }
            other => panic!("Expected FunctionDef, got {:?}", other),
        }
    }
}
