# Berth — Roadmap

Schedule assumes one developer, evenings + weekends, on an Apple Silicon Mac. Dates are *ordering*, not promises. Each milestone ends with a demoable app; no milestone may leave `main` unable to run a terminal.

M0–M4 are closed at **0.14.1**. What is left is not architecture: it is the terminal
floor, the window model, shell integration, durability and distribution. The evidence
for every item below, including what competitors ship and what the code actually does,
is [docs/11-competitive-gaps.md](11-competitive-gaps.md).

## Shipped

| Milestone | Shipped | What closed it |
| --- | --- | --- |
| **M0 Foundations** | before 0.9 | GhosttyKit host, Metal surface, input/IME/clipboard, config passthrough; the M0 smoke checklist in `docs/05-build.md` is the standing check. The last display defect closed in 0.8.4. |
| **M1 Workspace shell** | before 0.9 | Split tree with weights, per-pane tabs, sidebar metadata, session restore incl. cwd and weights, palette, editable shortcuts. |
| **M2 Core + protocol** | before 0.9 | `berth-core` model/router/events, UniFFI bridge, authenticated NDJSON transport, replay ring, subscriptions, backpressure, CLI, generated schema; `scripts/m2-acceptance.sh` drives the real app. |
| **M3 Agent layer** | 0.8.x | Typed `AgentSurface` state machine, attention ladder, one-second passive detection, `berth run`, resume payloads, agent-aware restore. |
| **M4 Daily driver** | 0.9–0.14 | Perf budgets met (cold start 135–143 ms, idle ~0 % CPU, 161 MB at 8 panes), crash hardening, hooks for four providers, notarization script ready. |

The three defects that daily use found (split-resize feedback crash, detection deadlock,
half-size rendering on 1× displays) are fixed and recorded in `CHANGELOG.md`. The
diagnostics that found them stay.

### M5 close-out

The M5 pull-forward list is closed. Items that finished:

- **Group management** — create/rename/colour in the portable window (0.13), via
  `group.create/update/delete`.
- **Agent mute, multi-agent counts, usage** — `muteProviders`, `agent.usage`, and a
  count on a workspace row (0.13).
- **Socket hardening** — peer-uid check, token in a 0600 file instead of the child
  environment, `trusted` mode warns (0.14).
- **Perf and notarization** — the Instruments trace and §9 budgets (0.14).
- **Portable parity** — search, rename/reorder, accessibility tree, kitty/sixel/iTerm2
  graphics, .deb and PKGBUILD. Remaining AppKit parity is now 1.0 work, below.

Items that did **not** close, and where they went:

| Item | Now |
| --- | --- |
| Geometric focus | 0.16 |
| Journal stores snapshots, not replayable events | 0.18 |
| Headless app-test loop for the real GUI | 0.18 |
| Notification list filters, bulk ack, dock badge | 0.18 |
| Windows notifications are a stub | 1.0 |

## The 1.0 bar

1. Two consecutive weeks of personal use with no fallback to another terminal. This is
   M4's acceptance criterion and the only one never recorded as met; it gates 1.0, not a
   release number.
2. A terminal that passes the protocol floor in §1 of the gap analysis, on both shells.
3. More than one window, and a pane you can maximise.
4. Installed by someone other than me, on three platforms, updating itself, with the
   socket contract frozen at v1.

## 0.15 — Terminal floor (≈3 weeks)

Goal: stop being a 2019 terminal in a 2026 shell. Every item is portable-only unless
tagged; the AppKit shell inherits these from Ghostty already.

- **Kitty keyboard protocol**: progressive enhancement, `CSI u` for modified keys, key
  release events, disambiguated Escape; advertise it in DA1. Agents and Neovim/Helix bind
  it.
- **Synchronized output (DECSET 2026)**: hold paint between begin/end so full-screen
  TUIs (Claude Code, docker, lazygit) stop tearing.
- **Styled text**: strikethrough, curly/dotted/dashed underline, underline colour
  (SGR 4:x, 58), and OSC 8 hyperlinks tracked by the parser instead of the text scan;
  hover underline and Cmd/Ctrl-click.
- **Cursor and focus**: DECSCUSR shapes, focus in/out (CSI ?1004), OSC 52 read with a
  confirmation dialog.
- **Ligatures**: shape runs rather than single cells, with a font-features key.
- **Palette and zoom**: a theme list with light/dark selection and a persisted choice
  (portable currently ships one hard-coded palette); font-zoom chords on both shells.
- **Resize reflow**: re-wrap wrapped lines on width change so shrinking a pane does not
  garble history. The largest item here and the one allowed to move to 0.16.
- **Bell**: `bell = none | visual | audio`.

**Done when:** `nvim`, `lazygit`, `yazi`, `htop` and a Claude Code session render and
respond identically to stock Ghostty at the same size, and a scripted transcript of each
sequence above produces the documented state.

## 0.16 — Window and pane parity (≈2 weeks)

Goal: the layout model cmux and every other terminal already have.

- **Multiple windows** in both shells: `window.create/list/close/select` in the protocol,
  windows in the session snapshot, per-window sidebar and workspace selection, CLI
  commands.
