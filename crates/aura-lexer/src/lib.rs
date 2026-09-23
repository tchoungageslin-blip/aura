//! Lexer for Aura source files.
//!
//! Produces a flat `Vec<Token>` plus a `lasso::Rodeo` interner holding every
//! identifier, keyword spelling and cooked string literal. The `Rodeo` is
//! owned by [`LexedFile`] and dropped with it — unlike `ustr`, nothing leaks,
//! which matters when the same process lives inside the LSP server.

mod token;

use aura_common::codes;
use aura_common::{Diagnostic, DiagnosticSink, FileId, Span};
use lasso::{Rodeo, Spur};
use unicode_xid::UnicodeXID;

pub use token::TokenKind;

/// A single lexed token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    /// Interned text for `Ident` and `StringLit` (cooked value); `None` else.
    pub sym: Option<Spur>,
}

/// Result of lexing one source file.
pub struct LexedFile {
    pub tokens: Vec<Token>,
    pub rodeo: Rodeo,
    pub diagnostics: Vec<Diagnostic>,
}

/// Lex `src` into tokens. Always terminates; malformed input yields `Error`
/// tokens plus diagnostics — the lexer never panics.
pub fn lex(src: &str, file: FileId) -> LexedFile {
    Lexer::new(src, file).run()
}

struct Lexer<'a> {
    src: &'a str,
    file: FileId,
    /// Byte offset of the next unread character.
    pos: usize,
    tokens: Vec<Token>,
    rodeo: Rodeo,
    diags: DiagnosticSink,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str, file: FileId) -> Self {
        Self {
            src,
            file,
            pos: 0,
            tokens: Vec::with_capacity(src.len() / 4),
            rodeo: Rodeo::default(),
            diags: DiagnosticSink::new(),
        }
    }

    fn run(mut self) -> LexedFile {
        while let Some(c) = self.peek() {
            let start = self.pos;
            match c {
                ' ' | '\t' | '\r' => {
                    self.bump();
                }
                '\n' => {
                    self.bump();
                    self.push(TokenKind::Newline, start, None);
                }
                '/' => {
                    if self.peek_at(1) == Some('/') {
                        self.line_comment();
                    } else if self.peek_at(1) == Some('*') {
                        self.block_comment(start);
                    } else {
                        self.bump();
                        self.push(TokenKind::Slash, start, None);
                    }
                }
                '0'..='9' => self.number(start),
                '"' => self.string(start),
                c if is_ident_start(c) => self.ident_or_keyword(start),
                _ => self.operator_or_punct(start, c),
            }
        }
        self.push(TokenKind::Eof, self.pos, None);
        LexedFile {
            tokens: self.tokens,
            rodeo: self.rodeo,
            diagnostics: self.diags.into_vec(),
        }
    }

    // ----- low-level cursor ------------------------------------------------

    fn peek(&self) -> Option<char> {
        self.src.get(self.pos..)?.chars().next()
    }

    fn peek_at(&self, ahead: usize) -> Option<char> {
        self.src.get(self.pos..)?.chars().nth(ahead)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn span_from(&self, start: usize) -> Span {
        Span::new(self.file, sat_u32(start), sat_u32(self.pos))
    }

    fn push(&mut self, kind: TokenKind, start: usize, sym: Option<Spur>) {
        self.tokens.push(Token {
            kind,
            span: self.span_from(start),
            sym,
        });
    }

    fn error(&mut self, code: &'static str, msg: impl Into<String>, start: usize) {
        let span = self.span_from(start);
        self.diags.error(code, msg, span);
        self.push(TokenKind::Error, start, None);
    }

    // ----- trivia ----------------------------------------------------------

    fn line_comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.bump();
        }
    }

    fn block_comment(&mut self, start: usize) {
        self.bump(); // '/'
        self.bump(); // '*'
        let mut depth = 1u32;
        while let Some(c) = self.bump() {
            match c {
                '*' if self.peek() == Some('/') => {
                    self.bump();
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                '/' if self.peek() == Some('*') => {
                    self.bump();
                    depth += 1;
                }
                _ => {}
            }
        }
        self.diags.error(
            codes::LEX_UNTERMINATED_BLOCK_COMMENT,
            "unterminated block comment",
            self.span_from(start),
        );
    }

    // ----- literals --------------------------------------------------------

    fn number(&mut self, start: usize) {
        let mut is_float = false;
        let mut digits = 0u32;

        if self.peek() == Some('0')
            && matches!(self.peek_at(1), Some('x' | 'X' | 'b' | 'B' | 'o' | 'O'))
        {
            let base_ch = self.peek_at(1).unwrap().to_ascii_lowercase();
            self.bump(); // '0'
            self.bump(); // base letter
            let valid: fn(char) -> bool = match base_ch {
                'x' => is_hex_digit,
                'b' => |c| c == '0' || c == '1',
                _ => |c| ('0'..='7').contains(&c),
            };
            while let Some(c) = self.peek() {
                if c == '_' {
                    self.bump();
                } else if valid(c) {
                    self.bump();
                    digits += 1;
                } else if c.is_ascii_alphanumeric() {
                    self.bump();
                    digits += 1;
                    self.diags.error(
                        codes::LEX_INVALID_NUMBER,
                        format!("invalid digit `{c}` for base-{}", base_name(base_ch)),
                        self.span_from(self.pos.saturating_sub(c.len_utf8())),
                    );
                } else {
                    break;
                }
            }
        } else {
            while let Some(c) = self.peek() {
                match c {
                    '0'..='9' | '_' => {
                        if c != '_' {
                            digits += 1;
                        }
                        self.bump();
                    }
                    // `1.5` is a float, but `1..2` and `1.foo` are not.
                    '.' if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) => {
                        is_float = true;
                        self.bump();
                    }
                    _ => break,
                }
            }
        }

        if digits == 0 {
            self.error(
                codes::LEX_INVALID_NUMBER,
                "number literal with no digits",
                start,
            );
            return;
        }
        let kind = if is_float {
            TokenKind::FloatLit
        } else {
            TokenKind::IntLit
        };
        self.push(kind, start, None);
    }

    fn string(&mut self, start: usize) {
        self.bump(); // opening quote
        let mut cooked = String::new();
        let mut closed = false;
        while let Some(c) = self.peek() {
            match c {
                '"' => {
                    self.bump();
                    closed = true;
                    break;
                }
                '\n' => break, // newline terminates (unterminated string)
                '\\' => {
                    self.bump();
                    match self.bump() {
                        Some('n') => cooked.push('\n'),
                        Some('t') => cooked.push('\t'),
                        Some('r') => cooked.push('\r'),
                        Some('0') => cooked.push('\0'),
                        Some('\\') => cooked.push('\\'),
                        Some('"') => cooked.push('"'),
                        Some(other) => {
                            self.diags.error(
                                codes::LEX_INVALID_ESCAPE,
                                format!("unknown escape `\\{other}`"),
                                self.span_from(self.pos - other.len_utf8() - 1),
                            );
                        }
                        None => break,
                    }
                }
                _ => {
                    cooked.push(c);
                    self.bump();
                }
            }
        }
        if !closed {
            self.diags.error(
                codes::LEX_UNTERMINATED_STRING,
                "unterminated string literal",
                self.span_from(start),
            );
            self.push(TokenKind::Error, start, None);
            return;
        }
        let sym = self.rodeo.get_or_intern(&cooked);
        self.push(TokenKind::StringLit, start, Some(sym));
    }

    // ----- identifiers & keywords ------------------------------------------

    fn ident_or_keyword(&mut self, start: usize) {
        while self.peek().is_some_and(is_ident_continue) {
            self.bump();
        }
        let text = &self.src[start..self.pos];
        if let Some(kind) = keyword_kind(text) {
            self.push(kind, start, None);
        } else {
            let sym = self.rodeo.get_or_intern(text);
            self.push(TokenKind::Ident, start, Some(sym));
        }
    }

    // ----- operators & punctuation ------------------------------------------

    fn operator_or_punct(&mut self, start: usize, c: char) {
        let two = |this: &Self, b: char| this.peek_at(1) == Some(b);
        let kind = match c {
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ',' => TokenKind::Comma,
            ':' => TokenKind::Colon,
            ';' => TokenKind::Semicolon,
            '?' => TokenKind::Question,
            '+' => TokenKind::Plus,
            '%' => TokenKind::Percent,
            '*' => TokenKind::Star,
            '=' if two(self, '>') => TokenKind::FatArrow,
            '=' if two(self, '=') => TokenKind::EqEq,
            '=' => TokenKind::Eq,
            '!' if two(self, '=') => TokenKind::BangEq,
            '!' => TokenKind::Bang,
            '<' if two(self, '=') => TokenKind::LtEq,
            '<' => TokenKind::Lt,
            '>' if two(self, '=') => TokenKind::GtEq,
            '>' => TokenKind::Gt,
            '&' if two(self, '&') => TokenKind::AndAnd,
            '|' if two(self, '|') => TokenKind::OrOr,
            '-' if two(self, '>') => TokenKind::Arrow,
            '-' => TokenKind::Minus,
            '.' if two(self, '.') => TokenKind::DotDot,
            '.' => TokenKind::Dot,
            _ => {
                self.bump();
                self.error(
                    codes::LEX_INVALID_CHAR,
                    format!("unexpected character `{c}`"),
                    start,
                );
                return;
            }
        };
        self.bump();
        if matches!(
            kind,
            TokenKind::EqEq
                | TokenKind::BangEq
                | TokenKind::LtEq
                | TokenKind::GtEq
                | TokenKind::AndAnd
                | TokenKind::OrOr
                | TokenKind::Arrow
                | TokenKind::FatArrow
                | TokenKind::DotDot
        ) {
            self.bump(); // second char of a two-char token
        }
        self.push(kind, start, None);
    }
}

