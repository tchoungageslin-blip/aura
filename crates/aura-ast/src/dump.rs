//! S-expression pretty-printer used by snapshot tests and `--emit ast`.

use lasso::{Rodeo, Spur};

use crate::{
    Ast, BinOp, BlockId, Expr, ExprId, Item, Literal, MatchArm, Pattern, Stmt, StmtId, TypeExpr,
    TypeExprId, UnOp, VariantPayload,
};

pub fn dump_items(items: &[Item], ast: &Ast, rodeo: &Rodeo) -> String {
    let mut d = Dumper {
        ast,
        rodeo,
        out: String::new(),
    };
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            d.out.push('\n');
        }
        d.item(item);
    }
    d.out
}

struct Dumper<'a> {
    ast: &'a Ast,
    rodeo: &'a Rodeo,
    out: String,
}

impl<'a> Dumper<'a> {
    /// Resolved text lives as long as the rodeo, not `self` — returning
    /// `&'a str` lets callers use it while mutating `self.out`.
    fn name(&self, spur: Spur) -> &'a str {
        self.rodeo.resolve(&spur)
    }

    fn item(&mut self, item: &Item) {
        match item {
            Item::Function(f) => {
                self.out.push_str("(fn ");
                self.out.push_str(self.name(f.name));
                self.out.push_str(" (params");
                for p in &f.params {
                    self.out.push(' ');
                    self.out.push_str(self.name(p.name));
                    self.out.push_str(": ");
                    self.ty(p.ty);
                }
                self.out.push(')');
                if let Some(ret) = f.ret {
                    self.out.push_str(" -> ");
                    self.ty(ret);
                }
                if let Some(body) = f.body {
                    self.out.push(' ');
                    self.block(body);
                } else {
                    self.out.push_str(" <bodiless>");
                }
                self.out.push(')');
            }
            Item::Struct(s) => {
                self.out.push_str("(struct ");
                self.out.push_str(self.name(s.name));
                for f in &s.fields {
                    self.out.push_str(" (");
                    self.out.push_str(self.name(f.name));
                    self.out.push_str(": ");
                    self.ty(f.ty);
                    self.out.push(')');
                }
                self.out.push(')');
            }
            Item::Enum(e) => {
                self.out.push_str("(enum ");
                self.out.push_str(self.name(e.name));
                for v in &e.variants {
                    self.out.push(' ');
                    self.out.push_str(self.name(v.name));
                    if let VariantPayload::Tuple(tys) = &v.payload {
                        self.out.push('(');
                        for (i, t) in tys.iter().enumerate() {
                            if i > 0 {
                                self.out.push_str(", ");
                            }
                            self.ty(*t);
                        }
                        self.out.push(')');
                    }
                }
                self.out.push(')');
            }
            Item::Use { path, .. } => {
                self.out.push_str("(use");
                for seg in path {
                    self.out.push(' ');
                    self.out.push_str(self.name(*seg));
                }
                self.out.push(')');
            }
            Item::ExternBlock { abi, fns, .. } => {
                self.out.push_str("(extern ");
                self.out.push_str(self.name(*abi));
                for f in fns {
                    self.out.push(' ');
                    self.item(&Item::Function(f.clone()));
                }
                self.out.push(')');
            }
            Item::Error { .. } => self.out.push_str("<error-item>"),
        }
    }

    fn block(&mut self, id: BlockId) {
        let block = self.ast.block(id);
        self.out.push_str("(block");
        for s in &block.stmts {
            self.out.push(' ');
            self.stmt(*s);
        }
        if let Some(tail) = block.tail {
            self.out.push(' ');
            self.expr(tail);
        }
        self.out.push(')');
    }

    fn stmt(&mut self, id: StmtId) {
        match self.ast.stmt(id) {
            Stmt::Let {
                name,
                mutable,
                ty,
                init,
            } => {
                self.out
                    .push_str(if *mutable { "(let-mut " } else { "(let " });
                self.out.push_str(self.name(*name));
                if let Some(t) = ty {
                    self.out.push_str(": ");
                    self.ty(*t);
                }
                self.out.push_str(" = ");
                self.expr(*init);
                self.out.push(')');
            }
            Stmt::Expr(e) => self.expr(*e),
            Stmt::Return(v) => {
                self.out.push_str("(return");
                if let Some(e) = v {
                    self.out.push(' ');
                    self.expr(*e);
                }
                self.out.push(')');
            }
            Stmt::While { cond, body } => {
                self.out.push_str("(while ");
                self.expr(*cond);
                self.out.push(' ');
                self.block(*body);
                self.out.push(')');
            }
            Stmt::Loop { body } => {
                self.out.push_str("(loop ");
                self.block(*body);
                self.out.push(')');
            }
            Stmt::Break => self.out.push_str("break"),
            Stmt::Continue => self.out.push_str("continue"),
            Stmt::Error => self.out.push_str("<error-stmt>"),
        }
    }

    fn expr(&mut self, id: ExprId) {
        match self.ast.expr(id) {
            Expr::Error => self.out.push_str("<error-expr>"),
            Expr::Literal(lit) => self.literal(lit),
            Expr::Ident(name) => self.out.push_str(self.name(*name)),
            Expr::Binary { op, lhs, rhs } => {
                self.out.push('(');
                self.out.push_str(binop_str(*op));
                self.out.push(' ');
                self.expr(*lhs);
                self.out.push(' ');
                self.expr(*rhs);
                self.out.push(')');
            }
            Expr::Unary { op, operand } => {
                self.out.push('(');
                self.out.push_str(match op {
                    UnOp::Neg => "neg",
                    UnOp::Not => "not",
                });
                self.out.push(' ');
                self.expr(*operand);
                self.out.push(')');
            }
            Expr::Assign { target, value } => {
                self.out.push_str("(= ");
                self.expr(*target);
                self.out.push(' ');
                self.expr(*value);
                self.out.push(')');
            }
            Expr::Call { callee, args } => {
                self.out.push_str("(call ");
                self.expr(*callee);
                for a in args {
                    self.out.push(' ');
                    self.expr(*a);
                }
                self.out.push(')');
            }
            Expr::Field { object, field } => {
                self.out.push_str("(. ");
                self.expr(*object);
                self.out.push(' ');
                self.out.push_str(self.name(*field));
                self.out.push(')');
            }
            Expr::If {
                cond,
                then_block,
                else_branch,
            } => {
                self.out.push_str("(if ");
                self.expr(*cond);
                self.out.push(' ');
                self.block(*then_block);
                if let Some(e) = else_branch {
                    self.out.push(' ');
                    self.expr(*e);
                }
                self.out.push(')');
            }
            Expr::Block(b) => self.block(*b),
            Expr::Match { scrutinee, arms } => {
                self.out.push_str("(match ");
                self.expr(*scrutinee);
                for arm in arms {
                    self.out.push(' ');
                    self.arm(arm);
                }
                self.out.push(')');
            }
            Expr::StructLit { name, fields } => {
                self.out.push('(');
                self.out.push_str(self.name(*name));
                for (fname, val) in fields {
                    self.out.push(' ');
                    self.out.push_str(self.name(*fname));
                    self.out.push_str(": ");
                    self.expr(*val);
                }
                self.out.push(')');
            }
            Expr::Try { expr } => {
                self.out.push_str("(try ");
                self.expr(*expr);
                self.out.push(')');
            }
            Expr::Unsafe(b) => {
                self.out.push_str("(unsafe ");
                self.block(*b);
                self.out.push(')');
            }
            Expr::Paren(inner) => {
                self.out.push_str("(paren ");
                self.expr(*inner);
                self.out.push(')');
            }
        }
    }

    fn arm(&mut self, arm: &MatchArm) {
        self.out.push_str("(arm ");
        self.pattern(&arm.pattern);
        self.out.push_str(" => ");
        self.expr(arm.body);
        self.out.push(')');
    }

    fn pattern(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Wildcard => self.out.push('_'),
            Pattern::Ident(n) => self.out.push_str(self.name(*n)),
            Pattern::Literal(l) => self.literal(l),
            Pattern::Variant { name, args } => {
                self.out.push_str(self.name(*name));
                if !args.is_empty() {
                    self.out.push('(');
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            self.out.push_str(", ");
                        }
                        self.pattern(a);
                    }
                    self.out.push(')');
                }
            }
        }
    }

    fn literal(&mut self, lit: &Literal) {
        match lit {
            Literal::Int(v) => self.out.push_str(&v.to_string()),
            Literal::Float(v) => self.out.push_str(&v.to_string()),
            Literal::Bool(v) => self.out.push_str(if *v { "true" } else { "false" }),
            Literal::Unit => self.out.push_str("()"),
            Literal::Str(s) => {
                self.out.push('"');
                self.out.push_str(self.name(*s));
                self.out.push('"');
            }
        }
    }

    fn ty(&mut self, id: TypeExprId) {
        match self.ast.ty(id) {
            TypeExpr::Error => self.out.push_str("<error-type>"),
            TypeExpr::Named { name, generic_args } => {
                self.out.push_str(self.name(*name));
                if !generic_args.is_empty() {
                    self.out.push('<');
                    for (i, t) in generic_args.iter().enumerate() {
                        if i > 0 {
                            self.out.push(',');
                        }
                        self.ty(*t);
                    }
                    self.out.push('>');
                }
            }
            TypeExpr::Pointer { mutable, pointee } => {
                self.out
                    .push_str(if *mutable { "*mut " } else { "*const " });
                self.ty(*pointee);
            }
            TypeExpr::Tuple(tys) => {
                self.out.push('(');
                for (i, t) in tys.iter().enumerate() {
                    if i > 0 {
                        self.out.push(',');
                    }
                    self.ty(*t);
                }
                self.out.push(')');
            }
            TypeExpr::Unit => self.out.push_str("()"),
            TypeExpr::Never => self.out.push('!'),
        }
    }
}

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}
