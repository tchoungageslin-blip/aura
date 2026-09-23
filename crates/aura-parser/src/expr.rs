//! Pratt expression parser. The infix loop is iterative so left-associative
//! chains of unbounded length cannot overflow the stack; only the right-hand
//! side of operators recurses (with `depth` capped by `MAX_DEPTH`).

use aura_ast::{BinOp, Expr, ExprId, Literal, MatchArm, Pattern, UnOp};
use aura_common::Span;
use aura_common::codes;
use aura_lexer::TokenKind;
use lasso::Spur;

use crate::{MAX_DEPTH, Parser, Restrictions};

/// `(l_bp, r_bp)` for infix operators. Right-associative `=` uses `r_bp ==
/// l_bp`; all binary ops are left-associative (`r_bp == l_bp + 1`).
fn infix_bp(kind: TokenKind) -> Option<(u8, u8)> {
    Some(match kind {
        TokenKind::Eq => (1, 1),
        TokenKind::OrOr => (3, 4),
        TokenKind::AndAnd => (5, 6),
        TokenKind::EqEq | TokenKind::BangEq => (7, 8),
        TokenKind::Lt | TokenKind::LtEq | TokenKind::Gt | TokenKind::GtEq => (9, 10),
        TokenKind::Plus | TokenKind::Minus => (11, 12),
        TokenKind::Star | TokenKind::Slash | TokenKind::Percent => (13, 14),
        _ => return None,
    })
}

fn binop(kind: TokenKind) -> BinOp {
    match kind {
        TokenKind::Plus => BinOp::Add,
        TokenKind::Minus => BinOp::Sub,
        TokenKind::Star => BinOp::Mul,
        TokenKind::Slash => BinOp::Div,
        TokenKind::Percent => BinOp::Rem,
        TokenKind::EqEq => BinOp::Eq,
        TokenKind::BangEq => BinOp::Ne,
        TokenKind::Lt => BinOp::Lt,
        TokenKind::LtEq => BinOp::Le,
        TokenKind::Gt => BinOp::Gt,
        TokenKind::GtEq => BinOp::Ge,
        TokenKind::AndAnd => BinOp::And,
        TokenKind::OrOr => BinOp::Or,
        _ => unreachable!("not a binary op: {kind:?}"),
    }
}

