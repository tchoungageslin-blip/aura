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
use aura_common::{Diagnostic, Span, codes};
use aura_mir::{Callee, Const, MirBody, MirStmt, MirTerm, Operand, Place, Rvalue};
use aura_salsa_db::{FileItems, ItemSig};
use aura_semantic::{FloatTy, Type};
use cranelift_codegen::ir::{
    AbiParam, InstBuilder, MemFlagsData, StackSlotData, StackSlotKind, TrapCode, Value,
    condcodes::{FloatCC, IntCC},
    types,
};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_codegen::settings;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{FuncId, Linkage, Module, default_libcall_names};
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
    let mut table = FnTable {
        fns: FxHashMap::default(),
        externs: FxHashMap::default(),
    };
    for (i, sig) in items.iter() {
        match sig {
            ItemSig::Fn { name, .. } => {
                let Some(clif_sig) = fn_sig(items, i, ptr, call_conv) else {
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
                    let Some(clif_sig) = extern_sig(items, sig, ptr, call_conv) else {
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

// ----- signatures --------------------------------------------------------------

/// Is `ty` passed/returned by hidden pointer (internal aggregate ABI)?
fn is_aggregate(ty: &Type) -> bool {
    matches!(ty, Type::Struct(_) | Type::Enum(_))
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

    /// `(size, align)` of an aggregate type's storage.
    fn agg_layout(&self, ty: &Type) -> Option<(u32, u32)> {
        let (Type::Struct(idx) | Type::Enum(idx)) = ty else {
            return None;
        };
        self.layouts
            .get(*idx as usize)
            .and_then(|l| l.as_ref())
            .map(|l| (l.size, l.align))
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
                let lv = self.operand_val(l);
                let rv2 = self.operand_val(r);
                let ty = self.operand_ty(l);
                let v = self.binop(*op, &ty, lv, rv2);
                self.store(place, v);
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

    fn call(&mut self, dest: &Place, callee: Callee, args: &[Operand]) {
        let fid = match callee {
            Callee::Fn(i) => self.table.fns.get(&i).copied(),
            Callee::Extern(b, f) => self.table.externs.get(&(b, f)).copied(),
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
            };
        let ret_agg = ret.is_some_and(|t| is_aggregate(&lower_ty(self.items, t)));
        let idx = if ret_agg { ai.checked_sub(1)? } else { ai };
        params.get(idx).map(|p| lower_ty(self.items, &p.ty))
    }

    fn binop(&mut self, op: BinOp, ty: &Type, lhs: Value, rhs: Value) -> Value {
        match ty {
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
                    if let Type::Struct(idx) = ty {
                        ty = self
                            .layouts
                            .get(idx as usize)
                            .and_then(|l| l.as_ref())
                            .and_then(|l| l.field_tys.get(*i as usize).cloned())
                            .unwrap_or(Type::Error);
                    } else {
                        return Type::Error;
                    }
                }
                aura_mir::Proj::VariantField { variant, field } => {
                    if let Type::Enum(idx) = ty {
                        ty = self
                            .layouts
                            .get(idx as usize)
                            .and_then(|l| l.as_ref())
                            .and_then(|l| l.variants.get(*variant as usize))
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
                    if let Type::Struct(idx) = ty
                        && let Some(l) = self.layouts.get(idx as usize).and_then(|l| l.as_ref())
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
                    if let Type::Enum(idx) = ty
                        && let Some(l) = self.layouts.get(idx as usize).and_then(|l| l.as_ref())
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
