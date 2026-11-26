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
  "wallpaperFollowDesktop": false,
  "wallpaper": "/Users/me/Pictures/peaks.jpg",
  "wallpaperOpacity": 0.3,
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
  An unknown name is skipped. An empty list, or no installed match, uses the
  first installed face from a built-in ligature list (JetBrains Mono, Fira Code,
  MesloLGS NF, Cascadia Code, Hack, Victor Mono, Iosevka and a few others)
  before the system monospace. Settings → **Font** cycles the default and every
  installed monospace face, applies the choice without a relaunch, and writes it
  here; the default keeps any fallbacks already in the list. The row names the
  resolved default, for example `Default (Monaco)` when no ligature face is
  installed.
- `uiFontFamily` takes the same shape and sets the **window** font: tabs, the
  sidebar, the menu bar and the status strip. Terminal cells keep `fontFamily`.
  An empty list uses the system sans-serif and skips the monospace ligature
  list, so the default is the platform interface face. Settings → **Window
  font** cycles the default and every installed family (not just monospaced
  ones) and applies the choice without a relaunch. Changing it never resets the
  terminal zoom.
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

- `theme` selects the palette. `system` (the default when the key is absent)
  follows the OS light/dark appearance and switches live when the system setting
  changes; `berth-dark` and `berth-light` force a palette regardless of the OS.
  An unknown name keeps the dark palette. The settings panel cycles `system`,
  `berth-dark` and `berth-light` and writes the choice back.
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
- `wallpaper` is an image path drawn behind the terminal. `wallpaperFollowDesktop`
  (`true`/`false`) uses the desktop's own wallpaper instead — on macOS that is read
  from `NSWorkspace`, so it follows whatever the system is set to, including a
  rotation. PNG and JPEG both decode. Only the portable window draws it; the AppKit
  shell ignores these keys. Settings → **Wallpaper** cycles off / desktop / the
  configured path, and **Wallpaper image…** opens the system file picker
  (`NSOpenPanel`, the XDG desktop portal, or `IFileDialog`) and adopts the choice.
  The picker offers only what the decoder can read; picking a file always turns the
  wallpaper on, so the result is visible immediately.