/// usize → u32 with saturation — byte offsets in a >4 GiB file clamp instead
/// of wrapping (diagnostics degrade gracefully, never mislead).
fn sat_u32(x: usize) -> u32 {
    u32::try_from(x).unwrap_or(u32::MAX)
}

fn is_ident_start(c: char) -> bool {
    c == '_' || UnicodeXID::is_xid_start(c) || c.is_ascii_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || UnicodeXID::is_xid_continue(c) || c.is_ascii_alphanumeric()
}

fn is_hex_digit(c: char) -> bool {
    c.is_ascii_hexdigit()
}

fn base_name(b: char) -> u8 {
    match b {
        'x' => 16,
        'b' => 2,
        _ => 8,
    }
}

fn keyword_kind(text: &str) -> Option<TokenKind> {
    Some(match text {
        "fn" => TokenKind::Fn,
        "let" => TokenKind::Let,
        "mut" => TokenKind::Mut,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "while" => TokenKind::While,
        "loop" => TokenKind::Loop,
        "return" => TokenKind::Return,
        "struct" => TokenKind::Struct,
        "enum" => TokenKind::Enum,
        "match" => TokenKind::Match,
        "use" => TokenKind::Use,
        "unsafe" => TokenKind::Unsafe,
        "extern" => TokenKind::Extern,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        _ => return None,
    })
}
