//! Aggregate layout — C-compatible field offsets for `struct` types.
//!
//! Fields are laid out in declaration order at natural alignment; the
//! struct's alignment is the maximum field alignment and its size is
//! padded up to a multiple of it. This is the C rule, which matters once
//! structs cross an `extern` boundary.

use aura_salsa_db::{FileItems, ItemSig, TypeName};
use aura_semantic::{Type, lower_typename};

/// Byte layout of one aggregate type (struct or enum).
///
/// Structs use `offsets`/`field_tys` directly. Enums are
/// `{ tag: i32 @0, payload: union @payload_off }` — `variants[v]` holds
/// the payload struct layout of variant `v` (`size == 0` payload for
/// unit variants).
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
    /// Enum payload region offset (tag is `i32` at 0). 0 for structs.
    pub payload_off: u32,
    /// Per-variant payload layouts (empty for structs).
    pub variants: Vec<Layout>,
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
        Type::Pointer { .. }
        | Type::Fn { .. }
        | Type::Str
        | Type::Struct(_)
        | Type::Enum(_)
        | Type::Result(..) => (ptr_size, ptr_size),
        Type::Unit | Type::Never | Type::Error | Type::Tuple(_) | Type::Var(_) => {
            return None;
        }
    })
}

/// Compute the [`Layout`] of `FileItems.items[idx]` — struct or enum.
/// `None` for other items or unrepresentable fields.
pub fn layout_of(items: &FileItems, idx: u32, ptr_size: u32) -> Option<Layout> {
    match items.items.get(idx as usize)? {
        ItemSig::Struct { .. } => struct_layout(items, idx, ptr_size),
        ItemSig::Enum { .. } => enum_layout(items, idx, ptr_size),
        _ => None,
    }
}

/// Compute the [`Layout`] of `FileItems.items[idx]` (must be a struct).
/// `None` for non-struct items or fields without a representation.
pub fn struct_layout(items: &FileItems, idx: u32, ptr_size: u32) -> Option<Layout> {
    let ItemSig::Struct { fields, .. } = items.items.get(idx as usize)? else {
        return None;
    };
    let field_tys: Vec<Type> = fields.iter().map(|f| field_ty(items, &f.ty)).collect();
    let (offsets, size, align) = layout_fields(items, &field_tys, ptr_size)?;
    Some(Layout {
        size,
        align,
        offsets,
        field_tys,
        payload_off: 0,
        variants: Vec::new(),
    })
}

/// Enum layout: `i32` tag at offset 0, then a union of the per-variant
/// payload structs at `payload_off` (aligned to the payload's alignment).
/// `None` for non-enum items or unrepresentable payloads.
pub fn enum_layout(items: &FileItems, idx: u32, ptr_size: u32) -> Option<Layout> {
    const TAG: u32 = 4; // i32 tag
    let ItemSig::Enum { variants, .. } = items.items.get(idx as usize)? else {
        return None;
    };
    let mut layouts = Vec::with_capacity(variants.len());
    let mut max_size = 0u32;
    let mut max_align = 1u32;
    for (_, payload) in variants {
        let tys: Vec<Type> = payload.iter().map(|t| field_ty(items, t)).collect();
        let (offsets, size, align) = layout_fields(items, &tys, ptr_size)?;
        max_size = max_size.max(size);
        max_align = max_align.max(align);
        layouts.push(Layout {
            size,
            align,
            offsets,
            field_tys: tys,
            payload_off: 0,
            variants: Vec::new(),
        });
    }
    let payload_off = align_to(TAG, max_align);
    Some(Layout {
        size: align_to(payload_off + max_size, max_align.max(TAG)),
        align: max_align.max(TAG),
        offsets: Vec::new(),
        field_tys: Vec::new(),
        payload_off,
        variants: layouts,
    })
}

/// Layout of the built-in `Result<ok, err>` — the same tagged repr as a
/// two-variant enum: `Ok` (variant 0) wraps `ok`, `Err` (variant 1)
/// wraps `err`. `None` if either payload is unrepresentable.
pub fn result_layout(items: &FileItems, ok: &Type, err: &Type, ptr_size: u32) -> Option<Layout> {
    const TAG: u32 = 4; // i32 tag
    let mut layouts = Vec::with_capacity(2);
    let mut max_size = 0u32;
    let mut max_align = 1u32;
    for payload in [ok, err] {
        let tys = vec![payload.clone()];
        let (offsets, size, align) = layout_fields(items, &tys, ptr_size)?;
        max_size = max_size.max(size);
        max_align = max_align.max(align);
        layouts.push(Layout {
            size,
            align,
            offsets,
            field_tys: tys,
            payload_off: 0,
            variants: Vec::new(),
        });
    }
    let payload_off = align_to(TAG, max_align);
    Some(Layout {
        size: align_to(payload_off + max_size, max_align.max(TAG)),
        align: max_align.max(TAG),
        offsets: Vec::new(),
        field_tys: Vec::new(),
        payload_off,
        variants: layouts,
    })
}

/// Shared C-layout math: field offsets at natural alignment, total size
/// padded to `align`. `None` if any field type is unrepresentable.
fn layout_fields(
    items: &FileItems,
    field_tys: &[Type],
    ptr_size: u32,
) -> Option<(Vec<u32>, u32, u32)> {
    let mut offsets = Vec::with_capacity(field_tys.len());
    let mut size: u32 = 0;
    let mut align: u32 = 1;
    for ty in field_tys {
        // Aggregate fields are stored inline — recurse for their size.
        let (fs, fa) = match ty {
            Type::Struct(inner) | Type::Enum(inner) => {
                let l = layout_of(items, *inner, ptr_size)?;
                (l.size, l.align)
            }
            Type::Result(ok, err) => {
                let l = result_layout(items, ok, err, ptr_size)?;
                (l.size, l.align)
            }
            t => scalar_size_align(t, ptr_size)?,
        };
        size = align_to(size, fa);
        offsets.push(size);
        size += fs;
        align = align.max(fa);
    }
    Some((offsets, align_to(size, align), align))
}

fn field_ty(items: &FileItems, tn: &TypeName) -> Type {
    lower_typename(items, tn)
}

const fn align_to(v: u32, align: u32) -> u32 {
    (v + align - 1) & !(align - 1)
}
