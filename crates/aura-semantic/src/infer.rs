//! Union-find inference context.
//!
//! Integer and float literals create vars constrained to a *family*
//! (`VarKind::Int`/`VarKind::Float`) — `42` unifies with `i32` or `u8`
//! alike, and defaults to `i64`/`f64` when nothing pins it down.
//!
//! `Type::Error` is a poison value: it unifies with anything so one bad
//! expression doesn't cascade. `Type::Never` unifies with anything too —
//! a `return`/`break` satisfies every expected type.

use crate::ty::{FloatTy, IntTy, Type};

/// Which families an inference var may unify with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    /// Unconstrained — anything.
    Any,
    /// Integer literal var — any `Int(..)` width.
    Int,
    /// Float literal var — `f32`/`f64`.
    Float,
}

#[derive(Debug, Clone)]
struct VarEntry {
    /// Union-find parent (self when root).
    parent: u32,
    kind: VarKind,
    /// Root vars bound to a concrete type carry it here.
    binding: Option<Type>,
}

/// Result of a failed unification — the two conflicting roots.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifyError {
    pub expected: Type,
    pub found: Type,
}

#[derive(Debug, Default)]
pub struct InferCtx {
    vars: Vec<VarEntry>,
}

impl InferCtx {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_var(&mut self, kind: VarKind) -> Type {
        self.vars.push(VarEntry {
            parent: u32::try_from(self.vars.len()).unwrap_or(u32::MAX),
            kind,
            binding: None,
        });
        Type::Var(u32::try_from(self.vars.len() - 1).unwrap_or(u32::MAX))
    }

    fn root(&mut self, mut v: u32) -> u32 {
        while self.vars[v as usize].parent != v {
            let p = self.vars[v as usize].parent;
            // path halving
            self.vars[v as usize].parent = self.vars[p as usize].parent;
            let next = self.vars[v as usize].parent;
            v = next;
        }
        v
    }

    /// Chase var bindings to a concrete type (or the var's root entry).
    /// Returns `(root_var_index, Option<concrete_binding>)` for vars.
    fn resolve_shallow(&mut self, ty: &Type) -> Type {
        if let Type::Var(v) = ty {
            let r = self.root(*v);
            if let Some(b) = self.vars[r as usize].binding.clone() {
                return self.resolve_shallow(&b);
            }
            return Type::Var(r);
        }
        ty.clone()
    }

    /// Fully resolve a type — nested structures included — replacing vars
    /// by their bindings. Unbound vars become `Type::Var(root)`; call
    /// [`InferCtx::default_var`] to give them a concrete default.
    pub fn resolve(&mut self, ty: &Type) -> Type {
        match self.resolve_shallow(ty) {
            Type::Fn { params, ret } => Type::Fn {
                params: params.iter().map(|p| self.resolve(p)).collect(),
                ret: Box::new(self.resolve(&ret)),
            },
            Type::Tuple(ts) => Type::Tuple(ts.iter().map(|t| self.resolve(t)).collect()),
            Type::Pointer { mutable, pointee } => Type::Pointer {
                mutable,
                pointee: Box::new(self.resolve(&pointee)),
            },
            Type::Result(ok, err) => {
                Type::Result(Box::new(self.resolve(&ok)), Box::new(self.resolve(&err)))
            }
            t => t,
        }
    }

    /// The family of a var root — for diagnostics (`{integer}` etc.).
    pub fn var_kind(&mut self, v: u32) -> VarKind {
        let r = self.root(v);
        self.vars[r as usize].kind
    }

    /// Give an unresolved var its default: `i64` for ints, `f64` for
    /// floats, `None` for unconstrained (`Any` vars — caller decides,
    /// usually an inference error).
    pub fn default_var(&mut self, ty: &Type) -> Option<Type> {
        let Type::Var(v) = self.resolve_shallow(ty) else {
            return Some(self.resolve(ty));
        };
        Some(match self.vars[v as usize].kind {
            VarKind::Int => Type::Int(IntTy::I64),
            VarKind::Float => Type::Float(FloatTy::F64),
            VarKind::Any => return None,
        })
    }

