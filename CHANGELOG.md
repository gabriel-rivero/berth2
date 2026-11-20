# Changelog

## Unreleased

### Fixes

- Next/previous tab in the portable window sent `surface.cycle` with `delta`, but the
  method takes a `direction` (`next`/`prev`); the request was rejected as `bad_request`
  and the chord did nothing. The window now sends the protocol's spelling of the step.

## 0.14.1 — 2026-11-20

### Features

- `shell` in `config.json` chooses the program a new terminal runs, as a path or
  an argv (`["C:\\Program Files\\Git\\bin\\bash.exe", "--login", "-i"]`). The
  settings panel cycles through the shells it finds: Git Bash, pwsh, MSYS2 and
  Cygwin zsh/bash and WSL on Windows, `/etc/shells` and `PATH` entries elsewhere.
  Agent panes keep their own command; the macOS app does not read the key yet.

### Fixes

- `ctrl+shift+[` and `ctrl+shift+]` (previous/next tab on Linux and Windows) are
  recognised. The keyboard reports the shifted character (`{`, `}`), so the
  chord spelling never matched and both bindings were dead. Every default chord
  now has a test that types it the way winit reports it on a US layout.
- `berth-gui` builds as a GUI-subsystem binary, so double-clicking it no longer
  leaves a console window with `berth-gui: restored 1 session(s)` and the launch
  banner above the terminal. Started from a terminal it still attaches to that
  terminal for `--help`, `--version`, `--smoke-*` and errors, and redirected
  stdout/stderr keep going to their file or pipe.

## 0.14.0 — 2026-11-19

### Security

- On Unix the socket server rejects a client whose effective uid differs from
  its own, before the token check. The token is no longer exported into child
  environments; clients read the 0600 token file. `trusted` mode still bypasses
  auth by design and now warns at launch.
- The CLI rejects a `hello` whose protocol version is stated and differs from
  its own.

### Fixes

- A horizontal wheel gesture (trackpad swipe or tilt wheel) is reported to a
  mouse-tracking program as xterm buttons 6 and 7. It used to be read as
  `y > 0.0 == false`, so a sideways swipe arrived as a wheel-down and moved the
  selection instead of shifting the view.

### Performance

- Cold start with a restored session is 135–143 ms (budget 150 ms) and an idle
  eight-pane window sits near 0 % CPU. The system font database is read once and
  the two font systems are built on worker threads while the window and adapter
  come up; passive agent detection now reads `libproc`/`sysctl` instead of
  forking `ps` twice per surface per second, which was the idle CPU cost.
- `BERTH_TRACE=1` prints startup phases and repaint times, the release binary
  logs the first frame in milliseconds, and `make perf-trace` records a
  symbolized Time Profiler trace of the launch path.

### Packaging

- `make gui-notarize` signs the bundle and nested binaries with a Developer ID
  and the hardened runtime, submits to `notarytool`, staples the ticket and
  checks it with `spctl`. It needs a paid Apple Developer certificate and stored
  notary credentials; the local build stays ad-hoc signed.

### Features

- Kitty graphics, verified with a real yazi preview: the PTY reader separates
  APC sequences, reassembles chunked transmissions, decodes PNG/RGB/RGBA,
  stores images by id and answers the `a=q` capability probe, so clients pick
  kitty instead of falling back to text. The paint pass renders U+10EEEE
  placeholder cells, decoding the row and column from the kitty diacritics.
- Sixel graphics: the scanner holds DCS sequences, decodes palette registers,
  raster attributes, repeats and carriage-return/newline motion, and paints the
  image at the cursor. Verified with `chafa --format sixel`. A DCS that is not
  sixel is replayed to the parser unchanged.
- `surface.resize` takes the text area in pixels. The program can then query the
  cell size with XTWINOPS (`CSI 16 t`, `CSI 14 t`), which image clients use to
  size a preview.
- The window is smoke-tested under a headless Weston Wayland session in CI,
  alongside the existing Xvfb frame.
- CLI: `rename-workspace`, `group-create`, `group-rename`, `group-color` and
  `group-delete` complete the documented commands.

## 0.13.0

### Features

- Tab drag shows the dragged tab outlined and an insertion line at the drop
  slot.
