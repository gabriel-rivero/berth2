# Minimal window (Linux/Windows)

Status: first visible slice. One window, one terminal, no GPU. The split tree,
tabs and sidebar are still core-only.

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

- Creates or restores a session, then shows the selected workspace's active
  surface.
- Renders the visible `vt100` grid with 256-color/RGB foregrounds and
  backgrounds, bold, inverse and a block cursor.
- Routes printable text, Ctrl/Alt/Shift combinations, Enter, Tab, Esc,
  Backspace, Delete, Insert, arrows, Home/End, Page keys and F1–F12 through the
  same key encoder as the headless shell. Cmd/Super is left to the platform.
- Resizes the PTY when the window changes size; the core surface records the
  new grid.
- Starts `berth-server` (unless `--mode off`), so `berth ls`, `berth send`,
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
berth-gui: window ready 960x600 (120x37 cells)
```

A screenshot under Xvfb after typing `echo gui-ready-ok` showed the command and
its output in the window.

## Limits

- **Only the active surface is drawn.** Splits, tabs and panes are represented
  in the core but not laid out; the window follows
  `selected_workspace → active_surface`.
- **No scrollback view.** `surface.read_text` supports it; the renderer always
  shows the live screen.
- **No mouse, selection, clipboard, IME or scroll gestures.**
- **Bitmap font only, ASCII-first.** Unicode falls back to `?`; a real font
  stack (`ab_glyph`/cosmic-text) is a later slice.
- **No child-exit callback.** The PTY dies and the screen freezes; the core is
  not told, so `surface.exited` still does not fire from this shell.
- **Linux/X11 is the only manually tested path.** Windows, macOS and Wayland are
  compile/CI coverage until someone runs them.
- **Spike, not packaged.** No app bundle, installer, single-instance handoff or
  menu bar.

## Next

1. Lay out the split tree: one `vt100` view per leaf, focus follows clicks.
2. Add a tab strip and workspace sidebar from the core snapshot.
3. Scrollback, mouse selection and clipboard.
4. Replace the bitmap font with a real monospace rasterizer.
5. Then decide whether the production renderer stays winit + a terminal grid or
   moves to GTK4 + libghostty-vt.
