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

## Salsa discipline

- AST nodes never hold `String`/`&str` — interned `Spur`s only.
- Tracked results never contain `HashMap`/`HashSet` — `IndexMap` or
  sorted `Vec` keep iteration deterministic.

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
