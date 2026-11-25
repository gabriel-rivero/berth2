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
| **M4 Daily driver** | 0.9–0.14 | Perf budgets met (cold start 135–143 ms, idle ~0 % CPU, 161 MB at 8 panes), crash hardening, hooks for five providers, notarization script ready. |

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
tagged; the AppKit shell inherits these from Ghostty already. Work through the list in
this order, one item per session: item 1 builds the CSI side-channel scanner that items
2 and 3 reuse, and each item lands with its tests and a changelog entry.

1. **Synchronized output (DECSET 2026)** — landed in 0.15.0. The PTY reader's CSI
   side channel records begin/end, `screen_view` carries the flag, and the window keeps
   the previous frame until the program ends it or 500 ms passes, so full-screen TUIs
   stop tearing and a crashed client cannot freeze the pane.
2. **Kitty keyboard protocol** — landed in 0.15.1. The scanner tracks the flags a
   program pushes, sets or pops (with separate main and alternate-screen stacks),
   answers `CSI ? u`, and the encoder honours the flags: `CSI u` for modified keys and
   disambiguated Escape, release and repeat events, alternate keys and associated text.
   Modifier-only key events are not reported. Agents and Neovim/Helix bind it.
3. **Styled text** — landed in 0.15.2. SGR 9/29 strikethrough, SGR 4:1..5 underline
   styles (single, double, curly, dotted, dashed), SGR 58 underline colour, and OSC 8
   hyperlink spans are recorded on the cells they style. Hover underlines a hyperlink
   and Cmd/Ctrl-click opens it; text URL detection is the fallback.
4. **Cursor and focus** — landed in 0.15.3. DECSCUSR selects block, underline and bar
   cursors with their blink state, CSI ?1004 reports focus in/out for the focused pane,
   and an OSC 52 read opens a confirmation dialog before returning the clipboard.
5. **Fonts and ligatures** — landed in 0.15.4. Adjacent cells with the same style are
   shaped as one run, so a face with ligatures draws them. `fontWeight`, `lineHeight`,
   `fontFeatures` and per-face `fontMetrics` are read from `config.json`, and SGR 5 text
   blinks.
6. **Appearance and zoom**: landed in 0.15.5. `theme` picks `berth-dark` or
   `berth-light` and persists, `opacity` fades the window on macOS, and
   `backgroundBlur` reaches Ghostty in the AppKit shell. `Cmd/Ctrl + =`, `-` and `0`
   zoom the font on both shells for the session; `fontSize` stays the base.
7. **Resize reflow**: landed in 0.15.6. A column change captures the grid with a newline
   only where the program broke the line, replays it at the new width, and restores the
   cursor and the input modes, so shrinking a pane re-wraps history instead of cutting
   it. Alternate-screen programs keep the plain resize because they redraw on SIGWINCH,
   and images skip it because their anchors are line-based.
8. **Bell**: landed in 0.15.5. `bell` selects `none`, `visual` (the default pane-border
   flash) or `audio`, which plays the system bell sound. A repeated bell rings once per
   batch, not once per frame.

**Done when:** `nvim`, `lazygit`, `yazi`, `htop` and a Claude Code session render and
respond identically to stock Ghostty at the same size, and a scripted transcript of each
sequence above produces the documented state.

## 0.16 — Window and pane parity (≈2 weeks)

Goal: the layout model cmux and every other terminal already have.

**Landed in 0.16.x** (protocol, core, persistence, CLI and both shells): multiple
windows with per-window sidebar selection and geometry, pane zoom/swap/move, tab
tear-off across panes and windows, a scrollable tab strip with an overflow menu,
broadcast input, geometric focus, sidebar filtering and one row per agent, an
in-window menu strip and status line with a resize cursor, right-click tab/pane
actions, and a v1→v2 session migration that keeps existing layouts. The AppKit shell
gained the window registry, pane zoom, geometry persistence and the new menu items;
its broadcast flag is not yet wired into the key path, and RTL chrome plus the
sidebar colour/icon picker remain.

- **Multiple windows** in both shells: `window.create/list/close/select` in the protocol,
  windows in the session snapshot, per-window sidebar and workspace selection, CLI
  commands.
- **Pane zoom**: maximise the focused pane and restore the tree, with the state visible
  in the status bar and surviving restore.
- **Tab tear-off**: drag a tab out to a new window, and move surfaces across panes and
  windows from the CLI (`surface.move` gains a window/pane target). Restart and duplicate
  a tab from the palette and the tab menu.
