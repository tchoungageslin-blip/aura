//! Stable text dump of a [`MirBody`] — used by tests and `aura dump`.

use std::fmt::Write;

use aura_ast::{BinOp, UnOp};

use crate::{Const, MirBody, MirStmt, MirTerm, Operand, Place, Rvalue};

/// Render `body` as indented MIR text.
///
/// # Panics
/// If `param_count` exceeds the declared locals (a malformed body —
/// lowering always emits `_0` plus one local per parameter).
pub fn dump(body: &MirBody) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "fn {}(", body.name);
    for i in 0..body.param_count {
        let l = body
            .locals
            .get(1 + i as usize)
            .expect("param local missing");
        let _ = writeln!(s, "    _{}: {},", i + 1, ty(l));
    }
    let _ = writeln!(s, ") -> {} {{", ty_name(&body.ret));
    let _ = writeln!(s, "    locals:");
    for (i, l) in body.locals.iter().enumerate() {
        let name = l.name.as_deref().unwrap_or("<tmp>");
        let _ = writeln!(s, "        _{i}: {name} {}", ty(l));
    }
    for (i, b) in body.blocks.iter().enumerate() {
        let _ = writeln!(s, "    bb{i}:");
        for st in &b.stmts {
            let _ = writeln!(s, "        {}", stmt(st));
        }
        let _ = writeln!(s, "        {}", term(&b.term));
    }
    let _ = writeln!(s, "}}");
    s
}

fn ty(l: &crate::MirLocal) -> String {
    ty_name(&l.ty)
}

fn ty_name(t: &aura_semantic::Type) -> String {
    use aura_semantic::Type;
    match t {
        Type::Error => "<err>".into(),
        Type::Never => "!".into(),
        Type::Unit => "()".into(),
        Type::Bool => "bool".into(),
        Type::Int(i) => i.name().into(),
        Type::Float(f) => f.name().into(),
        Type::Str => "str".into(),
        Type::Struct(i) | Type::Enum(i) => format!("#{i}"),
        Type::Result(ok, err) => format!("Result<{}, {}>", ty_name(ok), ty_name(err)),
        Type::Fn { .. } => "fn(..)".into(),
        Type::Tuple(_) => "(..)".into(),
        Type::Pointer { .. } => "*_".into(),
        Type::Var(v) => format!("?v{v}"),
    }
}

fn stmt(s: &MirStmt) -> String {
    match s {
        MirStmt::Assign(p, rv) => format!("{} = {}", place(p), rvalue(rv)),
        MirStmt::Nop => "nop".into(),
    }
}

fn term(t: &MirTerm) -> String {
    match t {
        MirTerm::Goto(b) => format!("goto bb{b}"),
        MirTerm::Branch { cond, then, else_ } => {
            format!("br {} [bb{then}] else [bb{else_}]", operand(cond))
        }
        MirTerm::Return => "return".into(),
        MirTerm::Unreachable => "unreachable".into(),
    }
}

fn place(p: &Place) -> String {
    let mut s = format!("_{}", p.local);
    for proj in &p.proj {
        match proj {
            crate::Proj::Field(i) => {
                let _ = write!(s, ".{i}");
            }
            crate::Proj::VariantField { variant, field } => {
                let _ = write!(s, ".<v{variant}>.{field}");
            }
        }
    }
    s
}

fn operand(o: &Operand) -> String {
    match o {
        Operand::Place(p) => place(p),
        Operand::Const(c) => konst(c),
    }
}

fn konst(c: &Const) -> String {
    match c {
        Const::Int(v, t) => format!("{v}{}", t.name()),
        Const::Float(v, t) => format!("{v}{}", t.name()),
        Const::Bool(b) => format!("{b}"),
        Const::Unit => "()".into(),
    }
}

fn rvalue(r: &Rvalue) -> String {
    match r {
        Rvalue::Use(o) => operand(o),
        Rvalue::Unary(op, o) => {
            let sym = match op {
                UnOp::Neg => "-",
                UnOp::Not => "!",
            };
            format!("{sym}{}", operand(o))
        }
        Rvalue::Binary(op, l, r) => {
            let sym = match op {
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
            };
            format!("{} {sym} {}", operand(l), operand(r))
        }
        Rvalue::Call(callee, args) => {
            let f = match callee {
                crate::Callee::Fn(i) => format!("fn#{i}"),
                crate::Callee::Extern(b, f) => format!("ext#{b}.{f}"),
            };
            let a: Vec<String> = args.iter().map(operand).collect();
            format!("{f}({})", a.join(", "))
        }
        Rvalue::StructLit { item, fields } => {
            let f: Vec<String> = fields
                .iter()
                .map(|(i, o)| format!("{i}: {}", operand(o)))
                .collect();
            format!("struct#{item} {{ {} }}", f.join(", "))
        }
        Rvalue::EnumLit {
            item,
            variant,
            fields,
        } => {
            let f: Vec<String> = fields
                .iter()
                .map(|(i, o)| format!("{i}: {}", operand(o)))
                .collect();
            if *item == crate::RESULT_ITEM {
                let v = if *variant == 0 { "Ok" } else { "Err" };
                format!("Result::{v} {{ {} }}", f.join(", "))
            } else {
                format!("enum#{item}::v{variant} {{ {} }}", f.join(", "))
            }
        }
        Rvalue::Discriminant(p) => format!("disc({})", place(p)),
    }
}
