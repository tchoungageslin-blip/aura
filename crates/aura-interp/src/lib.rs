//! `aura-interp` — reference tree-walk interpreter for Aura.
//!
//! Purpose: differential testing. [`run_source`] parses, semantically checks,
//! then interprets `main()` — its result is the ground truth that compiled
//! executables are compared against in fuzz/tests. It is deliberately simple
//! (clone-heavy, no GC): correctness over speed.
//!
//! Supported: ints, floats, bools, `str` literals, unit; `fn` calls;
//! structs, enums + `match`, `Result`/`?`; `if`/`while`/`loop`/`break`/
//! `continue`/`return`; `unsafe` blocks (interpreted normally); `extern`
//! calls dispatch to a small builtin table (`sqrt`).
//!
//! Unsupported (→ [`InterpError::Unsupported`]): raw pointers, extern fns with
//! no builtin, `use` items (no module loader yet).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use aura_ast::{BinOp, Block, Expr, ExprId, Item, Literal, Pattern, Stmt, StmtId, UnOp};
use aura_common::{Diagnostic, SourceCache};
use lasso::Spur;

/// Runtime value. `Struct`/`Variant` payloads are positional.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i128),
    Float(f64),
    Bool(bool),
    /// `Rc` shares the backing buffer across clones — `s.ptr` is stable
    /// for the same value, matching the compiled `{ptr, len}` repr.
    /// Bytes, not `str`: `str_slice` may split a UTF-8 boundary and
    /// `read_file` reads arbitrary bytes — the compiled repr is bytes.
    Str(Rc<[u8]>),
    /// `vec<T>` — `Rc<RefCell>` matches the compiled `{ptr, len, cap}`
    /// buffer: `vec_push`/`vec_set` mutate through shared references.
    Vec(Rc<RefCell<Vec<Value>>>),
    Unit,
    Struct {
        name: String,
        fields: Vec<Value>,
    },
    Variant {
        enum_name: String,
        variant: String,
        fields: Vec<Value>,
    },
}

impl Value {
    fn truthy(&self) -> Result<bool, InterpError> {
        match self {
            Value::Bool(b) => Ok(*b),
            v => Err(InterpError::Type(format!("expected bool, found {v:?}"))),
        }
    }
}

/// A runtime fault inside interpreted code, or a construct the interpreter
/// does not model.
#[derive(Debug, Clone, PartialEq)]
pub enum InterpError {
    /// Integer division/remainder by zero — mirrors the codegen trap.
    DivByZero,
    /// `main` missing or not a `fn main() -> i64`.
    NoMain,
    /// Expression produced a value of the wrong kind (post-check this means
    /// an interpreter bug or an unchecked path).
    Type(String),
    /// Feature exists in the language but not in the interpreter.
    Unsupported(&'static str),
    /// Fuel exhausted — runaway `loop`/`while` protection for fuzzing.
    StepLimit,
    /// A name did not resolve (should be impossible after `check_file`).
    Unresolved(String),
    /// Internal channel: `return`/`break`/`continue`/`?`-on-`Err` surfacing
    /// out of an expression-context block. `block()` converts these back to
    /// [`Flow`]; `call_fn` converts `Return` into the fn's result.
    #[doc(hidden)]
    Escape(Escape),
}

impl fmt::Display for InterpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InterpError::DivByZero => write!(f, "division by zero"),
            InterpError::NoMain => write!(f, "no `fn main() -> i64`"),
            InterpError::Type(m) => write!(f, "type error: {m}"),
            InterpError::Unsupported(w) => write!(f, "unsupported: {w}"),
            InterpError::StepLimit => write!(f, "step limit exceeded"),
            InterpError::Unresolved(n) => write!(f, "unresolved name `{n}`"),
            InterpError::Escape(_) => write!(f, "uncaught control flow"),
        }
    }
}

impl std::error::Error for InterpError {}