- **Tab overflow**: scrollable strip plus an overflow menu, replacing the current
  squeeze-and-truncate behaviour.
- **Broadcast input**: type into every pane of the workspace, off by default, with a
  persistent banner while it is on.
- **Geometric focus**: arrow-key spatial navigation (M5 carryover).
- **Window state**: persist geometry, monitor and sidebar width; a second launch brings
  the first window forward instead of exiting.
- **Sidebar depth**: filter, one row per agent, colour and icon picker, and right-to-left
  shaping for rows, tab labels and panel text (terminal panes already do this).
- **Menus, status bar and pointer**: a menu bar and a status line that shows chord mode,
  broadcast state, pane zoom and the focused pane's cwd and branch; a resize cursor over
  dividers; right-click menus on tabs and panes; pane swap and move between splits.

**Done when:** three windows with nested splits survive quit and relaunch with geometry
and focus intact, and a pane can be zoomed, broadcast to, and torn into a window without
the CLI.

## 0.17 — Shell integration and recall (≈2 weeks)

Goal: the prompt becomes a place in the buffer, not a string in a stream.

- **OSC 133/633 prompt marks** in the portable parser: prompt start, command start, exit
  status and duration; a gutter mark per command and click-to-jump between prompts.
  Without command blocks; that is a renderer change and stays out. **Landed in 0.17.0**
  as prompt start, command start and exit status: the reader records each mark against
  its absolute grid line, `screen_view` carries the visible ones, the portable window
  draws a green or red tick per prompt plus a dim mark where output began, a gutter
  click brings that command to the top of the pane, and `previousPrompt`/`nextPrompt`
  walk them (`Cmd+Shift+Up`/`Down`, `Ctrl+Shift+Up`/`Down`). Duration is not parsed, the
  gutter costs columns only in a pane whose shell emits marks, and the AppKit shell
  inherits Ghostty's own shell integration rather than this gutter.
- **Search parity**: search in the AppKit shell; regex and whole-word toggles; search
  across the panes of a workspace, not just the focused scrollback. **Landed in 0.17.0**:
  the portable panel compiles a literal or regex query once per keystroke, applies a
  whole-word test on the same character class a double-click uses, runs over every pane
  in the visible workspace, tints all hits with the selected one in the accent, and
  focuses the owning pane before jumping. The AppKit shell gained a find bar over
  Ghostty's own search (`Cmd+F`), with next/previous and a match count.
- **Copy mode**: vi-style keyboard selection, motions, yank in the portable window.
  **Landed in 0.17.0**: a modal key capture in front of the chord table, with
  `h`/`j`/`k`/`l`, word and line motions, counts, `g`/`G`, paging, a visual selection and
  a yank from a buffer captured on entry, so a selection can span more than one screen.
  The cursor is an absolute line, so it survives scrolling; a click moves it, and the
  wheel keeps the painted tint on the right rows. The AppKit shell uses Ghostty's own
  copy-mode bindings and needs nothing here.
- **Paste history**: the last N clipboards with a picker, never persisted to disk.
  **Landed in 0.17.0**: `pasteHistory` keeps twelve entries in memory, newest first, with
  move-to-front dedupe, a byte cap and a label clipped at record time. It records the
  user's copies and pastes, not OSC 52 writes or copy-on-select. On macOS the chord
  overlaps the fixed paste binding, so an empty history falls back to a plain paste.
- **Quick terminal**: a global-hotkey dropdown window per display, both shells.
  **Landed in 0.17.0** for the portable window: `quickHotkey` is an OS-wide chord, the
  dropdown is a `WindowKind::Quick` window that floats, hides on focus loss, toggles on
  the chord and is excluded from the session file, with `quickPlacement`/`quickHeight`
  setting where it sits. It has its own spawned workspace, and `window.quick` leaves
  `selected_window` alone so opening it cannot steal the window the user is in. The
  AppKit shell opens it from the Terminal menu but registers no OS-wide chord.
- **Commands and profiles**: user-defined commands from `config.json`, recent-choice
  ranking, named launch presets that replace the single `shell` key (argv, env, cwd,
  shell, label) on both shells, and saving the current tree as a named layout.
  **Landed in 0.17.0** as the shared `launch` module: profiles and commands parse in the
  core, so the palette, `berth new --profile` and `berth profiles` all resolve the same
  definition; `layout.save`/`layout.open` round-trip a workspace tree with fresh pane
  ids. Recent-choice ranking and the AppKit palette are not wired yet.
