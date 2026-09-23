//! Canonical Aura formatter — the engine behind `aura fmt`.
//!
//! A deterministic pretty-printer over the AST: 4-space indent, a
//! 100-column soft width (member/param/arg lists break onto one line
//! each when they don't fit), and comments preserved by attaching each
//! one to the item/statement that follows it in the source. Comments
//! always land on their own line — a comment inside an expression is
//! moved to just after its enclosing statement but is never dropped.
//!
//! Literals are re-emitted from their source spans, so spellings like
//! `0xFF` or `1e-3` survive; everything else is canonicalized.

use aura_ast::{
    Ast, BinOp, BlockId, Expr, ExprId, Item, Literal, MatchArm, Pattern, Stmt, TypeExpr, TypeExprId,
};
use aura_common::{Diagnostic, FileId};
use lasso::Rodeo;

/// Column limit before a construct breaks onto multiple lines.
pub const WIDTH: usize = 100;

/// Format `src` into canonical Aura text.
///
/// # Errors
///
/// Returns the parser's diagnostics when the source doesn't parse
/// cleanly — the formatter never rewrites a broken file.
pub fn format_source(src: &str) -> Result<String, Vec<Diagnostic>> {
    let parsed = aura_parser::parse_file(src, FileId(0));
    if !parsed.diagnostics.is_empty() {
        return Err(parsed.diagnostics);
    }
    let mut p = Printer {
        ast: &parsed.ast,
        rodeo: &parsed.rodeo,
        src,
        comments: scan_comments(src),
        ci: 0,
        out: String::with_capacity(src.len() + src.len() / 8),
        indent: 0,
    };
    p.file(&parsed.items);
    Ok(p.out)
}

// ----- comment trivia ---------------------------------------------------------

#[derive(Debug)]
struct Comment {
    start: usize,
    end: usize,
}

/// Collect `//` and `/* */` comment spans, skipping strings and nested
/// block comments — the lexer drops trivia, so the formatter rescans.
fn scan_comments(src: &str) -> Vec<Comment> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += usize::from(b[i] == b'\\') + 1;
                }
                i += 1; // closing quote (or EOF — parse errors bail anyway)
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let start = i;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                out.push(Comment { start, end: i });
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let start = i;
                let mut depth = 1u32;
                i += 2;
                while i < b.len() && depth > 0 {
                    if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        i += 2;
                    } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        depth += 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                out.push(Comment { start, end: i });
            }
            _ => i += 1,
        }
    }
    out
}

// ----- printer ----------------------------------------------------------------

struct Printer<'a> {
    ast: &'a Ast,
    rodeo: &'a Rodeo,
    src: &'a str,
    comments: Vec<Comment>,
    /// Index of the next not-yet-emitted comment (sorted by `start`).
    ci: usize,
    out: String,
    indent: usize,
}

