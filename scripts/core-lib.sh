#!/usr/bin/env bash
# Build the Rust core as a static library for the host architecture.
# SwiftPM links it via app/Package.swift; the output is gitignored.
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "core-lib: this builds an Apple static library and needs macOS." >&2
  echo "          On Linux run: cd core && cargo test --workspace" >&2
  exit 1
fi

case "$(uname -m)" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *) echo "core-lib: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac

if command -v rustup >/dev/null && ! rustup target list --installed | grep -qx "$TARGET"; then
  echo "==> adding rust target $TARGET"
  rustup target add "$TARGET"
fi

echo "==> cargo build -p berth-ffi --release ($TARGET)"
cargo build --manifest-path core/Cargo.toml -p berth-ffi --release --target "$TARGET"

LIB="core/target/$TARGET/release/libberth_ffi.a"
[[ -f "$LIB" ]] || { echo "core-lib: expected $LIB" >&2; exit 1; }
echo "ok: $LIB"