- **Output rules and logging**: regex rules that style matching lines, as ConEmu's
  Highlight and iTerm2's Triggers do, and a per-pane session log written to a file.
  **Landed in 0.17.0** for the portable window: `outputRules` tint a whole visible row
  (first match wins, invalid rules reported and skipped) and `sessionLog` appends each
  pane's output to `sessions/<id>.log` with a single 8 MiB rotation. The AppKit shell
  reads neither key yet.
- **Scrollback restore**: opt-in and capped by bytes, so a relaunch keeps the last screen
  of a pane instead of only its cwd. **Landed in 0.17.0** for the portable window:
  `scrollbackRestore` captures each pane's screen on a clean exit, capped at 64 KiB,
  replayed with CRLF per line and skipped for alternate-screen programs; the saved copy
  is consumed by the replay.
- **File drops**: dropping files and folders on a pane types their quoted paths.
  **Landed in 0.17.0** for the portable window, with per-shell quoting, batching and
  bracketed paste; paths with control characters and drops into full-screen programs are
  refused rather than typed wrong.
- **Palette depth**: workspace, tab and group commands, and a palette that indexes the
  new actions. **Landed in 0.17.0** for the portable window: the palette is
  heterogeneous, indexing actions, profiles, commands, layouts and the live session
  (workspaces, tabs with their pane, groups), with per-window recency that never
  promotes a destructive action.

**Done when:** a failing command can be found by jumping prompt to prompt and reading its
status, a search in one pane reaches the others, a selection can be made without the
mouse, and a WSL/Git Bash/admin preset starts in one click.

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
  (Gemini CLI, Cursor CLI, Amp) to the current four, and detect a provider behind a
  wrapper script or a renamed argv.
- **Harness skills and plugins.** `berth hooks install` currently writes lifecycle hooks
  only: the harness reports its state to Berth but is never told what Berth can do. Ship
  an installed skill/instruction bundle per harness (Claude Code, opencode, Copilot,
  Command Code, and the new providers) that teaches the agent when to create a window or
  pane, move a tab, zoom or broadcast, launch with `berth run`, and read a surface back;
  add `berth skills install|print` beside `berth hooks`, keep the text generated from the
  protocol so it cannot drift, and cover it in the acceptance script.
- **Journal**: replayable structural events, or an explicit decision to keep snapshots
  and say why in the protocol doc.
- **Headless app-test loop** for the real GUI, so acceptance runs on CI (M5 carryover).
- **Push instead of polling**: `SurfaceOps.surface_exited` and a screen-meta callback, so
  a shell or the daemon does not poll; a graceful snapshot on SIGINT/SIGTERM.
- **Notification depth**: filters, bulk acknowledge, dock/taskbar badge, optional sound, a
  macOS menu-bar item, a mute row in Settings, and a notifications entry reachable from
  the sidebar.
- **Accessibility beyond the pane**: sidebar, tabs and panels in the AccessKit tree, and a
  high-contrast mode.

**Done when:** relaunching or updating the app leaves agent sessions alive, and a
workspace's inbox shows every transition since it was created.

## 1.0 — Distribution and the macOS shell (≈2 weeks)

Goal: a product, not a repository.

- **macOS**: notarized Developer ID build in releases, Sparkle auto-update, Homebrew
  cask. Close the AppKit-only list: search, tab rename/reorder and overflow, group UI,
  geometric focus, terminal accessibility. Move the shell's own socket onto
  `berth-server`, so peer-uid and the token file apply there too, and read every
  documented `config.json` key.
- **Linux**: AppImage or Flatpak beside the .deb and PKGBUILD; AUR submission; a systemd
  user unit and autostart entry.
- **Windows**: MSI or winget package, real toast notifications, taskbar progress and jump
  lists, peer-SID check and a per-user pipe name, autostart, and the Windows 11
  default-terminal handoff so `cmd` and `pwsh` open in Berth. Elevated panes stay a
  separate instance, not a PTY.
- **Contract**: freeze the protocol at v1 with semver, regenerate
  `docs/schema/berth.schema.json`, publish typed client crates with an example, and close
  the transport gaps: socket lock, per-client identity, audit trail, rate limit and
  server-side protocol negotiation.
