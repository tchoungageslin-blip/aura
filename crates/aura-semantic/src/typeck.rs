//! Bidirectional type checking over cloned per-function bodies.
//!
//! `typeck_fn(db, file, item_index)` depends on `fn_body` (that fn's
//! subtree, spans included) and `file_items`/`resolved_file` (position-free
//! signatures) — never on `parsed`. An edit that leaves an fn's subtree and
//! the file's signatures equal therefore reuses this query's memoized
//! result wholesale (salsa early cutoff).
//!
//! Diagnostics are embedded in the output as data rather than pushed
//! through an accumulator: `fn_body` carries spans, so any edit that would
//! move a diagnostic's span also changes the body output and re-runs the
//! query — accumulated output can never go stale.

use aura_ast::{BinOp, BlockId, Expr, ExprId, Literal, Pattern, Stmt, TypeExpr, TypeExprId, UnOp};
use aura_common::{Diagnostic, Span, codes};
use aura_salsa_db::{Body, Db, FileItems, ItemSig, SourceFile, TypeName, file_items, fn_body};
use indexmap::IndexMap;
use lasso::Spur;

use crate::infer::{InferCtx, VarKind};
use crate::resolve::{Def, Resolution, resolved_file};
use crate::ty::{self, Type};

/// Result of checking one function body.
#[derive(Debug, Clone, PartialEq)]
pub struct FnTypes {
    /// Resolved type of every expression in `body.ast.exprs` — index is
    /// the `ExprId` index. Inference vars are always resolved.
    pub exprs: Vec<Type>,
    /// The function's declared return type.
    pub ret: Type,
    /// Diagnostics produced while checking this body. Spans are accurate
    /// for the revision that produced them (see module docs).
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
struct Local {
    mutable: bool,
    ty: Type,
}

struct Checker<'a> {
    body: &'a Body,
    items: &'a FileItems,
    res: &'a Resolution,
    infer: InferCtx,
    /// Lexical scope stack — innermost last.
    scopes: Vec<IndexMap<Spur, Local>>,
    /// Per-expr resolved types (indexed by `ExprId.0`).
    expr_types: Vec<Type>,
    diags: Vec<Diagnostic>,
    /// Expected return type for `return` statements.
    expected_ret: Type,
}

/// Type-check `parsed.items[index]`'s body. `None` for non-fn items or
/// bodiless (extern) fns.
#[salsa::tracked(returns(ref))]
pub fn typeck_fn(db: &dyn Db, file: SourceFile, index: u32) -> Option<FnTypes> {
    let items = file_items(db, file);
    let ItemSig::Fn {
        params,
        ret,
        is_extern,
        ..
    } = items.items.get(index as usize)?
    else {
        return None;
    };
    if *is_extern {
        return None;
    }
    let body = fn_body(db, file, index).as_ref()?;
    let res = resolved_file(db, file);
    let expected_ret = ret
        .as_ref()
        .map_or(Type::Unit, |t| lower_typename(items, t));
    let mut ck = Checker {
        body,
        items,
        res,
        infer: InferCtx::new(),
        scopes: vec![IndexMap::new()],
        expr_types: vec![Type::Error; body.ast.exprs.len()],
        diags: Vec::new(),
        expected_ret,
    };
    for (i, p) in params.iter().enumerate() {
        let spur = body.params.get(i).copied();
        ck.bind_param(
            spur,
            Local {
                mutable: false,
                ty: lower_typename(items, &p.ty),
            },
        );
    }
    let body_ty = ck.block(body.root);
    // The body's value must match the declared return type.
    let expected_ret = ck.expected_ret.clone();
    let span = ck.body.ast.block(body.root).tail.map_or_else(
        || ck.body.ast.block_span(body.root),
        |t| ck.body.ast.expr_span(t),
    );
    ck.unify_code(
        &expected_ret,
        &body_ty,
        span,
        codes::SEM_RETURN_TYPE,
        "return type mismatch",
    );

    // Finalize: resolve + default every recorded expr type. Unconstrained
    // `Any` vars (e.g. `let x = f()`) get one E2111 each.
    let mut reported: Vec<u32> = Vec::new();
    for i in 0..ck.expr_types.len() {
        let mut unbound = Vec::new();
        let ty = ck.infer.finalize(&ck.expr_types[i].clone(), &mut unbound);
        ck.expr_types[i] = ty;
        for v in unbound {
            if !reported.contains(&v) {
                reported.push(v);
                ck.err(
                    codes::SEM_CANNOT_INFER,
                    "cannot infer type of expression",
                    ck.body
                        .ast
                        .expr_span(aura_ast::ExprId(u32::try_from(i).unwrap_or(u32::MAX))),
                );
            }
        }
    }

    Some(FnTypes {
        exprs: ck.expr_types,
        ret: ck.expected_ret,
        diagnostics: ck.diags,
    })
}

