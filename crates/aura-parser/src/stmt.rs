//! Statement and block parsing, including statement-termination (ASI) rules.
//!
//! Termination rule: a statement ends at `;`, or at a newline whose
//! following token cannot continue the expression (`continues_expr`), or at
//! `}`/EOF. Block-like expressions (`if`/`match`/`{}`/`unsafe`) need no
//! terminator. Everything else is `E1001`.

use aura_ast::{Block, BlockId, Expr, ExprId, Stmt, StmtId, TypeExprId};
use aura_common::Span;
use aura_common::codes;
use aura_lexer::TokenKind;
use lasso::Spur;

use crate::{Parser, Restrictions};

/// Result of parsing one statement-slot inside a block.
pub(crate) enum StmtOutcome {
    /// A normal statement.
    Stmt(StmtId),
    /// An expression with no terminator sitting just before `}` — it is the
    /// block's tail value.
    Tail(ExprId),
}

impl Parser<'_> {
    /// `{ stmt* expr? }` — the `{` must be the current token.
    pub(crate) fn block(&mut self) -> BlockId {
        let lbrace = self.token();
        if !self.enter() {
            let span = lbrace.span;
            return self.ast.blocks.alloc(
                Block {
                    stmts: Vec::new(),
                    tail: None,
                    span,
                },
                span,
            );
        }
        if !self.at(TokenKind::LBrace) {
            self.diags.error(
                codes::PARSE_UNEXPECTED_TOKEN,
                format!("expected `{{`, found {}", self.kind().describe()),
                lbrace.span,
            );
            let span = Span::point(self.file, lbrace.span.start);
            return self.ast.blocks.alloc(
                Block {
                    stmts: Vec::new(),
                    tail: None,
                    span,
                },
                span,
            );
        }
        self.bump(); // '{'
        self.skip_newlines();

        let mut stmts = Vec::new();
        let mut tail = None;
        let mut end = lbrace.span.end;
        loop {
            match self.kind() {
                TokenKind::RBrace => {
                    end = self.bump().span.end;
                    break;
                }
                TokenKind::Eof => {
                    self.diags.error(
                        codes::PARSE_UNCLOSED_DELIMITER,
                        "unclosed `{` — expected `}`",
                        lbrace.span,
                    );
                    break;
                }
                _ => {}
            }
            match self.stmt() {
                Some(StmtOutcome::Stmt(s)) => stmts.push(s),
                Some(StmtOutcome::Tail(e)) => {
                    tail = Some(e);
                    // after a tail expr only `}`/EOF may follow
                    self.skip_newlines();
                    if self.at(TokenKind::RBrace) {
                        end = self.bump().span.end;
                    } else if !self.at(TokenKind::Eof) {
                        let t = self.token();
                        self.diags.error(
                            codes::PARSE_UNEXPECTED_TOKEN,
                            format!(
                                "expected `}}` after expression, found {}",
                                t.kind.describe()
                            ),
                            t.span,
                        );
                        self.synchronize_stmt();
                    }
                    break;
                }
                None => self.synchronize_stmt(),
            }
            self.skip_newlines();
        }
        let span = Span::new(self.file, lbrace.span.start, end);
        let id = self.ast.blocks.alloc(Block { stmts, tail, span }, span);
        self.leave();
        id
    }

    /// One statement. Returns `None` when nothing parseable remains (caller
    /// synchronizes).
    fn stmt(&mut self) -> Option<StmtOutcome> {
        match self.kind() {
            TokenKind::Let => Some(StmtOutcome::Stmt(self.let_stmt())),
            TokenKind::Return => Some(StmtOutcome::Stmt(self.return_stmt())),
            TokenKind::While => Some(StmtOutcome::Stmt(self.while_stmt())),
            TokenKind::Loop => Some(StmtOutcome::Stmt(self.loop_stmt())),
            TokenKind::Break => Some(StmtOutcome::Stmt(self.break_or_continue(Stmt::Break))),
            TokenKind::Continue => Some(StmtOutcome::Stmt(self.break_or_continue(Stmt::Continue))),
            TokenKind::Newline | TokenKind::Semicolon => {
                self.bump(); // stray separator — ignore
                self.stmt()
            }
            k if k.starts_expr() => Some(self.expr_stmt()),
            TokenKind::RBrace | TokenKind::Eof => None,
            _ => {
                let t = self.token();
                self.diags.error(
                    codes::PARSE_UNEXPECTED_TOKEN,
                    format!("expected statement, found {}", t.kind.describe()),
                    t.span,
                );
                self.bump();
                let span = t.span;
                Some(StmtOutcome::Stmt(self.ast.stmts.alloc(Stmt::Error, span)))
            }
        }
    }

    fn let_stmt(&mut self) -> StmtId {
        let start = self.bump(); // 'let'
        let mutable = self.at(TokenKind::Mut) && {
            self.bump();
            true
        };
        let name_tok = self.token();
        let name = if name_tok.kind == TokenKind::Ident {
            self.bump();
            name_tok.sym.unwrap()
        } else {
            self.diags.error(
                codes::PARSE_EXPECTED_IDENT,
                format!("expected variable name, found {}", name_tok.kind.describe()),
                name_tok.span,
            );
            self.rodeo.get_or_intern("<error>")
        };
        let ty: Option<TypeExprId> = if self.at(TokenKind::Colon) {
            self.bump();
            Some(self.ty())
        } else {
            None
        };
        let init = if self.at(TokenKind::Eq) {
            self.bump();
            self.expr(0, Restrictions::NONE)
        } else {
            let t = self.token();
            self.diags.error(
                codes::PARSE_UNEXPECTED_TOKEN,
                "expected `=` in `let` — Aura requires an initializer",
                t.span,
            );
            self.err_expr(t.span)
        };
        let end = self.ast.expr_span(init).end;
        self.stmt_end();
        let span = Span::new(self.file, start.span.start, end.max(self.prev_end()));
        self.ast.stmts.alloc(
            Stmt::Let {
                name,
                mutable,
                ty,
                init,
            },
            span,
        )
    }

    fn return_stmt(&mut self) -> StmtId {
        let start = self.bump(); // 'return'
        // ASI: `return` at end of line returns unit.
        let value = match self.kind() {
            TokenKind::Semicolon | TokenKind::Newline | TokenKind::RBrace | TokenKind::Eof => None,
            _ => Some(self.expr(0, Restrictions::NONE)),
        };
        self.stmt_end();
        let span = Span::new(self.file, start.span.start, self.prev_end());
        self.ast.stmts.alloc(Stmt::Return(value), span)
    }

    fn while_stmt(&mut self) -> StmtId {
        let start = self.bump(); // 'while'
        let cond = self.expr(0, Restrictions::NO_STRUCT_LITERAL);
        let body = self.block();
        // no terminator needed after a block-bodied statement
        let span = start.span.merge(self.ast.block_span(body));
        self.ast.stmts.alloc(Stmt::While { cond, body }, span)
    }

    fn loop_stmt(&mut self) -> StmtId {
        let start = self.bump(); // 'loop'
        let body = self.block();
        let span = start.span.merge(self.ast.block_span(body));
        self.ast.stmts.alloc(Stmt::Loop { body }, span)
    }

    fn break_or_continue(&mut self, stmt: Stmt) -> StmtId {
        let tok = self.bump();
        self.stmt_end();
        self.ast.stmts.alloc(stmt, tok.span)
    }

    /// Expression used as a statement, with terminator handling.
    fn expr_stmt(&mut self) -> StmtOutcome {
        let e = self.expr(0, Restrictions::NONE);
        let block_like = matches!(
            self.ast.expr(e),
            Expr::Block(_) | Expr::If { .. } | Expr::Match { .. } | Expr::Unsafe(_)
        );
        let span = self.ast.expr_span(e);
        match self.kind() {
            TokenKind::Semicolon => {
                self.bump();
                StmtOutcome::Stmt(self.ast.stmts.alloc(Stmt::Expr(e), span))
            }
            TokenKind::RBrace | TokenKind::Eof => StmtOutcome::Tail(e),
            TokenKind::Newline => {
                // Look past the newline: `}`/EOF → tail, else plain stmt.
                let save = self.pos;
                self.skip_newlines();
                match self.kind() {
                    TokenKind::RBrace | TokenKind::Eof => {
                        self.pos = save;
                        StmtOutcome::Tail(e)
                    }
                    _ => {
                        self.pos = save;
                        StmtOutcome::Stmt(self.ast.stmts.alloc(Stmt::Expr(e), span))
                    }
                }
            }
            _ if block_like => StmtOutcome::Stmt(self.ast.stmts.alloc(Stmt::Expr(e), span)),
            _ => {
                let t = self.token();
                self.diags.error(
                    codes::PARSE_UNEXPECTED_TOKEN,
                    format!("expected `;` or newline, found {}", t.kind.describe()),
                    t.span,
                );
                StmtOutcome::Stmt(self.ast.stmts.alloc(Stmt::Expr(e), span))
            }
        }
    }

    /// Consume the terminator after `let`/`return`/`break`/`continue`:
    /// `;`, or a newline, or `}`/EOF.
    fn stmt_end(&mut self) {
        match self.kind() {
            TokenKind::Semicolon | TokenKind::Newline => {
                self.bump();
                self.skip_newlines();
            }
            TokenKind::RBrace | TokenKind::Eof => {}
            _ => {
                let t = self.token();
                self.diags.error(
                    codes::PARSE_UNEXPECTED_TOKEN,
                    format!("expected `;` or newline, found {}", t.kind.describe()),
                    t.span,
                );
            }
        }
    }

    /// A type annotation (`ty` in `let x: ty`, params, `->` ret).
    pub(crate) fn ty(&mut self) -> TypeExprId {
        let t = self.token();
        if !self.enter() {
            return self.ast.types.alloc(aura_ast::TypeExpr::Error, t.span);
        }
        let id = self.ty_inner(t);
        self.leave();
        id
    }

    fn ty_inner(&mut self, t: aura_lexer::Token) -> TypeExprId {
        match t.kind {
            TokenKind::Ident => {
                self.bump();
                let name = t.sym.unwrap();
                let mut generic_args = Vec::new();
                if self.at(TokenKind::Lt) {
                    self.bump();
                    loop {
                        if self.at(TokenKind::Gt) {
                            self.bump();
                            break;
                        }
                        if self.at(TokenKind::Eof) {
                            break;
                        }
                        generic_args.push(self.ty());
                        if self.at(TokenKind::Comma) {
                            self.bump();
                        } else if self.at(TokenKind::Gt) {
                            self.bump();
                            break;
                        } else {
                            let bad = self.token();
                            self.diags.error(
                                codes::PARSE_UNEXPECTED_TOKEN,
                                format!(
                                    "expected `,` or `>` in generic args, found {}",
                                    bad.kind.describe()
                                ),
                                bad.span,
                            );
                            break;
                        }
                    }
                }
                let end = self.prev_end();
                self.ast.types.alloc(
                    aura_ast::TypeExpr::Named { name, generic_args },
                    Span::new(self.file, t.span.start, end),
                )
            }
            TokenKind::Star => {
                // `*const T` / `*mut T`
                self.bump();
                let mutable = match self.kind() {
                    TokenKind::Mut => {
                        self.bump();
                        true
                    }
                    TokenKind::Ident if &self.src[self.token().span.range()] == "const" => {
                        self.bump();
                        false
                    }
                    _ => false,
                };
                let pointee = self.ty();
                let span = t.span.merge(self.ast.ty_span(pointee));
                self.ast
                    .types
                    .alloc(aura_ast::TypeExpr::Pointer { mutable, pointee }, span)
            }
            TokenKind::LParen => {
                self.bump();
                self.skip_newlines();
                if self.at(TokenKind::RParen) {
                    let end = self.bump();
                    return self
                        .ast
                        .types
                        .alloc(aura_ast::TypeExpr::Unit, t.span.merge(end.span));
                }
                let first = self.ty();
                let mut tys = vec![first];
                while self.at(TokenKind::Comma) {
                    self.bump();
                    self.skip_newlines();
                    if self.at(TokenKind::RParen) {
                        break;
                    }
                    tys.push(self.ty());
                }
                self.expect(TokenKind::RParen, "closing `)` in type");
                let span = Span::new(self.file, t.span.start, self.prev_end());
                if tys.len() == 1 {
                    return tys[0];
                }
                self.ast.types.alloc(aura_ast::TypeExpr::Tuple(tys), span)
            }
            TokenKind::Bang => {
                self.bump();
                self.ast.types.alloc(aura_ast::TypeExpr::Never, t.span)
            }
            _ => {
                self.diags.error(
                    codes::PARSE_EXPECTED_TYPE,
                    format!("expected type, found {}", t.kind.describe()),
                    t.span,
                );
                self.bump();
                self.ast.types.alloc(aura_ast::TypeExpr::Error, t.span)
            }
        }
    }

    /// Resolve a `Spur` to text — convenience for tests/diagnostics.
    #[allow(dead_code)]
    pub(crate) fn sym_text(&self, spur: Spur) -> &str {
        self.rodeo.resolve(&spur)
    }
}
