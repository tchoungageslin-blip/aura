# The Megaplan

Distilled from the original *Plan d'Implémentation Stratégique
Exhaustif* — see `ROADMAP.md` for the live tracker.

## Phases

- **Phase 0 — Foundation.** Cargo workspace (12 crates), ariadne
  diagnostics, CI, quality gates.
- **Phase 1 — Frontend.** Lexer with lasso interning; arena AST;
  Pratt expressions + recursive-descent statements; panic-mode
  recovery; ASI.
- **Phase 2 — Semantics.** Salsa query database; name resolution with
  scopes/DefIds; bidirectional type checker over union-find inference.
- **Phase 3 — Backend.** HIR/MIR lowering; Cranelift COFF/ELF object
  emission; freestanding `aura_runtime` staticlib; `LinkerDriver`
  (lld-link / MSVC / mold); `aura build`/`run`.
- **Phase 4 — Data types.** Structs; enums + exhaustive `match`;
  built-in `Result<T,E>` + `?`; `extern "C"` blocks.
- **Phase 5 — Tooling.** `aura fmt` (canonical, idempotent); `aura lsp`
  (diagnostics, hover, completion, definition, references,
  semanticTokens); VS Code extension.
- **Phase 6 — Hardening.** cargo-fuzz targets; `aura-interp` reference
  interpreter + differential tests; `aura.toml` manifests + path deps;
  prelude builtins (`io`, `vec`, `process`, `env`); `str` end-to-end.
- **Phase 7 — Release.** Benchmark suite (n-body, binary-trees, fib,
  strings) + `aura bench`; documentation; `0.1.0-alpha` binaries.

## Engineering invariants

These are non-negotiable and guarded by tests/lints:

- No `String`/`&str` inside AST nodes — `Spur` only.
- No `HashMap`/`HashSet` inside Salsa tracked results.
- No C varargs calls from generated code — fixed-signature wrappers.
- Struct literals banned in expression-head position
  (`if`/`while`/`match` scrutinee).
- `if` used as a value requires `else`; `!` subtypes everything.
- Binary ops banned directly after unparenthesized blocks.
- ARC destruction uses an explicit worklist — never recursive
  `decRef` chains.
- At least half of parser tests exercise malformed input.
- `final` stays an ordinary identifier.
- The reference interpreter and compiled output must agree.
