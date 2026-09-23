# Aura

A statically-typed compiled systems language with ARC memory management,
built in Rust on top of Salsa (incremental queries) and Cranelift (codegen).

Source files use the `.aura` extension.

## Status

Early development — see `docs/megaplan.md` for the full roadmap and
`ROADMAP.md` for the phase tracker.

| Phase | Scope | Status |
|-------|-------|--------|
| 0 | Workspace, diagnostics, CI | ✅ |
| 1 | Lexer + parser | ✅ |
| 2 | Salsa + name resolution + type checker | ✅ |
| 3 | HIR → MIR → Cranelift → linked PE binary | ✅ |
| 4–7 | Structs/enums/FFI, LSP, fuzzing, release | 📋 planned |

## Layout

```
crates/
  aura-common     Span, FileId, Diagnostic, ariadne renderer
  aura-lexer      Tokens + lasso string interning
  aura-ast        Arena-allocated AST (Vec-indexed AstId)
  aura-parser     Pratt + recursive-descent parser, panic-mode recovery, ASI
  aura-salsa-db   Incremental query database
  aura-semantic   Name resolution + bidirectional type checker
  aura-hir        High-level IR
  aura-mir        Mid-level IR + ARC incRef/decRef injection
  aura-codegen    Cranelift backend behind CodegenBackend trait
  aura-linker     LinkerDriver abstraction (lld-link / MSVC / mold / cc)
  aura-runtime    staticlib: ARC + io exports (extern "C", fixed arity)
  aura-cli        `aura` binary: build / run / check
```

## Building

Requires Rust stable (MSVC toolchain on Windows). See `rust-toolchain.toml`.

```sh
cargo build --workspace
cargo test --workspace
cargo run -p aura-cli -- check examples/hello.aura
cargo run -p aura-cli -- run examples/hello.aura
```

On Windows the produced binaries link with the `lld-link` bundled inside the
Rust toolchain, falling back to MSVC `link.exe`. On Linux the order is
mold → ld.lld → cc.
