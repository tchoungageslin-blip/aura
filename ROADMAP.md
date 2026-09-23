# Aura Roadmap — Phase Tracker

Full strategy: `docs/megaplan.md`. This file tracks what is done.

## Completed

### Phase 0 — Infrastructure
- Cargo workspace, 12 crates, workspace lints (`unsafe_code = deny`)
- `aura-common`: `FileId`, `Span`, `Diagnostic`/`DiagnosticSink`, namespaced
  error codes, ariadne renderer
- GitHub Actions CI (test ×{windows,ubuntu}, clippy, fmt, audit, llvm-cov)
- `rust-toolchain.toml`, `.cargo/config.toml`

### Phase 1 — Front-end
- `aura-lexer`: full `TokenKind`, lasso `Rodeo` interning, nested block
  comments, `0x/0b/0o` + `_` separators, string escapes, Unicode idents,
  `Newline` tokens for ASI, error recovery (`Error` tokens, never panics)
- `aura-ast`: Vec-indexed arena (`AstId`), `Expr`/`Stmt`/`Item`/`Pattern`,
  `ErrorNode` variants, s-expr pretty printer
- `aura-parser`: iterative Pratt loop (left-assoc safe, depth cap 256),
  `Restrictions::NO_STRUCT_LITERAL`, panic-mode recovery with sync tokens,
  deterministic ASI rules, insta snapshot fixtures

### Phase 2 — Semantics
- `aura-salsa-db`: `SourceFile` input, tracked `lex/parse/resolve/type_check`,
  deterministic collections only (no `HashMap` in tracked results)
- `aura-semantic`: scope tree + `DefId`, `E2xxx` errors, `Type` incl. `Never`,
  union-find inference, bidirectional checker, coercion rules

### Phase 3 — Codegen
- `aura-hir`/`aura-mir`: lowering, basic blocks, SSA, ARC op injection scaffold
- `aura-codegen`: `CodegenBackend` trait + Cranelift impl → PE `.obj`
- `aura-linker`: `LinkerDriver` — lld-link → MSVC on Windows; mold → lld → cc on Linux
- `aura-runtime`: `staticlib`, `extern "C"` fixed-signature exports
- `aura run examples/hello.aura` prints `Hello, World!`

## Planned (not started)

### Phase 4 — Structs, enums, FFI
struct decls/instantiation/field access, C-compatible layout; enums +
exhaustive `match`; built-in monomorphized `Result<T,E>` + `?`;
`extern "C"` blocks + `unsafe` (no ARC inside unsafe).

### Phase 5 — Tooling
`aura fmt` (4sp indent, 100col, idempotent); `aura lsp` via async-lsp
(diagnostics, hover, completion, definition, references, semanticTokens);
VS Code extension (TextMate grammar + LSP client).

### Phase 6 — Hardening
cargo-fuzz targets (lexer/parser/semantic/codegen/e2e differential vs
reference interpreter `aura-interp`); stdlib (`io`, `string`, `vec`,
`process`, `env`); `aura.toml` manifests + local path deps.

### Phase 7 — Release
benchmark suite (n-body, binary-trees, fib, strings; hyperfine vs C −O2 ±15%);
mdBook docs; 0.1.0-alpha binaries for linux x86_64/aarch64 + windows x86_64.

## Engineering invariants (from megaplan — do not regress)

- No `String`/`&str` in AST nodes — `Spur` only (lasso frees on drop).
- No `HashMap`/`HashSet` inside Salsa tracked results — `IndexMap`/sorted `Vec`.
- No C varargs calls from generated code — fixed-signature runtime wrappers.
- Struct literals banned in expr-head position (`if`/`while`/`match` scrutinee).
- `if` used as a value requires `else`; `!` (Never) subtypes everything.
- Binary ops banned directly after unparenthesized blocks — require parens.
- ARC destruction via explicit worklist, never recursive `decRef` chains.
- ≥50% of parser tests must exercise malformed input.
