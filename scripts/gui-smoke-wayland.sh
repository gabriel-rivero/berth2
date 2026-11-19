#!/bin/sh
# Render the portable window under a headless Weston compositor. This is the
# only scripted Wayland check; X11 is covered by gui-smoke.sh.
# Usage: scripts/gui-smoke-wayland.sh [path-to-berth-gui] [png-out]
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bin=${1:-"$root/core/target/release/berth-gui"}

if [ ! -x "$bin" ]; then
    echo "gui-smoke-wayland: missing $bin (run: make gui)" >&2
    exit 2
fi
if ! command -v weston >/dev/null 2>&1; then
    echo "gui-smoke-wayland: weston not installed; skipping" >&2
    exit 0
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/berth-gui-wayland.XXXXXX")
runtime="$tmp/run"
mkdir -p "$runtime"
chmod 700 "$runtime"
png=${2:-"$tmp/frame.png"}
export XDG_RUNTIME_DIR="$runtime"
weston --backend=headless-backend.so --socket=wayland-berth --idle-time=0 \
    >"$tmp/weston.log" 2>&1 &
weston_pid=$!
trap 'kill $weston_pid 2>/dev/null || true; rm -rf "$tmp"' EXIT INT TERM

i=0
while [ "$i" -lt 50 ]; do
    [ -S "$runtime/wayland-berth" ] && break
    sleep 0.2
    i=$((i + 1))
done
if [ ! -S "$runtime/wayland-berth" ]; then
    echo "gui-smoke-wayland: weston did not start" >&2
    cat "$tmp/weston.log" >&2 || true
    exit 1
fi

echo "gui-smoke-wayland: $bin"
WAYLAND_DISPLAY=wayland-berth WINIT_UNIX_BACKEND=wayland \
    BERTH_DATA_DIR="$tmp" BERTH_SOCKET_PATH="$tmp/berth.sock" \
    BERTH_CONFIG="$tmp/config.json" \
    "$bin" --mode trusted --no-restore --smoke-ms 1500 --smoke-png "$png"

if [ ! -s "$png" ]; then
    echo "gui-smoke-wayland: no frame written to $png" >&2
    exit 1
fi
echo "gui-smoke-wayland: frame $png"
