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
    "settings": "cmd+,",
    "zoomIn": "cmd+=",
    "zoomOut": "cmd+-",
    "zoomReset": "cmd+0"
  },
  "fontSize": 14,
  "fontScale": 0,
  "fontFamily": ["MesloLGS NF", "JetBrains Mono"],
  "fontWeight": 400,
  "lineHeight": 1.35,
  "fontFeatures": ["calt", "liga"],
  "fontMetrics": {"MesloLGS NF": {"width": 9, "height": 20}},
  "theme": "berth-dark",
  "opacity": 1.0,
  "backgroundBlur": 0,
  "bell": "visual",
  "compactSidebar": false,
  "closeExitedPanes": true,
  "restoreLaunchFlags": true,
  "claudeBypassPermissions": true,
  "copyOnSelect": true,
  "scrollbackLines": 4000,
  "graphics": true,
  "resumeArgs": {"codex": ["--sandbox", "read-only"]},
  "muteProviders": [],
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
- `fontWeight` is the normal-text weight, a number `100…900` or a name such as
  `"medium"` or `"semi-bold"`. Bold cells use at least 700.
- `lineHeight` is the line height as a multiple of the font size, clamped to
  `0.8…3`. The default is `1.35`.
- `fontFeatures` is a list of OpenType tags to enable, such as `["calt", "liga"]`,
  or `"-liga"` to disable one. Four-character tags only; invalid entries are
  skipped.
- `fontMetrics` overrides the measured cell for one family, as
  `{"Fira Code": {"width": 9, "height": 20}}`. Only the family that shapes
  normal text changes the grid; the other keys are ignored. Values are clamped
  to `1…256`.

It does not read the Ghostty config, so `font-family` set only in
`~/.config/ghostty/config` does not apply there, and neither do opacity or
padding. Ligatures render through shaping runs: adjacent cells with the same
style are shaped as one string, so a face with `calt`/`liga` draws them.

## Appearance

- `theme` selects the palette: `berth-dark` (the default) or `berth-light`. An
  unknown name keeps the dark palette. The settings panel cycles the list and
  writes the choice back.
- `opacity` is the window opacity, `0.3…1.0`. One is opaque. The portable window
  applies it on macOS; Linux and Windows keep the window opaque because winit does
  not expose it there. The AppKit shell applies `opacity` and `backgroundBlur` only
  when the key is present, so an absent key keeps whatever the user's Ghostty config
  already says.
- `bell` is what a BEL does: `none`, `visual` (flash the pane border, the default) or
  `audio`, which plays the system bell sound (Glass on macOS, the freedesktop bell on
  Linux and the console beep on Windows). `none` also stops the bell counting as
  attention. OSC 9/777 notifications are separate and still reach the desktop.
- `backgroundBlur` is a blur radius, `0…64`, passed to Ghostty by the AppKit shell
  when it is set.
  The portable renderer writes opaque pixels, so it cannot show what is behind the
  window and ignores the key.
- `zoomIn`, `zoomOut` and `zoomReset` are ordinary shortcuts: they step the font
  size by one point, and reset returns to the configured `fontSize`. Zoom lasts for
  the session and writes nothing, so a relaunch starts at `fontSize`. The defaults
  are `cmd+=`, `cmd+-` and `cmd+0` on macOS and `ctrl+=`, `ctrl+-`, `ctrl+0`
  elsewhere.
- The AppKit shell has no settings rows for `opacity` and `backgroundBlur`; set
  them in `config.json`.

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
| `search` | `cmd+f` | `ctrl+shift+f` |

A value overrides the default for that action; an empty string unbinds it. The
settings panel (`⌘,` / `Ctrl+,`) edits this table, the font size, the display
scale, the shell, the sidebar density, close-on-exit and the socket mode, and
writes the same file. A shifted symbol is spelled with its unshifted key
(`ctrl+shift+]`), and Shift+letter chords are case-insensitive. Fixed
bindings that do not appear in the table: `Super+1…9` selects a workspace, and
`Super+C`/`Ctrl+Shift+C` copy and `Super+V`/`Ctrl+Shift+V` paste.

The portable window also stores `sidebarWidth` when the sidebar edge is dragged;
the macOS shell ignores that key.

## Shell

`shell` is the program a new terminal runs. Omit it for the platform default:
`$SHELL` on Linux and macOS, `%ComSpec%` (cmd.exe) on Windows.

```json
{ "shell": "C:\\Program Files\\Git\\bin\\bash.exe" }
```

A string is the program alone, so a Windows path with spaces needs no quoting.
Extra arguments need the list form:

