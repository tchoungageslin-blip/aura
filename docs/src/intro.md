# Introduction

Aura is a statically-typed compiled systems language. The toolchain is
written in Rust on top of Salsa (incremental queries) and Cranelift
(code generation), and produces native executables — PE/COFF on Windows,
ELF via mold/lld/cc on Linux.

Source files use the `.aura` extension.

```aura
fn main() -> i64 {
    println("Hello, World!")
    0
}
```

```sh
aura run hello.aura    # compile, link, and run
aura check hello.aura  # type-check only
aura interp hello.aura # run on the reference interpreter
```

## Design goals

- **Statically typed, inferred** — Hindley-Milner-style local inference;
  function signatures are always explicit.
- **Native output** — Cranelift generates real object files; a pluggable
  `LinkerDriver` produces executables.
- **Incremental** — every query from parse to type-check is a Salsa
  tracked function, so re-checking after an edit is cheap (this also
  powers the LSP).
- **Deterministic destruction** — memory is managed by ARC
  (automatic reference counting) injected at the MIR level, with an
  explicit worklist so destruction never recurses.
- **Verified by construction** — a tree-walk reference interpreter
  (`aura interp`) must agree with compiled output; the e2e suite is
  differential.

## Status

Pre-`0.1.0-alpha`. The language surface implemented so far: functions,
structs, enums + exhaustive `match`, `Result<T,E>` + `?`, `str`,
generic `vec<T>`, `extern "C"` FFI for scalar signatures, projects with
`aura.toml` + local path dependencies, and the prelude builtins listed
in [Standard Library](./stdlib.md).
