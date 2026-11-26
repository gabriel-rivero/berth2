# Portable window (Linux, Windows, macOS)

Status: through 0.21.3. One window per core window (several can be open), with real text,
tabs, dividers, a sidebar with groups and agent rows, selection, clipboard, scrollback,
shell integration, copy mode, paste history, a quick terminal, wallpaper and native menus
on macOS. It is the shell the portable half of the project is built on, and the place where
feature parity with the macOS app is tracked.

```
berth-core  ──  AppState + dispatch (panes, tabs, workspaces, agents)
    ▲
    │ SurfaceOps
berth-headless  ── portable-pty + vt100 (screen, scrollback, OSC events)
    ▲
berth-gui  ── winit + softbuffer + cosmic-text (window, input, painting)
    ▲
berth-server  ── the same socket/named pipe the CLI and hooks use
```

## Stack

- **winit 0.30** for the window, keyboard, pointer and wheel events.
- **softbuffer 0.4** presents an XRGB buffer. No GPU, shader toolchain or
  graphics driver assumptions.
- **cosmic-text 0.19** shapes and rasterises the system monospace face, with
  font fallback, so Unicode, CJK and colour emoji render. Glyphs are cached per
  cell and tinted at paint time; the cache turns a screen of repeated cells into
  a few hundred rasterised bitmaps.
