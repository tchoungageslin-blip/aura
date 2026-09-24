# The Web Plan

Second megaplan: turn the Aura site into a complete A→Z learning
platform. Executed 2025 — all steps shipped and live at
<https://tchoungageslin-blip.github.io/aura/>.

## Goals

- Everything learnable without leaving the site: install → guided
  path → tutorials → reference → 50 real programs → error index.
- Bilingual EN/FR across static pages, generated gallery, and docs.
- Zero drift: example code is generated from the same `testsuite/`
  files that `aura test` runs; docs code blocks are compiled by the
  `docs_examples` gate.
- Continuous deployment: the site republishes on every relevant
  `master` push, not only on release tags.

## Steps

### S1 — i18n foundation

- `site/i18n.js`: inline `len`/`lfr` spans toggled by `EN | FR` nav
  button; language persisted in `localStorage` under `aura-lang`;
  `lang` attribute and `<title>` updated on switch.
- CSS `.len`/`.lfr` display rules; `index.html`, `learn.html`,
  `download.html`, `errors.html` fully bilingual.

### S2 — examples.html generator

- `build-site.ps1` section 2b walks `testsuite/*/`, reads each
  program's header comment as the English description, pulls French
  descriptions from `site/examples-fr.json` (UTF-8 JSON, read with
  `-Encoding UTF8` — PS 5.1-safe), groups the 50 scenarios into 8
  categories, and emits one collapsible `<details>` block per program
  with syntax-highlighted code, listed input files, and a GitHub
  source link.
- Literals inside the PowerShell script itself use HTML entities so
  the script file stays pure ASCII.

### S3 — learn.html learning path

- Rewritten as a numbered A→Z path: install → `aura new` → language
  tour → first real tool (wordcount) → reading table → exercises →
  editor setup.
- Fixed a doc bug: `aura run` was described as interpreting; it
  compiles + links. `aura interp` is the interpreter.

### S4 — tutorials

- `docs/src/tutorials/` — four guided builds, EN + FR mirrors:
  `cli-tool` (grep-lite), `file-processing` (line counter),
  `kv-store` (stdin-driven key/value store), `project` (multi-package
  with `aura.toml` + `use`).
- Every `aura` block compiles standalone under the docs gate;
  cross-package calls are marked `aura,ignore`.
- Registered in `aura doc tutorials`.

### S5 — deployment split

- `.github/workflows/pages.yml`: build + deploy on `master` pushes
  touching `site/`, `docs/`, `testsuite/` (and manual dispatch).
- `release.yml` keeps only artifact publishing; its Pages job was
  removed.

### S6 — gates and publish

- Local build: mdBook 0.5.4 → `docs/`, generator → `dist/`, link
  checker — **zero broken links**.
- `cargo fmt`, `cargo clippy`, `docs_examples` green.
- Pushed `301da28..6347b11`; `pages.yml` run deployed; live checks:
  `/`, `/examples.html`, `/i18n.js`, `/docs/tutorials/` all 200.

## Known limitation

No in-browser playground — running Aura in the browser would require
compiling the toolchain to WASM. Download-first, like early Rust/Go.
