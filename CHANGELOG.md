# Changelog

## Unreleased

### Portable window: real text and daily terminal basics

- `berth-gui` now renders real system fonts through `cosmic-text` instead of the
  8×16 bitmap face: font discovery with fallback, so Cyrillic, CJK and colour
  emoji draw; real bold, italic, dim and underline; per-cell glyph cache; a
  smaller chrome font for tabs and the sidebar. Cell metrics follow the same
  `fontSize` × `fontScale` rule as the macOS shell, including the automatic
  `2 / backing` scale.
- The window became a terminal: wheel and `Shift+PageUp`/`Shift+PageDown` scroll
  the 10,000-line scrollback, drag/shift-drag/double-click select, and
  `Super+C`/`Ctrl+Shift+C` copy while `Super+V`/`Ctrl+Shift+V` paste with
  bracketed paste when the program asks for it.
- Terminal events now reach the core: OSC 0/2 titles become tab labels and the
  window title, OSC 7 sets the cwd, BEL counts and flashes the pane, OSC 9;4
  reports progress, OSC 52 writes the clipboard, and OSC 9/777 create
  notifications. A child exit is reported once as `surface.exited` and leaves a
  "process exited" banner instead of a frozen screen.
- `berth-headless` grew the plumbing: `ScreenMeta` callbacks for title, cwd,
  bell, progress, clipboard and notifications; `ScreenView` now carries cursor
  visibility, scrollback offset, bracketed paste and per-cell dim/italic/
  underline/wide flags; plus `screen_meta`, `take_screen_meta_requests`,
  `set_scrollback`, `is_alive`, `contents_between` and `line_text`.
- Layout parity: draggable split dividers (`split.resize`), a resizable sidebar,
  tab strips that share width and truncate labels with a close cross, a `+`
  button and double-click to create a workspace, middle-click to close a tab or
  workspace, group chevrons (`group.update`), pinned-first and attention-ranked
  ordering, agent rows with state, detail and OSC progress, and `⌘⇧U` jump to
  the newest attention.
- The command palette (`⌘⇧P`/`Ctrl+Shift+P`) lists all 14 configurable actions
  with their chord and a fuzzy filter; the attention panel (`⌘I`) lists
  unacknowledged notifications newest first and jumps to the workspace and
  surface that raised one on Enter. Both share one panel geometry, keyboard
  handling and hit testing; clicking outside or focusing another window closes
  them.
- `config.json` is now read by the portable window: `fontSize`, `fontScale`,
  `compactSidebar`, `socket` and all 14 `shortcuts`, with a Ctrl-based default
  table outside macOS. `BERTH_CONFIG` overrides the path.
- `--smoke-png` renders a frame to PNG and exits, which is how the visual work
  was checked on macOS; `--smoke-ms` keeps its headless Xvfb role. New tests
  cover font metrics and caching, sidebar ordering and groups, geometry and
  dividers, config parsing, shortcut resolution, selection and paint blending.
- The workspace MSRV moves to 1.89 for `cosmic-text`, and the lints that
  MSRV-aware clippy now raises in existing crates were collapsed to let-chains.

### Portable foundation: Linux and Windows

- `berth-core` now owns every platform path decision (`berth_core::paths`):
  `~/Library/Application Support/Berth` on macOS (unchanged), `$XDG_DATA_HOME` /
  `~/.local/share/berth` on Linux, `%APPDATA%\Berth` on Windows, with
  `BERTH_DATA_DIR`, `BERTH_SOCKET_PATH` and `BERTH_TOKEN_FILE` overrides.
- New `berth-transport` crate: one local-socket API — Unix domain sockets on
  Linux/macOS, named pipes on Windows — plus the NDJSON framer and its limits.
- New `berth-server` crate: the NDJSON server (hello, token auth, read-only
  classification, subscription filters, 1024-event replay ring with `gap`,
  constant-time token compare, backpressure) in Rust, hostable by any shell.
  `session.save` is a shell callback; events are drained after every request.