- Settings has a "Reinstall agent hooks" row that runs the CLI beside the
  window, so a moved app or repo refreshes all three hooks in one step.
- CLI: `new-tab`, `select-workspace`, `focus-pane`, `close-pane`,
  `close-surface`, `rename`, `move-tab` and `save` join the client, closing the
  documented tab/pane/session gaps. The portable window warns at launch when the
  socket runs in `trusted` mode.
- Agent polish: `muteProviders` silences OS notifications per provider while the
  in-app attention stays, `agent.usage` (and `berth agent-usage`) records tokens
  and cost for the sidebar row, and the opencode plugin handles passive surfaces.
- `resumeArgs` adds per-provider arguments to every resume command, the general
  form of `claudeBypassPermissions`.
- Copilot CLI has a hook integration: `berth hooks install copilot` writes
  `~/.copilot/hooks/berth.json`, and `agentStop`/`preCompact`/`sessionEnd` update
  the state and record the session id for `copilot --resume`. The opencode
  plugin now also works for a plain `opencode` in a pane, not just `berth run`.
- Sidebar groups can be created from `+G` (the selected workspace moves into
  the new group), renamed by double-click, and coloured by Shift-click;
  workspace rows render their stored colour and icon, and a workspace with
  several agents shows the count.
- Linux packaging: every archive now contains the `berth` CLI beside
  `berth-gui`, and releases attach a Debian `.deb` (both binaries, desktop
  entry, icon) built in CI. An Arch `PKGBUILD` builds both from the tag.
- `scrollbackLines` configures how many rows of history each surface keeps
  (default 4,000, ceiling 100,000). The history lives in memory in the terminal
  parser and is not part of the snapshot.
- iTerm2 inline images (`OSC 1337 File=...`) render at the cursor cell, scaled to
  the requested cell or pixel size and clipped to the pane. `graphics` (default
  on) controls decoding; the per-surface cache is capped at eight images and
  32 MB. Kitty graphics is next.

### Fixes

- Wide characters (emoji) were cut in half: the continuation cell's background
  was filled after the glyph. Both halves are filled first now, so emoji render
  whole.

## 0.12.1

### Fixes

- `Option+Left` and `Option+Right` move the prompt caret a word at a time by
  sending `ESC b`/`ESC f`, which readline and Claude Code bind. The window sent
  `ESC ESC [D` and no modifier-bearing function key was encoded.
- Named keys now use xterm modifier sequences: `Ctrl+Left` is `CSI 1;5D`,
  `Shift+Left` is `CSI 1;2D`, and arrows, Home/End, Insert/Delete, Page keys and
  F1–F12 carry Shift/Alt/Ctrl instead of being sent unmodified.

## 0.12.0

### Features

- Copy on select: releasing a multi-cell selection writes it to the clipboard,
  and `Super+C` falls back to the last pane that had a selection. Configurable
  with `copyOnSelect` and a Settings row.
- Tabs can be renamed with a user title that OSC does not overwrite, and dragged
  to reorder inside their pane (`surface.rename`, `surface.move`). Workspaces
  and groups rename from the sidebar with a double-click.
- Scrollback search (`Super+F` / `Ctrl+Shift+F`): case-insensitive, with a match
  count, Enter/Shift+Enter to walk matches, scrolling and highlighting each.

## 0.11.4

### Changes

- A restored Claude session now starts with permissions bypassed: Berth appends
  `--permission-mode bypassPermissions` to the resume command.
  `claudeBypassPermissions` in `config.json`, or the Settings row, turns it off
  (default on). The mode is applied at restore time and is not written into the
  saved launch command, so the setting can be reversed later.

## 0.11.3

### Changes

- A resumed agent keeps the flags it was launched with. The provider template
  used to replace the whole command, so
  `claude --allow-dangerously-skip-permissions` came back without it; the saved
  launch argv is now captured for passive agents too and replayed on resume.
  `restoreLaunchFlags` in `config.json`, or the Settings row, turns it off
  (default on).
- A pane whose process exits now closes by default, so `Ctrl+D` in a shell
  removes its tab and the layout reflows instead of leaving a "process exited"
  banner. If it was the last pane in the last workspace, a fresh workspace
  opens. `closeExitedPanes` in `config.json`, or the Settings row, keeps the
  final screen for inspection when set to `false`.

