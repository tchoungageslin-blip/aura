# Tutorials

Guided builds — each page walks one small but real program from
requirements to working code. Every `aura` block is compiled by CI,
so what you read is what runs.

| Tutorial | You build | You learn |
|----------|-----------|-----------|
| [CLI tool](cli-tool.md) | `grep-lite` — stdin filter with args | `args`, `read_stdin`, slicing, exit codes |
| [File processing](file-processing.md) | line counter writing a report | `read_file`, `write_file`, `match` on `Result` |
| [KV store](kv-store.md) | `SET/GET` server over stdin | tokenizing, parallel `vec`s, command loop |
| [Multi-file project](project.md) | app + `mathlib` dependency | `aura.toml`, `src/lib.aura`, packages |

French versions live under `fr/`: [tutoriels](fr/).

More complete programs: the [examples gallery](https://tchoungageslin-blip.github.io/aura/examples.html)
lists all 50 validation scenarios with full source.
