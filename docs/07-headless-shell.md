# Headless shell (Linux/Windows spike)

Status: first slice landed. Real PTYs, a VT screen and the full protocol run
without a window, and the same host backs the portable window — see
[docs/08-minimal-window.md](08-minimal-window.md).

This is the "Linux shell spike" from [docs/04-roadmap.md](04-roadmap.md), split
so the expensive, unproven part — a portable terminal host behind `SurfaceOps`
— is testable before any renderer exists.

## What runs today

`berth-headless` is a small daemon:

```
berth-core  ──  AppState + dispatch
    ▲
    │ SurfaceOps
berth-headless ── portable-pty (PTY/ConPTY) + vt100 (screen + scrollback)
    ▲
berth-server ── hello, auth, subscriptions, replay, backpressure
    ▲
Unix socket / named pipe  ←  berth CLI, hooks, any client
```

- One PTY per surface, spawned with the `BERTH_SURFACE_ID` and
  `BERTH_WORKSPACE_ID` the core injects. `BERTH_TOKEN` and
  `BERTH_SOCKET_PATH` are injected too, so `berth` and hooks work inside the
  shell unchanged.
- `surface.send_text` writes raw bytes. `surface.send_key` encodes the protocol's
  synthetic names (`enter`, `tab`, `escape`, arrows, `home`/`end`, page keys,
  `f1`–`f12`, single characters, `ctrl`/`alt`/`shift`). Unknown names are sent
  literally, like the macOS shell's fallback.
- `surface.read_text` renders the visible screen, or the whole scrollback
  (10,000 lines per surface), through `vt100`. `max_bytes` truncates on a
  character boundary.
- `surface.resize` updates the kernel PTY and the parser.
- `pid_info` asks the PTY master for the foreground process group
  (`tcgetpgrp`) and reads `/proc/<pid>/{comm,cmdline}` on Linux, feeding the
  passive agent detector.
- `session.save` writes `<data>/session.json` atomically (`0600` on Unix). Boot
  restores it and respawns the terminals. Scrollback is not part of a snapshot,
  matching the macOS app.
- `local`, `trusted` and `off` auth modes; `local` loads or creates the token
  with the same code as the macOS server.

The acceptance test is `core/crates/berth-headless/tests/headless.rs`: it starts
a real server, creates a workspace (which spawns a real shell), sends
`echo berth-spike-ok` plus Enter, polls `surface.read_text` until the output
appears, resizes, and closes the workspace. It runs in the Linux/macOS/Windows CI
matrix with no display and no Ghostty.

## Try it

```sh
cd core
cargo build -p berth-headless -p berth-cli

# A throwaway instance, no token prompt:
BERTH_DATA_DIR=/tmp/berth-spike \
BERTH_SOCKET_PATH=/tmp/berth-spike/berth.sock \
  ./target/debug/berth-headless --mode trusted

# In another shell, with the same BERTH_SOCKET_PATH:
./target/debug/berth new --name spike
SID=$(./target/debug/berth active)
./target/debug/berth send "$SID" "echo hello from berth"
./target/debug/berth send-key "$SID" enter
./target/debug/berth read "$SID"
./target/debug/berth split "$SID" --direction right
./target/debug/berth snapshot
```

Use `--mode local` for the normal token path, `--no-restore` to ignore a saved
session, and `--no-workspace` to boot empty.

## Why a headless slice first

- The portability seam is `SurfaceOps`, not GTK. If PTY lifecycle, resize,
  readback, key encoding and session restore are wrong, a renderer only makes
  the bugs prettier.
- It runs in CI and on a headless box, so the protocol criterion is checked on
  every commit. A GTK window is a manual check.
- A future GUI shell can reuse `PtyHost` as-is and add a view. The renderer
  decision (GTK4 + `libghostty-vt`, or a Rust window + `vt100` grid) does not
  block the engine.

## Stack decision for the renderer

The roadmap target remains **GTK4 + libghostty-vt** with a minimal renderer. It
was not built in this slice because the development box has no GTK4 development
headers, no `libghostty-vt`, and no display; adding an unbuildable dependency
would have broken `cargo test --workspace`.

`vt100` is intentionally the spike's screen model: pure Rust, no C, and good
enough for readback and tests. A GUI can either:

1. render the `vt100` grid (fastest path to pixels, but a second terminal
   implementation next to Ghostty), or
2. keep `PtyHost` for process/lifecycle and feed the PTY stream to
   `libghostty-vt` for the renderer (matches the macOS engine, more bootstrap).

Do not decide from the spike alone; build the smallest window that can show a
prompt and type into it, then compare input latency and packaging cost.

## Known gaps

This page lists the portable shell's current limits. The full audited inventory
across the GUI, headless shell, core/server/CLI and macOS app is
[docs/09-known-gaps.md](09-known-gaps.md).

- **Exited children are polled.** `SurfaceOps` still has no `surface_exited`;
  the host exposes `is_alive` and `berth-gui` dispatches `surface.exited`, but a
  different shell must poll the same way.
- **Terminal events are polled, not pushed.** `screen_meta` carries OSC title,
  cwd, bell count, progress, clipboard and notification requests; a shell reads
  it after processing output.
- **No mouse reporting or IME.** The host encodes keys only; a GUI must send
  mouse escapes itself.
- **Windows/macOS PTY paths are compile-tested and CI-tested, not manually
  smoke-tested.** ConPTY output and key encoding deserve a real Windows run.
- **No graceful shutdown hook.** `session.save` is explicit; SIGTERM does not
  write a snapshot.
- **`berth-headless` is a spike binary, not a packaged service.**

## Next slice

1. ~~A minimal window that renders one `vt100` screen and forwards keys.~~
   Landed as `berth-gui`; see docs/08-minimal-window.md.
2. ~~Add the core's split tree to that window: one surface per leaf, focus
   routing.~~ Landed in `berth-gui`: weighted horizontal/vertical layout and
   click-to-focus.
3. ~~Add a tab strip and workspace sidebar.~~ Landed in `berth-gui`: per-pane
   tabs with click/shortcut switching and a clickable workspace list. Divider
   dragging remains.
4. ~~Add divider dragging, scrollback, selection and clipboard.~~ Landed in
   `berth-gui`, along with OSC events and the exit banner.
5. Decide `vt100` vs `libghostty-vt` from measured input latency and packaging.
6. Add the child-exit callback to `SurfaceOps`, then wire agent exit states
   without polling.