- `berth` CLI now runs on Linux and Windows: portable socket connect, platform
  path resolution, `PATHEXT`-aware `berth run` resolution, absolute-path hook
  commands (`>/dev/null 2>&1` on POSIX, `>NUL` on Windows), and atomic file
  replacement that works with Windows `rename` semantics.
- `SurfaceOps` is now `Send + Sync`, so shells can hand it to the server thread.
- New `berth-headless` crate (`berth-headless` binary): the first non-Swift
  shell slice. Real PTYs via `portable-pty`, a `vt100` screen with 10k lines of
  scrollback, synthetic key encoding, `tcgetpgrp` foreground `pid_info`,
  atomic session save/restore, and the full `berth-server` contract. Its
  integration test spawns a workspace, types `echo …` + Enter, reads the screen
  back, resizes, and closes the PTY on Linux/macOS/Windows CI. No GUI yet; see
  `docs/07-headless-shell.md`.
- New `berth-gui` crate (`berth-gui` binary): the first visible Linux/Windows
  window. `winit` + `softbuffer` + an 8×16 bitmap grid, 256/RGB colors, block
  cursor, key routing through the headless encoder, PTY resize, embedded
  `berth-server`, restore/save. Draws the selected workspace's weighted split
  tree with one terminal per leaf; clicking a pane focuses its surface and
  broadcasts the focus events. Per-pane tab strips (click, Ctrl+Shift+T/W,
  Ctrl+PageUp/PageDown) and a clickable workspace sidebar sit on the same
  snapshot. Divider dragging, scrollback, selection and clipboard are next.
  `--smoke-ms` renders under Xvfb for headless checks. See
  `docs/08-minimal-window.md`.
- CI runs fmt, clippy and tests on Ubuntu, macOS and Windows; the Swift app job
  remains macOS-only. The Swift app still uses its Swift transport until the
  UniFFI migration is smoke-tested on a Mac.
- New `docs/09-known-gaps.md`: audited inventory of missing GUI behavior,
  headless-shell limits, core/server/CLI gaps, macOS gaps and intentional
  non-goals, plus the test-coverage matrix.

## 0.9.0

### Focus fix

- Clicking a pane no longer loses focus a moment later. A click made the view
  first responder but never told the core, so the core's active surface stayed on
  the other pane and the next reconcile pass (an agent event, the one-second
  detection pass, a socket call) handed focus back. A click now reports
  `surface.activate`, and a reconcile pass never places focus while a terminal
  already holds it.
- The correction is logged as `ui: reconcile placed focus on <surface>`, so a
  recurrence shows up in `berth.log` instead of looking like a random jump.
- `FocusPolicyTests` pins the rule that focus is not taken from a terminal.

## 0.8.4

### Display fix

- On a 1× display the terminal drew at half size in the top-left corner of the
  pane, leaving the rest empty, and clicks selected text at double their visual
  position. Ghostty sizes its drawable from the layer's `contentsScale` but
  draws cells with the surface's content scale, and it sets the layer scale only
  once, at surface creation, from the config's `scale_factor`. Berth creates
  surfaces before they are in a window, so that value was always 2.0 while a 1×
  display set the content scale to 1.0. `syncSize` now keeps the layer scale
  equal to the window's backing scale, as Ghostty's own macOS app does, and the
  surface is created with the screen's scale instead of a hardcoded 2.0.
- `~/Library/Logs/Berth/berth.log` records one `surface:` line per geometry
  change: bounds, backing scale, layer scale, surface pixels, grid, cell size
  and font. A grid that disagrees with the drawn text is visible there.

## 0.8.3

### Lock fix

- The app could stop responding a second or two after launch, with the busy
  cursor and dead keystrokes. `Engine.dispatch` holds the core's state mutex for
  the whole request, and passive agent detection runs off the main thread and
  asked for the foreground pid with `DispatchQueue.main.sync`. When the main
  thread was inside a core dispatch, it was waiting for the mutex the detection
  thread held while waiting for the main thread: both wedged. `pidInfo` now
  reads a pid snapshot the main thread publishes once per detection tick, and
  notifications are queued instead of blocking.
- Detection no longer hops to the main thread at all, so `ps` still runs
  off-main and keystrokes never wait for it.

### Diagnostics

