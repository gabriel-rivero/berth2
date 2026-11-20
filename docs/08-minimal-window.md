# Portable window (Linux, Windows, macOS)

Status: fourth slice. One window with real text, tabs, dividers, a sidebar with
groups and agent rows, selection, clipboard and scrollback. It is the shell the
portable half of the project is built on, and the place where feature parity
with the macOS app is tracked.

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

Chrome (sidebar, tab labels) uses the same renderer at a smaller size, matching
the macOS window's 12pt labels.

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
  else schedules its own next wake: the cursor blink, bell flash expiry, the
  autosave debounce and the agent polls. An idle window costs under 1 % of a
  core; a screenful of output costs a burst. A child printing without pause
  cannot flood the loop, because at most one wake is in flight at a time.
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
  layout reflows; the last workspace exit opens a fresh one. Set
  `closeExitedPanes` to `false` to keep the final screen with a red "process
  exited" banner instead (then the exit is reported as `surface.exited` and the
  tab stays until you close it).
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
  `opencode` or `copilot` started inside a pane becomes a sidebar agent row.
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
- `Super+F`/`Ctrl+Shift+F` searches the focused pane's configured scrollback
  case-insensitively. The panel shows the match count; Enter and Shift+Enter
  walk the matches, scrolling each into view and highlighting it, and Escape
  closes and clears the highlight.
- Keys go through the headless encoder: printable text, Ctrl/Alt/Shift
  combinations, Enter, Tab, Esc, Backspace, Delete, Insert, arrows, Home/End,
  Page keys and F1–F12. `Shift+Enter` sends `ESC CR`, the sequence Claude Code
  binds for a prompt newline, `Cmd+Left`/`Cmd+Right` send Home/End for the
  beginning and end of the line, and `Option+Left`/`Option+Right` send
  `ESC b`/`ESC f` for word movement. Other modified function keys use the xterm
  `CSI 1;<mod>` form.

**Panels**

- `⌘⇧P` opens the command palette: a fuzzy-matched list of all 15 configurable
  actions with their current chord, arrow keys to move, Enter to run, Escape to
  close. `Ctrl+Shift+P` outside macOS.
- `⌘I` opens the attention list: unacknowledged notifications newest first, with
  the level colour, body and workspace. Enter acknowledges the notification,
  selects its workspace and focuses the surface that raised it.
- `⌘,` opens settings: font size, display scale, compact sidebar, close on exit,
  socket mode, and all 15 shortcuts. Arrows adjust or cycle a value, Enter
  toggles or starts recording a chord, Backspace unbinds. Every change applies
  live and is written to `config.json` with the keys the window does not own
  preserved.
- Long panels scroll to follow the selection. Clicking outside a panel or
  focusing another window closes it.

**Window actions**

Defaults come from the macOS table on macOS and from a Ctrl+Shift table on
Linux and Windows (see [docs/config.md](config.md)); every entry can be
rebound in `config.json`:

split right/down, new tab, close tab, next/previous tab, next/previous pane,
new workspace, workspace 1–9, jump to attention, toggle sidebar, copy, paste,
scroll top/bottom. `Super+1…9` always selects a workspace.

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

- **One window, one visible workspace.** No multi-window, no detachable panes, no pane
  zoom.
- **The protocol floor is otherwise complete.** Synchronized output (DECSET 2026), the
  Kitty keyboard protocol, styled text, OSC 8 hyperlinks, cursor shapes, focus reporting
  and OSC 52 read are supported; OSC 133 shell integration is not. The design is in
  [docs/11-competitive-gaps.md](11-competitive-gaps.md); it is 0.15 work.
- **One palette, no ligatures.** The theme is hard-coded and each cell is shaped alone.
- **No copy mode, paste history or command blocks.** Selection is mouse-only.
- **No installers on macOS or Windows.** Linux has a `.deb` and a `PKGBUILD`; there is no
  AppImage, Flatpak, Windows installer or auto-update.
- **Detection is process-name only**, and a Windows notification is not raised.
- **Linux/X11 is the only scripted path.** Windows and Wayland are compile coverage
  (Wayland now has a smoke); macOS runs the same code path and is the development host.
- **No menu bar or single-instance handoff.** A second launch refuses to start
  when the endpoint is live but does not bring the first window forward.
- **Window geometry is not persisted**; the window opens at 1180×720 each launch.

## Next

Owned by [docs/04-roadmap.md](04-roadmap.md): 0.15 is the protocol floor above, 0.16 the
window model, 0.17 shell integration and recall, 0.18 durability. Platform packaging moved
to 1.0.
