//! MIR → Cranelift IR → object file emission.
//!
//! ABI decisions:
//! - Scalars map to their natural clif types (`i64`→`I64`, `bool`→`I8`,
//!   `f64`→`F64`); `()`/`!` map to a dummy `I8` that is never observed.
//! - Aggregates (structs) are *addressed*: a MIR local of struct type is
//!   a variable holding a pointer to a stack slot. Parameters and returns
//!   of aggregate type lower to a hidden pointer argument (sret-style) —
//!   an internal convention; `extern` fns keep their declared scalar-only
//!   ABI since the host ABI for by-value aggregates is platform-specific.
//! - Calling convention is the target default (`WindowsFastcall` on
//!   x64-windows, `SystemV` elsewhere) for both internal and extern fns.

use aura_ast::{BinOp, UnOp};
use aura_common::{BuiltinFn, Diagnostic, Span, codes};
use aura_mir::{Callee, Const, MirBody, MirStmt, MirTerm, Operand, Place, Rvalue};
use aura_salsa_db::{FileItems, ItemSig};
use aura_semantic::{FloatTy, Type};
use cranelift_codegen::ir::{
    AbiParam, GlobalValue, InstBuilder, MemFlagsData, Signature, StackSlotData, StackSlotKind,
    TrapCode, Value,
    condcodes::{FloatCC, IntCC},
    types,
};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_codegen::settings;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};
use rustc_hash::FxHashMap;

use crate::layout::{Layout, layout_of};

/// Everything codegen learned about the file, kept for diagnostics.
pub struct Emitted {
    /// COFF/ELF object bytes.
    pub object: Vec<u8>,
}

/// All signatures/ids the per-function emitters need to agree on.
struct FnTable {
    /// `FuncId` per `FileItems` index for Aura fns.
    fns: FxHashMap<u32, FuncId>,
    /// `FuncId` per `(extern-block idx, fn idx)` for extern fns.
    externs: FxHashMap<(u32, u32), FuncId>,
    /// Runtime `aura_str_eq`/`aura_str_concat` helpers — imported
    /// unconditionally; unreferenced imports are dropped.
    str_eq: FuncId,
    str_concat: FuncId,
    /// Runtime helpers for prelude builtins (`aura_rt_print`, …) —
    /// imported unconditionally; unreferenced imports are dropped.
    builtins: FxHashMap<BuiltinFn, FuncId>,
}

/// Compile every function body into one object module and emit bytes.
///
/// Caller guarantees `check_file` and every `mir_fn` produced no errors.
/// `mirs` pairs each non-extern `ItemSig::Fn` item index with its MIR.
pub fn emit_object(items: &FileItems, mirs: &[(u32, MirBody)]) -> Result<Emitted, Vec<Diagnostic>> {
    let flags = settings::Flags::new(settings::builder());
    let isa = cranelift_native::builder()
        .map_err(internal)?
        .finish(flags)
        .map_err(|e| internal(&format!("isa: {e}")))?;
    let ptr = isa.pointer_type();
    let call_conv = isa.default_call_conv();
    let frontend_cfg = isa.frontend_config();

    let builder = ObjectBuilder::new(
        isa,
        "aura",
        Box::new(default_libcall_names())
            as Box<dyn Fn(cranelift_codegen::ir::LibCall) -> String + Send + Sync>,
    )
    .map_err(|e| internal(&e.to_string()))?;
    let mut module = ObjectModule::new(builder);

    // Pass 1 — declare every callable symbol before bodies reference them.
    let (str_eq, str_concat) = declare_str_rt(&mut module, ptr, call_conv)?;
    let builtins = declare_builtins(&mut module, ptr, call_conv)?;
    let mut table = FnTable {
        fns: FxHashMap::default(),
        externs: FxHashMap::default(),
        str_eq,
        str_concat,
        builtins,
    };
    declare_items(items, &mut module, ptr, call_conv, &mut table)?;

    // Struct layouts for every struct item (None → not a struct).
    let layouts: Vec<Option<Layout>> = items
        .items
        .iter()
        .enumerate()
        .map(|(i, _)| layout_of(items, u32::try_from(i).unwrap_or(u32::MAX), ptr.bytes()))
        .collect();

    // Pass 2 — emit each fn body.
    let mut fbc = FunctionBuilderContext::new();
    for (idx, mir) in mirs {
        let Some(&func_id) = table.fns.get(idx) else {
            continue;
        };
        let Some(sig) = fn_sig(items, *idx, ptr, call_conv) else {
            continue;
        };
        let mut ctx = module.make_context();
        ctx.func.signature = sig;
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            let mut fg = FnGen {
                b: &mut b,
                mir,
                items,
                layouts: &layouts,
                table: &table,
                module: &mut module,
                ptr,
                frontend_cfg,
                vars: Vec::new(),
                blocks: Vec::new(),
                sret: None,
            };
            fg.emit();
            b.seal_all_blocks();
            b.finalize(frontend_cfg);
        }
        module
            .define_function(func_id, &mut ctx)
            .map_err(|e| internal(&format!("define {}: {e}", mir.name)))?;
    }

    let product = module.finish();
    let bytes = product
        .emit()
        .map_err(|e| internal(&format!("object emit: {e}")))?;
    Ok(Emitted { object: bytes })
}

fn internal(msg: &str) -> Vec<Diagnostic> {
    vec![Diagnostic::error(
        codes::CG_INTERNAL,
        msg.to_owned(),
        Span::point(aura_common::FileId(0), 0),
    )]
}

