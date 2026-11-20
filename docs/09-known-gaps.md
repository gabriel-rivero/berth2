# Known gaps and missing behavior

Last audited: 2026-11-20, code at the 0.14.1 release; 0.15 items are marked as they land.
The audit covers real text, scrollback,
selection with auto-scroll, clipboard, mouse reporting, IME, links, OSC events,
divider and sidebar dragging, the palette, attention and settings panels, agent
detection, autosave, journal, accessibility, logs and per-platform archives. This is the
honest inventory of what is **not** in the tree. It separates missing UI, missing
platform work, and deliberate non-goals so the three do not get confused. The
competitive view, and which milestone owns each item, is
[docs/11-competitive-gaps.md](11-competitive-gaps.md) and
[docs/04-roadmap.md](04-roadmap.md).

The portable window is no longer a spike: it renders real text and is usable as a
daily terminal. It is still not feature parity with the macOS app, and the macOS
app itself has the gaps listed in
[docs/05-build.md](05-build.md#known-gaps) and [docs/04-roadmap.md](04-roadmap.md).

## 1. `berth-gui` (portable window)

### Terminal rendering

- **The protocol floor is incomplete.** Synchronized output (DECSET 2026) is in, but
  otherwise the window sends and understands xterm only. No Kitty keyboard protocol, no
  focus in/out (CSI ?1004), no cursor shapes (DECSCUSR), no OSC 8 hyperlink spans (link
  clicking is a text heuristic, `links.rs`), no OSC 52 read, and no OSC 133 shell
  integration. Modern TUIs and agents degrade or tear without these. The AppKit shell
  inherits all of them from Ghostty.
- **One font family, no ligatures.** `fontFamily` picks the face (first
  installed name wins, else system monospace) at `fontSize` × `fontScale`. There
  is no weight, line-height or zoom control, and no per-font metric tuning.
  Ligatures cannot render: each cell is shaped and cached on its own.
- **One palette.** The window ships a hard-coded dark theme and a fixed ANSI table; there
  is no theme selection or light mode. The AppKit shell reads the user's Ghostty config
  instead.
- **No text effects beyond bold/italic/dim/underline.** No strikethrough, blink,
  underline colour, styled underlines or per-cell font changes.
- **No audio bell.** Bells flash a pane ring; OSC 9/777 notifications also reach the
  desktop on macOS and Linux.
- **Partial graphics protocols.** iTerm2 inline images, kitty graphics and
  sixel render. Kitty shared memory, z-index and direct placements are not
  supported, so clients fall back to base64 or text. Sixel covers the raster
  subset: 256 colour registers, RGB and HLS definitions, repeats and motion. No
  hyperlink
  (`OSC 8`) hover styling beyond the text scan.


### Input and clipboard

- **Accessibility covers the terminal only.** The focused pane is a `Terminal`
  node with its text; the sidebar, tabs and panels are not in the tree, and there
  is no high-contrast mode.
- **Settings stops at appearance and shortcuts.** The `⌘,` panel edits font size,
  display scale, sidebar density, exit/restore, socket mode and every shortcut; font
  family, theme, profiles and per-provider mute stay in `config.json`.
- **No file drops.** Tab, workspace and divider drags are internal; a file or folder
  dropped on a pane does nothing.
- **Right-to-left text is shaped in terminal panes only.** Sidebar rows, tab
  labels and panel text still draw one character at a time, so an RTL workspace
  name or OSC title is not reordered there.
- **The palette lists actions only.** It has no workspace or session commands
  beyond the 14 bound actions, and it does not remember recent choices.

### Output and logs

- **No output rules or session log.** Nothing styles matching lines (ConEmu's Highlight,
  iTerm2's Triggers), and a pane's output cannot be written to a file; scrollback lives in
  memory only.

### Layout, workspaces and tabs

- **One window, one visible workspace.** No multiple windows, detachable
  surfaces or overview.
- **Sidebar polish is partial.** Groups can be created, renamed, collapsed and
  picked from a fixed colour cycle; workspaces show an icon if one is stored and
  a colour bar. There is no colour picker or icon picker, no sidebar filter, and
  a workspace still shows one agent line plus an agent count rather than a row
  per agent.
- **No tab overflow menu.** Tabs resize down to a minimum label; a very large
  number of tabs squeezes every label. Rename and drag-reorder are in.
- **No pane maximize/zoom, swap or move.** Splits and closes are created by
  chords or the protocol; there is no context menu.
- **No divider cursor change.** The grab band is 7 px and the pointer stays an
  arrow.
- **No menus or status bar.**
- **Window geometry is not persisted.** A dragged sidebar width and the window
  size are lost on relaunch; the window opens at 1180×720 every time.
- **No copy mode or paste history.** Selection is mouse-only and the clipboard
  keeps one entry.

### Agents, attention and notifications

- **Copilot resumes from a hook.** `berth hooks install copilot` writes
  `~/.copilot/hooks/berth.json`; its known events arrive after the first turn,
  so a session quit before any turn has no id to resume.
- **Detection is process-name only.** A provider is recognised from the
  foreground process name and argv; an agent whose CLI name changed, or one
  running through a wrapper script, is not detected.
- **Notifications are only in the attention list.** `⌘I` lists unacknowledged
  notifications (Enter jumps and acknowledges); there is no filter, grouping or
  bulk acknowledge, and the list is not reachable from the sidebar.
- **No dock/taskbar badge, sound or menu-bar item.** Desktop banners exist on macOS and
  Linux; the Windows path is a stub that returns without notifying, so the attention list
  is the only surface there. `notify_os` in the headless shell prints to stderr.
- **Mute is config-only.** `muteProviders` silences desktop notifications at
  launch; there is no Settings row. Usage needs a client to report it: `agent.usage`
  exists and the sidebar shows it, but the bundled hooks do not report tokens or cost
  yet.

### Session and lifecycle

- **No scrollback restore.** A relaunch respawns each terminal from its saved cwd and
  argv; the buffer it had on screen is gone.
- **Autosave is debounced.** A change is written 500 ms after the last event, and
  the same snapshot is appended to `journal.ndjson` (five kept). A crash inside
  the debounce loses the last interaction; a corrupt newest entry falls back to
  the older snapshots, not a replay of structural events.
- **No single-instance handoff.** A second launch detects the live endpoint and
  exits instead of rebinding it, but it cannot bring the first window forward.

### Platform and distribution

- **Wayland and X11 are both smoke-tested** (`gui-smoke.sh` under Xvfb and a
  headless Weston session); Windows and macOS remain compile and test coverage
  with manual punch-through. Mixed-DPI hotplug is still manual; the automatic
  scale rule and the font rebuild on `ScaleFactorChanged` are covered.
- **Only two packages.** Releases attach a Debian `.deb` and ship an Arch
  `PKGBUILD` in the tree, both installing the window and the CLI with a desktop
  entry; there is no AppImage, Flatpak, rpm, Windows installer, signing or
  auto-update. The AUR submission is manual.
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
- Debian/Arch packages exist; there is no systemd unit, launch agent or
  autostart. The log keeps one rotated file.

## 3. Portable core, transport, server and CLI

- **Local endpoints only.** Unix domain socket or Windows named pipe; no TCP,
  TLS, SSH tunnel helper or remote transport.
- **Token auth is the whole authorization model.** `local` needs a token for
  mutations; `trusted` bypasses it; `off` denies. Unix endpoints reject a
  different peer uid before the token check; Windows named pipes have no
  peer-SID check. There is no per-client identity, audit trail, rate limit or
  documented client cap.
- **Tokens live in a 0600 file**, not in child environments. `trusted` mode
  intentionally has no token check.
- **Socket ownership is probed, not locked.** `Server::start` connects first and
  refuses with `AddrInUse` when a live instance answers, and only the handle that
  bound the endpoint unlinks it. Two starts that race inside the probe window can
  still clash; there is no lockfile.
- **CLI is complete for the documented methods.** Tabs, pane focus/close,
  surface rename/close, session save, workspace rename/select, tab move and the
  group commands exist; `berth schema` is still documentation, not a typed
  client.
- **No client library** beyond the Rust crates and the generated Swift bindings.
- **Protocol negotiation is client-side only.** The client rejects a stated
  `hello.protocol` mismatch; the server does not negotiate or downgrade.
- **Windows named-pipe path is per-user but still one global name**; multiple
  profiles/instances share the collision risk.
- **No Windows service, Linux systemd unit, macOS launch agent or autostart.**

## 4. macOS app (existing, not regressions)

The Swift/AppKit app still uses its own `SocketServer.swift`/
`CoreHandler.swift`; it has not been migrated onto `berth-server` through
UniFFI. Known gaps already tracked in the macOS docs:

- Group management UI (create/rename/color) is absent even though
  `group.create/update/delete` exist.
- No in-terminal search: `search` is not in its `ShortcutAction` enum, so the
  binding is ignored.
- No tab rename UI, tab drag-reorder, or tab overflow handling.
- Geometric focus: `⌘]`/`⌘[` walk tree order; arrow-key spatial navigation is
  missing.
- Agent mute and multi-agent workspace rows are incomplete; banners have no
  per-provider mute.
- Journal stores whole snapshots, not replayable structural events; a crash can
  lose the last ~500 ms.
- No headless app-test loop for the real GUI; `scripts/m2-acceptance.sh` needs
  the app window.
- No terminal accessibility model; VoiceOver sees the sidebar and panels only.
- Socket hardening: peer-uid and the token file are in the core and portable
  path, not in this shell's own `SocketServer.swift`, so the Swift app still
  exposes `BERTH_TOKEN` to children.
- The 0.14 Instruments trace covers `berth-gui` only. The Swift shell has no profiling
  run, and the portable numbers need a rerun once 0.15 touches the renderer.
- Scrollback is intentionally not restored.
- Most `config.json` keys are read only by the portable window
  (`fontFamily`, `closeExitedPanes`, `restoreLaunchFlags`, `copyOnSelect`,
  `scrollbackLines`, `graphics`, `shell`, `resumeArgs`, `muteProviders`,
  `sidebarWidth`); the Swift shell reads `shortcuts`, `fontSize`, `fontScale`,
  `compactSidebar` and `socket` only.

## 5. Explicit non-goals (intentional, not gaps)

From [docs/01-vision.md](01-vision.md#4-non-goals-for-v1):

- Browser panes, cookie import, agent-driven web automation.
- SSH workspaces, remote tmux mirroring, cloud VMs.
- iOS companion, push notifications, teams/multiplayer.
- Plugin marketplace, TUI mode, custom command DSLs.
- Feature parity with cmux.

Deferred until after v1: webview panes, cloud, plugins, telemetry and
auto-update. The renderer decision is settled: winit + `vt100` +
cosmic-text, with a side-channel scanner for graphics, so there is no planned
move to GTK4 + `libghostty-vt`.

## 6. Coverage summary

| Surface | Automated | Manual / CI |
| --- | --- | --- |
| `berth-core`, `berth-transport`, `berth-server` | Unit + integration tests | Linux local; Ubuntu/macOS/Windows CI |
| `berth-cli` | Integration round-trip | Linux local; CI |
| `berth-headless` | PTY integration test + OSC/meta unit tests | Linux local; CI |
| `berth-gui` | Geometry, view-model, font, config, keymap, mouse, link, panel, a11y and journal tests | PNG frames on macOS; Xvfb smoke + artifact on Linux CI; release archives per platform |
| macOS app | Swift tests + build in CI | manual daily use |

## 7. Recommended order

The roadmap supersedes this list; it is kept as the ordering the gap analysis
arrived at:

1. **Terminal floor in the portable window** (0.15): Kitty keyboard, synchronized
   output, styled text and OSC 8, focus reporting, cursor shapes, ligatures, reflow.
2. **Window model** (0.16): multiple windows, pane zoom, tab tear-off, broadcast input,
   geometric focus, persisted geometry.
3. **Shell integration and recall** (0.17): OSC 133 prompt marks, AppKit search, copy
   mode, paste history, quick terminal.
4. **Durability and agent depth** (0.18): detached PTY host, agent inbox, cost meter,
   more providers, headless test loop.
5. **Distribution** (1.0): notarization, Sparkle, Homebrew cask, AppImage or Flatpak,
   Windows package.
6. **Kitty protocol depth:** shared memory, z-index and direct placements, only
   if a tool in use needs them.