- **Platform coverage**: mixed-DPI hotplug tested, and window smokes on Windows and macOS
  CI, not only Xvfb and Weston.
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
4. **Kitty graphics depth** (shared memory, z-index, direct placements) and **automatic
   profile switching** per directory, only if a tool or workflow needs them.
5. **Browser pane**, **SSH workspaces**, **remote tmux mirror** — only if the above is
   boring and stable.

## Explicitly deferred

Webview panes; iOS; cloud VMs; teams; plugin system; TUI; telemetry; browser automation;
collapsible command blocks (prompt marks and jump-to-prompt land in 0.17); agent
hibernation under memory pressure (the eight-pane budget is 161 MB); a task manager or
process list (use the shell); Far Manager and DOSBox integration (ConPTY made the hook
layer obsolete). Auto-update moved into 1.0. Notarized distribution ships with 1.0, when
there is someone other than me installing it.

## Coverage ledger

Every gap in [docs/11-competitive-gaps.md](11-competitive-gaps.md) and
[docs/09-known-gaps.md](09-known-gaps.md) maps to one row here. Anything that is not in a
milestone is listed as deferred with its reason, so a missing feature is a decision and
not an oversight.

| Area | Owned by |
| --- | --- |
| Kitty keyboard, synchronized output, styled underlines, OSC 8, OSC 52 read, focus and cursor reporting | 0.15 |
| Font weight, line height, metrics, ligatures, blink | 0.15 |
| Themes, opacity and blur, font zoom | 0.15 |
| Resize reflow, audio bell | 0.15 |
| Scrollback restore, paste history, copy mode, search parity and cross-pane search | 0.17 |
| OSC 133 prompt marks, jump-to-prompt, exit status | 0.17 |
| Output highlight rules, session logging | 0.17 |
| Launch profiles, custom commands, named layouts, palette depth | 0.17 |
| Quake dropdown, file drops | 0.17 |
| Multiple windows, pane zoom, pane swap, tab tear-off, tab overflow, restart and duplicate | 0.16 |
| Broadcast input, geometric focus | 0.16 |
| Menus, status bar, divider cursor, right-click menus, RTL chrome | 0.16 |
| Window geometry, single-instance handoff, sidebar filter and pickers, per-agent rows | 0.16 |
| Agent inbox, cost meter, provider coverage and detection, mute row | 0.18 |
| Harness skills/plugins (installed instructions for windows, panes, broadcast, run) | 0.18 |
| Notification filters, bulk ack, badge, sound, menu-bar item | 0.18 |
| Journal semantics, headless app test, push callbacks, graceful SIGTERM | 0.18 |
| Accessibility beyond the pane, high contrast | 0.18 |
| Live process durability (detached PTY host or tmux attach) | 0.18 |
| Notarization, Sparkle, Homebrew, AppImage or Flatpak, AUR, Windows package, autostart | 1.0 |
| AppKit parity (search, rename/reorder/overflow, groups, geometric focus, a11y, socket, config keys) | 1.0 |
| Protocol freeze, typed clients, socket lock, client identity and audit, rate limit, negotiation | 1.0 |
| Taskbar progress and jump lists, Windows 11 default-terminal handoff, elevated instance | 1.0 |
| Mixed-DPI hotplug and Windows/macOS window CI | 1.0 |
| Session search, editor shim, remote clients, kitty graphics depth, automatic profiles | After 1.0 |
| Browser pane, SSH, remote tmux, webview, iOS, cloud, teams, plugins, TUI | Deferred by design |
| Command blocks, agent hibernation, task manager, Far/DOSBox | Deferred by design |

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

## Known gaps after 0.16.1

These are the 0.16 items that shipped partially, recorded so they are decisions and not
oversights. They are picked up by the milestones named beside them.

| Gap | Where it goes |
| --- | --- |
| AppKit broadcast sets the flag but typed input still goes only to the focused surface | 0.16.x follow-up |
| AppKit has no tab tear-off, tab overflow, status bar or right-click tab/pane actions (right-click gives Copy/Paste/Select All) | 0.16.x follow-up |
| AppKit sidebar has no filter, one-row-per-agent or colour/icon pickers | 0.16.x follow-up |
| RTL shaping for sidebar rows, tab labels and panel text in either shell | 0.16.x follow-up |
| Pane swap/move are protocol and CLI only; no palette or menu entry | 0.16.x follow-up |
| The portable menu strip is decorative; it has no dropdowns | 0.16.x follow-up |
| The portable shell still exits on a busy socket instead of raising the running window | 0.16.x follow-up |
| Harness plugins are lifecycle hooks only; they install no skills or instructions | 0.18 |
| AppKit parity list (search, rename/reorder, groups, a11y, socket, config keys) | 1.0 |

