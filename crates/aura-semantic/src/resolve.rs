//! File-level name resolution.
//!
//! [`resolved_file`] builds the file's definition namespace from
//! `file_items` — position-free, so the map is stable across body edits
//! and downstream queries are backdated.
//!
//! Aura uses a **single namespace**: fns, structs, enums, enum variants,
//! and extern fns all compete for one name table (`Circle(..)` in a call
//! position must mean the enum variant). Duplicates are recorded as
//! [`Resolution::duplicates`] data — `check_file` attaches the spans and
//! emits `E2003`, since this query deliberately carries no positions.

use aura_salsa_db::{Db, FileItems, ItemSig, SourceFile, file_items};
use indexmap::IndexMap;

/// What a bare name resolves to in value position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Def {
    /// `fn name(..)` — payload is the item index into `FileItems`.
    Fn(u32),
    /// `struct Name { .. }` — usable as value only via struct literal.
    Struct(u32),
    /// `enum Name { .. }` — not itself a value; variants are.
    Enum(u32),
    /// `Variant` of enum at item index `.0`, variant index `.1`.
    Variant(u32, u32),
    /// `fn` inside an `extern` block — `(block item idx, fn idx)`.
    ExternFn(u32, u32),
}

/// A redefinition: `name` defined at both `first` and `dup` item indices.
#[derive(Debug, Clone, PartialEq)]
pub struct Duplicate {
    pub name: String,
    /// Index of the winning (first) definition in `FileItems`.
    pub first: u32,
    /// Index of the rejected later definition.
    pub dup: u32,
}

/// The file's definition table — position-free on purpose.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Resolution {
    /// name → what it refers to. First definition wins; later duplicates
    /// are reported in `duplicates`.
    pub defs: IndexMap<String, Def>,
    pub duplicates: Vec<Duplicate>,
}

impl Resolution {
    pub fn lookup(&self, name: &str) -> Option<Def> {
        self.defs.get(name).copied()
    }
}

/// Resolve the file's top-level names. Depends only on `file_items`.
#[salsa::tracked(returns(ref))]
pub fn resolved_file(db: &dyn Db, file: SourceFile) -> Resolution {
    resolve_items(file_items(db, file))
}

fn resolve_items(items: &FileItems) -> Resolution {
    let mut res = Resolution::default();
    for (i, sig) in items.iter() {
        match sig {
            ItemSig::Fn { name, .. } => insert(&mut res, name, i, Def::Fn(i)),
            ItemSig::Struct { name, .. } => insert(&mut res, name, i, Def::Struct(i)),
            ItemSig::Enum { name, variants, .. } => {
                insert(&mut res, name, i, Def::Enum(i));
                for (v, (vname, _)) in variants.iter().enumerate() {
                    insert(
                        &mut res,
                        vname,
                        i,
                        Def::Variant(i, u32::try_from(v).unwrap_or(u32::MAX)),
                    );
                }
            }
            ItemSig::ExternBlock { fns, .. } => {
                for (f, fsig) in fns.iter().enumerate() {
                    insert(
                        &mut res,
                        &fsig.name,
                        i,
                        Def::ExternFn(i, u32::try_from(f).unwrap_or(u32::MAX)),
                    );
                }
            }
            ItemSig::Use { .. } | ItemSig::Error => {}
        }
    }
    res
}

fn insert(res: &mut Resolution, name: &str, item: u32, def: Def) {
    if res.defs.contains_key(name) {
        let first = match res.defs.get(name) {
            Some(
                Def::Fn(i)
                | Def::Struct(i)
                | Def::Enum(i)
                | Def::Variant(i, _)
                | Def::ExternFn(i, _),
            ) => *i,
            None => item,
        };
        res.duplicates.push(Duplicate {
            name: name.to_owned(),
            first,
            dup: item,
        });
        return; // first wins
    }
    res.defs.insert(name.to_owned(), def);
}
