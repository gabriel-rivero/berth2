# Berth — Roadmap

Schedule assumes one developer, evenings + weekends, on an Apple Silicon Mac. Dates are *ordering*, not promises. Each milestone ends with a demoable app; no milestone may leave `main` unable to run a terminal.

## M0 — Foundations (≈2 weeks)

Goal: a window with a fast terminal in it.

- Repo scaffold, `vendor/ghostty` submodule pinned to a release tag, `scripts/bootstrap` (Zig pin, `zig build` → `GhosttyKit.xcframework`, FFI codegen).
- `GhosttyHost` adapter: `ghostty_init`, config load, `ghostty_app_t`, one `GhosttySurfaceView` with `CAMetalLayer`, tick/refresh loop, resize, content scale on display change.
- Key input → `ghostty_surface_key`, IME via `preedit`, clipboard, mouse selection, scroll.
- Ghostty config passthrough (theme/font), ad-hoc signing, `make run`.

**Done when:** `vim`, `htop`, `less` and a 4k-line `cat` look and feel identical to stock Ghostty; resize is artifact-free; closing the window kills children; idle CPU < 1 %.

## M1 — Workspace shell (≈3 weeks)

Goal: the cmux shape, without agent features.

- Split tree (n-ary, h/v, weights), per-pane tab stacks, focus model, keyboard nav (`⌘↩` split, `⌘[`/`⌘]` panes, `⌘1..9` workspaces).
- Sidebar: workspace rows with live title, cwd, git branch, listening ports, unread dot; groups; pinned; drag-reorder.
- Per-surface metadata pipeline from Ghostty actions: `SET_TITLE`, `PWD`, `PROGRESS_REPORT`, `RING_BELL`, `SHOW_CHILD_EXITED`.
- Persistence: `session.json` + journal, restore of windows/workspaces/tree/tabs/cwd.
- Command palette (fuzzy, ~10 verbs), shortcuts config, app settings window.

**Done when:** I can build a 3-workspace, 6-pane layout, quit, relaunch, and land exactly where I left off — including cwd — with no visible startup hitch.

## M2 — Core + protocol (≈3 weeks)

Goal: the product's spine.

- `berth-core` Rust crate: model, router, event bus, codec, error taxonomy, schema generation.
- UniFFI scaffolding + `SurfaceOps` callback implemented in Swift; `xcodebuild` phase runs `cargo build` + bindgen.
- Socket server: NDJSON framing, uid/token auth, subscriptions, replay ring buffer, backpressure disconnect.
- All v1 methods from `docs/03-protocol.md`; `berth` CLI (`ls`, `send`, `snapshot`, `events`).
- Env injection into children; headless test suite (spawn app in test mode, drive via socket).

**Done when:** a shell script can create a workspace, split twice, type into a pane, read the screen back, and a second client sees the same events with monotonic `seq` — with the UI showing exactly the same state.

Status: model/router/events, UniFFI bridge, authenticated NDJSON transport, replay ring, subscriptions, backpressure, CLI and transport unit tests have landed. `scripts/m2-acceptance.sh` is the end-to-end check against a running app. A true headless app-test mode is still future work; core and transport tests run without a window.

## M3 — Agent layer (≈3 weeks)

Goal: the reason to exist.

**Status:** implemented. Core state machine/attention/staleness, provider table,
`agent.detect` passive tier, `berth run` wrapper with explicit agent ids and
resume payloads, sidebar agent rows with attention-first sorting, pane
attention rings, `⌘⇧U`/`⌘I`, and agent-aware restore are in. Hooks recipes and
provider mute remain M4/M5 polish.

- `AgentSurface` model + state machine + TTL/staleness in core; provider table (claude-code, codex, opencode).
- `berth run -- <argv>` wrapper (registers, injects `BERTH_AGENT_ID`, records resume payload, maps exit codes).
- Passive detection via foreground pid/argv → `unknown` badges.
- Sidebar agent rendering: provider glyph, state chip, one-line detail, progress bar; attention-first sort.
- Attention ladder: pane ring, unread badge, `⌘⇧U` newest-attention jump, `⌘I` popover, macOS notification with dedupe/mute.
- Agent-aware restore: resume argv per agent, resurrection on launch, explicit "resume failed" state instead of silent lies.

