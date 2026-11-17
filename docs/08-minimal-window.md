# Minimal window (Linux/Windows)

Status: third visible slice. One window, the core's split tree, per-pane tab
strips and a workspace sidebar, no GPU.

This is the second half of the Linux shell spike. It reuses `berth-headless`'s
PTY host and `berth-server`, and adds a renderer and input routing:

```
berth-core  ──  AppState
    ▲
    │ SurfaceOps
berth-headless  ── portable-pty + vt100
    ▲
berth-gui  ── winit window + softbuffer bitmap grid
    ▲
berth-server  ── the same socket/named pipe the CLI uses
```

## Stack

- **winit 0.30** for the window and keyboard events.
- **softbuffer 0.4** for a CPU-presented XRGB buffer. No GPU, shader toolchain or
  graphics driver assumptions.
- **font8x8** for an 8×16 bitmap cell. It is deliberately crude; it keeps the
  first slice dependency-light and readable.

GTK4 + libghostty-vt remains the planned production renderer, but this machine
has no GTK4 development headers, no libghostty-vt, and until Xvfb was used, no
display. winit/softbuffer lets the window path be built and smoke-tested now
without adding system packages. The terminal engine and protocol do not depend
on that choice.

## What works

- Creates or restores a session, then lays out the selected workspace's
  `PaneNode` tree. Every leaf gets its own `vt100` view, sized to its rectangle;
  nested horizontal and vertical splits use the core's weights.
- Draws a flat workspace sidebar from the core snapshot. Clicking a row selects
  that workspace and focuses its active surface; unread workspaces get a dot.
- Draws a tab strip whenever a pane has more than one surface. Clicking a tab
  activates it; the terminal grid shrinks to make room.
- `Ctrl+Shift+T` opens a tab in the focused pane, `Ctrl+Shift+W` closes it (never
  the pane's last tab), and `Ctrl+PageUp`/`Ctrl+PageDown` cycle tabs.
- Clicking a pane focuses its active surface (and broadcasts
  `pane.focus_changed`/`workspace.changed`). The focused pane gets a blue border
  and the only block cursor.
- Renders each visible `vt100` grid with 256-color/RGB foregrounds and
  backgrounds, bold, inverse and a block cursor.
- Routes printable text, Ctrl/Alt/Shift combinations, Enter, Tab, Esc,
  Backspace, Delete, Insert, arrows, Home/End, Page keys and F1–F12 through the
  same key encoder as the headless shell, to the focused surface. Cmd/Super is
  left to the platform.
- Resizes every visible PTY to its pane grid when the window or tree changes;
  the core surface records the new size.
- Starts `berth-server` (unless `--mode off`), so `berth split`, `berth send`,
  hooks and `session.save` work against the window.
- Saves a snapshot on close.

## Run it

On a desktop session:

```sh
cd core
cargo run -p berth-gui
# or: make gui && ./target/release/berth-gui
```

Options mirror the headless shell: `--socket`, `--mode off|local|trusted`,
`--token`, `--no-restore`. `--smoke-ms 1500` renders a frame, saves, and exits;
that is the headless/Xvfb check:

```sh
Xvfb :99 -screen 0 1280x800x24 &
DISPLAY=:99 BERTH_DATA_DIR=/tmp/berth-gui \
  cargo run -p berth-gui -- --mode trusted --no-restore --smoke-ms 1500
```

Or from the repo root, `make gui-smoke` builds the release binary and runs the
same check under `xvfb-run` (it skips cleanly when Xvfb is absent).

On this machine the smoke run prints:

```
berth-gui 0.8.0 · socket /tmp/berth-gui/berth.sock · auth trusted
berth-gui: window ready 960x600 (1 pane(s))
```

A screenshot under Xvfb after typing `echo gui-ready-ok`, then running
`berth split <surface> --direction right`, showed both terminals side by side
with independent output and a focus border. Clicking the left pane switched
`berth active` back to its surface, and typing went to the clicked pane.

With two workspaces in the snapshot, the sidebar listed both and clicking a row
switched the visible split tree. `Ctrl+Shift+T` twice produced three tabs with
the third active; clicking the first tab activated it, Ctrl+PageDown moved to
the second, and Ctrl+Shift+W removed it.

## Limits

These are the window's limits; [docs/09-known-gaps.md](09-known-gaps.md) has the
full audited inventory across the portable and macOS shells.

- **One workspace visible at a time.** The sidebar lists workspaces flat: no
  group headers, collapse or scrolling, and rows that do not fit are clipped.
  Creating, renaming and closing workspaces still goes through the CLI/protocol.
- **Tabs do not overflow or reorder.** Very many tabs run out of strip width.
  Labels come from the surface title, falling back to the cwd basename plus a
  number; there is no rename or drag-reorder UI.
- **No scrollback view.** `surface.read_text` supports it; the renderer always
  shows the live screen.
- **No selection, clipboard, IME or scroll gestures.** Mouse input is limited to
  panes, tabs and sidebar rows. Splits are created through the protocol
  (`berth split`) or another client; there are no split keybindings yet.
- **No divider dragging.** `pane.resize`/`split.resize` exist in the core, but
  the window does not hit-test the dividers.
- **Bitmap font only, ASCII-first.** Unicode falls back to `?`; a real font
  stack (`ab_glyph`/cosmic-text) is a later slice.
- **No child-exit callback.** The PTY dies and the screen freezes; the core is
  not told, so `surface.exited` still does not fire from this shell.
- **Linux/X11 is the only manually tested path.** Windows, macOS and Wayland are
  compile/CI coverage until someone runs them.
- **Spike, not packaged.** No app bundle, installer, single-instance handoff or
  menu bar.

## Next

1. Divider dragging wired to `pane.resize`/`split.resize`, plus split/close
   keybindings.
2. Sidebar groups and workspace create/close, tab overflow and renaming.
3. Scrollback, mouse selection and clipboard.
4. Replace the bitmap font with a real monospace rasterizer.
5. Then decide whether the production renderer stays winit + a terminal grid or
   moves to GTK4 + libghostty-vt.