// ----- checking ---------------------------------------------------------------

impl Checker<'_> {
    fn bind_param(&mut self, spur: Option<Spur>, local: Local) {
        if let Some(s) = spur {
            self.scopes.last_mut().unwrap().insert(s, local);
        }
    }

    fn bind(&mut self, spur: Spur, local: Local) {
        self.scopes.last_mut().unwrap().insert(spur, local);
    }

    fn lookup(&self, spur: Spur) -> Option<&Local> {
        self.scopes.iter().rev().find_map(|s| s.get(&spur))
    }

    fn push_scope(&mut self) {
        self.scopes.push(IndexMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn span(&self, id: ExprId) -> Span {
        self.body.ast.expr_span(id)
    }

    fn err(&mut self, code: &'static str, msg: impl Into<String>, span: Span) {
        self.diags.push(Diagnostic::error(code, msg, span));
    }

    fn name(&self, spur: Spur) -> &str {
        self.body.name(spur)
    }

    /// Record `id`'s type and return it. Does NOT default inference vars —
    /// later unifications must still be able to bind them (e.g. `42` first
    /// seen as `Var(Int)` may become `i32` via `x + 7i32`). Unbound vars
    /// are defaulted in the finalization pass at the end of `typeck_fn`.
    fn set(&mut self, id: ExprId, ty: &Type) -> Type {
        let ty = self.infer.resolve(ty);
        if let Some(slot) = self.expr_types.get_mut(id.0 as usize) {
            *slot = ty.clone();
        }
        ty
    }

    fn unify_at(&mut self, expected: &Type, found: &Type, span: Span) {
        self.unify_code(
            expected,
            found,
            span,
            codes::SEM_TYPE_MISMATCH,
            "type mismatch",
        );
    }

    /// Bidirectional `check` — infer, then unify with `expected`.
    fn check(&mut self, id: ExprId, expected: &Type) -> Type {
        let found = self.expr(id);
        self.unify_at(expected, &found, self.span(id));
        found
    }

    /// Display a type for diagnostics — unresolved vars render by family
    /// (`{integer}`/`{float}`/`{unknown}`), matching Rust's convention
    /// rather than leaking `?v0`-style internals.
    fn ty_display(&mut self, ty: &Type) -> String {
        match self.infer.resolve(ty) {
            Type::Var(v) => match self.infer.var_kind(v) {
                VarKind::Int => "{integer}".into(),
                VarKind::Float => "{float}".into(),
                VarKind::Any => "{unknown}".into(),
            },
            t => t.display(self.items),
        }
    }

    /// Unify with a specific error code (E2107 return-type, E2105
    /// condition, …) instead of the generic E2100.
    fn unify_code(
        &mut self,
        expected: &Type,
        found: &Type,
        span: Span,
        code: &'static str,
        what: &str,
    ) {
        if let Err(e) = self.infer.unify(expected, found) {
            let expected = self.ty_display(&e.expected);
            let found = self.ty_display(&e.found);
            self.err(
                code,
                format!("{what}: expected `{expected}`, found `{found}`"),
                span,
            );
        }
    }

    /// `cond` positions require `bool` — distinct error code E2105.
    fn check_bool(&mut self, id: ExprId) {
        let found = self.expr(id);
        self.unify_code(
            &Type::Bool,
            &found,
            self.span(id),
            codes::SEM_NOT_BOOL_CONDITION,
            "condition must be `bool`",
        );
    }

    /// Infer the type of expression `id`.
    fn expr(&mut self, id: ExprId) -> Type {
        let ty = match self.body.ast.expr(id) {
            Expr::Error => Type::Error,
            Expr::Literal(l) => match l {
                Literal::Int(_) => self.infer.new_var(VarKind::Int),
                Literal::Float(_) => self.infer.new_var(VarKind::Float),
                Literal::Bool(_) => Type::Bool,
                Literal::Unit => Type::Unit,
                Literal::Str(_) => Type::Str,
            },
            Expr::Ident(name) => self.ident(*name, id),
            Expr::Paren(inner) => self.expr(*inner),
            Expr::Unary { op, operand } => self.unary(*op, *operand),
            Expr::Binary { op, lhs, rhs } => self.binary(*op, *lhs, *rhs, id),
            Expr::Assign { target, value } => self.assign(*target, *value),
            Expr::Call { callee, args } => self.call(*callee, args, id),
            Expr::Field { object, field } => self.field(*object, *field, id),
            Expr::If {
                cond,
                then_block,
                else_branch,
            } => self.if_expr(*cond, *then_block, *else_branch, id),
            Expr::Block(b) | Expr::Unsafe(b) => self.block(*b),
            Expr::Match { scrutinee, arms } => self.match_expr(*scrutinee, arms, id),
            Expr::StructLit { name, fields } => self.struct_lit(*name, fields, id),
            // `e?` — deferred until `Result` generics land; passes the
            // operand's type through unchecked.
            Expr::Try { expr } => self.expr(*expr),
        };
        self.set(id, &ty)
    }

    fn ident(&mut self, name: Spur, id: ExprId) -> Type {
        if let Some(l) = self.lookup(name) {
            return l.ty.clone();
        }
        match self.res.lookup(self.name(name)) {
            Some(Def::Fn(idx)) => fn_type(self.items, idx),
            Some(Def::ExternFn(block, f)) => extern_fn_type(self.items, block, f),
            Some(Def::Variant(e, v)) => {
                let payload = enum_variant_payload(self.items, e, v);
                if payload.is_empty() {
                    // Unit-like variant used bare: `Empty`.
                    Type::Enum(e)
                } else {
                    // `Circle(f64)` — a constructor `fn(payload..) -> Enum`.
                    Type::Fn {
                        params: payload,
                        ret: Box::new(Type::Enum(e)),
                    }
                }
            }
            Some(Def::Struct(_) | Def::Enum(_)) => {
                self.err(
                    codes::SEM_UNDECLARED_VAR,
                    format!("`{}` names a type, not a value", self.name(name)),
                    self.span(id),
                );
                Type::Error
            }
            None => {
                self.err(
                    codes::SEM_UNDECLARED_VAR,
                    format!("undeclared name `{}`", self.name(name)),
                    self.span(id),
                );
                Type::Error
            }
        }
    }

    fn unary(&mut self, op: UnOp, operand: ExprId) -> Type {
        let t = self.expr(operand);
        match op {
            UnOp::Neg => {
                if !self.is_numeric(&t) {
                    self.err(
                        codes::SEM_TYPE_MISMATCH,
                        format!("cannot negate `{}`", t.display(self.items)),
                        self.span(operand),
                    );
                    return Type::Error;
                }
                t
            }
            UnOp::Not => {
                // `!` on bool (logical not) or integer (bitwise not).
                let t = self.infer.resolve(&t);
                if matches!(t, Type::Bool | Type::Int(_) | Type::Var(_) | Type::Error) {
                    t
                } else {
                    self.err(
                        codes::SEM_TYPE_MISMATCH,
                        format!("cannot apply `!` to `{}`", t.display(self.items)),
                        self.span(operand),
                    );
                    Type::Error
                }
            }
        }
    }

    fn binary(&mut self, op: BinOp, lhs: ExprId, rhs: ExprId, id: ExprId) -> Type {
        match op {
            BinOp::And | BinOp::Or => {
                self.check(lhs, &Type::Bool);
                self.check(rhs, &Type::Bool);
                Type::Bool
            }
            BinOp::Eq | BinOp::Ne => {
                let l = self.expr(lhs);
                self.check(rhs, &l);
                Type::Bool
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let l = self.expr(lhs);
                self.check(rhs, &l);
                let l = self.infer.resolve(&l);
                if !(self.is_numeric(&l) || matches!(l, Type::Str | Type::Var(_) | Type::Error)) {
                    self.err(
                        codes::SEM_TYPE_MISMATCH,
                        format!("cannot compare `{}`", l.display(self.items)),
                        self.span(id),
                    );
                }
                Type::Bool
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                let l = self.expr(lhs);
                let l = self.infer.resolve(&l);
                if self.is_numeric(&l) {
                    self.check(rhs, &l);
                    l
                } else {
                    // Non-numeric lhs poisons the op — still visit rhs for
                    // its own diagnostics, but skip the operand unification
                    // to avoid a second cascading error.
                    self.expr(rhs);
                    if !matches!(l, Type::Error) {
                        self.err(
                            codes::SEM_TYPE_MISMATCH,
                            format!("cannot apply arithmetic to `{}`", l.display(self.items)),
                            self.span(id),
                        );
                    }
                    Type::Error
                }
            }
        }
    }

    fn is_numeric(&mut self, t: &Type) -> bool {
        matches!(
            self.infer.resolve(t),
            Type::Int(_) | Type::Float(_) | Type::Var(_) | Type::Error
        )
    }

    fn assign(&mut self, target: ExprId, value: ExprId) -> Type {
        // Lvalue + mutability check on plain identifiers.
        if let Expr::Ident(name) = self.body.ast.expr(target) {
            match self.lookup(*name) {
                Some(l) if !l.mutable => self.err(
                    codes::SEM_MUTATE_IMMUTABLE,
                    format!("cannot assign to immutable `{}`", self.name(*name)),
                    self.span(target),
                ),
                Some(_) => {}
                None => {
                    // Not a local — assigning to an item name is an error.
                    self.err(
                        codes::SEM_UNDECLARED_VAR,
                        format!("cannot assign to `{}`", self.name(*name)),
                        self.span(target),
                    );
                }
            }
        }
        let lt = self.expr(target);
        self.check(value, &lt);
        Type::Unit
    }

    fn call(&mut self, callee: ExprId, args: &[ExprId], id: ExprId) -> Type {
        // Variant constructors: `Circle(1.0)` resolves the callee ident to
        // `fn(payload..) -> Enum`.
        let callee_ty = self.expr(callee);
        let Type::Fn { params, ret } = self.infer.resolve(&callee_ty) else {
            if !matches!(self.infer.resolve(&callee_ty), Type::Error) {
                self.err(
                    codes::SEM_NOT_CALLABLE,
                    format!("`{}` is not callable", callee_ty.display(self.items)),
                    self.span(callee),
                );
            }
            for &a in args {
                self.expr(a);
            }
            return Type::Error;
        };
        if params.len() != args.len() {
            self.err(
                codes::SEM_ARG_COUNT,
                format!(
                    "expected {} argument(s), found {}",
                    params.len(),
                    args.len()
                ),
                self.span(id),
            );
        }
        for (i, &a) in args.iter().enumerate() {
            if let Some(p) = params.get(i) {
                self.check(a, p);
            } else {
                self.expr(a);
            }
        }
        *ret
    }

    fn field(&mut self, object: ExprId, field: Spur, id: ExprId) -> Type {
        let raw = self.expr(object);
        let obj = self.infer.resolve(&raw);
        let Type::Struct(idx) = obj else {
            if !matches!(obj, Type::Error) {
                self.err(
                    codes::SEM_NO_FIELD,
                    format!("type `{}` has no fields", obj.display(self.items)),
                    self.span(id),
                );
            }
            return Type::Error;
        };
        let fname = self.name(field);
        if let Some(f) = self
            .items
            .items
            .get(idx as usize)
            .and_then(|s| match s {
                ItemSig::Struct { fields, .. } => Some(fields),
                _ => None,
            })
            .and_then(|fields| fields.iter().find(|f| f.name == fname))
        {
            return lower_typename(self.items, &f.ty);
        }
        self.err(
            codes::SEM_NO_FIELD,
            format!("no field `{fname}` on `{}`", obj.display(self.items)),
            self.span(id),
        );
        Type::Error
    }

    fn if_expr(
        &mut self,
        cond: ExprId,
        then_block: BlockId,
        else_branch: Option<ExprId>,
        id: ExprId,
    ) -> Type {
        self.check(cond, &Type::Bool);
        self.check_bool(cond);
        let then_ty = self.block(then_block);
        match else_branch {
            Some(e) => {
                let else_ty = self.expr(e);
                // Unify arms — `if` used as a value needs both sides.
                self.unify_at(&then_ty, &else_ty, self.span(id));
                then_ty
            }
            None => {
                // Value-less `if` is fine only in statement position; the
                // block's own tail must be unit-shaped — enforce by
                // unifying with Unit only when the arm has a tail value.
                if self.body.ast.block(then_block).tail.is_some() {
                    self.err(
                        codes::SEM_IF_MISSING_ELSE,
                        "`if` with a value requires an `else` branch",
                        self.span(id),
                    );
                    Type::Error
                } else {
                    Type::Unit
                }
            }
        }
    }

    fn block(&mut self, b: BlockId) -> Type {
        self.push_scope();
        let block = self.body.ast.block(b).clone();
        let mut diverges = false;
        for &s in &block.stmts {
            self.stmt(s);
            // `return`/`break`/`continue` never fall through — a block
            // ending in one diverges (`Never`), so `if c { return 1 }
            // else { return 0 }` satisfies any expected return type.
            diverges = matches!(
                self.body.ast.stmt(s),
                Stmt::Return(_) | Stmt::Break | Stmt::Continue
            );
        }
        let ty = match block.tail {
            Some(t) => self.expr(t),
            None if diverges => Type::Never,
            None => Type::Unit,
        };
        self.pop_scope();
        ty
    }

    fn stmt(&mut self, s: aura_ast::StmtId) {
        match self.body.ast.stmt(s) {
            Stmt::Let {
                name,
                mutable,
                ty,
                init,
            } => {
                let t = match ty {
                    Some(ty_id) => {
                        let ann = self.lower_ast_ty(*ty_id);
                        self.check(*init, &ann.clone());
                        ann
                    }
                    None => self.expr(*init),
                };
                let t = self.infer.resolve(&t);
                let (name, mutable) = (*name, *mutable);
                self.bind(name, Local { mutable, ty: t });
            }
            Stmt::Expr(e) => {
                self.stmt_expr(*e);
            }
            Stmt::Return(e) => {
                let expected = self.expected_ret.clone();
                match e {
                    Some(e) => {
                        let found = self.expr(*e);
                        self.unify_code(
                            &expected,
                            &found,
                            self.span(*e),
                            codes::SEM_RETURN_TYPE,
                            "return type mismatch",
                        );
                    }
                    None => {
                        self.unify_code(
                            &Type::Unit,
                            &expected,
                            self.body.ast.stmt_span(s),
                            codes::SEM_RETURN_TYPE,
                            "return type mismatch",
                        );
                    }
                }
            }
            Stmt::While { cond, body } => {
                self.check_bool(*cond);
                self.block(*body);
            }
            Stmt::Loop { body } => {
                self.block(*body);
            }
            Stmt::Break | Stmt::Continue | Stmt::Error => {}
        }
    }

    /// Expression in statement position. A bare `if` (no `else`) needs no
    /// else-branch; with `else`, arms must still agree (Rust parity:
    /// `if c {1} else {"s"};` is an error there too).
    fn stmt_expr(&mut self, e: ExprId) {
        let bare_if = matches!(
            self.body.ast.expr(e),
            Expr::If {
                else_branch: None,
                ..
            }
        );
        if bare_if {
            let Expr::If {
                cond, then_block, ..
            } = self.body.ast.expr(e)
            else {
                return;
            };
            let (cond, then_block) = (*cond, *then_block);
            self.check_bool(cond);
            self.block(then_block);
            self.set(e, &Type::Unit);
        } else {
            self.expr(e);
        }
    }

    fn match_expr(&mut self, scrutinee: ExprId, arms: &[aura_ast::MatchArm], id: ExprId) -> Type {
        let scrut = self.expr(scrutinee);
        let scrut_resolved = self.infer.resolve(&scrut);
        let mut result: Option<Type> = None;
        // Exhaustiveness bookkeeping for enum scrutinees.
        let mut covered: Vec<String> = Vec::new();
        let mut has_wildcard = false;
        for arm in arms {
            self.push_scope();
            self.check_pattern(
                &arm.pattern.clone(),
                &scrut.clone(),
                arm.span,
                &mut covered,
                &mut has_wildcard,
            );
            let arm_ty = self.expr(arm.body);
            self.pop_scope();
            result = Some(match result {
                None => arm_ty,
                Some(prev) => {
                    self.unify_at(&prev, &arm_ty, arm.span);
                    prev
                }
            });
        }
        // Exhaustiveness: enum scrutinee needs every variant or a wildcard.
        if !has_wildcard {
            match scrut_resolved {
                Type::Enum(idx) => {
                    if let Some(ItemSig::Enum { variants, .. }) = self.items.items.get(idx as usize)
                    {
                        let missing: Vec<&str> = variants
                            .iter()
                            .map(|(n, _)| n.as_str())
                            .filter(|n| !covered.iter().any(|c| c == n))
                            .collect();
                        if !missing.is_empty() {
                            self.err(
                                codes::SEM_NON_EXHAUSTIVE_MATCH,
                                format!("non-exhaustive match: missing {}", missing.join(", ")),
                                self.span(id),
                            );
                        }
                    }
                }
                Type::Bool => {
                    let need = ["true", "false"];
                    if !need.iter().all(|n| covered.iter().any(|c| c == n)) {
                        self.err(
                            codes::SEM_NON_EXHAUSTIVE_MATCH,
                            "non-exhaustive match: missing `true` or `false` arm",
                            self.span(id),
                        );
                    }
                }
                // Infinite domains need a wildcard.
                Type::Int(_) | Type::Str => self.err(
                    codes::SEM_NON_EXHAUSTIVE_MATCH,
                    "non-exhaustive match: add a `_` arm",
                    self.span(id),
                ),
                _ => {}
            }
        }
        result.unwrap_or(Type::Error)
    }

    /// Check `pattern` against `scrut` type; binds identifiers into the
    /// current scope. `arm_span` backs pattern diagnostics — `Pattern`
    /// nodes carry no spans of their own in the AST.
    fn check_pattern(
        &mut self,
        p: &Pattern,
        scrut: &Type,
        arm_span: Span,
        covered: &mut Vec<String>,
        has_wildcard: &mut bool,
    ) {
        match p {
            Pattern::Wildcard => *has_wildcard = true,
            Pattern::Ident(name) => {
                // A name matching a known variant is a variant pattern;
                // anything else binds the scrutinee (catch-all).
                let text = self.name(*name).to_owned();
                if let Some(Def::Variant(e, _)) = self.res.lookup(&text) {
                    covered.push(text);
                    self.unify_at(scrut, &Type::Enum(e), arm_span);
                } else {
                    *has_wildcard = true;
                    self.bind(
                        *name,
                        Local {
                            mutable: false,
                            ty: scrut.clone(),
                        },
                    );
                }
            }
            Pattern::Literal(l) => {
                let lit_ty = match l {
                    Literal::Int(_) => self.infer.new_var(VarKind::Int),
                    Literal::Float(_) => self.infer.new_var(VarKind::Float),
                    Literal::Bool(b) => {
                        covered.push(if *b { "true".into() } else { "false".into() });
                        Type::Bool
                    }
                    Literal::Unit => Type::Unit,
                    Literal::Str(_) => Type::Str,
                };
                self.unify_at(scrut, &lit_ty, arm_span);
            }
            Pattern::Variant { name, args } => {
                let text = self.name(*name).to_owned();
                match self.res.lookup(&text) {
                    Some(Def::Variant(e, vi)) => {
                        covered.push(text.clone());
                        self.unify_at(scrut, &Type::Enum(e), arm_span);
                        // Bind positional payload args.
                        let payload = enum_variant_payload(self.items, e, vi);
                        if args.len() != payload.len() {
                            self.err(
                                codes::SEM_ARG_COUNT,
                                format!(
                                    "variant `{text}` expects {} field(s), found {}",
                                    payload.len(),
                                    args.len()
                                ),
                                arm_span,
                            );
                        }
                        for (i, arg) in args.iter().enumerate() {
                            let ty = payload.get(i).cloned().unwrap_or(Type::Error);
                            let mut sink = Vec::new();
                            let mut wc = false;
                            self.check_pattern(arg, &ty, arm_span, &mut sink, &mut wc);
                        }
                    }
                    _ => self.err(
                        codes::SEM_UNKNOWN_VARIANT,
                        format!("unknown variant `{text}`"),
                        arm_span,
                    ),
                }
            }
        }
    }

    fn struct_lit(&mut self, name: Spur, fields: &[(Spur, ExprId)], id: ExprId) -> Type {
        let sname = self.name(name).to_owned();
        let Some(Def::Struct(idx)) = self.res.lookup(&sname) else {
            self.err(
                codes::SEM_UNDECLARED_VAR,
                format!("unknown struct `{sname}`"),
                self.span(id),
            );
            for (_, e) in fields {
                self.expr(*e);
            }
            return Type::Error;
        };
        let field_sigs: Vec<(String, TypeName)> = match self.items.items.get(idx as usize) {
            Some(ItemSig::Struct { fields, .. }) => fields
                .iter()
                .map(|f| (f.name.clone(), f.ty.clone()))
                .collect(),
            _ => Vec::new(),
        };
        let mut seen: Vec<String> = Vec::new();
        for (fname, e) in fields {
            let ftext = self.name(*fname).to_owned();
            if let Some((_, tn)) = field_sigs.iter().find(|(n, _)| *n == ftext) {
                let expected = lower_typename(self.items, tn);
                self.check(*e, &expected);
                seen.push(ftext);
            } else {
                self.err(
                    codes::SEM_NO_FIELD,
                    format!("unknown field `{ftext}` on `{sname}`"),
                    self.span(*e),
                );
                self.expr(*e);
            }
        }
        let missing: Vec<&str> = field_sigs
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| !seen.iter().any(|s| s == n))
            .collect();
        if !missing.is_empty() {
            self.err(
                codes::SEM_MISSING_FIELDS,
                format!("`{sname}` is missing field(s): {}", missing.join(", ")),
                self.span(id),
            );
        }
        Type::Struct(idx)
    }

    /// Lower an annotation inside the body arena (`let x: T`, …) to `Type`.
    fn lower_ast_ty(&mut self, ty_id: TypeExprId) -> Type {
        let span = self.body.ast.ty_span(ty_id);
        match self.body.ast.ty(ty_id) {
            TypeExpr::Error => Type::Error,
            TypeExpr::Unit => Type::Unit,
            TypeExpr::Never => Type::Never,
            TypeExpr::Tuple(tys) => {
                Type::Tuple(tys.clone().iter().map(|&t| self.lower_ast_ty(t)).collect())
            }
            TypeExpr::Pointer { mutable, pointee } => {
                let (mutable, pointee) = (*mutable, *pointee);
                Type::Pointer {
                    mutable,
                    pointee: Box::new(self.lower_ast_ty(pointee)),
                }
            }
            TypeExpr::Named { name, .. } => {
                let text = self.name(*name).to_owned();
                match ty::primitive(&text) {
                    Some(t) => t,
                    None => match self.res.lookup(&text) {
                        Some(Def::Struct(i)) => Type::Struct(i),
                        Some(Def::Enum(i)) => Type::Enum(i),
                        _ => {
                            self.err(
                                codes::SEM_UNKNOWN_TYPE,
                                format!("unknown type `{text}`"),
                                span,
                            );
                            Type::Error
                        }
                    },
                }
            }
        }
    }
}

