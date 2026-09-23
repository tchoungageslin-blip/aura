//! Semantic analysis for Aura: name resolution and type checking on top
//! of the salsa query layer.
//!
//! Queries (all `#[salsa::tracked]`):
//! - [`resolved_file`] — name → [`Def`] table (position-free; early-cutoff
//!   boundary on the signature side)
//! - [`typeck_fn`] — per-function bidirectional type check; depends only
//!   on `fn_body` + `file_items`, so body edits in *other* functions reuse
//!   the memoized result
//! - [`check_file`] — aggregation entry point: parse diagnostics,
//!   redefinitions, then every fn's `typeck_fn`

mod infer;
mod resolve;
mod ty;
mod typeck;

pub use infer::{InferCtx, UnifyError, VarKind};
pub use resolve::{Def, Duplicate, Resolution, resolved_file};
pub use ty::{FloatTy, IntTy, Type, primitive};
pub use typeck::{FnTypes, check_file, lower_typename, typeck_fn};
