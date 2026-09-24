# Releasing Aura

How to publish a release to friends: push to GitHub, tag, and let
`release.yml` build the installer, ZIP, checksums and the website.

## One-time GitHub setup

1. **Create the repository** (public — required for free GitHub Pages):
   <https://github.com/new> → name `aura`, Public, no README/license
   (the repo already has them).

2. **Push:**
   ```sh
   git remote add origin https://github.com/<you>/aura.git
   git push -u origin master
   ```

3. **Enable Pages:** repo → Settings → Pages → Source:
   **GitHub Actions**. The workflow deploys `site/dist` there —
   the site lands at `https://<you>.github.io/aura`.

   If your repo name or org differs, update:
   - `AppURL` in `installer/aura.iss`
   - `aura docs` fallback URL in `crates/aura-cli/src/main.rs`
   - `repository.url` in `editors/vscode/package.json`
   - links in `site/*.html` and `docs/book.toml`

## Cutting a release

```sh
git tag v0.1.0-alpha
git push origin v0.1.0-alpha
```

The `release.yml` workflow then:

1. runs every gate — fmt, clippy, `cargo test --workspace`,
   the docs-as-code gate and the 50-scenario `aura test` suite;
2. builds `aura.exe` + `aura_runtime.lib`, bundles `lld-link.exe`;
3. packages the VS Code `.vsix`;
4. smoke-tests the dist on a bare PATH (proves no Rust needed);
5. compiles `AuraSetup-<version>.exe` with Inno Setup;
6. zips the portable dist, writes `SHA256SUMS.txt`;
7. creates a GitHub Release with all assets + generated notes;
8. builds the site and deploys it to GitHub Pages.

Watch it: repo → Actions → "Release". Assets appear under Releases.

## Manual release (no GitHub Actions)

```powershell
cargo test --workspace                       # gates
cargo run --release -p aura-cli -- test      # 50 scenarios
./scripts/package-dist.sh                   # dist/aura-<v>-<triple>/
# Inno Setup:  iscc installer\aura.iss      # installer/AuraSetup-*.exe
./scripts/build-site.ps1                    # site/dist/ (needs mdbook)
```

Then upload `AuraSetup-*.exe`, the ZIP and `SHA256SUMS.txt` to a
release created by hand on GitHub.

## After release

- Bump `version` in `Cargo.toml` (workspace) and `AURA_VERSION` in
  `release.yml`.
- Keep `docs/src/errors.md` and the changelog sections current — the
  docs gate fails the build if examples drift.
