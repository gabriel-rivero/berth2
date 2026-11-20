#!/bin/sh
# Record a symbolized Time Profiler trace of the window's launch path.
# Runs against a temporary data directory, so the real session is untouched.
# Usage: scripts/perf-trace.sh [seconds]
set -eu
cd "$(dirname "$0")/.."
seconds=${1:-5}
if ! command -v xcrun >/dev/null 2>&1; then
  echo "perf-trace: xcrun not found (macOS only)" >&2
  exit 1
fi
echo "==> building the profiling profile"
cargo build --manifest-path core/Cargo.toml --profile profiling -p berth-gui -p berth-cli >/dev/null
tmp=$(mktemp -d "${TMPDIR:-/tmp}/berth-perf.XXXXXX")
cat > "$tmp/run.sh" <<RUN
#!/bin/sh
export BERTH_DATA_DIR="$tmp"
export BERTH_SOCKET_PATH="$tmp/berth.sock"
export BERTH_CONFIG="$tmp/config.json"
export BERTH_TRACE=1
exec "$PWD/core/target/profiling/berth-gui" --socket "$tmp/berth.sock" --mode trusted --no-restore
RUN
chmod +x "$tmp/run.sh"
mkdir -p build
rm -rf build/berth-start.trace
echo "==> recording ${seconds}s"
status=0
xcrun xctrace record --template 'Time Profiler' \
  --output build/berth-start.trace --time-limit "${seconds}s" --launch -- "$tmp/run.sh" \
  || status=$?
# 54 is xctrace's "time limit reached", which is how the recording ends.
if [ "$status" -ne 0 ] && [ "$status" -ne 54 ]; then
  rm -rf "$tmp"
  exit "$status"
fi
rm -rf "$tmp"
echo "trace: build/berth-start.trace"
echo "open it with: open build/berth-start.trace"