## 0.11.2

### Fixes

- Harnesses started by typing `claude` or `opencode` in a shell now resume after
  a relaunch. Passive detection registered those agents without a resume
  payload, so the `SessionStart` hook's session id failed with
  `unsupported: agent … has no resume payload` and the hook command hid the
  error. The first reported session id now builds a resume payload from the
  surface's cwd, and the hook adds its own `PATH`, because an app launched from
  Finder cannot resolve a provider installed under the user's shell path. Agents
  that still have no resume come back `unknown · not resumed` with no attention
  instead of carrying stale state onto a plain shell.
- Clicking a pane whose program tracks the mouse (Claude Code, opencode, vim)
  now moves keyboard focus to that pane. The press was sent to the program and
  the handler returned before `surface.activate`, so copy and paste kept using
  the previously focused pane.
- `Shift+Enter` sends `ESC CR`, which Claude Code maps to a prompt newline, so a
  partial prompt is no longer submitted when the text spans two lines.
- Arabic, Hebrew and other right-to-left phrases render. A cell was shaped on
  its own, and a right-to-left glyph is placed at the right edge of the shaping
  buffer, so the ink landed outside the cell and the text appeared blank. Whole
  runs are now shaped once and drawn at their own width.
- The opencode plugin's session id now builds a resume payload like the Claude
  hook does, and `copilot` is a known provider (`copilot --resume <ref>`), so it
  is detected and resumes once a session id is recorded.
- `Cmd+Left` and `Cmd+Right` move the prompt caret to the beginning and end of
  the line by sending Home/End. The window used to drop every Super chord that
  was not a configured shortcut.
- Block elements are painted as sub-cell rectangles instead of font glyphs. The
  fallback face drew `█` 11x16 inside a 10x22 cell, so Claude Code's pixel-art
  mascot came out as disconnected bars; it now tiles exactly.

## 0.11.1

### Fixes

- The portable window no longer deadlocks when a tab strip is double-clicked or
  `⌘T` is pressed. `new_tab` held the state lock and then called a helper that
  took the same non-reentrant lock, so the main thread blocked forever and the
  window beachballed. The helper now reads a borrowed state.
- Double-clicking a pane's tab strip opens the tab in **that** pane. The click
  used to resolve the workspace's active surface, which is usually the other
  pane after a split.
- Arrow keys move the settings, palette and attention selection. `overlay_key`
  took the overlay out of `self` and then measured the row count through
  `self.overlay_len()`, which saw `None` and clamped every key press back to the
  first row.
- The portable window repaints when something changes instead of on a 30 Hz
  timer. Pty output, child exits and socket requests wake the loop; the only
  scheduled wakes are the 530 ms cursor blink, bell flash expiry, the autosave
  debounce, the one-second agent poll and the thirty-second stale sweep, none of
  which paints unless something changed. Idle CPU went from a full core to under
  1 %.
- The macOS shell's config save merges into the file instead of rewriting it
  with only the keys it owns, so `fontFamily` and the portable window's keys
  survive a font-size change.

### Portable window

- `fontFamily` in `config.json` takes one name or a list, like Ghostty's
  `font-family`. The first installed name draws the text and the rest fill in
  the glyphs it lacks, so a Nerd Font listed after the text face supplies prompt
  icons and powerline separators. The system monospace stays the last resort.
  The resolved list is logged at startup, with the cell size.
- Glyphs are rasterised into a padded box, so icons and powerline separators are
  no longer clipped at the cell edge, and a face that lacks a character hands
  over to the next family instead of silently borrowing an unrelated font.

## 0.11.0

The portable window closes the remaining daily-use parity with the AppKit shell
and ships as a download.

### Portable window: parity closure

- Input methods: composition is drawn at the cursor and committed text is sent
  on `Ime::Commit`; the candidate window follows the cursor cell. Dead keys and
  CJK input work through the platform IME.
- Selection auto-scrolls when a drag passes the top or bottom of a pane.
- Crash journal: every save appends `journal.ndjson` (last five snapshots, like
  the macOS shell), restore takes the newest of the snapshot and the journal, and
  a clean exit clears it.
