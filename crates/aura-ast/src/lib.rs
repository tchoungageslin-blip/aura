//! Arena-allocated AST for Aura.
//!
//! Nodes live in per-kind `Vec` arenas and reference each other by 32-bit
//! ids. No `String`/`&str` anywhere in the tree — all text is a `Spur`
//! resolved through the lexer's `Rodeo`. This keeps nodes small and the
//! whole tree trivially `Send`-free, cloneable, and cache-friendly.

mod dump;

pub use dump::dump_items;

use aura_common::Span;
use lasso::Spur;

/// Typed index into an [`Arena`]. Implemented by the `id!` newtypes.
pub trait ArenaId: Copy {
    fn idx(self) -> usize;
    fn from_raw(raw: u32) -> Self;
}

macro_rules! id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub u32);

        impl $name {
            pub const NONE: Self = Self(u32::MAX);
            pub const fn is_none(self) -> bool {
                self.0 == u32::MAX
            }
        }

        impl ArenaId for $name {
            fn idx(self) -> usize {
                self.0 as usize
            }
            fn from_raw(raw: u32) -> Self {
                Self(raw)
            }
        }
    };
}

id!(ExprId);
id!(StmtId);
id!(BlockId);
id!(TypeExprId);

/// A node `T` paired with the `Span` it was parsed from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

/// Vec-indexed arena. The id type is chosen by inference at the call site.
#[derive(Debug, Clone)]
pub struct Arena<T> {
    nodes: Vec<Spanned<T>>,
}

// Manual impl: derived `Default` would wrongly require `T: Default`.
impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

impl<T> Arena<T> {
    pub fn alloc<I: ArenaId>(&mut self, node: T, span: Span) -> I {
        let raw = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX - 1);
        self.nodes.push(Spanned { node, span });
        I::from_raw(raw)
    }

    pub fn get<I: ArenaId>(&self, id: I) -> &T {
        &self.nodes[id.idx()].node
    }

    pub fn span<I: ArenaId>(&self, id: I) -> Span {
        self.nodes[id.idx()].span
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl<T: PartialEq> PartialEq for Arena<T> {
    fn eq(&self, other: &Self) -> bool {
        self.nodes == other.nodes
    }
}

/// All arenas of one parsed file, bundled so functions can pass `&Ast`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Ast {
    pub exprs: Arena<Expr>,
    pub stmts: Arena<Stmt>,
    pub blocks: Arena<Block>,
    pub types: Arena<TypeExpr>,
}

impl Ast {
    pub fn expr(&self, id: ExprId) -> &Expr {
        self.exprs.get(id)
    }
    pub fn expr_span(&self, id: ExprId) -> Span {
        self.exprs.span(id)
    }
    pub fn stmt(&self, id: StmtId) -> &Stmt {
        self.stmts.get(id)
    }
    pub fn stmt_span(&self, id: StmtId) -> Span {
        self.stmts.span(id)
    }
    pub fn block(&self, id: BlockId) -> &Block {
        self.blocks.get(id)
    }
    pub fn block_span(&self, id: BlockId) -> Span {
        self.blocks.span(id)
    }
    pub fn ty(&self, id: TypeExprId) -> &TypeExpr {
        self.types.get(id)
    }
    pub fn ty_span(&self, id: TypeExprId) -> Span {
        self.types.span(id)
    }
}

// ----- expressions ---------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Literal {
    Int(u64),
    Float(f64),
    Bool(bool),
    /// `()`
    Unit,
    /// Cooked string contents (escapes already resolved by the lexer).
    Str(Spur),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Placeholder inserted by parser error recovery.
    Error,
    Literal(Literal),
    Ident(Spur),
    Binary {
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    Unary {
        op: UnOp,
        operand: ExprId,
    },
    Assign {
        target: ExprId,
        value: ExprId,
    },
    Call {
        callee: ExprId,
        args: Vec<ExprId>,
    },
    Field {
        object: ExprId,
        field: Spur,
    },
    /// `else` is either another `If` (else-if chain) or a `Block`.
    If {
        cond: ExprId,
        then_block: BlockId,
        else_branch: Option<ExprId>,
    },
    Block(BlockId),
    Match {
        scrutinee: ExprId,
        arms: Vec<MatchArm>,
    },
    /// `Name { field: expr, .. }` — never produced in expr-head position.
    StructLit {
        name: Spur,
        fields: Vec<(Spur, ExprId)>,
    },
    /// `expr?` — `Result` propagation operator.
    Try {
        expr: ExprId,
    },
    /// `unsafe { .. }` block — ARC ops are not injected inside (Phase 4).
    Unsafe(BlockId),
    /// Parenthesized expression — kept so `(if c {1}) + 2` is allowed while
    /// `if c {1} + 2` is not (block-like exprs can't take binary ops).
    Paren(ExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: ExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `_`
    Wildcard,
    /// binds the value to a name
    Ident(Spur),
    Literal(Literal),
    /// `Variant` or `Variant(a, b)` — resolved to its enum in semantics.
    Variant {
        name: Spur,
        args: Vec<Pattern>,
    },
}

// ----- statements ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: Spur,
        mutable: bool,
        ty: Option<TypeExprId>,
        init: ExprId,
    },
    Expr(ExprId),
    Return(Option<ExprId>),
    While {
        cond: ExprId,
        body: BlockId,
    },
    Loop {
        body: BlockId,
    },
    Break,
    Continue,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<StmtId>,
    /// Trailing expression with no `;` — the block's value.
    pub tail: Option<ExprId>,
    pub span: Span,
}

// ----- types (syntactic) -----------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Error,
    /// `i32`, `f64`, `Point`, `Result` — resolved by name in semantics.
    Named {
        name: Spur,
        generic_args: Vec<TypeExprId>,
    },
    /// `*const T` / `*mut T` — FFI only.
    Pointer {
        mutable: bool,
        pointee: TypeExprId,
    },
    Tuple(Vec<TypeExprId>),
    /// `()` — the unit type.
    Unit,
    /// `!` — the never type (return type of diverging functions).
    Never,
}

// ----- items -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: Spur,
    pub ty: TypeExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDef {
    pub name: Spur,
    pub params: Vec<Param>,
    pub ret: Option<TypeExprId>,
    pub body: Option<BlockId>,
    pub span: Span,
    pub is_extern: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDef {
    pub name: Spur,
    pub ty: TypeExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: Spur,
    pub fields: Vec<FieldDef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VariantPayload {
    None,
    /// `Circle(f64)` — positional fields.
    Tuple(Vec<TypeExprId>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantDef {
    pub name: Spur,
    pub payload: VariantPayload,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: Spur,
    pub variants: Vec<VariantDef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Function(FnDef),
    Struct(StructDef),
    Enum(EnumDef),
    /// `use a.b.c`
    Use {
        path: Vec<Spur>,
        span: Span,
    },
    /// `extern "C" { fn ...; }` — functions have `is_extern` set, body `None`.
    ExternBlock {
        abi: Spur,
        fns: Vec<FnDef>,
        span: Span,
    },
    Error {
        span: Span,
    },
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Function(f) => f.span,
            Item::Struct(s) => s.span,
            Item::Enum(e) => e.span,
            Item::Use { span, .. } | Item::ExternBlock { span, .. } | Item::Error { span } => *span,
        }
    }
}
