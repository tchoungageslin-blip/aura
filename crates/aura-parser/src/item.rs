//! Top-level item parsing: `fn`, `struct`, `enum`, `use`, `extern` blocks.

use aura_ast::{EnumDef, FieldDef, FnDef, Item, Param, StructDef, VariantDef, VariantPayload};
use aura_common::Span;
use aura_common::codes;
use aura_lexer::TokenKind;
use lasso::Spur;

use crate::Parser;

impl Parser<'_> {
    /// One top-level item. `None` → caller synchronizes to the next item.
    pub(crate) fn parse_item(&mut self) -> Option<Item> {
        match self.kind() {
            TokenKind::Fn => self.fn_item(false),
            TokenKind::Struct => self.struct_item(),
            TokenKind::Enum => self.enum_item(),
            TokenKind::Use => Some(self.use_item()),
            TokenKind::Extern => Some(self.extern_block()),
            _ => {
                let t = self.token();
                self.diags.error(
                    codes::PARSE_EXPECTED_ITEM,
                    format!(
                        "expected item (`fn`, `struct`, `enum`, `use`, `extern`), found {}",
                        t.kind.describe()
                    ),
                    t.span,
                );
                None
            }
        }
    }

    /// `fn name(params) (-> ty)? { body }` — or bodiless when `is_extern`.
    fn fn_item(&mut self, is_extern: bool) -> Option<Item> {
        let start = self.bump(); // 'fn'
        let name_tok = self.token();
        let name = if name_tok.kind == TokenKind::Ident {
            self.bump();
            name_tok.sym.unwrap()
        } else {
            self.diags.error(
                codes::PARSE_EXPECTED_IDENT,
                format!("expected function name, found {}", name_tok.kind.describe()),
                name_tok.span,
            );
            return None;
        };
        if self.at(TokenKind::Lt) {
            let t = self.token();
            self.diags.error(
                codes::PARSE_UNEXPECTED_TOKEN,
                "generic functions are not supported yet (post-MVP)",
                t.span,
            );
        }
        let params = self.params();
        let ret = if self.at(TokenKind::Arrow) {
            self.bump();
            Some(self.ty())
        } else {
            None
        };
        self.skip_newlines();
        let body = if self.at(TokenKind::LBrace) {
            Some(self.block())
        } else {
            if !is_extern {
                let t = self.token();
                self.diags.error(
                    codes::PARSE_UNEXPECTED_TOKEN,
                    format!("expected `{{` function body, found {}", t.kind.describe()),
                    t.span,
                );
            }
            None
        };
        let end = body.map_or(self.prev_end(), |b| self.ast.block_span(b).end);
        let span = Span::new(self.file, start.span.start, end);
        Some(Item::Function(FnDef {
            name,
            params,
            ret,
            body,
            span,
            is_extern,
        }))
    }

    /// `(` `name: ty, ...` `)`
    fn params(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        if self
            .expect(TokenKind::LParen, "`(` before parameters")
            .is_none()
        {
            return params;
        }
        self.skip_newlines();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            let name_tok = self.token();
            if name_tok.kind != TokenKind::Ident {
                self.diags.error(
                    codes::PARSE_EXPECTED_IDENT,
                    format!(
                        "expected parameter name, found {}",
                        name_tok.kind.describe()
                    ),
                    name_tok.span,
                );
                break;
            }
            self.bump();
            self.expect(TokenKind::Colon, "`:` after parameter name");
            let ty = self.ty();
            params.push(Param {
                name: name_tok.sym.unwrap(),
                ty,
                span: name_tok.span,
            });
            self.skip_newlines();
            if self.at(TokenKind::Comma) {
                self.bump();
                self.skip_newlines();
            } else if !self.at(TokenKind::RParen) {
                let t = self.token();
                self.diags.error(
                    codes::PARSE_UNEXPECTED_TOKEN,
                    format!(
                        "expected `,` or `)` in parameters, found {}",
                        t.kind.describe()
                    ),
                    t.span,
                );
                break;
            }
        }
        self.expect(TokenKind::RParen, "closing `)` after parameters");
        params
    }

    /// `struct Name { field: ty, ... }`
    fn struct_item(&mut self) -> Option<Item> {
        let start = self.bump(); // 'struct'
        let name = self.item_name("struct")?;
        let mut fields = Vec::new();
        if self
            .expect(TokenKind::LBrace, "`{` before struct fields")
            .is_some()
        {
            self.skip_newlines();
            while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
                let fstart = self.token();
                if fstart.kind == TokenKind::Ident {
                    self.bump();
                    self.expect(TokenKind::Colon, "`:` after field name");
                    let ty = self.ty();
                    fields.push(FieldDef {
                        name: fstart.sym.unwrap(),
                        ty,
                        span: fstart.span,
                    });
                } else {
                    self.diags.error(
                        codes::PARSE_EXPECTED_IDENT,
                        format!("expected field name, found {}", fstart.kind.describe()),
                        fstart.span,
                    );
                    self.bump();
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
                            "expected `,` or `}}` in struct, found {}",
                            t.kind.describe()
                        ),
                        t.span,
                    );
                    self.synchronize_item();
                }
            }
            self.expect(TokenKind::RBrace, "closing `}` after struct fields");
        }
        let span = Span::new(self.file, start.span.start, self.prev_end());
        Some(Item::Struct(StructDef { name, fields, span }))
    }

    /// `enum Name { Variant, Variant2(ty, ty), ... }`
    fn enum_item(&mut self) -> Option<Item> {
        let start = self.bump(); // 'enum'
        let name = self.item_name("enum")?;
        let mut variants = Vec::new();
        if self
            .expect(TokenKind::LBrace, "`{` before enum variants")
            .is_some()
        {
            self.skip_newlines();
            while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
                let vt = self.token();
                if vt.kind == TokenKind::Ident {
                    self.bump();
                    let payload = if self.at(TokenKind::LParen) {
                        self.bump();
                        let mut tys = Vec::new();
                        self.skip_newlines();
                        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
                            tys.push(self.ty());
                            self.skip_newlines();
                            if self.at(TokenKind::Comma) {
                                self.bump();
                                self.skip_newlines();
                            } else if !self.at(TokenKind::RParen) {
                                break;
                            }
                        }
                        self.expect(TokenKind::RParen, "closing `)` in variant");
                        VariantPayload::Tuple(tys)
                    } else {
                        VariantPayload::None
                    };
                    variants.push(VariantDef {
                        name: vt.sym.unwrap(),
                        payload,
                        span: vt.span,
                    });
                } else {
                    self.diags.error(
                        codes::PARSE_EXPECTED_IDENT,
                        format!("expected variant name, found {}", vt.kind.describe()),
                        vt.span,
                    );
                    self.bump();
                }
                self.skip_newlines();
                if self.at(TokenKind::Comma) {
                    self.bump();
                    self.skip_newlines();
                } else if !self.at(TokenKind::RBrace) {
                    let t = self.token();
                    self.diags.error(
                        codes::PARSE_UNEXPECTED_TOKEN,
                        format!("expected `,` or `}}` in enum, found {}", t.kind.describe()),
                        t.span,
                    );
                    self.synchronize_item();
                }
            }
            self.expect(TokenKind::RBrace, "closing `}` after enum variants");
        }
        let span = Span::new(self.file, start.span.start, self.prev_end());
        Some(Item::Enum(EnumDef {
            name,
            variants,
            span,
        }))
    }

    /// `use a.b.c` — dotted path only for now.
    fn use_item(&mut self) -> Item {
        let start = self.bump(); // 'use'
        let mut path: Vec<Spur> = Vec::new();
        loop {
            let t = self.token();
            if t.kind == TokenKind::Ident {
                self.bump();
                path.push(t.sym.unwrap());
                if self.at(TokenKind::Dot) {
                    self.bump();
                    continue;
                }
            } else if path.is_empty() {
                self.diags.error(
                    codes::PARSE_EXPECTED_IDENT,
                    format!("expected path after `use`, found {}", t.kind.describe()),
                    t.span,
                );
            }
            break;
        }
        let span = Span::new(self.file, start.span.start, self.prev_end());
        Item::Use { path, span }
    }

    /// `extern "abi" { fn sig; ... }`
    fn extern_block(&mut self) -> Item {
        let start = self.bump(); // 'extern'
        let abi_tok = self.token();
        let abi = if abi_tok.kind == TokenKind::StringLit {
            self.bump();
            abi_tok.sym.unwrap()
        } else {
            self.diags.error(
                codes::PARSE_UNEXPECTED_TOKEN,
                format!(
                    "expected ABI string after `extern`, found {}",
                    abi_tok.kind.describe()
                ),
                abi_tok.span,
            );
            self.rodeo.get_or_intern("C")
        };
        let mut fns = Vec::new();
        if self
            .expect(TokenKind::LBrace, "`{` before extern block")
            .is_some()
        {
            self.skip_newlines();
            while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
                match self.kind() {
                    TokenKind::Fn => {
                        if let Some(Item::Function(f)) = self.fn_item(true) {
                            fns.push(f);
                        } else {
                            self.synchronize_stmt();
                        }
                    }
                    TokenKind::Semicolon | TokenKind::Newline => {
                        self.bump();
                    }
                    _ => {
                        let t = self.token();
                        self.diags.error(
                            codes::PARSE_EXPECTED_ITEM,
                            format!("expected `fn` in extern block, found {}", t.kind.describe()),
                            t.span,
                        );
                        self.synchronize_stmt();
                    }
                }
                self.skip_newlines();
            }
            self.expect(TokenKind::RBrace, "closing `}` after extern block");
        }
        let span = Span::new(self.file, start.span.start, self.prev_end());
        Item::ExternBlock { abi, fns, span }
    }

    fn item_name(&mut self, what: &str) -> Option<Spur> {
        let t = self.token();
        if t.kind == TokenKind::Ident {
            self.bump();
            Some(t.sym.unwrap())
        } else {
            self.diags.error(
                codes::PARSE_EXPECTED_IDENT,
                format!("expected {what} name, found {}", t.kind.describe()),
                t.span,
            );
            None
        }
    }
}