- **Pane zoom**: maximise the focused pane and restore the tree, with the state visible
  in the status bar and surviving restore.
- **Tab tear-off**: drag a tab out to a new window, and move surfaces across panes and
  windows from the CLI (`surface.move` gains a window/pane target).
- **Tab overflow**: scrollable strip plus an overflow menu, replacing the current
  squeeze-and-truncate behaviour.
- **Broadcast input**: type into every pane of the workspace, off by default, with a
  persistent banner while it is on.
- **Geometric focus**: arrow-key spatial navigation (M5 carryover).
- **Window state**: persist geometry, monitor and sidebar width; a second launch brings
  the first window forward instead of exiting.
- **Sidebar depth**: filter, one row per agent, colour and icon picker.

**Done when:** three windows with nested splits survive quit and relaunch with geometry
and focus intact, and a pane can be zoomed, broadcast to, and torn into a window without
the CLI.

## 0.17 — Shell integration and recall (≈2 weeks)

Goal: the prompt becomes a place in the buffer, not a string in a stream.

- **OSC 133/633 prompt marks** in the portable parser: prompt start, command start, exit
  status and duration; a gutter mark per command and click-to-jump between prompts.
  Without command blocks; that is a renderer change and stays out.
- **Search parity**: search in the AppKit shell; regex and whole-word toggles; search
  across the panes of a workspace, not just the focused scrollback.
- **Copy mode**: vi-style keyboard selection, motions, yank in the portable window.
- **Paste history**: the last N clipboards with a picker, never persisted to disk.
- **Quick terminal**: a global-hotkey dropdown window per display, both shells.
- **Palette depth**: workspace, tab and group commands, recent-choice ranking, and
  user-defined commands from `config.json`.

**Done when:** a failing command can be found by jumping prompt to prompt and reading its
status, a search in one pane reaches the others, and a selection can be made without the
mouse.

## 0.18 — Durability and agent depth (≈3 weeks)

Goal: an app update should not kill a running agent, and the typed agent model should
show its history.

- **Live process durability.** Decide with a two-day spike: a detached PTY host
  (`berthd`, the app as a client, which the protocol was shaped for), or attach to
  tmux/zellij. Then ship one. This is the only item that changes the process model.
- **Agent inbox**: persist the per-agent timeline (state transitions, notifications,
  cost) and show it in a panel; the events already exist, this is storage and UI.
- **Cost and token meter** per workspace, from `agent.usage`.
- **Provider coverage**: add the providers that hook without per-project configuration
  (Gemini CLI, Cursor CLI, Amp) to the current four.
- **Journal**: replayable structural events, or an explicit decision to keep snapshots
  and say why in the protocol doc.
- **Headless app-test loop** for the real GUI, so acceptance runs on CI (M5 carryover).
- **Notification depth**: filters, bulk acknowledge, dock/taskbar badge, optional sound.

**Done when:** relaunching or updating the app leaves agent sessions alive, and a
workspace's inbox shows every transition since it was created.

## 1.0 — Distribution and the macOS shell (≈2 weeks)

Goal: a product, not a repository.

- **macOS**: notarized Developer ID build in releases, Sparkle auto-update, Homebrew
  cask. Close the AppKit-only list: search, tab rename/reorder, group UI, geometric
  focus, terminal accessibility.
- **Linux**: AppImage or Flatpak beside the .deb and PKGBUILD; AUR submission.
- **Windows**: MSI or winget package, real toast notifications, peer-SID check on the
  named pipe.
- **Contract**: freeze the protocol at v1 with semver, regenerate
  `docs/schema/berth.schema.json`, and publish a typed client example.
- **Docs**: every `config.json` key documented for both shells and a migration note for
  anyone on 0.x.

**Done when:** a stranger can install on three platforms without opening a terminal, the
app updates itself, and an editor plugin can be written against a frozen schema.

## After 1.0 (ordered, not scheduled)

1. **Session search** — find the workspace where that `pnpm test` failed, across all
   scrollback.
2. **Editor integration** — a `berth` shim so an agent in Neovim/Zed reports state
   without hooks.
3. **Remote clients** — mobile or web read-only view over the same event stream.
4. **Browser pane**, **SSH workspaces**, **remote tmux mirror** — only if the above is
   boring and stable.

## Explicitly deferred

Webview panes; iOS; cloud VMs; teams; plugin system; TUI; telemetry; browser automation.
Auto-update moved into 1.0. Notarized distribution ships with 1.0, when there is someone
other than me installing it.

## Kill criteria

Stop or re-scope if any holds after 0.16:

- Keystroke latency measurably worse than Ghostty in the same machine session (renderer
  integration is not viable).
- Ghostty API churn costs more than one day per upgrade, twice in a row.
- Agent adapters need per-project configuration to be useful (the typed model is not
  paying for itself).
- I catch myself opening another terminal for real work two days in a row.
- The terminal floor forces replacing the portable window's shaping or buffer model
  rather than extending it: reflow plus ligatures plus OSC 8 must fit in 0.15–0.16, or
  the portable window becomes a thin client of the macOS core instead.