- Accessibility: an AccessKit tree publishes the focused pane as a `Terminal`
  node with its visible text and accepts `ReplaceSelectedText`; the window is
  created hidden so the adapter exists before it is shown.
- `Ctrl+C` and `SIGTERM` save the session and exit cleanly instead of losing the
  last changes.
- Releases now attach `berth-gui-<version>-<platform>` archives for Linux
  x86_64, Windows x86_64 and macOS arm64, built by a new CI matrix job.

## 0.10.1

### Fixes

- Windows and Linux build again. `default_socket_path` returned from a
  `cfg(windows)` block and left unreachable code that `-D warnings` rejects, and
  the macOS-only AppleScript helper was dead code elsewhere; both are fixed and
  the workspace now checks and lints for all three targets.
- The terminal answers cursor-position (`CSI 6n`, `CSI 5n`) and device-attribute
  queries. Without the reply a shell can stall before its first prompt, which is
  what Windows CI exposed: the child's first bytes were a cursor query and the
  screen stayed empty.
- The PTY acceptance test re-sends its command on a cold console, waits longer
  and reports the surface's state on failure.
- CI pins the core macOS matrix entry to `macos-15`. GitHub cancelled the
  `macos-latest` job twice with "not acquired by a runner" while the app job on
  `macos-15` ran; on capacity-constrained days that job may still be skipped by
  GitHub, but the pin removes the label ambiguity.

## 0.10.0

Berth now ships a portable window. `berth-gui` runs on Linux, Windows and macOS
from one Rust core, speaks the same protocol as the macOS app, and has closed the
daily-use gaps against it: real text, scrollback, selection, clipboard, mouse
reporting, links, panels and session safety. The macOS app is unchanged apart
from the shared core and its version.

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
- Passive agent detection runs in the window on the macOS cadence: once a second
  over surfaces without an agent, then a stale sweep every 30 seconds. macOS
  process names come from `ps`, so a `claude`/`codex`/`opencode` started inside a
  pane appears as a sidebar agent row.
- The session autosaves 500 ms after the last core event, including changes made
  by another client through the socket.
- `Server::start` probes the endpoint before binding and refuses with
  `AddrInUse` when another Berth is listening, so a second launch can no longer
  unlink a live socket. A handle that did not bind also no longer removes the
  endpoint on drop, which previously made `--mode off` delete a running
  instance's socket.
- `config.json` is now read by the portable window: `fontSize`, `fontScale`,
  `compactSidebar`, `socket` and all 14 `shortcuts`, with a Ctrl-based default
  table outside macOS. `BERTH_CONFIG` overrides the path.
- `--smoke-png` renders a frame to PNG and exits, which is how the visual work
  was checked on macOS; `--smoke-ms` keeps its headless Xvfb role. New tests
  cover font metrics and caching, sidebar ordering and groups, geometry and
  dividers, config parsing, shortcut resolution, selection and paint blending.
- The workspace MSRV moves to 1.89 for `cosmic-text`, and the lints that
  MSRV-aware clippy now raises in existing crates were collapsed to let-chains.

### Portable window: settings, mouse, links and release hardening

- Settings panel (`⌘,` / `Ctrl+,`): font size, display scale, compact sidebar,
  socket mode and all 14 shortcuts. Arrows adjust or cycle, Enter toggles or
  records a chord, Backspace unbinds; changes apply live and are written back to
  `config.json` with unknown keys preserved.
- Mouse reporting to the PTY in X10, UTF-8 and SGR encodings, with modifier
  bits, drags and wheel events. Shift-click keeps local selection inside a TUI.
- `Super`/`Ctrl`-click opens URLs and absolute paths under the pointer; the
  pointer changes over links, dividers, the sidebar edge and terminal text.
- Multi-line paste opens a preview and sends only on Enter.
- Panes show a scrollbar while scrolled; the sidebar scrolls with the wheel and
  rows drag-reorder live through `workspace.move`.
- Agent rows show token counts and cost when the hooks report them; OSC 9/777
  notifications also raise a desktop notification on macOS and Linux.
- Breadcrumbs go to `berth.log` beside the macOS app's log, rotating at 1 MiB.
- CI renders a window frame under Xvfb on Linux and uploads it as an artifact.
- The workspace version now matches the release (`0.10.0`), so `berth-gui
  --version` reports the shipped version.

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
