#!/usr/bin/env bash
# Regenerate the UniFFI Swift bindings into app/Sources.
#
# Bindings are committed: the app build must not need uniffi-bindgen, and the
# generated diff is the review surface when the FFI changes. CI fails if they
# are stale (see .github/workflows/ci.yml).
set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> building berth-ffi"
cargo build --manifest-path core/Cargo.toml -p berth-ffi

LIB=""
for candidate in \
  core/target/debug/libberth_ffi.so \
  core/target/debug/libberth_ffi.dylib; do
  if [[ -f "$candidate" ]]; then LIB="$candidate"; break; fi
done
[[ -n "$LIB" ]] || { echo "bindings: no berth-ffi library found" >&2; exit 1; }

OUT="$(mktemp -d)"
trap 'rm -rf "$OUT"' EXIT

echo "==> uniffi-bindgen ($LIB)"
# uniffi-bindgen shells out to `cargo metadata`, so run it from the workspace root.
LIB_ABS="$(cd "$(dirname "$LIB")" && pwd)/$(basename "$LIB")"
(
  cd core
  cargo run --quiet --features cli -p berth-ffi --bin uniffi-bindgen -- \
    generate --library "$LIB_ABS" --language swift --out-dir "$OUT"
)

mkdir -p app/Sources/BerthCore app/Sources/berth_ffiFFI/include
cp "$OUT/berth_ffi.swift" app/Sources/BerthCore/berth_ffi.swift
cp "$OUT/berth_ffiFFI.h" app/Sources/berth_ffiFFI/include/berth_ffiFFI.h
cp "$OUT/berth_ffiFFI.modulemap" app/Sources/berth_ffiFFI/include/module.modulemap

echo "ok: app/Sources/BerthCore/berth_ffi.swift"
echo "ok: app/Sources/berth_ffiFFI/include/{berth_ffiFFI.h,module.modulemap}"
