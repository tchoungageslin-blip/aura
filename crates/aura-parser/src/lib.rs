//! Parser for Aura: iterative Pratt expression parsing + recursive descent
//! for statements and items, with panic-mode error recovery and
//! deterministic ASI (automatic semicolon insertion).
//!
//! Invariants this parser upholds:
//! - never panics on any token stream (malformed input → `Error` nodes)
//! - collects *all* errors in a file, not just the first
//! - struct literals are not parsed in expr-head position
//!   (`if`/`while`/`match` scrutinee) — pitfall #10 of the megaplan
//! - left-associative chains are built by the iterative infix loop, never
//!   by recursion — `1 + 2 + ... + 1_000_000` cannot overflow the stack

mod expr;
mod item;
mod stmt;

use aura_ast::Ast;
use aura_common::{Diagnostic, DiagnosticSink, FileId, Span};
use aura_lexer::{LexedFile, Token, TokenKind};
use lasso::Rodeo;

/// Hard recursion-depth cap for nested expressions/delimiters.
const MAX_DEPTH: u32 = 256;

/// Result of parsing one file. Always returned — even fully broken input
/// produces a `ParsedFile` whose items contain `Error` nodes.
pub struct ParsedFile {
    pub file: FileId,
    pub items: Vec<aura_ast::Item>,
    pub ast: Ast,
    /// String interner carried over from lexing — owns every `Spur`'s text.
    pub rodeo: Rodeo,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParsedFile {
    /// Render the AST as an s-expression tree (debugging / snapshots).
    pub fn dump(&self) -> String {
        aura_ast::dump_items(&self.items, &self.ast, &self.rodeo)
    }
}

/// Parse a whole source file.
pub fn parse_file(src: &str, file: FileId) -> ParsedFile {
    let lexed = aura_lexer::lex(src, file);
    Parser::new(lexed, src).parse()
}

/// Restrictions propagated down the expression parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Restrictions {
    /// When set, `Name { .. }` is NOT parsed as a struct literal — the
    /// brace is the head of a following block (`if x == Point { ... } {`).
    pub no_struct_literal: bool,
}

impl Restrictions {
    pub const NONE: Self = Self {
        no_struct_literal: false,
    };
    pub const NO_STRUCT_LITERAL: Self = Self {
        no_struct_literal: true,
    };
}

pub(crate) struct Parser<'a> {
    pub tokens: Vec<Token>,
    pub pos: usize,
    pub file: FileId,
    pub src: &'a str,
    pub ast: Ast,
    pub rodeo: Rodeo,
    pub diags: DiagnosticSink,
    pub depth: u32,
}

impl<'a> Parser<'a> {
    fn new(lexed: LexedFile, src: &'a str) -> Self {
        let mut diags = DiagnosticSink::new();
        let file = lexed_file_id(&lexed);
        diags.extend(lexed.diagnostics);
        Self {
            tokens: lexed.tokens,
            pos: 0,
            file,
            src,
            ast: Ast::default(),
            rodeo: lexed.rodeo,
            diags,
            depth: 0,
        }
    }

    fn parse(mut self) -> ParsedFile {
        let mut items = Vec::new();
        self.skip_newlines();
        while !self.at(TokenKind::Eof) {
            if self.at(TokenKind::Semicolon) {
                self.bump(); // stray `;` between items — ignore
                continue;
            }
            match self.parse_item() {
                Some(item) => items.push(item),
                None => self.synchronize_item(),
            }
            self.skip_newlines();
        }
        ParsedFile {
            file: self.file,
            items,
            ast: self.ast,
            rodeo: self.rodeo,
            diagnostics: self.diags.into_vec(),
        }
    }

    // ----- token cursor ------------------------------------------------------

    /// Current token kind, skipping nothing (newline-aware callers peek
    /// explicitly).
    fn kind(&self) -> TokenKind {
        self.tokens.get(self.pos).map_or(TokenKind::Eof, |t| t.kind)
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.kind() == kind
    }

    fn token(&self) -> Token {
        self.tokens.get(self.pos).copied().unwrap_or(Token {
            kind: TokenKind::Eof,
            span: Span::point(self.file, u32::MAX),
            sym: None,
        })
    }

    fn bump(&mut self) -> Token {
        let t = self.token();
        if t.kind != TokenKind::Eof {
            self.pos += 1;
        }
        t
    }

    /// Skip `Newline` tokens — used between items/statements, never inside
    /// expression continuation checks (those look *past* newlines).
    fn skip_newlines(&mut self) {
        while self.at(TokenKind::Newline) {
            self.pos += 1;
        }
    }

    /// Eat `expected` or emit an `E1001` diagnostic. Returns the token on
    /// success.
    fn expect(&mut self, expected: TokenKind, what: &str) -> Option<Token> {
        if self.at(expected) {
            return Some(self.bump());
        }
        let t = self.token();
        self.diags.error(
            aura_common::codes::PARSE_UNEXPECTED_TOKEN,
            format!(
                "expected {what} ({}), found {}",
                expected.describe(),
                t.kind.describe()
            ),
            t.span,
        );
        None
    }

    /// Point span at the current token — for "expected X here" errors.
    fn here(&self) -> Span {
        let t = self.token();
        Span::point(self.file, t.span.start)
    }

    // ----- error recovery ------------------------------------------------------

    /// Panic-mode recovery at item level: skip tokens until something that
    /// can start an item (or EOF/RBrace for unbalanced braces inside items).
    fn synchronize_item(&mut self) {
        loop {
            match self.kind() {
                TokenKind::Eof
                | TokenKind::Fn
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Use
                | TokenKind::Extern => return,
                TokenKind::Semicolon | TokenKind::Newline => {
                    self.bump();
                    return;
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    /// Recovery inside a block: stop at statement-ish tokens or `}`.
    fn synchronize_stmt(&mut self) {
        loop {
            match self.kind() {
                TokenKind::Eof | TokenKind::RBrace => return,
                TokenKind::Semicolon | TokenKind::Newline => {
                    self.bump();
                    return;
                }
                _ => {
                    self.bump();
                }
            }
        }
    }
}

/// Extract the `FileId` from a lexed file (all tokens share it).
fn lexed_file_id(lexed: &LexedFile) -> FileId {
    lexed
        .tokens
        .first()
        .map_or(FileId::SYNTHETIC, |t| t.span.file)
}
