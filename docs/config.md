# Configuration reference

Berth reads one JSON file at `~/.config/berth/config.json`. Missing keys use
the defaults below. Changes to shortcuts, font, and sidebar density apply
without reinstalling; the socket mode is read at launch.

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
  "compactSidebar": false,
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

## Sidebar

- `compactSidebar: true` reduces row height from 62 to 52 points and hides the
  metadata line (workspace name, cwd, git branch, listening ports).

## Automation socket

`off`, `local`, or `trusted`.

- `local` (default) creates a 0600 token at
  `~/Library/Application Support/Berth/token` and requires it for mutating
  methods. Read-only methods (`*.list`, `*.get`, `system.*`,
  `events.subscribe`) do not need it.
- `trusted` is intended for a same-user automation setup and bypasses the token.
- `off` disables the socket entirely.

Environment overrides for clients and children:

| variable | meaning |
| --- | --- |
| `BERTH_SOCKET_PATH` | socket path override |
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
`socket`, `agent`, `restore`, and `ui`:

```sh
log stream --predicate 'subsystem == "dev.berth.app"' --level debug
```

Uncaught exceptions are appended to `~/Library/Logs/Berth/crash.log`.
