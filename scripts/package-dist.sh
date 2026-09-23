#!/usr/bin/env bash
# Package a self-contained aura release into dist/.
#
#   scripts/package-dist.sh [version] [target-triple]
#
# Produces dist/aura-<version>-<triple>/ containing aura[.exe] +
# aura_runtime.lib (found beside the exe at link time) + bench/ + README.
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
mkdir -p "$pkg/bench"
cp "target/release/aura$ext" "$pkg/"
cp "runtime/target/release/aura_runtime.lib" "$pkg/"
cp bench/*.aura "$pkg/bench/"
cp README.md "$pkg/"

echo "packaged $pkg"
