//! `file_items` — the signature-level view of a source file.
//!
//! Everything in [`FileItems`] is **position-free**: names are resolved to
//! `String`, types to [`TypeName`], and no `Span` appears anywhere. A text
//! edit that only touches function bodies (or whitespace) produces an equal
//! `FileItems`, so salsa backdates it and every dependent query is spared —
//! this is the signature/body split that powers incremental checking.

use aura_ast::{Item, TypeExpr, TypeExprId};
use aura_parser::ParsedFile;

use crate::{Db, SourceFile, parsed};

/// A syntactic type with names resolved to text and no positions.
/// Conversion happens once in `file_items`; semantic lowering maps
/// `TypeName` → `semantic::Type`.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeName {
    /// Unparseable / missing type syntax.
    Error,
    /// `Name`, `Name<T, ..>` — primitive (`i32`) or user type (`Point`).
    Named {
        name: String,
        args: Vec<TypeName>,
    },
    /// `*const T` / `*mut T`.
    Pointer {
        mutable: bool,
        pointee: Box<TypeName>,
    },
    Tuple(Vec<TypeName>),
    Unit,
    Never,
}

impl TypeName {
    /// Lower a parsed [`TypeExpr`] into a position-free `TypeName`.
    fn from_ast(ast: &aura_ast::Ast, rodeo: &lasso::Rodeo, id: TypeExprId) -> Self {
        match ast.ty(id) {
            TypeExpr::Error => TypeName::Error,
            TypeExpr::Named { name, generic_args } => TypeName::Named {
                name: rodeo.resolve(name).to_owned(),
                args: generic_args
                    .iter()
                    .map(|&a| TypeName::from_ast(ast, rodeo, a))
                    .collect(),
            },
            TypeExpr::Pointer { mutable, pointee } => TypeName::Pointer {
                mutable: *mutable,
                pointee: Box::new(TypeName::from_ast(ast, rodeo, *pointee)),
            },
            TypeExpr::Tuple(tys) => TypeName::Tuple(
                tys.iter()
                    .map(|&t| TypeName::from_ast(ast, rodeo, t))
                    .collect(),
            ),
            TypeExpr::Unit => TypeName::Unit,
            TypeExpr::Never => TypeName::Never,
        }
    }
}

/// One function parameter: `name: ty`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamSig {
    pub name: String,
    pub ty: TypeName,
}

/// Signature-level description of one top-level item.
///
/// `file_items(db, file).items[i]` always corresponds to
/// `parsed(db, file).items[i]` — same order, same count (parse errors become
/// [`ItemSig::Error`]).
#[derive(Debug, Clone, PartialEq)]
pub enum ItemSig {
    Fn {
        name: String,
        params: Vec<ParamSig>,
        ret: Option<TypeName>,
        /// `fn` declared `extern` (no body) — parser sets this on `FnDef`.
        is_extern: bool,
    },
    Struct {
        name: String,
        fields: Vec<ParamSig>,
    },
    Enum {
        name: String,
        /// `(variant name, positional payload types)`.
        variants: Vec<(String, Vec<TypeName>)>,
    },
    /// `use a.b.c` — path segments.
    Use {
        path: Vec<String>,
    },
    /// `extern "abi" { fn ...; ... }` — whole block in one slot so indices
    /// stay aligned with the parsed items (extern fns never have bodies).
    ExternBlock {
        abi: String,
        fns: Vec<ExternFnSig>,
    },
    /// Parser produced `Item::Error`.
    Error,
}

/// Signature of one function inside an `extern` block.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternFnSig {
    pub name: String,
    pub params: Vec<ParamSig>,
    pub ret: Option<TypeName>,
}

impl ItemSig {
    /// Item name for namespace purposes (`fn`/`struct`/`enum`), if any.
    pub fn name(&self) -> Option<&str> {
        match self {
            ItemSig::Fn { name, .. }
            | ItemSig::Struct { name, .. }
            | ItemSig::Enum { name, .. } => Some(name),
            ItemSig::Use { .. } | ItemSig::ExternBlock { .. } | ItemSig::Error => None,
        }
    }
}

