# Known gaps and missing behavior

Last audited: 2026-11-26, code at the 0.21.3 release. The audit covers real text,
scrollback, selection with auto-scroll, clipboard, mouse reporting, IME, links, OSC events,
prompt marks and command timing, copy mode, paste history, search across panes, output
rules, session logging, scrollback restore, file drops, quality-of-life window work
(wallpaper, window font, system theme, menus), divider and sidebar dragging, the palette,
attention and settings panels, agent detection, autosave, journal, accessibility, logs and
per-platform archives. This is the
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

- **The protocol floor is nearly complete.** Synchronized output (DECSET 2026), the Kitty
  keyboard protocol, styled text, OSC 8 hyperlinks, cursor shapes, focus in/out, OSC 52
  read and OSC 133/633 prompt marks are in. The AppKit shell inherits the floor from
  Ghostty. Prompt marks are a gutter, jump-to-prompt and a per-command timing shown in the
  status line: there are no collapsible command blocks, and a pane spends gutter columns
  only once its shell emits a mark. The reader times the gap between the `C` and `D` marks
  itself because no documented dialect puts a duration on `D`; a `D` that carries one is
  honoured as milliseconds.
- **Font options, ligatures and zoom.** `fontFamily` picks the face at `fontSize` ×
  `fontScale`; `uiFontFamily` sets the chrome face; `fontWeight`, `lineHeight`,
  `fontFeatures` and per-face `fontMetrics` tune it. Adjacent cells with the same style are
  shaped as one run, so ligatures render, and the default prefers an installed ligature
  face. Settings → **Font** cycles the installed monospace faces, and **Window font** cycles
  every installed family. `Cmd/Ctrl + =`, `-` and `0` zoom.
- **Two palettes, or the system's.** `theme` follows the OS light/dark appearance
  (`system`, the default) or forces `berth-dark`/`berth-light`, switching live on a system
  appearance change; `opacity` fades the window on macOS. The terminal's 16-colour ANSI
  table is still fixed, and the portable window cannot blur what is behind it: its software
  renderer writes opaque pixels, so `backgroundBlur` applies only to the AppKit shell.
- **Wallpaper is composited, not transparent.** `wallpaper` / `wallpaperFollowDesktop`
  draw a picture behind the terminal in the portable window by blending it over the
  theme background; the AppKit shell leaves this to Ghostty's `background-image`. The
  picture covers the whole window, so a split shows one continuous image instead of a
  copy per pane, and chrome (sidebar, tab strips, menus) stays solid. Settings →
  **Wallpaper image…** opens the platform picker, so no config editing is needed.
  Following the *desktop's* wallpaper is macOS-only (`NSWorkspace`); Linux and Windows
  have no portable API for it, so there the explicit path or the picker is the way in.
  Blend strength is the only tuning: there is no per-pane image, fit mode, position or
  repeat key yet.
- **Text effects stop short of per-cell fonts.** Strikethrough, underline colour, single,
  double, curly, dotted and dashed underlines, and blink are in. There is no per-cell
  font change. Hyperlinks open only http(s), mailto, ftp(s) and local paths; other
  schemes are refused.
- **Bells are one or the other.** `bell` selects `none`, `visual` or `audio`; the AppKit
  shell follows Ghostty's `bell-features` instead. OSC 9/777 notifications also reach the
  desktop on macOS and Linux.
- **Partial graphics protocols.** iTerm2 inline images, kitty graphics and
  sixel render. Kitty shared memory, z-index and direct placements are not
  supported, so clients fall back to base64 or text. Sixel covers the raster
  subset: 256 colour registers, RGB and HLS definitions, repeats and motion.


### Input and clipboard

- **Accessibility covers the terminal only.** The focused pane is a `Terminal`
  node with its text; the sidebar, tabs and panels are not in the tree, and there
  is no high-contrast mode.
- **Settings covers appearance, launches and shortcuts — not everything.** The `⌘,` panel
  edits terminal and window font, font size, display scale, theme, opacity, wallpaper,
  bell, shell, VT engine, sidebar density and width, exit/restore, copy-on-select, hook
  reinstall, socket mode and every shortcut. `fontWeight`, `lineHeight`, `fontFeatures`,
  `fontMetrics`, profiles, commands, output rules, session logging, mute providers and
  `quickHotkey` stay in `config.json`.
- **The quick terminal is portable-only for the OS-wide chord.** `quickHotkey` is
  registered globally by `berth-gui`; the AppKit shell opens the same window from the
  Terminal menu and the `quickTerminal` shortcut but registers no system-wide hotkey.
  There is one placement, not one dropdown per display. On a pure-Wayland session the
  OS-wide chord is silently unavailable (the backend is X11-only) and the menu or
  `quickTerminal` chord is the only way in.
- **File drops are portable-only.** Dropping a file on a pane types its quoted path, with
  the quoting chosen per shell; the AppKit shell does not accept a dropped file yet. The
  target pane comes from the last pointer position, which winit does not update during a
  native drag, so releasing over a second pane can type into the first. Tab, workspace and divider drags are internal; a file or folder
  dropped on a pane does nothing.
- **Right-to-left text is shaped in terminal panes only.** Sidebar rows, tab
  labels and panel text still draw one character at a time, so an RTL workspace
  name or OSC title is not reordered there.
- **The palette is one list of everything.** It indexes actions, profiles, commands,
  layouts and the live session, and remembers the keys picked in each window so an empty
  palette opens on the last thing. Its session rows are scoped to the window that opened
  it, so a window cannot list or switch to another window's workspace; the list is still
  rebuilt on every paint.

