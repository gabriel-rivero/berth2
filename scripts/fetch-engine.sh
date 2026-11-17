#!/usr/bin/env bash
# Fetch the CI-built Ghostty engine for the pinned submodule commit.
#
# scripts/bootstrap.sh compiles GhosttyKit, which needs Xcode's Metal shader
# compiler. A machine with only the Command Line Tools cannot do that, so this
# pulls the framework and runtime resources that CI already built for the same
# engine commit. Prefer bootstrap.sh when Xcode is installed: it is the same
# artifact built from this machine's SDK.
set -euo pipefail

cd "$(dirname "$0")/.."

FRAMEWORK="vendor/ghostty/macos/GhosttyKit.xcframework"
RESOURCES="vendor/ghostty/zig-out/share/ghostty"

if [[ -d "$FRAMEWORK" && -d "$RESOURCES" && "${FORCE:-0}" != "1" ]]; then
  echo "fetch-engine: $FRAMEWORK already present (FORCE=1 to replace)"
  exit 0
fi

git submodule status vendor/ghostty >/dev/null 2>&1 || {
  echo "fetch-engine: submodule not initialized; run git submodule update --init vendor/ghostty" >&2
  exit 1
}
SHA="$(git rev-parse HEAD:vendor/ghostty)"
NAME="ghostty-engine-$SHA"

echo "==> looking for artifact $NAME"
RUN="$(gh api "repos/{owner}/{repo}/actions/artifacts?name=$NAME&per_page=1" \
  --jq '.artifacts[0].workflow_run.id // empty' 2>/dev/null || true)"
if [[ -z "$RUN" ]]; then
  echo "fetch-engine: no artifact $NAME yet." >&2
  echo "              CI uploads it from a push to main; check 'gh run list --branch main'." >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
echo "==> downloading from run $RUN"
gh run download "$RUN" --name "$NAME" --dir "$TMP"

mkdir -p "$(dirname "$FRAMEWORK")" "$(dirname "$RESOURCES")"
rm -rf "$FRAMEWORK" "$RESOURCES"
cp -R "$TMP/macos/GhosttyKit.xcframework" "$FRAMEWORK"
cp -R "$TMP/zig-out/share/ghostty" "$RESOURCES"

echo "ok: $FRAMEWORK"
echo "ok: $RESOURCES"
echo
echo "next: make core-lib && make app"
