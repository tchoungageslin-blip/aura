//! `fn_body` — per-function body as a self-contained cloned arena.
//!
//! The query clones every AST node reachable from the function's `body`
//! block into a fresh [`Ast`]. The clone preserves node `Span`s, so the
//! result is equal across revisions iff the function's subtree is
//! identical *and* unmoved — editing a later item's body does not shift
//! this fn's spans, so its `fn_body` output stays equal and dependents are
//! backdated.
//!
//! `Body` also carries a `Spur → text` map for every identifier in the
//! body, so consumers (name resolution, type checking) never have to read
//! `parsed(..)` — whose output changes on every text edit — just to render
//! a name in a diagnostic. That keeps `fn_body`'s dependents inside the
//! early-cutoff boundary.

use aura_ast::{
    Ast, Block, BlockId, Expr, ExprId, Item, MatchArm, Pattern, Stmt, StmtId, TypeExpr, TypeExprId,
};
use lasso::Spur;
use rustc_hash::FxHashMap;

use crate::{Db, SourceFile, parsed};

/// One function's body, cloned into its own arena. `None` for items without
/// bodies (extern fns, structs, …).
#[derive(Debug, PartialEq)]
pub struct Body {
    /// Arena containing only this body's nodes. Ids are remapped — they do
    /// NOT index into `parsed(..).ast`.
    pub ast: Ast,
    /// Root block of the body inside `ast.blocks`.
    pub root: BlockId,
    /// Text of every `Spur` appearing in this body — diagnostics resolve
    /// names from here without depending on `parsed`'s interner.
    pub names: FxHashMap<Spur, Box<str>>,
    /// Parameter-name `Spur`s, parallel to the `FnSig.params` list —
    /// scope binding keys for the checker.
    pub params: Vec<Spur>,
}

impl Body {
    /// Resolve an identifier to its text (empty string if absent).
    pub fn name(&self, spur: Spur) -> &str {
        self.names.get(&spur).map_or("", Box::as_ref)
    }
}

/// Clone the body of `parsed.items[index]` (must be `Item::Function` with a
/// body; otherwise returns `None`).
#[salsa::tracked(returns(ref))]
pub fn fn_body(db: &dyn Db, file: SourceFile, index: u32) -> Option<Body> {
    let parsed = parsed(db, file);
    let Item::Function(f) = parsed.items.get(index as usize)? else {
        return None;
    };
    let root = f.body?;
    let mut cloner = Cloner {
        src: &parsed.ast,
        src_rodeo: &parsed.rodeo,
        dst: Ast::default(),
        names: FxHashMap::default(),
    };
    let root = cloner.block(root);
    let params = f.params.iter().map(|p| cloner.intern(p.name)).collect();
    Some(Body {
        ast: cloner.dst,
        root,
        names: cloner.names,
        params,
    })
}

/// Deep-copies AST subtrees from `src` into a fresh `dst` arena,
/// collecting identifier text along the way.
struct Cloner<'a> {
    src: &'a Ast,
    src_rodeo: &'a lasso::Rodeo,
    dst: Ast,
    names: FxHashMap<Spur, Box<str>>,
}

impl Cloner<'_> {
    /// Record a `Spur`'s text in the body's name map.
    fn intern(&mut self, spur: Spur) -> Spur {
        self.names
            .entry(spur)
            .or_insert_with(|| self.src_rodeo.resolve(&spur).into());
        spur
    }
}