/// Pass-1 declaration of every Aura fn (`main` exported, others local)
/// and extern fn (imported) into the object module's `table`.
fn declare_items(
    items: &FileItems,
    module: &mut ObjectModule,
    ptr: cranelift_codegen::ir::Type,
    cc: cranelift_codegen::isa::CallConv,
    table: &mut FnTable,
) -> Result<(), Vec<Diagnostic>> {
    for (i, sig) in items.iter() {
        match sig {
            ItemSig::Fn { name, .. } => {
                let Some(clif_sig) = fn_sig(items, i, ptr, cc) else {
                    continue; // body-less or broken signature — typeck reported
                };
                let linkage = if name == "main" {
                    Linkage::Export
                } else {
                    Linkage::Local
                };
                let id = module
                    .declare_function(name, linkage, &clif_sig)
                    .map_err(|e| internal(&format!("declare {name}: {e}")))?;
                table.fns.insert(i, id);
            }
            ItemSig::ExternBlock { fns, .. } => {
                for (f, sig) in fns.iter().enumerate() {
                    let Some(clif_sig) = extern_sig(items, sig, ptr, cc) else {
                        continue;
                    };
                    let id = module
                        .declare_function(&sig.name, Linkage::Import, &clif_sig)
                        .map_err(|e| internal(&format!("declare {}: {e}", sig.name)))?;
                    table
                        .externs
                        .insert((i, u32::try_from(f).unwrap_or(u32::MAX)), id);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Import the runtime string helpers: `aura_str_eq(l_ptr, l_len,
/// r_ptr, r_len) -> i32` and `aura_str_concat(l_ptr, l_len, r_ptr,
/// r_len) -> ptr`. Declared unconditionally — unreferenced imports are
/// dropped from the object.
fn declare_str_rt(
    module: &mut ObjectModule,
    ptr: cranelift_codegen::ir::Type,
    cc: cranelift_codegen::isa::CallConv,
) -> Result<(FuncId, FuncId), Vec<Diagnostic>> {
    let mut sig = Signature::new(cc);
    sig.params.extend([AbiParam::new(ptr); 4]);
    sig.returns.push(AbiParam::new(types::I32));
    let eq = module
        .declare_function("aura_str_eq", Linkage::Import, &sig)
        .map_err(|e| internal(&format!("declare aura_str_eq: {e}")))?;
    sig.returns.pop();
    sig.returns.push(AbiParam::new(ptr));
    let concat = module
        .declare_function("aura_str_concat", Linkage::Import, &sig)
        .map_err(|e| internal(&format!("declare aura_str_concat: {e}")))?;
    Ok((eq, concat))
}

/// Import every `aura_rt_*` builtin helper — `print`-family takes
/// `{ptr, len}`, `exit` takes `i64`; all return void. Declared
/// unconditionally; unreferenced imports are dropped.
fn declare_builtins(
    module: &mut ObjectModule,
    ptr: cranelift_codegen::ir::Type,
    cc: cranelift_codegen::isa::CallConv,
) -> Result<FxHashMap<BuiltinFn, FuncId>, Vec<Diagnostic>> {
    let mut out = FxHashMap::default();
    for &b in BuiltinFn::ALL {
        // Builtins with no runtime import (`vec_new` zeroes storage
        // inline; `vec_set` is `vec_get` + an element copy).
        if b.runtime_symbol().is_empty() {
            continue;
        }
        let mut sig = Signature::new(cc);
        match b {
            BuiltinFn::Print | BuiltinFn::Println | BuiltinFn::Eprint | BuiltinFn::Eprintln => {
                sig.params.extend([AbiParam::new(ptr); 2]); // str → {ptr, len}
            }
            BuiltinFn::Exit => sig.params.push(AbiParam::new(types::I64)),
            // (data, len, cap, elem_ptr, elem_size, cap_out) -> data'
            BuiltinFn::VecPush => {
                sig.params.extend([AbiParam::new(ptr); 6]);
                sig.returns.push(AbiParam::new(ptr));
            }
            // (data, len, idx, elem_size) -> elem ptr (bounds-checked)
            BuiltinFn::VecGet => {
                sig.params.extend([AbiParam::new(ptr); 4]);
                sig.returns.push(AbiParam::new(ptr));
            }
            // (out: *mut {ptr,len,cap}) — runtime fills the vec triple
            BuiltinFn::Args => sig.params.push(AbiParam::new(ptr)),
            // (name_ptr, name_len, out: *mut {ptr,len})
            BuiltinFn::Env => sig.params.extend([AbiParam::new(ptr); 3]),
            BuiltinFn::VecNew | BuiltinFn::VecSet => unreachable!(),
        }
        let id = module
            .declare_function(b.runtime_symbol(), Linkage::Import, &sig)
            .map_err(|e| internal(&format!("declare {}: {e}", b.runtime_symbol())))?;
        out.insert(b, id);
    }
    Ok(out)
}

// ----- signatures --------------------------------------------------------------

/// Is `ty` passed/returned by hidden pointer (internal aggregate ABI)?
fn is_aggregate(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Struct(_) | Type::Enum(_) | Type::Result(..) | Type::Str | Type::Vec(_)
    )
}

/// clif representation of `ty`: scalars get their natural type,
/// aggregates/pointers get the target pointer type, `()`/`!` a dummy I8.
fn clif_ty(ty: &Type, ptr: cranelift_codegen::ir::Type) -> cranelift_codegen::ir::Type {
    match ty {
        Type::Bool | Type::Unit | Type::Never | Type::Error | Type::Var(_) => types::I8,
        Type::Int(i) => match i.bytes() {
            1 => types::I8,
            2 => types::I16,
            4 => types::I32,
            // 8 + anything wider — i128 deferred, E3004 guards the path.
            _ => types::I64,
        },
        Type::Float(FloatTy::F32) => types::F32,
        Type::Float(FloatTy::F64) => types::F64,
        Type::Str
        | Type::Struct(_)
        | Type::Enum(_)
        | Type::Result(..)
        | Type::Vec(_)
        | Type::Pointer { .. }
        | Type::Fn { .. }
        | Type::Tuple(_) => ptr,
    }
}

/// Signature of Aura `fn` item `idx`: aggregate params become pointers;
/// an aggregate return becomes a leading hidden out-pointer.
fn fn_sig(
    items: &FileItems,
    idx: u32,
    ptr: cranelift_codegen::ir::Type,
    cc: cranelift_codegen::isa::CallConv,
) -> Option<cranelift_codegen::ir::Signature> {
    let ItemSig::Fn { params, ret, .. } = items.items.get(idx as usize)? else {
        return None;
    };
    let mut sig = cranelift_codegen::ir::Signature::new(cc);
    let ret_ty = ret.as_ref().map_or(Type::Unit, |t| lower_ty(items, t));
    if is_aggregate(&ret_ty) {
        sig.params.push(AbiParam::new(ptr));
    }
    for p in params {
        let t = lower_ty(items, &p.ty);
        sig.params.push(if is_aggregate(&t) {
            AbiParam::new(ptr)
        } else {
            AbiParam::new(clif_ty(&t, ptr))
        });
    }
    if !is_aggregate(&ret_ty) && !matches!(ret_ty, Type::Unit | Type::Never) {
        sig.returns.push(AbiParam::new(clif_ty(&ret_ty, ptr)));
    }
    Some(sig)
}

/// Signature of an extern fn — identical lowering; aggregates are
/// diagnosed upstream (the host C ABI for by-value aggregates is not
/// implemented yet).
fn extern_sig(
    items: &FileItems,
    sig: &aura_salsa_db::ExternFnSig,
    ptr: cranelift_codegen::ir::Type,
    cc: cranelift_codegen::isa::CallConv,
) -> Option<cranelift_codegen::ir::Signature> {
    let mut out = cranelift_codegen::ir::Signature::new(cc);
    let ret_ty = sig.ret.as_ref().map_or(Type::Unit, |t| lower_ty(items, t));
    if is_aggregate(&ret_ty) {
        return None;
    }
    for p in &sig.params {
        let t = lower_ty(items, &p.ty);
        if is_aggregate(&t) {
            return None;
        }
        out.params.push(AbiParam::new(clif_ty(&t, ptr)));
    }
    if !matches!(ret_ty, Type::Unit | Type::Never) {
        out.returns.push(AbiParam::new(clif_ty(&ret_ty, ptr)));
    }
    Some(out)
}

fn lower_ty(items: &FileItems, t: &aura_salsa_db::TypeName) -> Type {
    aura_semantic::lower_typename(items, t)
}

// ----- per-function emission -------------------------------------------------------

struct FnGen<'a, 'f> {
    b: &'a mut FunctionBuilder<'f>,
    mir: &'a MirBody,
    items: &'a FileItems,
    layouts: &'a [Option<Layout>],
    table: &'a FnTable,
    module: &'a mut ObjectModule,
    ptr: cranelift_codegen::ir::Type,
    frontend_cfg: TargetFrontendConfig,
    /// One frontend variable per MIR local. Scalars hold the value;
    /// aggregate locals hold the *address* of their stack storage.
    vars: Vec<Variable>,
    /// clif block per MIR block.
    blocks: Vec<cranelift_codegen::ir::Block>,
    /// Hidden destination pointer when the return type is an aggregate.
    sret: Option<Value>,
}

impl FnGen<'_, '_> {
    fn emit(&mut self) {
        for _ in &self.mir.blocks {
            let bb = self.b.create_block();
            self.blocks.push(bb);
        }
        let entry = self.blocks[0];
        self.b.append_block_params_for_function_params(entry);
        self.b.switch_to_block(entry);

        // Declare every local's variable up front.
        for l in &self.mir.locals {
            let ty = if is_aggregate(&l.ty) {
                self.ptr
            } else {
                clif_ty(&l.ty, self.ptr)
            };
            self.vars.push(self.b.declare_var(ty));
        }

        // Bind function parameters to `_1..` (and the hidden sret param to `_0`).
        let bp: Vec<Value> = self.b.block_params(entry).to_vec();
        let mut pi = 0usize;
        if is_aggregate(&self.mir.ret) {
            self.sret = Some(bp[0]);
            self.b.def_var(self.vars[0], bp[0]);
            pi = 1;
        }
        for i in 0..self.mir.param_count as usize {
            if let Some(&v) = bp.get(pi + i) {
                let var = self.vars[1 + i];
                self.b.def_var(var, v);
            }
        }

        // Default-init the rest: scalars → zero; aggregates → fresh stack
        // slot whose address lands in the local's variable.
        let n_params = 1 + self.mir.param_count as usize;
        for (i, l) in self.mir.locals.iter().enumerate().skip(n_params.max(1)) {
            self.init_local(i, &l.ty.clone());
        }
        // `_0` when it is a scalar (return place) also gets a safe default.
        if !is_aggregate(&self.mir.ret) {
            self.init_local(0, &self.mir.ret.clone());
        }

        // Emit MIR blocks in order — entry is already the current block.
        for (i, blk) in self.mir.blocks.iter().enumerate() {
            if i > 0 {
                self.b.switch_to_block(self.blocks[i]);
            }
            for s in &blk.stmts {
                self.stmt(s);
            }
            self.term(&blk.term);
        }
    }

    /// `def_var` a zero/default for local `i` (idempotent for entry).
    fn init_local(&mut self, i: usize, ty: &Type) {
        if is_aggregate(ty) {
            let size_align = self.agg_layout(ty);
            if let Some((size, align)) = size_align {
                let slot = self.b.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    size,
                    u8::try_from(align.trailing_zeros()).unwrap_or(u8::MAX),
                ));
                let addr = self.b.ins().stack_addr(self.ptr, slot, 0);
                self.b.def_var(self.vars[i], addr);
            } else {
                let z = self.b.ins().iconst(self.ptr, 0);
                self.b.def_var(self.vars[i], z);
            }
        } else {
            let z = self.zero_of(ty);
            self.b.def_var(self.vars[i], z);
        }
    }

    fn zero_of(&mut self, ty: &Type) -> Value {
        match ty {
            Type::Float(FloatTy::F32) => self.b.ins().f32const(0.0f32),
            Type::Float(FloatTy::F64) => self.b.ins().f64const(0.0f64),
            _ => self.b.ins().iconst(clif_ty(ty, self.ptr), 0),
        }
    }

    /// [`Layout`] of any aggregate type — declared items via the
    /// precomputed table, built-in `Result`/`str` computed on the fly.
    fn layout_for(&self, ty: &Type) -> Option<Layout> {
        match ty {
            Type::Struct(idx) | Type::Enum(idx) => {
                self.layouts.get(*idx as usize).cloned().flatten()
            }
            Type::Result(ok, err) => {
                crate::layout::result_layout(self.items, ok, err, self.ptr.bytes())
            }
            Type::Str => Some(crate::layout::str_layout(self.ptr.bytes())),
            Type::Vec(_) => Some(crate::layout::vec_layout(self.ptr.bytes())),
            _ => None,
        }
    }

    /// `(size, align)` of an aggregate type's storage.
    fn agg_layout(&self, ty: &Type) -> Option<(u32, u32)> {
        self.layout_for(ty).map(|l| (l.size, l.align))
    }

    // ----- statements ------------------------------------------------------------

    fn stmt(&mut self, s: &MirStmt) {
        if let MirStmt::Assign(place, rv) = s {
            self.assign(place, rv);
        }
    }

    fn assign(&mut self, place: &Place, rv: &Rvalue) {
        match rv {
            Rvalue::Use(op) => {
                let dest_ty = self.place_ty(place);
                let src_ty = self.operand_ty(op);
                if is_aggregate(&dest_ty) || is_aggregate(&src_ty) {
                    self.copy_aggregate(place, op);
                } else {
                    let v = self.operand_val(op);
                    self.store(place, v);
                }
            }
            Rvalue::Unary(op, o) => {
                let v = self.operand_val(o);
                let ty = self.operand_ty(o);
                let r = match op {
                    UnOp::Neg => match ty {
                        Type::Float(_) => self.b.ins().fneg(v),
                        _ => self.b.ins().ineg(v),
                    },
                    UnOp::Not => {
                        if let Type::Int(_) = ty {
                            self.b.ins().bnot(v)
                        } else {
                            // `!b` for bools-as-i8: `b == 0`.
                            let z = self.b.ins().iconst(types::I8, 0);
                            self.b.ins().icmp(IntCC::Equal, v, z)
                        }
                    }
                };
                self.store(place, r);
            }
            Rvalue::Binary(op, l, r) => {
                let ty = self.operand_ty(l);
                if matches!(ty, Type::Str) && matches!(op, BinOp::Add) {
                    // `str + str` — heap-concatenated fresh string into
                    // the destination's `{ptr, len}` storage.
                    self.str_concat(place, l, r);
                } else {
                    let lv = self.operand_val(l);
                    let rv2 = self.operand_val(r);
                    let v = self.binop(*op, &ty, lv, rv2);
                    self.store(place, v);
                }
            }
            Rvalue::Call(callee, args) => self.call(place, *callee, args),
            Rvalue::StructLit { fields, .. } => {
                for (fidx, op) in fields {
                    let mut p = place.clone();
                    p.proj.push(aura_mir::Proj::Field(*fidx));
                    let fty = self.place_ty(&p);
                    if is_aggregate(&fty) {
                        self.copy_aggregate(&p, op);
                    } else {
                        let v = self.operand_val(op);
                        self.store(&p, v);
                    }
                }
            }
            Rvalue::EnumLit {
                variant, fields, ..
            } => {
                // Tag at offset 0, payload fields at `payload_off + inner`.
                let addr = self.place_addr(place);
                let tag = self.b.ins().iconst(types::I32, i64::from(*variant));
                self.b.ins().store(MemFlagsData::trusted(), tag, addr, 0);
                for (fidx, op) in fields {
                    let mut p = place.clone();
                    p.proj.push(aura_mir::Proj::VariantField {
                        variant: *variant,
                        field: *fidx,
                    });
                    let fty = self.place_ty(&p);
                    if is_aggregate(&fty) {
                        self.copy_aggregate(&p, op);
                    } else {
                        let v = self.operand_val(op);
                        self.store(&p, v);
                    }
                }
            }
            Rvalue::StrLit(text) => {
                // Bytes go into anonymous read-only data; the `str` place
                // gets `{ ptr: bytes, len }` stored through its fields.
                let data = self.str_data(text);
                let addr = self.b.ins().symbol_value(self.ptr, data);
                let mut p = place.clone();
                p.proj.push(aura_mir::Proj::Field(0));
                self.store(&p, addr);
                let mut p = place.clone();
                p.proj.push(aura_mir::Proj::Field(1));
                let len = self
                    .b
                    .ins()
                    .iconst(self.ptr, i64::try_from(text.len()).unwrap_or(i64::MAX));
                self.store(&p, len);
            }
            Rvalue::Discriminant(p) => {
                let addr = self.place_addr(p);
                let v = self
                    .b
                    .ins()
                    .load(types::I32, MemFlagsData::trusted(), addr, 0);
                self.store(place, v);
            }
        }
    }

    /// Emit `text` as anonymous read-only data and return its
    /// function-local [`GlobalValue`].
    fn str_data(&mut self, text: &str) -> GlobalValue {
        let id = self
            .module
            .declare_anonymous_data(false, false)
            .expect("anonymous data declaration cannot fail");
        let mut desc = DataDescription::new();
        desc.define(text.as_bytes().to_vec().into_boxed_slice());
        self.module
            .define_data(id, &desc)
            .expect("anonymous data definition cannot fail");
        self.module.declare_data_in_func(id, self.b.func)
    }

    /// `str ==`/`!=` via `aura_str_eq(l.ptr, l.len, r.ptr, r.len)`.
    /// `l`/`r` are the operands' aggregate base addresses.
    fn str_eq(&mut self, op: BinOp, l: Value, r: Value) -> Value {
        let len_off = i32::try_from(self.ptr.bytes()).unwrap_or(i32::MAX);
        let lp = self.b.ins().load(self.ptr, MemFlagsData::trusted(), l, 0);
        let ll = self
            .b
            .ins()
            .load(self.ptr, MemFlagsData::trusted(), l, len_off);
        let rp = self.b.ins().load(self.ptr, MemFlagsData::trusted(), r, 0);
        let rl = self
            .b
            .ins()
            .load(self.ptr, MemFlagsData::trusted(), r, len_off);
        let fref = self
            .module
            .declare_func_in_func(self.table.str_eq, self.b.func);
        let call = self.b.ins().call(fref, &[lp, ll, rp, rl]);
        let eq = self.b.inst_results(call)[0];
        let zero = self.b.ins().iconst(types::I32, 0);
        let cc = if op == BinOp::Ne {
            IntCC::Equal
        } else {
            IntCC::NotEqual
        };
        self.b.ins().icmp(cc, eq, zero)
    }

    /// `l + r` on `str` — `aura_str_concat` allocates and returns the new
    /// buffer; `len` is `l.len + r.len`. Stored into `place` fieldwise.
    fn str_concat(&mut self, place: &Place, l: &Operand, r: &Operand) {
        let len_off = i32::try_from(self.ptr.bytes()).unwrap_or(i32::MAX);
        let la = self.operand_addr(l);
        let lp = self.b.ins().load(self.ptr, MemFlagsData::trusted(), la, 0);
        let ll = self
            .b
            .ins()
            .load(self.ptr, MemFlagsData::trusted(), la, len_off);
        let ra = self.operand_addr(r);
        let rp = self.b.ins().load(self.ptr, MemFlagsData::trusted(), ra, 0);
        let rl = self
            .b
            .ins()
            .load(self.ptr, MemFlagsData::trusted(), ra, len_off);
        let fref = self
            .module
            .declare_func_in_func(self.table.str_concat, self.b.func);
        let call = self.b.ins().call(fref, &[lp, ll, rp, rl]);
        let new_ptr = self.b.inst_results(call)[0];
        let new_len = self.b.ins().iadd(ll, rl);
        let base = self.place_addr(place);
        self.b
            .ins()
            .store(MemFlagsData::trusted(), new_ptr, base, 0);
        self.b
            .ins()
            .store(MemFlagsData::trusted(), new_len, base, len_off);
    }

    fn call(&mut self, dest: &Place, callee: Callee, args: &[Operand]) {
        if let Callee::Builtin(b) = callee {
            self.call_builtin(dest, b, args);
            return;
        }
        let fid = match callee {
            Callee::Fn(i) => self.table.fns.get(&i).copied(),
            Callee::Extern(b, f) => self.table.externs.get(&(b, f)).copied(),
            Callee::Builtin(_) => unreachable!(),
        };
        let Some(fid) = fid else { return };
        let fref = self.module.declare_func_in_func(fid, self.b.func);

        let mut argvals: Vec<Value> = Vec::new();
        // sret: an aggregate destination means the callee returns through a
        // hidden leading pointer (internal fns only — externs with aggregate
        // returns were rejected at declaration).
        let ret_agg = is_aggregate(&self.place_ty(dest));
        if ret_agg {
            argvals.push(self.place_addr(dest));
        }
        let pi = usize::from(ret_agg);
        for (ai, op) in args.iter().enumerate() {
            let pty = self.operand_param_ty(callee, pi + ai);
            if pty.as_ref().is_some_and(is_aggregate) {
                argvals.push(self.operand_addr(op));
            } else {
                argvals.push(self.operand_val(op));
            }
        }
        let inst = self.b.ins().call(fref, &argvals);
        let results = self.b.inst_results(inst);
        if let Some(&v) = results.first() {
            self.store(dest, v);
        }
    }

    /// Prelude builtin dispatch — `print`-family flattens `str` to
    /// `{ptr, len}`; `exit` takes its `i64` code; the `vec` family is
    /// byte-oriented (`elem_size` is a runtime arg, so no
    /// monomorphization is needed).
    fn call_builtin(&mut self, dest: &Place, b: BuiltinFn, args: &[Operand]) {
        match b {
            BuiltinFn::Print | BuiltinFn::Println | BuiltinFn::Eprint | BuiltinFn::Eprintln => {
                let Some(op) = args.first() else { return };
                let len_off = i32::try_from(self.ptr.bytes()).unwrap_or(i32::MAX);
                let base = self.operand_addr(op);
                let lp = self
                    .b
                    .ins()
                    .load(self.ptr, MemFlagsData::trusted(), base, 0);
                let ll = self
                    .b
                    .ins()
                    .load(self.ptr, MemFlagsData::trusted(), base, len_off);
                self.call_builtin_sym(b, &[lp, ll]);
            }
            BuiltinFn::Exit => {
                let Some(op) = args.first() else { return };
                let v = self.operand_val(op);
                self.call_builtin_sym(b, &[v]);
            }
            BuiltinFn::VecNew => {
                // `vec<T>` = `{ptr: 0, len: 0, cap: 0}` — an unallocated
                // buffer; `vec_push` allocates on first use.
                let base = self.place_addr(dest);
                let z = self.b.ins().iconst(self.ptr, 0);
                let ps = i32::try_from(self.ptr.bytes()).unwrap_or(i32::MAX);
                for off in [0, ps, 2 * ps] {
                    self.b.ins().store(MemFlagsData::trusted(), z, base, off);
                }
            }
            BuiltinFn::VecPush => self.vec_push(args),
            BuiltinFn::VecGet => self.vec_get_elem(dest, args),
            BuiltinFn::VecSet => self.vec_set(args),
            BuiltinFn::Args => {
                // `aura_rt_args(out)` — the runtime writes the whole
                // `{ptr,len,cap}` vec<str> triple into `dest`.
                let da = self.place_addr(dest);
                self.call_builtin_sym(b, &[da]);
            }
            BuiltinFn::Env => {
                // `aura_rt_env(name.ptr, name.len, out)` — writes the
                // `{ptr,len}` str into `dest`.
                let Some(op) = args.first() else { return };
                let len_off = i32::try_from(self.ptr.bytes()).unwrap_or(i32::MAX);
                let base = self.operand_addr(op);
                let np = self
                    .b
                    .ins()
                    .load(self.ptr, MemFlagsData::trusted(), base, 0);
                let nl = self
                    .b
                    .ins()
                    .load(self.ptr, MemFlagsData::trusted(), base, len_off);
                let da = self.place_addr(dest);
                self.call_builtin_sym(b, &[np, nl, da]);
            }
        }
    }

    /// Call a declared `aura_*` builtin import, returning the result
    /// value if the symbol returns one (`vec_push`/`vec_get` return the
    /// data/element pointer; the print family and `exit` return void).
    fn call_builtin_sym(&mut self, b: BuiltinFn, argvals: &[Value]) -> Option<Value> {
        let &fid = self.table.builtins.get(&b)?;
        let fref = self.module.declare_func_in_func(fid, self.b.func);
        let inst = self.b.ins().call(fref, argvals);
        self.b.inst_results(inst).first().copied()
    }

    /// Byte size of a `vec` element — aggregate layout size for
    /// aggregates, scalar size otherwise.
    fn elem_size(&self, ty: &Type) -> u32 {
        if is_aggregate(ty) {
            self.agg_layout(ty).map_or(self.ptr.bytes(), |(s, _)| s)
        } else {
            crate::layout::scalar_size_align(ty, self.ptr.bytes()).map_or(1, |(s, _)| s)
        }
    }

    /// `vec<T>`'s element type from a `vec` operand.
    fn vec_elem_ty(&self, v: &Operand) -> Option<Type> {
        match self.operand_ty(v) {
            Type::Vec(t) => Some(*t),
            _ => None,
        }
    }

    /// Address of an element operand's bytes — the operand's aggregate
    /// address for aggregates; a stack temp holding the scalar value
    /// otherwise.
    fn elem_addr(&mut self, op: &Operand, ty: &Type) -> Value {
        if is_aggregate(ty) {
            return self.operand_addr(op);
        }
        let (size, align) =
            crate::layout::scalar_size_align(ty, self.ptr.bytes()).unwrap_or((1, 1));
        let slot = self.b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            size,
            u8::try_from(align.trailing_zeros()).unwrap_or(u8::MAX),
        ));
        let addr = self.b.ins().stack_addr(self.ptr, slot, 0);
        let v = self.operand_val(op);
        self.b.ins().store(MemFlagsData::trusted(), v, addr, 0);
        addr
    }

    /// `vec_push(v, x)` — `aura_vec_push(data, len, cap, &x, esize)`
    /// grows+appends in the runtime and returns the (possibly
    /// reallocated) data pointer; `v.ptr`/`v.len` update in place.
    fn vec_push(&mut self, args: &[Operand]) {
        let [v, x] = args else { return };
        let Some(ety) = self.vec_elem_ty(v) else {
            return;
        };
        let esize = self.elem_size(&ety);
        let ps = i32::try_from(self.ptr.bytes()).unwrap_or(i32::MAX);
        let va = self.operand_addr(v);
        let data = self.b.ins().load(self.ptr, MemFlagsData::trusted(), va, 0);
        let len = self.b.ins().load(self.ptr, MemFlagsData::trusted(), va, ps);
        let cap = self
            .b
            .ins()
            .load(self.ptr, MemFlagsData::trusted(), va, 2 * ps);
        let xa = self.elem_addr(x, &ety);
        let esz = self.b.ins().iconst(self.ptr, i64::from(esize));
        // The runtime reports the post-push capacity through `cap_out`.
        let cap_slot = self.b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            self.ptr.bytes(),
            u8::try_from(self.ptr.bytes().trailing_zeros()).unwrap_or(u8::MAX),
        ));
        let cap_out = self.b.ins().stack_addr(self.ptr, cap_slot, 0);
        let Some(new_data) =
            self.call_builtin_sym(BuiltinFn::VecPush, &[data, len, cap, xa, esz, cap_out])
        else {
            return;
        };
        self.b.ins().store(MemFlagsData::trusted(), new_data, va, 0);
        let new_cap = self
            .b
            .ins()
            .load(self.ptr, MemFlagsData::trusted(), cap_out, 0);
        self.b
            .ins()
            .store(MemFlagsData::trusted(), new_cap, va, 2 * ps);
        let one = self.b.ins().iconst(self.ptr, 1);
        let new_len = self.b.ins().iadd(len, one);
        self.b.ins().store(MemFlagsData::trusted(), new_len, va, ps);
    }

    /// `aura_vec_get(data, len, idx, esize) -> elem_ptr` — bounds-checks
    /// in the runtime (exit 101 on OOB) and returns the element address.
    fn vec_elem_ptr(&mut self, v: &Operand, i: &Operand) -> Option<Value> {
        let ps = i32::try_from(self.ptr.bytes()).unwrap_or(i32::MAX);
        let ety = self.vec_elem_ty(v)?;
        let esize = self.elem_size(&ety);
        let va = self.operand_addr(v);
        let data = self.b.ins().load(self.ptr, MemFlagsData::trusted(), va, 0);
        let len = self.b.ins().load(self.ptr, MemFlagsData::trusted(), va, ps);
        let iv = self.operand_val(i);
        let esz = self.b.ins().iconst(self.ptr, i64::from(esize));
        self.call_builtin_sym(BuiltinFn::VecGet, &[data, len, iv, esz])
    }

    /// `vec_get(v, i) -> T` — scalar elements load from the returned
    /// pointer; aggregates copy `esize` bytes into the destination.
    fn vec_get_elem(&mut self, dest: &Place, args: &[Operand]) {
        let [v, i] = args else { return };
        let Some(ety) = self.vec_elem_ty(v) else {
            return;
        };
        let Some(ep) = self.vec_elem_ptr(v, i) else {
            return;
        };
        if is_aggregate(&ety) {
            let size = self.elem_size(&ety);
            let align8 = u8::try_from(self.ptr.bytes()).unwrap_or(u8::MAX);
            let da = self.place_addr(dest);
            self.b.emit_small_memory_copy(
                self.frontend_cfg,
                da,
                ep,
                u64::from(size),
                align8,
                align8,
                true,
                MemFlagsData::trusted(),
            );
        } else {
            let ct = clif_ty(&ety, self.ptr);
            let x = self.b.ins().load(ct, MemFlagsData::trusted(), ep, 0);
            self.store(dest, x);
        }
    }

    /// `vec_set(v, i, x)` — bounds-checked element pointer, then an
    /// `esize`-byte copy of `x`'s bytes into the slot.
    fn vec_set(&mut self, args: &[Operand]) {
        let [v, i, x] = args else { return };
        let Some(ety) = self.vec_elem_ty(v) else {
            return;
        };
        let Some(ep) = self.vec_elem_ptr(v, i) else {
            return;
        };
        let xa = self.elem_addr(x, &ety);
        let size = self.elem_size(&ety);
        let align8 = u8::try_from(self.ptr.bytes()).unwrap_or(u8::MAX);
        self.b.emit_small_memory_copy(
            self.frontend_cfg,
            ep,
            xa,
            u64::from(size),
            align8,
            align8,
            true,
            MemFlagsData::trusted(),
        );
    }

    /// Declared type of the `ai`-th *clif* parameter of `callee` (accounting
    /// for the hidden sret param).
    fn operand_param_ty(&self, callee: Callee, ai: usize) -> Option<Type> {
        let (params, ret): (&[aura_salsa_db::ParamSig], Option<&aura_salsa_db::TypeName>) =
            match callee {
                Callee::Fn(i) => match self.items.items.get(i as usize) {
                    Some(ItemSig::Fn { params, ret, .. }) => (params, ret.as_ref()),
                    _ => return None,
                },
                Callee::Extern(b, f) => match self.items.items.get(b as usize) {
                    Some(ItemSig::ExternBlock { fns, .. }) => {
                        let s = fns.get(f as usize)?;
                        (&s.params, s.ret.as_ref())
                    }
                    _ => return None,
                },
                Callee::Builtin(_) => return None, // handled by `call_builtin`
            };
        let ret_agg = ret.is_some_and(|t| is_aggregate(&lower_ty(self.items, t)));
        let idx = if ret_agg { ai.checked_sub(1)? } else { ai };
        params.get(idx).map(|p| lower_ty(self.items, &p.ty))
    }

    fn binop(&mut self, op: BinOp, ty: &Type, lhs: Value, rhs: Value) -> Value {
        match ty {
            // `str` equality — byte-wise via the runtime helper; `lhs`/`rhs`
            // are the operands' aggregate addresses.
            Type::Str => self.str_eq(op, lhs, rhs),
            Type::Float(_) => match op {
                BinOp::Add => self.b.ins().fadd(lhs, rhs),
                BinOp::Sub => self.b.ins().fsub(lhs, rhs),
                BinOp::Mul => self.b.ins().fmul(lhs, rhs),
                BinOp::Div => self.b.ins().fdiv(lhs, rhs),
                BinOp::Rem => {
                    // fmod via libcall isn't wired yet — fdiv-ftrunc-fmul chain.
                    let quot = self.b.ins().fdiv(lhs, rhs);
                    let tr = self.b.ins().trunc(quot);
                    let prod = self.b.ins().fmul(tr, rhs);
                    self.b.ins().fsub(lhs, prod)
                }
                _ => self.fcmp_of(op, lhs, rhs),
            },
            Type::Int(i) => {
                let signed = i.is_signed();
                match op {
                    BinOp::Add => self.b.ins().iadd(lhs, rhs),
                    BinOp::Sub => self.b.ins().isub(lhs, rhs),
                    BinOp::Mul => self.b.ins().imul(lhs, rhs),
                    BinOp::Div if signed => self.b.ins().sdiv(lhs, rhs),
                    BinOp::Div => self.b.ins().udiv(lhs, rhs),
                    BinOp::Rem if signed => self.b.ins().srem(lhs, rhs),
                    BinOp::Rem => self.b.ins().urem(lhs, rhs),
                    _ => self.icmp_of(op, lhs, rhs, signed),
                }
            }
            // bool/str/unit comparisons — ints in disguise.
            _ => self.icmp_of(op, lhs, rhs, true),
        }
    }

    fn icmp_of(&mut self, op: BinOp, lhs: Value, rhs: Value, signed: bool) -> Value {
        let cc = match (op, signed) {
            (BinOp::Ne, _) => IntCC::NotEqual,
            (BinOp::Lt, true) => IntCC::SignedLessThan,
            (BinOp::Lt, false) => IntCC::UnsignedLessThan,
            (BinOp::Le, true) => IntCC::SignedLessThanOrEqual,
            (BinOp::Le, false) => IntCC::UnsignedLessThanOrEqual,
            (BinOp::Gt, true) => IntCC::SignedGreaterThan,
            (BinOp::Gt, false) => IntCC::UnsignedGreaterThan,
            (BinOp::Ge, true) => IntCC::SignedGreaterThanOrEqual,
            (BinOp::Ge, false) => IntCC::UnsignedGreaterThanOrEqual,
            (BinOp::And, _) => return self.b.ins().band(lhs, rhs),
            (BinOp::Or, _) => return self.b.ins().bor(lhs, rhs),
            // `Eq` and any leftover op land here.
            _ => IntCC::Equal,
        };
        self.b.ins().icmp(cc, lhs, rhs)
    }

    fn fcmp_of(&mut self, op: BinOp, lhs: Value, rhs: Value) -> Value {
        let cc = match op {
            BinOp::Ne => FloatCC::NotEqual,
            BinOp::Lt => FloatCC::LessThan,
            BinOp::Le => FloatCC::LessThanOrEqual,
            BinOp::Gt => FloatCC::GreaterThan,
            BinOp::Ge => FloatCC::GreaterThanOrEqual,
            // `Eq` and non-comparison ops land here.
            _ => FloatCC::Equal,
        };
        self.b.ins().fcmp(cc, lhs, rhs)
    }

    // ----- places / operands --------------------------------------------------------

    /// Type at `place` — local type walked through field projections.
    fn place_ty(&self, p: &Place) -> Type {
        let mut ty = self
            .mir
            .locals
            .get(p.local as usize)
            .map_or(Type::Error, |l| l.ty.clone());
        for proj in &p.proj {
            match proj {
                aura_mir::Proj::Field(i) => {
                    if matches!(ty, Type::Struct(_) | Type::Str | Type::Vec(_)) {
                        ty = self
                            .layout_for(&ty)
                            .and_then(|l| l.field_tys.get(*i as usize).cloned())
                            .unwrap_or(Type::Error);
                    } else {
                        return Type::Error;
                    }
                }
                aura_mir::Proj::VariantField { variant, field } => {
                    if matches!(ty, Type::Enum(_) | Type::Result(..)) {
                        ty = self
                            .layout_for(&ty)
                            .and_then(|l| l.variants.get(*variant as usize).cloned())
                            .and_then(|v| v.field_tys.get(*field as usize).cloned())
                            .unwrap_or(Type::Error);
                    } else {
                        return Type::Error;
                    }
                }
            }
        }
        ty
    }

    /// Byte address of `place` — aggregates locals hold their base address;
    /// each `Field` projection adds the field offset.
    fn place_addr(&mut self, p: &Place) -> Value {
        // `_0` under sret is the hidden dest pointer.
        let base = if p.local == 0
            && let Some(dest) = self.sret
        {
            dest
        } else {
            self.b.use_var(self.vars[p.local as usize])
        };
        self.add_proj(base, p)
    }

    fn add_proj(&mut self, mut addr: Value, p: &Place) -> Value {
        // Walk projections through struct layouts.
        let mut ty = self
            .mir
            .locals
            .get(p.local as usize)
            .map_or(Type::Error, |l| l.ty.clone());
        for proj in &p.proj {
            match proj {
                aura_mir::Proj::Field(i) => {
                    if matches!(ty, Type::Struct(_) | Type::Str | Type::Vec(_))
                        && let Some(l) = self.layout_for(&ty)
                    {
                        let off = *l.offsets.get(*i as usize).unwrap_or(&0);
                        if off != 0 {
                            addr = self.b.ins().iadd_imm_s(addr, i64::from(off));
                        }
                        ty = l.field_tys.get(*i as usize).cloned().unwrap_or(Type::Error);
                        continue;
                    }
                }
                aura_mir::Proj::VariantField { variant, field } => {
                    if matches!(ty, Type::Enum(_) | Type::Result(..))
                        && let Some(l) = self.layout_for(&ty)
                        && let Some(vl) = l.variants.get(*variant as usize)
                    {
                        let off =
                            l.payload_off + vl.offsets.get(*field as usize).copied().unwrap_or(0);
                        if off != 0 {
                            addr = self.b.ins().iadd_imm_s(addr, i64::from(off));
                        }
                        ty = vl
                            .field_tys
                            .get(*field as usize)
                            .cloned()
                            .unwrap_or(Type::Error);
                        continue;
                    }
                }
            }
            ty = Type::Error;
        }
        addr
    }

    /// Value of `op` (loads through projections for scalar places;
    /// aggregate places yield their address — callers route by type).
    fn operand_val(&mut self, op: &Operand) -> Value {
        match op {
            Operand::Const(c) => self.konst(c),
            Operand::Place(p) => {
                let ty = self.place_ty(p);
                if is_aggregate(&ty) {
                    self.place_addr(p)
                } else if p.proj.is_empty() {
                    self.b.use_var(self.vars[p.local as usize])
                } else {
                    let addr = self.place_addr(p);
                    self.b
                        .ins()
                        .load(clif_ty(&ty, self.ptr), MemFlagsData::trusted(), addr, 0)
                }
            }
        }
    }

    /// Address of `op`'s storage — for aggregate-typed operands only.
    fn operand_addr(&mut self, op: &Operand) -> Value {
        match op {
            Operand::Place(p) => self.place_addr(p),
            Operand::Const(_) => self.operand_val(op), // unreachable for aggregates
        }
    }

    fn operand_ty(&self, op: &Operand) -> Type {
        match op {
            Operand::Const(c) => match c {
                Const::Int(_, i) => Type::Int(*i),
                Const::Float(_, f) => Type::Float(*f),
                Const::Bool(_) => Type::Bool,
                Const::Unit => Type::Unit,
            },
            Operand::Place(p) => self.place_ty(p),
        }
    }

    /// Field-by-field copy for aggregate `dst = src`.
    fn copy_aggregate(&mut self, dst: &Place, src: &Operand) {
        let Some((size, align)) = self.agg_layout(&self.place_ty(dst)) else {
            return;
        };
        let src_addr = self.operand_addr(src);
        let dst_addr = self.place_addr(dst);
        let align8 = u8::try_from(align).unwrap_or(u8::MAX);
        self.b.emit_small_memory_copy(
            self.frontend_cfg,
            dst_addr,
            src_addr,
            u64::from(size),
            align8,
            align8,
            true,
            MemFlagsData::trusted(),
        );
    }

    /// Store scalar `v` into `place`. Bare locals are SSA-defined; projected
    /// places (and `_0` under sret, which is a pointer) store to memory.
    /// A `()`-placeholder value (`I8`) landing in a non-`I8` local is dead
    /// noise from a diverging arm — dropped rather than `def_var`'d, which
    /// would trip the frontend's type check.
    fn store(&mut self, place: &Place, v: Value) {
        let is_sret_dest = place.local == 0 && is_aggregate(&self.mir.ret);
        if place.proj.is_empty() && !is_sret_dest {
            let var = self.vars[place.local as usize];
            let want = clif_ty(&self.mir.locals[place.local as usize].ty, self.ptr);
            if self.b.func.dfg.value_type(v) == want {
                self.b.def_var(var, v);
            }
            return;
        }
        let addr = self.place_addr(place);
        self.b.ins().store(MemFlagsData::trusted(), v, addr, 0);
    }

    #[allow(clippy::cast_possible_truncation)] // f64 literal → f32 const is the intended narrowing
    fn konst(&mut self, c: &Const) -> Value {
        match c {
            Const::Int(v, i) => {
                let ty = clif_ty(&Type::Int(*i), self.ptr);
                self.b.ins().iconst(ty, (*v).cast_signed())
            }
            Const::Float(v, FloatTy::F32) => self.b.ins().f32const(*v as f32),
            Const::Float(v, FloatTy::F64) => self.b.ins().f64const(*v),
            Const::Bool(b) => self.b.ins().iconst(types::I8, i64::from(*b)),
            Const::Unit => self.b.ins().iconst(types::I8, 0),
        }
    }

    // ----- terminators ------------------------------------------------------------

    fn term(&mut self, t: &MirTerm) {
        match t {
            MirTerm::Goto(b) => {
                let bb = self.blocks[*b as usize];
                self.b.ins().jump(bb, &[]);
            }
            MirTerm::Branch { cond, then, else_ } => {
                let c = self.operand_val(cond);
                let t_bb = self.blocks[*then as usize];
                let e_bb = self.blocks[*else_ as usize];
                self.b.ins().brif(c, t_bb, &[], e_bb, &[]);
            }
            MirTerm::Return => {
                // Aggregate returns wrote through the sret pointer; unit/never
                // return nothing — both emit a bare `ret`.
                if is_aggregate(&self.mir.ret) || matches!(self.mir.ret, Type::Unit | Type::Never) {
                    self.b.ins().return_(&[]);
                } else {
                    let v = self.b.use_var(self.vars[0]);
                    self.b.ins().return_(&[v]);
                }
            }
            MirTerm::Unreachable => {
                self.b.ins().trap(TrapCode::unwrap_user(1));
            }
        }
    }
}
