# Configuration reference

Berth reads one JSON file at `~/.config/berth/config.json` (on Linux,
`$XDG_CONFIG_HOME/berth/config.json`). Missing keys use the defaults below.
Changes to shortcuts, font, and sidebar density apply without reinstalling; the
socket mode is read at launch.

```json
{
  "shortcuts": {
    "splitRight": "cmd+d",
    "splitDown": "cmd+shift+d",
    "newTab": "cmd+t",
    "closeSurface": "cmd+w",
    "newWorkspace": "cmd+n",
    "palette": "cmd+shift+p",
    "jumpAttention": "cmd+shift+u",
    "showAttention": "cmd+i",
    "toggleSidebar": "cmd+opt+s",
    "settings": "cmd+,"
  },
  "fontSize": 14,
  "fontScale": 0,
  "fontFamily": ["MesloLGS NF", "JetBrains Mono"],
  "compactSidebar": false,
  "closeExitedPanes": true,
  "restoreLaunchFlags": true,
  "socket": "local"
}
```

## Fonts and displays

- `fontSize` is the terminal point size, clamped to `8…72`.
- `fontScale` is a per-display multiplier. `0` means automatic:
  `2 / backingScaleFactor`, so a 1× external monitor gets the same physical
  glyph size as a 2× Retina panel. Set `1.0` to disable the adjustment, or a
  value in `0.5…3` to override it manually.
- Settings → **Display Font Scale** writes this value and applies it to live
  surfaces immediately (`ghostty_surface_update_config`), so no relaunch is
  needed. Each surface re-evaluates when it moves between screens.

The portable window reads `fontSize` and `fontScale` with the same rule,
rasterises the configured face with cosmic-text, and redraws with the new metrics
when the window moves to a display with a different scale factor.
- `fontFamily` takes one name or a list, the shape Ghostty accepts. The first
  installed name draws the text; the rest fill in what it lacks, in order, so a
  Nerd Font later in the list supplies prompt icons and powerline separators.
  An unknown name is skipped, and an empty list or no installed match keeps the
  system monospace. Terminal and chrome (tabs, sidebar) use the same list. It
  applies at startup and when the window rebuilds its fonts, so restart after
  editing it.

It does not read the Ghostty config, so `font-family` set only in
`~/.config/ghostty/config` does not apply there, and neither do opacity or
padding. Ligatures do not render: the portable window shapes and caches one cell
at a time, so a face with ligatures draws them as separate glyphs.

## Portable window shortcuts

`berth-gui` reads the same `shortcuts` object. Chord strings use the macOS
spelling; outside macOS `cmd` means the Super/Windows key, which desktops usually
capture, so the window ships a Ctrl-based default table instead:

| Action | macOS | Linux / Windows |
| --- | --- | --- |
| `splitRight` | `cmd+d` | `ctrl+shift+d` |
| `splitDown` | `cmd+shift+d` | `ctrl+shift+e` |
| `newTab` | `cmd+t` | `ctrl+shift+t` |
| `closeSurface` | `cmd+w` | `ctrl+shift+w` |
| `nextTab` | `cmd+shift+]` | `ctrl+shift+]` |
| `previousTab` | `cmd+shift+[` | `ctrl+shift+[` |
| `nextPane` | `cmd+]` | `ctrl+alt+]` |
| `previousPane` | `cmd+[` | `ctrl+alt+[` |
| `newWorkspace` | `cmd+n` | `ctrl+shift+n` |
| `palette` | `cmd+shift+p` | `ctrl+shift+p` |
| `jumpAttention` | `cmd+shift+u` | `ctrl+shift+u` |
| `showAttention` | `cmd+i` | `ctrl+shift+i` |
| `toggleSidebar` | `cmd+opt+s` | `ctrl+alt+s` |
| `settings` | `cmd+,` | `ctrl+,` |

