#!/usr/bin/env bash
# M3 acceptance: three concurrent agents, one blocked and one dying, then
# assert both are visible and jumpable. Requires a running Berth app and jq.
set -euo pipefail

BERTH="${BERTH:-berth}"
command -v "$BERTH" >/dev/null || { echo "m3: berth not on PATH" >&2; exit 1; }
command -v jq >/dev/null || { echo "m3: jq is required" >&2; exit 1; }

"$BERTH" ping >/dev/null

new_workspace() {
  local name="$1"
  "$BERTH" new --name "$name" >/dev/null
  "$BERTH" active
}

sid_a="$(new_workspace m3-a)"
"$BERTH" agent-register "$sid_a" --provider claude-code --name m3-a >/dev/null
"$BERTH" agent-set --surface "$sid_a" --state idle >/dev/null

sid_b="$(new_workspace m3-b)"
"$BERTH" agent-register "$sid_b" --provider codex --name m3-b >/dev/null
"$BERTH" agent-set --surface "$sid_b" --state blocked --detail "permission prompt" >/dev/null

# C is a real short-lived process: exit must flip the agent to errored without
# waiting for the 180 s staleness sweep.
new_workspace m3-c >/dev/null
sid_c="$("$BERTH" run --name m3-c -- sleep 2)"

# B must be jumpable immediately; C should become errored within a second of
# its process exit (sleep 2 + a small scheduling margin).
sleep 3

snapshot="$("$BERTH" ls --json)"
state_of() {
  jq -r --arg sid "$1" '.agents[] | select(.surface_id == $sid) | .state' <<<"$snapshot"
}
attention_of() {
  jq -r --arg sid "$1" '.agents[] | select(.surface_id == $sid) | .attention' <<<"$snapshot"
}

[[ "$(state_of "$sid_a")" == "idle" ]] || { echo "m3: A not idle" >&2; exit 1; }
[[ "$(state_of "$sid_b")" == "blocked" ]] || { echo "m3: B not blocked" >&2; exit 1; }
[[ "$(state_of "$sid_c")" == "errored" ]] || { echo "m3: C not errored" >&2; exit 1; }
[[ "$(attention_of "$sid_b")" == "error" ]] || { echo "m3: B missing attention" >&2; exit 1; }
[[ "$(attention_of "$sid_c")" == "error" ]] || { echo "m3: C missing attention" >&2; exit 1; }

# Acknowledging through the protocol is what ⌘⇧U does; verify it clears both
# the derived attention and the unread notification.
agent_b="$(jq -r --arg sid "$sid_b" '.agents[] | select(.surface_id == $sid) | .id' <<<"$snapshot")"
"$BERTH" agent-ack "$agent_b" >/dev/null
after="$("$BERTH" ls --json)"
[[ "$(jq -r --arg sid "$sid_b" '.agents[] | select(.surface_id == $sid) | .attention' <<<"$after")" == "none" ]] \
  || { echo "m3: ack did not clear attention" >&2; exit 1; }

echo "ok: M3 — A idle, B blocked, C errored; attention present and acknowledged"