    /// Fully resolve + default a type for output. Returns the finalized
    /// type plus the root ids of any unconstrained `Any` vars that had to
    /// collapse to `Error` (caller reports `E2111` once per root).
    pub fn finalize(&mut self, ty: &Type, unbound: &mut Vec<u32>) -> Type {
        match self.resolve(ty) {
            Type::Var(v) => {
                if let Some(d) = self.default_var(&Type::Var(v)) {
                    d
                } else {
                    unbound.push(v);
                    Type::Error
                }
            }
            Type::Fn { params, ret } => Type::Fn {
                params: params.iter().map(|p| self.finalize(p, unbound)).collect(),
                ret: Box::new(self.finalize(&ret, unbound)),
            },
            Type::Tuple(ts) => Type::Tuple(ts.iter().map(|t| self.finalize(t, unbound)).collect()),
            Type::Pointer { mutable, pointee } => Type::Pointer {
                mutable,
                pointee: Box::new(self.finalize(&pointee, unbound)),
            },
            Type::Result(ok, err) => Type::Result(
                Box::new(self.finalize(&ok, unbound)),
                Box::new(self.finalize(&err, unbound)),
            ),
            t => t,
        }
    }

    /// Unify two types. `Ok` records the binding; `Err` carries the two
    /// shallow-resolved conflict roots for diagnostics.
    ///
    /// # Errors
    /// Returns [`UnifyError`] when the types are irreconcilable (mismatched
    /// constructors, different arities, or `Int`/`Float` family conflict).
    pub fn unify(&mut self, expected: &Type, found: &Type) -> Result<(), UnifyError> {
        let a = self.resolve_shallow(expected);
        let b = self.resolve_shallow(found);
        // Poison / divergence absorb any mismatch.
        if matches!(a, Type::Error | Type::Never) || matches!(b, Type::Error | Type::Never) {
            return Ok(());
        }
        if let Type::Var(v) = a {
            return self.bind_var(v, &b);
        }
        if let Type::Var(v) = b {
            return self.bind_var(v, &a);
        }
        match (&a, &b) {
            _ if a == b => Ok(()),
            (
                Type::Fn {
                    params: ap,
                    ret: ar,
                },
                Type::Fn {
                    params: bp,
                    ret: br,
                },
            ) => {
                if ap.len() != bp.len() {
                    return Err(UnifyError {
                        expected: a,
                        found: b,
                    });
                }
                for (x, y) in ap.iter().zip(bp.iter()) {
                    self.unify(x, y)?;
                }
                self.unify(ar, br)
            }
            (Type::Tuple(at), Type::Tuple(bt)) => {
                if at.len() != bt.len() {
                    return Err(UnifyError {
                        expected: a,
                        found: b,
                    });
                }
                for (x, y) in at.iter().zip(bt.iter()) {
                    self.unify(x, y)?;
                }
                Ok(())
            }
            (
                Type::Pointer {
                    mutable: am,
                    pointee: ap,
                },
                Type::Pointer {
                    mutable: bm,
                    pointee: bp,
                },
            ) if am == bm => self.unify(ap, bp),
            (Type::Result(ao, ae), Type::Result(bo, be)) => {
                self.unify(ao, bo)?;
                self.unify(ae, be)
            }
            _ => Err(UnifyError {
                expected: a,
                found: b,
            }),
        }
    }

    /// Bind var root `v` to `other`, respecting the var's family.
    fn bind_var(&mut self, v: u32, other: &Type) -> Result<(), UnifyError> {
        // Var ∘ Var: merge roots, keep the more constrained family.
        if let Type::Var(ov) = other {
            let oroot = self.root(*ov);
            if oroot == v {
                return Ok(());
            }
            let merged = match (self.vars[v as usize].kind, self.vars[oroot as usize].kind) {
                (VarKind::Int, VarKind::Int) => VarKind::Int,
                (VarKind::Float, VarKind::Float) => VarKind::Float,
                (VarKind::Any, k) | (k, VarKind::Any) => k,
                // Int ∘ Float — irreconcilable families.
                _ => {
                    return Err(UnifyError {
                        expected: Type::Var(v),
                        found: other.clone(),
                    });
                }
            };
            self.vars[v as usize].parent = oroot;
            self.vars[oroot as usize].kind = merged;
            return Ok(());
        }
        // Family check against a concrete type.
        let ok = match self.vars[v as usize].kind {
            VarKind::Any => true,
            VarKind::Int => matches!(other, Type::Int(_)),
            VarKind::Float => matches!(other, Type::Float(_)),
        };
        if !ok {
            return Err(UnifyError {
                expected: Type::Var(v),
                found: other.clone(),
            });
        }
        self.vars[v as usize].binding = Some(other.clone());
        Ok(())
    }
}