A value overrides the default for that action; an empty string unbinds it. The
settings panel (`⌘,` / `Ctrl+,`) edits this table, the font size, the display
scale, the sidebar density, close-on-exit and the socket mode, and writes the
same file. Fixed
bindings that do not appear in the table: `Super+1…9` selects a workspace, and
`Super+C`/`Ctrl+Shift+C` copy and `Super+V`/`Ctrl+Shift+V` paste.

The portable window also stores `sidebarWidth` when the sidebar edge is dragged;
the macOS shell ignores that key.


## Sidebar

- `compactSidebar: true` reduces row height from 62 to 52 points and hides the
  metadata line (workspace name, cwd, git branch, listening ports).

## Panes on exit

- `closeExitedPanes` defaults to `true`. When a pane's process exits, its tab
  closes; an emptied pane is removed, and closing the last pane closes the
  workspace. If that was the last workspace, a fresh one opens so the window
  stays usable.
- Set it to `false` to keep the final screen with the red "process exited"
  banner and close the tab yourself. The key is read by the portable window;
  the macOS app keeps its own surfaces until you close them.

## Agent resumes

- `restoreLaunchFlags` defaults to `true`. When Berth restores an agent, extra
  flags from the saved launch command are kept, so
  `claude --allow-dangerously-skip-permissions` comes back as
  `claude --resume <id> --allow-dangerously-skip-permissions`. A passively
  detected agent's command is captured when its hook reports the session id.
- Set it to `false` to resume with the provider command and session id only
  (`claude --resume <id>`), dropping flags such as `--model` as well.

## Automation socket

`off`, `local`, or `trusted`.

- `local` (default) creates a 0600 token under the platform data directory
  (`~/Library/Application Support/Berth/token` on macOS, `%APPDATA%\Berth\token`
  on Windows, `$XDG_DATA_HOME/berth/token` on Linux) and requires it for mutating
  methods. Read-only methods (`*.list`, `*.get`, `system.*`,
  `events.subscribe`) do not need it.
- `trusted` is intended for a same-user automation setup and bypasses the token.
- `off` disables the socket entirely.

Environment overrides for clients and children:

| variable | meaning |
| --- | --- |
| `BERTH_SOCKET_PATH` | socket path override (Unix path or `\\.\pipe\name`) |
| `BERTH_DATA_DIR` | data directory override (token, session snapshot) |
| `BERTH_TOKEN_FILE` | token file override |
| `BERTH_CONFIG` | `config.json` override (portable window and tests) |
| `BERTH_TOKEN` | token injected into every child; used by `berth` |
| `BERTH_SURFACE_ID` / `BERTH_WORKSPACE_ID` | injected into spawned terminals |
| `BERTH_AGENT_ID` | injected by `berth run` and agent restore |

## Agent hooks

`berth hooks install` writes Claude Code hooks into `~/.claude/settings.json`
and an opencode plugin into `~/.config/opencode/plugins/berth.ts`. Both are
idempotent and preserve unrelated hooks. `berth hooks print` shows the recipes
without writing. See `docs/05-build.md` for the acceptance checkpoints.

## Logs

Unified logging uses subsystem `dev.berth.app` with categories `app`, `core`,
`socket`, `agent`, `restore`, `ui`, and `layout`:

```sh
log stream --predicate 'subsystem == "dev.berth.app"' --level debug
```

Two files under `~/Library/Logs/Berth/` are the fastest way to diagnose a hang
or a crash:

- `berth.log` — rolling breadcrumbs: reconciles and their duration, layout
  rebuilds, ignored divider resizes, core dispatches, detection passes, and one
  `surface:` line per geometry change (bounds, backing scale, layer scale,
  surface pixels, grid, cell size, font). Capped at 2 MiB, rotated once to
  `berth.log.1`.
- `crash.log` — uncaught exceptions and main-thread stalls, each with the 40
  breadcrumbs that preceded it. A stall line means the main thread stopped
  servicing the runloop; the breadcrumbs name what was in flight.

A background watchdog pings the main queue once a second and records a stall
when a ping stays unserviced for two seconds or more.