```json
{ "shell": ["C:\\Program Files\\Git\\bin\\bash.exe", "--login", "-i"] }
```

The portable window's settings row cycles through the shells it can find,
`default` first:

| found by probing | Windows | Linux / macOS |
| --- | --- | --- |
| `pwsh` | `%ProgramFiles%\PowerShell\7\pwsh.exe` | — |
| `windows powershell` | `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe` | — |
| `git bash` | `%ProgramFiles%\Git\bin\bash.exe`, `%LOCALAPPDATA%\Programs\Git\bin\bash.exe` | — |
| `msys2 zsh`, `msys2 bash` | `C:\msys64\usr\bin\…` | — |
| `cygwin zsh`, `cygwin bash` | `C:\cygwin64\bin\…` | — |
| `wsl` | `%SystemRoot%\System32\wsl.exe` | — |
| `zsh`, `fish`, `nu`, `bash` | — | `/etc/shells`, then `PATH` |

Only programs that exist are offered. Bash entries run as `--login -i` and zsh
as `-l`, so the profile loads. Agent panes are unaffected: their argv comes from
the launch or resume command. A change applies to panes created afterwards,
including restored workspaces whose shell was not recorded; panes already
running keep their program. The macOS app does not read this key yet.


## Sidebar

- `compactSidebar: true` reduces row height from 62 to 52 points and hides the
  metadata line (workspace name, cwd, git branch, listening ports).

## Selection and search

- `copyOnSelect` defaults to `true`: releasing the mouse after selecting more
  than one cell writes the selection to the clipboard. A single-cell click
  never replaces it. `Super+C`/`Ctrl+Shift+C` still copies the selection of the
  focused pane, or of the last pane that had one.
- `search` opens the scrollback search for the focused pane. Enter moves to the
  next match, Shift+Enter to the previous one, Escape closes and clears the
  highlight.

## Panes on exit

- `closeExitedPanes` defaults to `true`. When a pane's process exits, its tab
  closes; an emptied pane is removed, and closing the last pane closes the
  workspace. If that was the last workspace, a fresh one opens so the window
  stays usable.
- Set it to `false` to keep the final screen with the red "process exited"
  banner and close the tab yourself. The key is read by the portable window;
  the macOS app keeps its own surfaces until you close them.

## Scrollback

- `scrollbackLines` is how many rows of history each surface keeps, with a
  default of `10000` and a ceiling of `100000`. It is read at launch and applies
  to every surface spawned after it, including restored ones.
- The scrollback lives in memory in the terminal parser, per surface. It is not
  written to disk and is not part of the session snapshot, so a relaunch starts
  with an empty history. Budget roughly the row count times the columns times
  the cell size: 10,000 rows at 200 columns is on the order of tens of MB per
  busy surface, so lower it if you run many surfaces on a small machine.
- `0` disables history; the view still shows the live screen.
- `graphics` (default `true`) decodes terminal image protocols. It is read at
  launch and applies to new surfaces. Images are bounded to 4096 px per axis,
  eight per surface and 32 MB per surface; set it to `false` on memory-tight
  machines.

## Agent resumes

- `restoreLaunchFlags` defaults to `true`. When Berth restores an agent, extra
  flags from the saved launch command are kept, so
  `claude --allow-dangerously-skip-permissions` comes back as
  `claude --resume <id> --allow-dangerously-skip-permissions`. A passively
  detected agent's command is captured when its hook reports the session id.
- Set it to `false` to resume with the provider command and session id only
  (`claude --resume <id>`), dropping flags such as `--model` as well.
- `resumeArgs` adds arguments per provider id on every resume, on top of the
  saved launch flags, for example `{"codex": ["--sandbox", "read-only"]}`.
  Exact duplicates are kept once. This is the general form of
  `claudeBypassPermissions`; both can be set.

- `claudeBypassPermissions` defaults to `true`: a restored Claude session runs
  with `--permission-mode bypassPermissions`, so its permission checks are
  skipped. The mode is added at restore time and is not written back into the
  saved launch command, so turning it off later restores the previous behavior.
  Set it to `false`, or use the Settings row, to keep the session's own mode.

## Agent notifications

- `muteProviders` lists provider ids whose OS notifications are silenced, for
  example `["codex"]`. The sidebar row, attention ring and in-app list still
  update; only the desktop banner is skipped. Read at launch.
- Tokens and cost appear on the agent row when a client reports them with
  `berth agent-usage AGENT_ID --tokens-in N --tokens-out N --cost USD` (or the
  `agent.usage` protocol method). Nothing renders while they are zero.

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

