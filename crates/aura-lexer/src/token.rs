/// Every token kind the lexer can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    // Keywords
    Fn,
    Let,
    Mut,
    If,
    Else,
    While,
    Loop,
    Return,
    Struct,
    Enum,
    Match,
    Use,
    Unsafe,
    Extern,
    Break,
    Continue,
    True,
    False,

    // Literals & identifiers
    Ident,
    IntLit,
    FloatLit,
    StringLit,

    // Operators
    Plus,     // +
    Minus,    // -
    Star,     // *
    Slash,    // /
    Percent,  // %
    Eq,       // =
    EqEq,     // ==
    Bang,     // !
    BangEq,   // !=
    Lt,       // <
    LtEq,     // <=
    Gt,       // >
    GtEq,     // >=
    AndAnd,   // &&
    OrOr,     // ||
    Arrow,    // ->
    FatArrow, // =>
    Question, // ?

    // Delimiters & punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Semicolon,
    Dot,
    DotDot,

    // Trivia / control
    Newline,
    Eof,
    /// Unlexable input — always accompanied by a diagnostic.
    Error,
}

impl TokenKind {
    /// Can this token start an expression?
    pub fn starts_expr(self) -> bool {
        matches!(
            self,
            TokenKind::Ident
                | TokenKind::IntLit
                | TokenKind::FloatLit
                | TokenKind::StringLit
                | TokenKind::True
                | TokenKind::False
                | TokenKind::LParen
                | TokenKind::LBrace
                | TokenKind::If
                | TokenKind::Match
                | TokenKind::Minus
                | TokenKind::Bang
        )
    }

    /// Can this token extend an expression already on the left?
    /// Used by ASI: a newline ends the statement unless the next token
    /// continues the expression.
    pub fn continues_expr(self) -> bool {
        matches!(
            self,
            TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::Percent
                | TokenKind::Eq
                | TokenKind::EqEq
                | TokenKind::BangEq
                | TokenKind::Lt
                | TokenKind::LtEq
                | TokenKind::Gt
                | TokenKind::GtEq
                | TokenKind::AndAnd
                | TokenKind::OrOr
                | TokenKind::LParen
                | TokenKind::Dot
                | TokenKind::DotDot
                | TokenKind::Question
        )
    }

    pub fn describe(self) -> &'static str {
        match self {
            TokenKind::Fn => "`fn`",
            TokenKind::Let => "`let`",
            TokenKind::Mut => "`mut`",
            TokenKind::If => "`if`",
            TokenKind::Else => "`else`",
            TokenKind::While => "`while`",
            TokenKind::Loop => "`loop`",
            TokenKind::Return => "`return`",
            TokenKind::Struct => "`struct`",
            TokenKind::Enum => "`enum`",
            TokenKind::Match => "`match`",
            TokenKind::Use => "`use`",
            TokenKind::Unsafe => "`unsafe`",
            TokenKind::Extern => "`extern`",
            TokenKind::Break => "`break`",
            TokenKind::Continue => "`continue`",
            TokenKind::True => "`true`",
            TokenKind::False => "`false`",
            TokenKind::Ident => "identifier",
            TokenKind::IntLit => "integer literal",
            TokenKind::FloatLit => "float literal",
            TokenKind::StringLit => "string literal",
            TokenKind::Plus => "`+`",
            TokenKind::Minus => "`-`",
            TokenKind::Star => "`*`",
            TokenKind::Slash => "`/`",
            TokenKind::Percent => "`%`",
            TokenKind::Eq => "`=`",
            TokenKind::EqEq => "`==`",
            TokenKind::Bang => "`!`",
            TokenKind::BangEq => "`!=`",
            TokenKind::Lt => "`<`",
            TokenKind::LtEq => "`<=`",
            TokenKind::Gt => "`>`",
            TokenKind::GtEq => "`>=`",
            TokenKind::AndAnd => "`&&`",
            TokenKind::OrOr => "`||`",
            TokenKind::Arrow => "`->`",
            TokenKind::FatArrow => "`=>`",
            TokenKind::Question => "`?`",
            TokenKind::LParen => "`(`",
            TokenKind::RParen => "`)`",
            TokenKind::LBrace => "`{`",
            TokenKind::RBrace => "`}`",
            TokenKind::LBracket => "`[`",
            TokenKind::RBracket => "`]`",
            TokenKind::Comma => "`,`",
            TokenKind::Colon => "`:`",
            TokenKind::Semicolon => "`;`",
            TokenKind::Dot => "`.`",
            TokenKind::DotDot => "`..`",
            TokenKind::Newline => "newline",
            TokenKind::Eof => "end of file",
            TokenKind::Error => "invalid token",
        }
    }
}
