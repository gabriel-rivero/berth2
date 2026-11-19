# Known gaps and missing behavior

Last audited: 2026-11-19, code at the 0.11.3 release: real text, scrollback,
selection with auto-scroll, clipboard, mouse reporting, IME, links, OSC events,
divider and sidebar dragging, the palette, attention and settings panels, agent
detection, autosave, journal, accessibility, logs and per-platform archives. This is the
honest inventory of what is **not** in the tree. It separates missing UI, missing
platform work, and deliberate non-goals so the three do not get confused.

The portable window is no longer a spike: it renders real text and is usable as a
daily terminal. It is still not feature parity with the macOS app, and the macOS
app itself has the gaps listed in
[docs/05-build.md](05-build.md#known-gaps) and [docs/04-roadmap.md](04-roadmap.md).

## 1. `berth-gui` (portable window)

### Terminal rendering

- **One font family, no ligatures.** `fontFamily` picks the face (first
  installed name wins, else system monospace) at `fontSize` × `fontScale`. There
  is no weight, line-height or zoom control, and no per-font metric tuning.
  Ligatures cannot render: each cell is shaped and cached on its own.
- **No text effects beyond bold/italic/dim/underline.** No strikethrough, blink,
  underline colour, hyperlinks or per-cell font changes.
- **No scrollbar.** Wheel and Shift+Page keys scroll, but nothing shows how far
  into the scrollback the view is.
- **No audible bell, dock badge or per-provider mute.** Bells flash a pane ring;
  OSC 9/777 notifications also reach the desktop on macOS and Linux.
- **No search**, no graphics protocols (sixel/kitty/iTerm images), and no
  hyperlink (`OSC 8`) hover styling beyond the text scan.


### Input and clipboard

- **No selection auto-scroll.** Dragging past the top or bottom of a pane does
  not extend the view, and words cannot be dragged over.
- **Accessibility covers the terminal only.** The focused pane is a `Terminal`
  node with its text; the sidebar, tabs and panels are not in the tree, and there
  is no high-contrast mode.
- **No settings window.** `⌘,` prints "no window yet"; font size, font scale,
  sidebar density and shortcuts are edited by hand in `config.json`.
- **Right-to-left text is shaped in terminal panes only.** Sidebar rows, tab
  labels and panel text still draw one character at a time, so an RTL workspace
  name or OSC title is not reordered there.
- **The palette lists actions only.** It has no workspace or session commands
  beyond the 14 bound actions, and it does not remember recent choices.

### Layout, workspaces and tabs

- **One window, one visible workspace.** No multiple windows, detachable
  surfaces or overview.
- **Sidebar has no colours, icons or filter.** Groups can only be collapsed, not
  created, renamed or coloured from the window.
- **Tabs cannot be renamed or reordered.** No overflow menu; a very large number
  of tabs squeezes every label.
- **No pane maximize/zoom, swap or move.** Splits and closes are created by
  chords or the protocol; there is no context menu.
- **No divider cursor change.** The grab band is 7 px and the pointer stays an
  arrow.
- **No menus or status bar.**

### Agents, attention and notifications

- **Detection is process-name only.** A provider is recognised from the
  foreground process name and argv; an agent whose CLI name changed, or one
  running through a wrapper script, is not detected.
- **Notifications are only in the attention list.** `⌘I` lists unacknowledged
  notifications (Enter jumps and acknowledges); there is no filter, grouping or
  bulk acknowledge, and the list is not reachable from the sidebar.
- **No OS banners, sounds or dock/taskbar badges.** `notify_os` still prints to
  stderr.
- **No mute, token or cost display.**

### Session and lifecycle

- **Autosave is debounced.** A change is written 500 ms after the last event, and
  the same snapshot is appended to `journal.ndjson` (five kept). A crash inside
  the debounce loses the last interaction; a corrupt newest entry falls back to
  the older snapshots, not a replay of structural events.
- **No single-instance handoff.** A second launch detects the live endpoint and
  exits instead of rebinding it, but it cannot bring the first window forward.

### Platform and distribution

- **Linux/X11 is the only scripted path.** Wayland, Windows and macOS are compile
  and test coverage; punch-through to a real desktop is manual.
- **No installers.** Releases publish the portable window as single-binary
  archives (plus the macOS `.app`); there is no `.deb`/AppImage/Flatpak, Windows
  installer, signing or auto-update.
- **HiDPI and multi-monitor** are exercised through winit's scale factor and the
  font rebuild on scale change; mixed-DPI hotplug is untested.

## 2. `berth-headless` (portable shell)

- Spike binary, not a packaged service; no GUI, tray or window.
- Fixed 10,000-line scrollback; no scrollback restore.
- **Exited children are polled, not pushed.** `PtyHost::is_alive` reports it and
  `berth-gui` dispatches `surface.exited`, but `SurfaceOps` still has no
  `surface_exited` callback, so a different shell must poll the same way.
- Bell, OSC title/cwd, progress, clipboard and notification requests are exposed
  through `screen_meta`; a shell must poll it.
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
- **Socket ownership is probed, not locked.** `Server::start` connects first and
  refuses with `AddrInUse` when a live instance answers, and only the handle that
  bound the endpoint unlinks it. Two starts that race inside the probe window can
  still clash; there is no lockfile.
- **CLI surface is incomplete.** No `tab`/`surface.create`, `session.save`,
  `workspace.select`, pane focus/close or workspace rename command; those exist
  only in the protocol. `berth schema` is documentation, not a typed client.
- **No client library** beyond the Rust crates and the generated Swift bindings.
- **No protocol version negotiation beyond `protocol: 1`** in hello; mismatches
  are not rejected with a dedicated error path.
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
- Feature parity with cmux.

Deferred until after v1: webview panes, cloud, plugins, telemetry, auto-update,
notarized distribution, and the final decision between this renderer and GTK4 +
`libghostty-vt`.

## 6. Coverage summary

| Surface | Automated | Manual / CI |
| --- | --- | --- |
| `berth-core`, `berth-transport`, `berth-server` | Unit + integration tests | Linux local; Ubuntu/macOS/Windows CI |
| `berth-cli` | Integration round-trip | Linux local; CI |
| `berth-headless` | PTY integration test + OSC/meta unit tests | Linux local; CI |
| `berth-gui` | Geometry, view-model, font, config, keymap, mouse, link, panel, a11y and journal tests | PNG frames on macOS; Xvfb smoke + artifact on Linux CI; release archives per platform |
| macOS app | Swift tests + build in CI | manual daily use |

## 7. Recommended order

1. **Settings window**, the last overlay on the existing chord.
2. **Protocol parity for input:** mouse reporting, IME preedit, link open.
3. **Sidebar completion:** scrolling, drag-reorder, groups UI, workspace rename.
4. **Lifecycle:** agent detection loop, autosave debounce, single-instance guard,
   log file.
5. **Packaging and CI:** window smoke on Linux CI, Windows/Wayland runs,
   installers.
