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
- The sidebar lists workspaces with groups (chevron toggles `group.update`),
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

- Real text: bold, italic, dim, underline, inverse, wide characters and 256/RGB
  colours, with a block cursor that blinks and hides when the pane is unfocused
  or the view is scrolled.
- Wheel and `Shift+PageUp`/`Shift+PageDown` scroll the 10,000-line scrollback;
  any keystroke returns to the live screen. The cursor disappears while
  scrolled, like Ghostty.
- Drag, shift-drag and double-click selection over the visible view or
  scrollback, with auto-scroll when the drag passes the top or bottom; `Super+C`/`Ctrl+Shift+C` copies, `Super+V`/`Ctrl+Shift+V` pastes,
  bracketed paste is honoured when the program enables it, and a multi-line
  paste opens a preview that sends only on Enter.
- Mouse reporting: a program that enables X10, UTF-8 or SGR tracking receives
  presses, releases, drags and wheel events with modifier bits. Shift-click
  always selects locally, so copy still works inside a TUI.
- Links: `Super`-click (macOS) or `Ctrl`-click opens a URL or an absolute path
  under the pointer with the platform handler. The pointer changes shape over
  links, dividers, the sidebar edge and terminal text.
- A thin scrollbar appears in a pane while its view is scrolled, so the position
  in the scrollback is visible.
- OSC 0/2 titles reach the core as `surface.title_changed` and become tab labels
  and the window title. OSC 7 sets the cwd, BEL and the visual bell increment
  `surface.bell`, OSC 9;4 reports progress, OSC 52 writes the clipboard, and
  OSC 9/777 create a core notification.
- A child exit is reported once as `surface.exited`; the pane keeps its final
  screen with a red "process exited" banner until the tab is closed.
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
  on macOS), then sweeps stale agents every 30 seconds. A `claude`, `codex` or
  `opencode` started inside a pane becomes a sidebar agent row.
- The session autosaves 500 ms after the last core event, including changes made
  by the CLI or hooks through the socket, so a crash loses at most one
  interaction.
- Keys go through the headless encoder: printable text, Ctrl/Alt/Shift
  combinations, Enter, Tab, Esc, Backspace, Delete, Insert, arrows, Home/End,
  Page keys and F1–F12.

**Panels**

- `⌘⇧P` opens the command palette: a fuzzy-matched list of all 14 configurable
  actions with their current chord, arrow keys to move, Enter to run, Escape to
  close. `Ctrl+Shift+P` outside macOS.
- `⌘I` opens the attention list: unacknowledged notifications newest first, with
  the level colour, body and workspace. Enter acknowledges the notification,
  selects its workspace and focuses the surface that raised it.
- `⌘,` opens settings: font size, display scale, compact sidebar, socket mode,
  and all 14 shortcuts. Arrows adjust or cycle a value, Enter toggles or starts
  recording a chord, Backspace unbinds. Every change applies live and is written
  to `config.json` with the keys the window does not own preserved.
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
run from a terminal.

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

- **One window, one visible workspace.** No multi-window, no detachable panes.
- **No graphics protocols** (sixel/kitty/iTerm images): `vt100` cannot carry
  them, so images stay with the renderer decision.
- **No search, tab rename or tab reorder.** Tracked on the roadmap, not present
  in the macOS app either.
- **No installers.** The release publishes archives; there is no `.deb`,
  AppImage or Windows installer.
- **Detection is process-name only**, and a Windows notification is not raised.
- **Linux/X11 is the only scripted path.** Windows and Wayland are compile
  coverage; macOS runs the same code path and is the development host.
- **No menu bar or single-instance handoff.** A second launch refuses to start
  when the endpoint is live but does not bring the first window forward.

## Next

1. Search, tab rename and tab reorder (roadmap: they need core support).
2. OS notifications on Windows; dock/taskbar badges.
3. Installers and notarization for the portable archives.