impl Cloner<'_> {
    fn block(&mut self, id: BlockId) -> BlockId {
        let b = self.src.block(id).clone();
        let block = Block {
            stmts: b.stmts.iter().map(|&s| self.stmt(s)).collect(),
            tail: b.tail.map(|e| self.expr(e)),
            span: b.span,
        };
        self.dst.blocks.alloc(block, self.src.block_span(id))
    }

    fn stmt(&mut self, id: StmtId) -> StmtId {
        let span = self.src.stmt_span(id);
        let stmt = match self.src.stmt(id) {
            Stmt::Let {
                name,
                mutable,
                ty,
                init,
            } => Stmt::Let {
                name: self.intern(*name),
                mutable: *mutable,
                ty: ty.map(|t| self.ty(t)),
                init: self.expr(*init),
            },
            Stmt::Expr(e) => Stmt::Expr(self.expr(*e)),
            Stmt::Return(e) => Stmt::Return(e.map(|e| self.expr(e))),
            Stmt::While { cond, body } => Stmt::While {
                cond: self.expr(*cond),
                body: self.block(*body),
            },
            Stmt::Loop { body } => Stmt::Loop {
                body: self.block(*body),
            },
            Stmt::Break => Stmt::Break,
            Stmt::Continue => Stmt::Continue,
            Stmt::Error => Stmt::Error,
        };
        self.dst.stmts.alloc(stmt, span)
    }

    fn expr(&mut self, id: ExprId) -> ExprId {
        let span = self.src.expr_span(id);
        let expr = match self.src.expr(id) {
            Expr::Error => Expr::Error,
            Expr::Literal(l) => Expr::Literal(*l),
            Expr::Ident(n) => Expr::Ident(self.intern(*n)),
            Expr::Binary { op, lhs, rhs } => Expr::Binary {
                op: *op,
                lhs: self.expr(*lhs),
                rhs: self.expr(*rhs),
            },
            Expr::Unary { op, operand } => Expr::Unary {
                op: *op,
                operand: self.expr(*operand),
            },
            Expr::Assign { target, value } => Expr::Assign {
                target: self.expr(*target),
                value: self.expr(*value),
            },
            Expr::Call { callee, args } => Expr::Call {
                callee: self.expr(*callee),
                args: args.iter().map(|&a| self.expr(a)).collect(),
            },
            Expr::Field { object, field } => Expr::Field {
                object: self.expr(*object),
                field: self.intern(*field),
            },
            Expr::If {
                cond,
                then_block,
                else_branch,
            } => Expr::If {
                cond: self.expr(*cond),
                then_block: self.block(*then_block),
                else_branch: else_branch.map(|e| self.expr(e)),
            },
            Expr::Block(b) => Expr::Block(self.block(*b)),
            Expr::Match { scrutinee, arms } => Expr::Match {
                scrutinee: self.expr(*scrutinee),
                arms: arms
                    .iter()
                    .map(|a| MatchArm {
                        pattern: self.pattern(&a.pattern),
                        body: self.expr(a.body),
                        span: a.span,
                    })
                    .collect(),
            },
            Expr::StructLit { name, fields } => Expr::StructLit {
                name: self.intern(*name),
                fields: fields
                    .iter()
                    .map(|&(f, e)| (self.intern(f), self.expr(e)))
                    .collect(),
            },
            Expr::Try { expr } => Expr::Try {
                expr: self.expr(*expr),
            },
            Expr::Unsafe(b) => Expr::Unsafe(self.block(*b)),
            Expr::Paren(e) => Expr::Paren(self.expr(*e)),
        };
        self.dst.exprs.alloc(expr, span)
    }

    fn pattern(&mut self, p: &Pattern) -> Pattern {
        match p {
            Pattern::Wildcard => Pattern::Wildcard,
            Pattern::Ident(n) => Pattern::Ident(self.intern(*n)),
            Pattern::Literal(l) => Pattern::Literal(*l),
            Pattern::Variant { name, args } => Pattern::Variant {
                name: self.intern(*name),
                args: args.iter().map(|a| self.pattern(a)).collect(),
            },
        }
    }

    fn ty(&mut self, id: TypeExprId) -> TypeExprId {
        let span = self.src.ty_span(id);
        let ty = match self.src.ty(id) {
            TypeExpr::Error => TypeExpr::Error,
            TypeExpr::Named { name, generic_args } => TypeExpr::Named {
                name: self.intern(*name),
                generic_args: generic_args.iter().map(|&a| self.ty(a)).collect(),
            },
            TypeExpr::Pointer { mutable, pointee } => TypeExpr::Pointer {
                mutable: *mutable,
                pointee: self.ty(*pointee),
            },
            TypeExpr::Tuple(tys) => TypeExpr::Tuple(tys.iter().map(|&t| self.ty(t)).collect()),
            TypeExpr::Unit => TypeExpr::Unit,
            TypeExpr::Never => TypeExpr::Never,
        };
        self.dst.types.alloc(ty, span)
    }
}
