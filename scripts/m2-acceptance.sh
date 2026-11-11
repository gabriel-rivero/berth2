#!/usr/bin/env bash
# M2 acceptance: drive a running Berth app over its socket and prove the
# protocol spine end to end. Requires the app running with socket mode `local`
# or `trusted`, and `berth` on PATH (or BERTH_CLI set).
set -euo pipefail

CLI=${BERTH_CLI:-berth}
work=$(mktemp -d "${TMPDIR:-/tmp}/berth-m2.XXXXXX")
ws=""
pids=()

cleanup() {
    for pid in "${pids[@]:-}"; do
        kill "$pid" 2>/dev/null || true
    done
    if [[ -n "$ws" ]]; then
        "$CLI" close-workspace "$ws" >/dev/null 2>&1 || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

command -v "$CLI" >/dev/null || fail "berth CLI not found (set BERTH_CLI)"
"$CLI" ping >/dev/null || fail "cannot reach Berth; start the app first"

# Two independent clients, both replaying from sequence zero.
"$CLI" events --since 0 >"$work/events-a.ndjson" &
pids+=("$!")
"$CLI" events --since 0 >"$work/events-b.ndjson" &
pids+=("$!")
sleep 0.5

ws=$("$CLI" new --name m2-acceptance) || fail "workspace.create failed"
[[ "$ws" == ws_* ]] || fail "unexpected workspace id: $ws"

surface=$("$CLI" active)
[[ "$surface" == srf_* ]] || fail "no active surface after workspace.create"

right=$("$CLI" split "$surface" --direction right) || fail "first split failed"
[[ "$right" == srf_* ]] || fail "unexpected split id: $right"
down=$("$CLI" split "$right" --direction down) || fail "second split failed"
[[ "$down" == srf_* ]] || fail "unexpected split id: $down"

"$CLI" send "$down" "printf 'BERTH_M2_%s\\n' OK" || fail "send_text failed"
"$CLI" send-key "$down" enter || fail "send_key failed"
sleep 1.2

screen=$("$CLI" read "$down") || fail "read_text failed"
[[ "$screen" == *BERTH_M2_OK* ]] || fail "screen did not contain BERTH_M2_OK"

# Give both event streams a moment to receive the same batch, then stop them.
sleep 0.8
for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
wait "${pids[0]}" 2>/dev/null || true
wait "${pids[1]}" 2>/dev/null || true
pids=()

[[ -s "$work/events-a.ndjson" ]] || fail "client A saw no events"
[[ -s "$work/events-b.ndjson" ]] || fail "client B saw no events"
diff -u "$work/events-a.ndjson" "$work/events-b.ndjson" \
    || fail "the two clients saw different event streams"

# `seq` must be strictly increasing within a stream.
previous=0
count=0
for seq in $(sed -n 's/.*"seq":\([0-9][0-9]*\).*/\1/p' "$work/events-a.ndjson"); do
    if (( seq <= previous )); then
        fail "event sequence went backwards: $previous -> $seq"
    fi
    previous=$seq
    count=$((count + 1))
done
(( count > 0 )) || fail "no sequenced events were observed"

# Both clients must have observed the structural operations.
grep -q '"event":"surface.created"' "$work/events-a.ndjson" \
    || fail "surface.created missing from the event stream"
grep -q '"event":"workspace.created"' "$work/events-a.ndjson" \
    || fail "workspace.created missing from the event stream"

echo "PASS: workspace=$ws splits=$right,$down events=$count"