// ----- signature lowering ------------------------------------------------------

/// Lower a position-free [`TypeName`] to a semantic [`Type`]. Unknown
/// names become `Type::Error` (diagnosed once by `check_file`).
pub fn lower_typename(items: &FileItems, tn: &TypeName) -> Type {
    match tn {
        TypeName::Error => Type::Error,
        TypeName::Unit => Type::Unit,
        TypeName::Never => Type::Never,
        TypeName::Tuple(ts) => Type::Tuple(ts.iter().map(|t| lower_typename(items, t)).collect()),
        TypeName::Pointer { mutable, pointee } => Type::Pointer {
            mutable: *mutable,
            pointee: Box::new(lower_typename(items, pointee)),
        },
        TypeName::Named { name, .. } => {
            if let Some(t) = ty::primitive(name) {
                return t;
            }
            match items.find(name) {
                Some(i) => match items.items.get(i as usize) {
                    Some(ItemSig::Struct { .. }) => Type::Struct(i),
                    Some(ItemSig::Enum { .. }) => Type::Enum(i),
                    _ => Type::Error,
                },
                None => Type::Error,
            }
        }
    }
}

/// Signature of `FileItems.items[idx]` as a callable type, or `Error`.
fn fn_type(items: &FileItems, idx: u32) -> Type {
    match items.items.get(idx as usize) {
        Some(ItemSig::Fn { params, ret, .. }) => Type::Fn {
            params: params
                .iter()
                .map(|p| lower_typename(items, &p.ty))
                .collect(),
            ret: Box::new(
                ret.as_ref()
                    .map_or(Type::Unit, |t| lower_typename(items, t)),
            ),
        },
        _ => Type::Error,
    }
}