/// Failure before or during interpretation.
#[derive(Debug)]
pub enum RunError {
    /// Frontend diagnostics (parse/resolve/typeck).
    Diagnostics(Vec<Diagnostic>),
    /// Runtime fault.
    Interp(InterpError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Diagnostics(d) => write!(f, "{} diagnostic(s)", d.len()),
            RunError::Interp(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RunError {}

/// Parse, check, and interpret `src`'s `main`. Returns `main`'s `i64`.
///
/// # Errors
///
/// - [`RunError::Diagnostics`] when `check_file` reports errors.
/// - [`RunError::Interp`] on runtime faults or unsupported constructs.
pub fn run_source(src: &str) -> Result<i64, RunError> {
    let mut cache = SourceCache::new();
    let file_id = cache.add("<interp>".to_owned(), src.to_owned());
    let parsed = aura_parser::parse_file(src, file_id);

    // Semantic check via the salsa pipeline (same as `aura check`).
    let db = aura_salsa_db::AuraDatabase::new();
    let file = aura_salsa_db::SourceFile::new(&db, src.to_owned(), file_id);
    let diags = aura_semantic::check_file(&db, file);
    if diags.iter().any(Diagnostic::is_error) {
        return Err(RunError::Diagnostics(diags.clone()));
    }

    run_parsed(&parsed).map_err(RunError::Interp)
}

/// Interpret `parsed`'s `main` (caller has already checked semantics).
/// Useful when the caller wants the [`aura_parser::ParsedFile`] for other
/// purposes.
///
/// # Errors
/// See [`run_source`].
pub fn run_parsed(parsed: &aura_parser::ParsedFile) -> Result<i64, InterpError> {
    run_files(&[parsed])
}

/// Like [`run_parsed`] but returns everything the program wrote to
/// stdout/stderr instead of forwarding it — for `aura test` and tools.
/// `cfg` injects the process environment (`read_stdin` input, working
/// directory for relative file paths, `args()` argv); defaults run on
/// the real process environment.
pub fn run_parsed_capture(parsed: &aura_parser::ParsedFile, cfg: RunConfig) -> Captured {
    run_files_capture(&[parsed], cfg)
}

/// Interpret `main` across a project's files (flat namespace — dep items
/// are visible everywhere). `files` matches `Project::files` order; the
/// entry file is last but `main` may resolve from any file.
///
/// # Errors
/// See [`run_source`].
pub fn run_project(
    db: &dyn aura_salsa_db::Db,
    project: aura_salsa_db::Project,
) -> Result<i64, InterpError> {
    let files: Vec<&aura_parser::ParsedFile> = project
        .files(db)
        .iter()
        .map(|f| aura_salsa_db::parsed(db, *f))
        .collect();
    run_files(&files)
}

/// Like [`run_project`] but captures stdout/stderr — see
/// [`run_parsed_capture`] for `cfg`.
pub fn run_project_capture(
    db: &dyn aura_salsa_db::Db,
    project: aura_salsa_db::Project,
    cfg: RunConfig,
) -> Captured {
    let files: Vec<&aura_parser::ParsedFile> = project
        .files(db)
        .iter()
        .map(|f| aura_salsa_db::parsed(db, *f))
        .collect();
    run_files_capture(&files, cfg)
}

fn run_files(files: &[&aura_parser::ParsedFile]) -> Result<i64, InterpError> {
    let c = run_files_capture(files, RunConfig::default());
    emit_captured(&c);
    c.result
}

fn emit_captured(c: &Captured) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(&c.stdout);
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().write_all(&c.stderr);
    let _ = std::io::stderr().flush();
}

fn run_files_capture(files: &[&aura_parser::ParsedFile], cfg: RunConfig) -> Captured {
    // Deep Aura recursion is Rust recursion here — the default 1–8 MB
    // thread stack overflows on real programs (fib(25) ≈ 240k calls).
    // Run on a dedicated 256 MB stack; `scope` joins before returning.
    // `InterpError` isn't `Send` (Rc inside `Value`), so it crosses the
    // thread boundary as a `(tag, msg)` pair.
    std::thread::scope(|s| {
        let r = std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn_scoped(s, || {
                let (res, out, err) = run_files_inner(files, cfg);
                (res.map_err(|e| encode_err(&e)), out, err)
            });
        match r {
            Err(_) => Captured {
                result: Err(InterpError::Unsupported("spawn interp thread")),
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
            Ok(h) => match h.join() {
                Ok((res, out, err)) => Captured {
                    result: res.map_err(|(t, m)| decode_err(t, m)),
                    stdout: out,
                    stderr: err,
                },
                Err(_) => Captured {
                    result: Err(InterpError::Unsupported("interp thread panicked")),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                },
            },
        }
    })
}

fn encode_err(e: &InterpError) -> (u8, String) {
    match e {
        InterpError::DivByZero => (0, String::new()),
        InterpError::NoMain => (1, String::new()),
        InterpError::Type(m) => (2, m.clone()),
        InterpError::Unsupported(s) => (3, (*s).into()),
        InterpError::StepLimit => (4, String::new()),
        InterpError::Unresolved(s) => (5, s.clone()),
        InterpError::Escape(Escape::Break) => (6, String::new()),
        InterpError::Escape(Escape::Continue) => (7, String::new()),
        InterpError::Escape(other) => (8, format!("{other:?}")),
    }
}

fn decode_err(tag: u8, msg: String) -> InterpError {
    match tag {
        0 => InterpError::DivByZero,
        1 => InterpError::NoMain,
        2 => InterpError::Type(msg),
        3 => InterpError::Unsupported("unsupported construct"),
        4 => InterpError::StepLimit,
        5 => InterpError::Unresolved(msg),
        6 => InterpError::Escape(Escape::Break),
        7 => InterpError::Escape(Escape::Continue),
        8 => InterpError::Type(format!("unhandled escape: {msg}")),
        _ => InterpError::Type(format!("wire {tag}: {msg}")),
    }
}

fn run_files_inner(
    files: &[&aura_parser::ParsedFile],
    cfg: RunConfig,
) -> (Result<i64, InterpError>, Vec<u8>, Vec<u8>) {
    let mut interp = Interp::new(files, cfg);
    let res = match interp.call_main() {
        Ok(Value::Int(v)) => {
            i64::try_from(v).map_err(|_| InterpError::Type("main result out of i64 range".into()))
        }
        Ok(v) => Err(InterpError::Type(format!("main returned {v:?}"))),
        // `exit(code)` anywhere in the program ends it here.
        Err(InterpError::Escape(Escape::Exit(code))) => Ok(code),
        Err(e) => Err(e),
    };
    (
        res,
        std::mem::take(&mut interp.out),
        std::mem::take(&mut interp.err),
    )
}

/// Non-value control flow out of a block/statement.
enum Flow {
    Value(Value),
    Return(Value),
    Break,
    Continue,
}

/// An escape bubbling through expression position — `return`, `break`,
/// `continue`, and `expr?` on `Err` all surface this way when the innermost
/// enclosing construct is an expression rather than a statement loop.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub enum Escape {
    Return(Value),
    Break,
    Continue,
    /// `exit(code)` — terminates the *program*, not just the call frame;
    /// propagates as `Err` past every block/loop boundary.
    Exit(i64),
}

impl Escape {
    fn into_flow(self) -> Flow {
        match self {
            Escape::Return(v) => Flow::Return(v),
            Escape::Break => Flow::Break,
            Escape::Continue => Flow::Continue,
            Escape::Exit(_) => unreachable!("Exit propagates as Err, never converts to Flow"),
        }
    }
}

/// Extern functions the interpreter can execute without FFI — user
/// `extern` declarations plus the runtime-backed prelude builtins.
#[derive(Debug, Clone, Copy)]
enum Builtin {
    Sqrt,
    Print,
    Println,
    Eprint,
    Eprintln,
    Exit,
    VecNew,
    VecPush,
    VecGet,
    VecSet,
    VecPop,
    ReadFile,
    WriteFile,
    ReadStdin,
    Exec,
    Args,
    Env,
    StrFromInt,
    StrFromBool,
    StrGet,
    StrSlice,
    StrFromByte,
    StrFromFloat,
    F64FromInt,
}

/// The interpreter: item tables plus a scope stack and fuel.
struct Interp<'a> {
    /// One `ParsedFile` per project unit (single-file runs hold one).
    files: Vec<&'a aura_parser::ParsedFile>,
    /// Index into `files` of the file whose function is executing — Spurs
    /// are interned per-file, so name resolution follows the *current*
    /// file's rodeo.
    cur: usize,
    /// `fn name` → `(file index, item index)`.
    fns: HashMap<String, (usize, usize)>,
    /// `struct name` → field names in declared order (resolved at table
    /// build time so accessor rodeos don't matter).
    structs: HashMap<String, Vec<String>>,
    /// `variant name` → `(enum name, payload arity)`.
    variants: HashMap<String, (String, usize)>,
    /// `extern fn` names the interpreter can execute (see [`Builtin`]).
    builtins: HashSet<String>,
    scopes: Vec<HashMap<Spur, Value>>,
    steps: u64,
    /// Captured stdout/stderr — `print`/`eprint` write here so callers
    /// can inspect output (`run_*` flushes them to the real streams).
    out: Vec<u8>,
    err: Vec<u8>,
    /// `read_stdin` source — see [`StdinIo`].
    stdin: StdinIo,
    /// Working directory for relative `read_file`/`write_file`/`exec`
    /// paths — the compiled binary gets this via `current_dir`.
    cwd: std::path::PathBuf,
    /// `args()` argv — injected by the harness; `None` = real argv.
    args: Option<Vec<String>>,
}

/// `read_stdin` backing state: `provided = Some` is harness-fed input;
/// `None` reads the real process stdin lazily. `done` marks the first
/// read — later calls get `""` (drained), matching the compiled OS read.
struct StdinIo {
    provided: Option<Vec<u8>>,
    done: bool,
}

/// Process-environment inputs a host can inject — everything `aura
/// test` fakes so interpreted runs match spawned binaries.
#[derive(Default)]
pub struct RunConfig {
    /// `read_stdin` bytes; `None` = read the real stdin lazily.
    pub stdin: Option<Vec<u8>>,
    /// Working directory for relative file paths and `exec` children.
    pub cwd: Option<std::path::PathBuf>,
    /// `args()` result: `Some` = harness argv (program name included at
    /// [0]); `None` = the real process argv.
    pub args: Option<Vec<String>>,
}

/// `f64_from_int` — the signed-i64 conversion contract: `v as i64`
/// truncates like codegen's `ireduce`, `as f64` is `fcvt_from_sint`.
/// Precision loss above 2^53 is inherent to f64.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn f64_from_int(v: i128) -> f64 {
    v as i64 as f64
}

