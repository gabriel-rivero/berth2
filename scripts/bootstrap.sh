#!/usr/bin/env bash
# One-time (and per-Ghostty-bump) setup: fetch the pinned engine and build it.
# macOS only. Everything lands in vendor/ghostty; nothing is installed globally.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "bootstrap: this must run on macOS (GhosttyKit needs the macOS SDK)." >&2
  echo "           The Rust core builds anywhere: cd core && cargo test" >&2
  exit 1
fi

echo "==> submodule"
git submodule update --init --recursive vendor/ghostty

WANT="$(grep -o 'minimum_zig_version = "[^"]*"' vendor/ghostty/build.zig.zon | head -1 | cut -d'"' -f2)"
echo "    ghostty $(git -C vendor/ghostty describe --tags 2>/dev/null || echo '(unknown)'), needs zig >= ${WANT:-0.17.0}"

echo "==> zig"
ZIG="${ZIG:-$(command -v zig || true)}"
if [[ -z "${ZIG}" && -x "$HOME/.local/bin/zig" ]]; then ZIG="$HOME/.local/bin/zig"; fi
if [[ -z "${ZIG}" ]]; then
  echo "bootstrap: zig not found. Install ${WANT:-0.17.x} from https://ziglang.org/download/" >&2
  echo "           and put it on PATH (or set ZIG=/path/to/zig)." >&2
  exit 1
fi
HAVE="$("$ZIG" version)"
echo "    using $ZIG ($HAVE)"
if [[ "$(printf '%s\n%s\n' "$WANT" "$HAVE" | sort -V | head -1)" != "$WANT" ]]; then
  echo "bootstrap: zig $HAVE is older than required $WANT" >&2
  exit 1
fi

echo "==> building GhosttyKit (this takes a few minutes the first time)"
pushd vendor/ghostty >/dev/null
"$ZIG" build -Doptimize=ReleaseFast -Dapp-runtime=none -Demit-xcframework=true install
popd >/dev/null

FRAMEWORK="vendor/ghostty/macos/GhosttyKit.xcframework"
RESOURCES="vendor/ghostty/zig-out/share/ghostty"
for path in "$FRAMEWORK" "$RESOURCES"; do
  if [[ ! -e "$path" ]]; then
    echo "bootstrap: expected $path — build succeeded but output is missing." >&2
    exit 1
  fi
done

echo
echo "ok: $FRAMEWORK"
echo "ok: $RESOURCES"
echo
echo "next: make build && make run"
