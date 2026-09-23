//! aura-hir — the typed, self-contained function body ("HIR" stage).
//!
//! In this pipeline HIR is deliberately thin: [`hir_fn`] packages the
//! per-function [`Body`] (cloned subtree arena) together with its finalized
//! [`FnTypes`] table — i.e. the AST after name resolution and type
//! checking, which is precisely the input MIR lowering needs. Syntactic
//! desugaring (paren erasure, `for`→`while`, method-call lowering) will
//! live here as those features land; keeping a distinct query stage means
//! desugaring gets its own early-cutoff boundary for free.
//!
//! One `hir_fn` edge fans into every MIR consumer, so a body edit that
//! leaves both `fn_body` and `typeck_fn` outputs equal still backdates
//! all downstream MIR queries in one step.

use aura_salsa_db::{Body, Db, Project, SourceFile, fn_body, project_items};
use aura_semantic::{FnTypes, typeck_fn, typeck_project_fn};

/// A function's resolved body + finalized type table.
#[derive(Debug, PartialEq)]
pub struct HirBody {
    /// The function's AST subtree arena (ids are local to `body.ast`).
    pub body: Body,
    /// `exprs[ExprId]` = finalized type; `ret` = declared return type.
    pub types: FnTypes,
}

/// Package `parsed.items[index]`'s checked body. `None` for non-fn items
/// and bodiless extern fns. Run `check_file` first — this query assumes a
/// well-typed body.
#[salsa::tracked(returns(ref))]
pub fn hir_fn(db: &dyn Db, file: SourceFile, index: u32) -> Option<HirBody> {
    let body = fn_body(db, file, index).as_ref()?;
    let types = typeck_fn(db, file, index).as_ref()?;
    Some(HirBody {
        body: body.clone(),
        types: types.clone(),
    })
}

/// Project-path counterpart of [`hir_fn`]: `index` is a global index into
/// `project_items`; the body comes from the declaring file via the item
/// map while the type table was checked against the merged namespace.
#[salsa::tracked(returns(ref))]
pub fn hir_project_fn(db: &dyn Db, project: Project, index: u32) -> Option<HirBody> {
    let (file, local) = project_items(db, project).locate(index)?;
    let body = fn_body(db, file, local).as_ref()?;
    let types = typeck_project_fn(db, project, index).as_ref()?;
    Some(HirBody {
        body: body.clone(),
        types: types.clone(),
    })
}