/// `str_from_f64` — `%.6f` fixed notation. Rust's `{:.6}` rounds the
/// exact decimal value correctly (dyadic f64s never hit a `.6f` tie),
/// matching the runtime's exact u128 fixed-point implementation.
/// `nan`/`±inf` mirror the runtime contract, including the
/// |x| ≥ 1e38 bound.
fn fmt_f64(v: f64) -> Vec<u8> {
    if v.is_nan() {
        b"nan".to_vec()
    } else if v.abs() >= 1e38 || v.is_infinite() {
        if v.is_sign_negative() {
            b"-inf".to_vec()
        } else {
            b"inf".to_vec()
        }
    } else {
        format!("{v:.6}").into_bytes()
    }
}

/// Resolve `path` against `cwd` — relative paths root at the scenario
/// dir (matching the compiled binary's `current_dir`).
fn resolve_path(cwd: &std::path::Path, path: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

impl StdinIo {
    /// Drain the source once; subsequent calls return empty.
    fn read(&mut self) -> Vec<u8> {
        use std::io::Read;
        if self.done {
            return Vec::new();
        }
        self.done = true;
        if let Some(b) = self.provided.take() {
            b
        } else {
            let mut v = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut v);
            v
        }
    }
}

/// Output captured from an interpreted run: the result plus everything
/// the program wrote to stdout/stderr.
pub struct Captured {
    pub result: Result<i64, InterpError>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

const STEP_LIMIT: u64 = 5_000_000;

impl<'a> Interp<'a> {
    fn new(files: &[&'a aura_parser::ParsedFile], cfg: RunConfig) -> Self {
        let mut this = Self {
            files: files.to_vec(),
            cur: 0,
            fns: HashMap::new(),
            structs: HashMap::new(),
            variants: HashMap::new(),
            builtins: HashSet::new(),
            scopes: vec![HashMap::new()],
            steps: 0,
            out: Vec::new(),
            err: Vec::new(),
            stdin: StdinIo {
                provided: cfg.stdin,
                done: false,
            },
            cwd: cfg
                .cwd
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into())),
            args: cfg.args,
        };
        for (fi, parsed) in this.files.iter().enumerate() {
            this.cur = fi;
            for (i, item) in parsed.items.iter().enumerate() {
                match item {
                    Item::Function(f) if !f.is_extern => {
                        this.fns.entry(this.name_of(f.name)).or_insert((fi, i));
                    }
                    Item::Struct(s) => {
                        let fields = s.fields.iter().map(|f| this.name_of(f.name)).collect();
                        this.structs.entry(this.name_of(s.name)).or_insert(fields);
                    }
                    Item::Enum(e) => {
                        let ename = this.name_of(e.name);
                        for v in &e.variants {
                            let arity = match &v.payload {
                                aura_ast::VariantPayload::None => 0,
                                aura_ast::VariantPayload::Tuple(ts) => ts.len(),
                            };
                            this.variants
                                .entry(this.name_of(v.name))
                                .or_insert((ename.clone(), arity));
                        }
                    }
                    Item::ExternBlock { fns, .. } => {
                        for f in fns {
                            let name = this.name_of(f.name);
                            if Builtin::by_name(&name).is_some() {
                                this.builtins.insert(name);
                            }
                        }
                    }
                    Item::Function(_) | Item::Use { .. } | Item::Error { .. } => {}
                }
            }
        }
        this.cur = 0;
        // Built-in `Result` constructors — single-payload variants.
        this.variants.insert("Ok".into(), ("Result".into(), 1));
        this.variants.insert("Err".into(), ("Result".into(), 1));
        this
    }