**Done when:** three concurrent agents run in three workspaces; one blocks on a permission prompt and one dies; both are visible within one second, jumpable with one keystroke, and after a quit/relaunch all three resume with their previous state visible.

## M4 — Daily driver (≈2 weeks)

**Status:** in progress. Hooks landed: `berth hooks install` merges Claude Code
settings and writes an opencode plugin; both record state/detail and the
provider session id, so restore uses `--resume`. Perf, crash hardening, and
polish remain.

- Hook recipes for Claude Code and opencode (shipped as copy-paste snippets + `berth hooks install`). **Done.**
- Perf pass against the budgets in `docs/02-architecture.md` §9; Instruments trace of startup and keystroke path.
- Crash/quit safety, zombie PTY audit, restore fuzzing (kill -9 mid-session).
- Polish: notification popover, empty states, sidebar density, theme tokens, onboarding (one screen).
- Docs: README, protocol reference, config reference.

**Done when:** two weeks of personal use with zero fallbacks to another terminal.

## M5 — MVP polish (pull-forward list)

Ship the smallest useful daily driver first; these are the known gaps that should
not be mistaken for missing architecture.

- **Group management UI.** Groups render and collapse, but there is no
  create/rename/color picker even though `group.create/update/delete` exist.
- **Split weights round-trip.** `NSSplitView` divider positions are not fed back
  into the core, so relaunch resets pane sizes.
- **Geometric focus.** ⌘] / ⌘[ walk the tree; arrow-key spatial navigation is not
  implemented.
- **Agent mute and multi-agent rows.** macOS banners are suppressed for the
  focused workspace, but there is no per-provider mute, and a workspace row
  shows only its highest-attention agent when several run in the same
  workspace.
- **Journal semantics.** `journal.ndjson` stores snapshots (newest wins) instead
  of replayable structural events; a crash can lose the last 500 ms.
- **Headless app-test mode.** `scripts/m2-acceptance.sh` drives the real GUI app
  over the socket. There is no test-mode app loop that runs the windowless
  surface layer on CI, so the end-to-end criterion is still a manual Mac check.
- **Generated JSON Schema.** `docs/03-protocol.md` promises a schema generated
  from the Rust types under `docs/schema/`; `berth schema` currently prints the
  method/event capability list only.
- **Socket hardening.** Token auth, read-only classification, and replay are in
  place. Remaining: no peer-UID/audit-token verification, `BERTH_TOKEN` is
  visible to child processes, and `trusted` mode bypasses the token entirely.
- **Discoverability and input polish.** Workspace/tab creation buttons,
  double-click/middle-click gestures, and font sizing are in place as of 0.4.1;
  keep testing them on a real Mac.
- **Live font reload.** The font size is written into Ghostty's config before
  startup, so changing it in Settings needs a relaunch. Apply it to live
  surfaces without restarting the app.

MVP gate stays the M1 restore criterion plus the M0 smoke checklist, not this
list.

## After v1 (ordered, not scheduled)

1. **Agent inbox / journal** — per-agent timeline of state transitions and notifications (we already generate the events; this is persistence + UI).
2. **Cost + token meter** per workspace, from adapter-reported usage.
3. **Session search** — find the workspace where that `pnpm test` failed, from screen text we already can read.
4. **Editor integration** — `berth` LSP-ish shim so an agent in Neovim/Zed can report state without hooks.
5. **Linux shell spike** — one week, throwaway: GTK4 + libghostty-vt + a minimal renderer, reusing `berth-core` and the identical protocol. Decide from evidence whether Windows/Linux is a real roadmap item.
6. **Browser pane**, **SSH workspaces**, **remote tmux mirror** — only if the above is boring and stable.

## Explicitly deferred

Webview panes; iOS; cloud VMs; teams; plugin system; TUI; telemetry; auto-update (Sparkle) until v1 is stable; notarized distribution until someone other than me installs it.

## Kill criteria

Stop or re-scope if any holds after M3:

- Keystroke latency measurably worse than Ghostty in the same machine session (renderer integration is not viable).
- Ghostty API churn costs more than one day per upgrade, twice in a row.
- Agent adapters need per-project configuration to be useful (the typed model is not paying for itself).
- I catch myself opening another terminal for real work two days in a row.
