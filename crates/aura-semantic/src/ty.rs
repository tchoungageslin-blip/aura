//! Semantic types for Aura — the resolved counterpart of
//! [`TypeName`](aura_salsa_db::TypeName).
//!
//! `Type` is `Eq`-comparable and fully owned (no arena ids), so it can live
//! inside salsa query outputs. Inference variables exist only inside
//! [`InferCtx`](crate::infer::InferCtx) during checking and are always
//! resolved away before a `Type` reaches a query result.

/// Integer width/signedness. `I64` is the default for unconstrained
/// integer literals (Rust defaults to `i32`; Aura chooses `i64`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntTy {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
}

impl IntTy {
    /// Width in bytes on the 64-bit targets Aura supports.
    /// (`isize`/`usize` are pointer-sized.)
    pub fn bytes(self) -> u32 {
        match self {
            IntTy::I8 | IntTy::U8 => 1,
            IntTy::I16 | IntTy::U16 => 2,
            IntTy::I32 | IntTy::U32 => 4,
            IntTy::I64 | IntTy::U64 | IntTy::Isize | IntTy::Usize => 8,
            IntTy::I128 | IntTy::U128 => 16,
        }
    }

    /// Whether the type is signed (`usize`/`isize` follow their
    /// fixed-width siblings).
    pub fn is_signed(self) -> bool {
        !matches!(
            self,
            IntTy::U8 | IntTy::U16 | IntTy::U32 | IntTy::U64 | IntTy::U128 | IntTy::Usize
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            IntTy::I8 => "i8",
            IntTy::I16 => "i16",
            IntTy::I32 => "i32",
            IntTy::I64 => "i64",
            IntTy::I128 => "i128",
            IntTy::Isize => "isize",
            IntTy::U8 => "u8",
            IntTy::U16 => "u16",
            IntTy::U32 => "u32",
            IntTy::U64 => "u64",
            IntTy::U128 => "u128",
            IntTy::Usize => "usize",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FloatTy {
    F32,
    F64,
}

impl FloatTy {
    pub fn name(self) -> &'static str {
        match self {
            FloatTy::F32 => "f32",
            FloatTy::F64 => "f64",
        }
    }
}

/// A resolved Aura type.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// Poison — produced after an error; unifies with everything to
    /// suppress cascading diagnostics.
    Error,
    /// `!` — diverging expressions (`return`, `break`, `loop {}`).
    Never,
    Unit,
    Bool,
    Int(IntTy),
    Float(FloatTy),
    /// String slice type (borrowed, `'static` for literals).
    Str,
    /// Struct type — payload is the item index into `FileItems`.
    Struct(u32),
    /// Enum type — payload is the item index into `FileItems`.
    Enum(u32),
    /// `fn(T, ..) -> R` — function/extern-fn signature as a value type.
    Fn {
        params: Vec<Type>,
        ret: Box<Type>,
    },
    Tuple(Vec<Type>),
    /// `*const T` / `*mut T` — raw pointer (FFI).
    Pointer {
        mutable: bool,
        pointee: Box<Type>,
    },
    /// Inference variable — internal to `InferCtx`, never in outputs.
    Var(u32),
}

impl Type {
    /// Short display name for diagnostics.
    pub fn display(&self, items: &aura_salsa_db::FileItems) -> String {
        match self {
            Type::Error => "<error>".into(),
            Type::Never => "!".into(),
            Type::Unit => "()".into(),
            Type::Bool => "bool".into(),
            Type::Int(i) => i.name().into(),
            Type::Float(f) => f.name().into(),
            Type::Str => "str".into(),
            Type::Struct(idx) | Type::Enum(idx) => items
                .items
                .get(*idx as usize)
                .and_then(|s| s.name())
                .unwrap_or("<?>")
                .to_owned(),
            Type::Fn { params, ret } => {
                let ps: Vec<String> = params.iter().map(|p| p.display(items)).collect();
                format!("fn({}) -> {}", ps.join(", "), ret.display(items))
            }
            Type::Tuple(ts) => {
                let ts: Vec<String> = ts.iter().map(|t| t.display(items)).collect();
                format!("({})", ts.join(", "))
            }
            Type::Pointer { mutable, pointee } => {
                format!(
                    "*{} {}",
                    if *mutable { "mut" } else { "const" },
                    pointee.display(items)
                )
            }
            Type::Var(v) => format!("?v{v}"),
        }
    }
}

/// Map a primitive type name to its `Type`, if it is one.
pub fn primitive(name: &str) -> Option<Type> {
    Some(match name {
        "i8" => Type::Int(IntTy::I8),
        "i16" => Type::Int(IntTy::I16),
        "i32" => Type::Int(IntTy::I32),
        "i64" => Type::Int(IntTy::I64),
        "i128" => Type::Int(IntTy::I128),
        "isize" => Type::Int(IntTy::Isize),
        "u8" => Type::Int(IntTy::U8),
        "u16" => Type::Int(IntTy::U16),
        "u32" => Type::Int(IntTy::U32),
        "u64" => Type::Int(IntTy::U64),
        "u128" => Type::Int(IntTy::U128),
        "usize" => Type::Int(IntTy::Usize),
        "f32" => Type::Float(FloatTy::F32),
        "f64" => Type::Float(FloatTy::F64),
        "bool" => Type::Bool,
        "str" => Type::Str,
        _ => return None,
    })
}