/// Position-free signature tree of one file — the early-cutoff boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct FileItems {
    /// One entry per parsed top-level item, in source order.
    pub items: Vec<ItemSig>,
}

impl FileItems {
    /// Iterate `(ast_index, sig)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &ItemSig)> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, s)| (u32::try_from(i).unwrap_or(u32::MAX), s))
    }

    /// Find an item's index by name (first match wins; duplicates are
    /// diagnosed by the resolver).
    pub fn find(&self, name: &str) -> Option<u32> {
        self.iter()
            .find(|(_, s)| s.name() == Some(name))
            .map(|(i, _)| i)
    }
}

/// Extract the signature tree of `file`. Depends only on signature text —
/// body edits yield an equal result and are backdated by salsa.
#[salsa::tracked(returns(ref))]
pub fn file_items(db: &dyn Db, file: SourceFile) -> FileItems {
    let parsed = parsed(db, file);
    extract_items(parsed)
}

fn extract_items(parsed: &ParsedFile) -> FileItems {
    let ast = &parsed.ast;
    let rodeo = &parsed.rodeo;
    let mut items = Vec::with_capacity(parsed.items.len());
    for item in &parsed.items {
        let sig = match item {
            Item::Function(f) => fn_sig(ast, rodeo, f),
            Item::Struct(s) => ItemSig::Struct {
                name: rodeo.resolve(&s.name).to_owned(),
                fields: s
                    .fields
                    .iter()
                    .map(|f| ParamSig {
                        name: rodeo.resolve(&f.name).to_owned(),
                        ty: TypeName::from_ast(ast, rodeo, f.ty),
                    })
                    .collect(),
            },
            Item::Enum(e) => ItemSig::Enum {
                name: rodeo.resolve(&e.name).to_owned(),
                variants: e
                    .variants
                    .iter()
                    .map(|v| {
                        let payload = match &v.payload {
                            aura_ast::VariantPayload::None => Vec::new(),
                            aura_ast::VariantPayload::Tuple(tys) => tys
                                .iter()
                                .map(|&t| TypeName::from_ast(ast, rodeo, t))
                                .collect(),
                        };
                        (rodeo.resolve(&v.name).to_owned(), payload)
                    })
                    .collect(),
            },
            Item::Use { path, .. } => ItemSig::Use {
                path: path.iter().map(|s| rodeo.resolve(s).to_owned()).collect(),
            },
            Item::ExternBlock { abi, fns, .. } => ItemSig::ExternBlock {
                abi: rodeo.resolve(abi).to_owned(),
                fns: fns
                    .iter()
                    .map(|f| ExternFnSig {
                        name: rodeo.resolve(&f.name).to_owned(),
                        params: f
                            .params
                            .iter()
                            .map(|p| ParamSig {
                                name: rodeo.resolve(&p.name).to_owned(),
                                ty: TypeName::from_ast(ast, rodeo, p.ty),
                            })
                            .collect(),
                        ret: f.ret.map(|r| TypeName::from_ast(ast, rodeo, r)),
                    })
                    .collect(),
            },
            Item::Error { .. } => ItemSig::Error,
        };
        items.push(sig);
    }
    FileItems { items }
}

fn fn_sig(ast: &aura_ast::Ast, rodeo: &lasso::Rodeo, f: &aura_ast::FnDef) -> ItemSig {
    ItemSig::Fn {
        name: rodeo.resolve(&f.name).to_owned(),
        params: f
            .params
            .iter()
            .map(|p| ParamSig {
                name: rodeo.resolve(&p.name).to_owned(),
                ty: TypeName::from_ast(ast, rodeo, p.ty),
            })
            .collect(),
        ret: f.ret.map(|r| TypeName::from_ast(ast, rodeo, r)),
        is_extern: f.is_extern,
    }
}
