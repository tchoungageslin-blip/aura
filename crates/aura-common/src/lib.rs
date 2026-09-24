//! Shared types for the Aura compiler: source locations, diagnostics, error codes.
//!
//! Every crate in the workspace depends on this one. It must stay small and
//! dependency-light.

mod diagnostic;
mod render;
mod span;

pub use diagnostic::{Diagnostic, DiagnosticSink, Label, Severity};
pub use render::{SourceCache, render_diagnostics};
pub use span::{FileId, Span};

/// Prelude functions implemented by the runtime, callable like ordinary
/// `fn`s — name resolution binds these to builtin defs; codegen maps
/// each call to a runtime import.
///
/// `str` arguments are passed as the flattened `{ptr, len}` pair — the
/// runtime ABI stays scalar-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinFn {
    /// `print(s: str)` — write `s`'s bytes to stdout.
    Print,
    /// `println(s: str)` — `print` plus a newline.
    Println,
    /// `eprint(s: str)` — write `s`'s bytes to stderr.
    Eprint,
    /// `eprintln(s: str)` — `eprint` plus a newline.
    Eprintln,
    /// `exit(code: i64) -> !` — terminate the process.
    Exit,
    /// `sqrt(x: f64) -> f64` — hardware square root.
    Sqrt,
    /// `str_from_int(v: i64) -> str` — decimal rendering, heap-allocated.
    StrFromInt,
    /// `str_from_bool(v: bool) -> str` — `"true"`/`"false"`.
    StrFromBool,
    /// `str_get(s: str, i: usize) -> i64` — bounds-checked byte load;
    /// widened like C's `getchar` (Aura has no int casts yet).
    StrGet,
    /// `str_slice(s: str, lo: usize, hi: usize) -> str` — bounds-checked
    /// `{ptr+lo, hi-lo}` view into the same buffer (no copy).
    StrSlice,
    /// `vec_new<T>() -> vec<T>` — empty `{ptr: 0, len: 0, cap: 0}`.
    VecNew,
    /// `vec_push<T>(v: vec<T>, x: T)` — grow + append in place.
    VecPush,
    /// `vec_get<T>(v: vec<T>, i: usize) -> T` — bounds-checked load.
    VecGet,
    /// `vec_set<T>(v: vec<T>, i: usize, x: T)` — bounds-checked store.
    VecSet,
    /// `vec_pop<T>(v: vec<T>) -> T` — remove + return the last element;
    /// exit 101 on empty (same trap as `vec_get` OOB).
    VecPop,
    /// `args() -> vec<str>` — process command line, program name first.
    Args,
    /// `env(name: str) -> str` — environment variable value or `""`.
    Env,
    /// `read_file(path: str) -> Result<str, str>` — whole file as bytes;
    /// `Err(msg)` when the OS refuses (missing, denied, …).
    ReadFile,
    /// `write_file(path: str, data: str) -> bool` — create/truncate +
    /// write all bytes; `false` on OS failure.
    WriteFile,
    /// `read_stdin() -> str` — stdin to EOF as bytes.
    ReadStdin,
    /// `exec(cmd: str) -> i64` — spawn, wait, return the child's exit
    /// code; `-1` when the OS can't spawn it. The child gets no stdio —
    /// observable effects are the exit code and file system changes.
    Exec,
    /// `str_from_byte(b: i64) -> str` — the single-byte string for the
    /// low 8 bits; the primitive for building bytes programmatically.
    StrFromByte,
    /// `str_from_f64(x: f64) -> str` — `%.6f` fixed notation with exact
    /// rounding ("3.141593", "-0.000000"); `"nan"`/`"±inf"` for
    /// non-finite or |x| ≥ 1e38.
    StrFromFloat,
    /// `f64_from_int(v) -> f64` — int→float conversion, generic over
    /// int widths like `str_from_int`.
    F64FromInt,
}

impl BuiltinFn {
    /// Every builtin, for prelude injection.
    pub const ALL: &[Self] = &[
        Self::Print,
        Self::Println,
        Self::Eprint,
        Self::Eprintln,
        Self::Exit,
        Self::Sqrt,
        Self::StrFromInt,
        Self::StrFromBool,
        Self::StrGet,
        Self::StrSlice,
        Self::VecNew,
        Self::VecPush,
        Self::VecGet,
        Self::VecSet,
        Self::VecPop,
        Self::Args,
        Self::Env,
        Self::ReadFile,
        Self::WriteFile,
        Self::ReadStdin,
        Self::Exec,
        Self::StrFromByte,
        Self::StrFromFloat,
        Self::F64FromInt,
    ];

