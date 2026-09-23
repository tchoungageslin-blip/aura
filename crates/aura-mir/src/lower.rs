//! AST → MIR lowering.
//!
//! The lowerer re-walks the checked body with its own lexical scope map
//! (`Spur → LocalId`); `typeck_fn` already validated the tree, so
//! unresolvable names/fields become `E3004` "unsupported" or silently
//! dead code rather than fresh semantic errors.
//!
//! Control-flow lowering is standard: expressions return [`Operand`]s and
//! may create temporaries; `if`/`while`/`loop`/`return`/`break`/`continue`
//! manipulate a current-block cursor and emit [`MirTerm`] edges. `&&`/`||`
//! short-circuit through a branch + join so operand side effects are
//! preserved.

use aura_ast::{BinOp, BlockId, Expr, ExprId, Literal, MatchArm, Pattern, Stmt, StmtId};
use aura_common::{Diagnostic, Span, codes};
use aura_hir::hir_fn;
use aura_salsa_db::{Body, Db, FileItems, ItemSig, SourceFile, file_items};
use aura_semantic::{
    Def, FnTypes, IntTy, Resolution, Type, enum_variant_payload, lower_typename, resolved_file,
};
use indexmap::IndexMap;
use lasso::Spur;

use crate::{
    BbId, Callee, Const, LocalId, MirBlock, MirBody, MirLocal, MirStmt, MirTerm, Operand, Place,
    Proj, RESULT_ITEM, Rvalue,
};

/// Lower `parsed.items[index]` to MIR. `None` for non-fn items and extern
/// (bodiless) fns. Semantic diagnostics never reach here — callers must
/// check `check_file` first; this query only reports codegen-level
/// unsupported constructs.
#[salsa::tracked(returns(ref))]
pub fn mir_fn(db: &dyn Db, file: SourceFile, index: u32) -> Option<MirBody> {
    let items = file_items(db, file);
    let ItemSig::Fn {
        name,
        params,
        ret,
        is_extern,
    } = items.items.get(index as usize)?
    else {
        return None;
    };
    if *is_extern {
        return None;
    }
    let hir = hir_fn(db, file, index).as_ref()?;
    let body = &hir.body;
    let types = &hir.types;
    let res = resolved_file(db, file);
    let ret_ty = ret
        .as_ref()
        .map_or(Type::Unit, |t| lower_typename(items, t));

    let mut locals = Vec::new();
    // _0: return place.
    locals.push(MirLocal {
        ty: ret_ty.clone(),
        name: None,
        mutable: true,
    });
    let mut scopes: Vec<IndexMap<Spur, LocalId>> = vec![IndexMap::new()];
    for (i, p) in params.iter().enumerate() {
        let spur = body.params.get(i).copied();
        let id = u32::try_from(locals.len()).unwrap_or(u32::MAX);
        locals.push(MirLocal {
            ty: lower_typename(items, &p.ty),
            name: Some(p.name.clone()),
            mutable: false,
        });
        if let Some(s) = spur {
            scopes[0].insert(s, id);
        }
    }

    let mut l = Lowerer {
        body,
        types,
        items,
        res,
        locals,
        blocks: vec![MirBlock {
            stmts: Vec::new(),
            term: MirTerm::Unreachable,
        }],
        scopes,
        break_stack: Vec::new(),
        cont_stack: Vec::new(),
        diags: Vec::new(),
        cur: 0,
        terminated: false,
    };
    let tail = l.lower_block(body.root);
    if !l.terminated {
        // A `()` operand here is the placeholder for a diverging tail —
        // skip assigning it into a typed `_0`.
        if let Some(op) = tail {
            let meaningful =
                !matches!(op, Operand::Const(Const::Unit)) || matches!(ret_ty, Type::Unit);
            if meaningful {
                l.assign(Place::local(0), Rvalue::Use(op));
            }
        }
        l.terminate(MirTerm::Return);
    }
    Some(MirBody {
        name: name.clone(),
        locals: l.locals,
        param_count: u32::try_from(params.len()).unwrap_or(u32::MAX),
        blocks: l.blocks,
        ret: ret_ty,
        diagnostics: l.diags,
    })
}

