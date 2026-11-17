#!/bin/sh
# Render the minimal window under Xvfb for a headless smoke check.
# Usage: scripts/gui-smoke.sh [path-to-berth-gui]
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bin=${1:-"$root/core/target/release/berth-gui"}

if [ ! -x "$bin" ]; then
    echo "gui-smoke: missing $bin (run: make gui)" >&2
    exit 2
fi
if ! command -v xvfb-run >/dev/null 2>&1; then
    echo "gui-smoke: xvfb-run not installed; skipping" >&2
    exit 0
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/berth-gui-smoke.XXXXXX")
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "gui-smoke: $bin"
xvfb-run -a --server-args="-screen 0 1280x800x24" \
    env BERTH_DATA_DIR="$tmp" BERTH_SOCKET_PATH="$tmp/berth.sock" \
    "$bin" --mode trusted --no-restore --smoke-ms 1200