impl Parser<'_> {
    /// Parse an expression with binding power at least `min_bp`.
    pub(crate) fn expr(&mut self, min_bp: u8, r: Restrictions) -> ExprId {
        if !self.enter() {
            let t = self.token();
            return self.err_expr(t.span);
        }
        let id = self.expr_inner(min_bp, r);
        self.leave();
        id
    }

    /// Shared recursion guard. `false` → depth cap hit (diagnostic emitted).
    pub(crate) fn enter(&mut self) -> bool {
        if self.depth >= MAX_DEPTH {
            let t = self.token();
            self.diags
                .error(codes::PARSE_MAX_DEPTH, "nesting too deep", t.span);
            return false;
        }
        self.depth += 1;
        true
    }

    pub(crate) fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn expr_inner(&mut self, min_bp: u8, r: Restrictions) -> ExprId {
        self.skip_newlines();
        let mut lhs = self.prefix(r);
        let lhs_is_block_like = matches!(
            self.ast.expr(lhs),
            Expr::Block(_) | Expr::If { .. } | Expr::Match { .. } | Expr::Unsafe(_)
        );

        loop {
            // ASI-aware continuation: look past newlines for a token that
            // extends this expression; if none, leave pos at the newline.
            let save = self.pos;
            self.skip_newlines();
            let kind = self.kind();
            match kind {
                // Postfix — tightest binding.
                TokenKind::LParen => {
                    let start = self.ast.expr_span(lhs).start;
                    let args = self.call_args();
                    lhs = self.alloc_expr(
                        Expr::Call { callee: lhs, args },
                        Span::new(self.file, start, self.prev_end()),
                    );
                }
                TokenKind::Dot => {
                    self.bump();
                    let start = self.ast.expr_span(lhs).start;
                    let Some(field) = self.field_name() else {
                        break;
                    };
                    lhs = self.alloc_expr(
                        Expr::Field { object: lhs, field },
                        Span::new(self.file, start, self.prev_end()),
                    );
                }
                TokenKind::Question => {
                    self.bump();
                    let start = self.ast.expr_span(lhs).start;
                    lhs = self.alloc_expr(
                        Expr::Try { expr: lhs },
                        Span::new(self.file, start, self.prev_end()),
                    );
                }
                _ => {
                    let Some((l_bp, r_bp)) = infix_bp(kind) else {
                        self.pos = save;
                        break;
                    };
                    if l_bp < min_bp {
                        self.pos = save;
                        break;
                    }
                    // Block-like exprs can't be the left of a binary op —
                    // `if c {1} + 2` would be ambiguous (pitfall #13).
                    if lhs_is_block_like {
                        let t = self.token();
                        self.diags.error(
                            codes::PARSE_BLOCK_IN_EXPR_POS,
                            "binary operator after a block expression — wrap the block in parentheses",
                            t.span,
                        );
                    }
                    self.bump();
                    // Restrictions propagate through the whole sub-expression:
                    // `if x == Point { .. }` must not parse a struct literal
                    // on the right side either (pitfall #10).
                    let rhs = self.expr(r_bp, r);
                    let span = self.ast.expr_span(lhs).merge(self.ast.expr_span(rhs));
                    if kind == TokenKind::Eq {
                        lhs = self.alloc_expr(
                            Expr::Assign {
                                target: lhs,
                                value: rhs,
                            },
                            span,
                        );
                    } else {
                        let op = binop(kind);
                        lhs = self.alloc_expr(Expr::Binary { op, lhs, rhs }, span);
                    }
                }
            }
        }
        lhs
    }

    /// Prefix parslet — literals, idents, groups, unary ops, block forms.
    fn prefix(&mut self, r: Restrictions) -> ExprId {
        let t = self.token();
        match t.kind {
            TokenKind::IntLit => {
                self.bump();
                let lit = self.parse_int_lit(t.span);
                self.alloc_expr(Expr::Literal(lit), t.span)
            }
            TokenKind::FloatLit => {
                self.bump();
                let lit = self.parse_float_lit(t.span);
                self.alloc_expr(Expr::Literal(lit), t.span)
            }
            TokenKind::StringLit => {
                self.bump();
                self.alloc_expr(Expr::Literal(Literal::Str(t.sym.unwrap())), t.span)
            }
            TokenKind::True => {
                self.bump();
                self.alloc_expr(Expr::Literal(Literal::Bool(true)), t.span)
            }
            TokenKind::False => {
                self.bump();
                self.alloc_expr(Expr::Literal(Literal::Bool(false)), t.span)
            }
            TokenKind::Ident => {
                self.bump();
                let name = t.sym.unwrap();
                // Struct literal — only when `{` follows immediately and the
                // restriction isn't in force (pitfall #10).
                if !r.no_struct_literal && self.at(TokenKind::LBrace) {
                    return self.struct_lit(name, t.span);
                }
                self.alloc_expr(Expr::Ident(name), t.span)
            }
            TokenKind::LParen => self.paren_or_unit(t.span),
            TokenKind::Minus => {
                self.bump();
                let operand = self.expr(15, r);
                let span = t.span.merge(self.ast.expr_span(operand));
                self.alloc_expr(
                    Expr::Unary {
                        op: UnOp::Neg,
                        operand,
                    },
                    span,
                )
            }
            TokenKind::Bang => {
                self.bump();
                let operand = self.expr(15, r);
                let span = t.span.merge(self.ast.expr_span(operand));
                self.alloc_expr(
                    Expr::Unary {
                        op: UnOp::Not,
                        operand,
                    },
                    span,
                )
            }
            TokenKind::LBrace => {
                let b = self.block();
                let span = self.ast.block_span(b);
                self.alloc_expr(Expr::Block(b), span)
            }
            TokenKind::If => self.if_expr(t.span),
            TokenKind::Match => self.match_expr(t.span),
            TokenKind::Unsafe => {
                self.bump();
                let b = self.block();
                let span = t.span.merge(self.ast.block_span(b));
                self.alloc_expr(Expr::Unsafe(b), span)
            }
            _ => {
                self.diags.error(
                    codes::PARSE_EXPECTED_EXPR,
                    format!("expected expression, found {}", t.kind.describe()),
                    t.span,
                );
                self.bump();
                self.err_expr(t.span)
            }
        }
    }

    /// `(` expr `)` or `()`.
    fn paren_or_unit(&mut self, start_span: Span) -> ExprId {
        self.bump(); // '('
        self.skip_newlines();
        if self.at(TokenKind::RParen) {
            let end = self.bump();
            let span = start_span.merge(end.span);
            return self.alloc_expr(Expr::Literal(Literal::Unit), span);
        }
        let inner = self.expr(0, Restrictions::NONE);
        self.skip_newlines();
        match self.expect(TokenKind::RParen, "closing `)`") {
            Some(end) => {
                let span = start_span.merge(end.span);
                self.alloc_expr(Expr::Paren(inner), span)
            }
            None => inner, // recovery: pretend parens matched
        }
    }

    /// `Name { field: expr, ... }`
    fn struct_lit(&mut self, name: Spur, name_span: Span) -> ExprId {
        self.bump(); // '{'
        self.skip_newlines();
        let mut fields = Vec::new();
        let mut end = name_span.end;
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let fname_tok = self.token();
            if fname_tok.kind == TokenKind::Ident {
                let fname = fname_tok.sym.unwrap();
                self.bump();
                self.expect(TokenKind::Colon, "`:` after field name");
                let value = self.expr(0, Restrictions::NONE);
                fields.push((fname, value));
            } else {
                self.diags.error(
                    codes::PARSE_EXPECTED_IDENT,
                    format!("expected field name, found {}", fname_tok.kind.describe()),
                    fname_tok.span,
                );
                self.synchronize_struct_field();
            }
            self.skip_newlines();
            if self.at(TokenKind::Comma) {
                self.bump();
                self.skip_newlines();
            } else if !self.at(TokenKind::RBrace) {
                let t = self.token();
                self.diags.error(
                    codes::PARSE_UNEXPECTED_TOKEN,
                    format!(
                        "expected `,` or `}}` in struct literal, found {}",
                        t.kind.describe()
                    ),
                    t.span,
                );
                self.synchronize_struct_field();
            }
        }
        if self.at(TokenKind::RBrace) {
            end = self.bump().span.end;
        } else {
            self.diags.error(
                codes::PARSE_UNCLOSED_DELIMITER,
                "unclosed `{` in struct literal",
                name_span,
            );
        }
        let span = Span::new(self.file, name_span.start, end);
        self.alloc_expr(Expr::StructLit { name, fields }, span)
    }

    fn synchronize_struct_field(&mut self) {
        loop {
            match self.kind() {
                TokenKind::Comma | TokenKind::Newline => {
                    self.bump();
                    return;
                }
                TokenKind::RBrace | TokenKind::Eof => return,
                _ => {
                    self.bump();
                }
            }
        }
    }

    /// `if cond { } [else (if | { })]` — expression form.
    fn if_expr(&mut self, if_span: Span) -> ExprId {
        self.bump(); // 'if'
        let cond = self.expr(0, Restrictions::NO_STRUCT_LITERAL);
        let then_block = self.block();
        let mut end = self.ast.block_span(then_block).end;
        // `else` may sit on its own line.
        let save = self.pos;
        self.skip_newlines();
        let else_branch = if self.at(TokenKind::Else) {
            self.bump();
            self.skip_newlines();
            if self.at(TokenKind::If) {
                let s = self.token().span;
                let e = self.if_expr(s);
                end = self.ast.expr_span(e).end;
                Some(e)
            } else {
                let b = self.block();
                let bspan = self.ast.block_span(b);
                end = bspan.end;
                let e = self.alloc_expr(Expr::Block(b), bspan);
                Some(e)
            }
        } else {
            self.pos = save;
            None
        };
        let span = Span::new(self.file, if_span.start, end);
        self.alloc_expr(
            Expr::If {
                cond,
                then_block,
                else_branch,
            },
            span,
        )
    }

    /// `match scrutinee { pat => expr, ... }`
    fn match_expr(&mut self, match_span: Span) -> ExprId {
        self.bump(); // 'match'
        let scrutinee = self.expr(0, Restrictions::NO_STRUCT_LITERAL);
        let mut arms: Vec<MatchArm> = Vec::new();
        let mut end = self.ast.expr_span(scrutinee).end;
        if self
            .expect(TokenKind::LBrace, "`{` after match scrutinee")
            .is_none()
        {
            let span = Span::new(self.file, match_span.start, end);
            return self.alloc_expr(Expr::Match { scrutinee, arms }, span);
        }
        self.skip_newlines();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let arm_start = self.token().span.start;
            let pattern = self.pattern();
            self.expect(TokenKind::FatArrow, "`=>` after match pattern");
            let body = self.expr(0, Restrictions::NONE);
            let arm_span = Span::new(self.file, arm_start, self.ast.expr_span(body).end);
            arms.push(MatchArm {
                pattern,
                body,
                span: arm_span,
            });
            self.skip_newlines();
            if self.at(TokenKind::Comma) {
                self.bump();
            }
            self.skip_newlines();
        }
        if self.at(TokenKind::RBrace) {
            end = self.bump().span.end;
        } else {
            self.diags.error(
                codes::PARSE_UNCLOSED_DELIMITER,
                "unclosed `{` in match expression",
                match_span,
            );
        }
        let span = Span::new(self.file, match_span.start, end);
        self.alloc_expr(Expr::Match { scrutinee, arms }, span)
    }

    /// `pat := '_' | literal | ident | Variant '(' pat, ... ')'`
    pub(crate) fn pattern(&mut self) -> Pattern {
        let t = self.token();
        match t.kind {
            TokenKind::Ident if &self.src[t.span.range()] == "_" => {
                self.bump();
                Pattern::Wildcard
            }
            TokenKind::Ident => {
                self.bump();
                let name = t.sym.unwrap();
                if self.at(TokenKind::LParen) {
                    self.bump();
                    self.skip_newlines();
                    let mut args = Vec::new();
                    while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
                        args.push(self.pattern());
                        self.skip_newlines();
                        if self.at(TokenKind::Comma) {
                            self.bump();
                            self.skip_newlines();
                        } else if !self.at(TokenKind::RParen) {
                            let bad = self.token();
                            self.diags.error(
                                codes::PARSE_UNEXPECTED_TOKEN,
                                format!(
                                    "expected `,` or `)` in pattern, found {}",
                                    bad.kind.describe()
                                ),
                                bad.span,
                            );
                            break;
                        }
                    }
                    self.expect(TokenKind::RParen, "closing `)` in pattern");
                    Pattern::Variant { name, args }
                } else {
                    Pattern::Ident(name)
                }
            }
            TokenKind::IntLit => {
                self.bump();
                Pattern::Literal(self.parse_int_lit(t.span))
            }
            TokenKind::FloatLit => {
                self.bump();
                Pattern::Literal(self.parse_float_lit(t.span))
            }
            TokenKind::StringLit => {
                self.bump();
                Pattern::Literal(Literal::Str(t.sym.unwrap()))
            }
            TokenKind::True | TokenKind::False => {
                self.bump();
                Pattern::Literal(Literal::Bool(t.kind == TokenKind::True))
            }
            _ => {
                self.diags.error(
                    codes::PARSE_UNEXPECTED_TOKEN,
                    format!("expected pattern, found {}", t.kind.describe()),
                    t.span,
                );
                self.bump();
                Pattern::Wildcard // recovery: wildcard never fails later
            }
        }
    }

    // ----- helpers -----------------------------------------------------------

    fn call_args(&mut self) -> Vec<ExprId> {
        self.bump(); // '('
        self.skip_newlines();
        let mut args = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            args.push(self.expr(0, Restrictions::NONE));
            self.skip_newlines();
            if self.at(TokenKind::Comma) {
                self.bump();
                self.skip_newlines();
            } else if !self.at(TokenKind::RParen) {
                let t = self.token();
                self.diags.error(
                    codes::PARSE_UNEXPECTED_TOKEN,
                    format!(
                        "expected `,` or `)` in call arguments, found {}",
                        t.kind.describe()
                    ),
                    t.span,
                );
                break;
            }
        }
        if self.at(TokenKind::RParen) {
            self.bump();
        } else {
            self.diags.error(
                codes::PARSE_UNCLOSED_DELIMITER,
                "unclosed `(` in call arguments",
                self.here(),
            );
        }
        args
    }

    fn field_name(&mut self) -> Option<Spur> {
        let t = self.token();
        if t.kind == TokenKind::Ident {
            self.bump();
            Some(t.sym.unwrap())
        } else {
            self.diags.error(
                codes::PARSE_EXPECTED_IDENT,
                format!("expected field name after `.`, found {}", t.kind.describe()),
                t.span,
            );
            None
        }
    }

    pub(crate) fn parse_int_lit(&self, span: Span) -> Literal {
        let text: String = self.src[span.range()].replace('_', "");
        let (digits, radix) = if let Some(d) = text.strip_prefix("0x") {
            (d, 16)
        } else if let Some(d) = text.strip_prefix("0b") {
            (d, 2)
        } else if let Some(d) = text.strip_prefix("0o") {
            (d, 8)
        } else {
            (text.as_str(), 10)
        };
        match u64::from_str_radix(digits, radix) {
            Ok(v) => Literal::Int(v),
            Err(_) => Literal::Int(u64::MAX), // diag already emitted by lexer
        }
    }

    fn parse_float_lit(&self, span: Span) -> Literal {
        let text: String = self.src[span.range()].replace('_', "");
        Literal::Float(text.parse().unwrap_or(f64::NAN))
    }

    pub(crate) fn alloc_expr(&mut self, expr: Expr, span: Span) -> ExprId {
        self.ast.exprs.alloc(expr, span)
    }

    pub(crate) fn err_expr(&mut self, span: Span) -> ExprId {
        self.alloc_expr(Expr::Error, span)
    }

    /// Byte offset just past the previous token.
    pub(crate) fn prev_end(&self) -> u32 {
        if self.pos == 0 {
            return 0;
        }
        self.tokens[self.pos - 1].span.end
    }
}