- `~/Library/Logs/Berth/berth.log` keeps rolling breadcrumbs (reconcile and
  dispatch durations, layout rebuilds, ignored divider resizes, detection
  passes), capped at 2 MiB with one rotation.
- A watchdog pings the main queue once a second and appends a stall to
  `crash.log` with the 40 breadcrumbs before it when the runloop stops
  answering for two seconds.
- `make fetch-engine` downloads the CI-built GhosttyKit, so a Mac with only the
  Command Line Tools can build and run the app.

## 0.8.2

### Crash fix

- ⌘D (and any other structural layout change: ⌘T, ⌘W, workspace switch) could
  crash the app with a stack overflow. Rebuilding the pane tree posts
  `NSSplitViewDidResizeSubviews`; the handler reported those half-laid-out
  divider positions to the core, which synchronously rebuilt the tree again and
  re-entered `MainWindow.apply` until the stack ran out
  (`EXC_BAD_ACCESS` on the stack guard region). Two guards close the loop:
  resizes observed while a rebuild is running are ignored, and `reconcileAll`
  defers a re-entrant call to the next runloop turn (coalesced with
  `scheduleReconcile`) instead of recursing.

## 0.8.1

### Release fix

- `bundle.sh` now expands `CFBundleShortVersionString` and `CFBundleVersion`;
  0.8.0 shipped literal `${VERSION}`/`${BUILD}` placeholders, which makes the
  bundle invalid to LaunchServices (typically "The application cannot be
  opened" with error `-47` / `fBsyErr`).
- The bundler rejects unexpanded placeholders and non-numeric build numbers,
  and CI validates both keys with `plutil` before uploading the archive.

## 0.8.0

### Agents (M3)

- A dead agent is visible immediately: `SHOW_CHILD_EXITED` marks the bound live
  agent `errored` (`reason: process_exit`) and emits `surface.exited`.
- Passive detection runs every second, off the main thread; `ps` no longer
  blocks the render path.
- Agent attention now reaches macOS Notification Center (previously only
  explicit `notification.create` did).
- `⌘⇧U` acknowledges the agent it visits; `agent.ack` clears derived attention
  and its unread notifications. Sidebar attention ranking now matches the core
  (error > needs_input).
- New `agent-ack` CLI and `make m3` acceptance script.
- The generated opencode plugin is now an OpenCode V2 `Plugin.define` with
  `id`/`setup` and an event-stream subscription; a CI test locks that shape so
  the "Expected object at [default]" load failure cannot return.

### Protocol and persistence (M2/M4)

- `session.save` is wired to the shell's real atomic snapshot + journal write.
- `session.restore` replaces live state after validation instead of appending.
- `berth schema` now emits a real JSON Schema from the Rust types at
  `docs/schema/berth.schema.json`; the CLI can run it without a daemon.
- Event contract fixed: `surface.exited`, `agent.attention_changed`,
  `workspace.layout`, `split.resize`, and group/move events are documented and
  advertised. The never-emitted `session.dirty` event was removed.
- `system.hello` reports the real transport auth mode; `system.ping` reports
  uptime.
- Journal is capped at 5 snapshots / 4 MiB; deterministic restore fuzzing and a
  replace-state test were added.

### Display and UI (M4)

- Font sizing is per-surface and per-display: 1× monitors get an automatic 2×
  point multiplier, overridable in Settings (`fontScale`, `0` = auto). Font
  changes apply live via `ghostty_surface_update_config`.
- Split weights round-trip: divider positions are restored and dragged
  positions are persisted with `split.resize`.
- Dedicated `⌘I` attention panel, one-screen onboarding, theme tokens, and a
  compact sidebar option.
- Startup, restore, save, core dispatch, socket auth/backpressure, and surface
  create/teardown now log under subsystem `dev.berth.app`; uncaught exceptions
  append to `~/Library/Logs/Berth/crash.log`.
- Configuration reference at `docs/config.md`.

### Known gaps

Headless app-test mode, peer-UID socket verification, hiding `BERTH_TOKEN` from
children, group management UI, geometric focus, and replayable journal events
remain in `docs/04-roadmap.md` (M5).