fn extern_fn_type(items: &FileItems, block: u32, f: u32) -> Type {
    let Some(ItemSig::ExternBlock { fns, .. }) = items.items.get(block as usize) else {
        return Type::Error;
    };
    let Some(sig) = fns.get(f as usize) else {
        return Type::Error;
    };
    Type::Fn {
        params: sig
            .params
            .iter()
            .map(|p| lower_typename(items, &p.ty))
            .collect(),
        ret: Box::new(
            sig.ret
                .as_ref()
                .map_or(Type::Unit, |t| lower_typename(items, t)),
        ),
    }
}

/// Positional payload types of `variants[v]` in enum item `e`.
#[must_use]
pub fn enum_variant_payload(items: &FileItems, e: u32, v: u32) -> Vec<Type> {
    match items.items.get(e as usize) {
        Some(ItemSig::Enum { variants, .. }) => variants
            .get(v as usize)
            .map(|(_, payload)| payload.iter().map(|t| lower_typename(items, t)).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

// ----- top-level query ---------------------------------------------------------

/// Whole-file check: parse (diagnostics via `parsed`), name resolution
/// (E2003 duplicates get spans here), then per-function type checking.
/// Returns all diagnostics sorted by span start — the CLI prints these.
#[salsa::tracked(returns(ref))]
pub fn check_file(db: &dyn Db, file: SourceFile) -> Vec<Diagnostic> {
    let parsed = aura_salsa_db::parsed(db, file);
    let items = file_items(db, file);
    let res = resolved_file(db, file);

    let mut diags = parsed.diagnostics.clone();

    // Redefinitions — resolution is position-free, so spans come from the
    // parse here at the always-fresh top level.
    for dup in &res.duplicates {
        let span = parsed
            .items
            .get(dup.dup as usize)
            .map_or_else(|| Span::point(parsed.file, 0), aura_ast::Item::span);
        let first_span = parsed
            .items
            .get(dup.first as usize)
            .map_or_else(|| Span::point(parsed.file, 0), aura_ast::Item::span);
        diags.push(
            Diagnostic::error(
                codes::SEM_REDEFINITION,
                format!("redefinition of `{}`", dup.name),
                span,
            )
            .with_label(first_span, "first defined here"),
        );
    }

    // Type-check every fn body.
    for (i, sig) in items.iter() {
        if !matches!(sig, ItemSig::Fn { .. }) {
            continue;
        }
        if let Some(types) = typeck_fn(db, file, i).as_ref() {
            diags.extend(types.diagnostics.iter().cloned());
        }
    }

    diags.sort_by_key(|d| d.span.map_or(0, |s| s.start));
    diags
}