    /// Look up a builtin by its source-level name (`vec_push`, …).
    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|b| b.name() == name)
    }

    /// One-line signature + short doc, for `aura doc` and LSP hover.
    /// Single source of truth — keep in sync with the doc comments.
    pub fn doc(self) -> &'static str {
        match self {
            Self::Print => "print(s: str)\nWrite `s` to stdout.",
            Self::Println => "println(s: str)\nWrite `s` + newline to stdout.",
            Self::Eprint => "eprint(s: str)\nWrite `s` to stderr.",
            Self::Eprintln => "eprintln(s: str)\nWrite `s` + newline to stderr.",
            Self::Exit => "exit(code: i64) -> !\nTerminate the process with `code`.",
            Self::Sqrt => "sqrt(x: f64) -> f64\nSquare root.",
            Self::StrFromInt => {
                "str_from_int(n) -> str\nDecimal rendering of an integer (any int width)."
            }
            Self::StrFromBool => "str_from_bool(b: bool) -> str\n\"true\" / \"false\".",
            Self::StrGet => {
                "str_get(s: str, i) -> i64\nByte at index `i` (0-255). Exits 101 if out of bounds."
            }
            Self::StrSlice => {
                "str_slice(s: str, lo, hi) -> str\nBytes `lo..hi` as a view — no copy. Exits 101 on a bad range."
            }
            Self::VecNew => {
                "vec_new<T>() -> vec<T>\nEmpty vector; T inferred from first use — annotate when empty."
            }
            Self::VecPush => {
                "vec_push<T>(v: vec<T>, x: T)\nAppend `x`, doubling capacity as needed."
            }
            Self::VecGet => {
                "vec_get<T>(v: vec<T>, i: usize) -> T\nBounds-checked load. Exits 101 if out of bounds."
            }
            Self::VecSet => {
                "vec_set<T>(v: vec<T>, i: usize, x: T)\nBounds-checked store. Exits 101 if out of bounds."
            }
            Self::VecPop => {
                "vec_pop<T>(v: vec<T>) -> T\nRemove + return the last element. Exits 101 on empty."
            }
            Self::Args => "args() -> vec<str>\nCommand-line arguments, program name first.",
            Self::Env => "env(name: str) -> str\nEnvironment variable value, or \"\" when unset.",
            Self::ReadFile => {
                "read_file(path: str) -> Result<str, str>\nWhole file contents; Err(msg) on OS failure."
            }
            Self::WriteFile => {
                "write_file(path: str, data: str) -> Result<bool, str>\nCreate/truncate and write; Ok(true) on success."
            }
            Self::ReadStdin => "read_stdin() -> str\nAll of stdin until EOF.",
            Self::Exec => {
                "exec(cmd: str) -> i64\nSpawn a program (whitespace-split args, no shell), wait, return its exit code. -1 if spawn fails."
            }
            Self::StrFromByte => {
                "str_from_byte(b) -> str\nOne-byte string from the low 8 bits of `b`."
            }
            Self::StrFromFloat => {
                "str_from_f64(x: f64) -> str\nFixed %.6f formatting: 3.5 -> \"3.500000\", -0.0 -> \"-0.000000\", huge -> \"inf\", NaN -> \"nan\"."
            }
            Self::F64FromInt => {
                "f64_from_int(n) -> f64\nSigned integer to f64 conversion (any int width)."
            }
        }
    }

    /// Source-level name (`println`, `exit`, `vec_push`, …).
    pub fn name(self) -> &'static str {
        match self {
            Self::Print => "print",
            Self::Println => "println",
            Self::Eprint => "eprint",
            Self::Eprintln => "eprintln",
            Self::Exit => "exit",
            Self::Sqrt => "sqrt",
            Self::StrFromInt => "str_from_int",
            Self::StrFromBool => "str_from_bool",
            Self::StrGet => "str_get",
            Self::StrSlice => "str_slice",
            Self::VecNew => "vec_new",
            Self::VecPush => "vec_push",
            Self::VecGet => "vec_get",
            Self::VecSet => "vec_set",
            Self::VecPop => "vec_pop",
            Self::Args => "args",
            Self::Env => "env",
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::ReadStdin => "read_stdin",
            Self::Exec => "exec",
            Self::StrFromByte => "str_from_byte",
            Self::StrFromFloat => "str_from_f64",
            Self::F64FromInt => "f64_from_int",
        }
    }

    /// Symbol imported from `aura_runtime` — `""` for builtins emitted
    /// inline by codegen (`vec_new` zeroes storage; `vec_set` is
    /// `vec_get` + an element copy).
    pub fn runtime_symbol(self) -> &'static str {
        match self {
            Self::Print => "aura_rt_print",
            Self::Println => "aura_rt_println",
            Self::Eprint => "aura_rt_eprint",
            Self::Eprintln => "aura_rt_eprintln",
            Self::Exit => "aura_rt_exit",
            Self::Sqrt => "sqrt",
            Self::StrFromInt => "aura_str_from_int",
            Self::StrFromBool => "aura_str_from_bool",
            Self::StrGet => "aura_str_get",
            Self::StrSlice => "aura_str_slice",
            Self::VecPush => "aura_vec_push",
            Self::VecGet => "aura_vec_get",
            Self::Args => "aura_rt_args",
            Self::Env => "aura_rt_env",
            Self::ReadFile => "aura_read_file",
            Self::WriteFile => "aura_write_file",
            Self::ReadStdin => "aura_read_stdin",
            Self::Exec => "aura_exec",
            Self::StrFromByte => "aura_str_from_byte",
            Self::StrFromFloat => "aura_str_from_f64",
            Self::VecNew | Self::VecSet | Self::VecPop | Self::F64FromInt => "",
        }
    }

    /// Does the call take a `str` argument (passed as `{ptr, len}`)?
    pub fn takes_str(self) -> bool {
        matches!(
            self,
            Self::Print | Self::Println | Self::Eprint | Self::Eprintln
        )
    }
}

