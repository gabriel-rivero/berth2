# Berth — Vision

> A terminal workspace where agents are first-class objects, not stray strings in a tab title.

## 1. The one-liner

Berth is a native macOS terminal multiplexer for people who run several coding agents at once. It gives you Ghostty-grade terminals, a split grid, and one thing cmux does not: a **typed, observable agent lifecycle** with a push-based event protocol.

## 2. User and moment

Primary user: me. A developer running 2–8 concurrent agent sessions (Claude Code, Codex, opencode) across 3–6 workspaces, switching context constantly, and losing track of *which one is waiting on me*.

The moment that matters:

> I come back from lunch. Three agents finished, one is blocked on a permission prompt, one crashed. I want to see that at a glance, jump to the blocked one, and know what it was doing — without reading five terminal buffers.

cmux solved the "sidebar next to terminals" shape. It did not solve trust in the state shown there: its status is free-form strings pushed by hooks, its agent lifecycle is *inferred* from journals and heuristics, and there is no way to subscribe to changes — integrations poll.

## 3. Principles

1. **Terminals stay terminals.** Berth is not an orchestrator. Anything that runs in a TTY runs here, untouched, with no wrapper required. Agent features decay gracefully to nothing when unused.
2. **The agent is a schema, not a string.** An agent surface has provider, lifecycle state, detail, progress, attention, and resume data. Unknown agents are explicitly `unknown`, never guessed.
3. **Push, don't poll.** Every state change is an ordered event on a subscription. Polling is what the client does when it isn't listening.
4. **Attention over information.** The UI answers one question loudly: *who needs me now?* Everything else is quiet, small, and monochrome.
5. **Local, single process, no accounts.** No cloud, no sync, no telemetry, no login. One app, one socket, one config file.
6. **Portable where it counts.** The domain logic and the protocol are not macOS artifacts. The shell is.

## 4. Non-goals for v1

Explicitly out of scope, so they stop being tempting:

- Embedded browser panes, cookie import, agent-driven web automation
- SSH workspaces, remote tmux mirroring, cloud VMs
- iOS companion, push notifications, team/multiplayer features
- Plugin marketplaces, TUI mode, custom command DSLs
- Windows/Linux builds (the *design* keeps that door open; the code does not open it yet)
- Feature parity with cmux. Roughly a fifth of cmux's surface is the product; copying it is how this fails.

## 5. Differentiators

| Axis | cmux | Berth v1 |
| --- | --- | --- |
| Agent model | free-form status strings + hooks, lifecycle inferred | typed `AgentSurface` with a defined state machine |
| Integration | CLI calls, request/response socket, polling | NDJSON JSON-RPC with subscriptions, ordered events, replay |
| Restore | heuristics + journals + evidence files | explicit resume payload per agent, deterministic |
| Portability | macOS + Swift only, GPL | MIT, Rust domain core + wire protocol reusable off-macOS |
| UI | "ugly but indispensable" | restrained, keyboard-first, attention-driven |
| Process model | everything in one app, plus cloud tiers | one local process, no accounts, no telemetry |

The protocol is the real product. A UI can be re-skinned; a contract that agents and editors speak is what makes the tool stick.

## 6. What "done" means for v1

Personal daily driver, judged by:

- I use it as my only terminal for two consecutive weeks and don't open Ghostty/iTerm "just to get something done".
- Cold start to interactive first frame under 150 ms with a restored 3-workspace session.
- Keystroke-to-paint latency indistinguishable from stock Ghostty in the same session.
- Restoring a session reproduces every workspace, split, tab, cwd and agent with zero manual repair, including after a crash.
- Every agent I actually run shows a correct state without me configuring anything per-project (via wrapper), and states get *better*, not different, when I add hooks.
- Zero polling anywhere in the stack; `berth events --json` is enough to drive an external dashboard.

## 7. Naming

Working name **Berth** (a place where a vessel ties up; the thing next to the terminal). Bundle `dev.<user>.Berth`, CLI `berth`, socket `berth.sock`, env prefix `BERTH_`. Renaming later is a config change if we keep one constant module per layer.

## 8. Risks to the vision

- **GhosttyKit is not a stable public API.** It is the app's internal C surface. Mitigation: pin a Ghostty tag, keep every C call behind one adapter file, budget a day per Ghostty upgrade.
- **Agent ecosystems move fast.** Claude Code hooks, Codex, opencode all change shape monthly. Mitigation: the core owns a small adapter table; providers are data, not code paths.
- **Attention UI is easy to make annoying.** Mitigation: one escalation ladder, aggressive dedupe, per-workspace mute, and no sound in v1.
