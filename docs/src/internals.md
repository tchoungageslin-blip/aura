# Compiler Internals

## Pipeline

```text
.aura ──▶ aura-lexer ──▶ aura-parser ──▶ aura-ast
                                         │ (arena, AstId-indexed)
                  aura-salsa-db ◀────────┘  incremental inputs/queries
                         │
              aura-semantic ──▶ resolve (DefId) + typeck (InferCtx)
                         │
              aura-hir ──▶ typed body bundle
                         │
              aura-mir ──▶ basic blocks, operands, ARC incRef/decRef
                         │
              aura-codegen ──▶ Cranelift ──▶ .obj (COFF/ELF)
                         │
              aura-linker ──▶ LinkerDriver ──▶ executable
```

`aura-interp` is a parallel consumer: it walks `ParsedFile` directly
after semantic checking and must produce identical observable behavior.

## Lexer (`aura-lexer`)

- Single pass over bytes; emits `Vec<Token>` where identifiers,
  keywords and cooked strings are `lasso::Spur`s into a `Rodeo`
  interner **owned by `LexedFile`** — nothing leaks, which matters
  because the same interner pattern lives inside the long-running LSP
  process.
- `\n` produces a `Newline` token; the parser's ASI rules decide when
  it ends a statement (never inside `()`/`{}`/`[]` or after an
  operator/comma).
- String literals are *cooked* during lexing (escapes resolved), so
  downstream passes see final bytes.
- All malformed input yields `E000x` diagnostics, never a panic.

## Parser (`aura-parser`)

- Iterative **Pratt** (binding-power) expression parser: `infix_bp`
  gives each operator `(l_bp, r_bp)`; right-associative `=` uses
  `r_bp == l_bp`, everything else is left-associative.
- **Panic-mode recovery**: on an error the parser emits a diagnostic,
  synchronizes at the next statement boundary, and inserts an
  `Expr::Error` placeholder so checking can continue.
- `MAX_DEPTH` guards against pathological nesting (E1007) — iterative
  or not, a hostile `((((…` input can't overflow the stack.
- Struct literals are contextually banned in expression-head position
  (`if S{..}.f == …`), resolved by the same trick Rust uses.

## Semantic (`aura-semantic`)

- Two passes: `resolve` builds a `Def` table (functions, structs,
  enum variants, builtins `Ok`/`Err`, prelude builtins), then `typeck`
  runs **bidirectional inference** over each body.
- `InferCtx` is a union-find over type variables tagged by
  `VarKind::{Int, Float, Any}`; unresolved vars render as
  `{integer}`/`{float}`/`{unknown}` in diagnostics — never internal
  `?v0` names.
- `Type::Never` (`!`) unifies with everything, so `exit(1)` and
  diverging branches type-check anywhere.

## MIR and ARC (`aura-mir`)

- Bodies lower to basic blocks over operands; aggregates move through
  the `{tag, payload}` / `{ptr, len(, cap)}` layouts described below.
- ARC scaffolding exists: `incRef`/`decRef` operations are represented
  in MIR, and `unsafe { }` blocks are exempt from injection — the
  escape hatch for manual memory work in a later phase.

## Salsa discipline

- AST nodes never hold `String`/`&str` — interned `Spur`s only.
- Tracked results never contain `HashMap`/`HashSet` — `IndexMap` or
  sorted `Vec` keep iteration deterministic.
- Early-cutoff: editing a function body doesn't re-typecheck
  unchanged callers (proven by a dedicated salsa test).

## Codegen and linking

- `aura-codegen` drives Cranelift: one module per compile, aggregates
  via sret, `i128` constants built with `iconcat`, division/remainder
  lower to raw `sdiv`/`udiv`/`srem`/`urem` — a zero divisor or
  `MIN / -1` is a hardware trap, not a checked exit.
- `aura-linker` abstracts `lld-link` (Windows), `mold` (Linux) and a
  generic `ld` fallback; on Windows it searches the bundled
  `lld-link.exe` beside `aura.exe` first, then PATH, then Rust
  toolchains.

## Multi-file

`project_items` merges every project file's `ItemSig`s into one global
table, so `Def::Fn(u32)` indices stay file-agnostic and every consumer
(resolution, typeck, MIR, codegen, interp) works unchanged across
project boundaries.

## ABI

- **Aggregates** (`str`, `vec`, structs, enums, tuples) are passed to
  and returned from internal Aura functions through hidden pointers
  (sret-style). Scalar args go in registers.
- `str` = `{ptr, len}` (16 bytes); `vec<T>` = `{ptr, len, cap}`
  (24 bytes); both stored inline inside structs.
- `extern "C"` is scalar-only — aggregates are rejected (`E2112`)
  because the C ABI path is not implemented.
- Runtime calls are fixed-signature (`aura_rt_*`, `aura_str_*`,
  `aura_vec_*`); `vec_*` helpers are byte-oriented and take the element
  size as a parameter, so the runtime never monomorphizes.

## Runtime

`runtime/` is a `#![no_std]` `staticlib` calling kernel32 via raw
dynamic linking (`HeapAlloc`/`HeapFree`/`HeapReAlloc`, `WriteFile`,
`ExitProcess`, `CommandLineToArgvW`, `GetEnvironmentVariableW`) plus a
libc-compatible `sqrt`. Generated objects import exactly the runtime
symbols their builtins reference.

## Interpreter

Tree-walking over the parsed AST. Runs on a dedicated 256 MB-stack
thread (deep Aura recursion is Rust recursion; the default Windows
main-thread stack is ~1 MB). A fuel counter (`STEP_LIMIT`) bounds
execution for fuzzing.

## Fuzzing

`fuzz/` (cargo-fuzz) targets the lexer, parser, semantic checker,
codegen, and the interpreter — differential coverage against compiled
output lives in the e2e suite instead (linking is too slow for a fuzz
harness).