/// Namespaced error codes. Each compiler phase owns a range:
/// `E0xxx` lexer, `E1xxx` parser, `E2xxx` semantic, `E3xxx` codegen.
pub mod codes {
    // Lexer (E0xxx)
    pub const LEX_INVALID_CHAR: &str = "E0001";
    pub const LEX_UNTERMINATED_STRING: &str = "E0002";
    pub const LEX_UNTERMINATED_BLOCK_COMMENT: &str = "E0003";
    pub const LEX_INVALID_NUMBER: &str = "E0004";
    pub const LEX_INVALID_ESCAPE: &str = "E0005";
    pub const LEX_INVALID_UTF8: &str = "E0006";

    // Parser (E1xxx)
    pub const PARSE_UNEXPECTED_TOKEN: &str = "E1001";
    pub const PARSE_EXPECTED_EXPR: &str = "E1002";
    pub const PARSE_EXPECTED_IDENT: &str = "E1003";
    pub const PARSE_EXPECTED_TYPE: &str = "E1004";
    pub const PARSE_UNCLOSED_DELIMITER: &str = "E1005";
    pub const PARSE_EXPECTED_ITEM: &str = "E1006";
    pub const PARSE_MAX_DEPTH: &str = "E1007";
    pub const PARSE_BLOCK_IN_EXPR_POS: &str = "E1008";

    // Semantic (E2xxx)
    pub const SEM_UNDECLARED_VAR: &str = "E2001";
    pub const SEM_USE_BEFORE_INIT: &str = "E2002";
    pub const SEM_REDEFINITION: &str = "E2003";
    pub const SEM_MUTATE_IMMUTABLE: &str = "E2004";
    pub const SEM_CIRCULAR_DEP: &str = "E2010";
    pub const SEM_TYPE_MISMATCH: &str = "E2100";
    pub const SEM_IF_MISSING_ELSE: &str = "E2101";
    pub const SEM_ARG_COUNT: &str = "E2102";
    pub const SEM_NOT_CALLABLE: &str = "E2103";
    pub const SEM_NO_FIELD: &str = "E2104";
    pub const SEM_NOT_BOOL_CONDITION: &str = "E2105";
    pub const SEM_NON_EXHAUSTIVE_MATCH: &str = "E2106";
    pub const SEM_RETURN_TYPE: &str = "E2107";
    pub const SEM_UNKNOWN_TYPE: &str = "E2108";
    pub const SEM_UNKNOWN_VARIANT: &str = "E2109";
    pub const SEM_MISSING_FIELDS: &str = "E2110";
    pub const SEM_CANNOT_INFER: &str = "E2111";
    pub const SEM_EXTERN_AGGREGATE: &str = "E2112";

    // Codegen (E3xxx)
    pub const CG_INTERNAL: &str = "E3001";
    pub const CG_LINK_FAILED: &str = "E3002";
    pub const CG_LINKER_NOT_FOUND: &str = "E3003";
    pub const CG_UNSUPPORTED: &str = "E3004";
    pub const CG_MAIN_TYPE: &str = "E3005";
}
