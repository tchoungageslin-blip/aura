use aura_common::FileId;
use aura_lexer::{TokenKind, lex};

fn kinds(src: &str) -> Vec<TokenKind> {
    lex(src, FileId(0)).tokens.iter().map(|t| t.kind).collect()
}

/// Token kinds without the trailing `Eof`.
fn kinds_no_eof(src: &str) -> Vec<TokenKind> {
    let mut k = kinds(src);
    assert_eq!(k.last(), Some(&TokenKind::Eof));
    k.pop();
    k
}

fn diag_codes(src: &str) -> Vec<&'static str> {
    lex(src, FileId(0))
        .diagnostics
        .iter()
        .filter_map(|d| d.code)
        .collect()
}

#[test]
fn empty_file() {
    assert_eq!(kinds(""), vec![TokenKind::Eof]);
    assert_eq!(kinds("   \n  "), vec![TokenKind::Newline, TokenKind::Eof]);
}

#[test]
fn keywords_and_idents() {
    let src = "fn let mut if else while loop return struct enum match use unsafe extern break continue true false";
    let got = kinds_no_eof(src);
    assert_eq!(
        got,
        vec![
            TokenKind::Fn,
            TokenKind::Let,
            TokenKind::Mut,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::While,
            TokenKind::Loop,
            TokenKind::Return,
            TokenKind::Struct,
            TokenKind::Enum,
            TokenKind::Match,
            TokenKind::Use,
            TokenKind::Unsafe,
            TokenKind::Extern,
            TokenKind::Break,
            TokenKind::Continue,
            TokenKind::True,
            TokenKind::False,
        ]
    );
    // prefix of a keyword is an ident
    assert_eq!(
        kinds_no_eof("fnx letter mutiny"),
        vec![TokenKind::Ident, TokenKind::Ident, TokenKind::Ident]
    );
}

#[test]
fn integer_literals() {
    assert_eq!(
        kinds_no_eof("0 42 1_000_000"),
        vec![TokenKind::IntLit, TokenKind::IntLit, TokenKind::IntLit]
    );
    assert_eq!(
        kinds_no_eof("0xFF 0b101 0o17"),
        vec![TokenKind::IntLit, TokenKind::IntLit, TokenKind::IntLit]
    );
    assert_eq!(kinds_no_eof("0x"), vec![TokenKind::Error]);
    assert!(diag_codes("0x").contains(&aura_common::codes::LEX_INVALID_NUMBER));
    // invalid digit in base literal
    assert!(diag_codes("0b12").contains(&aura_common::codes::LEX_INVALID_NUMBER));
}

#[test]
fn float_literals() {
    assert_eq!(
        kinds_no_eof("1.5 0.25 1_000.5"),
        vec![
            TokenKind::FloatLit,
            TokenKind::FloatLit,
            TokenKind::FloatLit
        ]
    );
    // `1.` is int + dot, `1..2` is int dotdot int — no ambiguity
    assert_eq!(
        kinds_no_eof("1.x"),
        vec![TokenKind::IntLit, TokenKind::Dot, TokenKind::Ident]
    );
    assert_eq!(
        kinds_no_eof("1..2"),
        vec![TokenKind::IntLit, TokenKind::DotDot, TokenKind::IntLit]
    );
}

#[test]
fn string_literals() {
    let f = lex(r#""hello" "a\nb\t\"q\"""#, FileId(0));
    let strs: Vec<_> = f
        .tokens
        .iter()
        .filter(|t| t.kind == TokenKind::StringLit)
        .collect();
    assert_eq!(strs.len(), 2);
    assert_eq!(f.rodeo.resolve(&strs[0].sym.unwrap()), "hello");
    assert_eq!(f.rodeo.resolve(&strs[1].sym.unwrap()), "a\nb\t\"q\"");
}

#[test]
fn unterminated_string() {
    assert!(diag_codes("\"abc").contains(&aura_common::codes::LEX_UNTERMINATED_STRING));
    // newline also terminates-with-error
    assert!(diag_codes("\"abc\n").contains(&aura_common::codes::LEX_UNTERMINATED_STRING));
    assert!(diag_codes("\"a\\q\"").contains(&aura_common::codes::LEX_INVALID_ESCAPE));
}

#[test]
fn comments() {
    assert_eq!(
        kinds_no_eof("x // comment\ny"),
        vec![TokenKind::Ident, TokenKind::Newline, TokenKind::Ident]
    );
    assert_eq!(
        kinds_no_eof("a /* b */ c"),
        vec![TokenKind::Ident, TokenKind::Ident]
    );
    // nested block comments
    assert_eq!(
        kinds_no_eof("a /* x /* y */ z */ c"),
        vec![TokenKind::Ident, TokenKind::Ident]
    );
    assert!(
        diag_codes("/* unclosed").contains(&aura_common::codes::LEX_UNTERMINATED_BLOCK_COMMENT)
    );
    assert!(
        diag_codes("/* a /* b */").contains(&aura_common::codes::LEX_UNTERMINATED_BLOCK_COMMENT)
    );
}

#[test]
fn operators_disambiguation() {
    let src = "= == => ! != < <= > >= && || -> + - * / % ? . ..";
    assert_eq!(
        kinds_no_eof(src),
        vec![
            TokenKind::Eq,
            TokenKind::EqEq,
            TokenKind::FatArrow,
            TokenKind::Bang,
            TokenKind::BangEq,
            TokenKind::Lt,
            TokenKind::LtEq,
            TokenKind::Gt,
            TokenKind::GtEq,
            TokenKind::AndAnd,
            TokenKind::OrOr,
            TokenKind::Arrow,
            TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::Percent,
            TokenKind::Question,
            TokenKind::Dot,
            TokenKind::DotDot,
        ]
    );
}

#[test]
fn invalid_chars_recover() {
    let f = lex("a @ b", FileId(0));
    let ks: Vec<_> = f.tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        ks,
        vec![
            TokenKind::Ident,
            TokenKind::Error,
            TokenKind::Ident,
            TokenKind::Eof
        ]
    );
    assert!(
        f.diagnostics
            .iter()
            .any(|d| d.code == Some(aura_common::codes::LEX_INVALID_CHAR))
    );
}

#[test]
fn unicode_idents() {
    let f = lex("café 变量 _x9", FileId(0));
    let idents: Vec<_> = f
        .tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Ident)
        .collect();
    assert_eq!(idents.len(), 3);
    assert_eq!(f.rodeo.resolve(&idents[0].sym.unwrap()), "café");
    assert_eq!(f.rodeo.resolve(&idents[1].sym.unwrap()), "变量");
}

#[test]
fn spans_are_byte_offsets() {
    let f = lex("é x", FileId(0));
    // 'é' is 2 bytes: ident at [0,2), 'x' at [3,4)
    assert_eq!(f.tokens[0].span.range(), 0..2);
    assert_eq!(f.tokens[1].span.range(), 3..4);
}

#[test]
fn interning_dedupes() {
    let f = lex("foo foo foo", FileId(0));
    let idents: Vec<_> = f.tokens.iter().filter_map(|t| t.sym).collect();
    assert_eq!(idents.len(), 3);
    assert!(idents.iter().all(|s| *s == idents[0]));
}

#[test]
fn never_panics_on_garbage() {
    let garbage: &[&str] = &[
        "\u{0}\u{1}\u{2}",
        "\"",
        "/*",
        "0x",
        "‘’“”",
        "fn(((((",
        "a...b",
        "*/",
        "\\",
    ];
    for g in garbage {
        let _ = lex(g, FileId(0)); // must not panic
    }
}