struct Lowerer<'a> {
    body: &'a Body,
    types: &'a FnTypes,
    items: &'a FileItems,
    res: &'a Resolution,
    locals: Vec<MirLocal>,
    blocks: Vec<MirBlock>,
    scopes: Vec<IndexMap<Spur, LocalId>>,
    /// Loop exit targets for `break` — innermost last.
    break_stack: Vec<BbId>,
    /// Loop continue targets — innermost last.
    cont_stack: Vec<BbId>,
    diags: Vec<Diagnostic>,
    /// Block currently being appended to.
    cur: BbId,
    /// True once `cur` has a terminator — the next statement silently
    /// continues into a fresh (unreachable) block.
    terminated: bool,
}

impl Lowerer<'_> {
    // ----- block plumbing ------------------------------------------------------

    fn new_block(&mut self) -> BbId {
        let id = u32::try_from(self.blocks.len()).unwrap_or(u32::MAX);
        self.blocks.push(MirBlock {
            stmts: Vec::new(),
            term: MirTerm::Unreachable,
        });
        id
    }

    fn goto(&mut self, target: BbId) {
        self.terminate(MirTerm::Goto(target));
    }

    /// Emit a statement; no-ops after a terminator (dead code).
    fn push(&mut self, stmt: MirStmt) {
        if self.terminated {
            return;
        }
        let cur = self.cur as usize;
        self.blocks[cur].stmts.push(stmt);
    }

    fn terminate(&mut self, term: MirTerm) {
        if self.terminated {
            return;
        }
        let cur = self.cur as usize;
        self.blocks[cur].term = term;
        self.terminated = true;
    }

    /// Move the append cursor to `bb` (used after lowering both arms of a
    /// branch to start the join block).
    fn switch_to(&mut self, bb: BbId) {
        self.cur = bb;
        self.terminated = false;
    }

    fn assign(&mut self, place: Place, rv: Rvalue) {
        self.push(MirStmt::Assign(place, rv));
    }

    // ----- locals / scopes -------------------------------------------------------

    fn new_local(&mut self, ty: Type, name: Option<String>, mutable: bool) -> LocalId {
        let id = u32::try_from(self.locals.len()).unwrap_or(u32::MAX);
        self.locals.push(MirLocal { ty, name, mutable });
        id
    }

    fn temp(&mut self, ty: Type) -> LocalId {
        self.new_local(ty, None, true)
    }

    fn lookup(&self, spur: Spur) -> Option<LocalId> {
        self.scopes.iter().rev().find_map(|s| s.get(&spur)).copied()
    }

    fn name(&self, spur: Spur) -> &str {
        self.body.name(spur)
    }

    /// Finalized type of body expression `id` (inference vars already
    /// resolved by `typeck_fn`).
    fn expr_ty(&self, id: ExprId) -> &Type {
        self.types.exprs.get(id.0 as usize).unwrap_or(&Type::Error)
    }

    fn span(&self, id: ExprId) -> Span {
        self.body.ast.expr_span(id)
    }

    fn unsupported(&mut self, what: &str, span: Span) {
        self.diags.push(Diagnostic::error(
            codes::CG_UNSUPPORTED,
            format!("codegen: {what} is not yet supported"),
            span,
        ));
    }

    // ----- statements ------------------------------------------------------------

    /// Lower a block in statement context. Returns the tail value's operand
    /// (for value-position blocks); `None` if the block diverged or has no
    /// tail.
    fn lower_block(&mut self, b: BlockId) -> Option<Operand> {
        self.scopes.push(IndexMap::new());
        let block = self.body.ast.block(b).clone();
        for &s in &block.stmts {
            self.stmt(s);
        }
        let tail = if self.terminated {
            None
        } else {
            block.tail.map(|t| self.expr(t))
        };
        self.scopes.pop();
        tail
    }

    fn stmt(&mut self, s: StmtId) {
        match self.body.ast.stmt(s) {
            Stmt::Let {
                name,
                mutable,
                init,
                ..
            } => {
                let op = self.expr(*init);
                let ty = self.expr_ty(*init).clone();
                let id = self.new_local(ty, Some(self.name(*name).to_owned()), *mutable);
                self.assign(Place::local(id), Rvalue::Use(op));
                self.scopes.last_mut().unwrap().insert(*name, id);
            }
            Stmt::Expr(e) => {
                self.expr(*e);
            }
            Stmt::Return(e) => {
                let op = e.map(|e| self.expr(e));
                if let Some(op) = op {
                    self.assign(Place::local(0), Rvalue::Use(op));
                }
                self.terminate(MirTerm::Return);
            }
            Stmt::While { cond, body } => self.while_stmt(*cond, *body),
            Stmt::Loop { body } => self.loop_stmt(*body),
            Stmt::Break => {
                if let Some(&exit) = self.break_stack.last() {
                    self.goto(exit);
                }
            }
            Stmt::Continue => {
                if let Some(&header) = self.cont_stack.last() {
                    self.goto(header);
                }
            }
            Stmt::Error => {}
        }
    }

    fn while_stmt(&mut self, cond: ExprId, body: BlockId) {
        let header = self.new_block();
        let body_bb = self.new_block();
        let exit = self.new_block();
        self.goto(header);
        self.switch_to(header);
        let c = self.expr(cond);
        self.terminate(MirTerm::Branch {
            cond: c,
            then: body_bb,
            else_: exit,
        });
        self.break_stack.push(exit);
        self.cont_stack.push(header);
        self.switch_to(body_bb);
        self.lower_block(body);
        self.goto(header);
        self.break_stack.pop();
        self.cont_stack.pop();
        self.switch_to(exit);
    }

    fn loop_stmt(&mut self, body: BlockId) {
        let body_bb = self.new_block();
        let exit = self.new_block();
        self.goto(body_bb);
        self.break_stack.push(exit);
        self.cont_stack.push(body_bb);
        self.switch_to(body_bb);
        self.lower_block(body);
        self.goto(body_bb);
        self.break_stack.pop();
        self.cont_stack.pop();
        self.switch_to(exit);
    }

    // ----- expressions -------------------------------------------------------------

    /// Evaluate expression `id`, returning the operand holding its value.
    /// Aggregate results are always `Operand::Place` (the temp local holds
    /// an address); scalar calls/literals may produce temporaries too.
    fn expr(&mut self, id: ExprId) -> Operand {
        match self.body.ast.expr(id) {
            Expr::Error => Operand::Const(Const::Unit),
            Expr::Literal(l) => match l {
                Literal::Int(v) => match self.expr_ty(id) {
                    Type::Int(i) => Operand::Const(Const::Int(*v, *i)),
                    _ => Operand::Const(Const::Unit),
                },
                Literal::Float(v) => match self.expr_ty(id) {
                    Type::Float(f) => Operand::Const(Const::Float(*v, *f)),
                    _ => Operand::Const(Const::Unit),
                },
                Literal::Bool(b) => Operand::Const(Const::Bool(*b)),
                Literal::Unit => Operand::Const(Const::Unit),
                Literal::Str(_) => {
                    self.unsupported("string literals", self.span(id));
                    Operand::Const(Const::Unit)
                }
            },
            Expr::Ident(name) => self.ident(*name, id),
            Expr::Paren(inner) => self.expr(*inner),
            Expr::Unary { op, operand } => {
                let v = self.expr(*operand);
                let ty = self.expr_ty(id).clone();
                self.mk_temp(ty, Rvalue::Unary(*op, v))
            }
            Expr::Binary { op, lhs, rhs } => match op {
                BinOp::And | BinOp::Or => self.lazy_bool(*op, *lhs, *rhs, id),
                _ => {
                    let l = self.expr(*lhs);
                    let r = self.expr(*rhs);
                    let ty = self.expr_ty(id).clone();
                    self.mk_temp(ty, Rvalue::Binary(*op, l, r))
                }
            },
            Expr::Assign { target, value } => {
                let op = self.expr(*value);
                if let Some(place) = self.place(*target) {
                    self.assign(place, Rvalue::Use(op));
                }
                Operand::Const(Const::Unit)
            }
            Expr::Call { callee, args } => self.call(*callee, args, id),
            Expr::Field { object, field } => {
                if let Some(p) = self.place(id) {
                    Operand::Place(p)
                } else {
                    let _ = object;
                    let _ = field;
                    Operand::Const(Const::Unit)
                }
            }
            Expr::If {
                cond,
                then_block,
                else_branch,
            } => self.if_expr(*cond, *then_block, *else_branch, id),
            Expr::Block(b) | Expr::Unsafe(b) => {
                let ty = self.expr_ty(id).clone();
                match self.lower_block(*b) {
                    Some(op) if !matches!(ty, Type::Unit | Type::Never | Type::Error) => {
                        let t = self.temp(ty);
                        self.assign(Place::local(t), Rvalue::Use(op));
                        Operand::Place(Place::local(t))
                    }
                    Some(op) => op,
                    None => Operand::Const(Const::Unit),
                }
            }
            Expr::Match { scrutinee, arms } => self.match_expr(*scrutinee, arms, id),
            Expr::StructLit { name, fields } => self.struct_lit(*name, fields, id),
            Expr::Try { expr } => self.try_expr(*expr),
        }
    }

    fn mk_temp(&mut self, ty: Type, rv: Rvalue) -> Operand {
        if matches!(ty, Type::Unit | Type::Never | Type::Error) {
            return Operand::Const(Const::Unit);
        }
        let t = self.temp(ty);
        self.assign(Place::local(t), rv);
        Operand::Place(Place::local(t))
    }

    fn ident(&mut self, name: Spur, id: ExprId) -> Operand {
        if let Some(l) = self.lookup(name) {
            return Operand::Place(Place::local(l));
        }
        match self.res.lookup(self.name(name)) {
            // Bare unit variant (`Color::Red` written as `Red`) constructs
            // a tag-only enum value.
            Some(Def::Variant(e, vi))
                if aura_semantic::enum_variant_payload(self.items, e, vi).is_empty() =>
            {
                self.enum_lit(e, vi, Vec::new(), id)
            }
            // Bare fn/payload-variant/`Ok`/`Err` used as a value — needs
            // function pointers, deferred.
            Some(
                Def::Fn(_) | Def::ExternFn(..) | Def::Variant(..) | Def::ResultOk | Def::ResultErr,
            ) => {
                self.unsupported("function/variant values", self.span(id));
                Operand::Const(Const::Unit)
            }
            _ => {
                // Undefined names were diagnosed by typeck; emit Unit.
                Operand::Const(Const::Unit)
            }
        }
    }

    /// `a && b` / `a || b` — short-circuit so `b`'s side effects only run
    /// when the lhs forces evaluation.
    fn lazy_bool(&mut self, op: BinOp, lhs: ExprId, rhs: ExprId, id: ExprId) -> Operand {
        let _ = id;
        let res = self.temp(Type::Bool);
        let rhs_bb = self.new_block();
        let shortcut = self.new_block();
        let join = self.new_block();
        let l = self.expr(lhs);
        // `a && b`: true → rhs, false → shortcut(res=false).
        // `a || b`: true → shortcut(res=true), false → rhs.
        let (then, else_) = match op {
            BinOp::And => (rhs_bb, shortcut),
            _ => (shortcut, rhs_bb),
        };
        self.terminate(MirTerm::Branch {
            cond: l,
            then,
            else_,
        });
        self.switch_to(shortcut);
        let short_val = Const::Bool(matches!(op, BinOp::Or));
        self.assign(Place::local(res), Rvalue::Use(Operand::Const(short_val)));
        self.goto(join);
        self.switch_to(rhs_bb);
        let r = self.expr(rhs);
        self.assign(Place::local(res), Rvalue::Use(r));
        self.goto(join);
        self.switch_to(join);
        Operand::Place(Place::local(res))
    }

    fn if_expr(
        &mut self,
        cond: ExprId,
        then_block: BlockId,
        else_branch: Option<ExprId>,
        id: ExprId,
    ) -> Operand {
        let ty = self.expr_ty(id).clone();
        let needs_value = !matches!(ty, Type::Unit | Type::Never | Type::Error);
        let res = needs_value.then(|| self.temp(ty));
        let then_bb = self.new_block();
        let else_bb = self.new_block();
        let join = self.new_block();
        let c = self.expr(cond);
        self.terminate(MirTerm::Branch {
            cond: c,
            then: then_bb,
            else_: else_bb,
        });
        self.switch_to(then_bb);
        if let Some(op) = self.lower_block(then_block)
            && let Some(r) = res
        {
            self.assign(Place::local(r), Rvalue::Use(op));
        }
        self.goto(join);
        self.switch_to(else_bb);
        if let Some(e) = else_branch {
            let op = self.expr(e);
            if let Some(r) = res {
                self.assign(Place::local(r), Rvalue::Use(op));
            }
        }
        self.goto(join);
        self.switch_to(join);
        res.map_or(Operand::Const(Const::Unit), |r| {
            Operand::Place(Place::local(r))
        })
    }

    fn call(&mut self, callee: ExprId, args: &[ExprId], id: ExprId) -> Operand {
        let target = match self.body.ast.expr(callee) {
            Expr::Ident(name) if self.lookup(*name).is_none() => {
                match self.res.lookup(self.name(*name)) {
                    Some(Def::Fn(i)) => Callee::Fn(i),
                    Some(Def::ExternFn(b, f)) => Callee::Extern(b, f),
                    Some(Def::Variant(e, vi)) => {
                        let ops: Vec<Operand> = args.iter().map(|&a| self.expr(a)).collect();
                        return self.enum_lit(e, vi, ops, id);
                    }
                    // Built-in `Ok`/`Err` — `Result` shares the enum repr;
                    // the dest place's `Type::Result` drives layout.
                    Some(Def::ResultOk | Def::ResultErr) => {
                        let variant = u32::from(matches!(
                            self.res.lookup(self.name(*name)),
                            Some(Def::ResultErr)
                        ));
                        let ops: Vec<Operand> = args.iter().map(|&a| self.expr(a)).collect();
                        return self.enum_lit(RESULT_ITEM, variant, ops, id);
                    }
                    _ => {
                        for &a in args {
                            self.expr(a);
                        }
                        return Operand::Const(Const::Unit);
                    }
                }
            }
            _ => {
                self.unsupported("indirect calls", self.span(callee));
                for &a in args {
                    self.expr(a);
                }
                return Operand::Const(Const::Unit);
            }
        };
        let ops: Vec<Operand> = args.iter().map(|&a| self.expr(a)).collect();
        let ty = self.expr_ty(id).clone();
        let t = self.temp(ty);
        self.assign(Place::local(t), Rvalue::Call(target, ops));
        Operand::Place(Place::local(t))
    }

    fn struct_lit(&mut self, name: Spur, fields: &[(Spur, ExprId)], id: ExprId) -> Operand {
        let text = self.name(name).to_owned();
        let Some(Def::Struct(item)) = self.res.lookup(&text) else {
            for (_, e) in fields {
                self.expr(*e);
            }
            return Operand::Const(Const::Unit);
        };
        // Map each literal field to its declared index.
        let field_sigs: &Vec<aura_salsa_db::ParamSig> = match self.items.items.get(item as usize) {
            Some(ItemSig::Struct { fields, .. }) => fields,
            _ => return Operand::Const(Const::Unit),
        };
        let mut parts: Vec<(u32, Operand)> = Vec::with_capacity(fields.len());
        for (fname, e) in fields {
            let ftext = self.name(*fname);
            let idx = field_sigs
                .iter()
                .position(|f| f.name == ftext)
                .map(|i| u32::try_from(i).unwrap_or(u32::MAX));
            let op = self.expr(*e);
            if let Some(i) = idx {
                parts.push((i, op));
            }
        }
        let ty = self.expr_ty(id).clone();
        let t = self.temp(ty);
        self.assign(
            Place::local(t),
            Rvalue::StructLit {
                item,
                fields: parts,
            },
        );
        Operand::Place(Place::local(t))
    }

    /// `Enum::Variant(args..)` construction — a temp holding
    /// `{ tag: i32 = variant, payload }`.
    fn enum_lit(&mut self, item: u32, variant: u32, args: Vec<Operand>, id: ExprId) -> Operand {
        let ty = self.expr_ty(id).clone();
        let t = self.temp(ty);
        let fields: Vec<(u32, Operand)> = (0..).take(args.len()).zip(args).collect();
        self.assign(
            Place::local(t),
            Rvalue::EnumLit {
                item,
                variant,
                fields,
            },
        );
        Operand::Place(Place::local(t))
    }

    /// `e?` — desugars to a discriminant branch: `Ok` continues with the
    /// payload place `scr.<v0>.0`; `Err` writes `_0 = Err(scr.<v1>.0)` and
    /// returns. The error path diverges, so no join block is needed.
    fn try_expr(&mut self, inner: ExprId) -> Operand {
        let scr = self.expr(inner);
        let scr_ty = self.expr_ty(inner).clone();
        if !matches!(scr_ty, Type::Result(..)) {
            // Typeck already diagnosed; emit something harmless.
            return scr;
        }
        let scr_place = if let Operand::Place(p) = scr {
            p
        } else {
            let t = self.temp(scr_ty);
            self.assign(Place::local(t), Rvalue::Use(scr));
            Place::local(t)
        };
        let ok_bb = self.new_block();
        let err_bb = self.new_block();
        let cond = self.variant_test(&scr_place, RESULT_ITEM, 0);
        self.terminate(MirTerm::Branch {
            cond,
            then: ok_bb,
            else_: err_bb,
        });
        // `Err(e)` → `_0 = Err(e); return` — the fn's declared return
        // type is `Result`, so `_0`'s layout matches.
        self.switch_to(err_bb);
        let e_place = Place {
            local: scr_place.local,
            proj: {
                let mut p = scr_place.proj.clone();
                p.push(Proj::VariantField {
                    variant: 1,
                    field: 0,
                });
                p
            },
        };
        self.assign(
            Place::local(0),
            Rvalue::EnumLit {
                item: RESULT_ITEM,
                variant: 1,
                fields: vec![(0, Operand::Place(e_place))],
            },
        );
        self.terminate(MirTerm::Return);
        // `Ok(v)` → the payload place itself is the expression's value.
        self.switch_to(ok_bb);
        let mut proj = scr_place.proj;
        proj.push(Proj::VariantField {
            variant: 0,
            field: 0,
        });
        Operand::Place(Place {
            local: scr_place.local,
            proj,
        })
    }

    // ----- match ------------------------------------------------------------------

    /// `match scr { arms }` — a linear test chain: each arm gets a test
    /// block (discriminant compare / literal compare / unconditional) and
    /// a body block; all bodies join at `join`. Exhaustiveness is already
    /// enforced by typeck, so the fall-through block is `Unreachable`.
    fn match_expr(&mut self, scrutinee: ExprId, arms: &[MatchArm], id: ExprId) -> Operand {
        let scr = self.expr(scrutinee);
        let scr_ty = self.expr_ty(scrutinee).clone();
        // Materialize the scrutinee into a place we can project/test.
        let scr_place = if let Operand::Place(p) = scr {
            p
        } else {
            let t = self.temp(scr_ty.clone());
            self.assign(Place::local(t), Rvalue::Use(scr));
            Place::local(t)
        };
        let ty = self.expr_ty(id).clone();
        let res = (!matches!(ty, Type::Unit | Type::Never | Type::Error)).then(|| self.temp(ty));
        let join = self.new_block();

        for arm in arms {
            let arm_bb = self.new_block();
            let next = self.new_block();
            match self.match_test(&arm.pattern, &scr_place, &scr_ty) {
                Some(cond) => self.terminate(MirTerm::Branch {
                    cond,
                    then: arm_bb,
                    else_: next,
                }),
                None => self.terminate(MirTerm::Goto(arm_bb)),
            }
            self.switch_to(arm_bb);
            self.scopes.push(IndexMap::new());
            self.bind_pattern(&arm.pattern, &scr_place, &scr_ty);
            let op = self.expr(arm.body);
            if let Some(r) = res {
                self.assign(Place::local(r), Rvalue::Use(op));
            }
            self.scopes.pop();
            self.goto(join);
            self.switch_to(next);
        }
        // Non-exhaustive residue: unreachable by typeck's exhaustiveness
        // proof; the terminator keeps the CFG well-formed if that was
        // suppressed by earlier errors.
        self.terminate(MirTerm::Unreachable);
        self.switch_to(join);
        res.map_or(Operand::Const(Const::Unit), |r| {
            Operand::Place(Place::local(r))
        })
    }

    /// Emit the test for `pat` into the current block. Returns the bool
    /// operand guarding the arm, or `None` for an unconditional arm
    /// (wildcard / binding ident).
    fn match_test(&mut self, pat: &Pattern, scr: &Place, scr_ty: &Type) -> Option<Operand> {
        match pat {
            Pattern::Wildcard => None,
            // An ident that names a variant is a discriminant test; any
            // other ident binds unconditionally.
            Pattern::Ident(n) => {
                let text = self.name(*n).to_owned();
                match self.res.lookup(&text) {
                    Some(Def::Variant(e, vi)) => Some(self.variant_test(scr, e, vi)),
                    Some(Def::ResultOk) => Some(self.variant_test(scr, RESULT_ITEM, 0)),
                    Some(Def::ResultErr) => Some(self.variant_test(scr, RESULT_ITEM, 1)),
                    _ => None,
                }
            }
            Pattern::Literal(l) => {
                let c = Self::lit_const(l, scr_ty);
                let eq = self.temp(Type::Bool);
                self.assign(
                    Place::local(eq),
                    Rvalue::Binary(BinOp::Eq, Operand::Place(scr.clone()), Operand::Const(c)),
                );
                Some(Operand::Place(Place::local(eq)))
            }
            Pattern::Variant { name, args } => {
                let text = self.name(*name).to_owned();
                let (vi, payload): (u32, Vec<Type>) = match self.res.lookup(&text) {
                    Some(Def::Variant(e, vi)) => (vi, enum_variant_payload(self.items, e, vi)),
                    // `Ok`/`Err` — payload type is the scrutinee's `T`/`E`.
                    Some(Def::ResultOk | Def::ResultErr) => {
                        let vi = u32::from(matches!(self.res.lookup(&text), Some(Def::ResultErr)));
                        let payload = match scr_ty {
                            Type::Result(ok, err) => {
                                vec![if vi == 0 {
                                    (**ok).clone()
                                } else {
                                    (**err).clone()
                                }]
                            }
                            _ => Vec::new(),
                        };
                        (vi, payload)
                    }
                    _ => return None, // unresolved — typeck already diagnosed
                };
                let mut cond = self.variant_test(scr, RESULT_ITEM, vi);
                // Nested patterns in the payload refine the test.
                for (i, sub) in args.iter().enumerate() {
                    let mut proj = scr.proj.clone();
                    proj.push(Proj::VariantField {
                        variant: vi,
                        field: u32::try_from(i).unwrap_or(u32::MAX),
                    });
                    let sub_place = Place {
                        local: scr.local,
                        proj,
                    };
                    let fty = payload.get(i).cloned().unwrap_or(Type::Error);
                    if let Some(sub) = self.match_test(sub, &sub_place, &fty) {
                        let both = self.temp(Type::Bool);
                        self.assign(Place::local(both), Rvalue::Binary(BinOp::And, cond, sub));
                        cond = Operand::Place(Place::local(both));
                    }
                }
                Some(cond)
            }
        }
    }

    /// `disc(scr) == variant` → a fresh bool local holding the result.
    fn variant_test(&mut self, scr: &Place, _e: u32, vi: u32) -> Operand {
        let d = self.temp(Type::Int(IntTy::I32));
        self.assign(Place::local(d), Rvalue::Discriminant(scr.clone()));
        let eq = self.temp(Type::Bool);
        self.assign(
            Place::local(eq),
            Rvalue::Binary(
                BinOp::Eq,
                Operand::Place(Place::local(d)),
                Operand::Const(Const::Int(u64::from(vi), IntTy::I32)),
            ),
        );
        Operand::Place(Place::local(eq))
    }

    /// Install `pat`'s bindings in the current scope — runs inside the
    /// arm block, after the discriminant test proved the pattern.
    fn bind_pattern(&mut self, pat: &Pattern, scr: &Place, scr_ty: &Type) {
        match pat {
            Pattern::Ident(n) => {
                // Variant names don't bind — they're tested, not bound.
                let text = self.name(*n).to_owned();
                if matches!(
                    self.res.lookup(&text),
                    Some(Def::Variant(..) | Def::ResultOk | Def::ResultErr)
                ) {
                    return;
                }
                if let Some(s) = self.scopes.last_mut() {
                    s.insert(*n, scr.local);
                }
            }
            Pattern::Variant { name, args } => {
                let text = self.name(*name).to_owned();
                let (vi, payload): (u32, Vec<Type>) = match self.res.lookup(&text) {
                    Some(Def::Variant(e, vi)) => (vi, enum_variant_payload(self.items, e, vi)),
                    Some(Def::ResultOk | Def::ResultErr) => {
                        let vi = u32::from(matches!(self.res.lookup(&text), Some(Def::ResultErr)));
                        let payload = match scr_ty {
                            Type::Result(ok, err) => {
                                vec![if vi == 0 {
                                    (**ok).clone()
                                } else {
                                    (**err).clone()
                                }]
                            }
                            _ => Vec::new(),
                        };
                        (vi, payload)
                    }
                    _ => return,
                };
                for (i, sub) in args.iter().enumerate() {
                    let mut proj = scr.proj.clone();
                    proj.push(Proj::VariantField {
                        variant: vi,
                        field: u32::try_from(i).unwrap_or(u32::MAX),
                    });
                    let sub_place = Place {
                        local: scr.local,
                        proj,
                    };
                    let ty = payload.get(i).cloned().unwrap_or(Type::Error);
                    if let Pattern::Ident(n) = sub {
                        let l = self.new_local(ty, Some(self.name(*n).to_owned()), false);
                        self.assign(Place::local(l), Rvalue::Use(Operand::Place(sub_place)));
                        if let Some(s) = self.scopes.last_mut() {
                            s.insert(*n, l);
                        }
                    } else {
                        self.bind_pattern(sub, &sub_place, &ty);
                    }
                }
            }
            Pattern::Wildcard | Pattern::Literal(_) => {}
        }
    }

    /// `Literal` → [`Const`], typing ints/floats by the scrutinee type.
    fn lit_const(l: &Literal, scr_ty: &Type) -> Const {
        match l {
            Literal::Int(v) => {
                let ity = match scr_ty {
                    Type::Int(i) => *i,
                    _ => IntTy::I64,
                };
                Const::Int(*v, ity)
            }
            Literal::Float(v) => {
                let fty = match scr_ty {
                    Type::Float(f) => *f,
                    _ => aura_semantic::FloatTy::F64,
                };
                Const::Float(*v, fty)
            }
            Literal::Bool(b) => Const::Bool(*b),
            Literal::Unit | Literal::Str(_) => Const::Unit,
        }
    }

    /// Resolve `id` to a [`Place`] (lvalue) — `Ident` or a chain of
    /// `Field` projections on one.
    fn place(&mut self, id: ExprId) -> Option<Place> {
        match self.body.ast.expr(id) {
            Expr::Ident(name) => self.lookup(*name).map(Place::local),
            Expr::Field { object, field } => {
                let mut base = self.place(*object)?;
                let obj_ty = self.expr_ty(*object).clone();
                let idx = match obj_ty {
                    Type::Struct(item) => {
                        let fname = self.name(*field);
                        match self.items.items.get(item as usize) {
                            Some(ItemSig::Struct { fields, .. }) => fields
                                .iter()
                                .position(|f| f.name == fname)
                                .map(|i| u32::try_from(i).unwrap_or(u32::MAX)),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                base.proj.push(Proj::Field(idx?));
                Some(base)
            }
            _ => {
                // Non-place expression used as a place (e.g. `f().x`) —
                // evaluate into a temp and project it.
                let op = self.expr(id);
                match op {
                    Operand::Place(p) => Some(p),
                    Operand::Const(_) => None,
                }
            }
        }
    }
}
