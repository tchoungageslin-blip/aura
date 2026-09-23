//! aura-codegen — object-file emission via Cranelift.
//!
//! [`compile_file`] drives the backend pipeline for one source file:
//! collect each function's MIR (diagnostics included), validate the
//! entry point, then emit a single COFF/ELF object through
//! `cranelift-object`.
//!
//! The output is a relocatable object — final linking is `aura-linker`'s
//! job (`aura build`/`aura run`).

mod emit;
mod layout;

pub use emit::Emitted;
pub use layout::{Layout, enum_layout, layout_of, scalar_size_align, struct_layout};

use aura_common::{Diagnostic, FileId, Span, codes};
use aura_mir::{MirBody, mir_fn, mir_project_fn};
use aura_salsa_db::{Db, FileItems, ItemSig, Project, SourceFile, file_items, project_items};
use aura_semantic::{IntTy, Type, lower_typename};

/// Result of compiling one file.
pub struct CompileOutput {
    /// Object bytes — `None` if diagnostics contain errors.
    pub object: Option<Vec<u8>>,
    /// MIR-level and codegen diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Compile `file` to an object file. Expects `check_file` to have passed;
/// unsupported constructs surface as `E3004` diagnostics here (and abort
/// the object).
pub fn compile_file(db: &dyn Db, file: SourceFile) -> CompileOutput {
    let items = file_items(db, file);
    let file_id = file.file_id(db);
    let mut diagnostics = Vec::new();
    let mut mirs = Vec::new();

    for (i, sig) in items.iter() {
        if !matches!(sig, ItemSig::Fn { .. }) {
            continue;
        }
        let Some(mir) = mir_fn(db, file, i).as_ref() else {
            continue;
        };
        diagnostics.extend(mir.diagnostics.iter().cloned());
        mirs.push((i, mir.clone()));
    }
    finish_compile(items, &mirs, diagnostics, file_id)
}

/// Compile a multi-file [`Project`]: same pipeline as [`compile_file`]
/// over the merged `project_items` table — callables resolve across files
/// because `Callee`/`Def` payloads index that table globally.
pub fn compile_project(db: &dyn Db, project: Project) -> CompileOutput {
    let pi = project_items(db, project);
    let file_id = project.files(db)[0].file_id(db);
    let mut diagnostics = Vec::new();
    let mut mirs = Vec::new();

    for (g, sig) in pi.merged.iter() {
        if !matches!(sig, ItemSig::Fn { .. }) {
            continue;
        }
        let Some(mir) = mir_project_fn(db, project, g).as_ref() else {
            continue;
        };
        diagnostics.extend(mir.diagnostics.iter().cloned());
        mirs.push((g, mir.clone()));
    }
    finish_compile(&pi.merged, &mirs, diagnostics, file_id)
}

/// Validate collected MIR bodies and emit the object — the shared tail
/// of the single-file and project compile paths.
fn finish_compile(
    items: &FileItems,
    mirs: &[(u32, MirBody)],
    mut diagnostics: Vec<Diagnostic>,
    file_id: FileId,
) -> CompileOutput {
    // Reject types with no codegen representation yet (128-bit ints,
    // str/enum/tuple — most are already diagnosed in MIR).
    for (_, mir) in mirs {
        for l in &mir.locals {
            if unsupported_ty(&l.ty, items) {
                diagnostics.push(Diagnostic::error(
                    codes::CG_UNSUPPORTED,
                    format!(
                        "codegen: type `{}` is not yet supported",
                        l.ty.display(items)
                    ),
                    Span::point(file_id, 0),
                ));
            }
        }
    }

    // Entry-point validation.
    match items.find("main") {
        None => diagnostics.push(Diagnostic::error(
            codes::CG_MAIN_TYPE,
            "no `main` function".to_owned(),
            Span::point(file_id, 0),
        )),
        Some(idx) => {
            if let Some(ItemSig::Fn { params, ret, .. }) = items.items.get(idx as usize) {
                let ok_ret = ret
                    .as_ref()
                    .is_none_or(|t| matches!(lower_typename(items, t), Type::Int(_) | Type::Unit));
                if !params.is_empty() || !ok_ret {
                    diagnostics.push(Diagnostic::error(
                        codes::CG_MAIN_TYPE,
                        "`main` must take no parameters and return an integer".to_owned(),
                        Span::point(file_id, 0),
                    ));
                }
            }
        }
    }

    if diagnostics.iter().any(aura_common::Diagnostic::is_error) {
        return CompileOutput {
            object: None,
            diagnostics,
        };
    }

    match emit::emit_object(items, mirs) {
        Ok(e) => CompileOutput {
            object: Some(e.object),
            diagnostics,
        },
        Err(ds) => {
            diagnostics.extend(ds);
            CompileOutput {
                object: None,
                diagnostics,
            }
        }
    }
}

fn unsupported_ty(t: &Type, items: &FileItems) -> bool {
    match t {
        Type::Int(IntTy::I128 | IntTy::U128) | Type::Str | Type::Tuple(_) | Type::Fn { .. } => true,
        Type::Pointer { pointee, .. } => unsupported_ty(pointee, items),
        // An aggregate is supported iff a C-compatible layout exists for
        // it (all fields representable; enums additionally need a tag).
        Type::Struct(i) | Type::Enum(i) => layout_of(items, *i, 8).is_none(),
        Type::Result(ok, err) => crate::layout::result_layout(items, ok, err, 8).is_none(),
        Type::Int(_)
        | Type::Float(_)
        | Type::Bool
        | Type::Unit
        | Type::Never
        | Type::Error
        | Type::Var(_) => false,
    }
}
