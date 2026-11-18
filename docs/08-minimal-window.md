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
- `--smoke-ms` and `--smoke-png` render for headless checks; the PNG flag is the
  visual diff tool used while closing the gap.

**Terminal**

- Real text: bold, italic, dim, underline, inverse, wide characters and 256/RGB
  colours, with a block cursor that blinks and hides when the pane is unfocused
  or the view is scrolled.
- Wheel and `Shift+PageUp`/`Shift+PageDown` scroll the 10,000-line scrollback;
  any keystroke returns to the live screen. The cursor disappears while
  scrolled, like Ghostty.
- Drag, shift-drag and double-click selection over the visible view or
  scrollback; `Super+C`/`Ctrl+Shift+C` copies, `Super+V`/`Ctrl+Shift+V` pastes,
  bracketed paste is honoured when the program enables it.
- OSC 0/2 titles reach the core as `surface.title_changed` and become tab labels
  and the window title. OSC 7 sets the cwd, BEL and the visual bell increment
  `surface.bell`, OSC 9;4 reports progress, OSC 52 writes the clipboard, and
  OSC 9/777 create a core notification.
- A child exit is reported once as `surface.exited`; the pane keeps its final
  screen with a red "process exited" banner until the tab is closed.
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
- Clicking outside a panel or focusing another window closes it.

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

- **No settings window.** `⌘,` is bound and prints "no window yet"; the palette
  and attention list are the only panels, and settings are edited in
  `config.json` by hand.
- **One window, one visible workspace.** No multi-window, no detachable panes.
- **No mouse reporting to the PTY.** Wheel scrolling is local; a program that
  enables mouse tracking does not receive clicks or wheel events.
- **No IME preedit.** Committed text works; composition windows do not.
- **No search, links or graphics protocols** (sixel/kitty/iTerm images).
- **No tab drag-reorder, no workspace drag-reorder, no sidebar scrolling.** Rows
  past the bottom are clipped.
- **No divider cursor change**; the grab band is 7 px and the pointer stays an
  arrow.
- **Linux/X11 is the only manually scripted path.** Windows and Wayland are
  compile coverage; macOS runs the same code path and is the development host.
- **Spike, not packaged.** No installer, single-instance handoff or menu bar.

## Next

1. Settings window, the last overlay on an existing chord.
2. Mouse reporting, IME preedit, link open.
3. Workspace drag-reorder and sidebar scrolling.
4. Packaging: single instance, Linux/Windows installers, CI window smoke.
