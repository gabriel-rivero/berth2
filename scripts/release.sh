#!/usr/bin/env bash
# Cut a Berth release: bump versions, run the checks, commit, tag, and build
# the local platform's artifacts into ~/Downloads.
#
# Usage: scripts/release.sh <version> [--no-build] [--build-only] [--no-push]
set -euo pipefail

cd "$(dirname "$0")/.."

usage() {
  cat >&2 <<'EOF'
usage: scripts/release.sh <version> [--no-build] [--build-only] [--no-push]

  version      semver, for example 0.15.1
  --no-build   bump, test, commit and tag only
  --build-only build and copy artifacts for the current version
  --no-push    keep the tag local instead of pushing it
EOF
}

VERSION=""
DO_BUILD=1
BUILD_ONLY=0
DO_PUSH=1
for arg in "$@"; do
  case "$arg" in
    --no-build) DO_BUILD=0 ;;
    --build-only) BUILD_ONLY=1 ;;
    --no-push) DO_PUSH=0 ;;
    --push) DO_PUSH=1 ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "release: unknown option: $arg" >&2
      usage
      exit 2
      ;;
    *)
      if [ -n "$VERSION" ]; then
        echo "release: give exactly one version" >&2
        exit 2
      fi
      VERSION="$arg"
      ;;
  esac
done

CURRENT="$(sed -n 's/^version = "\([0-9][0-9.]*\)"$/\1/p' core/Cargo.toml | head -n 1)"
if [ -z "$CURRENT" ]; then
  echo "release: could not read the workspace version" >&2
  exit 1
fi

if [ "$BUILD_ONLY" -eq 1 ]; then
  VERSION="${VERSION:-$CURRENT}"
  if [ "$VERSION" != "$CURRENT" ]; then
    echo "release: --build-only builds $CURRENT, not $VERSION" >&2
    exit 1
  fi
else
  if [ -z "$VERSION" ]; then
    usage
    exit 2
  fi
  if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "release: '$VERSION' is not semver (for example 0.15.1)" >&2
    exit 2
  fi
  if [ -n "$(git status --porcelain)" ]; then
    echo "release: commit or stash the worktree first" >&2
    git status --short >&2
    exit 1
  fi
  if git rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null; then
    echo "release: tag v$VERSION already exists" >&2
    exit 1
  fi
  if [ "$CURRENT" = "$VERSION" ]; then
    echo "release: the workspace is already at $VERSION" >&2
    exit 1
  fi
  if ! awk '/^## Unreleased$/{found=1; next} found && /^## /{exit} found' CHANGELOG.md |
    grep -q '^### '; then
    echo "release: CHANGELOG.md has no Unreleased section with content" >&2
    exit 1
  fi

  echo "==> $CURRENT -> $VERSION"
  perl -pi -e "s/^version = \"\Q$CURRENT\E\"/version = \"$VERSION\"/" core/Cargo.toml
  perl -pi -e "s/^pkgver=.*/pkgver=$VERSION/" packaging/arch/PKGBUILD
  DATE="$(date +%Y-%m-%d)"
  perl -0pi -e "s/^## Unreleased$/## $VERSION — $DATE/m" CHANGELOG.md

  echo "==> updating Cargo.lock"
  (cd core && cargo check --workspace --quiet)

  echo "==> formatting"
  (cd core && cargo fmt --all -- --check)

  echo "==> tests"
  (cd core && cargo test --workspace)

  echo "==> lint"
  (cd core && cargo clippy --workspace --all-targets)

  git add CHANGELOG.md core/Cargo.toml core/Cargo.lock packaging/arch/PKGBUILD
  git commit -m "release: $VERSION"
  git tag -a "v$VERSION" -m "Berth $VERSION"
  echo "==> tagged v$VERSION"
fi

DOWNLOADS="$HOME/Downloads"
mkdir -p "$DOWNLOADS"

