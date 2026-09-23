# The Toolchain

The `aura` binary is the whole frontend. With no path argument it
discovers the enclosing project by walking up to `aura.toml`.

```sh
aura check [path]   # parse + resolve + type-check; render diagnostics
aura parse <file>   # dump the AST (s-expression)
aura mir   [path]   # dump the MIR for every function
aura build [path]   # compile + link → <project>/build/<name>[.exe]
aura run   [path]   # compile, link into a temp dir, run, forward exit code
aura interp [path]  # run main on the reference interpreter (no codegen)
aura bench [path]   # time N runs of compiled vs interp; verify parity
                    #   -i/--iters N  (default 3)
aura fmt   [path]   # canonical formatter — -c/--check, -s/--stdout
aura lsp            # LSP server over stdio
```

`check`, `mir`, `build`, `run`, `interp`, `bench`, and `fmt` all accept
a `.aura` file, a directory containing `aura.toml`, or nothing
(current-directory discovery). `parse` takes a file.

## Projects: `aura.toml`

```toml
[package]
name = "app"

[dependencies]
util = { path = "../util" }
```

- The entry unit is `src/main.aura` for binaries (the manifest name
  becomes the output binary name, written to `build/`).
- Path dependencies are discovered transitively; cycles, missing
  manifests, and name mismatches are reported precisely.
- All files of a project share one flat namespace — dep items are
  visible everywhere; `use` is for documentation intent.
- Diagnostics attribute to the file they occurred in, including inside
  dependencies.

## Formatter

`aura fmt` prints the canonical form (4-space indent, trailing commas
in multiline literals). `--check` exits nonzero when a file isn't
canonical; `--stdout` writes to stdout instead of in place. On a
project it formats every source unit.

## LSP + VS Code

`aura lsp` speaks LSP over stdio: diagnostics, hover, goto-definition,
completion, references, semantic tokens, and document symbols —
powered by the same Salsa queries as the CLI.

`editors/vscode` contains the VS Code extension (TextMate grammar +
LSP client wired to `aura lsp`).

## Linking

`aura build`/`run` emit a COFF (Windows) or ELF (Linux) object via
Cranelift, then link through `LinkerDriver`:

- **Windows** — `lld-link` from the Rust toolchain, falling back to
  MSVC `link.exe`, against `runtime/target/.../aura_runtime.lib`.
- **Linux** — mold → ld.lld → cc.

## Benchmarks

`aura bench bench/nbody.aura -i 3` produces:

```text
bench bench/nbody.aura
  compiled: 3 runs, min 38.4ms avg 60.8ms (exit Some(0))
  interp:   3 runs, min 1648.4ms avg 1952.4ms (exit Some(0))
  exit codes agree
```

`bench/` contains the reference suite: `fib`, `strings`, `bintree`,
`nbody`.
