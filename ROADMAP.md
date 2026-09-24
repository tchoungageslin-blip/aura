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

### Phase 4 — Structs, enums, FFI
- struct decls/instantiation/field access + assignment, C-compatible layout
- enums + `match`; built-in `Result<T,E>` + `?`
- `extern "C"` decls + `unsafe`; `*const/*mut T` pointers
- `str` (`{ptr,len}`, `+`, `==`, `.len`/`.ptr`); `vec<T>` (`{ptr,len,cap}`,
  inline in aggregates, doubling growth, `vec_get/set/push/pop`)

### Phase 5 — Tooling
- `aura fmt` (4sp indent, 100col, idempotent)
- `aura lsp` via async-lsp + VS Code extension

### Phase 6 — Hardening
- cargo-fuzz targets; `aura-interp` reference interpreter (256 MB scoped
  stack for deep recursion; `RunConfig` injects stdin/cwd/argv)
- stdlib builtins: `str_from_int/bool/byte/f64` (`%.6f` exact),
  `f64_from_int`, `str_get/str_slice`, `vec_*`, `read_file`/`write_file`,
  `read_stdin`, `exec`, `args`, `env`, `sqrt`, `exit`
- `aura.toml` manifests + local path deps (flat alpha namespace)
- i128/u128 literals + arithmetic (constants via `iconcat`; E0004 on
  u128 overflow)

### Phase 7 — Release
- `aura bench` (compiled-vs-interp timing); `bench/` suite: fib, strings,
  bintree, nbody (compiled ~10–70× faster than interp)
- mdBook docs in `docs/`; `scripts/package-dist.sh` → verified
  `dist/aura-0.1.0-alpha-windows-x86_64/`
- Blocked by environment: linux x86_64/aarch64 binaries (no linux
  linker on this host), hyperfine-vs-C (no hyperfine/C toolchain)

### Validation ladder — `aura test` (50/50)
Progressive scenario corpus in `testsuite/`, run by `aura test` —
compiled and interpreted engines must agree byte-for-byte on stdout and
exit code; side effects isolated in a temp workdir copy; stop at first
failure; every fix adds a permanent regression test.

- L1 01–10 foundations (incl. i128 codegen — was E3004)
- L2 11–20 algorithms (added `str_get`/`str_slice`/`vec_pop`)
- L3 21–28 system: files, stdin, exec, env, args, copy, miniapp
  (added file/process builtins + `str_from_byte`; interp `RunConfig`)
- L4 29–35 performance (added `str_from_f64` `%.6f`, `f64_from_int`,
  scientific-notation literals)
- L5 36–40 scientific (integration, stats, Monte Carlo, linalg, iterates)
- L6 41–45 real apps (JSON parser, CLI, KV store, log processor,
  template engine; fixed no-else `if` with unit/Never tails)
- L7 46–48 multi-file `aura.toml` projects + path deps
- L8 49–50 regression corpus + determinism gate (fixed silent u64
  clamp on big literals — widened to u128)

## Engineering invariants (from megaplan — do not regress)

- No `String`/`&str` in AST nodes — `Spur` only (lasso frees on drop).
- No `HashMap`/`HashSet` inside Salsa tracked results — `IndexMap`/sorted `Vec`.
- No C varargs calls from generated code — fixed-signature runtime wrappers.
- Struct literals banned in expr-head position (`if`/`while`/`match` scrutinee).
- `if` used as a value requires `else`; `!` (Never) subtypes everything.
- Binary ops banned directly after unparenthesized blocks — require parens.
- ARC destruction via explicit worklist, never recursive `decRef` chains.
- ≥50% of parser tests must exercise malformed input.
