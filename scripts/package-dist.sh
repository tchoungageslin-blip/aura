#!/usr/bin/env bash
# Package a self-contained aura release into dist/.
#
#   scripts/package-dist.sh [version] [target-triple]
#
# Produces dist/aura-<version>-<triple>/ — a folder a user with NO Rust
# toolchain can copy anywhere and use: aura.exe + aura_runtime.lib +
# lld-link.exe (bundled linker, found beside the exe at link time) +
# bench/ + docs/ + licenses/ + README.txt + the .vsix if built.
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:-0.1.0-alpha}"
triple="${2:-$(rustc -vV | sed -n 's/^host: //p')}"

cargo build --release -p aura-cli
cargo build --release --manifest-path runtime/Cargo.toml

ext=""
[[ "$triple" == *windows* ]] && ext=".exe"
pkg="dist/aura-$version-$triple"
rm -rf "$pkg"
mkdir -p "$pkg/bench" "$pkg/licenses"

# Toolchain
cp "target/release/aura$ext" "$pkg/"
cp "runtime/target/release/aura_runtime.lib" "$pkg/"

# Bundled linker — pulled from the Rust sysroot so `aura build` works on
# machines without Rust (aura-linker searches beside the exe first).
# NB: `gcc-ld/lld-link.exe` is a thin wrapper that spawns `rust-lld.exe`;
# LLD selects its COFF/MSVC driver from argv[0], so the real binary
# renamed `lld-link.exe` IS lld-link.
sysroot="$(rustc --print sysroot)"
lld="$sysroot/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe"
[[ -f "$lld" ]] || { echo "error: rust-lld not found at $lld" >&2; exit 1; }
cp "$lld" "$pkg/lld-link.exe"

# User-facing content
cp packaging/README.txt "$pkg/README.txt"
cp bench/*.aura "$pkg/bench/"
cp LICENSE-MIT LICENSE-APACHE "$pkg/"
cp licenses/LICENSE.lld.txt "$pkg/licenses/"
[[ -d docs/src ]] && { cp -r docs/src "$pkg/docs"; }
for v in editors/vscode/*.vsix; do
  [[ -f "$v" ]] && cp "$v" "$pkg/"
done

echo "packaged $pkg"