- `wallpaperOpacity` is how strongly the picture shows through, `0.0…1.0` (a
  fraction, like Ghostty's `background-image-opacity`); `0` is off. The default when
  a source is named is `0.3`. The picture is composited *over* the theme background
  rather than replacing it, so the theme colour stays the dominant term and text
  keeps its contrast; a cell that sets its own background still paints it. Settings
  → **Wallpaper strength** steps it 5% at a time.
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
| `previousPrompt` | `cmd+shift+up` | `ctrl+shift+up` |
| `nextPrompt` | `cmd+shift+down` | `ctrl+shift+down` |
| `copyMode` | `cmd+shift+c` | `ctrl+shift+space` |
| `pasteHistory` | `cmd+shift+v` | `ctrl+alt+v` |
| `quickTerminal` | `cmd+shift+backquote` | `ctrl+shift+backquote` |

Copy mode is vi-style keyboard selection over the pane's scrollback. The chord opens
it on the focused pane with the cursor on the newest line; press it again (or `q`,
`Esc`, or `Space`) to leave. Motions are `h`/`j`/`k`/`l` and the arrow keys, `w`/`b`/`e`
for words, `0`/`$`/`Home`/`End` for the line, `g`/`G` for the oldest and newest line,
and `Ctrl+D`/`Ctrl+U` (or `PageDown`/`PageUp`) for pages; a count prefixes any of them,
as in `5j`. `v` starts a visual selection, `y` or `Enter` copies it, and a click moves
the cursor. Because the buffer is captured when the mode opens, a yank can span more
than one screen, which a mouse selection cannot.

`pasteHistory` opens a picker over the last twelve copies and pastes, newest first.
The history lives in memory for the session only and is never written to disk, and it
records the user's own copies and pastes rather than OSC 52 writes from a program. On
macOS the chord sits on the fixed paste binding, so with an empty history it falls back
to an ordinary paste instead of doing nothing; a multi-line entry still asks for the
paste confirmation.

`previousPrompt` and `nextPrompt` walk the prompts a shell marks with OSC 133/633, and
clicking a gutter tick brings that command's output to the top of the pane. The window
times the command itself, between the `C` and `D` marks the shell emits, and shows the
result in the status line for a few seconds after the jump — `1.5s · ok`, or
`4m12s · failed`. A pane that emitted no `C` has nothing to time, so its prompt shows
only the outcome. The gutter takes columns only once a pane's shell emits a mark, so a
shell without integration keeps the full grid.

Search also takes two toggles while the panel is open: `Cmd/Ctrl+R` switches to a
regular expression, and `Cmd/Ctrl+W` restricts the match to whole words. A query
is literal unless the regex toggle is on, so a search for `a[` finds those two
characters rather than failing to compile. Search covers every pane of the
visible workspace, and the detail column names the pane each hit came from.

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

## Output rules

`outputRules` tints the rows whose text matches a regex, the way ConEmu's
Highlight and iTerm2's Triggers do. Rules match a whole visible row at paint
time, so they apply to scrollback as well as to new output, and the first rule
in the file wins.

```json
{
  "outputRules": [
    { "pattern": "\berror\b", "fg": "#ff5555", "bold": true },
    { "name": "warn", "pattern": "warning", "bg": "#3a2a00" }
  ]
}
```

`fg` and `bg` are `#rrggbb` (the `#` is optional); `bold` thickens the glyphs.
A rule that sets none of the three, or whose pattern does not compile, is
reported at startup and skipped rather than silently doing nothing. A rule
cannot change a glyph or its size, so a highlight cannot break the cell grid.

## Session log

`sessionLog` writes each pane's output to a file under the log directory
(`sessions/<surface id>.log`), appended as the pane produces it. It is off by
default: unlike the clipboard, a log outlives the session, and session output
can contain a token. One file rotates to `.log.1` at 8 MiB, and logs are closed
when their pane closes.

## Command palette

The palette (`palette`, `Cmd/Ctrl+Shift+P`) indexes more than the shortcut table: every
action, the configured profiles and commands, saved layouts, and the live session —
each workspace, the tabs of the visible workspace (labelled with their pane), and each
group, whose row says whether Enter will collapse or expand it.

With an empty query the rows are ordered by what you picked last in this window, so the
palette opens on the thing you keep reaching for. A destructive action (close, restart,
duplicate, new window, quick terminal) is never promoted that way, because the palette
preselects the first row and a repeat would then be one Enter away. Typing a query
filters on the label, including its prefix (`Workspace `, `Tab `, `Group `).

## File drops

Dropping a file or folder on a pane types its path, quoted for the shell that will
read it: POSIX single quotes for `sh`/`bash`/`zsh`/`fish` (and a WSL pane on
Windows), PowerShell's single-quoted literal form for `pwsh`, and double quotes
for `cmd`. The quoting is chosen from the pane's program rather than the host OS.
Several files dropped at once become one space-separated command line with a
trailing space, and the pane under the pointer is highlighted while a file hovers.

Two things are refused rather than typed wrong: a path containing a control
character (the line editor acts on those before the shell sees the quotes, so
`^U` would erase the opening quote), and a drop into a full-screen program (in
vim normal mode a path is a sequence of commands). When the program enabled
bracketed paste, the text is wrapped so the line editor inserts it literally.

## Scrollback restore

`scrollbackRestore` keeps each pane's last visible screen in the session file and
replays it into the pane on the next launch, so a relaunch shows the screen you
left instead of only the cwd. It is off by default: the saved text is a copy of
program output, which can contain a token.

The text is capped at 64 KiB per pane, plain (no colours or attributes), and
replayed with a carriage return per line so the screen does not staircase. A
full-screen program's screen (an editor, a pager) is not saved, because it
redraws on start and a replay would flash over it. The saved copy is consumed by
the replay, so `clear`, a restart, or a crash cannot bring an old screen back.

The portable window captures screens on a clean exit (quit, Ctrl-C, SIGTERM);
the AppKit shell does not read this key yet.

## Profiles and commands

`profiles` and `commands` are named launches. A profile is offered when a tab or
workspace starts; a command is something you run on purpose. Both take the same
fields, so one shape covers everything from a plain shell to a `wsl.exe` preset:

```json
{
  "profiles": [
    { "name": "wsl", "command": ["wsl.exe", "-e"], "args": ["bash", "-l"],
      "label": "Ubuntu", "cwd": "/home/me", "env": { "TERM": "xterm-256color" } }
  ],
  "commands": [
    { "name": "serve", "command": ["make", "serve"], "cwd": "./app" }
  ]
}
```

`command` is either a bare program path (so a Windows path with spaces needs no
quoting) or a list of program plus fixed arguments; `args` appends more, `env`
is added to the child's environment, and `cwd` is where it starts. `shell` wraps
the whole thing in another program, which is how `wsl.exe -e bash -lc …` is
spelled. `label` names the tab, defaulting to `name`. Two launches may not share
a name.

They are reachable from three places, all resolving the same core definition:
the command palette (`Launch <name>`, `<name> Workspace`, `Command <name>`),
`berth new --profile NAME` / `berth new-tab --profile NAME`, and `berth profiles`
to list them. The core remembers which one was opened last, so `berth profiles` and the
palette both put recent picks first; the order is per app run and lives in memory.

## Layouts

`layout.save NAME` captures the selected workspace's split tree with each pane's
cwd and command; `layout.open NAME` restores it as a new workspace with fresh
pane ids. The palette offers `Save Layout <workspace name>` and `Layout <name>`.
Layouts live in memory for the session, like the paste history: they are not
written to disk yet.

## Quick terminal

`quickHotkey` is a global chord that opens a dropdown terminal over whatever
application is frontmost. It is registered with the OS, so it works while Berth
is in the background; an empty string disables it, and the Terminal menu still
opens the dropdown.

```json
{ "quickHotkey": "cmd+shift+`", "quickPlacement": "top", "quickHeight": 0.45 }
```

`quickPlacement` is `top`, `bottom` or `center`, and `quickHeight` is the
fraction of the display the dropdown covers, clamped to `0.15…1`. The dropdown
has no sidebar, floats above other windows, hides when it loses focus, and is
never written to the session file: it is recreated on demand, so it never pops
up at launch. The default chord avoids the backquote chord, which `nextWindow` owns.

Portable window only: the AppKit shell reaches the dropdown from the Terminal
menu (and the `quickTerminal` shortcut) but registers no OS-wide hotkey.

## Terminal engine

`terminalBackend` picks the parser/grid a new surface uses. It is read at
launch; a change applies to surfaces created afterwards.

```json
{ "terminalBackend": "alacritty" }
```

| value | status |
| --- | --- |
| `builtin` (default) | the `vt100` grid, with Berth's capture-and-replay reflow |
| `alacritty` | `alacritty_terminal`'s grid with native reflow, scrollback and selection |
| `wezterm` | accepted, not built in yet — falls back to `builtin` |
| `ghostty` | accepted, not built in yet — needs a `libghostty-vt` build |

An unknown value, or a backend this build does not include, falls back to
`builtin`; the window logs the substitution to `berth.log`. The reader's side
channel (synchronized output, the kitty keyboard protocol, DECSCUSR, kitty
graphics, sixel and OSC 8) is shared by every engine and behaves the same. The
settings row is labelled **VT engine (relaunch)**.

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

`berth hooks install` writes Claude Code hooks into `~/.claude/settings.json`,
an opencode plugin into `~/.config/opencode/plugins/berth.ts`, Copilot hooks into
`~/.copilot/hooks/berth.json` and Command Code hooks into
`~/.commandcode/settings.json`. All are idempotent and preserve unrelated hooks.
`berth hooks print` shows the recipes without writing. See `docs/05-build.md` for
the acceptance checkpoints.

Command Code's CLI is `cmd` on Unix, `cmdc` on Windows and `command-code`
everywhere; the short `cmd` alias never matches the Windows `cmd.exe` shell.

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

