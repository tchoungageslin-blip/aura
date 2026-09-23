//! aura-mir — mid-level IR between the typed AST and Cranelift.
//!
//! [`mir_fn`] lowers one checked function body into a flat control-flow
//! graph: expression trees become temporaries ([`MirLocal`]) and
//! `if`/`while`/`return` become explicit [`MirTerm`] edges between
//! [`MirBlock`]s. The query depends on `typeck_fn` (finalized expression
//! types) and `fn_body` (the subtree arena), so salsa's early cutoff makes
//! body edits re-lower only the touched function.
//!
//! Local layout: `_0` is the return place, `_1..=param_count` are the
//! parameters, then user `let` bindings and temporaries follow in
//! evaluation order. Aggregate-typed locals (structs, later enums) hold
//! *addresses* — codegen allocates their storage.

mod dump;
mod lower;

pub use dump::dump;
pub use lower::mir_fn;

use aura_ast::{BinOp, UnOp};
use aura_common::Diagnostic;
use aura_semantic::{FloatTy, IntTy, Type};

/// Basic-block id — index into [`MirBody::blocks`].
pub type BbId = u32;
/// Local id — index into [`MirBody::locals`].
pub type LocalId = u32;

/// MIR for one function body.
#[derive(Debug, Clone, PartialEq)]
pub struct MirBody {
    /// Function name (item name from `FileItems`).
    pub name: String,
    /// `locals[0]` is the return place; `locals[1..=param_count]` params.
    pub locals: Vec<MirLocal>,
    /// Number of source-level parameters.
    pub param_count: u32,
    /// Basic blocks; `blocks[0]` is the entry block.
    pub blocks: Vec<MirBlock>,
    /// Declared return type (aggregate returns lower to a hidden out-param
    /// at codegen — MIR stays value-shaped).
    pub ret: Type,
    /// Lowering-time diagnostics (`E3004` unsupported constructs). A
    /// non-empty list means the body must not reach codegen.
    pub diagnostics: Vec<Diagnostic>,
}

/// One MIR local: a parameter, `let` binding, or compiler temporary.
#[derive(Debug, Clone, PartialEq)]
pub struct MirLocal {
    pub ty: Type,
    /// Source name for dumps/diagnostics (`None` for temporaries).
    pub name: Option<String>,
    pub mutable: bool,
}

/// A basic block: straight-line statements, then exactly one terminator.
#[derive(Debug, Clone, PartialEq)]
pub struct MirBlock {
    pub stmts: Vec<MirStmt>,
    pub term: MirTerm,
}

/// Non-terminator statement.
#[derive(Debug, Clone, PartialEq)]
pub enum MirStmt {
    /// `place = rvalue`
    Assign(Place, Rvalue),
    /// Keeps block shapes stable when a source stmt lowers to nothing.
    Nop,
}

/// Block terminator — control flow lives only here.
#[derive(Debug, Clone, PartialEq)]
pub enum MirTerm {
    /// Unconditional edge.
    Goto(BbId),
    /// `brif cond [then] else [else_]`
    Branch {
        cond: Operand,
        then: BbId,
        else_: BbId,
    },
    /// Return `_0`.
    Return,
    /// Dead end — marks unreachable continuations (post-`return` code).
    Unreachable,
}

/// A storage location: a local plus field projections.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub local: LocalId,
    pub proj: Vec<Proj>,
}

impl Place {
    /// Bare `local` with no projections.
    pub fn local(local: LocalId) -> Self {
        Self {
            local,
            proj: Vec::new(),
        }
    }
}

/// Place projection — struct field access by field index.
#[derive(Debug, Clone, PartialEq)]
pub enum Proj {
    /// `.0`, `.x` — index into the struct's declared field list.
    Field(u32),
}

/// A value producer: read a place or a constant.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Place(Place),
    Const(Const),
}

/// A constant scalar.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Int(u64, IntTy),
    Float(f64, FloatTy),
    Bool(bool),
    Unit,
}

/// Right-hand side of an assignment.
#[derive(Debug, Clone, PartialEq)]
pub enum Rvalue {
    /// Copy/move a scalar or aggregate reference.
    Use(Operand),
    Unary(UnOp, Operand),
    Binary(BinOp, Operand, Operand),
    /// `f(a, b)` — callee is a file item, never a first-class pointer yet.
    Call(Callee, Vec<Operand>),
    /// `Struct { f: v, .. }` — writes each field into the destination's
    /// storage. `fields` pairs field *indices* (into the struct's declared
    /// field list) with their operands.
    StructLit {
        /// `FileItems` index of the struct definition.
        item: u32,
        fields: Vec<(u32, Operand)>,
    },
}

/// What a call target resolves to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Callee {
    /// `fn` defined in this file — payload is the `FileItems` index.
    Fn(u32),
    /// `extern` fn — `(extern-block item index, fn index inside it)`.
    Extern(u32, u32),
}