    fn parsed(&self) -> &'a aura_parser::ParsedFile {
        self.files[self.cur]
    }

    fn name_of(&self, spur: Spur) -> String {
        self.parsed().rodeo.resolve(&spur).to_owned()
    }

    fn tick(&mut self) -> Result<(), InterpError> {
        self.steps += 1;
        if self.steps > STEP_LIMIT {
            return Err(InterpError::StepLimit);
        }
        Ok(())
    }

    fn call_main(&mut self) -> Result<Value, InterpError> {
        let Some(&(fi, idx)) = self.fns.get("main") else {
            return Err(InterpError::NoMain);
        };
        self.call_fn(fi, idx, &[])
    }

    // ----- functions ----------------------------------------------------------

    fn call_fn(
        &mut self,
        file_idx: usize,
        item_idx: usize,
        args: &[Value],
    ) -> Result<Value, InterpError> {
        self.tick()?;
        // Function bodies see only their own scope — swap the caller's
        // scope stack out so callee idents can never bind to caller locals
        // (and so per-file Spurs from different files can't collide).
        let saved_scopes = std::mem::replace(&mut self.scopes, vec![HashMap::new()]);
        let saved_cur = self.cur;
        self.cur = file_idx;
        let out = self.call_fn_inner(item_idx, args);
        self.cur = saved_cur;
        self.scopes = saved_scopes;
        out
    }

    fn call_fn_inner(&mut self, item_idx: usize, args: &[Value]) -> Result<Value, InterpError> {
        let Item::Function(f) = &self.parsed().items[item_idx] else {
            return Err(InterpError::Unresolved(format!("item {item_idx}")));
        };
        let Some(body_id) = f.body else {
            return Err(InterpError::Unsupported("bodiless fn"));
        };
        if args.len() != f.params.len() {
            return Err(InterpError::Type(format!(
                "arity mismatch: {} args for {} params",
                args.len(),
                f.params.len()
            )));
        }
        self.scopes.push(HashMap::new());
        for (p, a) in f.params.iter().zip(args.iter()) {
            self.scopes
                .last_mut()
                .expect("scope")
                .insert(p.name, a.clone());
        }
        let flow = match self.block(body_id) {
            Ok(flow) => flow,
            // `expr?`-on-`Err` / `return` inside an expr-block escapes here.
            Err(InterpError::Escape(Escape::Return(v))) => Flow::Return(v),
            Err(e) => {
                self.scopes.pop();
                return Err(e);
            }
        };
        self.scopes.pop();
        match flow {
            Flow::Return(v) | Flow::Value(v) => Ok(v),
            Flow::Break | Flow::Continue => {
                Err(InterpError::Type("break/continue escaped function".into()))
            }
        }
    }

    // ----- blocks & statements ----------------------------------------------------

    fn block(&mut self, id: aura_ast::BlockId) -> Result<Flow, InterpError> {
        self.tick()?;
        let Block { stmts, tail, .. } = self.parsed().ast.block(id).clone();
        self.scopes.push(HashMap::new());
        let mut out = Flow::Value(Value::Unit);
        for sid in stmts {
            match self.stmt(sid) {
                Ok(Flow::Value(_)) => {}
                Ok(other) => {
                    out = other;
                    break;
                }
                Err(InterpError::Escape(e)) if !matches!(e, Escape::Exit(_)) => {
                    out = e.into_flow();
                    break;
                }
                Err(e) => {
                    self.scopes.pop();
                    return Err(e);
                }
            }
        }
        if matches!(out, Flow::Value(_))
            && let Some(t) = tail
        {
            match self.expr(t) {
                Ok(v) => out = Flow::Value(v),
                Err(InterpError::Escape(e)) if !matches!(e, Escape::Exit(_)) => {
                    out = e.into_flow();
                }
                Err(e) => {
                    self.scopes.pop();
                    return Err(e);
                }
            }
        }
        self.scopes.pop();
        Ok(out)
    }

    fn stmt(&mut self, id: StmtId) -> Result<Flow, InterpError> {
        self.tick()?;
        match self.parsed().ast.stmt(id).clone() {
            Stmt::Let { name, init, .. } => {
                let v = self.expr(init)?;
                self.scopes.last_mut().expect("scope").insert(name, v);
                Ok(Flow::Value(Value::Unit))
            }
            Stmt::Expr(e) => self.expr(e).map(Flow::Value),
            Stmt::Return(e) => {
                let v = match e {
                    Some(e) => self.expr(e)?,
                    None => Value::Unit,
                };
                Ok(Flow::Return(v))
            }
            Stmt::While { cond, body } => {
                loop {
                    self.tick()?;
                    if !self.expr(cond)?.truthy()? {
                        break;
                    }
                    match self.block(body)? {
                        Flow::Value(_) | Flow::Continue => {}
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                    }
                }
                Ok(Flow::Value(Value::Unit))
            }
            Stmt::Loop { body } => {
                loop {
                    self.tick()?;
                    match self.block(body)? {
                        Flow::Value(_) | Flow::Continue => {}
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                    }
                }
                Ok(Flow::Value(Value::Unit))
            }
            Stmt::Break => Ok(Flow::Break),
            Stmt::Continue => Ok(Flow::Continue),
            Stmt::Error => Err(InterpError::Type("error statement".into())),
        }
    }

    // ----- expressions -------------------------------------------------------------

    fn expr(&mut self, id: ExprId) -> Result<Value, InterpError> {
        self.tick()?;
        match self.parsed().ast.expr(id).clone() {
            Expr::Error => Err(InterpError::Type("error expr".into())),
            Expr::Literal(l) => Ok(self.literal(&l)),
            Expr::Ident(name) => self.lookup(name),
            Expr::Binary { op, lhs, rhs } => self.binary(op, lhs, rhs),
            Expr::Unary { op, operand } => {
                let v = self.expr(operand)?;
                Self::unary(op, &v)
            }
            Expr::Assign { target, value } => {
                let v = self.expr(value)?;
                self.assign(target, v.clone())?;
                Ok(v)
            }
            Expr::Call { callee, args } => self.call(callee, &args),
            Expr::Field { object, field } => {
                let obj = self.expr(object)?;
                let fname = self.name_of(field);
                match obj {
                    Value::Struct { fields, name } => {
                        let names = self
                            .structs
                            .get(&name)
                            .ok_or_else(|| InterpError::Unresolved(name.clone()))?;
                        let idx = names
                            .iter()
                            .position(|f| *f == fname)
                            .ok_or_else(|| InterpError::Unresolved(fname.clone()))?;
                        fields
                            .get(idx)
                            .cloned()
                            .ok_or(InterpError::Type("struct field index".into()))
                    }
                    // `str` exposes its `{ ptr, len }` repr — `len` is the
                    // byte count, `ptr` the buffer's address as an opaque int.
                    Value::Str(s) => match fname.as_str() {
                        "len" => Ok(Value::Int(i128::try_from(s.len()).unwrap_or(i128::MAX))),
                        "ptr" => Ok(Value::Int(i128::from(
                            u64::try_from(s.as_ptr().addr()).unwrap_or(u64::MAX),
                        ))),
                        _ => Err(InterpError::Type(format!("no field `{fname}` on `str`"))),
                    },
                    // `vec<T>` exposes `{ ptr, len, cap }` — same opaque-
                    // int treatment for `ptr` as `str`.
                    Value::Vec(v) => match fname.as_str() {
                        "len" => Ok(Value::Int(
                            i128::try_from(v.borrow().len()).unwrap_or(i128::MAX),
                        )),
                        "cap" => Ok(Value::Int(
                            i128::try_from(v.borrow().capacity()).unwrap_or(i128::MAX),
                        )),
                        "ptr" => Ok(Value::Int(i128::from(
                            u64::try_from(v.borrow().as_ptr().addr()).unwrap_or(u64::MAX),
                        ))),
                        _ => Err(InterpError::Type(format!("no field `{fname}` on `vec<T>`"))),
                    },
                    v => Err(InterpError::Type(format!("field access on {v:?}"))),
                }
            }
            Expr::If {
                cond,
                then_block,
                else_branch,
            } => {
                if self.expr(cond)?.truthy()? {
                    return self.block_value(then_block);
                }
                match else_branch {
                    Some(e) => self.expr(e),
                    None => Ok(Value::Unit),
                }
            }
            Expr::Block(b) | Expr::Unsafe(b) => self.block_value(b),
            Expr::Paren(inner) => self.expr(inner),
            Expr::Match { scrutinee, arms } => self.match_expr(scrutinee, &arms),
            Expr::StructLit { name, fields } => self.struct_lit(name, &fields),
            Expr::Try { expr } => {
                let v = self.expr(expr)?;
                match v {
                    Value::Variant {
                        variant, fields, ..
                    } if variant == "Ok" => Ok(fields.into_iter().next().unwrap_or(Value::Unit)),
                    Value::Variant {
                        variant, fields, ..
                    } if variant == "Err" => {
                        Err(InterpError::Escape(Escape::Return(Value::Variant {
                            enum_name: "Result".into(),
                            variant: "Err".into(),
                            fields,
                        })))
                    }
                    v => Err(InterpError::Type(format!("`?` on {v:?}"))),
                }
            }
        }
    }

    /// Evaluate a block in expression position. `return`/`break`/`continue`
    /// inside it do not become the block's value — they escape upward and are
    /// re-caught by the enclosing statement-level `block()`.
    fn block_value(&mut self, id: aura_ast::BlockId) -> Result<Value, InterpError> {
        match self.block(id)? {
            Flow::Value(v) => Ok(v),
            Flow::Return(v) => Err(InterpError::Escape(Escape::Return(v))),
            Flow::Break => Err(InterpError::Escape(Escape::Break)),
            Flow::Continue => Err(InterpError::Escape(Escape::Continue)),
        }
    }

    fn struct_lit(&mut self, name: Spur, fields: &[(Spur, ExprId)]) -> Result<Value, InterpError> {
        let sname = self.name_of(name);
        let defs = self
            .structs
            .get(&sname)
            .cloned()
            .ok_or_else(|| InterpError::Unresolved(sname.clone()))?;
        let mut vals = Vec::with_capacity(fields.len());
        for (fname, eid) in fields {
            vals.push((self.name_of(*fname), self.expr(*eid)?));
        }
        // Reorder into declared field order.
        let mut ordered = Vec::with_capacity(defs.len());
        for d in &defs {
            let v = vals
                .iter()
                .find(|(n, _)| n == d)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| InterpError::Unresolved(d.clone()))?;
            ordered.push(v);
        }
        Ok(Value::Struct {
            name: sname,
            fields: ordered,
        })
    }

    // ----- operators ---------------------------------------------------------------

    fn literal(&self, l: &Literal) -> Value {
        match l {
            Literal::Int(v) => Value::Int((*v).cast_signed()),
            Literal::Float(v) => Value::Float(*v),
            Literal::Bool(v) => Value::Bool(*v),
            Literal::Unit => Value::Unit,
            Literal::Str(s) => Value::Str(Rc::from(self.name_of(*s).into_bytes())),
        }
    }

    fn lookup(&self, name: Spur) -> Result<Value, InterpError> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(&name) {
                return Ok(v.clone());
            }
        }
        // Bare unit variants (`Empty`) evaluate to their variant value.
        let n = self.name_of(name);
        if let Some((en, 0)) = self.variants.get(&n) {
            return Ok(Value::Variant {
                enum_name: en.clone(),
                variant: n,
                fields: Vec::new(),
            });
        }
        Err(InterpError::Unresolved(n))
    }

    fn binary(&mut self, op: BinOp, lhs: ExprId, rhs: ExprId) -> Result<Value, InterpError> {
        // Short-circuit first — `rhs` may be invalid when lhs decides.
        if op == BinOp::And {
            let l = self.expr(lhs)?.truthy()?;
            return Ok(Value::Bool(l && self.expr(rhs)?.truthy()?));
        }
        if op == BinOp::Or {
            let l = self.expr(lhs)?.truthy()?;
            return Ok(Value::Bool(l || self.expr(rhs)?.truthy()?));
        }
        let l = self.expr(lhs)?;
        let r = self.expr(rhs)?;
        match (op, &l, &r) {
            (BinOp::Add, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_add(*b))),
            (BinOp::Sub, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_sub(*b))),
            (BinOp::Mul, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_mul(*b))),
            (BinOp::Div | BinOp::Rem, Value::Int(_), Value::Int(0)) => Err(InterpError::DivByZero),
            (BinOp::Div, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_div(*b))),
            (BinOp::Rem, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_rem(*b))),
            (BinOp::Add, Value::Str(a), Value::Str(b)) => {
                let mut buf = Vec::with_capacity(a.len() + b.len());
                buf.extend_from_slice(a);
                buf.extend_from_slice(b);
                Ok(Value::Str(Rc::from(buf)))
            }
            (BinOp::Add, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
            (BinOp::Sub, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
            (BinOp::Mul, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
            (BinOp::Div, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
            (BinOp::Rem, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a % b)),
            (BinOp::Eq, a, b) => Ok(Value::Bool(a == b)),
            (BinOp::Ne, a, b) => Ok(Value::Bool(a != b)),
            (BinOp::Lt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
            (BinOp::Le, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
            (BinOp::Gt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
            (BinOp::Ge, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
            (BinOp::Lt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
            (BinOp::Le, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a <= b)),
            (BinOp::Gt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
            (BinOp::Ge, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a >= b)),
            _ => Err(InterpError::Type(format!(
                "bad operands {l:?} {op:?} {r:?}"
            ))),
        }
    }

    fn unary(op: UnOp, v: &Value) -> Result<Value, InterpError> {
        match (op, &v) {
            (UnOp::Neg, Value::Int(a)) => Ok(Value::Int(a.wrapping_neg())),
            (UnOp::Neg, Value::Float(a)) => Ok(Value::Float(-a)),
            (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
            _ => Err(InterpError::Type(format!("bad unary {op:?} {v:?}"))),
        }
    }

    // ----- calls & match ----------------------------------------------------------

    /// Assign `v` to an lvalue: `x = v` or `p.a.b = v` — mutates the
    /// struct field in place through the root binding (value semantics,
    /// matching codegen's stack-slot store).
    fn assign(&mut self, target: ExprId, v: Value) -> Result<(), InterpError> {
        let mut path: Vec<String> = Vec::new();
        let mut cur = target;
        let root = loop {
            match self.parsed().ast.expr(cur) {
                Expr::Ident(n) => break *n,
                Expr::Field { object, field } => {
                    path.push(self.name_of(*field));
                    cur = *object;
                }
                _ => return Err(InterpError::Unsupported("non-lvalue assignment")),
            }
        };
        path.reverse();
        for scope in self.scopes.iter_mut().rev() {
            if let Some(slot) = scope.get_mut(&root) {
                return set_field_path(&self.structs, slot, &path, v);
            }
        }
        Err(InterpError::Unresolved(self.name_of(root)))
    }

    fn call(&mut self, callee: ExprId, args: &[ExprId]) -> Result<Value, InterpError> {
        let vals: Vec<Value> = args
            .iter()
            .map(|a| self.expr(*a))
            .collect::<Result<_, _>>()?;
        let Expr::Ident(name) = self.parsed().ast.expr(callee) else {
            return Err(InterpError::Unsupported("indirect call"));
        };
        let cname = self.name_of(*name);

        // Variant constructors (enums + Result's Ok/Err) take precedence —
        // matching typeck, which resolves a bare `Name(..)` to a variant when
        // a variant of that name exists.
        if let Some((en, arity)) = self.variants.get(&cname).cloned()
            && arity == vals.len()
        {
            return Ok(Value::Variant {
                enum_name: en,
                variant: cname,
                fields: vals,
            });
        }
        if self.builtins.contains(&cname) {
            let b = Builtin::by_name(&cname).expect("registered builtin");
            return b.call(
                &vals,
                &mut self.out,
                &mut self.err,
                &mut self.stdin,
                &self.cwd,
                self.args.as_deref(),
            );
        }
        let Some(&(fi, idx)) = self.fns.get(&cname) else {
            // Prelude builtins — reached only when no user fn/variant
            // claimed the name above, so user definitions shadow them.
            if let Some(b) = Builtin::by_name(&cname) {
                return b.call(
                    &vals,
                    &mut self.out,
                    &mut self.err,
                    &mut self.stdin,
                    &self.cwd,
                    self.args.as_deref(),
                );
            }
            return Err(InterpError::Unresolved(cname));
        };
        self.call_fn(fi, idx, &vals)
    }

    fn match_expr(
        &mut self,
        scrutinee: ExprId,
        arms: &[aura_ast::MatchArm],
    ) -> Result<Value, InterpError> {
        let scrut = self.expr(scrutinee)?;
        for arm in arms {
            self.scopes.push(HashMap::new());
            if self.try_bind(&arm.pattern, &scrut)? {
                let v = self.expr(arm.body);
                self.scopes.pop();
                return v;
            }
            self.scopes.pop();
        }
        Err(InterpError::Type("non-exhaustive match".into()))
    }

    /// Try to bind `pat` against `v`, pushing bindings into the top scope.
    /// Returns whether the pattern matched.
    fn try_bind(&mut self, pat: &Pattern, v: &Value) -> Result<bool, InterpError> {
        match pat {
            Pattern::Wildcard => Ok(true),
            Pattern::Ident(name) => {
                // An ident pattern that names a unit variant *tests* rather
                // than binds (same resolution rule as typeck).
                let n = self.name_of(*name);
                if let Some((_, 0)) = self.variants.get(&n)
                    && let Value::Variant { variant, .. } = v
                {
                    return Ok(*variant == n);
                }
                self.scopes
                    .last_mut()
                    .expect("scope")
                    .insert(*name, v.clone());
                Ok(true)
            }
            Pattern::Literal(l) => Ok(&self.literal(l) == v),
            Pattern::Variant { name, args } => {
                let pname = self.name_of(*name);
                let Value::Variant {
                    variant, fields, ..
                } = v
                else {
                    return Ok(false);
                };
                if *variant != pname || fields.len() != args.len() {
                    return Ok(false);
                }
                for (p, f) in args.iter().zip(fields.iter()) {
                    if !self.try_bind(p, f)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }
}

impl Builtin {
    fn by_name(name: &str) -> Option<Self> {
        match name {
            "sqrt" => Some(Self::Sqrt),
            "print" => Some(Self::Print),
            "println" => Some(Self::Println),
            "eprint" => Some(Self::Eprint),
            "eprintln" => Some(Self::Eprintln),
            "exit" => Some(Self::Exit),
            "vec_new" => Some(Self::VecNew),
            "vec_push" => Some(Self::VecPush),
            "vec_get" => Some(Self::VecGet),
            "vec_set" => Some(Self::VecSet),
            "args" => Some(Self::Args),
            "env" => Some(Self::Env),
            "str_from_int" => Some(Self::StrFromInt),
            "str_from_bool" => Some(Self::StrFromBool),
            "str_get" => Some(Self::StrGet),
            "str_slice" => Some(Self::StrSlice),
            "str_from_byte" => Some(Self::StrFromByte),
            "str_from_f64" => Some(Self::StrFromFloat),
            "f64_from_int" => Some(Self::F64FromInt),
            "vec_pop" => Some(Self::VecPop),
            "read_file" => Some(Self::ReadFile),
            "write_file" => Some(Self::WriteFile),
            "read_stdin" => Some(Self::ReadStdin),
            "exec" => Some(Self::Exec),
            _ => None,
        }
    }

    fn call(
        self,
        args: &[Value],
        out: &mut dyn std::io::Write,
        err: &mut dyn std::io::Write,
        stdin: &mut StdinIo,
        cwd: &std::path::Path,
        args_argv: Option<&[String]>,
    ) -> Result<Value, InterpError> {
        use std::io::Write;
        let mut write = |stderr: bool, s: &[u8], nl: bool| {
            let w: &mut dyn Write = if stderr { &mut *err } else { &mut *out };
            let _ = w.write_all(s);
            if nl {
                let _ = w.write_all(b"\n");
            }
            let _ = w.flush();
        };
        match (self, args) {
            (Self::Sqrt, [Value::Float(x)]) => Ok(Value::Float(x.sqrt())),
            (Self::Print, [Value::Str(s)]) => {
                write(false, s, false);
                Ok(Value::Unit)
            }
            (Self::Println, [Value::Str(s)]) => {
                write(false, s, true);
                Ok(Value::Unit)
            }
            (Self::Eprint, [Value::Str(s)]) => {
                write(true, s, false);
                Ok(Value::Unit)
            }
            (Self::Eprintln, [Value::Str(s)]) => {
                write(true, s, true);
                Ok(Value::Unit)
            }
            (Self::Exit, [Value::Int(c)]) => Err(InterpError::Escape(Escape::Exit(
                i64::try_from(*c).unwrap_or(i64::MAX),
            ))),
            (Self::VecNew, []) => Ok(Value::Vec(Rc::new(RefCell::new(Vec::new())))),
            (Self::VecPush, [Value::Vec(v), x]) => {
                v.borrow_mut().push(x.clone());
                Ok(Value::Unit)
            }
            (Self::VecGet, [Value::Vec(v), Value::Int(i)]) => {
                let i = usize::try_from(*i).unwrap_or(usize::MAX);
                // Compiled OOB is ExitProcess(101) — match it.
                v.borrow()
                    .get(i)
                    .cloned()
                    .ok_or(InterpError::Escape(Escape::Exit(101)))
            }
            (Self::VecSet, [Value::Vec(v), Value::Int(i), x]) => {
                let i = usize::try_from(*i).unwrap_or(usize::MAX);
                let mut b = v.borrow_mut();
                let Some(slot) = b.get_mut(i) else {
                    return Err(InterpError::Escape(Escape::Exit(101)));
                };
                *slot = x.clone();
                Ok(Value::Unit)
            }
            (Self::VecPop, [Value::Vec(v)]) => {
                // Empty pop: compiled `vec_get(len-1)` wraps to
                // usize::MAX → exit 101 — match it.
                v.borrow_mut()
                    .pop()
                    .ok_or(InterpError::Escape(Escape::Exit(101)))
            }
            (Self::Args, []) => Ok(Value::Vec(Rc::new(RefCell::new(
                match args_argv {
                    Some(a) => a.to_vec(),
                    None => std::env::args().collect(),
                }
                .into_iter()
                .map(|a| Value::Str(Rc::from(a.into_bytes())))
                .collect(),
            )))),
            (Self::Env, [Value::Str(name)]) => Ok(Value::Str(Rc::from(
                std::env::var(std::str::from_utf8(name).unwrap_or_default())
                    .unwrap_or_default()
                    .into_bytes(),
            ))),
            (Self::StrFromInt, [Value::Int(v)]) => {
                Ok(Value::Str(Rc::from(format!("{v}").into_bytes())))
            }
            (Self::StrFromBool, [Value::Bool(b)]) => Ok(Value::Str(Rc::from(if *b {
                b"true".as_slice()
            } else {
                b"false".as_slice()
            }))),
            // Low 8 bits of the i128 — the byte value.
            (Self::StrFromByte, [Value::Int(v)]) => Ok(Value::Str(Rc::from([v.to_le_bytes()[0]]))),
            (Self::StrFromFloat, [Value::Float(x)]) => Ok(Value::Str(Rc::from(fmt_f64(*x)))),
            (Self::F64FromInt, [Value::Int(v)]) => Ok(Value::Float(f64_from_int(*v))),
            (Self::StrGet, [Value::Str(s), Value::Int(i)]) => {
                let i = usize::try_from(*i).unwrap_or(usize::MAX);
                // Compiled OOB is ExitProcess(101) — match it.
                s.get(i)
                    .map(|&b| Value::Int(i128::from(b)))
                    .ok_or(InterpError::Escape(Escape::Exit(101)))
            }
            (Self::StrSlice, [Value::Str(s), Value::Int(lo), Value::Int(hi)]) => {
                let lo = usize::try_from(*lo).unwrap_or(usize::MAX);
                let hi = usize::try_from(*hi).unwrap_or(usize::MAX);
                if lo > hi || hi > s.len() {
                    return Err(InterpError::Escape(Escape::Exit(101)));
                }
                Ok(Value::Str(Rc::from(&s[lo..hi])))
            }
            (Self::ReadFile | Self::WriteFile | Self::ReadStdin | Self::Exec, _) => {
                self.file_proc_io(args, stdin, cwd)
            }
            _ => Err(InterpError::Type("bad builtin args".into())),
        }
    }

    /// `read_file`/`write_file`/`read_stdin`/`exec` — the file and
    /// process side of the builtin set, split out of `call` to keep
    /// it under the line limit. Paths resolve against `cwd` so a
    /// scenario's working dir matches the spawned binary's.
    fn file_proc_io(
        self,
        args: &[Value],
        stdin: &mut StdinIo,
        cwd: &std::path::Path,
    ) -> Result<Value, InterpError> {
        match (self, args) {
            (Self::ReadFile, [Value::Str(path)]) => {
                let p = resolve_path(cwd, &String::from_utf8_lossy(path));
                let (variant, payload) = match std::fs::read(&p) {
                    Ok(b) => ("Ok", Value::Str(Rc::from(b))),
                    Err(_) => ("Err", Value::Str(Rc::from(&b"cannot open file"[..]))),
                };
                Ok(Value::Variant {
                    enum_name: "Result".into(),
                    variant: variant.into(),
                    fields: vec![payload],
                })
            }
            (Self::WriteFile, [Value::Str(path), Value::Str(data)]) => {
                let p = resolve_path(cwd, &String::from_utf8_lossy(path));
                Ok(Value::Bool(std::fs::write(&p, data.as_ref()).is_ok()))
            }
            (Self::ReadStdin, []) => Ok(Value::Str(Rc::from(stdin.read()))),
            (Self::Exec, [Value::Str(cmd)]) => {
                let c = String::from_utf8_lossy(cmd).into_owned();
                // Match CreateProcessW's split-on-whitespace cmdline
                // parsing: first token is the program (resolved via
                // PATH), the rest are args. Spawn failure → -1, like
                // the runtime. Child output is captured and dropped —
                // `exec`'s contract is the exit code.
                let mut it = c.split_whitespace();
                let code = match it.next() {
                    None => -1,
                    Some(prog) => std::process::Command::new(prog)
                        .args(it)
                        .current_dir(cwd)
                        .output()
                        .ok()
                        .and_then(|o| o.status.code())
                        .map_or(-1, i64::from),
                };
                Ok(Value::Int(i128::from(code)))
            }
            _ => Err(InterpError::Type("bad builtin args".into())),
        }
    }
}

/// Walk `slot`'s struct fields along `path` and assign `v` — `p.a.b = v`
/// mutates in place through the root binding (value semantics, matching
/// codegen's stack-slot store).
fn set_field_path(
    structs: &HashMap<String, Vec<String>>,
    slot: &mut Value,
    path: &[String],
    v: Value,
) -> Result<(), InterpError> {
    let Some((f, rest)) = path.split_first() else {
        *slot = v;
        return Ok(());
    };
    let Value::Struct { name, fields } = slot else {
        return Err(InterpError::Type("assign through non-struct".into()));
    };
    let names = structs
        .get(name.as_str())
        .ok_or_else(|| InterpError::Unresolved(name.clone()))?;
    let Some(idx) = names.iter().position(|n| n == f) else {
        return Err(InterpError::Unresolved(f.clone()));
    };
    set_field_path(structs, &mut fields[idx], rest, v)
}