impl<'a> Printer<'a> {
    /// Resolve a `Spur` through the rodeo — the `&'a str` outlives the
    /// `&self` borrow, so names can feed `self.w(...)` directly.
    fn name(&self, s: lasso::Spur) -> &'a str {
        self.rodeo.resolve(&s)
    }

    /// Current output column (1-based on the first line — close enough
    /// for width decisions).
    fn col(&self) -> usize {
        match self.out.rfind('\n') {
            Some(i) => self.out.len() - i - 1,
            None => self.out.len(),
        }
    }

    fn w(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn nl(&mut self) {
        // No trailing whitespace: trim a pending indent-only tail.
        while self.out.ends_with(' ') {
            self.out.pop();
        }
        self.out.push('\n');
    }

    fn pad(&mut self) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
    }

    /// Emit comments that end before `off`, each on its own line at the
    /// current indent.
    fn flush_comments(&mut self, off: usize) {
        while self.comments.get(self.ci).is_some_and(|c| c.end <= off) {
            let c = &self.comments[self.ci];
            let text = self.src[c.start..c.end].trim_end();
            self.pad();
            self.w(text);
            self.nl();
            self.ci += 1;
        }
    }

    /// Line number (0-based) of a byte offset.
    fn line_of(&self, off: usize) -> usize {
        self.src
            .get(..off.min(self.src.len()))
            .map_or(0, |s| s.matches('\n').count())
    }

    /// End of a node's content — stmt spans can swallow terminator AND
    /// blank-line newlines (`stmt_end` skips them all), so walk back
    /// over whitespace to the last real character.
    fn content_end(&self, end: usize) -> usize {
        let b = self.src.as_bytes();
        let mut e = end.min(b.len());
        while e > 0 && b[e - 1].is_ascii_whitespace() {
            e -= 1;
        }
        e
    }

    /// Did the source leave a blank line between the content ending at
    /// `start` and the node at `end`? True when they're ≥2 lines apart.
    fn gap_blank(&self, start: usize, end: usize) -> bool {
        self.line_of(end) >= self.line_of(start) + 2
    }

    // ----- top level ------------------------------------------------------------

    fn file(&mut self, items: &[Item]) {
        let mut first = true;
        for item in items {
            let off = item.span().start as usize;
            self.flush_comments(off);
            if !first {
                self.nl(); // one blank line between items
            }
            first = false;
            self.item(item);
        }
        // Trailing file comments.
        while self.ci < self.comments.len() {
            if !first {
                self.nl();
            }
            first = false;
            self.flush_comments(usize::MAX);
        }
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    fn item(&mut self, item: &Item) {
        self.pad();
        match item {
            Item::Function(f) => {
                self.w("fn ");
                self.w(self.name(f.name));
                // Width budget for the signature: params + ` -> R` + ` {`.
                let tail = f.ret.map_or(2, |r| self.ty_text(r).len() + 4) + 2;
                self.params(&f.params, tail);
                if let Some(ret) = f.ret {
                    self.w(" -> ");
                    self.ty(ret);
                }
                self.w(" ");
                self.block(f.body.expect("non-extern fn has a body"), true);
            }
            Item::Struct(s) => {
                self.w("struct ");
                self.w(self.name(s.name));
                let fields: Vec<String> = s
                    .fields
                    .iter()
                    .map(|f| format!("{}: {}", self.name(f.name), self.ty_text(f.ty)))
                    .collect();
                self.member_block(&fields);
            }
            Item::Enum(e) => {
                self.w("enum ");
                self.w(self.name(e.name));
                let variants: Vec<String> = e
                    .variants
                    .iter()
                    .map(|v| {
                        let n = self.name(v.name);
                        match &v.payload {
                            aura_ast::VariantPayload::None => n.to_owned(),
                            aura_ast::VariantPayload::Tuple(tys) => {
                                let ts: Vec<String> =
                                    tys.iter().map(|&t| self.ty_text(t)).collect();
                                format!("{n}({})", ts.join(", "))
                            }
                        }
                    })
                    .collect();
                self.member_block(&variants);
            }
            Item::Use { path, .. } => {
                self.w("use ");
                let parts: Vec<&str> = path.iter().map(|&p| self.name(p)).collect();
                self.w(&parts.join("."));
            }
            Item::ExternBlock { abi, fns, .. } => {
                self.w("extern \"");
                self.w(self.name(*abi));
                self.w("\" {");
                self.nl();
                self.indent += 1;
                for f in fns {
                    self.pad();
                    self.w("fn ");
                    self.w(self.name(f.name));
                    let tail = f.ret.map_or(0, |r| self.ty_text(r).len() + 4);
                    self.params(&f.params, tail);
                    if let Some(ret) = f.ret {
                        self.w(" -> ");
                        self.ty(ret);
                    }
                    self.nl();
                }
                self.indent -= 1;
                self.pad();
                self.w("}");
            }
            Item::Error { .. } => self.w("<error>"),
        }
        self.nl();
    }

    /// `fn(..)` parameter list — inline, or one-per-line past the width.
    /// `extra` counts chars that follow the list on the same line
    /// (` -> R {`), so the *whole* signature fits the budget.
    fn params(&mut self, params: &[aura_ast::Param], extra: usize) {
        let parts: Vec<String> = params
            .iter()
            .map(|p| format!("{}: {}", self.name(p.name), self.ty_text(p.ty)))
            .collect();
        let inline = format!("({})", parts.join(", "));
        if self.col() + inline.len() + extra <= WIDTH || parts.len() <= 1 {
            self.w(&inline);
            return;
        }
        self.w("(");
        self.nl();
        self.indent += 1;
        for part in &parts {
            self.pad();
            self.w(part);
            self.w(",");
            self.nl();
        }
        self.indent -= 1;
        self.pad();
        self.w(")");
    }

    /// `{ m, m }` inline when it fits, else one member per line.
    fn member_block(&mut self, members: &[String]) {
        let inline = format!(" {{ {} }}", members.join(", "));
        if members.is_empty() {
            self.w(" {}");
        } else if self.col() + inline.len() <= WIDTH && members.len() <= 4 && !inline.contains('\n')
        {
            self.w(&inline);
        } else {
            self.w(" {");
            self.nl();
            self.indent += 1;
            for m in members {
                self.pad();
                self.w(m);
                self.nl();
            }
            self.indent -= 1;
            self.pad();
            self.w("}");
        }
    }

    // ----- statements / blocks --------------------------------------------------

    /// `{ stmts }` — always multiline for fn bodies (`top = true` forces
    /// it); short single-statement blocks may stay inline otherwise.
    fn block(&mut self, b: BlockId, top: bool) {
        let blk = self.ast.block(b);
        if blk.stmts.is_empty() && blk.tail.is_none() {
            self.w("{}");
            return;
        }
        if !top && let Some(inline) = self.block_inline(b) {
            self.w(&inline);
            return;
        }
        self.w("{");
        self.nl();
        self.indent += 1;
        self.stmts(b);
        // Comments between the last stmt and `}` stay inside the block.
        self.flush_comments(self.ast.block_span(b).end as usize);
        self.indent -= 1;
        self.pad();
        self.w("}");
    }

    /// Render a block inline (`{ stmt }`) if it's short enough —
    /// single statement or lone tail, no comments inside.
    fn block_inline(&self, b: BlockId) -> Option<String> {
        let blk = self.ast.block(b);
        let count = blk.stmts.len() + usize::from(blk.tail.is_some());
        if count != 1 {
            return None;
        }
        let mut probe = Printer {
            ast: self.ast,
            rodeo: self.rodeo,
            src: self.src,
            comments: Vec::new(),
            ci: 0,
            out: String::new(),
            indent: 0,
        };
        if let Some(&s) = blk.stmts.first() {
            probe.stmt(s);
        } else if let Some(t) = blk.tail {
            probe.expr(t, 0);
        }
        let one = format!("{{ {} }}", probe.out.trim_end());
        (self.col() + one.len() <= WIDTH && !probe.out.contains('\n')).then_some(one)
    }

    fn stmts(&mut self, b: BlockId) {
        let blk = self.ast.block(b);
        let mut prev_end = self.ast.block_span(b).start as usize;
        for (i, &s) in blk.stmts.iter().enumerate() {
            let off = self.ast.stmt_span(s).start as usize;
            self.flush_comments(off);
            if i > 0 && self.gap_blank(prev_end, off) {
                self.nl();
            }
            self.stmt(s);
            prev_end = self.content_end(self.ast.stmt_span(s).end as usize);
            self.nl();
        }
        if let Some(t) = blk.tail {
            let off = self.ast.expr_span(t).start as usize;
            self.flush_comments(off);
            if !blk.stmts.is_empty() && self.gap_blank(prev_end, off) {
                self.nl();
            }
            self.pad();
            self.expr(t, 0);
            self.nl();
        }
    }

    fn stmt(&mut self, s: aura_ast::StmtId) {
        self.pad();
        match self.ast.stmt(s) {
            Stmt::Let {
                name,
                mutable,
                ty,
                init,
            } => {
                self.w("let ");
                if *mutable {
                    self.w("mut ");
                }
                self.w(self.name(*name));
                if let Some(t) = ty {
                    self.w(": ");
                    self.ty(*t);
                }
                self.w(" = ");
                self.expr(*init, 0);
            }
            Stmt::Expr(e) => self.expr(*e, 0),
            Stmt::Return(e) => {
                self.w("return");
                if let Some(e) = e {
                    self.w(" ");
                    self.expr(*e, 0);
                }
            }
            Stmt::While { cond, body } => {
                self.w("while ");
                self.expr(*cond, 0);
                self.w(" ");
                self.block(*body, false);
            }
            Stmt::Loop { body } => {
                self.w("loop ");
                self.block(*body, false);
            }
            Stmt::Break => self.w("break"),
            Stmt::Continue => self.w("continue"),
            Stmt::Error => self.w("<error>"),
        }
    }

    // ----- expressions ----------------------------------------------------------

    /// Emit `id`, parenthesizing when its binding power is below `min`.
    fn expr(&mut self, id: ExprId, min: u8) {
        let own = self.prec(id);
        let parens = own < min;
        if parens {
            self.w("(");
        }
        match self.ast.expr(id) {
            Expr::Error => self.w("<error>"),
            Expr::Literal(_) | Expr::Ident(_) => {
                // Source-verbatim literal / interned ident.
                if let Expr::Ident(n) = self.ast.expr(id) {
                    self.w(self.name(*n));
                } else {
                    let sp = self.ast.expr_span(id);
                    self.w(&self.src[sp.start as usize..sp.end as usize]);
                }
            }
            Expr::Binary { op, lhs, rhs } => {
                self.expr(*lhs, own);
                self.w(" ");
                self.w(binop(*op));
                self.w(" ");
                // Right side must bind strictly tighter to stay bare.
                self.expr(*rhs, own + 1);
            }
            Expr::Unary { op, operand } => {
                self.w(match op {
                    aura_ast::UnOp::Neg => "-",
                    aura_ast::UnOp::Not => "!",
                });
                self.expr(*operand, 7);
            }
            Expr::Assign { target, value } => {
                self.expr(*target, 8);
                self.w(" = ");
                self.expr(*value, 0);
            }
            Expr::Call { callee, args } => {
                self.expr(*callee, 8);
                self.call_args(args);
            }
            Expr::Field { object, field } => {
                self.expr(*object, 8);
                self.w(".");
                self.w(self.name(*field));
            }
            Expr::If {
                cond,
                then_block,
                else_branch,
            } => {
                self.w("if ");
                self.expr(*cond, 0);
                self.w(" ");
                self.block(*then_block, false);
                if let Some(e) = else_branch {
                    self.w(" else ");
                    match self.ast.expr(*e) {
                        Expr::If { .. } => self.expr(*e, 0), // `else if`
                        _ => self.block_expr(*e),
                    }
                }
            }
            Expr::Block(b) => self.block(*b, false),
            Expr::Match { scrutinee, arms } => {
                self.w("match ");
                self.expr(*scrutinee, 0);
                self.w(" {");
                self.nl();
                self.indent += 1;
                for arm in arms {
                    self.arm(arm);
                }
                self.indent -= 1;
                self.pad();
                self.w("}");
            }
            Expr::StructLit { name, fields } => {
                self.w(self.name(*name));
                let parts: Vec<String> = fields
                    .iter()
                    .map(|(n, e)| format!("{}: {}", self.name(*n), self.expr_text(*e)))
                    .collect();
                self.member_block(&parts);
            }
            Expr::Try { expr } => {
                self.expr(*expr, 8);
                self.w("?");
            }
            Expr::Unsafe(b) => {
                self.w("unsafe ");
                self.block(*b, false);
            }
            Expr::Paren(e) => {
                self.w("(");
                self.expr(*e, 0);
                self.w(")");
            }
        }
        if parens {
            self.w(")");
        }
    }

    /// Expression-position block operand (`else { }`).
    fn block_expr(&mut self, e: ExprId) {
        if let Expr::Block(b) = self.ast.expr(e) {
            self.block(*b, false);
        } else {
            self.expr(e, 0);
        }
    }

    /// `(a, b, c)` — inline, or one arg per line past the width.
    fn call_args(&mut self, args: &[ExprId]) {
        let parts: Vec<String> = args.iter().map(|&a| self.expr_text(a)).collect();
        let inline = format!("({})", parts.join(", "));
        if self.col() + inline.len() <= WIDTH || parts.len() <= 1 {
            self.w(&inline);
            return;
        }
        self.w("(");
        self.nl();
        self.indent += 1;
        for part in &parts {
            self.pad();
            self.w(part);
            self.w(",");
            self.nl();
        }
        self.indent -= 1;
        self.pad();
        self.w(")");
    }

    fn arm(&mut self, arm: &MatchArm) {
        self.flush_comments(arm.span.start as usize);
        self.pad();
        self.pattern(&arm.pattern);
        self.w(" => ");
        // Short bodies stay on the arm line; block bodies print braces.
        self.expr(arm.body, 0);
        self.w(",");
        self.nl();
    }

    fn pattern(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Wildcard => self.w("_"),
            Pattern::Ident(n) => self.w(self.name(*n)),
            Pattern::Literal(l) => self.literal(*l),
            Pattern::Variant { name, args } => {
                self.w(self.name(*name));
                if !args.is_empty() {
                    self.w("(");
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            self.w(", ");
                        }
                        self.pattern(a);
                    }
                    self.w(")");
                }
            }
        }
    }

    /// Literal rendered from the AST value (patterns carry no span, so
    /// ints print decimal and strings are re-escaped).
    fn literal(&mut self, l: Literal) {
        match l {
            Literal::Int(v) => self.w(&v.to_string()),
            Literal::Float(v) => self.w(&v.to_string()),
            Literal::Bool(b) => self.w(if b { "true" } else { "false" }),
            Literal::Unit => self.w("()"),
            Literal::Str(s) => {
                self.w("\"");
                for c in self.name(s).chars() {
                    match c {
                        '"' => self.w("\\\""),
                        '\\' => self.w("\\\\"),
                        '\n' => self.w("\\n"),
                        '\t' => self.w("\\t"),
                        '\r' => self.w("\\r"),
                        c => {
                            let mut buf = [0u8; 4];
                            self.w(c.encode_utf8(&mut buf));
                        }
                    }
                }
                self.w("\"");
            }
        }
    }

    /// Binding power of an expr node — mirrors the parser's table.
    fn prec(&self, id: ExprId) -> u8 {
        match self.ast.expr(id) {
            Expr::Assign { .. } => 1,
            Expr::Binary { op, .. } => match op {
                BinOp::Or => 2,
                BinOp::And => 3,
                BinOp::Eq | BinOp::Ne => 4,
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 5,
                BinOp::Add | BinOp::Sub => 6,
                BinOp::Mul | BinOp::Div | BinOp::Rem => 7,
            },
            Expr::Unary { .. } => 7,
            Expr::Call { .. } | Expr::Field { .. } | Expr::Try { .. } => 8,
            _ => 9,
        }
    }

    /// Render an expr to a scratch string for width measurements.
    fn expr_text(&self, e: ExprId) -> String {
        let mut probe = Printer {
            ast: self.ast,
            rodeo: self.rodeo,
            src: self.src,
            comments: Vec::new(),
            ci: 0,
            out: String::new(),
            indent: 0,
        };
        probe.expr(e, 0);
        probe.out
    }

    // ----- types ----------------------------------------------------------------

    fn ty(&mut self, id: TypeExprId) {
        match self.ast.ty(id) {
            TypeExpr::Error => self.w("<error>"),
            TypeExpr::Named { name, generic_args } => {
                self.w(self.name(*name));
                if !generic_args.is_empty() {
                    self.w("<");
                    for (i, &a) in generic_args.iter().enumerate() {
                        if i > 0 {
                            self.w(", ");
                        }
                        self.ty(a);
                    }
                    self.w(">");
                }
            }
            TypeExpr::Pointer { mutable, pointee } => {
                self.w(if *mutable { "*mut " } else { "*const " });
                self.ty(*pointee);
            }
            TypeExpr::Tuple(tys) => {
                self.w("(");
                for (i, &t) in tys.iter().enumerate() {
                    if i > 0 {
                        self.w(", ");
                    }
                    self.ty(t);
                }
                self.w(")");
            }
            TypeExpr::Unit => self.w("()"),
            TypeExpr::Never => self.w("!"),
        }
    }

    /// Render a type to a scratch string for width measurements.
    fn ty_text(&self, t: TypeExprId) -> String {
        let mut probe = Printer {
            ast: self.ast,
            rodeo: self.rodeo,
            src: self.src,
            comments: Vec::new(),
            ci: 0,
            out: String::new(),
            indent: 0,
        };
        probe.ty(t);
        probe.out
    }
}

fn binop(op: BinOp) -> &'static str {
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