## Known gaps after 0.16.2

The right-click menus and the app icon closed two of the 0.16.x follow-ups. What they
left open is recorded here; the rows of the 0.16.1 table that are not repeated still
stand.

| Gap | Where it goes |
| --- | --- |
| The AppKit right-click menu is terminal-only: no tab/pane actions, no separators, no search or paste-as-plain-text | 0.16.x follow-up |
| The portable right-click panel is one flat list: Copy cannot be greyed out when the pane has no selection, and there are no separators between groups | 0.16.x follow-up |
| Windows executables carry no icon; only the macOS bundles and the Linux packages do | 1.0 |

## Known gaps after 0.17.0

The 0.17 items that shipped partially, recorded so they are decisions and not oversights.

| Gap | Where it goes |
| --- | --- |
| OSC 133 `D` may carry a command duration; it is not parsed, so there is no per-command timing | 0.18 inbox |
| Paste history is portable-only; the AppKit shell ignores the key and `Cmd+Shift+V` pastes through Ghostty | 1.0 AppKit parity |
| Profiles and commands are offered in the portable palette only; the AppKit palette does not index them | 1.0 AppKit parity |
| Recent-choice ranking for profiles is implemented in the core but no shell records picks into it | 0.18 |
| The palette's session rows are core-global, so a window's palette can list and switch to another window's workspace | 0.18 |
| Palette rows are rebuilt on every paint; the list is uncapped, so a very large session makes each frame allocate | 0.18 |
| AppKit has no palette entry for profiles, commands, layouts or session rows | 1.0 AppKit parity |
| File drops are portable-only; the AppKit shell does not accept a dropped file yet | 1.0 AppKit parity |
| A drop uses the last pointer position, which winit freezes during a native drag, so a multi-pane release can target the wrong pane | 0.18 |
| `cmd` cannot escape `"` or `%` inside a path, so such a path is emitted without the quote and `%` is left unexpanded | 0.18 |
| Scrollback restore is portable-only and captured at clean exit, so a crash restores the previous screen or none | 1.0 AppKit parity |
| Output rules and session logging are portable-only; the AppKit shell reads neither key | 1.0 AppKit parity |
| Output rules tint a row but cannot match across a wrapped line, and cannot rewrite text (no capture-group substitution) | 0.18 |
| A saved layout captures only each pane's active tab, not every tab, and drops per-pane `env` | 0.18 |
| `layout.open` leaves a saved layout's weights unnormalised, so a hand-made layout can jump on the first resize | 0.18 |
| Saved layouts live in memory for the session; nothing writes them to disk or to `config.json` | 0.18 |
| Quick terminal registers no OS-wide hotkey in the AppKit shell; it is menu- and shortcut-only | 1.0 AppKit parity |
| The quick terminal has one display placement, not a per-display dropdown, and is not positioned inside the work area's menu-bar/dock insets | 0.18 |
| A global `quickHotkey` colliding with an in-app binding is not detected; the OS takes the key silently | 0.18 |
| The OS-wide quick chord is X11/macOS/Windows only; a pure-Wayland session falls back to the menu | 0.18 |
| Paste history is one ring shared by every native window, not per window | 0.18 |
| Picking a single-line history entry does not move it to the front, while a confirmed multi-line one does | 0.18 |
| The AppKit shell has no prompt gutter: it inherits Ghostty's marks and `jump_to_prompt` but draws no gutter | 1.0 AppKit parity |
| Prompt marks are dropped on a column change rather than remapped through the reflow | 0.18 |
| Search indexes each pane's active surface only, so a non-active tab in a pane is not searched | 0.18 palette/session search |
| Search captures its buffers when the panel opens; output that arrives while it is open is not indexed | 0.18 session search |
| The AppKit find bar is focused-surface only and has no regex or whole-word toggles (Ghostty owns the matching) | 1.0 AppKit parity |
| Copy mode has no `vi` operator-pending grammar (`d`/`c`), no block selection and no in-mode `/` search | After 1.0 |
| Copy mode captures its buffer on entry, so output that arrives mid-session is not selectable | 0.18 |