### Terminal engine

- **Two engines are built; four are selectable.** `terminalBackend` accepts
  `builtin` (`vt100`) and `alacritty` (both shipped). `wezterm` and `ghostty`
  are accepted by the config but fall back to `builtin`: `termwiz` is a parser
  without scrollback parity, and Ghostty needs a `libghostty-vt` build through
  its Zig toolchain.
- **Inline images are side-channel decoded.** Both engines share the reader's
  OSC 1337/sixel/kitty handling, so images are not an engine feature; the
  builtin engine anchors iTerm2 images in its parser callback and other engines
  through the side channel.

### Output and logs

- **Output rules tint but neither rewrite nor wrap.** `outputRules` colours a row whose
  text matches a regex (first rule wins, invalid rules reported and skipped), like ConEmu's
  Highlight and iTerm2's Triggers. A rule cannot substitute text or match across a wrapped
  line, and it cannot change a glyph or its size.
- **Session logging is one rotated file per pane.** `sessionLog` appends each pane's output
  to `sessions/<surface id>.log` with a single rotation at 8 MiB; scrollback itself stays in
  memory, and the AppKit shell reads neither key.

### Layout, workspaces and tabs

- **Multiple windows, pane zoom, swap and move are in.** Windows are a core model with
  per-window selection and persisted geometry; a pane can be maximised, swapped or moved
  between splits. Tab tear-off works across panes and windows in the portable shell.
- **Sidebar polish is partial.** Groups can be created, renamed, collapsed and
  picked from a fixed colour cycle; workspaces show an icon if one is stored and
  a colour bar. The portable window has a sidebar filter, a colour/icon cycle and one
  row per agent; there is no free-form colour or icon picker.
- **Tab overflow lands as a scrollable strip.** The strip scrolls with the wheel and a
  `»` button opens the hidden tabs. Rename and drag-reorder are in.
- **Menus are native only on macOS.** `berth-gui` puts real `NSMenu`s in the macOS system
  menu bar with the chords drawn beside each label; Linux and Windows keep the painted
  in-window bar because winit exposes no menu API there. The AppKit shell uses the native
  menu bar, and its right-click menu offers only Copy, Paste and Select All (no tab/pane
  actions).
- **One icon, three formats.** `packaging/berth.svg` is the master; `make icon` turns it
  into `packaging/Berth.icns` for the macOS bundles, and `packaging/berth-icon.png` and
  `packaging/berth.ico` are generated from it for Windows and Linux. The portable window
  sets the PNG as its window icon at runtime and `build.rs` embeds the `.ico` as the
  executable's icon resource, so Explorer, the taskbar and the title bar all show it. A
  cross-compiled Windows build needs `windres` (or the MSVC resource compiler) on the
  build host; CI builds on `windows-latest`, where it is present.
- **No divider cursor change in AppKit.** The portable window switches to a resize cursor
  over dividers; the AppKit split views keep the arrow.
- **Window geometry is persisted** in `Window.bounds`. A second launch does not hand off:
  it finds the busy endpoint and exits, telling you to close the other instance or pass
  `--socket`.
- **Copy mode is vi-only.** `copyMode` opens a modal selection
  with `h`/`j`/`k`/`l`, words, counts, `g`/`G`, paging, visual selection and `y`; there
  is no `/` search, no block selection and no `vi` operator-pending grammar (`d`/`c`).
  The buffer is captured when the mode opens, so output that arrives mid-session is not
  selectable.
- **Paste history is portable-only and session-only.** `pasteHistory` keeps twelve
  entries in memory, never on disk, and records the user's own copies and pastes but not
  OSC 52 writes from a program. The AppKit shell has no such action, so the shared
  `pasteHistory` key in `config.json` is ignored there and `Cmd+Shift+V` pastes through
  Ghostty instead.

### Agents, attention and notifications

- **Copilot resumes from a hook.** `berth hooks install copilot` writes
  `~/.copilot/hooks/berth.json`; its known events arrive after the first turn,
  so a session quit before any turn has no id to resume.
- **Command Code's short alias differs on Windows.** The CLI is `cmd` on
  Unix, `cmdc` on Windows and `command-code` everywhere; `cmd` deliberately does
  not match the Windows `cmd.exe` shell. `berth hooks install cmd` writes
  `~/.commandcode/settings.json`, and its `SessionStart` hook carries the session
  id immediately.
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

- **Scrollback restore is opt-in and clean-exit only.** `scrollbackRestore` saves each
  pane's screen (capped at 64 KiB) and replays it; the capture runs on a clean exit, so a
  crash restores the previous screen or none. The AppKit shell does not read the key.
- **A relaunch respawns each terminal from its saved cwd and argv.** The buffer it had
  on screen is gone unless `scrollbackRestore` captured it.
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
- Scrollback defaults to 10,000 lines per surface and is configurable; no scrollback
  restore.
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
- Key encoding covers the documented named keys and the kitty keyboard protocol;
  modifier-only key events and other xterm modifier combinations are not modelled.
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
- In-terminal search is a find bar over Ghostty's own incremental search (`Cmd+F`),
  with next/previous and a match count; it has no regex or whole-word toggles, and it
  covers the focused surface only, not the other panes of the workspace. Prompt
  navigation (`Cmd+Shift+Up/Down`) is Ghostty's `jump_to_prompt`, so it has no gutter.
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
