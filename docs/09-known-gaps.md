# Known gaps and missing behavior

Last audited: 2026-11-17, code at commit `76577c7` (split tree, tabs,
sidebar). This is the honest inventory of what is **not** in the tree. It
separates missing UI, missing platform work, and deliberate non-goals so the
three do not get confused.

The portable shell is a spike that has become usable; it is not feature parity
with the macOS app, and the macOS app itself still has the gaps listed in
[docs/05-build.md](05-build.md#known-gaps) and
[docs/04-roadmap.md](04-roadmap.md).

## 1. `berth-gui` (Linux/Windows window)

### Terminal rendering

- **ASCII bitmap font only.** `font8x8` renders an 8×16 cell; Unicode without a
  glyph becomes `?`. No wide characters, combining marks, emoji, grapheme
  clusters or script shaping.
- **No font stack, sizing or zoom.** Cell size is fixed at `8×16 × scale
  factor`; `config.json` font/display settings are ignored.
- **Limited attributes.** Bold is a double-draw; there is no italic, underline,
  strikethrough, dim, blink, underline color or hyperlink handling.
- **No scrollback view.** `vt100` keeps 10,000 lines and
  `surface.read_text --scrollback` can read them, but the window always paints
  the live screen. No wheel, page keys or scrollbar scroll the view.
- **No bell or notification presentation.** OSC bell counts, OSC 9;4 progress
  and desktop notifications are not shown; only the core records them.
- **No OSC title/cwd updates.** Tab labels and the window title never learn a
  shell's title or cwd; labels fall back to the cwd basename.
- **No search**, link detection, URL opening, text selection, shell-integration
  prompt marks or graphics protocols (sixel/kitty/iTerm images).

### Input and clipboard

- **No copy/paste or selection.** Ctrl+C is sent to the PTY as SIGINT; there is
  no clipboard read/write path, bracketed paste or paste confirmation. `Ctrl+V`
  is not special.
- **No mouse reporting to the PTY**, wheel scrolling, drag selection,
  right/middle click, double-click word selection or drag-and-drop.
- **No IME preedit.** Committed `event.text` works, but there is no composition
  window or preedit display; dead keys depend on the platform.
- **No configurable keybindings.** The only window shortcuts are
  `Ctrl+Shift+T` (new tab), `Ctrl+Shift+W` (close tab) and
  `Ctrl+PageUp/PageDown` (cycle tabs). New workspace, split, close pane and focus
  movement have no UI shortcut.
- **No accessibility.** The window is one painted buffer: no accessibility tree,
  screen-reader labels, high-contrast mode or keyboard traversal of the sidebar
  and tabs.

### Layout, workspaces and tabs

- **One window, one visible workspace.** No multiple windows, detachable
  surfaces or overview.
- **Sidebar is read-only and flat.** It lists workspaces in core order, with no
  group headers, collapse, colors, icons, pinning, drag reorder or scrolling;
  rows that do not fit are clipped. Creating, renaming, grouping, reordering and
  closing workspaces still requires the CLI/protocol.
- **Tabs are minimal.** A tab strip appears only when a pane has more than one
  surface. There is no overflow/scroll, close button, context menu, rename,
  drag-reorder or tab-creation button; shortcuts and clicks are the only
  controls.
- **No divider dragging.** `pane.resize` and `split.resize` exist in the core;
  the window does not hit-test or drag dividers.
- **No split/pane controls.** Splits and closes are created through
  `berth split`/the protocol; there are no split keybindings, pane close button,
  maximize/zoom, or swap/move pane operations.
- **No spatial focus navigation.** Focus is click- and tab-driven; there is no
  arrow-key movement between rectangles.
- **No command palette, menus, settings UI, status bar or onboarding.**

### Agents, attention and notifications

- **No agent UI.** Agent state, attention, progress, token counts and
  notifications exist in the core but are not rendered. The sidebar only shows a
  workspace unread dot.
- **No notification path.** `notify_os` writes `berth: notification …` to stderr;
  there are no OS banners, sounds, or a notification list/popover.
- **No agent detection loop in the GUI.** Passive detection is available through
  the protocol and `pid_info`; the window does not call it or surface results.
- **No bell/progress badges.**

### Session and lifecycle

- **No autosave/debounce.** The snapshot is written on window close,
  `session.save`, and the smoke exit. A crash or SIGTERM loses changes since the
  last explicit save.
- **The portable shells do not write `journal.ndjson`.** `berth-headless`'s
  `save_session` writes only `session.json`; the crash-tail journal is a
  macOS-store behavior, not a portable-shell behavior.
- **Scrollback is not restored** (intentional, matching the macOS app). Restore
  respawns PTYs with cwd and rebuilds the split/tab tree.
- **No PTY exit callback.** `SurfaceOps` has no child-exit method. When a shell
  exits, the screen freezes and the core is not told, so `surface.exited` and
  `agent.errored` do not fire from this shell.
- **Hidden tabs keep stale sizes** until activated and resized on the next
  frame.
- **No single-instance guard.** On Unix, a second `berth-gui` can unlink the
  existing socket and bind the same path; it does not detect or hand off to the
  first instance. Windows named pipes have the same collision risk.
- **No logs/diagnostics** from the window beyond stdout/stderr; `paths::log_dir`
  is unused by the portable shells.

### Platform and distribution

- **Linux/X11 is the only manually tested path.** Wayland builds but has not
  been run. Windows and macOS are compile/CI coverage for the Rust crates, not
  GUI smoke tests.
- **CI does not run `make gui-smoke`.** The core matrix runs fmt, clippy and
  tests; the GUI's geometry/label unit tests run, but no window is created in CI.
- **No Linux/Windows packaging.** No `.deb`/AppImage/Flatpak, Windows installer,
  `.app` wrapper, code signing/notarization, auto-update, taskbar/dock
  integration, or single-instance handoff.
- **HiDPI and multi-monitor** are only lightly exercised through winit's scale
  factor; no mixed-DPI or monitor-hotplug validation.
- **No desktop notifications, tray icon, menu bar or system theme integration.**

## 2. `berth-headless` (portable shell)

- Spike binary, not a packaged service; no GUI, tray or window.
- Fixed 10,000-line scrollback; no scrollback restore.
- No child-exit callback (same `SurfaceOps` gap), no bell/OSC title/cwd.
- `notify_os` prints to stderr; no desktop notification.
- No graceful SIGINT/SIGTERM snapshot; `session.save` is explicit.
- Linux/Unix PTYs are manually tested; Windows ConPTY and macOS PTYs are
  CI-only.
- `send_text` is deliberately raw; clients must send `\r` or use `send-key
  enter`.
- Key encoding covers the documented named keys, not every xterm modifier
  combination or kitty keyboard protocol.
- No package, service unit, autostart or log rotation.

## 3. Portable core, transport, server and CLI

- **Local endpoints only.** Unix domain socket or Windows named pipe; no TCP,
  TLS, SSH tunnel helper or remote transport.
- **Token auth is the whole authorization model.** `local` needs a token for
  mutations; `trusted` bypasses it; `off` denies. There is no peer-UID/peer-SID
  check, per-client identity, audit trail, rate limit or documented client cap.
- **`BERTH_TOKEN` is injected into child environments**, so processes in a
  terminal can read the token; `trusted` mode intentionally has no token check.
- **Socket ownership is not exclusive.** `bind` removes a stale socket file and
  rebinds; there is no lockfile/singleton handshake, so two servers can race for
  the same path.
- **CLI surface is incomplete.** No `tab`/`surface.create`, `session.save`,
  `workspace.select`, pane focus/close or workspace rename command; those exist
  only in the protocol. `berth schema` is documentation, not a typed client.
- **No client library** beyond the Rust crates and the generated Swift bindings.
- **No protocol version negotiation beyond `protocol: 1`** in hello; mismatches
  are not rejected with a dedicated error path.
- **No journal writing from portable shells**, as above.
- **Windows named-pipe path is per-user but still one global name**; multiple
  profiles/instances share the collision risk.
- **No Windows service, Linux systemd unit, macOS launch agent or autostart.**

## 4. macOS app (existing, not regressions)

The Swift/AppKit app still uses its own `SocketServer.swift`/
`CoreHandler.swift`; it has not been migrated onto `berth-server` through
UniFFI. Known gaps already tracked in the macOS docs:

- Group management UI (create/rename/color) is absent even though
  `group.create/update/delete` exist.
- Geometric focus: `⌘]`/`⌘[` walk tree order; arrow-key spatial navigation is
  missing.
- Agent mute and multi-agent workspace rows are incomplete; banners have no
  per-provider mute.
- Journal stores whole snapshots, not replayable structural events; a crash can
  lose the last ~500 ms.
- No headless app-test loop for the real GUI; `scripts/m2-acceptance.sh` needs
  the app window.
- Socket hardening: no peer-UID check, token is visible to children, `trusted`
  bypasses auth.
- No Instruments profile run against the §9 budgets.
- Scrollback is intentionally not restored.

## 5. Explicit non-goals (intentional, not gaps)

From [docs/01-vision.md](01-vision.md#4-non-goals-for-v1):

- Browser panes, cookie import, agent-driven web automation.
- SSH workspaces, remote tmux mirroring, cloud VMs.
- iOS companion, push notifications, teams/multiplayer.
- Plugin marketplace, TUI mode, custom command DSLs.
- Windows/Linux GUI parity as a v1 commitment (the portable shell is a spike;
  production packaging is undecided).
- Feature parity with cmux.

Deferred until after v1: webview panes, cloud, plugins, telemetry, auto-update,
notarized distribution, and a production Linux/Windows renderer choice
(winit+`vt100` vs GTK4+`libghostty-vt`).

## 6. Coverage summary

| Surface | Automated | Manual / CI |
| --- | --- | --- |
| `berth-core`, `berth-transport`, `berth-server` | Unit + integration tests | Linux local; Ubuntu/macOS/Windows CI |
| `berth-cli` | Integration round-trip | Linux local; CI |
| `berth-headless` | PTY integration test (spawn/type/read/resize/close) | Linux local; CI |
| `berth-gui` | Geometry + tab-label unit tests | Xvfb smoke + xdotool clicks/keys locally; **no CI window** |
| macOS app | Swift tests + build in CI | manual daily use |

## 7. Recommended order

1. **Correctness/lifecycle first:** child-exit callback, OSC title/cwd, snapshot
   debounce, single-instance guard.
2. **Daily terminal basics:** scrollback view, selection/clipboard, mouse
   reporting and wheel, real font rasterizer.
3. **Layout controls:** divider dragging, split/close/focus shortcuts, tab
   overflow/close buttons, sidebar groups and workspace operations.
4. **Attention UI:** agent rows, notifications, bell/progress badges.
5. **Packaging and CI:** run `gui-smoke` on Linux CI, add Wayland/Windows smoke,
   decide the production renderer, then package.