- **arboard 3** for the system clipboard.
- **A selectable terminal engine.** `terminalBackend` chooses `builtin`
  (`vt100`, the default) or `alacritty` (`alacritty_terminal`, with native
  reflow). The GUI only reads `ScreenView`, so the engine is a `berth-headless`
  concern; `wezterm` and `ghostty` are accepted by config but not built yet. See
  [docs/config.md](config.md#terminal-engine).

Chrome (sidebar, tab labels, menus, status) uses the same renderer at a smaller
size with its own `uiFontFamily`, defaulting to the system sans-serif. The
advances are measured per glyph, so a proportional interface face lays out
correctly.

## What works

**Layout**

- Creates or restores a session, then lays out the selected workspace's
  `PaneNode` tree. Nested splits use the core's weights; each boundary is a
  draggable divider that writes `split.resize`.
- Every pane has a tab strip with one tab per surface, a close cross and an
  active underline. Tabs share the strip equally and truncate their label, so a
  long OSC title cannot hide the tabs after it.
- The sidebar lists workspaces with groups (chevron toggles `group.update`; the
  `+G` button creates a group and moves the selected workspace into it;
  Shift-clicking a workspace or group cycles its colour),
  pinned-first ordering, then attention rank, with the active surface's title,
  the workspace name and cwd, an agent line and an OSC 9;4 progress bar.
- Each window's sidebar lists only the workspaces assigned to it, and a group row
  appears only when that window has a workspace in it; the palette follows the same
  scope. Painting and hit-testing share one builder, so a row that is drawn is
  clickable and a row that is not drawn is not. A workspace with no `window_id` stays
  visible in every window rather than becoming unreachable.
- The sidebar edge drags between 180 and 360 points; `+` and double-clicking
  empty space create a workspace; middle-click closes a workspace or tab.
- The sidebar scrolls with the wheel, and dragging a row reorders workspaces
  live through `workspace.move`.
- `--smoke-ms` and `--smoke-png` render for headless checks; the PNG flag is the
  visual diff tool used while closing the gap, and CI runs `scripts/gui-smoke.sh`
  under Xvfb and uploads the frame.
- Breadcrumbs go to `paths::log_dir()/berth.log`, rotating once at 1 MiB, so a
  crash leaves a trace next to the macOS app's log.

**Terminal**

- Repaints on change, not on a timer. Pty output, child exits and socket
  requests wake the event loop through one coalesced proxy event. Everything
  else schedules its own next wake: the cursor blink while a pane has a blinking
  cursor or blinking text, bell flash expiry, the autosave debounce and the agent
  polls. An idle window costs under 1 % of a core; a screenful of output costs a
  burst. A child printing without pause cannot flood the loop, because at most
  one wake is in flight at a time.
- A column change re-wraps wrapped history instead of cutting it: the grid is captured
  with a newline only where the program broke the line, replayed at the new width, and
  the cursor and input modes are restored. Full-screen programs and panes with images
  keep the plain resize. Hyperlink and underline-colour spans on existing rows are
  dropped, because the style map is keyed to the old layout.
- Real text: bold, italic, dim, underline, inverse, wide characters and 256/RGB
  colours, with a block cursor that blinks and hides when the pane is unfocused
  or the view is scrolled. Right-to-left phrases are shaped once per run, so
  Arabic and Hebrew join and read right to left instead of drawing one isolated
  cell at a time. Block elements (U+2580–U+259F) are filled as exact sub-cell
  rectangles, so pixel art and TUI mascots tile without seams.
- Wheel and `Shift+PageUp`/`Shift+PageDown` scroll the configured scrollback (`scrollbackLines`, 4,000 by default);
  any keystroke returns to the live screen. The cursor disappears while
  scrolled, like Ghostty.
- Drag, shift-drag and double-click selection over the visible view or
  scrollback, with auto-scroll when the drag passes the top or bottom. Releasing
  a selection of more than one cell copies it (`copyOnSelect`, on by default);
  `Super+C`/`Ctrl+Shift+C` copies the focused pane's selection or the last pane
  that had one, `Super+V`/`Ctrl+Shift+V` pastes,
  bracketed paste is honoured when the program enables it, and a multi-line
  paste opens a preview that sends only on Enter.
- Mouse reporting: a program that enables X10, UTF-8 or SGR tracking receives
  presses, releases, drags and wheel events with modifier bits. A horizontal
  gesture, such as a trackpad swipe or a tilt wheel, is reported as xterm
  buttons 6 and 7 on the dominant axis; it never turns into a vertical wheel.
  Programs that do not track the mouse ignore it. A press in
  another pane moves keyboard focus there first, so paste and typing follow the
  click even when the program consumes the mouse. Shift-click always selects
  locally, so copy still works inside a TUI.
- Links: `Super`-click (macOS) or `Ctrl`-click opens a URL or an absolute path
  under the pointer with the platform handler. The pointer changes shape over
  links, dividers, the sidebar edge and terminal text.
- A thin scrollbar appears in a pane while its view is scrolled, so the position
  in the scrollback is visible.
- `wallpaper` draws a picture behind the terminal — `wallpaperFollowDesktop` uses the
  desktop's own on macOS — blended *over* the theme background at `wallpaperOpacity`,
  so the palette stays the dominant term and text keeps its contrast. One image covers
  the window, so a split shows a continuous picture rather than a copy per pane, and
  chrome stays solid. Settings → **Wallpaper image…** opens the platform's own picker.
- iTerm2 inline images (`OSC 1337 File=...`) render at the cursor cell, scaled to
  their requested cell or pixel size, clipped to the pane, and scroll with the
  text. `graphics` turns decoding off; each surface keeps at most eight images
  within 32 MB. Kitty graphics is next, then sixel.
- OSC 0/2 titles reach the core as `surface.title_changed` and become tab labels
  and the window title. Double-click a tab to give it a user title that OSC does
  not overwrite; an empty title falls back to the OSC one. Dragging a tab
  reorders it inside its pane (`surface.move`), and double-clicking a sidebar
  workspace or group row renames it. OSC 7 sets the cwd, BEL and the visual bell increment
  `surface.bell`, OSC 9;4 reports progress, OSC 52 writes the clipboard, and
  OSC 9/777 create a core notification.
- A pane closes when its process exits, so `Ctrl+D` removes the tab and the
  layout reflows. When the last pane in the last workspace exits, the window
  closes and the app saves and quits, matching the macOS shell, instead of
  leaving an empty frame behind. Set `closeExitedPanes` to `false` to keep the
  final screen with a red "process exited" banner instead (then the exit is
  reported as `surface.exited` and the tab stays until you close it).
- OSC 9/777 notifications reach the desktop through `osascript` on macOS and
  `notify-send` on Linux, in addition to the attention list. Agent rows show
  token counts and cost when the hooks report them.
- Input methods: composition is shown at the cursor with an underline and the
  committed text is sent on `Ime::Commit`; the candidate window follows the
  cursor cell. Dead keys and CJK input work through the platform IME.
- Accessibility: the window publishes an AccessKit tree with the focused pane as
  a `Terminal` node carrying its visible text, refreshed when the text changes.
  Screen readers can read the screen and send text through
  `ReplaceSelectedText`.
- `Ctrl+C` and `SIGTERM` save the session and exit cleanly, like closing the
  window.
- Passive agent detection runs once a second over surfaces without an agent,
  using the foreground process group (`tcgetpgrp` plus `/proc` on Linux and `ps`
  on macOS), then sweeps stale agents every 30 seconds. A `claude`, `codex`,
  `opencode`, `copilot` or `cmd` started inside a pane becomes a sidebar agent row.
  Agent rows show token counts and cost when a client reports them, and
  `muteProviders` silences desktop notifications for chosen providers while
  keeping the row, ring and attention list. A
  resumed agent keeps the launch flags it had, such as
  `claude --allow-dangerously-skip-permissions`; `restoreLaunchFlags` turns
  that off. A restored Claude session also starts with permissions bypassed by
  default (`--permission-mode bypassPermissions`); `claudeBypassPermissions`
  turns that off.
- The session autosaves 500 ms after the last core event, including changes made
  by the CLI or hooks through the socket, so a crash loses at most one
  interaction.
- `Super+F`/`Ctrl+Shift+F` searches every pane of the visible workspace, not just
  the focused one. The query is literal and case-insensitive unless the regex toggle
  (`Cmd/Ctrl+R`) is on; `Cmd/Ctrl+W` restricts it to whole words on the same character
  class a double-click selects. The panel shows the match count and the pane each hit
  came from; Enter and Shift+Enter walk the matches, scrolling each into view, focusing
  its pane and highlighting it in the accent while every other hit stays tinted.
  Escape closes the panel and clears the tints; the mode chips and a bad pattern's error
  are in the title.
- Keys go through the headless encoder: printable text, Ctrl/Alt/Shift
  combinations, Enter, Tab, Esc, Backspace, Delete, Insert, arrows, Home/End,
  Page keys and F1–F12. `Shift+Enter` sends `ESC CR`, the sequence Claude Code
  binds for a prompt newline, `Cmd+Left`/`Cmd+Right` send Home/End for the
  beginning and end of the line, and `Option+Left`/`Option+Right` send
  `ESC b`/`ESC f` for word movement. Other modified function keys use the xterm
  `CSI 1;<mod>` form.

**Panels**

- `⌘⇧P` opens the command palette: a fuzzy-matched list of all 34 configurable
  actions with their current chord, arrow keys to move, Enter to run, Escape to
  close. `Ctrl+Shift+P` outside macOS.
- `⌘I` opens the attention list: unacknowledged notifications newest first, with
  the level colour, body and workspace. Enter acknowledges the notification,
  selects its workspace and focuses the surface that raised it.
- `⌘,` opens settings: terminal font, window font, font size, display scale,
  theme, window opacity, wallpaper source and strength, bell, shell, VT engine,
  compact sidebar, close on exit, restore launch flags, Claude bypass, copy on
  select, hook reinstall, socket mode, and all 34 shortcuts. Arrows adjust or
  cycle a value, Enter toggles or starts recording a chord, Backspace unbinds.
  Every change applies live and is written to `config.json` with the keys the
  window does not own preserved.
- The menu bar has `Berth`, `File`, `Edit`, `Terminal`, `View`, `Window` and
  `Help` on every platform, and every row runs the same command the chord table
  does. On macOS they are real `NSMenu`s in the system menu bar, appended to the
  application menu winit installs, and each row carries its chord as an AppKit
  key equivalent so the system draws the shortcut beside the label. The app
  menu's Quit is routed through Berth so it saves the session before exiting, and
  the window reserves no row for a painted bar. On Linux and Windows the bar is
  painted into the frame because winit exposes no menu API there: click a title,
  hover or arrow to move, click or Enter to run, Escape to close, with each row
  showing its chord.
- Long panels scroll to follow the selection. Clicking outside a panel or
  focusing another window closes it.

**Window actions**

All 34 entries can be rebound in `config.json`. Defaults come from the macOS table
on macOS and from a Ctrl+Shift table on Linux and Windows (see
[docs/config.md](config.md)):

split right/down, new tab, close tab, next/previous tab, next/previous pane, focus
pane left/right/up/down, new workspace, new window, next window, zoom pane, broadcast
input, duplicate tab, restart tab, filter sidebar, palette, jump to attention,
attention list, toggle sidebar, settings, search, previous/next prompt, copy mode,
paste history, quick terminal, and zoom in/out/reset. `Super+1…9` always selects a
workspace, and `Super+C`/`Super+V` copy and paste.

## Run it

```sh
cd core
cargo run -p berth-gui
# or: make gui && ./target/release/berth-gui
```

Options: `--socket`, `--mode off|local|trusted`, `--token`, `--no-restore`,
`--smoke-ms <ms>`, `--smoke-png <path>`. `--mode` defaults to the `socket` key in
`config.json`, then `local`.

Releases also ship the portable window as `berth-gui-<version>-<platform>`
archives (Linux x86_64, Windows x86_64, macOS arm64): single unsigned binaries,
run from a terminal. macOS also gets `…-macos-aarch64.app.zip`, the same binary
in an ad-hoc signed `Berth Portable.app` you can double-click.

Running it next to the macOS app needs a separate endpoint, because both want
the same socket path and a second server refuses to steal it:

```sh
BERTH_DATA_DIR=/tmp/berth-rust BERTH_SOCKET_PATH=/tmp/berth-rust/berth.sock \
  build/BerthGui.app/Contents/MacOS/berth-gui --mode trusted
```

`make gui-app` builds the bundle locally.

A one-frame check, isolated from any running Berth:

```sh
BERTH_DATA_DIR=/tmp/berth-gui BERTH_SOCKET_PATH=/tmp/berth-gui/berth.sock \
  ./target/debug/berth-gui --mode trusted --no-restore \
  --smoke-ms 1500 --smoke-png /tmp/berth-gui/frame.png
```

On macOS that PNG contains the shell's own prompt, because the window runs a real
PTY. `make gui-smoke` does the same under `xvfb-run` on Linux and skips cleanly
when Xvfb is absent.

## Limits

These are the window's limits; [docs/09-known-gaps.md](09-known-gaps.md) has the
full audited inventory.

- **Multiple windows and pane zoom are in.** `window.create/list/close/select`,
  per-window sidebar selection, persisted geometry and a maximised pane all come from the
  core, so both shells share them. Tab tear-off, broadcast input and geometric focus run on
  the same model; `quick` windows are excluded from the session file.
- **The protocol floor is otherwise complete.** Synchronized output (DECSET 2026), the
  Kitty keyboard protocol, styled text, OSC 8 hyperlinks, cursor shapes, focus reporting,
  OSC 52 read and OSC 133/633 prompt marks are supported. A shell that emits prompt marks
  gets a gutter beside the grid: a green tick per prompt, red when the command above it
  failed, and a dim mark where output started. Click a tick to bring that command to the
  top of the pane, or walk them with the `previousPrompt`/`nextPrompt` chords.
- **System and fixed palettes.** `theme` follows the OS appearance (`system`,
  the default) or forces `berth-dark`/`berth-light`, switching live on a
  system appearance change; `opacity` fades the window, and the zoom chords
  resize the font; ligatures render through shaping runs.
- **Copy mode and paste history are in; command blocks are not.** `Cmd+Shift+C` opens a
  vi-style modal selection (`h`/`j`/`k`/`l`, `w`/`b`/`e`, `0`/`$`, `g`/`G`, `Ctrl+D`/`U`,
  counts, `v`, `y`), and `Cmd+Shift+V` picks from the last twelve copies, held in memory
  for the session only. There are no command blocks.
- **No installers on macOS or Windows.** Linux has a `.deb` and a `PKGBUILD`; there is no
  AppImage, Flatpak, Windows installer or auto-update.
- **Detection is process-name only**, and a Windows notification is not raised.
- **Linux/X11 is the only scripted path.** Windows and Wayland are compile coverage
  (Wayland now has a smoke); macOS runs the same code path and is the development host.
- **Single-instance handoff is absent.** A second launch finds the live endpoint, logs
  `endpoint busy` and exits with advice to close the other instance or pass `--socket`;
  it does not raise the running window.
- **Window geometry is persisted** as a protocol `Window.bounds` value and restored on
  launch.

## Next

Owned by [docs/04-roadmap.md](04-roadmap.md): 0.15 was the protocol floor, 0.16 the
window model, 0.17 shell integration and recall, and 0.18 durability and agent depth.
Platform packaging moved to 1.0.