stage_engine() {
  local framework="vendor/ghostty/macos/GhosttyKit.xcframework"
  local resources="vendor/ghostty/zig-out/share/ghostty"
  if [ -f "$framework/Info.plist" ] && [ -d "$resources" ]; then
    return
  fi
  local primary
  primary="$(git worktree list --porcelain | sed -n '1s/^worktree //p')"
  if [ -n "$primary" ] && [ -f "$primary/$framework/Info.plist" ] &&
    [ -d "$primary/$resources" ]; then
    echo "==> staging the Ghostty engine from $primary"
    mkdir -p "$(dirname "$framework")" "$(dirname "$resources")"
    rm -rf "$framework" "$resources"
    ditto "$primary/$framework" "$framework"
    ditto "$primary/$resources" "$resources"
    return
  fi
  echo "release: the Ghostty engine is missing; run scripts/fetch-engine.sh" >&2
  return 1
}

copy_bundle() {
  local src="$1" name="$2" exe="$3"
  local target="$DOWNLOADS/$name"
  if [ ! -d "$src" ]; then
    echo "release: missing $src" >&2
    return 1
  fi
  rm -rf "$target"
  ditto "$src" "$target"
  local got
  got="$(plutil -extract CFBundleShortVersionString raw "$target/Contents/Info.plist")"
  if [ "$got" != "$VERSION" ]; then
    echo "release: $name reports $got, expected $VERSION" >&2
    return 1
  fi
  codesign --verify --deep --strict "$target"
  echo "ok: $target ($(file -b "$target/Contents/MacOS/$exe"))"
}

build_macos() {
  stage_engine
  make app
  (cd core && cargo build --release -p berth-gui -p berth-cli)
  ./scripts/bundle-gui.sh release
  copy_bundle build/Berth.app Berth.app Berth
  copy_bundle build/BerthGui.app BerthGui.app berth-gui
}

build_linux() {
  (cd core && cargo build --release -p berth-gui -p berth-cli)
  local arch stage
  arch="$(uname -m)"
  stage="$(mktemp -d)"
  cp core/target/release/berth-gui core/target/release/berth "$stage/"
  tar -czf "$DOWNLOADS/berth-gui-$VERSION-linux-$arch.tar.gz" -C "$stage" .
  rm -rf "$stage"
  echo "ok: $DOWNLOADS/berth-gui-$VERSION-linux-$arch.tar.gz"
  if command -v dpkg-deb >/dev/null 2>&1; then
    case "$arch" in
      aarch64|arm64) arch=arm64 ;;
      *) arch=amd64 ;;
    esac
    ARCH="$arch" packaging/debian/build-deb.sh "$VERSION" core/target/release "$DOWNLOADS"
  fi
}

build_windows() {
  (cd core && cargo build --release -p berth-gui -p berth-cli)
  local stage zip
  stage="$(mktemp -d)"
  cp core/target/release/berth-gui.exe core/target/release/berth.exe "$stage/"
  zip="$DOWNLOADS/berth-gui-$VERSION-windows-x86_64.zip"
  if command -v 7z >/dev/null 2>&1; then
    (cd "$stage" && 7z a "$(cygpath -w "$zip")" .)
  else
    powershell.exe -NoProfile -Command \
      "Compress-Archive -Force -Path '$(cygpath -w "$stage")\\*' -DestinationPath '$(cygpath -w "$zip")'"
  fi
  rm -rf "$stage"
  echo "ok: $zip"
}

if [ "$BUILD_ONLY" -eq 1 ] && [ "$DO_BUILD" -ne 1 ]; then
  echo "release: --build-only always builds" >&2
  exit 2
fi

if [ "$DO_BUILD" -eq 1 ]; then
  case "$(uname -s)" in
    Darwin) build_macos ;;
    Linux) build_linux ;;
    MINGW*|MSYS*|CYGWIN*) build_windows ;;
    *)
      echo "release: no artifact build for $(uname -s); rerun with --no-build" >&2
      exit 1
      ;;
  esac
fi

if [ "$BUILD_ONLY" -eq 1 ]; then
  exit 0
fi

if [ "$DO_PUSH" -eq 1 ]; then
  branch="$(git rev-parse --abbrev-ref HEAD)"
  git push origin "refs/tags/v$VERSION"
  if [ "$branch" = "main" ]; then
    git push origin main
    echo "==> pushed v$VERSION and main"
  else
    echo "==> pushed v$VERSION; merge $branch to main and push main"
  fi
fi
