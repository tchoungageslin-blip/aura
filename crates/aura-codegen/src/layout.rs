//! Aggregate layout — C-compatible field offsets for `struct` types.
//!
//! Fields are laid out in declaration order at natural alignment; the
//! struct's alignment is the maximum field alignment and its size is
//! padded up to a multiple of it. This is the C rule, which matters once
//! structs cross an `extern` boundary.

use aura_salsa_db::{FileItems, ItemSig, TypeName};
use aura_semantic::{Type, lower_typename};

/// Byte layout of one struct type.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Padded total size in bytes.
    pub size: u32,
    /// Natural alignment (max of field aligns).
    pub align: u32,
    /// Byte offset of each declared field, in declaration order.
    pub offsets: Vec<u32>,
    /// Lowered field types, in declaration order.
    pub field_tys: Vec<Type>,
}

/// Size/alignment of a scalar type in bytes. Aggregates are represented
/// by address everywhere, so their "field size" is the pointer size.
/// Returns `None` for types with no codegen representation yet
/// (`str`, enums, tuples, `fn`).
pub fn scalar_size_align(ty: &Type, ptr_size: u32) -> Option<(u32, u32)> {
    Some(match ty {
        Type::Bool => (1, 1),
        Type::Int(i) => {
            let s = i.bytes();
            (s, s.clamp(1, 8))
        }
        Type::Float(f) => match f {
            aura_semantic::FloatTy::F32 => (4, 4),
            aura_semantic::FloatTy::F64 => (8, 8),
        },
        Type::Pointer { .. } | Type::Fn { .. } | Type::Str | Type::Struct(_) | Type::Enum(_) => {
            (ptr_size, ptr_size)
        }
        Type::Unit | Type::Never | Type::Error | Type::Tuple(_) | Type::Var(_) => {
            return None;
        }
    })
}

/// Compute the [`Layout`] of `FileItems.items[idx]` (must be a struct).
/// `None` for non-struct items or fields without a representation.
pub fn struct_layout(items: &FileItems, idx: u32, ptr_size: u32) -> Option<Layout> {
    let ItemSig::Struct { fields, .. } = items.items.get(idx as usize)? else {
        return None;
    };
    let mut offsets = Vec::with_capacity(fields.len());
    let mut field_tys = Vec::with_capacity(fields.len());
    let mut size: u32 = 0;
    let mut align: u32 = 1;
    for f in fields {
        let ty = field_ty(items, &f.ty);
        // Aggregate fields are stored inline — recurse for their size.
        let (fs, fa) = match &ty {
            Type::Struct(inner) => {
                let l = struct_layout(items, *inner, ptr_size)?;
                (l.size, l.align)
            }
            t => scalar_size_align(t, ptr_size)?,
        };
        size = align_to(size, fa);
        offsets.push(size);
        field_tys.push(ty);
        size += fs;
        align = align.max(fa);
    }
    Some(Layout {
        size: align_to(size, align),
        align,
        offsets,
        field_tys,
    })
}

fn field_ty(items: &FileItems, tn: &TypeName) -> Type {
    lower_typename(items, tn)
}

const fn align_to(v: u32, align: u32) -> u32 {
    (v + align - 1) & !(align - 1)
}
